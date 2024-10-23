//! HTTP connectors using Hyper for Monoio-based applications.
//!
//! This module provides HTTP/1.1 and HTTP/2 connectors that integrate Hyper with Monoio,
//! allowing for efficient HTTP connections with built-in connection pooling. These connectors
//! are designed to work with poll-based I/O, making them compatible with various Monoio
//! transport layers.
//!
//! Key components:
//!
//! - [`HyperH1Connector`]: An HTTP/1.1 connector with connection pooling.
//! - [`HyperH2Connector`]: An HTTP/2 connector with connection pooling and stream management.
//! - [`HyperH1Connection`] and [`HyperH2Connection`]: Represent active HTTP connections.
//! - [`HyperError`]: Error type for Hyper connector operations.
//!
//! These connectors can be used with various underlying transport mechanisms (e.g., TCP, TLS)
//! by wrapping them with the appropriate Monoio connectors.
//!
//! Usage examples are provided for both HTTP/1.1 and HTTP/2 connectors, demonstrating how to
//! create and use them with different transport layers.
//!
//! Note: The HTTP/2 implementation includes a custom stream management system to work around
//! limitations in Hyper's stream counting capabilities.
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
    client::conn::{self, http1::Builder as H1Builder, http2::Builder as H2Builder},
    rt::{Read, Write},
};
pub use monoio_compat::hyper::{MonoioExecutor, MonoioTimer};
use thiserror::Error as ThisError;

use crate::{
    connectors::Connector,
    pool::{ConnectionPool, Key, Poolable, Pooled},
};

/// HTTP/1.1 connector using Hyper with built-in connection pooling.
///
/// This connector manages a pool of HTTP/1.1 connections, allowing for efficient
/// reuse of established connections.
pub struct HyperH1Connector<C, K, B> {
    connector: C,
    pool: ConnectionPool<K, HyperH1Connection<B>>,
    builder: H1Builder,
}

impl<C, K: 'static, B: 'static> HyperH1Connector<C, K, B> {
    /// Creates a new HyperH1Connector with default settings and connection pooling.
    ///
    /// # Arguments
    /// * `connector` - The underlying connector to use for establishing connections.
    ///
    /// # Examples
    ///
    /// Creating an HTTP connector:
    /// ```rust
    /// use bytes::Bytes;
    /// use http_body_util::Empty;
    /// use monoio_transports::connectors::{HyperH1Connector, PollIo, TcpConnector};
    ///
    /// let http_connector: HyperH1Connector<_, _, Empty<Bytes>> =
    ///     HyperH1Connector::new(PollIo(TcpConnector::default()));
    /// ```
    ///
    /// Creating an HTTPS connector:
    /// ```rust
    /// use bytes::Bytes;
    /// use http_body_util::Empty;
    /// use monoio_transports::connectors::{HyperH1Connector, PollIo, TcpConnector, TlsConnector};
    ///
    /// let tls_connector = TlsConnector::new(TcpConnector::default(), Default::default());
    /// let https_connector: HyperH1Connector<_, _, Empty<Bytes>> =
    ///     HyperH1Connector::new(PollIo(tls_connector));
    /// ```
    #[inline]
    pub fn new(connector: C) -> Self {
        Self {
            connector,
            pool: ConnectionPool::new(None),
            builder: H1Builder::new(),
        }
    }

    /// Creates a new HyperH1Connector with a connection pool.
    #[inline]
    pub fn new_with_pool(connector: C, pool: ConnectionPool<K, HyperH1Connection<B>>) -> Self {
        Self {
            connector,
            pool,
            builder: H1Builder::new(),
        }
    }
}

impl<C, K, B> HyperH1Connector<C, K, B> {
    /// Sets the Hyper builder for the connector.
    #[inline]
    pub fn with_hyper_builder(mut self, builder: H1Builder) -> Self {
        self.builder = builder;
        self
    }

    /// Returns a mutable reference to the Hyper builder.
    #[inline]
    pub fn hyper_builder(&mut self) -> &mut H1Builder {
        &mut self.builder
    }

    #[inline]
    pub fn get_connection_pool(&self) -> &ConnectionPool<K, HyperH1Connection<B>> {
        &self.pool
    }
}

// TODO: maybe use h2 crate directly?
/// HTTP/2 connector using Hyper with built-in connection pooling.
///
/// This connector manages a pool of HTTP/2 connections, allowing for efficient
/// reuse of established connections and multiplexing of streams.
pub struct HyperH2Connector<C, K, B> {
    connector: C,
    connecting: UnsafeCell<HashMap<K, Rc<tokio::sync::Mutex<()>>>>,
    pool: ConnectionPool<K, HyperH2Connection<B>>,
    builder: H2Builder<MonoioExecutor>,
    max_stream_sender: Option<usize>,
}

impl<C, K, B> HyperH2Connector<C, K, B> {
    /// Sets the Hyper builder for the connector.
    #[inline]
    pub fn with_hyper_builder(mut self, mut builder: H2Builder<MonoioExecutor>) -> Self {
        builder.timer(MonoioTimer);
        self.builder = builder;
        self
    }

    /// Returns a mutable reference to the Hyper builder.
    #[inline]
    pub fn hyper_builder(&mut self) -> &mut H2Builder<MonoioExecutor> {
        &mut self.builder
    }

    /// Peers may set max stream per connection, which may cause server side return
    /// error or client side wait.
    /// To avoid it, we may open new connection when too many open streams. However,
    /// there's no way to get the count of current active streams with hyper, nor the
    /// negotiation of max stream per connection.
    /// So here we can make a work around by tracking sender counts and assume it's close
    /// to the real stream count. When it reaches the limit, we open a new connection.
    /// To make it work, users need to hold sender until the received the response and read its
    /// body.
    #[inline]
    pub fn with_max_stream_sender(mut self, max_stream_sender: Option<usize>) -> Self {
        self.max_stream_sender = max_stream_sender;
        self
    }

    #[inline]
    pub fn get_connection_pool(&self) -> &ConnectionPool<K, HyperH2Connection<B>> {
        &self.pool
    }
}

impl<C, K: 'static, B: 'static> HyperH2Connector<C, K, B> {
    /// Creates a new HyperH2Connector with default settings and connection pooling.
    ///
    /// # Arguments
    /// * `connector` - The underlying connector to use for establishing connections.
    ///
    /// # Examples
    ///
    /// Creating an HTTP/2 connector (typically used over TLS):
    /// ```rust
    /// use bytes::Bytes;
    /// use http_body_util::Empty;
    /// use monoio_transports::connectors::{HyperH2Connector, PollIo, TcpConnector, TlsConnector};
    ///
    /// let tls_connector = TlsConnector::new(TcpConnector::default(), Default::default());
    /// let h2_connector: HyperH2Connector<_, _, Empty<Bytes>> =
    ///     HyperH2Connector::new(PollIo(tls_connector));
    /// ```
    ///
    /// Creating an HTTP/2 connector with custom settings:
    /// ```rust
    /// use bytes::Bytes;
    /// use http_body_util::Empty;
    /// use monoio_transports::connectors::{HyperH2Connector, PollIo, TcpConnector, TlsConnector};
    ///
    /// let tls_connector = TlsConnector::new(TcpConnector::default(), Default::default());
    /// let h2_connector: HyperH2Connector<_, _, Empty<Bytes>> =
    ///     HyperH2Connector::new(PollIo(tls_connector)).with_max_stream_sender(Some(100));
    /// ```
    #[inline]
    pub fn new(connector: C) -> Self {
        let mut builder = H2Builder::new(MonoioExecutor);
        builder.timer(MonoioTimer);
        Self {
            connector,
            connecting: UnsafeCell::new(HashMap::new()),
            pool: ConnectionPool::new(None),
            builder,
            max_stream_sender: None,
        }
    }

    #[inline]
    pub fn new_with_pool(connector: C, pool: ConnectionPool<K, HyperH2Connection<B>>) -> Self {
        let mut builder = H2Builder::new(MonoioExecutor);
        builder.timer(MonoioTimer);
        Self {
            connector,
            pool,
            connecting: UnsafeCell::new(HashMap::new()),
            builder,
            max_stream_sender: None,
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
    cnt: Option<Rc<usize>>,
}

impl<B> HyperH2Connection<B> {
    #[inline]
    pub fn to_owned(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            cnt: self.cnt.clone(),
        }
    }

    fn stream_full(&self) -> bool {
        let Some(rc) = &self.cnt else {
            return false;
        };
        Rc::strong_count(rc) >= *rc.as_ref()
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
        while let Some(mut pooled) = self.pool.get(&key) {
            if !pooled.tx.is_closed() {
                if pooled.tx.ready().await.is_err() {
                    continue;
                }
                return Ok(pooled);
            }
        }
        let underlying = self
            .connector
            .connect(key.clone())
            .await
            .map_err(HyperError::Connect)?;
        let (tx, conn) = self.builder.handshake::<_, B>(underlying).await?;
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
        macro_rules! try_get {
            ($pool:expr, $key:expr) => {
                if let Some(pooled) = $pool.and_then_mut($key, |mut conns| {
                    // remove invalid conn
                    conns.retain(|idle| idle.conn.is_ready());
                    // check count
                    for idle in conns.iter_mut() {
                        if idle.conn.stream_full() {
                            continue;
                        } else {
                            idle.reset_idle();
                            return Some(idle.conn.to_owned());
                        }
                    }
                    None
                }) {
                    return Ok(pooled);
                }
            };
        }

        try_get!(self.pool, &key);
        let lock = {
            let connecting = unsafe { &mut *self.connecting.get() };
            let lock = connecting
                .entry(key.clone())
                .or_insert_with(|| Rc::new(tokio::sync::Mutex::new(())));
            lock.clone()
        };

        // get lock and try again
        let _guard = lock.lock().await;
        try_get!(self.pool, &key);

        // create new h2 connection holding lock
        let underlying = self
            .connector
            .connect(key.clone())
            .await
            .map_err(HyperError::Connect)?;
        let (tx, conn) = self.builder.handshake::<_, B>(underlying).await?;
        monoio::spawn(conn);
        let cnt = self.max_stream_sender.map(Rc::new);
        let cn = HyperH2Connection { tx, cnt };
        self.pool.put(key, cn.to_owned());
        Ok(cn)
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
