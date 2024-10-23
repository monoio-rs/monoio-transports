//! Defines core traits and types for creating and composing network connectors.
//!
//! This module provides the following key components:
//!
//! - The [`Connector`] trait for establishing connections
//! - The [`ConnectorExt`] trait for adding timeout functionality
//! - The [`TransportConnMetadata`] trait for retrieving connection metadata
mod l4_connector;
#[cfg(feature = "hyper")]
pub mod pollio;
mod tls_connector;

use std::{future::Future, time::Duration};

pub use l4_connector::*;
pub use tls_connector::*;

/// The [`Connector`] trait defines an interface for establishing connections.
/// This trait is designed to be composable, allowing for the creation of modular
/// and stackable connectors. Each connector in the stack can add its own layer
/// of functionality, such as TCP connection, Unix Domain Socket (UDS) connection,
/// TLS encryption, or HTTP protocol handling.
///
/// # Type Parameters
///
/// - `K`: The key type used to identify the connection target.
///
/// # Associated Types
///
/// - `Connection`: The type of the established connection.
/// - `Error`: The error type that may occur during connection.
///
/// # Examples
///
/// Here are examples of how to stack different connectors:
///
/// ## HTTP over Unix Domain Socket
///
/// ```rust
/// use http::request;
/// use monoio_http::{common::body::HttpBody, h1::payload::Payload};
/// use monoio_transports::{
///     connectors::{Connector, UnixConnector},
///     http::HttpConnector,
/// };
///
/// #[monoio::main]
/// async fn main() -> Result<(), monoio_transports::TransportError> {
///     let connector: HttpConnector<UnixConnector, _, _> = HttpConnector::default();
///     let mut conn = connector.connect("./examples/uds.sock").await?;
///
///     let req = request::Builder::new()
///         .uri("/get")
///         .header("Host", "test")
///         .body(HttpBody::H1(Payload::None))
///         .unwrap();
///
///     let (res, _) = conn.send_request(req).await;
///     let resp = res?;
///     assert_eq!(200, resp.status());
///
///     Ok(())
/// }
/// ```
///
/// ## HTTPS over TCP
///
/// ```rust
/// use monoio_transports::{
///     connectors::{Connector, TcpConnector, TcpTlsAddr, TlsConnector},
///     http::HttpConnector,
/// };
///
/// #[monoio::main]
/// async fn main() -> Result<(), monoio_transports::TransportError> {
///     let connector: HttpConnector<TlsConnector<TcpConnector>, _, _> =
///         HttpConnector::build_tls_http2_only();
///
///     let addr: TcpTlsAddr = "https://example.com".try_into()?;
///     let conn = connector.connect(addr).await?;
///
///     // Use the connection...
///
///     Ok(())
/// }
/// ```
///
/// These examples demonstrate how connectors can be stacked to create
/// different connection types (HTTP over UDS, HTTPS over TCP) using the
/// same `Connector` trait.
pub trait Connector<K> {
    type Connection;
    type Error;

    /// Attempts to establish a connection.
    ///
    /// # Parameters
    ///
    /// - `key`: The key identifying the connection target.
    ///
    /// # Returns
    ///
    /// A `Future` that resolves to a `Result` containing either the established
    /// connection or an error.
    fn connect(&self, key: K) -> impl Future<Output = Result<Self::Connection, Self::Error>>;
}

/// Extends the `Connector` trait with timeout functionality.
///
/// This trait is automatically implemented for all types that implement `Connector<K>`.
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

/// Provides additional information about the connection.
///
/// This trait is useful for transport connectors like TLS, TCP, UDS, etc.
pub trait TransportConnMetadata {
    /// The type of metadata associated with this connection.
    type Metadata;

    /// Retrieves the metadata from the connection.
    ///
    /// # Returns
    ///
    /// An instance of the associated type `Metadata`.
    fn get_conn_metadata(&self) -> Self::Metadata;
}

/// Represents the Application-Layer Protocol Negotiation (ALPN) protocol.
#[derive(Default, Copy, Clone)]
pub enum Alpn {
    /// HTTP/2 protocol
    HTTP2,
    /// HTTP/1.1 protocol
    HTTP11,
    /// No ALPN protocol specified (default)
    #[default]
    None,
}

/// Holds metadata for a transport connection.
///
/// Currently only holds the ALPN protocol information.
#[derive(Default, Copy, Clone)]
pub struct TransportConnMeta {
    alpn: Alpn,
}

impl TransportConnMeta {
    /// Sets the ALPN protocol based on the provided byte vector.
    ///
    /// # Arguments
    ///
    /// * `alpn` - An optional vector of bytes representing the ALPN protocol.
    ///
    /// # Notes
    ///
    /// Sets `Alpn::HTTP2` if the vector contains "h2", otherwise sets `Alpn::None`.
    pub fn set_alpn(&mut self, alpn: Option<Vec<u8>>) {
        self.alpn = match alpn {
            Some(p) if p == b"h2" => Alpn::HTTP2,
            _ => Alpn::None,
        }
    }

    /// Checks if the ALPN protocol is set to HTTP/2.
    ///
    /// # Returns
    ///
    /// `true` if the ALPN protocol is HTTP/2, `false` otherwise.
    pub fn is_alpn_h2(&self) -> bool {
        matches!(self.alpn, Alpn::HTTP2)
    }
}
