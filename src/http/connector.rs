use std::{cell::UnsafeCell, collections::HashMap, rc::Rc};

use monoio::io::{AsyncReadRent, AsyncWriteRent, Split};
use monoio_http::{h1::codec::ClientCodec, h2::client::Builder as MonoioH2Builder};

use super::connection::{Http1Connection, Http2Connection, HttpConnection};
use crate::{
    connectors::{Connector, TcpConnector, TlsConnector, TransportConnMeta, TransportConnMetadata},
    pool::{ConnectionPool, Key, Pooled},
};

#[derive(Debug, Default, Copy, Clone)]
enum Protocol {
    HTTP2,
    HTTP11,
    #[default]
    Auto,
}

/// `HttpConnector` is a universal connector supporting both HTTP/1.1 and HTTP/2 protocols.
/// It can be used with a `TlsConnector` for HTTPS connections.
///
/// ## Protocol Selection
/// The protocol used depends on the context:
///
/// - When used with a `TlsConnector`, the protocol is determined by the ALPN negotiation. The
///   default `TlsConnector` sets the client's ALPN advertisement to `h2` and `http/1.1`.
///
/// - For plain text HTTP, the default protocol is HTTP/1.1 unless the user specifically sets the
///   client to a particular protocol.
///
/// | Connector Type | Protocol | Method | Example | Description |
/// | --- | --- | --- | --- | --- |
/// | `TcpConnector` | HTTP/1.1 | `build_tcp_http1_only()` | ```rust<br>let connector = HttpConnector::build_tcp_http1_only();<br>``` | Creates an `HttpConnector` that only supports HTTP/1.1 over TCP. |
/// | `TcpConnector` | HTTP/2 | `build_tcp_http2_only()` | ```rust<br>let connector = HttpConnector::build_tcp_http2_only();<br>``` | Creates an `HttpConnector` that only supports HTTP/2 over TCP. |
/// | `TlsConnector` | HTTP/1.1 and HTTP/2 | `default()` | ```rust<br>let connector: HttpConnector<TlsConnector<TcpConnector>, _, _> = HttpConnector::default();<br>``` | Creates a `HttpConnector` that supports both HTTP/1.1 and HTTP/2. |
/// | `TlsConnector` | HTTP/1.1 | `build_tls_http1_only()` | ```rust<br>let connector = HttpConnector::build_tls_http1_only();<br>``` | Creates an `HttpConnector` with a `TlsConnector` that only supports HTTP/1.1. |
/// | `TlsConnector` | HTTP/2 | `build_tls_http2_only()` | ```rust<br>let connector = HttpConnector::build_tls_http2_only();<br>``` | Creates an `HttpConnector` with a `TlsConnector` that only supports HTTP/2. |
pub struct HttpConnector<C, K, IO: AsyncWriteRent> {
    connector: C,
    protocol: Protocol, // User configured protocol
    h1_pool: Option<ConnectionPool<K, Http1Connection<IO>>>,
    h2_pool: ConnectionPool<K, Http2Connection>,
    connecting: UnsafeCell<HashMap<K, Rc<local_sync::semaphore::Semaphore>>>,
    h2_builder: MonoioH2Builder,
}

impl<C, K: 'static, IO: AsyncWriteRent + 'static> HttpConnector<C, K, IO> {
    #[inline]
    pub fn new(connector: C) -> Self {
        Self {
            connector,
            protocol: Protocol::default(),
            h1_pool: Some(ConnectionPool::default()),
            h2_pool: ConnectionPool::new(None),
            connecting: UnsafeCell::new(HashMap::new()),
            h2_builder: MonoioH2Builder::default(),
        }
    }

    /// Sets the protocol of the `HttpConnector` to HTTP/1.1 only.
    ///
    /// This method should be used with non-TLS connectors like `TcpConnector`, `UdsConnector`, etc.
    /// For TLS connectors, use `build_tls_http1_only` instead to set the correct ALPN.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut connector = HttpConnector::new(TcpConnector::new());
    /// connector.set_http1_only();
    /// ```
    pub fn set_http1_only(&mut self) {
        self.protocol = Protocol::HTTP11
    }

    /// Sets the protocol of the `HttpConnector` to HTTP/2 only.
    ///
    /// This method should be used with non-TLS connectors like `TcpConnector`, `UdsConnector`, etc.
    /// For TLS connectors, use `build_tls_http2_only` instead to set the correct ALPN.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut connector = HttpConnector::new(TcpConnector::new());
    /// connector.set_http2_only();
    /// ```
    pub fn set_http2_only(&mut self) {
        self.protocol = Protocol::HTTP2
    }

    #[inline]
    pub fn h2_builder(&mut self) -> &mut MonoioH2Builder {
        &mut self.h2_builder
    }

    fn is_config_h2(&self) -> bool {
        matches!(self.protocol, Protocol::HTTP2)
    }
}

impl<K: 'static, IO: AsyncWriteRent + 'static> HttpConnector<TcpConnector, K, IO> {
    /// Builds a new `HttpConnector` with a `TcpConnector` that supports only HTTP/1.1.
    ///
    /// This method sets the protocol of the `HttpConnector` to HTTP/1.1 only.
    ///
    /// # Examples
    ///
    /// ```
    /// let connector = HttpConnector::build_tcp_http1_only();
    /// ```
    pub fn build_tcp_http1_only() -> Self {
        Self {
            connector: TcpConnector::default(),
            protocol: Protocol::HTTP11,
            h1_pool: Some(ConnectionPool::default()),
            h2_pool: ConnectionPool::new(None),
            connecting: UnsafeCell::new(HashMap::new()),
            h2_builder: MonoioH2Builder::default(),
        }
    }

    /// Builds a new `HttpConnector` with a `TcpConnector` that supports only HTTP/2.
    ///
    /// This method sets the protocol of the `HttpConnector` to HTTP/2 only.
    ///
    /// # Examples
    ///
    /// ```
    /// let connector = HttpConnector::build_tcp_http2_only();
    /// ```
    pub fn build_tcp_http2_only() -> Self {
        Self {
            connector: TcpConnector::default(),
            protocol: Protocol::HTTP2,
            h1_pool: Some(ConnectionPool::default()),
            h2_pool: ConnectionPool::new(None),
            connecting: UnsafeCell::new(HashMap::new()),
            h2_builder: MonoioH2Builder::default(),
        }
    }
}

impl<C: Default, K: 'static, IO: AsyncWriteRent + 'static> HttpConnector<TlsConnector<C>, K, IO> {
    /// Builds a new `HttpConnector` with a `TlsConnector` that supports only HTTP/1.1.
    ///
    /// This method sets the client's ALPN advertisement to `http/1.1`.
    ///
    /// # Examples
    ///
    /// ```
    /// let connector = HttpConnector::build_tls_http1_only();
    /// ```
    pub fn build_tls_http1_only() -> Self {
        let alpn = vec![b"http/1.1".to_vec()];
        let tls_connector = TlsConnector::new_with_tls_default(C::default(), Some(alpn));
        Self {
            connector: tls_connector,
            protocol: Protocol::default(),
            h1_pool: Some(ConnectionPool::default()),
            h2_pool: ConnectionPool::new(None),
            connecting: UnsafeCell::new(HashMap::new()),
            h2_builder: MonoioH2Builder::default(),
        }
    }

    /// Builds a new `HttpConnector` with a `TlsConnector` that supports only HTTP/2.
    ///
    /// This method sets the client's ALPN advertisement to `h2`.
    ///
    /// # Examples
    ///
    /// ```
    /// let connector = HttpConnector::build_tls_http2_only();
    /// ```
    pub fn build_tls_http2_only() -> Self {
        let alpn = vec![b"h2".to_vec()];
        let tls_connector = TlsConnector::new_with_tls_default(C::default(), Some(alpn));
        Self {
            connector: tls_connector,
            protocol: Protocol::default(),
            h1_pool: Some(ConnectionPool::default()),
            h2_pool: ConnectionPool::new(None),
            connecting: UnsafeCell::new(HashMap::new()),
            h2_builder: MonoioH2Builder::default(),
        }
    }
}

impl<C: Default, K: 'static, IO: AsyncWriteRent + 'static> Default for HttpConnector<C, K, IO> {
    #[inline]
    fn default() -> Self {
        HttpConnector::new(C::default())
    }
}

macro_rules! try_get {
    ($self:ident, $pool:ident, $key:ident) => {
        $self.$pool.and_then_mut(&$key, |conns| {
            conns.retain(|idle| {
                // Remove any connections that have errored
                match idle.conn.conn_error() {
                    Some(_e) => {
                        println!("Removing connection");
                        #[cfg(feature = "logging")]
                        tracing::debug!("Removing invalid connection: {:?}", _e);
                        false
                    }
                    None => true,
                }
            });

            conns.front().map(|idle| idle.conn.to_owned())
        })
    };
}

impl<C, K: Key, IO> Connector<K> for HttpConnector<C, K, IO>
where
    C: Connector<K, Connection = IO>,
    C::Connection: TransportConnMetadata<Metadata = TransportConnMeta>,
    crate::TransportError: From<C::Error>,
    IO: AsyncReadRent + AsyncWriteRent + Split + Unpin + 'static,
{
    type Connection = HttpConnection<K, IO>;
    type Error = crate::TransportError;

    async fn connect(&self, key: K) -> Result<Self::Connection, Self::Error> {
        if let Some(conn) = try_get!(self, h2_pool, key) {
            return Ok(conn.into());
        }

        if let Some(h1_pool) = &self.h1_pool {
            if let Some(h1_pooled) = h1_pool.get(&key) {
                return Ok(h1_pooled.into());
            }
        }

        // We use ALPN to determine if connector should use HTTP/2 codecs or HTTP/1.1
        let transport_conn = self.connector.connect(key.clone()).await?;
        let conn_meta = transport_conn.get_conn_metadata();

        let connect_to_h2 = self.is_config_h2() || conn_meta.is_alpn_h2();

        if connect_to_h2 {
            let lock = {
                let connecting = unsafe { &mut *self.connecting.get() };
                let lock = connecting
                    .entry(key.clone())
                    .or_insert_with(|| Rc::new(local_sync::semaphore::Semaphore::new(1)));
                lock.clone()
            };

            // get lock and try again
            let _guard = lock.acquire().await?;
            if let Some(conn) = try_get!(self, h2_pool, key) {
                return Ok(conn.into());
            }

            let (tx, conn) = self.h2_builder.handshake(transport_conn).await?;
            monoio::spawn(conn);
            self.h2_pool.put(key, Http2Connection::new(tx.clone()));
            Ok(Http2Connection::new(tx.clone()).into())
        } else {
            let client_codec = ClientCodec::new(transport_conn);
            let http_conn = Http1Connection::new(client_codec);
            let pooled = if let Some(pool) = &self.h1_pool {
                pool.link(key, http_conn)
            } else {
                Pooled::unpooled(http_conn)
            };
            Ok(pooled.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::ToSocketAddrs;

    use http::{request, Uri};
    use monoio_http::{common::body::HttpBody, h1::payload::Payload};

    use super::*;
    use crate::connectors::{TcpConnector, TcpTlsAddr};

    #[monoio::test(enable_timer = true)]
    async fn test_default_https_connector() -> Result<(), crate::TransportError> {
        let connector: HttpConnector<TlsConnector<TcpConnector>, _, _> = HttpConnector::default();

        let uri = "https://httpbin.org/get".parse::<Uri>().unwrap();
        let addr: TcpTlsAddr = uri.try_into().unwrap();
        let mut conn = connector.connect(addr).await.unwrap();

        for _ in 0..10 {
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
        Ok(())
    }

    #[monoio::test(enable_timer = true)]
    async fn test_http2_tls_connector() -> Result<(), crate::TransportError> {
        let connector: HttpConnector<TlsConnector<TcpConnector>, _, _> =
            HttpConnector::build_tls_http2_only();

        let uri = "https://httpbin.org/get".parse::<Uri>().unwrap();
        let addr: TcpTlsAddr = uri.try_into().unwrap();
        let mut conn = connector.connect(addr).await.unwrap();

        for _ in 0..10 {
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
        Ok(())
    }

    #[monoio::test(enable_timer = true)]
    async fn test_http1_tls_connector() -> Result<(), crate::TransportError> {
        let connector: HttpConnector<TlsConnector<TcpConnector>, _, _> =
            HttpConnector::build_tls_http1_only();

        let uri = "https://httpbin.org/get".parse::<Uri>().unwrap();
        let addr: TcpTlsAddr = uri.try_into().unwrap();
        let mut conn = connector.connect(addr).await.unwrap();

        for _ in 0..10 {
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

    #[monoio::test(enable_timer = true)]
    async fn test_http1_tcp_connector() -> Result<(), crate::TransportError> {
        let connector: HttpConnector<TcpConnector, _, _> = HttpConnector::default();

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

        for _i in 0..10 {
            let uri = "http://httpbin.org/get".parse::<Uri>().unwrap();
            let host = uri.host().unwrap();
            let port = uri.port_u16().unwrap_or(80);
            let key = Key {
                host: host.to_string(),
                port,
            };
            let mut conn = connector.connect(key).await.unwrap();
            // assert!((i == 0) ^ conn.is_reused());

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
    // See http_with_tcp for plain text HTTP/2 example
}
