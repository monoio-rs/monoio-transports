use std::net::ToSocketAddrs;

use http::{request, Uri};
use monoio_http::{
    common::{body::HttpBody, request::RequestForEncoder, BorrowHeaderMap, FromParts},
    h1::payload::{FixedPayload, Payload},
};
use monoio_transports::{
    connectors::{Connector, TcpConnector},
    http::H1Connector,
};

#[monoio::main]
async fn main() -> Result<(), monoio_transports::Error> {
    #[derive(Debug, Clone, Eq, PartialEq, Hash)]
    struct Key {
        host: String,
        port: u16,
    }

    impl ToSocketAddrs for Key {
        type Iter = std::vec::IntoIter<std::net::SocketAddr>;
        fn to_socket_addrs(&self) -> std::io::Result<Self::Iter> {
            (self.host.as_str(), self.port).to_socket_addrs()
        }
    }

    let connector: H1Connector<TcpConnector, _, _> = H1Connector::default();
    let uri = "http://httpbin.org/get".parse::<Uri>().unwrap();
    let host = uri.host().unwrap();
    let port = uri.port_u16().unwrap_or(80);
    let key = Key {
        host: host.to_string(),
        port,
    };

    let mut conn = connector.connect(key).await.unwrap();
    let req = request::Builder::new()
        .uri("/get")
        .header("Host", "httpbin.org")
        .body(HttpBody::H1(Payload::Fixed(FixedPayload::new(
            Bytes::from_static(b"hello world"),
        ))))
        .unwrap();
    let (mut parts, body) = req.into_parts();
    let req = RequestForEncoder::from_parts(&mut parts, body.clone());
    let (res, _) = conn.send_request(req).await;
    let resp = res?;
    assert_eq!(200, resp.status());
    assert_eq!(
        "application/json".as_bytes(),
        resp.headers().get("content-type").unwrap().as_bytes()
    );
    let (header, _) = resp.into_parts();
    let req_header = parts.header_map();

    println!("resp header: {:?}", header);
    println!("req header: {:?}", req_header);
    println!("req body: {:?}", body);

    Ok(())
}
