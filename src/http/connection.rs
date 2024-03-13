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
    },
    h1::{
        codec::{
            decoder::{DecodeError, PayloadDecoder},
            ClientCodec,
        },
        payload::{fixed_payload_pair, stream_payload_pair, Payload},
    },
};

use crate::pool::Poolable;

pub enum HttpConnection<IO: AsyncWriteRent> {
    H1 {
        framed: ClientCodec<IO>,
        using: bool,
        open: bool,
    },
}

impl<IO: AsyncWriteRent> Poolable for HttpConnection<IO> {
    #[inline]
    fn is_open(&self) -> bool {
        match self {
            Self::H1 { using, open, .. } => *open && !*using,
        }
    }
}

impl<IO: AsyncReadRent + AsyncWriteRent> HttpConnection<IO> {
    pub async fn send_request<R, E>(
        &mut self,
        request: R,
    ) -> (Result<Response<HttpBody>, HttpError>, bool)
    where
        ClientCodec<IO>: Sink<R, Error = E>,
        E: std::fmt::Debug + Into<HttpError>,
    {
        match self {
            Self::H1 {
                framed,
                using,
                open,
            } => {
                *using = true;
                if let Err(e) = framed.send_and_flush(request).await {
                    #[cfg(feature = "logging")]
                    tracing::error!("send upstream request error {:?}", e);
                    *open = false;
                    *using = false;
                    return (Err(e.into()), false);
                }

                match framed.next().await {
                    Some(Ok(resp)) => {
                        let (parts, payload_decoder) = resp.into_parts();
                        match payload_decoder {
                            PayloadDecoder::None => {
                                *using = false;
                                let payload = Payload::None;
                                let response = Response::from_parts(parts, payload.into());
                                (Ok(response), false)
                            }
                            PayloadDecoder::Fixed(_) => {
                                let mut framed_payload = payload_decoder.with_io(framed);
                                let (payload, payload_sender) = fixed_payload_pair();
                                if let Some(data) = framed_payload.next_data().await {
                                    payload_sender.feed(data)
                                }
                                *using = false;
                                let payload = Payload::Fixed(payload);
                                let response = Response::from_parts(parts, payload.into());
                                (Ok(response), false)
                            }
                            PayloadDecoder::Streamed(_) => {
                                let mut framed_payload = payload_decoder.with_io(framed);
                                let (payload, mut payload_sender) = stream_payload_pair();
                                loop {
                                    match framed_payload.next_data().await {
                                        Some(Ok(data)) => payload_sender.feed_data(Some(data)),
                                        Some(Err(e)) => {
                                            #[cfg(feature = "logging")]
                                            tracing::error!(
                                                "decode upstream response error {:?}",
                                                e
                                            );
                                            *open = false;
                                            return (Err(e), false);
                                        }
                                        None => {
                                            payload_sender.feed_data(None);
                                            break;
                                        }
                                    }
                                }
                                *using = false;
                                let payload = Payload::Stream(payload);
                                let response = Response::from_parts(parts, payload.into());
                                (Ok(response), false)
                            }
                        }
                    }
                    Some(Err(e)) => {
                        #[cfg(feature = "logging")]
                        tracing::error!("decode upstream response error {:?}", e);
                        *open = false;
                        *using = false;
                        (Err(e), false)
                    }
                    None => {
                        #[cfg(feature = "logging")]
                        tracing::error!("upstream return eof");
                        *open = false;
                        *using = false;
                        (Err(DecodeError::UnexpectedEof.into()), false)
                    }
                }
            }
        }
    }
}
