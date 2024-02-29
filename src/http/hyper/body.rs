use std::{cell::UnsafeCell, future::poll_fn, mem::MaybeUninit, pin::Pin};

use monoio::buf::IoBuf;
use monoio_compat::box_future::MaybeArmedBoxFuture;

// Still in very early stage.
pub struct HyperBody<B, T> {
    fut: MaybeArmedBoxFuture<T>,
    body: UnsafeCell<B>,
}

impl<B, T> HyperBody<B, T> {
    pub fn new(body: B) -> Self {
        Self {
            body: UnsafeCell::new(body),
            #[allow(clippy::uninit_assumed_init)]
            fut: MaybeArmedBoxFuture::new(async { unsafe { MaybeUninit::uninit().assume_init() } }),
        }
    }
}

impl<B> hyper::body::Body for HyperBody<B, Option<Result<B::Data, B::Error>>>
where
    B: monoio_http::common::body::Body + Unpin + 'static,
    B::Data: hyper::body::Buf,
{
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        if !this.fut.armed() {
            let body = unsafe { &mut *this.body.get() };
            this.fut.arm_future(body.next_data());
        }
        this.fut
            .poll(cx)
            .map(|r| r.map(|d| d.map(hyper::body::Frame::data)))
    }
}

// Still in very early stage.
#[derive(Debug, Clone)]
pub struct MonoioBody<B> {
    body: B,
}

impl<B> MonoioBody<B> {
    pub fn new(body: B) -> Self {
        Self { body }
    }
}

impl<B> monoio_http::common::body::Body for MonoioBody<B>
where
    B: hyper::body::Body,
    B::Data: IoBuf,
{
    type Data = B::Data;
    type Error = B::Error;

    async fn next_data(&mut self) -> Option<Result<Self::Data, Self::Error>> {
        loop {
            let ret =
                poll_fn(|cx| unsafe { Pin::new_unchecked(&mut self.body) }.poll_frame(cx)).await;
            match ret {
                None => return None,
                Some(Err(e)) => return Some(Err(e)),
                Some(Ok(frame)) => {
                    if frame.is_data() {
                        return Some(Ok(unsafe { frame.into_data().unwrap_unchecked() }));
                    } else {
                        continue;
                    }
                }
            }
        }
    }

    fn stream_hint(&self) -> monoio_http::common::body::StreamHint {
        monoio_http::common::body::StreamHint::Stream
    }
}
