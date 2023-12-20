use http::request;
use monoio::net::UnixStream;
use monoio_http::{common::body::HttpBody, h1::payload::Payload};
use monoio_transports::{
    connectors::{Connector, UnixConnector},
    http::HttpConnector,
    pooled::connector::PooledConnector,
};

const UDS_PATH: &str = "./uds.sock";

type PoolHttpConnector = HttpConnector<PooledConnector<UnixConnector, String, UnixStream, ()>>;

#[monoio::main(enable_timer = true)]
async fn main() -> Result<(), monoio_transports::Error> {
    let connector: PoolHttpConnector = HttpConnector::default();
    let key = UDS_PATH.to_string();
    let mut conn = connector.connect(key.clone()).await.unwrap();
    let req = request::Builder::new()
        .uri("/get")
        .header("Host", "httpbin.org")
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
    drop(conn);

    let conn = connector.connect(key).await.unwrap();
    drop(conn);
    Ok(())
}
