use std::net::ToSocketAddrs;

use bytes::Bytes;
use http::{request, Uri};
use monoio::net::TcpListener;
use monoio_http::{
    common::{
        body::{Body, FixedBody, HttpBody},
        request::Request,
    },
    h1::payload::Payload,
};
use monoio_transports::{
    connectors::{Connector, TcpConnector},
    http::HttpConnector,
};

async fn serve_h2(
    io: monoio::net::TcpStream,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut connection = monoio_http::h2::server::handshake(io).await?;
    println!("H2 connection bound");

    while let Some(result) = connection.accept().await {
        let (request, send_resp) = result?;
        monoio::spawn(async move {
            let (parts, body) = request.into_parts();
            if let Err(e) =
                handle_request_h2(Request::from_parts(parts, body.into()), send_resp).await
            {
                println!("error while handling request: {e}");
            }
        });
    }

    println!("~~~~~~~~~~~ H2 connection CLOSE !!!!!! ~~~~~~~~~~~");
    Ok(())
}

async fn handle_request_h2(
    mut request: Request<HttpBody>,
    mut respond: monoio_http::h2::server::SendResponse<bytes::Bytes>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("GOT request: {request:?}");

    let body = request.body_mut();
    while let Some(data) = body.next_data().await {
        let data = data?;
        println!("<<<< recv {data:?}");
    }

    let response = http::Response::new(());
    let mut send = respond.send_response(response, false)?;
    println!(">>>> send");
    send.send_data(bytes::Bytes::from_static(b"hello "), false)?;
    send.send_data(bytes::Bytes::from_static(b"world\n"), true)?;

    Ok(())
}

#[monoio::main]
async fn main() -> Result<(), monoio_transports::TransportError> {
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

    // Plain text HTTP_1.1 connector.
    let connector: HttpConnector<TcpConnector, _, _> = HttpConnector::default();
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

    let listener = TcpListener::bind("127.0.0.1:5928").unwrap();
    println!("listening on {:?}", listener.local_addr());
    monoio::spawn(async move {
        loop {
            if let Ok((socket, _peer_addr)) = listener.accept().await {
                monoio::spawn(async move {
                    if let Err(e) = serve_h2(socket).await {
                        println!("  -> err={e:?}");
                    }
                });
            }
        }
    });

    // Construct uri and request
    let uri = "http://127.0.0.1:5928/get".parse::<Uri>().unwrap();
    let host = uri.host().unwrap();
    let port = uri.port_u16().unwrap_or(80);
    let key = Key {
        host: host.to_string(),
        port,
    };

    // Plain text HTTP_2 connector
    let connector = HttpConnector::build_tcp_http2_only();
    for _ in 0..3 {
        let mut pooled_conn = connector.connect(key.clone()).await.unwrap();
        let req = http::Request::builder()
            .uri(uri.clone())
            .body(HttpBody::fixed_body(Some(Bytes::from_static(
                b"hello world",
            ))))
            .unwrap();
        let (resp, _) = pooled_conn.send_request(req).await;
        let resp = resp.unwrap();
        println!("H2 Response status: {}", resp.status());

        // Get the body
        let mut body = resp.into_body();
        while let Some(chunk) = body.next_data().await {
            println!("GOT CHUNK = {:?}", chunk);
        }
    }

    Ok(())
}
