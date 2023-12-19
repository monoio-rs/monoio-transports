use std::path::Path;

use monoio::net::UnixStream;

use super::Connector;

#[derive(Default, Clone, Debug)]
pub struct UnixConnector;

impl<P> Connector<P> for UnixConnector
where
    P: AsRef<Path>,
{
    type Connection = UnixStream;
    type Error = crate::Error;

    async fn connect(&self, key: P) -> Result<Self::Connection, Self::Error> {
        UnixStream::connect(key).await.map_err(Into::into)
    }
}
