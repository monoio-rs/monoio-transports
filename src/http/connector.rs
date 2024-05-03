use std::{cell::UnsafeCell, collections::HashMap, rc::Rc, time::Duration};

use monoio::io::{AsyncReadRent, AsyncWriteRent, Split};
use monoio_http::{h1::codec::ClientCodec, h2::client::Builder as MonoioH2Builder};

use super::connection::{Http1Connection, Http2Connection, HttpConnection};
use crate::{
    connectors::{Connector, TlsConnector, TlsStream},
    pool::{ConnectionPool, Key, Pooled},
};
/// A connector for non-TLS HTTP/1.1 connections.
/// For TLS connections, use `HttpsConnector`, which
/// can be configured to use HTTP/1.1 or HTTP/2 with ALPN.
///  ['HttpsConnector']: HttpsConnector::http1_only(inner_connector)
pub struct H1Connector<C, K, IO: AsyncWriteRent> {
    inner_connector: C,
    pool: Option<ConnectionPool<K, Http1Connection<IO>>>,
    pub read_timeout: Option<Duration>,
}

impl<C: Clone, K, IO: AsyncWriteRent> Clone for H1Connector<C, K, IO> {
    fn clone(&self) -> Self {
        Self {
            inner_connector: self.inner_connector.clone(),
            pool: self.pool.clone(),
            read_timeout: self.read_timeout,
        }
    }
}

impl<C, K: 'static, IO: AsyncWriteRent + 'static> H1Connector<C, K, IO> {
    #[inline]
    pub const fn new(inner_connector: C) -> Self {
        Self {
            inner_connector,
            pool: None,
            read_timeout: None,
        }
    }

    #[inline]
    pub const fn new_with_timeout(inner_connector: C, timeout: Duration) -> Self {
        Self {
            inner_connector,
            pool: None,
            read_timeout: Some(timeout),
        }
    }

    #[inline]
    pub fn pool(&mut self) -> &mut Option<ConnectionPool<K, Http1Connection<IO>>> {
        &mut self.pool
    }

    #[inline]
    pub fn read_timeout(&mut self) -> &mut Option<Duration> {
        &mut self.read_timeout
    }
}

impl<C, K: 'static, IO: AsyncWriteRent + 'static> H1Connector<C, K, IO> {
    #[inline]
    pub fn with_default_pool(self) -> Self {
        Self {
            pool: Some(ConnectionPool::default()),
            ..self
        }
    }
}

impl<C: Default, K: 'static, IO: AsyncWriteRent + 'static> Default for H1Connector<C, K, IO> {
    #[inline]
    fn default() -> Self {
        H1Connector::new(C::default())
    }
}

/// A connector for non-TLS HTTP/2 connections.
/// For TLS connections, use `HttpsConnector`, which
/// can be configured to use HTTP/1.1 or HTTP/2 with ALPN
///  ['HttpsConnector']: HttpsConnector::http2_only(inner_connector)
pub struct H2Connector<C, K> {
    connector: C,
    connecting: UnsafeCell<HashMap<K, Rc<local_sync::semaphore::Semaphore>>>,
    pool: ConnectionPool<K, Http2Connection>,
    builder: MonoioH2Builder,
}

impl<C, K: 'static> H2Connector<C, K> {
    #[inline]
    pub fn new(connector: C) -> Self {
        Self {
            connector,
            connecting: UnsafeCell::new(HashMap::new()),
            pool: ConnectionPool::new(None),
            builder: MonoioH2Builder::default(),
        }
    }

    #[inline]
    pub fn builder(&mut self) -> &mut MonoioH2Builder {
        &mut self.builder
    }
}

/// A connector for TLS based HTTP/1.1 AND HTTP/2 connections,
/// which uses ALPN to determine the protocol. It can be configured
/// to use HTTP/1.1 or HTTP/2 only.
///  ['HttpsConnector']: HttpsConnector::default(inner_connector)
///  ['HttpsConnector']: HttpsConnector::http1_only(inner_connector)
///  ['HttpsConnector']: HttpsConnector::http2_only(inner_connector)
pub struct HttpsConnector<C, K, IO: AsyncWriteRent> {
    tls_connector: TlsConnector<C>,
    h1_pool: Option<ConnectionPool<K, Http1Connection<IO>>>,
    h2_pool: ConnectionPool<K, Http2Connection>,
    connecting: UnsafeCell<HashMap<K, Rc<local_sync::semaphore::Semaphore>>>,
    h2_builder: MonoioH2Builder,
}

impl<C, K: 'static, IO: AsyncWriteRent + 'static> HttpsConnector<C, K, IO> {
    #[inline]
    pub fn new(tls_connector: TlsConnector<C>) -> Self {
        Self {
            tls_connector,
            h1_pool: Some(ConnectionPool::default()),
            h2_pool: ConnectionPool::new(None),
            connecting: UnsafeCell::new(HashMap::new()),
            h2_builder: MonoioH2Builder::default(),
        }
    }
}

impl<C: Default, K: 'static, IO: AsyncWriteRent + 'static> HttpsConnector<C, K, IO> {
    #[inline]
    pub fn http1_only() -> Self {
        let alpn = vec![b"http/1.1".to_vec()];
        let tls_connector = TlsConnector::new_with_tls_default(C::default(), Some(alpn));
        Self {
            tls_connector,
            h1_pool: Some(ConnectionPool::default()),
            h2_pool: ConnectionPool::new(None),
            connecting: UnsafeCell::new(HashMap::new()),
            h2_builder: MonoioH2Builder::default(),
        }
    }

    #[inline]
    pub fn http2_only() -> Self {
        let alpn = vec![b"h2".to_vec()];
        let tls_connector = TlsConnector::new_with_tls_default(C::default(), Some(alpn));
        Self {
            tls_connector,
            h1_pool: Some(ConnectionPool::default()),
            h2_pool: ConnectionPool::new(None),
            connecting: UnsafeCell::new(HashMap::new()),
            h2_builder: MonoioH2Builder::default(),
        }
    }

    #[inline]
    pub fn h2_builder(&mut self) -> &mut MonoioH2Builder {
        &mut self.h2_builder
    }
}

impl<C: Default, K: 'static, IO: AsyncWriteRent + 'static> Default for HttpsConnector<C, K, IO> {
    #[inline]
    fn default() -> Self {
        let alpn = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        HttpsConnector::new(TlsConnector::new_with_tls_default(C::default(), Some(alpn)))
    }
}

impl<C, K, IO: AsyncWriteRent> Connector<K> for H1Connector<C, K, IO>
where
    C: Connector<K, Connection = IO>,
    K: Key,
    IO: AsyncWriteRent + Split,
{
    type Connection = Pooled<K, Http1Connection<IO>>;
    type Error = C::Error;

    async fn connect(&self, key: K) -> Result<Self::Connection, Self::Error> {
        if let Some(pool) = &self.pool {
            if let Some(conn) = pool.get(&key) {
                return Ok(conn);
            }
        }
        let io: IO = self.inner_connector.connect(key.clone()).await?;
        let client_codec = match self.read_timeout {
            Some(timeout) => ClientCodec::new_with_timeout(io, timeout),
            None => ClientCodec::new(io),
        };

        let http_conn = Http1Connection::new(client_codec);
        let pooled = if let Some(pool) = &self.pool {
            pool.link(key, http_conn)
        } else {
            Pooled::unpooled(http_conn)
        };
        Ok(pooled)
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

impl<C, K: Key, IO> Connector<K> for H2Connector<C, K>
where
    C: Connector<K, Connection = IO>,
    crate::TransportError: From<<C as Connector<K>>::Error>,
    IO: AsyncReadRent + AsyncWriteRent + Unpin + 'static,
{
    type Connection = Http2Connection;
    type Error = crate::TransportError;

    async fn connect(&self, key: K) -> Result<Self::Connection, Self::Error> {
        if let Some(conn) = try_get!(self, pool, key) {
            return Ok(conn);
        }

        let lock = {
            let connecting = unsafe { &mut *self.connecting.get() };
            let lock = connecting
                .entry(key.clone())
                .or_insert_with(|| Rc::new(local_sync::semaphore::Semaphore::new(1)));
            lock.clone()
        };

        // get semaphore and try again
        let _guard = lock.acquire().await?;
        if let Some(conn) = try_get!(self, pool, key) {
            return Ok(conn);
        }

        // create new h2 connection
        let io = self.connector.connect(key.clone()).await?;

        let (tx, conn) = self.builder.handshake(io).await?;
        monoio::spawn(conn);
        self.pool.put(key, Http2Connection::new(tx.clone()));
        Ok(Http2Connection::new(tx.clone()))
    }
}

impl<C, K: Key, IO> Connector<K> for HttpsConnector<C, K, TlsStream<IO>>
where
    TlsConnector<C>: Connector<K, Connection = TlsStream<IO>, Error = crate::connectors::TlsError>,
    IO: AsyncReadRent + AsyncWriteRent + Split + Unpin + 'static,
{
    type Connection = HttpConnection<K, TlsStream<IO>>;
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
        let tls_stream = self.tls_connector.connect(key.clone()).await?;
        let alpn_protocol = tls_stream.alpn_protocol();
        let is_h2 = alpn_protocol.map_or(false, |proto| proto == b"h2");

        if is_h2 {
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

            let (tx, conn) = self.h2_builder.handshake(tls_stream).await?;
            monoio::spawn(conn);
            self.h2_pool.put(key, Http2Connection::new(tx.clone()));
            Ok(Http2Connection::new(tx.clone()).into())
        } else {
            let client_codec = ClientCodec::new(tls_stream);
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
        let connector: HttpsConnector<TcpConnector, _, _> = HttpsConnector::default();
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
        let connector: HttpsConnector<TcpConnector, _, _> = HttpsConnector::http2_only();

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
        let connector: HttpsConnector<TcpConnector, _, _> = HttpsConnector::http1_only();
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
        let connector: H1Connector<TcpConnector, _, _> = H1Connector::default().with_default_pool();

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

        for i in 0..10 {
            let uri = "http://httpbin.org/get".parse::<Uri>().unwrap();
            let host = uri.host().unwrap();
            let port = uri.port_u16().unwrap_or(80);
            let key = Key {
                host: host.to_string(),
                port,
            };
            let mut conn = connector.connect(key).await.unwrap();
            assert!((i == 0) ^ conn.is_reused());

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
