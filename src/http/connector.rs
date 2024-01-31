use monoio::io::{AsyncReadRent, AsyncWriteRent, Split};
use monoio_http::h1::codec::ClientCodec;

use super::connection::HttpConnection;
use crate::connectors::Connector;

#[derive(Clone)]
pub struct HttpConnector<C> {
    inner_connector: C,
}

impl<C> HttpConnector<C> {
    pub fn new(inner_connector: C) -> Self {
        Self { inner_connector }
    }
}

impl<C: Default> Default for HttpConnector<C> {
    fn default() -> Self {
        HttpConnector::new(C::default())
    }
}

impl<T, C> Connector<T> for HttpConnector<C>
where
    C: Connector<T>,
    C::Connection: AsyncReadRent + AsyncWriteRent + Split,
    C::Error: Into<crate::Error>,
{
    type Connection = HttpConnection<C::Connection>;
    type Error = crate::Error;

    async fn connect(&self, key: T) -> Result<Self::Connection, Self::Error> {
        let io: <C as Connector<T>>::Connection = self
            .inner_connector
            .connect(key)
            .await
            .map_err(|e| e.into())?;
        Ok(HttpConnection::H1(ClientCodec::new(io)))
    }
}

#[cfg(test)]
mod tests {

    use http::{request, Uri};
    use monoio::net::TcpStream;
    use monoio_http::{common::body::HttpBody, h1::payload::Payload};

    use super::*;
    use crate::{
        connectors::{TcpConnector, TlsConnector, TlsStream},
        key::Key,
        pooled::connector::PooledConnector,
    };

    #[monoio::test]
    async fn test_http_connector() -> Result<(), crate::Error> {
        let connector = HttpConnector {
            inner_connector: TcpConnector::default(),
        };
        let uri = "http://httpbin.org/get".parse::<Uri>().unwrap();
        let key: Key = uri.try_into().unwrap();
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
        Ok(())
    }

    #[monoio::test]
    async fn test_https_connector() -> Result<(), crate::Error> {
        let connector: HttpConnector<TlsConnector<TcpConnector>> = HttpConnector {
            inner_connector: TlsConnector::default(),
        };
        let uri = "https://httpbin.org/get".parse::<Uri>().unwrap();
        let key: Key = uri.try_into().unwrap();
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
        Ok(())
    }

    #[monoio::test(enable_timer = true)]
    async fn test_pooled_http_connector() -> Result<(), crate::Error> {
        let pooled_connector: PooledConnector<TcpConnector, Key, TcpStream> =
            PooledConnector::with_connector(TcpConnector::default());
        let connector = HttpConnector {
            inner_connector: pooled_connector,
        };
        let uri = "http://httpbin.org/get".parse::<Uri>().unwrap();
        let key: Key = uri.try_into().unwrap();
        let mut conn = connector.connect(key.clone()).await.unwrap();
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
        let req = request::Builder::new()
            .uri("/get")
            .header("Host", "httpbin.org")
            .body(HttpBody::H1(Payload::None))
            .unwrap();
        drop(conn);
        let mut conn = connector.connect(key).await.unwrap();
        let (res, _) = conn.send_request(req).await;
        let resp = res?;
        assert_eq!(200, resp.status());
        assert_eq!(
            "application/json".as_bytes(),
            resp.headers().get("content-type").unwrap().as_bytes()
        );
        Ok(())
    }

    #[monoio::test(enable_timer = true)]
    async fn test_pooled_https_connector() -> Result<(), crate::Error> {
        let pooled_connector: PooledConnector<
            TlsConnector<TcpConnector>,
            Key,
            TlsStream<TcpStream>,
        > = PooledConnector::with_connector(TlsConnector::with_connector(TcpConnector::default()));
        let connector = HttpConnector {
            inner_connector: pooled_connector,
        };
        let uri = "https://httpbin.org/get".parse::<Uri>().unwrap();
        let key: Key = uri.try_into().unwrap();
        let mut conn = connector.connect(key.clone()).await.unwrap();
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
        let req = request::Builder::new()
            .uri("/get")
            .header("Host", "httpbin.org")
            .body(HttpBody::H1(Payload::None))
            .unwrap();
        drop(conn);
        let mut conn = connector.connect(key).await.unwrap();
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
