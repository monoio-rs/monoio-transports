use std::{
    cell::UnsafeCell,
    fmt::{Debug, Display},
    future::Future,
    hash::Hash,
    rc::Rc,
    task::ready,
    time::Duration,
};

use monoio::io::{AsyncReadRent, AsyncWriteRent, Split};
use monoio_codec::Framed;

use super::{connection::PooledConnection, Conns, WeakConns};

#[cfg(feature = "time")]
const DEFAULT_IDLE_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub struct ConnectionPool<K, IO: AsyncWriteRent> {
    conns: Conns<K, Framed<IO, ()>>,
}

impl<K, IO: AsyncWriteRent> Clone for ConnectionPool<K, IO> {
    fn clone(&self) -> Self {
        Self {
            conns: self.conns.clone(),
        }
    }
}

impl<K: 'static, IO: AsyncWriteRent + 'static> ConnectionPool<K, IO> {
    #[cfg(feature = "time")]
    fn new(idle_interval: Option<Duration>, max_idle: Option<usize>) -> Self {
        use super::SharedInner;

        let (tx, inner) = SharedInner::new(max_idle);
        let conns = Rc::new(UnsafeCell::new(inner));
        let idle_interval = idle_interval.unwrap_or(DEFAULT_IDLE_INTERVAL);
        monoio::spawn(IdleTask {
            tx,
            conns: Rc::downgrade(&conns),
            interval: monoio::time::interval(idle_interval),
            idle_dur: idle_interval,
        });

        Self { conns }
    }

    #[cfg(not(feature = "time"))]
    fn new(max_idle: Option<usize>) -> Self {
        let conns = Rc::new(UnsafeCell::new(SharedInner::new(max_idle)));
        Self { conns }
    }
}

impl<K: 'static, IO: AsyncWriteRent + 'static> Default for ConnectionPool<K, IO> {
    #[cfg(feature = "time")]
    fn default() -> Self {
        Self::new(None, None)
    }

    #[cfg(not(feature = "time"))]
    fn default() -> Self {
        Self::new(None)
    }
}

impl<K, IO> ConnectionPool<K, IO>
where
    K: Hash + Eq + ToOwned<Owned = K> + Display,
    IO: AsyncWriteRent + AsyncReadRent + Split,
{
    pub fn get(&self, key: &K) -> Option<PooledConnection<K, IO>> {
        let conns = unsafe { &mut *self.conns.get() };

        match conns.mapping.get_mut(key) {
            Some(v) => match v.pop_front() {
                Some(idle) => {
                    #[cfg(feature = "logging")]
                    tracing::debug!("connection got from pool for key: {:?} ", key.to_string());
                    Some(PooledConnection::new(
                        key.to_owned(),
                        idle.conn,
                        Rc::downgrade(&self.conns),
                    ))
                }
                None => None,
            },
            None => {
                #[cfg(feature = "logging")]
                tracing::debug!("no connection in pool for key: {:?} ", key.to_string());
                None
            }
        }
    }

    pub fn link(&self, key: K, conn: Framed<IO, ()>) -> PooledConnection<K, IO> {
        #[cfg(feature = "logging")]
        tracing::debug!("linked new connection to the pool");

        PooledConnection::new(key, conn, Rc::downgrade(&self.conns))
    }
}

// TODO: make interval not eq to idle_dur
struct IdleTask<K, IO: AsyncWriteRent> {
    tx: local_sync::oneshot::Sender<()>,
    conns: WeakConns<K, IO>,
    interval: monoio::time::Interval,
    idle_dur: Duration,
}

impl<K, IO: AsyncWriteRent> Future for IdleTask<K, IO> {
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

            ready!(this.interval.poll_tick(cx));
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
