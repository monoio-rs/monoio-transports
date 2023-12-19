pub mod connection;
pub mod connector;
pub mod pool;

use std::{
    cell::UnsafeCell,
    collections::{HashMap, VecDeque},
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

use monoio::io::AsyncWriteRent;

const DEFAULT_KEEPALIVE_CONNS: usize = 256;
const DEFAULT_POOL_SIZE: usize = 32;
// https://datatracker.ietf.org/doc/html/rfc6335
const MAX_KEEPALIVE_CONNS: usize = 16384;

type Conns<K, IO> = Rc<UnsafeCell<SharedInner<K, IO>>>;
type WeakConns<K, IO> = Weak<UnsafeCell<SharedInner<K, IO>>>;

pub(crate) struct IdleConnection<IO: AsyncWriteRent> {
    conn: IO,
    idle_at: Instant,
}

pub(crate) struct SharedInner<K, IO: AsyncWriteRent> {
    mapping: HashMap<K, VecDeque<IdleConnection<IO>>>,
    max_idle: usize,
    #[cfg(feature = "time")]
    _drop: local_sync::oneshot::Receiver<()>,
}

impl<K, IO: AsyncWriteRent> SharedInner<K, IO> {
    #[cfg(feature = "time")]
    fn new(max_idle: Option<usize>) -> (local_sync::oneshot::Sender<()>, Self) {
        let mapping = HashMap::with_capacity(DEFAULT_POOL_SIZE);
        let max_idle = max_idle
            .map(|n| n.min(MAX_KEEPALIVE_CONNS))
            .unwrap_or(DEFAULT_KEEPALIVE_CONNS);

        let (tx, _drop) = local_sync::oneshot::channel();
        (
            tx,
            Self {
                mapping,
                _drop,
                max_idle,
            },
        )
    }

    #[cfg(not(feature = "time"))]
    fn new(max_idle: Option<usize>) -> Self {
        let mapping = HashMap::with_capacity(DEFAULT_POOL_SIZE);
        let max_idle = max_idle
            .map(|n| n.min(MAX_KEEPALIVE_CONNS))
            .unwrap_or(DEFAULT_KEEPALIVE_CONNS);
        Self { mapping, max_idle }
    }

    fn clear_expired(&mut self, dur: Duration) {
        self.mapping.retain(|_, values| {
            values.retain(|entry| entry.idle_at.elapsed() <= dur);
            !values.is_empty()
        });
    }
}
