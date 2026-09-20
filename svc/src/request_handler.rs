use std::{
    cell::RefCell,
    rc::Rc,
};
use futures::{
    prelude::*,
    stream::StreamExt,
};
use remoc::rch;
use rinit_ipc::{
    ConnectionError as ConnectionErrorGeneric,
    Reply,
    Request,
    request_error::RequestError,
    SystemAction,
};
use rinit_service::service_state::{
    IdleServiceState,
    ServiceState,
};
use tokio::{
    net::UnixStream,
    sync::{
        RwLock,
        watch,
    },
    task,
};
use tracing::error;

use crate::{
    live_service::LiveService,
    live_service_graph::{
        LiveGraphError,
        LiveServiceGraph,
    },
};
use crate::pid1::{
    FinalAction,
    console_msg,
};

type ConnectionError = ConnectionErrorGeneric<Result<Reply, RequestError>>;

pub struct RequestHandler {
    graph: RwLock<LiveServiceGraph>,
    stop_ipc: watch::Sender<bool>,
    mode: rinit_service::Mode,
    final_action: Rc<RefCell<FinalAction>>,
}

impl RequestHandler {
    pub fn new(
        graph: LiveServiceGraph,
        stop_ipc: watch::Sender<bool>,
        mode: rinit_service::Mode,
        final_action: Rc<RefCell<FinalAction>>,
    ) -> Self {
        Self {
            graph: RwLock::new(graph),
            stop_ipc,
            mode,
            final_action,
        }
    }

    /// Read IPC messages (i.e. rctl)
    /// This function gets spawned into a separate task, meaning that we don't
    /// need to spawn anything else. This is important because otherwise,
    /// requests couldn't be handled in parallel
    pub async fn handle_ipc_stream(
        &self,
        stream: UnixStream,
    ) -> Result<(), ConnectionError> {
        let (socket_rx, socket_tx) = stream.into_split();
        let (conn, mut tx, mut rx): (
            _,
            rch::base::Sender<Result<Reply, RequestError>>,
            rch::base::Receiver<Request>,
        ) = remoc::Connect::io_buffered(remoc::Cfg::default(), socket_rx, socket_tx, 1024).await?;
        // This has to be spawned in a different task, otherwise everything blocks
        task::spawn_local(conn);
        loop {
            let request = match rx.recv().await {
                Ok(val) => {
                    match val {
                        Some(req) => req,
                        // No new requests
                        None => break,
                    }
                }
                Err(err) => {
                    match err {
                        rch::base::RecvError::Receive(err) if err.is_terminated() => break,
                        rch::base::RecvError::Receive(_)
                        | rch::base::RecvError::Deserialize(_)
                        | rch::base::RecvError::MissingPorts(_)
                        | rch::base::RecvError::MaxItemSizeExceeded => {
                            return Err(ConnectionError::ReceiveError { source: err });
                        }
                    }
                }
            };
            let reply = self.handle_request(request).await;
            tx.send(reply).await?;
        }

        Ok(())
    }

    pub async fn handle_request<'a>(
        &self,
        request: Request,
    ) -> Result<Reply, RequestError> {
        // Take the graph lock per arm and drop it before nested status updates
        // that may need a write lock (see UpdateServiceStatus Down path).
        Ok(match request {
            Request::ServicesStatus => {
                let graph = self.graph.read().await;
                let services: Vec<Result<&LiveService, LiveGraphError>> = graph
                    .live_services
                    .iter()
                    .map(|(_, live_service)| live_service)
                    .map(Result::Ok)
                    .collect();
                let states = stream::iter(services)
                    .then(async move |res| {
                        match res {
                            Ok(live_service) => {
                                Ok((
                                    live_service.node.name().to_owned(),
                                    *live_service.state.borrow(),
                                ))
                            }
                            Err(err) => Err(err),
                        }
                    })
                    .collect::<Vec<_>>()
                    .await;
                Reply::ServicesStates(states.into_iter().collect::<Result<Vec<_>, _>>()?)
            }
            Request::ServiceStatus(service) => {
                let wait = {
                    let graph = self.graph.read().await;
                    graph.get_service(&service)?.wait_idle_state()
                };
                Reply::ServiceState(service.clone(), ServiceState::Idle(wait.await))
            }
            Request::StartService { service } => {
                {
                    let graph = self.graph.read().await;
                    graph.start_service(graph.get_service(&service)?).await?;
                }
                let wait = {
                    let graph = self.graph.read().await;
                    graph.get_service(&service)?.wait_idle_state()
                };
                wait.await;
                Reply::Success()
            }
            Request::StopService { service } => {
                {
                    let graph = self.graph.read().await;
                    graph.stop_service(graph.get_service(&service)?).await?;
                }
                let wait = {
                    let graph = self.graph.read().await;
                    graph.get_service(&service)?.wait_idle_state()
                };
                wait.await;
                Reply::Success()
            }
            Request::StartAllServices => {
                if self.mode == rinit_service::Mode::Root {
                    console_msg("StartAllServices: Boot target");
                    {
                        let graph = self.graph.read().await;
                        graph
                            .start_target(rinit_service::graph::Target::Boot)
                            .await;
                    }
                    console_msg("StartAllServices: Boot done");
                }
                console_msg("StartAllServices: Default target");
                {
                    let graph = self.graph.read().await;
                    graph
                        .start_target(rinit_service::graph::Target::Default)
                        .await;
                }
                console_msg("StartAllServices: Default done");
                Reply::Empty
            }
            // This request can be generated by rctl or by sending a SIGTERM/SIGINT
            Request::StopAllServices { action } => {
                *self.final_action.borrow_mut() = match action {
                    SystemAction::Poweroff => FinalAction::Poweroff,
                    SystemAction::Reboot => FinalAction::Reboot,
                    SystemAction::Halt => FinalAction::Halt,
                };
                // Stop listening to IPC requests
                if let Err(err) = self.stop_ipc.send(true) {
                    error!("could not stop listening on IPC socket: {err}");
                }
                {
                    let graph = self.graph.read().await;
                    graph
                        .stop_target(rinit_service::graph::Target::Default)
                        .await;
                }
                if self.mode == rinit_service::Mode::Root {
                    let graph = self.graph.read().await;
                    graph
                        .stop_target(rinit_service::graph::Target::Boot)
                        .await;
                }
                Reply::Empty
            }
            Request::ReloadGraph => {
                let mut graph = self.graph.write().await;
                graph.reload_dependency_graph().await?;
                Reply::Empty
            }
            Request::UpdateServiceStatus(name, state) => {
                {
                    let graph = self.graph.read().await;
                    graph.update_service_state(&name, state)?;
                }
                // Write lock only after the read guard is dropped.
                if state == IdleServiceState::Down {
                    let mut graph = self.graph.write().await;
                    graph.update_service(&name)?;
                }
                Reply::Empty
            }
        })
    }
}
