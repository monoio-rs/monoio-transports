mod connection;
mod connector;

pub use connection::{Http1Connection, Http2Connection, HttpConnection};
pub use connector::{H1Connector, H2Connector, HttpsConnector};

#[cfg(feature = "hyper")]
pub mod hyper;
