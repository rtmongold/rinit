use clap::Parser;
use eyre::{
    Result,
    ensure,
};
use rinit_ipc::AsyncConnection;
use rinit_service::config::Config;

use crate::util::start_service;

#[derive(Parser)]
pub struct StartCommand {
    services: Vec<String>,
}

impl StartCommand {
    pub async fn run(
        self,
        config: Config,
    ) -> Result<()> {
        // TODO: Print duplicated service
        ensure!(
            !(1..self.services.len()).any(|i| self.services[i..].contains(&self.services[i - 1])),
            "duplicated service found"
        );

        let mut conn = AsyncConnection::new_host_address(config.mode).await?;
        let mut error = false;
        for service in self.services {
            if start_service(&mut conn, &service).await? {
                println!("Service {service} started successfully.");
            } else {
                println!("Service {service} failed to start.");
                error = true;
            }
        }

        ensure!(!error, "");
        Ok(())
    }
}
