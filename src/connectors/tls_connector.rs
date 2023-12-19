use std::fmt::Debug;

use monoio::io::{AsyncReadRent, AsyncWriteRent};

use super::Connector;
use crate::Error;

#[cfg(not(feature = "native-tls"))]
pub type TlsStream<C> = monoio_rustls::ClientTlsStream<C>;

#[cfg(feature = "native-tls")]
pub type TlsStream<C> = monoio_native_tls::TlsStream<C>;

#[derive(Clone)]
pub struct TlsConnector<C> {
    inner_connector: C,
    #[cfg(not(feature = "native-tls"))]
    tls_connector: monoio_rustls::TlsConnector,
    #[cfg(feature = "native-tls")]
    tls_connector: monoio_native_tls::TlsConnector,
}

impl<C: Debug> std::fmt::Debug for TlsConnector<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TlsConnector, inner: {:?}", self.inner_connector)
    }
}

impl<C> TlsConnector<C> {
    #[cfg(not(feature = "native-tls"))]
    pub fn new(inner_connector: C, tls_connector: monoio_rustls::TlsConnector) -> Self {
        Self {
            inner_connector,
            tls_connector: tls_connector,
        }
    }

    #[cfg(feature = "native-tls")]
    pub fn new(inner_connector: C, tls_connector: monoio_native_tls::TlsConnector) -> Self {
        Self {
            inner_connector,
            tls_connector: tls_connector,
        }
    }

    #[cfg(not(feature = "native-tls"))]
    pub fn with_connector(inner_connector: C) -> Self {
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
    pub fn with_connector(inner_connector: C) -> Self {
        TlsConnector::new(
            inner_connector,
            native_tls::TlsConnector::builder().build().unwrap().into(),
        )
    }
}

impl<C: Default> Default for TlsConnector<C> {
    fn default() -> Self {
        TlsConnector::with_connector(Default::default())
    }
}

#[cfg(not(feature = "native-tls"))]
impl<C, T> Connector<T> for TlsConnector<C>
where
    T: service_async::Param<Option<crate::key::ServerName>>,
    C: Connector<T>,
    C::Connection: AsyncReadRent + AsyncWriteRent,
    C::Error: Into<crate::Error>,
{
    type Connection = TlsStream<C::Connection>;
    type Error = crate::Error;

    async fn connect(&self, key: T) -> Result<Self::Connection, Self::Error> {
        let server_name = key.param();
        if server_name.is_none() {
            return Err(Error::Validation("server_name not provided".to_string()));
        }
        let server_name = server_name.unwrap();

        let stream = self
            .inner_connector
            .connect(key)
            .await
            .map_err(Into::into)?;
        let tls_stream = self.tls_connector.connect(server_name, stream).await?;
        Ok(tls_stream)
    }
}

#[cfg(feature = "native-tls")]
impl<C, T> Connector<T> for TlsConnector<C>
where
    T: service_async::Param<Option<crate::key::ServerName>>,
    C: Connector<T, Error = std::io::Error>,
    C::Connection: AsyncReadRent + AsyncWriteRent,
{
    type Connection = TlsStream<C::Connection>;
    type Error = monoio_native_tls::TlsError;
    type ConnectionFuture<'a> = impl Future<Output = Result<Self::Connection, Self::Error>> + 'a where Self: 'a, T: 'a;

    fn connect(&self, key: T) -> Self::ConnectionFuture<'_> {
        let server_name = key.param();
        async move {
            let stream = self.inner_connector.connect(key).await?;
            let tls_stream = self.tls_connector.connect(&server_name.0, stream).await?;
            Ok(tls_stream)
        }
    }
}
