use http::{request, Uri};
use monoio_http::{common::body::HttpBody, h1::payload::Payload};
use monoio_transports::{
    connectors::{Connector, TcpConnector, TcpTlsAddr},
    http::HttpsConnector,
};

#[monoio::main]
async fn main() -> Result<(), monoio_transports::TransportError> {
    // Https conector with HTTP_2 and HTTP_11 support.
    // TLS ALPN is set to h2, http/1.1. This will make the connector
    // to use HTTP_2 if the server supports it, otherwise it will fallback to HTTP_11.
    let mut connector: HttpsConnector<TcpConnector, _, _> = HttpsConnector::default();
    connector.h2_builder().max_concurrent_streams(150);
    let uri = "https://httpbin.org/get".parse::<Uri>().unwrap();
    let addr: TcpTlsAddr = uri.try_into().unwrap();
    let mut conn = connector.connect(addr).await.unwrap();

    for _i in 0..10 {
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

        assert_eq!(resp.version(), http::Version::HTTP_2);
    }

    // Https conenctor with HTTP_11 support only.
    // TLS ALPN is set to http/1.1. This will make the connector to use HTTP_11 only.
    let connector: HttpsConnector<TcpConnector, _, _> = HttpsConnector::http1_only();
    let uri = "https://httpbin.org/get".parse::<Uri>().unwrap();
    let addr: TcpTlsAddr = uri.try_into().unwrap();
    let mut conn = connector.connect(addr).await.unwrap();

    for _i in 0..10 {
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
        assert_eq!(resp.version(), http::Version::HTTP_11);
    }

    Ok(())
}
