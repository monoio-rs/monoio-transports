use std::ops::{Deref, DerefMut};

use monoio::io::{AsyncReadRent, AsyncWriteRent, Split};

use super::Poolable;
use crate::connectors::Connector;

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct Reuse<T> {
    inner: T,
    reuse: bool,
}

unsafe impl<T: Split> Split for Reuse<T> {}

impl<I: AsyncReadRent> AsyncReadRent for Reuse<I> {
    #[inline]
    fn read<T: monoio::buf::IoBufMut>(
        &mut self,
        buf: T,
    ) -> impl std::future::Future<Output = monoio::BufResult<usize, T>> {
        self.inner.read(buf)
    }

    #[inline]
    fn readv<T: monoio::buf::IoVecBufMut>(
        &mut self,
        buf: T,
    ) -> impl std::future::Future<Output = monoio::BufResult<usize, T>> {
        self.inner.readv(buf)
    }
}

impl<I: AsyncWriteRent> AsyncWriteRent for Reuse<I> {
    #[inline]
    fn write<T: monoio::buf::IoBuf>(
        &mut self,
        buf: T,
    ) -> impl std::future::Future<Output = monoio::BufResult<usize, T>> {
        self.inner.write(buf)
    }

    #[inline]
    fn writev<T: monoio::buf::IoVecBuf>(
        &mut self,
        buf_vec: T,
    ) -> impl std::future::Future<Output = monoio::BufResult<usize, T>> {
        self.inner.writev(buf_vec)
    }

    #[inline]
    fn flush(&mut self) -> impl std::future::Future<Output = std::io::Result<()>> {
        self.inner.flush()
    }

    #[inline]
    fn shutdown(&mut self) -> impl std::future::Future<Output = std::io::Result<()>> {
        self.inner.shutdown()
    }
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

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
