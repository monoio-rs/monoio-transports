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

/// `TransportConnMetadata` is a trait that provides additional information about the connection.
/// This is useful for transport connectors like TLS, TCP, UDS etc.
///
/// The `get_conn_metadata` method is used to retrieve the metadata from the connection.
pub trait TransportConnMetadata {
    type Metadata;

    /// Retrieves the metadata from the connection.
    ///
    /// # Returns
    ///
    /// An instance of the associated type `Metadata`.
    fn get_conn_metadata(&self) -> Self::Metadata;
}

#[derive(Default, Copy, Clone)]
pub enum Alpn {
    HTTP2,
    HTTP11,
    #[default]
    None,
}

/// `TransportConnMeta` is a struct that holds metadata for a transport connection.
/// It currently only holds the `Alpn` protocol.
#[derive(Default, Copy, Clone)]
pub struct TransportConnMeta {
    alpn: Alpn,
}

impl TransportConnMeta {
    pub fn set_alpn(&mut self, alpn: Option<Vec<u8>>) {
        self.alpn = match alpn {
            Some(p) if p == b"h2" => Alpn::HTTP2,
            _ => Alpn::None,
        }
    }

    pub fn is_alpn_h2(&self) -> bool {
        match self.alpn {
            Alpn::HTTP2 => true,
            _ => false,
        }
    }
}
