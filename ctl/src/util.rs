use eyre::Result;
use rinit_ipc::{
    AsyncConnection,
    Reply,
    Request,
};

pub async fn start_service(
    conn: &mut AsyncConnection,
    service: &str,
) -> Result<bool> {
    let request = Request::StartService {
        service: service.to_owned(),
    };
    match conn.send_request(request).await?? {
        Reply::Success() => Ok(true),
        _ => unreachable!(),
    }
}

pub async fn stop_service(
    conn: &mut AsyncConnection,
    service: &str,
) -> Result<bool> {
    let request = Request::StartService {
        service: service.to_owned(),
    };
    match conn.send_request(request).await?? {
        Reply::Success() => Ok(true),
        _ => unreachable!(),
    }
}
