use rinit_service::service_state::IdleServiceState;
use serde::{
    Deserialize,
    Serialize,
};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Request {
    UpdateServiceStatus(String, IdleServiceState),
    ServicesStatus,
    ServiceStatus(String),
    StartService { service: String },
    StopService { service: String },
    StartAllServices,
    StopAllServices,
    ReloadGraph,
}
