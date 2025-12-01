use std::{
    cell::RefCell,
    rc::Rc,
};

use clap::Parser;
use eyre::{
    Result,
    ensure,
};
use rinit_ipc::{
    AsyncConnection,
    Reply,
    Request,
};
use rinit_service::config::Config;
use tokio::task;

#[derive(Parser)]
pub struct StopCommand {
    services: Vec<String>,
}

impl StopCommand {
    pub async fn run(
        self,
        config: Config,
    ) -> Result<()> {
        // TODO: Print duplicated service
        ensure!(
            !(1..self.services.len()).any(|i| self.services[i..].contains(&self.services[i - 1])),
            "duplicated service found"
        );
        let conn = Rc::new(RefCell::new(
            AsyncConnection::new_host_address(config.mode).await?,
        ));
        let handles: Vec<task::JoinHandle<Result<bool>>> = self
            .services
            .into_iter()
            .map(
                move |service| -> task::JoinHandle<std::result::Result<bool, _>> {
                    let conn = conn.clone();
                    task::spawn_local(async move {
                        let request = Request::StopService {
                            service: service.clone(),
                        };
                        let res = conn.borrow_mut().send_request(request).await?;

                        match res {
                            Ok(reply) => {
                                match reply {
                                    Reply::Success() => {
                                        println!("Service {service} stopped successfully.");
                                        Ok(true)
                                    }
                                    _ => unreachable!(),
                                }
                            }
                            Err(err) => {
                                eprintln!("Service {service} failed to stop: {err}");
                                Ok(false)
                            }
                        }
                    })
                },
            )
            .collect();

        let mut _success = false;
        for handle in handles {
            _success |= handle.await??
        }

        Ok(())
    }
}
