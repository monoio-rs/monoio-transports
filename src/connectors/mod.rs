mod l4_connector;
#[cfg(feature = "hyper")]
pub mod pollio;
mod tls_connector;

use std::{future::Future, time::Duration};

pub use l4_connector::*;
pub use tls_connector::*;

pub trait Connector<K> {
    type Connection;
    type Error;

    fn connect(&self, key: K) -> impl Future<Output = Result<Self::Connection, Self::Error>>;
}

pub trait ConnectorExt<K>: Connector<K> {
    fn connect_with_timeout(
        &self,
        key: K,
        timeout: Duration,
    ) -> impl Future<Output = Result<Result<Self::Connection, Self::Error>, monoio::time::error::Elapsed>>;
}

impl<K, T: Connector<K>> ConnectorExt<K> for T {
    #[inline]
    fn connect_with_timeout(
        &self,
        key: K,
        timeout: Duration,
    ) -> impl Future<Output = Result<Result<Self::Connection, Self::Error>, monoio::time::error::Elapsed>>
    {
        monoio::time::timeout(timeout, self.connect(key))
    }
}
