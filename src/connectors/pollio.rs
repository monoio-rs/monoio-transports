//! Provides poll-based I/O abstractions for compatibility between Monoio and Hyper.
//!
//! This module offers unified I/O abstractions and polling mechanisms for TCP and Unix Domain
//! Sockets (UDS) that are compatible with Hyper's HTTP stack. It is specifically designed for
//! use with poll-based I/O streams, not the native io_uring I/O traits.
//!
//! Key components:
//!
//! - `UnifiedL4StreamPoll`: A unified stream enum for TCP and UDS that implements Tokio's I/O
//!   traits, essential for working with Hyper's HTTP stack.
//!
//! - `HyperIoWrapper`: A wrapper that implements Hyper I/O traits for types that implement Tokio's
//!   I/O traits, bridging the gap between Monoio and Hyper.
//!
//! - `PollIo`: A connector wrapper that converts Monoio-style connections into poll-based
//!   connections compatible with Hyper.
use std::{
    pin::Pin,
    task::{Context, Poll},
};

use monoio::io::IntoPollIo;
use thiserror::Error as ThisError;

use super::Connector;

//  They are required to work when using hyper' http stack
/// A unified stream for TCP and UDS which implement Tokio's IO traits.
///
/// This enum is required to work with Hyper's HTTP stack.
pub enum UnifiedL4StreamPoll {
    Tcp(monoio::net::tcp::stream_poll::TcpStreamPoll),
    Unix(monoio::net::unix::stream_poll::UnixStreamPoll),
}

impl IntoPollIo for super::UnifiedL4Stream {
    type PollIo = UnifiedL4StreamPoll;

    fn try_into_poll_io(self) -> Result<Self::PollIo, (std::io::Error, Self)> {
        match self {
            super::UnifiedL4Stream::Tcp(inner) => match inner.try_into_poll_io() {
                Ok(io) => Ok(UnifiedL4StreamPoll::Tcp(io)),
                Err((e, io)) => Err((e, super::UnifiedL4Stream::Tcp(io))),
            },
            super::UnifiedL4Stream::Unix(inner) => match inner.try_into_poll_io() {
                Ok(io) => Ok(UnifiedL4StreamPoll::Unix(io)),
                Err((e, io)) => Err((e, super::UnifiedL4Stream::Unix(io))),
            },
        }
    }
}

impl monoio::io::poll_io::AsyncRead for UnifiedL4StreamPoll {
    #[inline]
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut monoio::io::poll_io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        match this {
            UnifiedL4StreamPoll::Tcp(inner) => std::pin::Pin::new(inner).poll_read(cx, buf),
            UnifiedL4StreamPoll::Unix(inner) => std::pin::Pin::new(inner).poll_read(cx, buf),
        }
    }
}

impl monoio::io::poll_io::AsyncWrite for UnifiedL4StreamPoll {
    #[inline]
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        let this = self.get_mut();
        match this {
            UnifiedL4StreamPoll::Tcp(inner) => std::pin::Pin::new(inner).poll_write(cx, buf),
            UnifiedL4StreamPoll::Unix(inner) => std::pin::Pin::new(inner).poll_write(cx, buf),
        }
    }

    #[inline]
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        let this = self.get_mut();
        match this {
            UnifiedL4StreamPoll::Tcp(inner) => std::pin::Pin::new(inner).poll_flush(cx),
            UnifiedL4StreamPoll::Unix(inner) => std::pin::Pin::new(inner).poll_flush(cx),
        }
    }

    #[inline]
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        let this = self.get_mut();
        match this {
            UnifiedL4StreamPoll::Tcp(inner) => std::pin::Pin::new(inner).poll_shutdown(cx),
            UnifiedL4StreamPoll::Unix(inner) => std::pin::Pin::new(inner).poll_shutdown(cx),
        }
    }
}

// Borrowed from hyper-util
pin_project_lite::pin_project! {
    /// A wrapping implementing hyper IO traits for a type that
    /// implements Tokio's IO traits.
    #[derive(Debug)]
    pub struct HyperIoWrapper<T> {
        #[pin]
        inner: T,
    }
}

// ==== impl HyperIoWrapper =====
// Borrowed from hyper-util

impl<T> HyperIoWrapper<T> {
    /// Wrap a type implementing Tokio's IO traits.
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    /// Borrow the inner type.
    pub fn inner(&self) -> &T {
        &self.inner
    }

    /// Mut borrow the inner type.
    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Consume this wrapper and get the inner type.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T> hyper::rt::Read for HyperIoWrapper<T>
where
    T: monoio::io::poll_io::AsyncRead,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mut buf: hyper::rt::ReadBufCursor<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        let n = unsafe {
            let mut tbuf = monoio::io::poll_io::ReadBuf::uninit(buf.as_mut());
            match monoio::io::poll_io::AsyncRead::poll_read(self.project().inner, cx, &mut tbuf) {
                Poll::Ready(Ok(())) => tbuf.filled().len(),
                other => return other,
            }
        };

        unsafe {
            buf.advance(n);
        }
        Poll::Ready(Ok(()))
    }
}

impl<T> hyper::rt::Write for HyperIoWrapper<T>
where
    T: monoio::io::poll_io::AsyncWrite,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        monoio::io::poll_io::AsyncWrite::poll_write(self.project().inner, cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        monoio::io::poll_io::AsyncWrite::poll_flush(self.project().inner, cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        monoio::io::poll_io::AsyncWrite::poll_shutdown(self.project().inner, cx)
    }

    fn is_write_vectored(&self) -> bool {
        monoio::io::poll_io::AsyncWrite::is_write_vectored(&self.inner)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<Result<usize, std::io::Error>> {
        monoio::io::poll_io::AsyncWrite::poll_write_vectored(self.project().inner, cx, bufs)
    }
}

impl<T> monoio::io::poll_io::AsyncRead for HyperIoWrapper<T>
where
    T: hyper::rt::Read,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        tbuf: &mut monoio::io::poll_io::ReadBuf<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        // let init = tbuf.initialized().len();
        let filled = tbuf.filled().len();
        let sub_filled = unsafe {
            let mut buf = hyper::rt::ReadBuf::uninit(tbuf.unfilled_mut());

            match hyper::rt::Read::poll_read(self.project().inner, cx, buf.unfilled()) {
                Poll::Ready(Ok(())) => buf.filled().len(),
                other => return other,
            }
        };

        let n_filled = filled + sub_filled;
        // At least sub_filled bytes had to have been initialized.
        let n_init = sub_filled;
        unsafe {
            tbuf.assume_init(n_init);
            tbuf.set_filled(n_filled);
        }

        Poll::Ready(Ok(()))
    }
}

impl<T> monoio::io::poll_io::AsyncWrite for HyperIoWrapper<T>
where
    T: hyper::rt::Write,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        hyper::rt::Write::poll_write(self.project().inner, cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        hyper::rt::Write::poll_flush(self.project().inner, cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        hyper::rt::Write::poll_shutdown(self.project().inner, cx)
    }

    fn is_write_vectored(&self) -> bool {
        hyper::rt::Write::is_write_vectored(&self.inner)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<Result<usize, std::io::Error>> {
        hyper::rt::Write::poll_write_vectored(self.project().inner, cx, bufs)
    }
}

#[derive(ThisError, Debug)]
pub enum PollConnectError<CE> {
    #[error("Connect error")]
    Connect(CE),
    #[error("Convert error {0}")]
    Convert(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy)]
pub struct PollIo<C>(pub C);

impl<C, K, T> Connector<K> for PollIo<C>
where
    C: Connector<K, Connection = T>,
    T: IntoPollIo,
{
    type Connection = HyperIoWrapper<T::PollIo>;
    type Error = PollConnectError<C::Error>;

    async fn connect(&self, key: K) -> Result<Self::Connection, Self::Error> {
        let io = self
            .0
            .connect(key)
            .await
            .map_err(PollConnectError::Connect)?;
        let poll_io = io.into_poll_io().map_err(PollConnectError::Convert)?;
        Ok(HyperIoWrapper::new(poll_io))
    }
}
