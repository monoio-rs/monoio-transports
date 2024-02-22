use super::{ConnectionPool, Key, Poolable, Pooled};
use crate::connectors::Connector;

/// PooledConnector is a connector with a connection pool.
/// It is designed for non-multiplex transport, like http1.
#[derive(Debug)]
pub struct PooledConnector<C, K, T> {
    transport_connector: C,
    pool: ConnectionPool<K, T>,
}

impl<C, K, T> PooledConnector<C, K, T> {
    #[inline]
    pub const fn new(transport_connector: C, pool: ConnectionPool<K, T>) -> Self {
        Self {
            transport_connector,
            pool,
        }
    }

    #[inline]
    pub fn into_parts(self) -> (C, ConnectionPool<K, T>) {
        (self.transport_connector, self.pool)
    }

    #[inline]
    pub fn transport_connector(&self) -> &C {
        &self.transport_connector
    }

    #[inline]
    pub fn pool(&self) -> &ConnectionPool<K, T> {
        &self.pool
    }
}

impl<C: Default, K: 'static, T: 'static> Default for PooledConnector<C, K, T> {
    #[inline]
    fn default() -> Self {
        Self::new(Default::default(), Default::default())
    }
}

impl<C: Clone, K, T> Clone for PooledConnector<C, K, T> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            transport_connector: self.transport_connector.clone(),
            pool: self.pool.clone(),
        }
    }
}

impl<C, K: 'static, T: 'static> PooledConnector<C, K, T> {
    #[inline]
    pub fn new_with_default_pool(transport_connector: C) -> Self {
        Self::new(transport_connector, Default::default())
    }
}

impl<C, K: Key, T: Poolable> Connector<K> for PooledConnector<C, K, T>
where
    C: Connector<K, Connection = T>,
{
    type Connection = Pooled<K, T>;
    type Error = C::Error;

    #[inline]
    async fn connect(&self, key: K) -> Result<Self::Connection, Self::Error> {
        if let Some(conn) = self.pool.get(&key) {
            return Ok(conn);
        }
        let key_owned = key.to_owned();
        let io = self.transport_connector.connect(key).await?;
        Ok(self.pool.link(key_owned, io))
    }
}
