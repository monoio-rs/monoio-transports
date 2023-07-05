use bytes::Bytes;
use futures_core::Future;
use monoio::buf::IoBuf;

use super::error::HttpError;
use crate::{h1::payload::FramedPayloadRecvr, h2::RecvStream};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamHint {
    None,
    Fixed,
    Stream,
}

pub trait Body {
    type Data: IoBuf;
    type Error;
    type DataFuture<'a>: Future<Output = Option<Result<Self::Data, Self::Error>>>
    where
        Self: 'a;

    fn next_data(&mut self) -> Self::DataFuture<'_>;
    fn stream_hint(&self) -> StreamHint;
}

pub enum HttpBody {
    Ready(Option<Bytes>),
    H1(FramedPayloadRecvr),
    H2(RecvStream),
}

impl From<FramedPayloadRecvr> for HttpBody {
    fn from(p: FramedPayloadRecvr) -> Self {
        Self::H1(p)
    }
}

impl From<RecvStream> for HttpBody {
    fn from(p: RecvStream) -> Self {
        Self::H2(p)
    }
}

impl Default for HttpBody {
    fn default() -> Self {
        Self::Ready(None)
    }
}

impl Body for HttpBody {
    type Data = Bytes;
    type Error = HttpError;
    type DataFuture<'a> = impl Future<Output = Option<Result<Self::Data, Self::Error>>> + 'a where
        Self: 'a;

    fn next_data(&mut self) -> Self::DataFuture<'_> {
        async move {
            match self {
                Self::Ready(b) => b.take().map(Result::Ok),
                Self::H1(ref mut p) => p.next_data().await.map(|r| r.map_err(HttpError::from)),
                Self::H2(ref mut p) => p.next_data().await.map(|r| r.map_err(HttpError::from)),
            }
        }
    }

    fn stream_hint(&self) -> StreamHint {
        match self {
            Self::Ready(Some(_)) => StreamHint::Fixed,
            Self::Ready(None) => StreamHint::None,
            Self::H1(ref p) => p.stream_hint(),
            Self::H2(ref p) => p.stream_hint(),
        }
    }
}

pub trait FixedBody {
    type BodyType: Body;
    fn fixed_body(data: Option<Bytes>) -> Self::BodyType;
}

impl FixedBody for HttpBody {
    type BodyType = Self;
    fn fixed_body(data: Option<Bytes>) -> Self::BodyType {
        Self::Ready(data)
    }
}
