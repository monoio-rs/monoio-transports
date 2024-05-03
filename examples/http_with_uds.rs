use http::request;
use monoio_http::{common::body::HttpBody, h1::payload::Payload};
use monoio_transports::{
    connectors::{Connector, UnixConnector},
    http::H1Connector,
};

const UDS_PATH: &str = "./examples/uds.sock";

#[monoio::main]
async fn main() -> Result<(), monoio_transports::TransportError> {
    let connector: H1Connector<UnixConnector, _, _> = H1Connector::default();
    let mut conn = connector.connect(UDS_PATH).await.unwrap();
    let req = request::Builder::new()
        .uri("/get")
        .header("Host", "test")
        .header("Accept", "*/*")
        .body(HttpBody::H1(Payload::None))
        .unwrap();
    let (res, _) = conn.send_request(req).await;
    let resp = res?;
    assert_eq!(200, resp.status());
    assert_eq!(
        "application/json".as_bytes(),
        resp.headers().get("content-type").unwrap().as_bytes()
    );
    let (header, _) = resp.into_parts();
    println!("resp header: {:?}", header);
    Ok(())
}
