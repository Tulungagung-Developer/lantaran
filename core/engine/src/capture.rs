//! Records bodies while they are forwarded.
//!
//! A [`Tap`] wraps a body on its way through the proxy. Every frame passes
//! through unchanged and as soon as it arrives, so the first byte never waits
//! for the last; a copy of the data is kept up to a limit. When the body
//! ends, or is dropped, the copy is stored and the flow is told.

use crate::{CapturedBody, Direction, Error, FlowEvent, FlowId};
use bytes::{Bytes, BytesMut};
use http_body_util::{BodyExt, Full, Limited, combinators::BoxBody};
use hudsucker::{
    Body,
    hyper::{
        HeaderMap, Response,
        body::{Body as HttpBody, Frame, SizeHint},
        header::CONTENT_ENCODING,
    },
};
use std::{
    collections::HashMap,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::Instant,
};

/// The most a body may grow to when decoded for display. A few kilobytes of
/// compressed data can expand to gigabytes, so decoding stops here.
pub const MAX_DECODED_SIZE: usize = 64 * 1024 * 1024;

/// What a flow's two bodies share: when it started and how big the request
/// body turned out to be, which the response side reports.
pub(crate) struct FlowTracker {
    pub(crate) id: FlowId,
    pub(crate) started: Instant,
    request_size: AtomicU64,
}

impl FlowTracker {
    pub(crate) fn new(id: FlowId) -> Arc<Self> {
        Arc::new(Self {
            id,
            started: Instant::now(),
            request_size: AtomicU64::new(0),
        })
    }
}

/// Captured bodies, by flow and direction.
#[derive(Default)]
pub(crate) struct BodyStore {
    bodies: Mutex<HashMap<(FlowId, Direction), CapturedBody>>,
}

impl BodyStore {
    pub(crate) fn get(&self, id: FlowId, direction: Direction) -> Option<CapturedBody> {
        self.lock().get(&(id, direction)).cloned()
    }

    fn insert(&self, id: FlowId, direction: Direction, body: CapturedBody) {
        self.lock().insert((id, direction), body);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<(FlowId, Direction), CapturedBody>> {
        self.bodies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Where a finished tap reports to.
pub(crate) trait TapSink: Send + Sync + 'static {
    fn store(&self) -> &BodyStore;
    fn emit(&self, event: FlowEvent);
}

pub(crate) struct Tap<S: TapSink> {
    inner: Body,
    state: Option<TapState<S>>,
}

struct TapState<S: TapSink> {
    sink: Arc<S>,
    flow: Arc<FlowTracker>,
    direction: Direction,
    limit: usize,
    kept: BytesMut,
    size: u64,
    truncated: bool,
    content_encoding: Option<String>,
}

impl<S: TapSink> Tap<S> {
    /// Wraps `body` and returns the wrapped body, ready to forward.
    pub(crate) fn wrap(
        body: Body,
        headers: &HeaderMap,
        sink: Arc<S>,
        flow: Arc<FlowTracker>,
        direction: Direction,
        limit: usize,
    ) -> Body {
        let content_encoding = headers
            .get(CONTENT_ENCODING)
            .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned());

        let tap = Self {
            inner: body,
            state: Some(TapState {
                sink,
                flow,
                direction,
                limit,
                kept: BytesMut::new(),
                size: 0,
                truncated: false,
                content_encoding,
            }),
        };
        Body::from(BoxBody::new(tap))
    }

    fn finish(&mut self, outcome: Outcome) {
        let Some(state) = self.state.take() else {
            return;
        };
        let id = state.flow.id;

        state.sink.store().insert(
            id,
            state.direction,
            CapturedBody {
                bytes: state.kept.freeze(),
                size: state.size,
                truncated: state.truncated,
                incomplete: !matches!(outcome, Outcome::Ended),
                content_encoding: state.content_encoding,
            },
        );

        match state.direction {
            Direction::Request => state.flow.request_size.store(state.size, Ordering::Relaxed),
            Direction::Response => state.sink.emit(match outcome {
                Outcome::Ended => FlowEvent::Completed {
                    id,
                    request_size: state.flow.request_size.load(Ordering::Relaxed),
                    response_size: state.size,
                    duration: state.flow.started.elapsed(),
                },
                Outcome::Errored(error) => FlowEvent::Failed {
                    id,
                    error: format!("the response body was cut off: {error}"),
                },
                Outcome::Dropped => FlowEvent::Failed {
                    id,
                    error: "the client closed the connection before the response was delivered"
                        .to_owned(),
                },
            }),
        }
    }
}

enum Outcome {
    Ended,
    Errored(String),
    Dropped,
}

impl<S: TapSink> HttpBody for Tap<S> {
    type Data = Bytes;
    type Error = hudsucker::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let polled = Pin::new(&mut self.inner).poll_frame(cx);

        match &polled {
            Poll::Ready(Some(Ok(frame))) => {
                if let (Some(data), Some(state)) = (frame.data_ref(), self.state.as_mut()) {
                    state.size += data.len() as u64;
                    let room = state.limit.saturating_sub(state.kept.len());
                    if data.len() > room {
                        state.truncated = true;
                    }
                    state.kept.extend_from_slice(&data[..data.len().min(room)]);
                }
            }
            Poll::Ready(Some(Err(error))) => {
                let error = describe(error);
                self.finish(Outcome::Errored(error));
            }
            Poll::Ready(None) => self.finish(Outcome::Ended),
            Poll::Pending => {}
        }

        polled
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

impl<S: TapSink> Drop for Tap<S> {
    /// hyper stops polling a body once `is_end_stream` says it is done, so a
    /// complete body can be dropped without ever returning its final `None`.
    fn drop(&mut self) {
        let outcome = if self.inner.is_end_stream() {
            Outcome::Ended
        } else {
            Outcome::Dropped
        };
        self.finish(outcome);
    }
}

impl CapturedBody {
    /// The body with its `Content-Encoding` undone, for display.
    ///
    /// Fails when the encoding is unknown, when the captured bytes are not a
    /// complete compressed stream (as happens when the body was truncated), or
    /// when the result would exceed [`MAX_DECODED_SIZE`].
    pub async fn decoded(&self) -> Result<Bytes, Error> {
        let Some(encoding) = &self.content_encoding else {
            return Ok(self.bytes.clone());
        };

        let mut message = Response::new(Body::from(Full::new(self.bytes.clone())));
        message.headers_mut().insert(
            CONTENT_ENCODING,
            encoding.parse().map_err(|_| Error::Decode)?,
        );

        let decoded = hudsucker::decode_response(message).map_err(|_| Error::Decode)?;
        let collected = Limited::new(decoded.into_body(), MAX_DECODED_SIZE)
            .collect()
            .await
            .map_err(|_| Error::Decode)?;
        Ok(collected.to_bytes())
    }
}

pub(crate) fn describe(err: &(dyn std::error::Error + 'static)) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
