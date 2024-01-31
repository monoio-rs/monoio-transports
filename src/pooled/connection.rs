use std::{
    collections::VecDeque,
    fmt::Display,
    hash::Hash,
    ops::{Deref, DerefMut},
    time::Instant,
};

use monoio::{
    buf::{IoBuf, IoBufMut, IoVecBuf, IoVecBufMut},
    io::{AsyncReadRent, AsyncWriteRent, Split},
    BufResult,
};
use monoio_codec::Framed;

use super::{IdleConnection, WeakConns};

pub struct PooledConnection<K: Hash + Eq + Display, IO: AsyncWriteRent, Codec> {
    // option is for take when drop
    key: Option<K>,
    conn: Option<Framed<IO, Codec>>,
    pool: WeakConns<K, Framed<IO, Codec>>,
    reusable: bool,
}

impl<K, IO, Codec> PooledConnection<K, IO, Codec>
where
    K: Hash + Eq + Display,
    IO: AsyncWriteRent + AsyncReadRent + Split,
{
    pub(crate) fn new(
        key: K,
        conn: Framed<IO, Codec>,
        pool: WeakConns<K, Framed<IO, Codec>>,
    ) -> Self {
        Self {
            key: Some(key),
            conn: Some(conn),
            pool,
            reusable: true,
        }
    }

    pub fn set_reusable(&mut self, reusable: bool) {
        self.reusable = reusable;
    }

    pub fn map_codec(&mut self, f: impl FnOnce(Codec) -> Codec) {
        let conn = self.conn.take().expect("unable to take connection");
        let conn = conn.map_codec(f);
        self.conn = Some(conn);
    }
}

impl<K: Hash + Eq + Display, IO: AsyncWriteRent + AsyncReadRent, Codec> Deref
    for PooledConnection<K, IO, Codec>
{
    type Target = Framed<IO, Codec>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { self.conn.as_ref().unwrap_unchecked() }
    }
}

impl<K: Hash + Eq + Display, IO: AsyncWriteRent + AsyncReadRent, Codec> DerefMut
    for PooledConnection<K, IO, Codec>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.conn.as_mut().unwrap_unchecked() }
    }
}

impl<K: Hash + Eq + Display, IO: AsyncWriteRent + AsyncReadRent, Codec> AsyncReadRent
    for PooledConnection<K, IO, Codec>
{
    async fn read<T: IoBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        self.deref_mut().read(buf).await
    }

    async fn readv<T: IoVecBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        self.deref_mut().readv(buf).await
    }
}

impl<K: Hash + Eq + Display, IO: AsyncWriteRent + AsyncReadRent, Codec> AsyncWriteRent
    for PooledConnection<K, IO, Codec>
{
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.deref_mut().write(buf).await
    }

    async fn writev<T: IoVecBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.deref_mut().writev(buf).await
    }

    async fn flush(&mut self) -> std::io::Result<()> {
        self.deref_mut().flush().await
    }

    async fn shutdown(&mut self) -> std::io::Result<()> {
        self.deref_mut().shutdown().await
    }
}

unsafe impl<K: Hash + Eq + Display, IO: AsyncWriteRent + AsyncReadRent, Codec> Split
    for PooledConnection<K, IO, Codec>
{
}

impl<K: Hash + Eq + Display, IO: AsyncWriteRent, Codec> Drop for PooledConnection<K, IO, Codec> {
    fn drop(&mut self) {
        if let Some(pool) = self.pool.upgrade() {
            let key = self.key.take().expect("unable to take key");
            let conn = self.conn.take().expect("unable to take connection");
            let idle = IdleConnection {
                conn,
                idle_at: Instant::now(),
            };

            let conns = unsafe { &mut *pool.get() };
            #[cfg(feature = "logging")]
            let key_str = key.to_string();

            if self.reusable {
                let queue = conns
                    .mapping
                    .entry(key)
                    .or_insert(VecDeque::with_capacity(conns.max_idle));

                #[cfg(feature = "logging")]
                tracing::debug!(
                    "connection pool size: {:?} for key: {:?}",
                    queue.len(),
                    key_str
                );

                if queue.len() > conns.max_idle {
                    #[cfg(feature = "logging")]
                    tracing::info!("connection pool is full for key: {:?}", key_str);
                    let _ = queue.pop_front();
                }

                queue.push_back(idle);
                #[cfg(feature = "logging")]
                tracing::debug!("connection recycled");
            }
        }
    }
}
