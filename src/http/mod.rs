//! High-performance HTTP client implementations
//!
//! This module provides high-performance HTTP client implementations optimized for use with
//! monoio's asynchronous runtime and io_uring. It supports both HTTP/1.1 and HTTP/2 protocols,
//! offering a unified interface for handling HTTP connections and requests.
//!
//! # Key Components
//!
//! - [`HttpConnection`](HttpConnection): A unified enum representing either an HTTP/1.1 or HTTP/2
//!   connection. It provides a common interface for sending requests regardless of the underlying
//!   protocol.
//!
//! - [`HttpConnector`](HttpConnection): A universal connector supporting both HTTP/1.1 and HTTP/2
//!   protocols. It can be used with a `TlsConnector` for HTTPS connections and is specifically
//!   designed to work with monoio's native IO traits, which are built on top of io_uring.
//!
//! - [`H1Connector`]: A deprecated HTTP/1.1 connector retained for backwards compatibility. New
//!   code should use `HttpConnector` instead.
//!
//! # Features
//!
//! - Optimized for monoio's asynchronous runtime and io_uring
//! - Support for both HTTP/1.1 and HTTP/2 protocols.
//! - Connection pooling for efficient reuse of established connections.
//! - Unified interface for sending requests across different HTTP versions.
//! - TLS support for secure HTTPS connections.
//!
//! # Connector Types and Usage
//!
//! The [`HttpConnector`](HttpConnector) provides various methods to create connectors for different
//! protocols and transport layers:
//!
//! | Connector Type | Protocol | Method | Example | Description |
//! | --- | --- | --- | --- | --- |
//! | `TlsConnector` | HTTP/1.1 and HTTP/2 | `default()` | ```rust<br>let connector: HttpConnector<TlsConnector<TcpConnector>, *, *> = HttpConnector::default();<br>``` | Creates a `HttpConnector` that supports both HTTP/1.1 and HTTP/2, leveraging monoio's io_uring capabilities. |
//! | `TcpConnector` | HTTP/1.1 | `build_tcp_http1_only()` | ```rust<br>let connector = HttpConnector::build_tcp_http1_only();<br>``` | Creates an `HttpConnector` that only supports HTTP/1.1 over TCP, using monoio's native IO traits. |
//! | `TcpConnector` | HTTP/2 | `build_tcp_http2_only()` | ```rust<br>let connector = HttpConnector::build_tcp_http2_only();<br>``` | Creates an `HttpConnector` that only supports HTTP/2 over TCP, optimized for io_uring. |
//! | `TlsConnector` | HTTP/1.1 | `build_tls_http1_only()` | ```rust<br>let connector = HttpConnector::build_tls_http1_only();<br>``` | Creates an `HttpConnector` with a `TlsConnector` that only supports HTTP/1.1, using monoio's efficient I/O operations. |
//! | `TlsConnector` | HTTP/2 | `build_tls_http2_only()` | ```rust<br>let connector = HttpConnector::build_tls_http2_only();<br>``` | Creates an `HttpConnector` with a `TlsConnector` that only supports HTTP/2, fully utilizing io_uring's performance benefits. |
//!
//! # Hyper Integration
//!
//! When the `hyper` feature flag is enabled, this module provides additional connectors
//! that integrate with the Hyper HTTP library:
//!
//! - [`HyperH1Connector`](hyper::HyperH1Conenctor): An HTTP/1.1 connector compatible with Hyper's
//!   interfaces.
//! - [`HyperH2Connector`](hyper::HyperH2Conenctor): An HTTP/2 connector compatible with Hyper's
//!   interfaces.
mod connection;
mod connector;

pub use connection::HttpConnection;
pub use connector::{H1Connector, HttpConnector};

#[cfg(feature = "hyper")]
pub mod hyper;
