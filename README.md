# monoio-transports

`monoio-transports` is a high-performance, modular networking library built on top of the monoio
asynchronous runtime. It provides a set of connectors and utilities for efficient network
communications, optimized for use with io_uring.

## Key Features

- Modular and stackable connector architecture
- Support for various transport layers (TCP, Unix Domain Sockets)
- TLS support for secure communications
- HTTP/1.1 and HTTP/2 protocol support
- Advanced connection pooling for optimal performance
- Optimized for monoio's asynchronous runtime and io_uring
- High-performance client implementations with built-in connection reuse

## Core Concepts

### Connector Trait

The `Connector` trait is the foundation of this crate's modular architecture. It defines a common interface for establishing network connections:

```rust
pub trait Connector<K> {
    type Connection;
    type Error;
    fn connect(&self, key: K) -> impl Future<Output = Result<Self::Connection, Self::Error>>;
}
```

This trait allows for the creation of various connector types that can be easily composed
and stacked to create complex connection setups.

### TransportConnMetadata Trait

The `TransportConnMetadata` trait provides a way to retrieve additional information about
a connection, such as ALPN (Application-Layer Protocol Negotiation) details:

```rust
pub trait TransportConnMetadata {
    type Metadata;
    fn get_conn_metadata(&self) -> Self::Metadata;
}
```

## Connector Types

### L4 Connectors

- `TcpConnector`: Establishes TCP connections
- `UnixConnector`: Establishes Unix Domain Socket connections
- `UnifiedL4Connector`: A unified connector supporting both TCP and Unix Domain Sockets

### TLS Connector

`TlsConnector` adds TLS encryption to an underlying L4 connector, supporting both
native-tls and rustls backends.

### HTTP Connector

`HttpConnector` is a universal connector supporting both HTTP/1.1 and HTTP/2 protocols.
It can be used with various underlying connectors (TCP, Unix, TLS) and provides built-in
connection pooling for efficient resource usage and high performance.

## Connection Pooling

The crate provides a generic, flexible connection pooling implementation that can be used
with any type of connection:

- `ConnectionPool`: A generic pool that can manage and reuse any type of connection.
- `PooledConnector`: A wrapper that adds pooling capabilities to any connector.

This generic pooling system is currently utilized in:

- `HttpConnector`: Leverages connection pooling for both HTTP/1.1 and HTTP/2.
- Hyper connectors (when the `hyper` feature is enabled): Utilize the pooling system for
  efficient connection reuse compatible with the Hyper ecosystem.

The flexibility of this pooling implementation allows developers to easily add
connection reuse capabilities to their custom connectors or any other connection types.
This results in high-performance clients that efficiently manage connections,
significantly improving throughput and reducing latency in high-load scenarios across
various protocols and connection types.

## Stacking Connectors

Connectors can be easily stacked to create powerful, flexible connection setups. For example:

```rust
use monoio_transports::{
    connectors::{TcpConnector, TlsConnector},
    HttpConnector,
};

// Simple TCP conenctor
let tcp_connector = TcpConnector::default();
// TLS conencttor with custom ALPN protocols set
let tls_connector = TlsConnector::new_with_tls_default(tcp_connector, Some(vec!["http/1.1"]));
// Https conector with HTTP_2 and HTTP_11 support.
let https_connector: HttpConnector<TlsConnector<TcpConnector>, _, _> = HttpConnector::default();
```

This example creates a connector stack that uses TCP for the base connection, adds TLS
encryption, and then provides HTTP protocol handling on top with built-in connection pooling.

## Feature Flags

- `native-tls`: Enables the native-tls backend for TLS connections
- `hyper`: Enables integration with the Hyper HTTP library, including Hyper-compatible
  connectors with efficient connection pooling

By leveraging monoio's efficient asynchronous runtime, io_uring, and advanced connection
pooling, `monoio-transports` provides a powerful and flexible toolkit for building
high-performance network applications and HTTP clients.
