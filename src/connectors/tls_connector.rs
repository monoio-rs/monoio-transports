use std::{fmt::Debug, net::ToSocketAddrs};

use http::Uri;
use monoio::io::{AsyncReadRent, AsyncWriteRent, Split};
use service_async::Param;
use thiserror::Error as ThisError;

use super::Connector;
use crate::FromUriError;

#[cfg(not(feature = "native-tls"))]
pub type TlsStream<C> = monoio_rustls::ClientTlsStream<C>;

#[cfg(feature = "native-tls")]
pub type TlsStream<C> = monoio_native_tls::TlsStream<C>;

#[cfg(feature = "native-tls")]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerName(pub smol_str::SmolStr);
#[cfg(feature = "native-tls")]
pub use monoio_native_tls::TlsConnector as MonoioTlsConnector;
#[cfg(feature = "native-tls")]
pub use monoio_native_tls::TlsError;
#[cfg(not(feature = "native-tls"))]
pub use monoio_rustls::TlsConnector as MonoioTlsConnector;
#[cfg(not(feature = "native-tls"))]
pub use monoio_rustls::TlsError;
#[cfg(not(feature = "native-tls"))]
pub use rustls::ServerName;

#[cfg(feature = "native-tls")]
impl<T: Into<smol_str::SmolStr>> From<T> for ServerName {
    #[inline]
    fn from(value: T) -> Self {
        Self(value.into())
    }
}

#[derive(Clone)]
pub struct TlsConnector<C> {
    inner_connector: C,
    tls_connector: MonoioTlsConnector,
}

impl<C: Debug> std::fmt::Debug for TlsConnector<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TlsConnector, inner: {:?}", self.inner_connector)
    }
}

impl<C> TlsConnector<C> {
    pub const fn new(inner_connector: C, tls_connector: MonoioTlsConnector) -> Self {
        Self {
            inner_connector,
            tls_connector,
        }
    }

    #[cfg(not(feature = "native-tls"))]
    #[inline]
    pub fn new_with_tls_default(inner_connector: C) -> Self {
        let mut root_store = rustls::RootCertStore::empty();
        root_store.add_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.iter().map(|ta| {
            rustls::OwnedTrustAnchor::from_subject_spki_name_constraints(
                ta.subject,
                ta.spki,
                ta.name_constraints,
            )
        }));

        let cfg = rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        TlsConnector::new(inner_connector, cfg.into())
    }

    #[cfg(feature = "native-tls")]
    #[inline]
    pub fn new_with_tls_default(inner_connector: C) -> Self {
        TlsConnector::new(
            inner_connector,
            native_tls::TlsConnector::builder().build().unwrap().into(),
        )
    }

    #[inline]
    pub fn inner_connector(&self) -> &C {
        &self.inner_connector
    }

    #[inline]
    pub fn tls_connector(&self) -> &MonoioTlsConnector {
        &self.tls_connector
    }
}

impl<C: Default> Default for TlsConnector<C> {
    #[inline]
    fn default() -> Self {
        TlsConnector::new_with_tls_default(Default::default())
    }
}

impl<C, T, CN> Connector<T> for TlsConnector<C>
where
    T: AsRef<ServerName>,
    for<'a> C: Connector<&'a T, Error = std::io::Error, Connection = CN>,
    CN: AsyncReadRent + AsyncWriteRent,
{
    type Connection = TlsStream<CN>;
    type Error = TlsError;

    #[inline]
    async fn connect(&self, key: T) -> Result<Self::Connection, Self::Error> {
        let stream = self.inner_connector.connect(&key).await?;
        let server_name = key.as_ref();
        #[cfg(not(feature = "native-tls"))]
        let tls_stream = self
            .tls_connector
            .connect(server_name.clone(), stream)
            .await?;
        #[cfg(feature = "native-tls")]
        let tls_stream = self.tls_connector.connect(&server_name.0, stream).await?;
        Ok(tls_stream)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UnifiedTlsAddr {
    pub addr: super::UnifiedL4Addr,
    pub sn: ServerName,
}

impl Param<ServerName> for UnifiedTlsAddr {
    #[inline]
    fn param(&self) -> ServerName {
        self.sn.clone()
    }
}

impl AsRef<ServerName> for UnifiedTlsAddr {
    #[inline]
    fn as_ref(&self) -> &ServerName {
        &self.sn
    }
}

impl Param<super::UnifiedL4Addr> for UnifiedTlsAddr {
    #[inline]
    fn param(&self) -> super::UnifiedL4Addr {
        self.addr.clone()
    }
}

impl AsRef<super::UnifiedL4Addr> for UnifiedTlsAddr {
    #[inline]
    fn as_ref(&self) -> &super::UnifiedL4Addr {
        &self.addr
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TcpTlsAddr {
    pub host: smol_str::SmolStr,
    pub port: u16,
    pub sn: ServerName,
}

impl Param<ServerName> for TcpTlsAddr {
    #[inline]
    fn param(&self) -> ServerName {
        self.sn.clone()
    }
}

impl AsRef<ServerName> for TcpTlsAddr {
    #[inline]
    fn as_ref(&self) -> &ServerName {
        &self.sn
    }
}

impl ToSocketAddrs for TcpTlsAddr {
    type Iter = <(&'static str, u16) as ToSocketAddrs>::Iter;

    #[inline]
    fn to_socket_addrs(&self) -> std::io::Result<Self::Iter> {
        (self.host.as_str(), self.port).to_socket_addrs()
    }
}

impl TryFrom<&Uri> for TcpTlsAddr {
    type Error = FromUriError;

    #[inline]
    fn try_from(uri: &Uri) -> Result<Self, Self::Error> {
        let host = match uri.host() {
            Some(a) => a,
            None => return Err(FromUriError::NoAuthority),
        };

        let (tls, default_port) = match uri.scheme() {
            Some(scheme) if scheme == &http::uri::Scheme::HTTP => (false, 80),
            Some(scheme) if scheme == &http::uri::Scheme::HTTPS => (true, 443),
            _ => (false, 0),
        };
        if !tls {
            return Err(FromUriError::UnsupportScheme);
        }
        let host = smol_str::SmolStr::from(host);
        let port = uri.port_u16().unwrap_or(default_port);

        let sn = {
            #[cfg(feature = "native-tls")]
            {
                ServerName::from(host.clone())
            }
            #[cfg(not(feature = "native-tls"))]
            {
                ServerName::try_from(host.as_str())?
            }
        };

        Ok(TcpTlsAddr { host, port, sn })
    }
}

impl TryFrom<Uri> for TcpTlsAddr {
    type Error = FromUriError;

    #[inline]
    fn try_from(value: Uri) -> Result<Self, Self::Error> {
        Self::try_from(&value)
    }
}

#[derive(Debug, Clone)]
pub struct UnifiedConnector(pub TlsConnector<super::UnifiedL4Connector>);

impl UnifiedConnector {
    pub const fn new(
        inner_connector: super::UnifiedL4Connector,
        tls_connector: MonoioTlsConnector,
    ) -> Self {
        Self(TlsConnector::new(inner_connector, tls_connector))
    }

    #[inline]
    pub fn inner_connector(&self) -> &super::UnifiedL4Connector {
        &self.0.inner_connector
    }

    #[inline]
    pub fn tls_connector(&self) -> &MonoioTlsConnector {
        &self.0.tls_connector
    }
}

impl<'a> Connector<&'a UnifiedTlsAddr> for UnifiedConnector {
    type Connection = TlsStream<super::UnifiedL4Stream>;
    type Error = TlsError;

    #[inline]
    async fn connect(&self, key: &'a UnifiedTlsAddr) -> Result<Self::Connection, Self::Error> {
        let sn = &key.sn;
        let addr = &key.addr;
        let stream = self.0.inner_connector.connect(addr).await?;
        #[cfg(not(feature = "native-tls"))]
        let tls_stream = self.0.tls_connector.connect(sn.clone(), stream).await?;
        #[cfg(feature = "native-tls")]
        let tls_stream = self.0.tls_connector.connect(&sn.0, stream).await?;
        Ok(tls_stream)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnifiedAddr {
    pub addr: super::UnifiedL4Addr,
    pub sn: Option<ServerName>,
}

impl Param<Option<ServerName>> for UnifiedAddr {
    #[inline]
    fn param(&self) -> Option<ServerName> {
        self.sn.clone()
    }
}

impl AsRef<Option<ServerName>> for UnifiedAddr {
    #[inline]
    fn as_ref(&self) -> &Option<ServerName> {
        &self.sn
    }
}

impl Param<super::UnifiedL4Addr> for UnifiedAddr {
    #[inline]
    fn param(&self) -> super::UnifiedL4Addr {
        self.addr.clone()
    }
}

impl AsRef<super::UnifiedL4Addr> for UnifiedAddr {
    #[inline]
    fn as_ref(&self) -> &super::UnifiedL4Addr {
        &self.addr
    }
}

impl TryFrom<&Uri> for UnifiedAddr {
    type Error = FromUriError;

    #[inline]
    fn try_from(uri: &Uri) -> Result<Self, Self::Error> {
        let host = match uri.host() {
            Some(a) => a,
            None => return Err(FromUriError::NoAuthority),
        };

        let (tls, default_port) = match uri.scheme() {
            Some(scheme) if scheme == &http::uri::Scheme::HTTP => (false, 80),
            Some(scheme) if scheme == &http::uri::Scheme::HTTPS => (true, 443),
            _ => (false, 0),
        };
        let port = uri.port_u16().unwrap_or(default_port);

        let l4_addr = super::UnifiedL4Addr::Tcp(
            (host, port)
                .to_socket_addrs()?
                .next()
                .ok_or(crate::FromUriError::NoResolve)?,
        );

        let sn = if tls {
            #[cfg(feature = "native-tls")]
            {
                Some(ServerName::from(host))
            }
            #[cfg(not(feature = "native-tls"))]
            {
                Some(ServerName::try_from(host)?)
            }
        } else {
            None
        };

        Ok(UnifiedAddr { addr: l4_addr, sn })
    }
}

impl TryFrom<Uri> for UnifiedAddr {
    type Error = FromUriError;

    #[inline]
    fn try_from(value: Uri) -> Result<Self, Self::Error> {
        Self::try_from(&value)
    }
}

#[derive(Debug)]
pub enum UnifiedStream {
    L4(super::UnifiedL4Stream),
    Tls(TlsStream<super::UnifiedL4Stream>),
}

#[derive(ThisError, Debug)]
pub enum UnifiedError {
    #[error("L4 connect error {0}")]
    L4(std::io::Error),
    #[error("TLS connect error {0}")]
    Tls(TlsError),
}

impl<'a> Connector<&'a UnifiedAddr> for UnifiedConnector {
    type Connection = UnifiedStream;
    type Error = UnifiedError;

    #[inline]
    async fn connect(&self, key: &'a UnifiedAddr) -> Result<Self::Connection, Self::Error> {
        match &key.sn {
            Some(sn) => {
                let addr = &key.addr;
                let stream = self
                    .0
                    .inner_connector
                    .connect(addr)
                    .await
                    .map_err(|e| UnifiedError::Tls(TlsError::from(e)))?;
                #[cfg(not(feature = "native-tls"))]
                let tls_stream = self
                    .0
                    .tls_connector
                    .connect(sn.clone(), stream)
                    .await
                    .map_err(|e| UnifiedError::Tls(TlsError::from(e)))?;
                #[cfg(feature = "native-tls")]
                let tls_stream = self
                    .0
                    .tls_connector
                    .connect(&sn.0, stream)
                    .await
                    .map_err(UnifiedError::Tls)?;
                Ok(UnifiedStream::Tls(tls_stream))
            }
            None => {
                let addr = &key.addr;
                let stream = self
                    .0
                    .inner_connector
                    .connect(addr)
                    .await
                    .map_err(UnifiedError::L4)?;
                Ok(UnifiedStream::L4(stream))
            }
        }
    }
}

impl AsyncReadRent for UnifiedStream {
    #[inline]
    async fn read<T: monoio::buf::IoBufMut>(&mut self, buf: T) -> monoio::BufResult<usize, T> {
        match self {
            UnifiedStream::L4(inner) => inner.read(buf).await,
            UnifiedStream::Tls(inner) => inner.read(buf).await,
        }
    }

    #[inline]
    async fn readv<T: monoio::buf::IoVecBufMut>(&mut self, buf: T) -> monoio::BufResult<usize, T> {
        match self {
            UnifiedStream::L4(inner) => inner.readv(buf).await,
            UnifiedStream::Tls(inner) => inner.readv(buf).await,
        }
    }
}

impl AsyncWriteRent for UnifiedStream {
    #[inline]
    async fn write<T: monoio::buf::IoBuf>(&mut self, buf: T) -> monoio::BufResult<usize, T> {
        match self {
            UnifiedStream::L4(inner) => inner.write(buf).await,
            UnifiedStream::Tls(inner) => inner.write(buf).await,
        }
    }

    #[inline]
    async fn writev<T: monoio::buf::IoVecBuf>(
        &mut self,
        buf_vec: T,
    ) -> monoio::BufResult<usize, T> {
        match self {
            UnifiedStream::L4(inner) => inner.writev(buf_vec).await,
            UnifiedStream::Tls(inner) => inner.writev(buf_vec).await,
        }
    }

    #[inline]
    async fn flush(&mut self) -> std::io::Result<()> {
        match self {
            UnifiedStream::L4(inner) => inner.flush().await,
            UnifiedStream::Tls(inner) => inner.flush().await,
        }
    }

    #[inline]
    async fn shutdown(&mut self) -> std::io::Result<()> {
        match self {
            UnifiedStream::L4(inner) => inner.shutdown().await,
            UnifiedStream::Tls(inner) => inner.shutdown().await,
        }
    }
}

unsafe impl Split for UnifiedStream {}
