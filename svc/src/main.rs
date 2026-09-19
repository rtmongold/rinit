pub mod live_service;
pub mod live_service_graph;
pub mod pid1;
pub mod request_handler;
pub mod supervision;

use std::{
    cell::RefCell,
    path::{
        Path,
        PathBuf,
    },
    rc::Rc,
};

use eyre::{
    Context,
    Result,
};
use flexi_logger::{
    Cleanup,
    Criterion,
    FileSpec,
    Naming,
    WriteMode,
    writers::FileLogWriter,
};
use lexopt::prelude::{
    Long,
    Short,
};
use live_service_graph::LiveServiceGraph;
use nix::{
    sys::signal::Signal,
    unistd::{
        Pid,
        setpgid,
    },
};
use pid1::{
    FinalAction,
    finalize_as_init,
    is_pid1,
    prepare_early_fs,
    reap_orphans,
};
use request_handler::RequestHandler;
use rinit_ipc::{Request, SystemAction};
use rinit_service::config::Config;
use tokio::{
    fs,
    join,
    net::UnixListener,
    select,
    signal::unix::{
        SignalKind,
        signal,
    },
    sync::{
        Mutex,
        mpsc,
        watch,
    },
    task::{
        self,
        JoinError,
        spawn_local,
    },
};
use tracing::{
    debug,
    error,
    info,
};
use tracing_subscriber::{
    FmtSubscriber,
    filter::LevelFilter,
};

#[macro_use]
extern crate lazy_static;

struct Args {
    config: Option<PathBuf>,
    verbosity: u8,
}

fn parse_args() -> Result<Args, lexopt::Error> {
    let mut config: Option<PathBuf> = None;
    let mut parser = lexopt::Parser::from_env();
    // 0 => Error
    // 1 => Warn
    // 2 => Info
    // 3 => Debug
    // 4 => Trace
    let mut verbosity = 2;
    while let Some(arg) = parser.next()? {
        match arg {
            Short('c') | Long("config") => {
                config = Some(PathBuf::from(parser.value()?));
            }
            Long("help") => {
                println!("Usage: rsvc [-c|--config=CONFIG]");
                std::process::exit(0);
            }
            Short('q') | Long("quiet") => {
                // quiet set it to Warn
                verbosity = 1;
            }
            Short('v') | Long("verbose") => {
                // verbose set it to Debug
                verbosity = 3;
            }
            _ => return Err(arg.unexpected()),
        }
    }

    Ok(Args { config, verbosity })
}

lazy_static! {
    static ref SIGINT: Mutex<tokio::signal::unix::Signal> =
        Mutex::new(signal(SignalKind::interrupt()).unwrap());
    static ref SIGTERM: Mutex<tokio::signal::unix::Signal> =
        Mutex::new(signal(SignalKind::terminate()).unwrap());
    static ref SIGUSR1: Mutex<tokio::signal::unix::Signal> =
        Mutex::new(signal(SignalKind::user_defined1()).unwrap());
    static ref SIGUSR2: Mutex<tokio::signal::unix::Signal> =
        Mutex::new(signal(SignalKind::user_defined2()).unwrap());
}

///Returns (signal, final action when running as PID 1).
/// SIGINT/SIGTERM/SIGUSR2 → poweroff, SIGUSR1 → reboot.
pub async fn signal_wait() -> (Signal, FinalAction) {
    let mut sigint = SIGINT.lock().await;
    let mut sigterm = SIGTERM.lock().await;
    let mut sigusr1 = SIGUSR1.lock().await;
    let mut sigusr2 = SIGUSR2.lock().await;
    select! {
        _ = sigint.recv() => (Signal::SIGINT, FinalAction::Poweroff),
        _ = sigterm.recv() => (Signal::SIGTERM, FinalAction::Poweroff),
        _ = sigusr1.recv() => (Signal::SIGUSR1, FinalAction::Reboot),
        _ = sigusr2.recv() => (Signal::SIGUSR2, FinalAction::Poweroff),
    }
}

fn to_final(action: SystemAction) -> FinalAction {
    match action {
        SystemAction::Poweroff => FinalAction::Poweroff,
        SystemAction::Reboot => FinalAction::Reboot,
        SystemAction::Halt => FinalAction::Halt,
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = parse_args()?;
    let pid1 = is_pid1();
    let config = Config::new(args.config)?;
    if pid1 {
        prepare_early_fs(&config.dirs.rundir, &config.dirs.logdir)?;
    }
    let socket_addr = rinit_ipc::get_host_address(config.mode).to_string();
    // Setup socket listener

    // Setup logging
    let (file_writer, _fw_handle) = FileLogWriter::builder(
        FileSpec::default()
            .directory(&config.dirs.logdir)
            .basename("rinit"),
    )
    .rotate(
        Criterion::Size(1024 * 512),
        Naming::Numbers,
        Cleanup::KeepCompressedFiles(5),
    )
    .append()
    .write_mode(WriteMode::Async)
    .try_build_with_handle()
    .unwrap();

    let subscriber_builder = FmtSubscriber::builder()
        .with_writer(move || file_writer.clone())
        .with_max_level(match args.verbosity {
            0 => LevelFilter::ERROR,
            1 => LevelFilter::WARN,
            2 => LevelFilter::INFO,
            3.. => LevelFilter::DEBUG,
        });

    // Get ready to trace
    tracing::subscriber::set_global_default(subscriber_builder.finish())
        .expect("setting default subscriber failed");

    // Process groups are for daemons; skip when we are init.
    if !pid1 {
        setpgid(Pid::from_raw(0), Pid::from_raw(0))?;
    }

    let (tx, mut rx) = mpsc::channel::<Request>(20);
    let local = task::LocalSet::new();
    let mode = config.mode;
    let live_graph = LiveServiceGraph::new(config, tx.clone())?;

    fs::create_dir_all(Path::new(&socket_addr).parent().unwrap())
        .await
        .unwrap();

    let listener = UnixListener::bind(&socket_addr).wrap_err_with(|| {
        format!(
            "rinit is already running or didn't exit properly. Delete {:?} if needed",
            &socket_addr
        )
    })?;

    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let mut shutdown = shutdown_tx.subscribe();
    let mut shutdown_events = shutdown_tx.subscribe();
    
    // Shared with signals, events, IPC (rctl), and finalize_as_init
    let final_action = Rc::new(RefCell::new(FinalAction::Poweroff));
    let final_action_for_exit = final_action.clone();

    let handler = Rc::new (RequestHandler::new(
        live_graph,
        shutdown_tx,
        mode,
        final_action.clone(),
    ));
    let handles = Rc::new(RefCell::new(Vec::new()));

    local
        .run_until(async move {
            info!("Starting rinit{}.", if pid1 { " as PID 1" } else { "" });

            if pid1 {
                spawn_local(reap_orphans());
            }

            let handler_clone = handler.clone();
            let handles_clone = handles.clone();
            let ipc_handler_future = spawn_local(async move {
                let handler = handler_clone;
                let handles = handles_clone;
                loop {
                    // Accept the connection here, it is simpler than letting RequestHandler do that
                    let conn = select! {
                        res = listener.accept() => {
                            res
                        },
                        _ = shutdown.changed() => {
                            break;
                        }
                    };
                    let stream = match conn {
                        Ok((stream, _addr)) => stream,
                        Err(err) => {
                            error!("error while accepting a new connection: {err}");
                            return;
                        }
                    };
                    let handler = handler.clone();
                    handles.borrow_mut().push(task::spawn_local(async move {
                        if let Err(err) = handler.handle_ipc_stream(stream).await {
                            error!("{err}");
                        }
                    }));
                }
            });

            let handler_clone = handler.clone();
            let handles_clone = handles.clone();
            let final_action_for_events = final_action.clone();
            let events_future = spawn_local(async move {
                let handler = handler_clone;
                let handles = handles_clone;
                loop {
                    let request =  select! {
                        req = rx.recv() => {
                            match req {
                                Some(r) => r,
                                None => break,
                            }
                        }
                        _ = shutdown_events.changed() => break,
                    };
                    let handler = handler.clone();
                    if let Request::StopAllServices { action } = &request {
                        *final_action_for_events.borrow_mut() = to_final(*action);
                        // need a clone of final_action Rc inside events_future
                        if let Err(err) = handler.handle_request(request).await {
                            error!("{err}");
                        }
                        break;
                    }
                    handles.borrow_mut().push(task::spawn_local(async move {
                        if let Err(err) = handler.handle_request(request).await {
                            error!("{err}");
                        }
                    }));
                }
            });

            // Starting rinit consists of 2 different phases
            // The first one is starting the boot services, the second one starts all the
            // other services
            if let Err(err) = tx.send(Request::StartAllServices).await {
                error!("{err}");
            }

            let final_action_clone = final_action.clone();
            let (res1, res2, _) = join! {
                ipc_handler_future,
                events_future,
                async {
                    let signal = select! {
                        signal = signal_wait() => {
                            Some(signal)
                        },
                        _ = shutdown_rx.changed() => {
                            None
                        }
                    };
                    if let Some((signal, action)) = signal  {
                        debug!("received signal {signal} -> {action:?}");
                        *final_action_clone.borrow_mut() = action;
                        if let Err(err) = tx.send(Request::StopAllServices {
                            action: match action {
                                FinalAction::Poweroff => SystemAction::Poweroff,
                                FinalAction::Reboot => SystemAction::Reboot,
                                FinalAction::Halt => SystemAction::Halt,
                            },
                        }).await {
                            error!("{err}");
                        }
                    }
                }
            };
            res1.unwrap();
            res2.unwrap();

            while let Some(handle) = handles.borrow_mut().pop() {
                let res: Result<(), JoinError> = handle.await;
                res.unwrap();
            }
            debug!("awaited on all futures");
        })
        .await;

    let _ = fs::remove_file(socket_addr).await;

    if pid1 {
        finalize_as_init(*final_action_for_exit.borrow());
    }
    
    Ok(())
}
