use bytes::Bytes;
use http::{Request, Response};
use monoio::io::{sink::SinkExt, stream::Stream, AsyncReadRent, AsyncWriteRent};
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
    H1(ClientCodec<IO>, bool),
}

impl<IO: AsyncWriteRent> Poolable for HttpConnection<IO> {
    #[inline]
    fn is_open(&self) -> bool {
        match self {
            Self::H1(_, open) => *open,
        }
    }
}

impl<IO: AsyncReadRent + AsyncWriteRent> HttpConnection<IO> {
    pub async fn send_request<B>(
        &mut self,
        request: Request<B>,
    ) -> (crate::Result<Response<HttpBody>>, bool)
    where
        B: Body<Data = Bytes, Error = HttpError> + 'static,
    {
        match self {
            Self::H1(handle, open) => {
                if let Err(e) = handle.send_and_flush(request).await {
                    #[cfg(feature = "logging")]
                    tracing::error!("send upstream request error {:?}", e);
                    *open = false;
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
                                            tracing::error!(
                                                "decode upstream response error {:?}",
                                                e
                                            );
                                            *open = false;
                                            return (Err(e.into()), false);
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
                        *open = false;
                        (Err(e.into()), false)
                    }
                    None => {
                        #[cfg(feature = "logging")]
                        tracing::error!("upstream return eof");
                        *open = false;
                        (Err(DecodeError::UnexpectedEof.into()), false)
                    }
                }
            }
        }
    }
}
