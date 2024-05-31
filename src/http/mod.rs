mod connection;
mod connector;

pub use connection::HttpConnection;
pub use connector::HttpConnector;
pub use connector::H1Connector;

#[cfg(feature = "hyper")]
pub mod hyper;
