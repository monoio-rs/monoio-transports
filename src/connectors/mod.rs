mod tcp_connector;
mod tls_connector;
pub mod unified_connector;
mod unix_connector;

use std::future::Future;

pub use tcp_connector::TcpConnector;
pub use tls_connector::{TlsConnector, TlsStream};
pub use unix_connector::UnixConnector;

pub trait Connector<K> {
    type Connection;
    type Error;

    fn connect(&self, key: K) -> impl Future<Output = Result<Self::Connection, Self::Error>>;
}
