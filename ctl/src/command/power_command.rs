use clap::Parser;
use eyre::Result;
use rinit_ipc::{
    AsyncConnection,
    Request,
    SystemAction,
};
use rinit_service::config::Config;

#[derive(Parser)]
pub struct PoweroffCommand {}

#[derive(Parser)]
pub struct RebootCommand {}

#[derive(Parser)]
pub struct HaltCommand {}

async fn stop_all(config: Config, action: SystemAction) -> Result<()> {
    let mut conn = AsyncConnection::new_host_address(config.mode).await?;
    conn.send_request(Request::StopAllServices { action }).await??;
    Ok(())
}

impl PoweroffCommand {
    pub async fn run(self, config: Config) -> Result<()> {
        stop_all(config, SystemAction::Poweroff).await
    }
}
impl RebootCommand {
    pub async fn run(self, config: Config) -> Result<()> {
        stop_all(config, SystemAction::Reboot).await
    }
}
impl HaltCommand {
    pub async fn run(self, config: Config) -> Result<()> {
        stop_all(config, SystemAction::Halt).await
    }
}
