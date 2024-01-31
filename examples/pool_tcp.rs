use std::net::{SocketAddr, ToSocketAddrs};

use bytes::{Bytes, BytesMut};
use monoio::{io::sink::SinkExt, net::TcpStream};
use monoio_codec::Encoder;
use monoio_transports::{
    connectors::{Connector, TcpConnector},
    pooled::connector::PooledConnector,
};

type PoolTcpConnector = PooledConnector<TcpConnector, SocketAddr, TcpStream>;

struct RawEncoder {
    name: String,
}

impl RawEncoder {
    fn new(name: String) -> Self {
        Self { name }
    }
}

impl Encoder<Bytes> for RawEncoder {
    type Error = std::io::Error;

    fn encode(&mut self, item: Bytes, dst: &mut BytesMut) -> Result<(), Self::Error> {
        println!("Encoder: {}", self.name);
        dst.extend_from_slice(&item);
        Ok(())
    }
}

struct MyCodec {
    inner: RawEncoder,
    framed_enable: bool,
}

impl MyCodec {
    fn new(inner: RawEncoder, framed_enable: bool) -> Self {
        Self {
            inner,
            framed_enable,
        }
    }
}

impl Encoder<Bytes> for MyCodec {
    type Error = std::io::Error;

    fn encode(&mut self, item: Bytes, dst: &mut BytesMut) -> Result<(), Self::Error> {
        println!("Enable frame: {}", self.framed_enable);
        self.inner.encode(item, dst)
    }
}

#[monoio::main(enable_timer = true)]
async fn main() -> Result<(), monoio_transports::Error> {
    let connector = PoolTcpConnector::default();
    let key = ("127.0.0.1", 5000 as u16)
        .to_socket_addrs()
        .unwrap()
        .next()
        .unwrap();

    let conn = connector.connect(key).await.unwrap();
    let mut conn = conn.map_codec(|| MyCodec::new(RawEncoder::new("raw".to_string()), false));
    let data = "GET /get HTTP/1.1\r\nHost: httpbin.org\r\n\r\n";
    let _ = conn
        .send_and_flush(Bytes::from_static(data.as_bytes()))
        .await;

    Ok(())
}
