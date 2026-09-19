use rinit_service::service_state::IdleServiceState;
use serde::{
    Deserialize,
    Serialize,
};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemAction {
    Poweroff,
    Reboot,
    Halt,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Request {
    UpdateServiceStatus(String, IdleServiceState),
    ServicesStatus,
    ServiceStatus(String),
    StartService { service: String },
    StopService { service: String },
    StartAllServices,
    /// Stop all services, then (if PID 1) finalize with this action.
    StopAllServices { action: SystemAction },
    ReloadGraph,
}
