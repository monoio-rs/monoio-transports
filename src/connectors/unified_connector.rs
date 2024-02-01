use std::{
    fmt::Display,
    hash::Hash,
    io,
    net::ToSocketAddrs,
    path::{Path, PathBuf},
};

use monoio::{
    buf::{IoBuf, IoBufMut, IoVecBuf, IoVecBufMut},
    io::{AsyncReadRent, AsyncWriteRent, Split},
    net::{TcpStream, UnixStream},
    BufResult,
};
use service_async::Param;
use smol_str::SmolStr;

use super::{
    tcp_connector::TcpConnector,
    tls_connector::{TlsConnector, TlsStream},
    unix_connector::UnixConnector,
    Connector,
};
use crate::key::Key;

// TODO: make its PathBuf and SmolStr to ref
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnifiedTransportAddr {
    Tcp(SmolStr, u16),
    Unix(PathBuf),
    TcpTls(SmolStr, u16, crate::key::ServerName),
    UnixTls(PathBuf, crate::key::ServerName),
}

impl Display for UnifiedTransportAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnifiedTransportAddr::Tcp(addr, port) => write!(f, "{}:{}", addr, port),
            UnifiedTransportAddr::Unix(path) => write!(f, "{:?}", path),
            UnifiedTransportAddr::TcpTls(addr, port, sn) => write!(f, "{}:{}:{:?}", addr, port, sn),
            UnifiedTransportAddr::UnixTls(path, sn) => write!(f, "{:?}:{:?}", path, sn),
        }
    }
}

impl Hash for UnifiedTransportAddr {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let key = format!("{}", self);
        key.hash(state);
    }
}

impl Param<UnifiedTransportAddr> for Key {
    fn param(&self) -> UnifiedTransportAddr {
        if let Some(sn) = self.server_name.clone() {
            UnifiedTransportAddr::TcpTls(self.host.clone(), self.port, sn)
        } else {
            UnifiedTransportAddr::Tcp(self.host.clone(), self.port)
        }
    }
}

struct TcpTlsAddr<'a>(&'a SmolStr, u16, &'a crate::key::ServerName);
struct UnixTlsAddr<'a>(&'a PathBuf, &'a crate::key::ServerName);
impl<'a> ToSocketAddrs for TcpTlsAddr<'a> {
    type Iter = <(&'static str, u16) as ToSocketAddrs>::Iter;
    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        (self.0.as_str(), self.1).to_socket_addrs()
    }
}
impl<'a> service_async::Param<Option<crate::key::ServerName>> for TcpTlsAddr<'a> {
    fn param(&self) -> Option<crate::key::ServerName> {
        Some(self.2.clone())
    }
}
impl<'a> AsRef<Path> for UnixTlsAddr<'a> {
    fn as_ref(&self) -> &Path {
        self.0
    }
}
impl<'a> service_async::Param<Option<crate::key::ServerName>> for UnixTlsAddr<'a> {
    fn param(&self) -> Option<crate::key::ServerName> {
        Some(self.1.clone())
    }
}

#[derive(Default, Clone, Debug)]
pub struct UnifiedTransportConnector {
    raw_tcp: TcpConnector,
    raw_unix: UnixConnector,
    tcp_tls: TlsConnector<TcpConnector>,
    unix_tls: TlsConnector<UnixConnector>,
}

pub enum UnifiedTransportConnection {
    Tcp(TcpStream),
    Unix(UnixStream),
    TcpTls(TlsStream<TcpStream>),
    UnixTls(TlsStream<UnixStream>),
    // TODO
    // Custom(Box<dyn ...>)
}

impl std::fmt::Debug for UnifiedTransportConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tcp(_) => write!(f, "Tcp"),
            Self::Unix(_) => write!(f, "Unix"),
            Self::TcpTls(_) => write!(f, "TcpTls"),
            Self::UnixTls(_) => write!(f, "UnixTls"),
        }
    }
}

impl<T> Connector<T> for UnifiedTransportConnector
where
    T: Param<UnifiedTransportAddr>,
{
    type Connection = UnifiedTransportConnection;
    type Error = crate::Error;

    async fn connect(&self, key: T) -> Result<Self::Connection, Self::Error> {
        let unified_addr = key.param();
        match &unified_addr {
            UnifiedTransportAddr::Tcp(addr, port) => self
                .raw_tcp
                .connect((addr.as_str(), *port))
                .await
                .map_err(Into::into)
                .map(UnifiedTransportConnection::Tcp),
            UnifiedTransportAddr::Unix(path) => self
                .raw_unix
                .connect(path)
                .await
                .map_err(Into::into)
                .map(UnifiedTransportConnection::Unix),
            UnifiedTransportAddr::TcpTls(addr, port, tls) => self
                .tcp_tls
                .connect(TcpTlsAddr(addr, *port, tls))
                .await
                .map_err(Into::into)
                .map(UnifiedTransportConnection::TcpTls),
            UnifiedTransportAddr::UnixTls(path, tls) => self
                .unix_tls
                .connect(UnixTlsAddr(path, tls))
                .await
                .map_err(Into::into)
                .map(UnifiedTransportConnection::UnixTls),
        }
    }
}

impl AsyncReadRent for UnifiedTransportConnection {
    async fn read<T: IoBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        match self {
            UnifiedTransportConnection::Tcp(s) => s.read(buf).await,
            UnifiedTransportConnection::Unix(s) => s.read(buf).await,
            UnifiedTransportConnection::TcpTls(s) => s.read(buf).await,
            UnifiedTransportConnection::UnixTls(s) => s.read(buf).await,
        }
    }

    async fn readv<T: IoVecBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        match self {
            UnifiedTransportConnection::Tcp(s) => s.readv(buf).await,
            UnifiedTransportConnection::Unix(s) => s.readv(buf).await,
            UnifiedTransportConnection::TcpTls(s) => s.readv(buf).await,
            UnifiedTransportConnection::UnixTls(s) => s.readv(buf).await,
        }
    }
}

impl AsyncWriteRent for UnifiedTransportConnection {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        match self {
            UnifiedTransportConnection::Tcp(s) => s.write(buf).await,
            UnifiedTransportConnection::Unix(s) => s.write(buf).await,
            UnifiedTransportConnection::TcpTls(s) => s.write(buf).await,
            UnifiedTransportConnection::UnixTls(s) => s.write(buf).await,
        }
    }

    async fn writev<T: IoVecBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        match self {
            UnifiedTransportConnection::Tcp(s) => s.writev(buf).await,
            UnifiedTransportConnection::Unix(s) => s.writev(buf).await,
            UnifiedTransportConnection::TcpTls(s) => s.writev(buf).await,
            UnifiedTransportConnection::UnixTls(s) => s.writev(buf).await,
        }
    }

    async fn flush(&mut self) -> io::Result<()> {
        match self {
            UnifiedTransportConnection::Tcp(s) => s.flush().await,
            UnifiedTransportConnection::Unix(s) => s.flush().await,
            UnifiedTransportConnection::TcpTls(s) => s.flush().await,
            UnifiedTransportConnection::UnixTls(s) => s.flush().await,
        }
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        match self {
            UnifiedTransportConnection::Tcp(s) => s.shutdown().await,
            UnifiedTransportConnection::Unix(s) => s.shutdown().await,
            UnifiedTransportConnection::TcpTls(s) => s.shutdown().await,
            UnifiedTransportConnection::UnixTls(s) => s.shutdown().await,
        }
    }
}

unsafe impl Split for UnifiedTransportConnection {}
