//! Provides connection pooling functionality for efficient connection reuse.
//!
//! This module includes the `ConnectionPool` for managing connections,
//! `Pooled` for representing pooled connections, and related traits and types
//! for implementing and interacting with connection pools.
mod connector;
mod map;
mod reuse;
use std::{
    cell::UnsafeCell,
    collections::{HashMap, VecDeque},
    fmt::Debug,
    hash::Hash,
    ops::{Deref, DerefMut},
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

pub use connector::PooledConnector;
pub use map::{ConnectorMap, ConnectorMapper};
use monoio::io::{AsyncReadRent, AsyncWriteRent, Split};
pub use reuse::{Reuse, ReuseConnector};

pub(crate) const DEFAULT_KEEPALIVE_CONNS: usize = 256;
pub(crate) const DEFAULT_POOL_SIZE: usize = 32;
// https://datatracker.ietf.org/doc/html/rfc6335
pub(crate) const MAX_KEEPALIVE_CONNS: usize = 16384;

pub trait Poolable {
    fn is_open(&self) -> bool;
}

type SharedPool<K, IO> = Rc<UnsafeCell<PoolInner<K, IO>>>;
type WeakPool<K, IO> = Weak<UnsafeCell<PoolInner<K, IO>>>;

pub trait Key: Eq + Hash + Clone + 'static {}
impl<T: Eq + Hash + Clone + 'static> Key for T {}

// Partly borrow from hyper-util. All rights reserved.
pub struct Pooled<K: Key, T: Poolable> {
    value: Option<T>,
    is_reused: bool,
    key: Option<K>,
    pool: Option<WeakPool<K, T>>,
}

unsafe impl<K: Key, T: Poolable + Split> Split for Pooled<K, T> {}

impl<K: Key, I: Poolable + AsyncReadRent> AsyncReadRent for Pooled<K, I> {
    #[inline]
    fn read<T: monoio::buf::IoBufMut>(
        &mut self,
        buf: T,
    ) -> impl std::future::Future<Output = monoio::BufResult<usize, T>> {
        unsafe { self.value.as_mut().unwrap_unchecked() }.read(buf)
    }

    #[inline]
    fn readv<T: monoio::buf::IoVecBufMut>(
        &mut self,
        buf: T,
    ) -> impl std::future::Future<Output = monoio::BufResult<usize, T>> {
        unsafe { self.value.as_mut().unwrap_unchecked() }.readv(buf)
    }
}

impl<K: Key, I: Poolable + AsyncWriteRent> AsyncWriteRent for Pooled<K, I> {
    #[inline]
    fn write<T: monoio::buf::IoBuf>(
        &mut self,
        buf: T,
    ) -> impl std::future::Future<Output = monoio::BufResult<usize, T>> {
        unsafe { self.value.as_mut().unwrap_unchecked() }.write(buf)
    }

    #[inline]
    fn writev<T: monoio::buf::IoVecBuf>(
        &mut self,
        buf_vec: T,
    ) -> impl std::future::Future<Output = monoio::BufResult<usize, T>> {
        unsafe { self.value.as_mut().unwrap_unchecked() }.writev(buf_vec)
    }

    #[inline]
    fn flush(&mut self) -> impl std::future::Future<Output = std::io::Result<()>> {
        unsafe { self.value.as_mut().unwrap_unchecked() }.flush()
    }

    #[inline]
    fn shutdown(&mut self) -> impl std::future::Future<Output = std::io::Result<()>> {
        unsafe { self.value.as_mut().unwrap_unchecked() }.shutdown()
    }
}

impl<T: Poolable, K: Key> Pooled<K, T> {
    #[inline]
    pub(crate) const fn new(key: K, value: T, is_reused: bool, pool: WeakPool<K, T>) -> Self {
        Self {
            value: Some(value),
            is_reused,
            key: Some(key),
            pool: Some(pool),
        }
    }

    #[inline]
    pub(crate) const fn unpooled(value: T) -> Self {
        Self {
            value: Some(value),
            is_reused: false,
            key: None,
            pool: None,
        }
    }

    #[inline]
    pub fn is_reused(&self) -> bool {
        self.is_reused
    }

    #[inline]
    fn as_ref(&self) -> &T {
        self.value.as_ref().expect("not dropped")
    }

    #[inline]
    fn as_mut(&mut self) -> &mut T {
        self.value.as_mut().expect("not dropped")
    }
}

impl<T: Poolable, K: Key> Deref for Pooled<K, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.as_ref()
    }
}

impl<T: Poolable, K: Key> DerefMut for Pooled<K, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        self.as_mut()
    }
}

impl<T: Poolable, K: Key> Drop for Pooled<K, T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            if !value.is_open() {
                // If we *already* know the connection is done here,
                // it shouldn't be re-inserted back into the pool.
                return;
            }

            if let Some(weak) = &self.pool {
                if let Some(pool) = weak.upgrade() {
                    let pool = unsafe { &mut *pool.get() };
                    let key = self.key.take().expect("key is not empty");
                    let queue = pool
                        .idle_conns
                        .entry(key)
                        .or_insert(VecDeque::with_capacity(pool.max_idle));
                    if queue.len() >= pool.max_idle {
                        for _ in 0..queue.len() - pool.max_idle {
                            let _ = queue.pop_front();
                        }
                        let _ = queue.pop_front();
                    }

                    let idle = Idle::new(value);
                    queue.push_back(idle);
                }
            }
        }
    }
}

pub(crate) struct Idle<IO> {
    pub(crate) conn: IO,
    idle_at: Instant,
}

impl<IO> Idle<IO> {
    #[inline]
    pub(crate) fn new(io: IO) -> Self {
        Self {
            conn: io,
            idle_at: Instant::now(),
        }
    }

    #[allow(unused)]
    #[inline]
    pub(crate) fn expired(&self, max_elapsed: Duration) -> bool {
        self.idle_at.elapsed() > max_elapsed
    }

    #[allow(unused)]
    #[inline]
    pub(crate) fn expired_opt(&self, max_elapsed: Option<Duration>) -> bool {
        match max_elapsed {
            Some(e) => self.idle_at.elapsed() > e,
            None => false,
        }
    }

    #[allow(unused)]
    #[inline]
    pub(crate) fn reset_idle(&mut self) {
        self.idle_at = Instant::now()
    }
}

pub(crate) struct PoolInner<K, IO> {
    idle_conns: HashMap<K, VecDeque<Idle<IO>>>,
    max_idle: usize,
    #[cfg(feature = "time")]
    idle_dur: Option<Duration>,
    #[cfg(feature = "time")]
    _drop: Option<local_sync::oneshot::Receiver<()>>,
}

impl<K, IO> PoolInner<K, IO> {
    #[cfg(feature = "time")]
    fn new_with_dropper(max_idle: Option<usize>) -> (local_sync::oneshot::Sender<()>, Self) {
        let idle_conns = HashMap::with_capacity(DEFAULT_POOL_SIZE);
        let max_idle = max_idle
            .map(|n| n.min(MAX_KEEPALIVE_CONNS))
            .unwrap_or(DEFAULT_KEEPALIVE_CONNS);

        let (tx, drop) = local_sync::oneshot::channel();
        (
            tx,
            Self {
                idle_conns,
                max_idle,
                idle_dur: None,
                _drop: Some(drop),
            },
        )
    }

    fn new(max_idle: Option<usize>) -> Self {
        let idle_conns = HashMap::with_capacity(DEFAULT_POOL_SIZE);
        let max_idle = max_idle
            .map(|n| n.min(MAX_KEEPALIVE_CONNS))
            .unwrap_or(DEFAULT_KEEPALIVE_CONNS);
        Self {
            idle_conns,
            max_idle,
            #[cfg(feature = "time")]
            idle_dur: None,
            #[cfg(feature = "time")]
            _drop: None,
        }
    }

    #[allow(unused)]
    fn clear_expired(&mut self, dur: Duration) {
        self.idle_conns.retain(|_, values| {
            values.retain(|entry| !entry.expired(dur));
            !values.is_empty()
        });
    }
}

#[derive(Debug)]
pub struct ConnectionPool<K, T> {
    shared: SharedPool<K, T>,
}

impl<K, T> Clone for ConnectionPool<K, T> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}

impl<K: 'static, T: 'static> ConnectionPool<K, T> {
    #[cfg(feature = "time")]
    pub fn new_with_idle_interval(
        // `idle_interval` controls how often the pool will check for idle connections.
        // It is also used to determine if a connection is expired.
        idle_interval: Option<Duration>,
        // `max_idle` is max idle connection count
        max_idle: Option<usize>,
    ) -> Self {
        const MIN_INTERVAL: Duration = Duration::from_secs(1);

        if let Some(idle_interval) = idle_interval {
            let idle_dur = idle_interval;
            let idle_interval = idle_interval.max(MIN_INTERVAL);

            let (tx, inner) = PoolInner::new_with_dropper(max_idle);
            let shared = Rc::new(UnsafeCell::new(inner));
            monoio::spawn(IdleTask {
                tx,
                conns: Rc::downgrade(&shared),
                interval: monoio::time::interval(idle_interval),
                idle_dur,
            });

            Self { shared }
        } else {
            let shared = Rc::new(UnsafeCell::new(PoolInner::new(max_idle)));
            Self { shared }
        }
    }

    #[inline]
    pub fn new(max_idle: Option<usize>) -> Self {
        Self {
            shared: Rc::new(UnsafeCell::new(PoolInner::new(max_idle))),
        }
    }
}

impl<K: 'static, T: 'static> Default for ConnectionPool<K, T> {
    fn default() -> Self {
        Self::new(None)
    }
}

impl<K: Key, T: Poolable> ConnectionPool<K, T> {
    /// Consume the element and return.
    /// Mostly use by h1.
    #[inline]
    pub fn get(&self, key: &K) -> Option<Pooled<K, T>> {
        let inner = unsafe { &mut *self.shared.get() };

        #[cfg(feature = "time")]
        loop {
            let r = match inner.idle_conns.get_mut(key) {
                Some(v) => match v.pop_front() {
                    Some(idle) if !idle.expired_opt(inner.idle_dur) => Some(Pooled::new(
                        key.to_owned(),
                        idle.conn,
                        true,
                        Rc::downgrade(&self.shared),
                    )),
                    Some(_) => {
                        continue;
                    }
                    None => None,
                },
                None => None,
            };
            return r;
        }

        #[cfg(not(feature = "time"))]
        match inner.idle_conns.get_mut(key) {
            Some(v) => match v.pop_front() {
                Some(idle) => Some(Pooled::new(
                    key.to_owned(),
                    idle.conn,
                    true,
                    Rc::downgrade(&self.shared),
                )),
                None => None,
            },
            None => None,
        }
    }

    #[inline]
    pub fn put(&self, key: K, conn: T) {
        let inner = unsafe { &mut *self.shared.get() };
        let queue = inner
            .idle_conns
            .entry(key)
            .or_insert(VecDeque::with_capacity(inner.max_idle));
        if queue.len() > inner.max_idle {
            for _ in 0..queue.len() - inner.max_idle {
                let _ = queue.pop_front();
            }
            let _ = queue.pop_front();
        }

        let idle = Idle::new(conn);
        queue.push_back(idle);
    }

    /// Get a reference to the element and apply f with map.
    /// Mostly use by h2.
    #[inline]
    #[allow(unused)]
    pub(crate) fn map_mut<F: FnOnce(&mut VecDeque<Idle<T>>) -> O, O>(
        &self,
        key: &K,
        f: F,
    ) -> Option<O> {
        let inner = unsafe { &mut *self.shared.get() };
        inner.idle_conns.get_mut(key).map(f)
    }

    /// Get a reference to the element and apply f with and_then.
    /// Mostly use by h2.
    #[inline]
    #[allow(unused)]
    pub(crate) fn and_then_mut<F: FnOnce(&mut VecDeque<Idle<T>>) -> Option<O>, O>(
        &self,
        key: &K,
        f: F,
    ) -> Option<O> {
        let inner = unsafe { &mut *self.shared.get() };
        inner.idle_conns.get_mut(key).and_then(f)
    }

    #[inline]
    pub fn link(&self, key: K, conn: T) -> Pooled<K, T> {
        #[cfg(feature = "logging")]
        tracing::debug!("linked new connection to the pool");

        Pooled::new(key, conn, false, Rc::downgrade(&self.shared))
    }

    #[inline]
    pub fn get_idle_connection_count(&self) -> usize {
        let inner: &PoolInner<K, T> = unsafe { &*self.shared.get() };
        inner.idle_conns.values().map(|v: &VecDeque<Idle<T>>| v.len()).sum()
    }
}

// TODO: make interval not eq to idle_dur
#[cfg(feature = "time")]
struct IdleTask<K, T> {
    tx: local_sync::oneshot::Sender<()>,
    conns: WeakPool<K, T>,
    interval: monoio::time::Interval,
    idle_dur: Duration,
}

#[cfg(feature = "time")]
impl<K, T> std::future::Future for IdleTask<K, T> {
    type Output = ();

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.get_mut();
        loop {
            match this.tx.poll_closed(cx) {
                std::task::Poll::Ready(_) => {
                    #[cfg(feature = "logging")]
                    tracing::debug!("pool rx dropped, idle task exit");
                    return std::task::Poll::Ready(());
                }
                std::task::Poll::Pending => (),
            }

            std::task::ready!(this.interval.poll_tick(cx));
            if let Some(inner) = this.conns.upgrade() {
                let inner_mut = unsafe { &mut *inner.get() };
                inner_mut.clear_expired(this.idle_dur);
                #[cfg(feature = "logging")]
                tracing::debug!("pool clear expired");
                continue;
            }
            #[cfg(feature = "logging")]
            tracing::debug!("pool upgrade failed, idle task exit");
            return std::task::Poll::Ready(());
        }
    }
}
