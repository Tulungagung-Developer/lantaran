//! The bounded, batched stream of flow events.
//!
//! Forwarding traffic must never wait for whoever is reading events, and a
//! reader that falls behind must not make the proxy's memory grow without
//! limit. Events go into a queue of fixed capacity; when it is full, new
//! events are dropped and counted, and the reader is told how many it missed.
//! The reader takes events in batches, which suits a user interface that
//! redraws once per frame.

use crate::FlowEvent;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;

/// How many events wait for the reader before new ones are dropped.
pub const DEFAULT_EVENT_CAPACITY: usize = 100_000;

/// The most events in one batch.
pub const MAX_BATCH: usize = 500;

/// How long a batch waits for more events after the first one arrives.
pub const BATCH_WINDOW: Duration = Duration::from_millis(50);

/// Events delivered together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventBatch {
    pub events: Vec<FlowEvent>,
    /// Events dropped since the previous batch because the reader fell
    /// behind. Their flows may later produce events whose earlier ones are
    /// missing.
    pub dropped: u64,
}

pub(crate) struct EventQueue {
    events: Mutex<VecDeque<FlowEvent>>,
    capacity: usize,
    dropped: AtomicU64,
    closed: AtomicBool,
    ready: Notify,
}

impl EventQueue {
    pub(crate) fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            events: Mutex::new(VecDeque::new()),
            capacity,
            dropped: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            ready: Notify::new(),
        })
    }

    /// Adds an event without ever waiting. Drops it when the queue is full.
    pub(crate) fn push(&self, event: FlowEvent) {
        {
            let mut events = self.lock();
            if events.len() >= self.capacity {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                return;
            }
            events.push_back(event);
        }
        self.ready.notify_one();
    }

    /// Marks the queue as finished: the reader gets what is left, then `None`.
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.ready.notify_one();
    }

    fn len(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<FlowEvent>> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The reading end of an engine's events.
pub struct EventStream {
    queue: Arc<EventQueue>,
}

impl EventStream {
    pub(crate) fn new(queue: Arc<EventQueue>) -> Self {
        Self { queue }
    }

    /// Waits for the next batch. Returns `None` once the engine is gone and
    /// every event has been read.
    pub async fn next_batch(&mut self) -> Option<EventBatch> {
        // Wait for the first event.
        loop {
            let ready = self.queue.ready.notified();
            tokio::pin!(ready);
            ready.as_mut().enable();

            if self.queue.len() > 0 || self.queue.dropped.load(Ordering::Relaxed) > 0 {
                break;
            }
            if self.queue.closed.load(Ordering::Relaxed) {
                return None;
            }
            ready.await;
        }

        // Give the batch a short window to fill, unless it is already full.
        let fill = async {
            while self.queue.len() < MAX_BATCH && !self.queue.closed.load(Ordering::Relaxed) {
                let ready = self.queue.ready.notified();
                tokio::pin!(ready);
                ready.as_mut().enable();
                if self.queue.len() >= MAX_BATCH {
                    break;
                }
                ready.await;
            }
        };
        let _ = tokio::time::timeout(BATCH_WINDOW, fill).await;

        let events = {
            let mut queued = self.queue.lock();
            let take = queued.len().min(MAX_BATCH);
            queued.drain(..take).collect()
        };
        Some(EventBatch {
            events,
            dropped: self.queue.dropped.swap(0, Ordering::Relaxed),
        })
    }

    /// Takes whatever is queued without waiting. Useful in tests.
    pub fn try_next_batch(&mut self) -> Option<EventBatch> {
        let events: Vec<FlowEvent> = {
            let mut queued = self.queue.lock();
            let take = queued.len().min(MAX_BATCH);
            queued.drain(..take).collect()
        };
        let dropped = self.queue.dropped.swap(0, Ordering::Relaxed);
        (!events.is_empty() || dropped > 0).then_some(EventBatch { events, dropped })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: u64) -> FlowEvent {
        FlowEvent::Failed {
            id,
            error: String::new(),
        }
    }

    fn ids(batch: &EventBatch) -> Vec<u64> {
        batch
            .events
            .iter()
            .map(|e| match e {
                FlowEvent::Failed { id, .. } => *id,
                _ => unreachable!(),
            })
            .collect()
    }

    #[test]
    fn a_full_queue_drops_new_events_and_counts_them() {
        let queue = EventQueue::new(1_000);
        for id in 0..200_000 {
            queue.push(event(id));
        }

        assert_eq!(queue.len(), 1_000);
        let mut stream = EventStream::new(queue);
        let batch = stream.try_next_batch().unwrap();
        assert_eq!(batch.dropped, 199_000);
        assert_eq!(ids(&batch), (0..MAX_BATCH as u64).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn events_arrive_in_order_in_batches() {
        let queue = EventQueue::new(10_000);
        for id in 0..1_200 {
            queue.push(event(id));
        }
        let mut stream = EventStream::new(Arc::clone(&queue));

        let mut seen = Vec::new();
        while seen.len() < 1_200 {
            let batch = stream.next_batch().await.unwrap();
            assert!(batch.events.len() <= MAX_BATCH);
            seen.extend(ids(&batch));
        }
        assert_eq!(seen, (0..1_200).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn a_batch_waits_briefly_for_more_events() {
        let queue = EventQueue::new(10_000);
        let mut stream = EventStream::new(Arc::clone(&queue));

        let producer = Arc::clone(&queue);
        tokio::spawn(async move {
            producer.push(event(1));
            tokio::time::sleep(Duration::from_millis(10)).await;
            producer.push(event(2));
        });

        let batch = stream.next_batch().await.unwrap();
        assert_eq!(ids(&batch), vec![1, 2]);
    }

    #[tokio::test]
    async fn closing_ends_the_stream_after_the_last_events() {
        let queue = EventQueue::new(10);
        queue.push(event(1));
        queue.close();
        let mut stream = EventStream::new(queue);

        assert_eq!(ids(&stream.next_batch().await.unwrap()), vec![1]);
        assert!(stream.next_batch().await.is_none());
    }
}
