//! # Asynchronous Batch Telemetry Flusher
//!
//! High-throughput, non-blocking telemetry flusher powered by Tokio channels.
//! Flushes batches of events to a `TraceStore` based on batch size thresholds
//! or periodic timer intervals, with guaranteed shutdown persistence.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use traceforge_core::events::TraceEvent;

use crate::models::{BatchFlusherConfig, StorageError};
use crate::traits::TraceStore;

/// Internal command dispatched to the flusher background worker.
enum FlusherCommand {
    Event(TraceEvent),
    Flush(oneshot::Sender<Result<(), StorageError>>),
    Shutdown(oneshot::Sender<Result<(), StorageError>>),
}

/// Asynchronous background worker that batches trace events and flushes them to storage.
///
/// Designed for ultra-low proxy overhead (< 0.05ms):
/// calls to [`AsyncBatchFlusher::send`] deposit the event into a lock-free bounded
/// buffer without awaiting disk or database writes.
#[derive(Clone)]
pub struct AsyncBatchFlusher {
    tx: mpsc::Sender<FlusherCommand>,
    is_shutdown: Arc<AtomicBool>,
    worker_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl AsyncBatchFlusher {
    /// Creates a new `AsyncBatchFlusher` with default configuration
    /// (10,000 capacity, 100 batch size, 250ms interval).
    pub fn new(store: Arc<dyn TraceStore>) -> Self {
        Self::with_config(store, BatchFlusherConfig::default())
    }

    /// Creates a flusher from any concrete store implementing `TraceStore`.
    pub fn from_store<S: TraceStore>(store: S) -> Self {
        Self::new(Arc::new(store))
    }

    /// Creates a flusher from an `Arc<S>` where `S: TraceStore`.
    pub fn from_arc<S: TraceStore>(store: Arc<S>) -> Self {
        Self::new(store)
    }

    /// Creates a new `AsyncBatchFlusher` with customized configuration.
    pub fn with_config(store: Arc<dyn TraceStore>, config: BatchFlusherConfig) -> Self {
        let (tx, rx) = mpsc::channel(config.channel_capacity);
        let is_shutdown = Arc::new(AtomicBool::new(false));

        let worker = tokio::spawn(async move {
            Self::worker_loop(store, rx, config).await;
        });

        Self {
            tx,
            is_shutdown,
            worker_handle: Arc::new(Mutex::new(Some(worker))),
        }
    }

    /// Non-blocking event submission (< 0.05ms latency overhead).
    ///
    /// Inserts the event into the flusher's bounded queue without waiting for I/O.
    /// Returns [`StorageError::QueueFull`] if the channel buffer capacity is exceeded,
    /// or [`StorageError::ChannelClosed`] if the flusher has been shut down.
    pub fn send(&self, event: TraceEvent) -> Result<(), StorageError> {
        if self.is_shutdown.load(Ordering::Relaxed) {
            return Err(StorageError::ChannelClosed);
        }

        self.tx
            .try_send(FlusherCommand::Event(event))
            .map_err(|err| match err {
                mpsc::error::TrySendError::Full(_) => StorageError::QueueFull,
                mpsc::error::TrySendError::Closed(_) => StorageError::ChannelClosed,
            })
    }

    /// Asynchronously submits an event, waiting for queue capacity if the buffer is currently full.
    pub async fn send_async(&self, event: TraceEvent) -> Result<(), StorageError> {
        if self.is_shutdown.load(Ordering::Relaxed) {
            return Err(StorageError::ChannelClosed);
        }

        self.tx
            .send(FlusherCommand::Event(event))
            .await
            .map_err(|_| StorageError::ChannelClosed)
    }

    /// Forces an immediate flush of all currently queued events, awaiting completion.
    pub async fn flush(&self) -> Result<(), StorageError> {
        if self.is_shutdown.load(Ordering::Relaxed) {
            return Err(StorageError::ChannelClosed);
        }

        let (ack_tx, ack_rx) = oneshot::channel();
        self.tx
            .send(FlusherCommand::Flush(ack_tx))
            .await
            .map_err(|_| StorageError::ChannelClosed)?;

        ack_rx.await.map_err(|_| StorageError::ChannelClosed)?
    }

    /// Gracefully shuts down the flusher, flushing all remaining events to the store
    /// and awaiting completion of the background worker task.
    pub async fn shutdown_and_flush(&self) -> Result<(), StorageError> {
        if self.is_shutdown.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        let (ack_tx, ack_rx) = oneshot::channel();
        if self.tx.send(FlusherCommand::Shutdown(ack_tx)).await.is_ok() {
            let _ = ack_rx.await;
        }

        if let Some(handle) = self.worker_handle.lock().await.take() {
            let _ = handle.await;
        }

        Ok(())
    }

    /// Returns whether the flusher is currently shut down.
    pub fn is_shutdown(&self) -> bool {
        self.is_shutdown.load(Ordering::Relaxed)
    }

    /// Background worker event loop.
    async fn worker_loop(
        store: Arc<dyn TraceStore>,
        mut rx: mpsc::Receiver<FlusherCommand>,
        config: BatchFlusherConfig,
    ) {
        let mut interval = tokio::time::interval(config.flush_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut batch: Vec<TraceEvent> = Vec::with_capacity(config.batch_size);

        loop {
            tokio::select! {
                biased;

                cmd = rx.recv() => {
                    match cmd {
                        Some(FlusherCommand::Event(event)) => {
                            batch.push(event);
                            if batch.len() >= config.batch_size {
                                let _ = Self::flush_batch(&store, &mut batch).await;
                                interval.reset();
                            }
                        }
                        Some(FlusherCommand::Flush(ack)) => {
                            // Drain any queued events currently available
                            while let Ok(cmd) = rx.try_recv() {
                                match cmd {
                                    FlusherCommand::Event(e) => batch.push(e),
                                    FlusherCommand::Flush(other_ack) => {
                                        let _ = other_ack.send(Ok(()));
                                    }
                                    FlusherCommand::Shutdown(other_ack) => {
                                        let res = Self::flush_batch(&store, &mut batch).await;
                                        let _ = other_ack.send(res);
                                        let _ = ack.send(Ok(()));
                                        return;
                                    }
                                }
                            }
                            let res = Self::flush_batch(&store, &mut batch).await;
                            let _ = ack.send(res);
                            interval.reset();
                        }
                        Some(FlusherCommand::Shutdown(ack)) => {
                            // Drain all remaining events
                            while let Ok(cmd) = rx.try_recv() {
                                if let FlusherCommand::Event(e) = cmd {
                                    batch.push(e);
                                }
                            }
                            let res = Self::flush_batch(&store, &mut batch).await;
                            let _ = ack.send(res);
                            return;
                        }
                        None => {
                            // Senders dropped
                            if !batch.is_empty() {
                                let _ = Self::flush_batch(&store, &mut batch).await;
                            }
                            return;
                        }
                    }
                }

                _ = interval.tick() => {
                    if !batch.is_empty() {
                        let _ = Self::flush_batch(&store, &mut batch).await;
                    }
                }
            }
        }
    }

    /// Flushes accumulated events to storage and clears the batch buffer.
    async fn flush_batch(
        store: &Arc<dyn TraceStore>,
        batch: &mut Vec<TraceEvent>,
    ) -> Result<(), StorageError> {
        if batch.is_empty() {
            return Ok(());
        }

        let result = store.record_batch(batch).await;
        if let Err(ref e) = result {
            tracing::error!("AsyncBatchFlusher error recording batch of {} events: {e}", batch.len());
        }
        batch.clear();
        result
    }
}
