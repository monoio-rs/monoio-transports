use std::net::ToSocketAddrs;

use monoio::net::TcpStream;

use super::Connector;

#[derive(Default, Clone, Debug)]
pub struct TcpConnector;

impl<T> Connector<T> for TcpConnector
where
    T: ToSocketAddrs,
{
    type Connection = TcpStream;
    type Error = crate::Error;

    async fn connect(&self, key: T) -> Result<Self::Connection, Self::Error> {
        TcpStream::connect(key)
            .await
            .map(|io| {
                // we will ignore the set nodelay error
                let _ = io.set_nodelay(true);
                io
            })
            .map_err(Into::into)
    }
}
