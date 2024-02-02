use std::time::Duration;
use monoio::io::{AsyncReadRent, AsyncWriteRent, Split};
use monoio_http::h1::codec::ClientCodec;

use super::connection::HttpConnection;
use crate::connectors::Connector;

#[derive(Clone)]
pub struct HttpConnector<C> {
    inner_connector: C,
    pub timeout: Option<Duration>,
}

impl<C> HttpConnector<C> {
    pub fn new(inner_connector: C, timeout: Option<Duration>) -> Self {
        Self { inner_connector, timeout }
    }
}

impl<C: Default> Default for HttpConnector<C> {
    fn default() -> Self {
        HttpConnector::new(C::default(), None)
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
        Ok(HttpConnection::H1(ClientCodec::new(io, self.timeout)))
    }
}
