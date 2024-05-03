use thiserror::Error as ThisError;

#[derive(ThisError, Debug)]
pub enum TransportError {
    #[error("convert from uri error {0}")]
    FromUri(#[from] FromUriError),
    #[error("http header error")]
    Http(#[from] http::Error),
    #[error("decode error {0}")]
    H1Decode(#[from] monoio_http::h1::codec::decoder::DecodeError),
    #[error("io error {0}")]
    Io(#[from] std::io::Error),
    #[cfg(not(feature = "native-tls"))]
    #[error("rustls error {0}")]
    Rustls(#[from] monoio_rustls::TlsError),
    #[cfg(feature = "native-tls")]
    #[error("native-tls error {0}")]
    NativeTls(#[from] monoio_native_tls::TlsError),
    #[error("serde_json error {0}")]
    Json(#[from] serde_json::Error),
    #[error("H2 error {0}")]
    H2Error(#[from] monoio_http::h2::Error),
    #[error("Resp Recv from connection manager failed {0}")]
    ConnManagerRespRecvError(#[from] local_sync::oneshot::error::RecvError),
    #[error("Recv from conn manager failed")]
    ConnManagerReqSendError,
    #[error("Conn Manager marked this conn for close")]
    ClosePooledConnection,
    #[error("Http crate error {0}")]
    HttpError(#[from] monoio_http::common::error::HttpError),
    #[error("Codec missing from PooledConnection")]
    MissingCodec,
    #[error("Validation error {0}")]
    Validation(String),
    #[error("Acquire lock error {0}")]
    LockError(#[from] local_sync::semaphore::AcquireError),
}

pub type Result<T> = std::result::Result<T, TransportError>;

#[derive(ThisError, Debug)]
pub enum FromUriError {
    #[error("Invalid dns name {0}")]
    InvalidDnsName(#[from] rustls::pki_types::InvalidDnsNameError),
    #[error("Scheme not supported")]
    UnsupportScheme,
    #[error("Missing authority in uri")]
    NoAuthority,
    #[error("resolve error {0}")]
    Resolve(#[from] std::io::Error),
    #[error("no resolve result")]
    NoResolve,
}
