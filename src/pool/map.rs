use crate::connectors::Connector;

pub trait ConnectorMapper<C, E> {
    type Connection;
    type Error;
    fn map(&self, inner: Result<C, E>) -> Result<Self::Connection, Self::Error>;
}

impl<C, E, MC, ME, M> ConnectorMapper<C, E> for M
where
    M: Fn(Result<C, E>) -> Result<MC, ME>,
{
    type Connection = MC;
    type Error = ME;
    #[inline]
    fn map(&self, inner: Result<C, E>) -> Result<MC, ME> {
        (self)(inner)
    }
}

impl<C, E> ConnectorMapper<C, E> for () {
    type Connection = C;
    type Error = E;

    #[inline]
    fn map(&self, inner: Result<C, E>) -> Result<Self::Connection, Self::Error> {
        inner
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectorMap<C, M> {
    pub inner: C,
    pub mapper: M,
}

impl<C, M> ConnectorMap<C, M> {
    #[inline]
    pub const fn new(inner: C, mapper: M) -> Self {
        Self { inner, mapper }
    }

    #[inline]
    pub fn with_mapper<M2>(self, mapper: M2) -> ConnectorMap<C, M2> {
        ConnectorMap {
            inner: self.inner,
            mapper,
        }
    }
}

impl<C: Default> Default for ConnectorMap<C, ()> {
    #[inline]
    fn default() -> Self {
        Self {
            inner: Default::default(),
            mapper: (),
        }
    }
}

impl<C> From<C> for ConnectorMap<C, ()> {
    #[inline]
    fn from(inner: C) -> Self {
        Self { inner, mapper: () }
    }
}

impl<K, C, M> Connector<K> for ConnectorMap<C, M>
where
    C: Connector<K>,
    M: ConnectorMapper<C::Connection, C::Error>,
{
    type Connection = M::Connection;
    type Error = M::Error;

    async fn connect(&self, key: K) -> Result<Self::Connection, Self::Error> {
        let inner = self.inner.connect(key).await;
        self.mapper.map(inner)
    }
}
