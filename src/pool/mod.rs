mod connector;
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
pub use reuse::{Reuse, ReuseConnector};

pub(crate) const DEFAULT_KEEPALIVE_CONNS: usize = 256;
pub(crate) const DEFAULT_POOL_SIZE: usize = 32;
// https://datatracker.ietf.org/doc/html/rfc6335
pub(crate) const MAX_KEEPALIVE_CONNS: usize = 16384;
#[cfg(feature = "time")]
pub(crate) const DEFAULT_IDLE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

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
                    if queue.len() > pool.max_idle {
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
    conn: IO,
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
}

pub(crate) struct PoolInner<K, IO> {
    idle_conns: HashMap<K, VecDeque<Idle<IO>>>,
    max_idle: usize,
    #[cfg(feature = "time")]
    _drop: local_sync::oneshot::Receiver<()>,
}

impl<K, IO> PoolInner<K, IO> {
    #[cfg(feature = "time")]
    fn new(max_idle: Option<usize>) -> (local_sync::oneshot::Sender<()>, Self) {
        let idle_conns = HashMap::with_capacity(DEFAULT_POOL_SIZE);
        let max_idle = max_idle
            .map(|n| n.min(MAX_KEEPALIVE_CONNS))
            .unwrap_or(DEFAULT_KEEPALIVE_CONNS);

        let (tx, _drop) = local_sync::oneshot::channel();
        (
            tx,
            Self {
                idle_conns,
                max_idle,
                _drop,
            },
        )
    }

    #[cfg(not(feature = "time"))]
    fn new(max_idle: Option<usize>) -> Self {
        let idle_conns = HashMap::with_capacity(DEFAULT_POOL_SIZE);
        let max_idle = max_idle
            .map(|n| n.min(MAX_KEEPALIVE_CONNS))
            .unwrap_or(DEFAULT_KEEPALIVE_CONNS);
        Self {
            idle_conns,
            max_idle,
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
        idle_interval: Option<Duration>,
        max_idle: Option<usize>,
    ) -> Self {
        const MIN_INTERVAL: Duration = Duration::from_secs(1);

        let (tx, inner) = PoolInner::new(max_idle);
        let shared = Rc::new(UnsafeCell::new(inner));
        let idle_interval = idle_interval.unwrap_or(DEFAULT_IDLE_INTERVAL);
        monoio::spawn(IdleTask {
            tx,
            conns: Rc::downgrade(&shared),
            interval: monoio::time::interval(idle_interval.max(MIN_INTERVAL)),
            idle_dur: idle_interval,
        });

        Self { shared }
    }

    #[cfg(feature = "time")]
    #[inline]
    pub fn new(max_idle: Option<usize>) -> Self {
        Self::new_with_idle_interval(None, max_idle)
    }

    #[cfg(not(feature = "time"))]
    #[inline]
    pub fn new(max_idle: Option<usize>) -> Self {
        let shared = Rc::new(UnsafeCell::new(PoolInner::new(max_idle)));
        Self { shared }
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

    /// Get a reference to the element and apply f.
    /// Mostly use by h2.
    #[inline]
    pub fn map_ref<F: FnOnce(&T) -> O, O>(&self, key: &K, f: F) -> Option<O> {
        let inner = unsafe { &mut *self.shared.get() };

        inner
            .idle_conns
            .get_mut(key)
            .and_then(|conns| conns.front().map(|elem| &elem.conn))
            .map(f)
    }

    #[inline]
    pub fn link(&self, key: K, conn: T) -> Pooled<K, T> {
        #[cfg(feature = "logging")]
        tracing::debug!("linked new connection to the pool");

        Pooled::new(key, conn, false, Rc::downgrade(&self.shared))
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
