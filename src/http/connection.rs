use bytes::Bytes;
use http::Response;
use monoio::io::{
    sink::{Sink, SinkExt},
    stream::Stream,
    AsyncReadRent, AsyncWriteRent,
};
use monoio_http::{
    common::{
        body::{Body, HttpBody},
        error::HttpError,
        request::{Request, RequestHead},
        IntoParts,
    },
    h1::{
        codec::{
            decoder::{DecodeError, PayloadDecoder},
            ClientCodec,
        },
        payload::{fixed_payload_pair, stream_payload_pair, Payload},
    },
    h2::client::SendRequest,
};

use crate::pool::{Key, Poolable, Pooled};

pub struct Http1Connection<IO: AsyncWriteRent> {
    framed: ClientCodec<IO>,
    using: bool,
    open: bool,
}

impl<IO: AsyncWriteRent> Http1Connection<IO> {
    pub fn new(framed: ClientCodec<IO>) -> Self {
        Self {
            framed,
            using: false,
            open: true,
        }
    }
}

impl<IO: AsyncWriteRent> Poolable for Http1Connection<IO> {
    #[inline]
    fn is_open(&self) -> bool {
        match self {
            Self { using, open, .. } => *open && !*using,
        }
    }
}

impl<IO: AsyncReadRent + AsyncWriteRent> Http1Connection<IO> {
    pub async fn send_request<R, E>(
        &mut self,
        request: R,
    ) -> (Result<Response<HttpBody>, HttpError>, bool)
    where
        ClientCodec<IO>: Sink<R, Error = E>,
        E: std::fmt::Debug + Into<HttpError>,
    {
        let handle = &mut self.framed;

        if let Err(e) = handle.send_and_flush(request).await {
            #[cfg(feature = "logging")]
            tracing::error!("send upstream request error {:?}", e);
            self.open = false;
            return (Err(e.into()), false);
        }

        match handle.next().await {
            Some(Ok(resp)) => {
                let (parts, payload_decoder) = resp.into_parts();
                match payload_decoder {
                    PayloadDecoder::None => {
                        let payload = Payload::None;
                        let response = Response::from_parts(parts, payload.into());
                        (Ok(response), false)
                    }
                    PayloadDecoder::Fixed(_) => {
                        let mut framed_payload = payload_decoder.with_io(handle);
                        let (payload, payload_sender) = fixed_payload_pair();
                        if let Some(data) = framed_payload.next_data().await {
                            payload_sender.feed(data)
                        }
                        let payload = Payload::Fixed(payload);
                        let response = Response::from_parts(parts, payload.into());
                        (Ok(response), false)
                    }
                    PayloadDecoder::Streamed(_) => {
                        let mut framed_payload = payload_decoder.with_io(handle);
                        let (payload, mut payload_sender) = stream_payload_pair();
                        loop {
                            match framed_payload.next_data().await {
                                Some(Ok(data)) => payload_sender.feed_data(Some(data)),
                                Some(Err(e)) => {
                                    #[cfg(feature = "logging")]
                                    tracing::error!("decode upstream response error {:?}", e);
                                    self.open = false;
                                    return (Err(e), false);
                                }
                                None => {
                                    payload_sender.feed_data(None);
                                    break;
                                }
                            }
                        }
                        let payload = Payload::Stream(payload);
                        let response = Response::from_parts(parts, payload.into());
                        (Ok(response), false)
                    }
                }
            }
            Some(Err(e)) => {
                #[cfg(feature = "logging")]
                tracing::error!("decode upstream response error {:?}", e);
                self.open = false;
                (Err(e), false)
            }
            None => {
                #[cfg(feature = "logging")]
                tracing::error!("upstream return eof");
                self.open = false;
                (Err(DecodeError::UnexpectedEof.into()), false)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Http2Connection {
    tx: SendRequest<Bytes>,
}

impl Poolable for Http2Connection {
    #[inline]
    fn is_open(&self) -> bool {
        !self.tx.has_conn_error()
    }
}

impl Http2Connection {
    pub fn new(tx: SendRequest<Bytes>) -> Self {
        Self { tx }
    }

    #[allow(dead_code)]
    fn to_owned(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }

    pub fn conn_error(&self) -> Option<HttpError> {
        self.tx.conn_error()
    }
}

impl Http2Connection {
    pub async fn send_request<R>(
        &mut self,
        request: R,
    ) -> (Result<Response<HttpBody>, HttpError>, bool)
    where
        R: IntoParts<Parts = RequestHead>,
        R::Body: Body<Data = Bytes, Error = HttpError>,
    {
        let mut client = match self.tx.clone().ready().await {
            Ok(client) => client,
            Err(e) => {
                return (Err(e.into()), false);
            }
        };

        let (parts, mut body) = request.into_parts();
        let h2_request = Request::from_parts(parts, ());

        let (response, mut send_stream) = match client.send_request(h2_request, false) {
            Ok((response, send_stream)) => (response, send_stream),
            Err(e) => {
                return (Err(e.into()), false);
            }
        };

        while let Some(data) = body.next_data().await {
            match data {
                Ok(data) => {
                    if let Err(e) = send_stream.send_data(data, false) {
                        #[cfg(feature = "logging")]
                        tracing::error!("H2 client body send error {:?}", e);
                        return (Err(e.into()), false);
                    }
                }
                Err(e) => {
                    #[cfg(feature = "logging")]
                    tracing::error!("H2 request body stream error {:?}", e);
                    return (Err(e), false);
                }
            }
        }
        // Mark end of stream
        let _ = send_stream.send_data(Bytes::new(), true);

        let response = match response.await {
            Ok(response) => response,
            Err(e) => {
                #[cfg(feature = "logging")]
                tracing::error!("H2 client response error {:?}", e);
                return (Err(e.into()), false);
            }
        };

        let (parts, body) = response.into_parts();
        (Ok(Response::from_parts(parts, body.into())), true)
    }
}

pub enum HttpConnection<K: Key, IO: AsyncReadRent + AsyncWriteRent> {
    Http1(Pooled<K, Http1Connection<IO>>),
    Http2(Http2Connection),
}

impl<K: Key, IO: AsyncWriteRent + AsyncReadRent> Poolable for HttpConnection<K, IO> {
    #[inline]
    fn is_open(&self) -> bool {
        match self {
            Self::Http1(conn) => conn.is_open(),
            Self::Http2(conn) => conn.is_open(),
        }
    }
}

impl<K: Key, IO: AsyncReadRent + AsyncWriteRent> From<Pooled<K, Http1Connection<IO>>>
    for HttpConnection<K, IO>
{
    fn from(pooled_conn: Pooled<K, Http1Connection<IO>>) -> Self {
        Self::Http1(pooled_conn)
    }
}

impl<K: Key, IO: AsyncReadRent + AsyncWriteRent> From<Http2Connection> for HttpConnection<K, IO> {
    fn from(conn: Http2Connection) -> Self {
        Self::Http2(conn)
    }
}

impl<K: Key, IO: AsyncReadRent + AsyncWriteRent> HttpConnection<K, IO> {
    pub async fn send_request<R, E>(
        &mut self,
        request: R,
    ) -> (Result<Response<HttpBody>, HttpError>, bool)
    where
        ClientCodec<IO>: Sink<R, Error = E>,
        E: std::fmt::Debug + Into<HttpError>,
        R: IntoParts<Parts = RequestHead>,
        R::Body: Body<Data = Bytes, Error = HttpError>,
    {
        match self {
            Self::Http1(conn) => conn.send_request(request).await,
            Self::Http2(conn) => conn.send_request(request).await,
        }
    }
}
