use std::{
    net::{SocketAddr, ToSocketAddrs},
    time::Duration,
};

use bytes::{Bytes, BytesMut};
use monoio::{io::sink::SinkExt, net::TcpStream};
use monoio_codec::{Decoded, Decoder, Encoder};
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

struct RawDecoder {
    name: String,
}

impl RawDecoder {
    fn new(name: String) -> Self {
        Self { name }
    }
}

impl Decoder for RawDecoder {
    type Item = Bytes;
    type Error = std::io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Decoded<bytes::Bytes>, std::io::Error> {
        println!("Decoder: {}", self.name);
        Ok(Decoded::Some(src.split().freeze()))
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
    let key = ("52.206.0.51", 80 as u16)
        .to_socket_addrs()
        .unwrap()
        .next()
        .unwrap();

    {
        let mut conn = connector.connect(key).await.unwrap();
        let mut framed = conn.map_codec(MyCodec::new(RawEncoder::new("raw".to_string()), false));
        let mut decoder = RawDecoder::new("raw".to_string());

        let data = "GET /get HTTP/1.1\r\nHost: httpbin.org\r\n\r\n";
        match framed
            .send_and_flush(Bytes::from_static(data.as_bytes()))
            .await
        {
            Ok(_) => {
                println!("Start to receive data");
                let mut buf = BytesMut::new();
                while let Ok(Some(Ok(item))) =
                    monoio::time::timeout(Duration::from_secs(2), framed.next_with(&mut decoder))
                        .await
                {
                    buf.extend_from_slice(&item);
                    println!("Received: {:?}", buf);
                }
            }
            Err(e) => println!("Failed to send data: {:?}", e),
        }
    }

    {
        let _ = connector.connect(key).await.unwrap();
    }

    Ok(())
}