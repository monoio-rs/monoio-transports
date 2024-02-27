use std::time::Duration;

use monoio::io::{AsyncReadRent, AsyncWriteRent, Split};
use monoio_http::h1::codec::ClientCodec;

use super::connection::HttpConnection;
use crate::{
    connectors::Connector,
    pool::{ConnectionPool, Key, Pooled},
};

/// H1 connector with optional connection pool.
// Note: here we don't use pooled::connector::PooledConnector.
// In the future, it is expected to implement h2 connector(which mainly based on
// ConnectionPool::map_ref) and unify the two connectors(not like hyper-util which merges at pool
// level).
pub struct H1Connector<C, K, IO: AsyncWriteRent> {
    inner_connector: C,
    pool: Option<ConnectionPool<K, HttpConnection<IO>>>,
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

impl<C, K, IO: AsyncWriteRent> H1Connector<C, K, IO> {
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
    pub fn pool(&mut self) -> &mut Option<ConnectionPool<K, HttpConnection<IO>>> {
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

impl<C: Default, K, IO: AsyncWriteRent> Default for H1Connector<C, K, IO> {
    #[inline]
    fn default() -> Self {
        H1Connector::new(C::default())
    }
}

impl<C, K: Key, IO: AsyncWriteRent> Connector<K> for H1Connector<C, K, IO>
where
    C: Connector<K, Connection = IO>,
    // TODO: Remove AsyncReadRent after monoio-http new version published.
    IO: AsyncReadRent + AsyncWriteRent + Split,
{
    type Connection = Pooled<K, HttpConnection<IO>>;
    type Error = C::Error;

    #[inline]
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
        let http_conn = HttpConnection::H1(client_codec, true);
        let pooled = if let Some(pool) = &self.pool {
            pool.link(key, http_conn)
        } else {
            Pooled::unpooled(http_conn)
        };
        Ok(pooled)
    }
}

#[cfg(test)]
mod tests {
    use std::net::ToSocketAddrs;

    use http::{request, Uri};
    use monoio_http::{common::body::HttpBody, h1::payload::Payload};

    use super::*;
    use crate::connectors::{TcpConnector, TcpTlsAddr, TlsConnector};

    #[monoio::test(enable_timer = true)]
    async fn test_http_connector() -> Result<(), crate::Error> {
        let connector = H1Connector {
            inner_connector: TcpConnector::default(),
            pool: None,
            read_timeout: None,
        }
        .with_default_pool();

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

        for i in 0..3 {
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
        }

        Ok(())
    }

    #[monoio::test(enable_timer = true)]
    async fn test_https_connector() -> Result<(), crate::Error> {
        let connector: H1Connector<TlsConnector<TcpConnector>, _, _> = H1Connector::default();
        let uri = "https://httpbin.org/get".parse::<Uri>().unwrap();
        let addr: TcpTlsAddr = uri.try_into().unwrap();
        let mut conn = connector.connect(addr).await.unwrap();
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
        Ok(())
    }
}
