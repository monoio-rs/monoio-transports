use std::path::PathBuf;

use bytes::BytesMut;
use monoio::{
    io::{AsyncReadRent, AsyncWriteRentExt, Splitable},
    net::{UnixListener, UnixStream},
};

const UDS_PATH: &str = "./uds.sock";
const RESP: &[u8;103] = b"HTTP/1.1 200 OK\r\nContent-Length: 17\r\nContent-Type: application/json\r\nServer: hertz\r\n\r\n{\"hello\":\"world\"}";

async fn handle(stream: UnixStream) -> Result<(), std::io::Error> {
    let (mut read, mut write) = stream.into_split();

    let buffer = BytesMut::with_capacity(1024);
    let (res, buffer) = read.read(buffer).await;
    let res = res.unwrap();
    println!("read {} bytes", res);
    println!("read: {}", String::from_utf8_lossy(&buffer[..res]));

    let (res, _) = write.write_all(RESP).await;
    res.unwrap();

    Ok(())
}

#[monoio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(UDS_PATH);
    let _ = std::fs::remove_file(&path);
    println!("listen on {:?}", path);
    let listener = UnixListener::bind(&path).unwrap();
    loop {
        let stream = { listener.accept().await.unwrap() }.0;
        monoio::spawn(async move {
            let _ = handle(stream).await;
        });
    }
}
