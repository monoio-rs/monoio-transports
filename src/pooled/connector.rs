use std::{fmt::Display, hash::Hash, net::ToSocketAddrs};

use monoio::io::{AsyncReadRent, AsyncWriteRent, Split};
use monoio_codec::Framed;

use super::{connection::PooledConnection, pool::ConnectionPool};
use crate::connectors::Connector;

#[derive(Clone)]
pub struct PooledConnectorConfig {
    max_idle_conns: usize,
}

impl PooledConnectorConfig {
    pub fn max_idle_conns(mut self, max_conns: usize) -> Self {
        self.max_idle_conns = max_conns;
        self
    }
}

impl Default for PooledConnectorConfig {
    fn default() -> Self {
        Self { max_idle_conns: 32 }
    }
}

/// PooledConnector does 2 things:
/// 1. pool
/// 2. combine connection with codec(of cause with buffer)
pub struct PooledConnector<TC, K, IO: AsyncWriteRent, Codec> {
    config: PooledConnectorConfig,
    transport_connector: TC,
    pool: ConnectionPool<K, IO, Codec>,
}

impl<TC: Clone, K, IO: AsyncWriteRent, Codec> Clone for PooledConnector<TC, K, IO, Codec> {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            transport_connector: self.transport_connector.clone(),
            pool: self.pool.clone(),
        }
    }
}

impl<TC, K, IO: AsyncWriteRent, Codec> std::fmt::Debug for PooledConnector<TC, K, IO, Codec> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PooledConnector")
    }
}

impl<TC, K: 'static, IO: AsyncWriteRent + 'static, Codec: 'static> Default
    for PooledConnector<TC, K, IO, Codec>
where
    TC: Default,
{
    fn default() -> Self {
        Self::new(Default::default(), Default::default())
    }
}

impl<TC, K: 'static, IO: AsyncWriteRent + 'static, Codec: 'static>
    PooledConnector<TC, K, IO, Codec>
{
    pub fn new(config: PooledConnectorConfig, connector: TC) -> Self {
        Self {
            config,
            transport_connector: connector,
            pool: ConnectionPool::default(),
        }
    }

    pub fn with_connector(connector: TC) -> Self {
        Self::new(Default::default(), connector)
    }
}

impl<TC, K, IO, Codec> Connector<K> for PooledConnector<TC, K, IO, Codec>
where
    K: ToSocketAddrs + Hash + Eq + ToOwned<Owned = K> + Display + 'static,
    TC: Connector<K, Connection = IO>,
    IO: AsyncReadRent + AsyncWriteRent + Split + Unpin + 'static,
    crate::Error: From<<TC as Connector<K>>::Error>,
    Codec: Default,
{
    type Connection = PooledConnection<K, IO, Codec>;
    type Error = crate::Error;

    async fn connect(&self, key: K) -> Result<Self::Connection, Self::Error> {
        if let Some(conn) = self.pool.get(&key) {
            return Ok(conn);
        }
        let key_owned = key.to_owned();
        let io = self.transport_connector.connect(key).await?;
        Ok(self
            .pool
            .link(key_owned, Framed::new(io, Default::default())))
    }
}
