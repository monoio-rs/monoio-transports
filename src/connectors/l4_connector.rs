use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
};

use http::Uri;
use monoio::{
    io::{AsyncReadRent, AsyncWriteRent, Split},
    net::{TcpStream, UnixStream},
};

use super::{Connector, TransportConnMeta, TransportConnMetadata};

#[derive(Default, Clone, Copy, Debug)]
pub struct TcpConnector {
    pub no_delay: bool,
}

impl<T: ToSocketAddrs> Connector<T> for TcpConnector {
    type Connection = TcpStream;
    type Error = io::Error;

    #[inline]
    async fn connect(&self, key: T) -> Result<Self::Connection, Self::Error> {
        TcpStream::connect(key).await.map(|io| {
            if self.no_delay {
                // we will ignore the set nodelay error
                let _ = io.set_nodelay(true);
            }
            io
        })
    }
}

impl TransportConnMetadata for TcpStream {
    type Metadata = TransportConnMeta;

    fn get_conn_metadata(&self) -> Self::Metadata {
        TransportConnMeta::default()
    }
}

#[derive(Default, Clone, Copy, Debug)]
pub struct UnixConnector;

impl<P: AsRef<Path>> Connector<P> for UnixConnector {
    type Connection = UnixStream;
    type Error = io::Error;

    #[inline]
    async fn connect(&self, key: P) -> Result<Self::Connection, Self::Error> {
        UnixStream::connect(key).await
    }
}

impl TransportConnMetadata for UnixStream {
    type Metadata = TransportConnMeta;

    fn get_conn_metadata(&self) -> Self::Metadata {
        TransportConnMeta::default()
    }
}

#[derive(Default, Clone, Copy, Debug)]
pub struct UnifiedL4Connector {
    tcp: TcpConnector,
    unix: UnixConnector,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum UnifiedL4Addr {
    Tcp(SocketAddr),
    Unix(PathBuf),
}

impl AsRef<UnifiedL4Addr> for UnifiedL4Addr {
    #[inline]
    fn as_ref(&self) -> &UnifiedL4Addr {
        self
    }
}

impl TryFrom<&Uri> for UnifiedL4Addr {
    type Error = crate::FromUriError;

    #[inline]
    fn try_from(uri: &Uri) -> Result<Self, Self::Error> {
        let host = match uri.host() {
            Some(a) => a,
            None => return Err(crate::FromUriError::NoAuthority),
        };

        let default_port = match uri.scheme() {
            Some(scheme) if scheme == &http::uri::Scheme::HTTP => 80,
            Some(scheme) if scheme == &http::uri::Scheme::HTTPS => 443,
            _ => 0,
        };
        let port = uri.port_u16().unwrap_or(default_port);
        let addr = (host, port)
            .to_socket_addrs()?
            .next()
            .ok_or(crate::FromUriError::NoResolve)?;

        Ok(Self::Tcp(addr))
    }
}

impl TryFrom<Uri> for UnifiedL4Addr {
    type Error = crate::FromUriError;

    fn try_from(value: Uri) -> Result<Self, Self::Error> {
        Self::try_from(&value)
    }
}

#[derive(Debug)]
pub enum UnifiedL4Stream {
    Tcp(TcpStream),
    Unix(UnixStream),
}

impl<T: AsRef<UnifiedL4Addr>> Connector<T> for UnifiedL4Connector {
    type Connection = UnifiedL4Stream;
    type Error = io::Error;

    #[inline]
    async fn connect(&self, key: T) -> Result<Self::Connection, Self::Error> {
        match key.as_ref() {
            UnifiedL4Addr::Tcp(addr) => self.tcp.connect(addr).await.map(UnifiedL4Stream::Tcp),
            UnifiedL4Addr::Unix(path) => self.unix.connect(path).await.map(UnifiedL4Stream::Unix),
        }
    }
}

impl AsyncReadRent for UnifiedL4Stream {
    #[inline]
    async fn read<T: monoio::buf::IoBufMut>(&mut self, buf: T) -> monoio::BufResult<usize, T> {
        match self {
            UnifiedL4Stream::Tcp(inner) => inner.read(buf).await,
            UnifiedL4Stream::Unix(inner) => inner.read(buf).await,
        }
    }

    #[inline]
    async fn readv<T: monoio::buf::IoVecBufMut>(&mut self, buf: T) -> monoio::BufResult<usize, T> {
        match self {
            UnifiedL4Stream::Tcp(inner) => inner.readv(buf).await,
            UnifiedL4Stream::Unix(inner) => inner.readv(buf).await,
        }
    }
}

impl AsyncWriteRent for UnifiedL4Stream {
    #[inline]
    async fn write<T: monoio::buf::IoBuf>(&mut self, buf: T) -> monoio::BufResult<usize, T> {
        match self {
            UnifiedL4Stream::Tcp(inner) => inner.write(buf).await,
            UnifiedL4Stream::Unix(inner) => inner.write(buf).await,
        }
    }

    #[inline]
    async fn writev<T: monoio::buf::IoVecBuf>(
        &mut self,
        buf_vec: T,
    ) -> monoio::BufResult<usize, T> {
        match self {
            UnifiedL4Stream::Tcp(inner) => inner.writev(buf_vec).await,
            UnifiedL4Stream::Unix(inner) => inner.writev(buf_vec).await,
        }
    }

    #[inline]
    async fn flush(&mut self) -> std::io::Result<()> {
        match self {
            UnifiedL4Stream::Tcp(inner) => inner.flush().await,
            UnifiedL4Stream::Unix(inner) => inner.flush().await,
        }
    }

    #[inline]
    async fn shutdown(&mut self) -> std::io::Result<()> {
        match self {
            UnifiedL4Stream::Tcp(inner) => inner.shutdown().await,
            UnifiedL4Stream::Unix(inner) => inner.shutdown().await,
        }
    }
}

unsafe impl Split for UnifiedL4Stream {}
