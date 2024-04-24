mod body;
use std::{
    cell::UnsafeCell,
    collections::HashMap,
    ops::{Deref, DerefMut},
    rc::Rc,
};

pub use body::{HyperBody, MonoioBody};
use hyper::{
    body::Body,
    client::conn,
    rt::{Read, Write},
};
use thiserror::Error as ThisError;

use crate::{
    connectors::Connector,
    pool::{ConnectionPool, Key, Poolable, Pooled},
};

pub struct HyperH1Connector<C, K, B> {
    connector: C,
    pool: ConnectionPool<K, HyperH1Connection<B>>,
}

impl<C, K: 'static, B: 'static> HyperH1Connector<C, K, B> {
    #[inline]
    pub fn new(connector: C) -> Self {
        Self {
            connector,
            pool: ConnectionPool::new(None),
        }
    }

    #[inline]
    pub const fn new_with_pool(
        connector: C,
        pool: ConnectionPool<K, HyperH1Connection<B>>,
    ) -> Self {
        Self { connector, pool }
    }
}

pub struct HyperH2Connector<C, K, B> {
    connector: C,
    connecting: UnsafeCell<HashMap<K, Rc<tokio::sync::Mutex<()>>>>,
    pool: ConnectionPool<K, HyperH2Connection<B>>,
}

impl<C, K: 'static, B: 'static> HyperH2Connector<C, K, B> {
    #[inline]
    pub fn new(connector: C) -> Self {
        Self {
            connector,
            connecting: UnsafeCell::new(HashMap::new()),
            pool: ConnectionPool::new(None),
        }
    }

    #[inline]
    pub fn new_with_pool(connector: C, pool: ConnectionPool<K, HyperH2Connection<B>>) -> Self {
        Self {
            connector,
            pool,
            connecting: UnsafeCell::new(HashMap::new()),
        }
    }
}

#[derive(Debug)]
pub struct HyperH1Connection<B> {
    tx: conn::http1::SendRequest<B>,
}

#[derive(Debug, Clone)]
pub struct HyperH2Connection<B> {
    tx: conn::http2::SendRequest<B>,
}

impl<B> HyperH2Connection<B> {
    #[inline]
    pub fn to_owned(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl<B> Deref for HyperH1Connection<B> {
    type Target = conn::http1::SendRequest<B>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

impl<B> DerefMut for HyperH1Connection<B> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.tx
    }
}

impl<B> Poolable for HyperH1Connection<B> {
    fn is_open(&self) -> bool {
        !self.tx.is_closed()
    }
}

impl<B> Deref for HyperH2Connection<B> {
    type Target = conn::http2::SendRequest<B>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

impl<B> DerefMut for HyperH2Connection<B> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.tx
    }
}

impl<B> Poolable for HyperH2Connection<B> {
    fn is_open(&self) -> bool {
        !self.tx.is_closed()
    }
}

#[derive(ThisError, Debug)]
pub enum HyperError<CE> {
    #[error("Connection error")]
    Connect(CE),
    #[error("Hyper error {0}")]
    Hyper(#[from] hyper::Error),
}

impl<C, K, T, B> Connector<K> for HyperH1Connector<C, K, B>
where
    C: Connector<K, Connection = T>,
    K: Key,
    T: Read + Write + Unpin + 'static,
    B: Body + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    type Connection = Pooled<K, HyperH1Connection<B>>;
    type Error = HyperError<C::Error>;

    async fn connect(&self, key: K) -> Result<Self::Connection, Self::Error> {
        while let Some(pooled) = self.pool.get(&key) {
            if pooled.tx.is_ready() {
                return Ok(pooled);
            }
        }
        let underlying = self
            .connector
            .connect(key.clone())
            .await
            .map_err(HyperError::Connect)?;
        // TODO: use builder to allow user specify parameters
        let (tx, conn) = conn::http1::handshake::<_, B>(underlying).await?;
        monoio::spawn(conn);
        let pooled = self.pool.link(key, HyperH1Connection { tx });
        Ok(pooled)
    }
}

impl<C, K, T, B> Connector<K> for HyperH2Connector<C, K, B>
where
    C: Connector<K, Connection = T>,
    K: Key,
    T: Read + Write + Unpin + 'static,
    B: Body + Unpin + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    type Connection = HyperH2Connection<B>;
    type Error = HyperError<C::Error>;

    async fn connect(&self, key: K) -> Result<Self::Connection, Self::Error> {
        if let Some(pooled) = self.pool.and_then_mut(&key, |conns| {
            while let Some(first) = conns.front_mut() {
                if first.conn.is_ready() {
                    first.reset_idle();
                    return Some(first.conn.to_owned());
                }
                conns.pop_front();
            }
            None
        }) {
            return Ok(pooled);
        }
        let lock = {
            let connecting = unsafe { &mut *self.connecting.get() };
            let lock = connecting
                .entry(key.clone())
                .or_insert_with(|| Rc::new(tokio::sync::Mutex::new(())));
            lock.clone()
        };

        // get lock and try again
        let _guard = lock.lock().await;
        if let Some(pooled) = self.pool.and_then_mut(&key, |conns| {
            while let Some(first) = conns.front_mut() {
                if first.conn.is_ready() {
                    first.reset_idle();
                    return Some(first.conn.to_owned());
                }
                conns.pop_front();
            }
            None
        }) {
            return Ok(pooled);
        }
        // create new h2 connection
        let underlying = self
            .connector
            .connect(key.clone())
            .await
            .map_err(HyperError::Connect)?;
        let exec = monoio_compat::hyper::MonoioExecutor;
        // TODO: use builder to allow user specify parameters
        let (tx, conn) = conn::http2::handshake::<_, _, B>(exec, underlying).await?;
        monoio::spawn(conn);
        self.pool.put(key, HyperH2Connection { tx: tx.clone() });
        Ok(HyperH2Connection { tx })
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Write, net::ToSocketAddrs};

    use bytes::Bytes;
    use http::Uri;
    use monoio::{io::IntoPollIo, net::TcpListener};

    use super::*;
    use crate::connectors::{pollio::PollIo, TcpConnector};

    #[monoio::test(timer = true)]
    async fn h1_hyper() {
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

        let uri = "http://httpbin.org/get".parse::<Uri>().unwrap();
        let host = uri.host().unwrap();
        let port = uri.port_u16().unwrap_or(80);
        let key = Key {
            host: host.to_string(),
            port,
        };

        use http_body_util::{BodyExt, Empty};
        let connector: HyperH1Connector<_, _, Empty<Bytes>> =
            HyperH1Connector::new(PollIo(TcpConnector::default()));
        let mut pooled_conn = connector.connect(key.clone()).await.unwrap();
        let req = http::Request::builder()
            .header(hyper::header::HOST, host.to_string())
            .uri(&uri)
            .body(Empty::<Bytes>::new())
            .unwrap();
        let mut resp = pooled_conn.send_request(req).await.unwrap();
        println!(
            "H1 Response status: {}\nRead with hyper body interface",
            resp.status()
        );
        while let Some(next) = resp.frame().await {
            let frame = next.unwrap();
            if let Some(chunk) = frame.data_ref() {
                std::io::stdout().write_all(chunk).unwrap();
            }
        }

        let mut pooled_conn = connector.connect(key).await.unwrap();
        let req = http::Request::builder()
            .header(hyper::header::HOST, host.to_string())
            .uri(uri)
            .body(Empty::<Bytes>::new())
            .unwrap();
        let resp = pooled_conn.send_request(req).await.unwrap();
        println!(
            "H1 Response status: {}\nRead with monoio-http body interface",
            resp.status()
        );
        let body = MonoioBody::new(resp.into_body());
        let bytes = monoio_http::common::body::BodyExt::bytes(body)
            .await
            .unwrap();
        std::io::stdout().write_all(&bytes).unwrap();
    }

    #[monoio::test(timer = true)]
    async fn h2_hyper() {
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

        // Start background h2 server
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

        use http_body_util::{BodyExt, Empty};
        let connector: HyperH2Connector<_, _, Empty<Bytes>> =
            HyperH2Connector::new(PollIo(TcpConnector::default()));
        for _ in 0..3 {
            let mut pooled_conn = connector.connect(key.clone()).await.unwrap();
            let req = http::Request::builder()
                .uri(uri.clone())
                .body(Empty::<Bytes>::new())
                .unwrap();
            let mut resp = pooled_conn.send_request(req).await.unwrap();
            println!("H2 Response status: {}", resp.status());
            while let Some(next) = resp.frame().await {
                let frame = next.unwrap();
                if let Some(chunk) = frame.data_ref() {
                    std::io::stdout().write_all(chunk).unwrap();
                }
            }
        }
    }

    // Borrowed from monoio example.
    async fn serve_h2(
        io: monoio::net::TcpStream,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let io = io.into_poll_io()?;
        let mut connection = h2::server::handshake(io).await?;
        println!("H2 connection bound");

        while let Some(result) = connection.accept().await {
            let (request, respond) = result?;
            monoio::spawn(async move {
                if let Err(e) = handle_request_h2(request, respond).await {
                    println!("error while handling request: {e}");
                }
            });
        }

        println!("~~~~~~~~~~~ H2 connection CLOSE !!!!!! ~~~~~~~~~~~");
        Ok(())
    }

    // Borrowed from monoio example.
    async fn handle_request_h2(
        mut request: http::Request<h2::RecvStream>,
        mut respond: h2::server::SendResponse<bytes::Bytes>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        println!("GOT request: {request:?}");

        let body = request.body_mut();
        while let Some(data) = body.data().await {
            let data = data?;
            println!("<<<< recv {data:?}");
            let _ = body.flow_control().release_capacity(data.len());
        }

        let response = http::Response::new(());
        let mut send = respond.send_response(response, false)?;
        println!(">>>> send");
        send.send_data(bytes::Bytes::from_static(b"hello "), false)?;
        send.send_data(bytes::Bytes::from_static(b"world\n"), true)?;

        Ok(())
    }
}
