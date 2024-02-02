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

pub struct PooledConnection<K: Hash + Eq + Display, IO: AsyncWriteRent> {
    // option is for take when drop
    key: Option<K>,
    conn: Option<Framed<IO, ()>>,
    pool: WeakConns<K, Framed<IO, ()>>,
    reusable: bool,
}

impl<K, IO> PooledConnection<K, IO>
where
    K: Hash + Eq + Display,
    IO: AsyncWriteRent + AsyncReadRent + Split,
{
    pub(crate) fn new(key: K, conn: Framed<IO, ()>, pool: WeakConns<K, Framed<IO, ()>>) -> Self {
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

    pub fn map_codec<'a, NewCodec>(
        &'a mut self,
        codec: NewCodec,
    ) -> PooledConnectionWithCodec<'a, K, IO, NewCodec> {
        let conn = self.conn.take().expect("unable to take connection");
        let conn = conn.map_codec(|_| codec);
        PooledConnectionWithCodec::new(self, conn)
    }
}

impl<K: Hash + Eq + Display, IO: AsyncWriteRent + AsyncReadRent> Deref for PooledConnection<K, IO> {
    type Target = Framed<IO, ()>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { self.conn.as_ref().unwrap_unchecked() }
    }
}

impl<K: Hash + Eq + Display, IO: AsyncWriteRent + AsyncReadRent> DerefMut
    for PooledConnection<K, IO>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.conn.as_mut().unwrap_unchecked() }
    }
}

impl<K: Hash + Eq + Display, IO: AsyncWriteRent + AsyncReadRent> AsyncReadRent
    for PooledConnection<K, IO>
{
    async fn read<T: IoBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        self.deref_mut().read(buf).await
    }

    async fn readv<T: IoVecBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        self.deref_mut().readv(buf).await
    }
}

impl<K: Hash + Eq + Display, IO: AsyncWriteRent + AsyncReadRent> AsyncWriteRent
    for PooledConnection<K, IO>
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

unsafe impl<K: Hash + Eq + Display, IO: AsyncWriteRent + AsyncReadRent> Split
    for PooledConnection<K, IO>
{
}

impl<K: Hash + Eq + Display, IO: AsyncWriteRent> Drop for PooledConnection<K, IO> {
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

pub struct PooledConnectionWithCodec<'a, K: Hash + Eq + Display, IO: AsyncWriteRent, Codec> {
    pub pooled_conn: &'a mut PooledConnection<K, IO>,
    pub conn: Option<Framed<IO, Codec>>,
}

impl<'a, K: Hash + Eq + Display, IO: AsyncWriteRent, Codec>
    PooledConnectionWithCodec<'a, K, IO, Codec>
{
    pub fn new(
        pooled_conn: &'a mut PooledConnection<K, IO>,
        conn: Framed<IO, Codec>,
    ) -> PooledConnectionWithCodec<'a, K, IO, Codec> {
        PooledConnectionWithCodec {
            pooled_conn: pooled_conn,
            conn: Some(conn),
        }
    }
}

impl<'a, K: Hash + Eq + Display, IO: AsyncWriteRent, Codec> Drop
    for PooledConnectionWithCodec<'a, K, IO, Codec>
{
    fn drop(&mut self) {
        let conn = self.conn.take().expect("unable to take connection");
        let conn = conn.map_codec(|_| ()); // erase codec when the PooledConnectionWithCodec is dropped
        self.pooled_conn.conn = Some(conn);
    }
}

impl<'a, K: Hash + Eq + Display, IO: AsyncWriteRent + AsyncReadRent, Codec> Deref
    for PooledConnectionWithCodec<'a, K, IO, Codec>
{
    type Target = Framed<IO, Codec>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { self.conn.as_ref().unwrap_unchecked() }
    }
}

impl<'a, K: Hash + Eq + Display, IO: AsyncWriteRent + AsyncReadRent, Codec> DerefMut
    for PooledConnectionWithCodec<'a, K, IO, Codec>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.conn.as_mut().unwrap_unchecked() }
    }
}
