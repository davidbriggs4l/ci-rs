use futures_core::Stream;
use hyper::body::Body;
use hyper::body::Buf;
use hyper::body::Bytes;
use hyper::body::Incoming;
use hyper::upgrade::Upgraded;
use pin_project_lite::pin_project;
use serde::de::DeserializeOwned;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{cmp, io, marker::PhantomData};
use tokio_util::bytes::BytesMut;

use tokio::io::AsyncWrite;
use tokio::io::{AsyncRead, ReadBuf};
use tokio_util::codec::Decoder;

use super::utils::LogOutput;

use super::errors::Error;
use super::errors::Error::JsonDataError;

#[derive(Debug, Copy, Clone)]
enum NewlineLogOutputDecoderState {
    WaitingHeader,
    WaitingPayload(u8, usize), // StreamType, Length
}

#[derive(Debug, Copy, Clone)]
pub(crate) struct NewlineLogOutputDecoder {
    state: NewlineLogOutputDecoderState,
    is_tcp: bool,
}

impl NewlineLogOutputDecoder {
    pub(crate) fn new(is_tcp: bool) -> NewlineLogOutputDecoder {
        NewlineLogOutputDecoder {
            state: NewlineLogOutputDecoderState::WaitingHeader,
            is_tcp,
        }
    }
}

impl Decoder for NewlineLogOutputDecoder {
    type Item = LogOutput;
    type Error = std::io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        loop {
            match self.state {
                NewlineLogOutputDecoderState::WaitingHeader => {
                    // `start_exec` API on unix socket will emit values without a header
                    if !src.is_empty() && src[0] > 2 {
                        if self.is_tcp {
                            return Ok(Some(LogOutput::Console {
                                message: src.split().freeze(),
                            }));
                        }
                        let nl_index = src.iter().position(|b| *b == b'\n');
                        if let Some(pos) = nl_index {
                            return Ok(Some(LogOutput::Console {
                                message: src.split_to(pos + 1).freeze(),
                            }));
                        } else {
                            return Ok(None);
                        }
                    }

                    if src.len() < 8 {
                        return Ok(None);
                    }

                    let header = src.split_to(8);
                    let length =
                        u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
                    self.state = NewlineLogOutputDecoderState::WaitingPayload(header[0], length);
                }
                NewlineLogOutputDecoderState::WaitingPayload(typ, length) => {
                    if src.len() < length {
                        return Ok(None);
                    } else {
                        // trace!("NewlineLogOutputDecoder: Reading payload");
                        let message = src.split_to(length).freeze();
                        let item = match typ {
                            0 => LogOutput::StdIn { message },
                            1 => LogOutput::StdOut { message },
                            2 => LogOutput::StdErr { message },
                            _ => unreachable!(),
                        };

                        self.state = NewlineLogOutputDecoderState::WaitingHeader;
                        return Ok(Some(item));
                    }
                }
            }
        }
    }
}

pin_project! {
    #[derive(Debug)]
    pub(crate) struct JsonLineDecoder<T> {
        ty: PhantomData<T>,
    }
}

impl<T> JsonLineDecoder<T> {
    #[inline]
    pub(crate) fn new() -> JsonLineDecoder<T> {
        JsonLineDecoder { ty: PhantomData }
    }
}

fn decode_json_from_slice<T: DeserializeOwned>(slice: &[u8]) -> Result<Option<T>, Error> {
    // debug!(
    //     "Decoding JSON line from stream: {}",
    //     String::from_utf8_lossy(slice).to_string()
    // );

    match serde_json::from_slice(slice) {
        Ok(json) => Ok(json),
        Err(ref e) if e.is_data() => Err(JsonDataError {
            message: e.to_string(),
            column: e.column(),
            contents: String::from_utf8_lossy(slice).to_string(),
        }),
        Err(e) if e.is_eof() => Ok(None),
        Err(e) => Err(e.into()),
    }
}

impl<T> Decoder for JsonLineDecoder<T>
where
    T: DeserializeOwned,
{
    type Item = T;
    type Error = Error;
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let nl_index = src.iter().position(|b| *b == b'\n');

        if !src.is_empty() {
            if let Some(pos) = nl_index {
                let remainder = src.split_off(pos + 1);
                let slice = &src[..src.len() - 1];

                match decode_json_from_slice(slice) {
                    Ok(None) => {
                        // Unescaped newline inside the json structure
                        src.truncate(src.len() - 1); // Remove the newline
                        src.unsplit(remainder);
                        Ok(None)
                    }
                    Ok(json) => {
                        // Newline delimited json
                        src.unsplit(remainder);
                        src.advance(pos + 1);
                        Ok(json)
                    }
                    Err(e) => Err(e),
                }
            } else {
                // No newline delimited json.
                match decode_json_from_slice(src) {
                    Ok(None) => Ok(None),
                    Ok(json) => {
                        src.clear();
                        Ok(json)
                    }
                    Err(e) => Err(e),
                }
            }
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug)]
enum ReadState {
    Ready(Bytes, usize),
    NotReady,
}

pin_project! {
    #[derive(Debug)]
    pub(crate) struct StreamReader {
        #[pin]
        stream: Incoming,
        state: ReadState,
    }
}

impl StreamReader {
    #[inline]
    pub(crate) fn new(stream: Incoming) -> StreamReader {
        StreamReader {
            stream,
            state: ReadState::NotReady,
        }
    }
}

impl AsyncRead for StreamReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        read_buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            match self.as_mut().project().state {
                ReadState::Ready(ref mut chunk, ref mut pos) => {
                    let chunk_start = *pos;
                    let buf = read_buf.initialize_unfilled();
                    let len = cmp::min(buf.len(), chunk.len() - chunk_start);
                    let chunk_end = chunk_start + len;

                    buf[..len].copy_from_slice(&chunk[chunk_start..chunk_end]);
                    *pos += len;
                    read_buf.advance(len);

                    if *pos != chunk.len() {
                        return Poll::Ready(Ok(()));
                    }
                }

                ReadState::NotReady => match self.as_mut().project().stream.poll_frame(cx) {
                    Poll::Ready(Some(Ok(frame))) if frame.is_data() => {
                        *self.as_mut().project().state =
                            ReadState::Ready(frame.into_data().unwrap(), 0);

                        continue;
                    }
                    Poll::Ready(Some(Ok(_frame))) => return Poll::Ready(Ok(())),
                    Poll::Ready(None) => return Poll::Ready(Ok(())),
                    Poll::Pending => {
                        return Poll::Pending;
                    }
                    Poll::Ready(Some(Err(e))) => {
                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::Other,
                            e.to_string(),
                        )));
                    }
                },
            }

            *self.as_mut().project().state = ReadState::NotReady;

            return Poll::Ready(Ok(()));
        }
    }
}

pin_project! {
    #[derive(Debug)]
    pub(crate) struct AsyncUpgraded {
        #[pin]
        inner: Upgraded,
    }
}

impl AsyncUpgraded {
    pub(crate) fn new(upgraded: Upgraded) -> Self {
        Self { inner: upgraded }
    }
}

impl AsyncRead for AsyncUpgraded {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        read_buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let n = {
            let mut hbuf = hyper::rt::ReadBuf::new(read_buf.initialize_unfilled());
            match hyper::rt::Read::poll_read(self.project().inner, cx, hbuf.unfilled()) {
                Poll::Ready(Ok(())) => hbuf.filled().len(),
                other => return other,
            }
        };
        read_buf.advance(n);

        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for AsyncUpgraded {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        hyper::rt::Write::poll_write(self.project().inner, cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        hyper::rt::Write::poll_flush(self.project().inner, cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        hyper::rt::Write::poll_shutdown(self.project().inner, cx)
    }
}

pin_project! {
    #[derive(Debug)]
    pub(crate) struct IncomingStream {
        #[pin]
        inner: Incoming,
    }
}

impl IncomingStream {
    pub(crate) fn new(incoming: Incoming) -> Self {
        Self { inner: incoming }
    }
}

impl Stream for IncomingStream {
    type Item = Result<Bytes, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match futures_util::ready!(self.as_mut().project().inner.poll_frame(cx)?) {
            Some(frame) => match frame.into_data() {
                Ok(data) => Poll::Ready(Some(Ok(data))),
                Err(_) => Poll::Ready(None),
            },
            None => Poll::Ready(None),
        }
    }
}
