use std::ops::{Deref, DerefMut};

use super::Poolable;
use crate::connectors::Connector;

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct Reuse<T> {
    inner: T,
    reuse: bool,
}

impl<T> Reuse<T> {
    #[inline]
    pub const fn new(inner: T, reuse: bool) -> Self {
        Self { inner, reuse }
    }

    #[inline]
    pub fn into_inner(self) -> T {
        self.inner
    }

    #[inline]
    pub fn is_reused(&self) -> bool {
        self.reuse
    }

    #[inline]
    pub fn set_reuse(&mut self, reuse: bool) {
        self.reuse = reuse;
    }
}

impl<T> Poolable for Reuse<T> {
    #[inline]
    fn is_open(&self) -> bool {
        self.reuse
    }
}

impl<T> Deref for Reuse<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> DerefMut for Reuse<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T> AsRef<T> for Reuse<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        &self.inner
    }
}

impl<T> AsMut<T> for Reuse<T> {
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

pub struct ReuseConnector<C>(pub C);

impl<C, K> Connector<K> for ReuseConnector<C>
where
    C: Connector<K>,
{
    type Connection = Reuse<C::Connection>;
    type Error = C::Error;

    #[inline]
    async fn connect(&self, key: K) -> Result<Self::Connection, Self::Error> {
        Ok(Reuse::new(self.0.connect(key).await?, true))
    }
}
