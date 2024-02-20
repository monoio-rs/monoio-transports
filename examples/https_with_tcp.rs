use http::{request, Uri};
use monoio_http::{common::body::HttpBody, h1::payload::Payload};
use monoio_transports::{
    connectors::{Connector, TcpConnector, TcpTlsAddr, TlsConnector},
    http::H1Connector,
};

#[monoio::main]
async fn main() -> Result<(), monoio_transports::Error> {
    let connector: H1Connector<TlsConnector<TcpConnector>, _, _> = H1Connector::default();
    let uri = "https://httpbin.org/get".parse::<Uri>().unwrap();
    let unified_addr = TcpTlsAddr::try_from(&uri).unwrap();
    let mut conn = connector.connect(unified_addr).await.unwrap();
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
    Ok(())
}
