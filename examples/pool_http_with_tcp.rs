use http::{request, Uri};
use monoio::net::TcpStream;
use monoio_http::{common::body::HttpBody, h1::payload::Payload};
use monoio_transports::{
    connectors::{Connector, TcpConnector},
    http::HttpConnector,
    key::Key,
    pooled::connector::PooledConnector,
};

type PoolHttpConnector = HttpConnector<PooledConnector<TcpConnector, Key, TcpStream>>;

#[monoio::main(enable_timer = true)]
async fn main() -> Result<(), monoio_transports::Error> {
    let connector: PoolHttpConnector = HttpConnector::default();
    let uri = "http://httpbin.org/get".parse::<Uri>().unwrap();
    let key: Key = uri.try_into().unwrap();
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

    let req = request::Builder::new()
        .uri("/")
        .header("Host", "httpbin.org")
        .body(HttpBody::H1(Payload::None))
        .unwrap();
    let mut conn = connector.connect(key).await.unwrap();
    let (res, _) = conn.send_request(req).await;
    let resp = res?;
    assert_eq!(200, resp.status());
    assert_eq!(
        "text/html; charset=utf-8".as_bytes(),
        resp.headers().get("content-type").unwrap().as_bytes()
    );
    let (header, _) = resp.into_parts();
    println!("resp header: {:?}", header);
    Ok(())
}
