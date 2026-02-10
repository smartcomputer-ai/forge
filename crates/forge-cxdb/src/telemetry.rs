// Copyright 2025 StrongDM Inc
// SPDX-License-Identifier: Apache-2.0

//! Fire-and-forget telemetry sender with complete isolation from the main application.
//!
//! This module provides a non-blocking telemetry interface that ensures the main application
//! is never impacted by CXDB availability. All telemetry operations are processed on a
//! dedicated thread with a bounded queue that drops oldest entries when full.
//!
//! # Design
//!
//! - **Dedicated thread**: Complete isolation from the main application thread pool
//! - **Bounded queue**: Maximum 512 pending requests to prevent unbounded memory growth
//! - **Drop oldest**: When the queue is full, the oldest entry is dropped to accept new telemetry
//! - **Non-blocking send**: `send()` does not perform I/O (but may briefly lock a mutex)
//! - **Count-bounded queue**: Capacity is based on item count, not total bytes
//! - **Graceful degradation**: If CXDB is unavailable, telemetry is silently dropped
//!
//! # Example
//!
//! ```no_run
//! use cxdb::telemetry::{TelemetrySender, TelemetryConfig};
//! use cxdb::{AppendRequest, EncodingMsgpack};
//!
//! // Start the telemetry sender
//! let sender = TelemetrySender::start(TelemetryConfig {
//!     addr: "localhost:9009".into(),
//!     use_tls: false,
//!     ..Default::default()
//! });
//!
//! // Send telemetry (no I/O, drops oldest if queue full)
//! let req = AppendRequest::new(1, "telemetry.Event", 1, vec![0x91, 0x01]);
//! sender.send(req);
//! ```

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use crate::client::{dial, dial_tls, Client, ClientOption, RequestContext};
use crate::reconnect::is_connection_error;
use crate::turn::AppendRequest;

/// Default maximum queue size for telemetry requests.
pub const DEFAULT_QUEUE_CAPACITY: usize = 512;

/// Default reconnect delay when CXDB is unavailable.
pub const DEFAULT_RECONNECT_DELAY: Duration = Duration::from_secs(5);

/// Configuration for the telemetry sender.
#[derive(Clone)]
pub struct TelemetryConfig {
    /// Server address (e.g., "localhost:9009").
    pub addr: String,
    /// Whether to use TLS.
    pub use_tls: bool,
    /// Client options for the underlying connection.
    pub client_opts: Vec<ClientOption>,
    /// Maximum queue capacity (default: 512).
    pub queue_capacity: usize,
    /// Delay between reconnection attempts (default: 5 seconds).
    pub reconnect_delay: Duration,
}

impl std::fmt::Debug for TelemetryConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelemetryConfig")
            .field("addr", &self.addr)
            .field("use_tls", &self.use_tls)
            .field(
                "client_opts",
                &format!("[{} options]", self.client_opts.len()),
            )
            .field("queue_capacity", &self.queue_capacity)
            .field("reconnect_delay", &self.reconnect_delay)
            .finish()
    }
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            addr: String::new(),
            use_tls: false,
            client_opts: Vec::new(),
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            reconnect_delay: DEFAULT_RECONNECT_DELAY,
        }
    }
}

/// Statistics about the telemetry sender.
#[derive(Clone, Debug, Default)]
pub struct TelemetryStats {
    /// Number of requests successfully sent.
    pub sent: u64,
    /// Number of requests dropped due to queue overflow.
    pub dropped_overflow: u64,
    /// Number of requests dropped due to send failure.
    pub dropped_error: u64,
    /// Current queue length.
    pub queue_len: usize,
}

/// Internal state shared between sender and worker thread.
struct SharedState {
    queue: VecDeque<AppendRequest>,
    capacity: usize,
    stats: TelemetryStats,
    shutdown: bool,
}

/// Handle for sending telemetry requests.
///
/// This is the public interface for submitting telemetry. The `send()` method
/// never blocks and will drop the oldest queued request if the queue is full.
///
/// Cloning this handle is cheap (Arc-based) and all clones share the same
/// underlying queue and worker thread.
#[derive(Clone)]
pub struct TelemetrySender {
    state: Arc<(Mutex<SharedState>, Condvar)>,
}

impl TelemetrySender {
    /// Start the telemetry sender with the given configuration.
    ///
    /// This spawns a dedicated worker thread that owns the CXDB client connection
    /// and processes telemetry requests from the queue.
    pub fn start(config: TelemetryConfig) -> Self {
        let state = Arc::new((
            Mutex::new(SharedState {
                queue: VecDeque::with_capacity(config.queue_capacity),
                capacity: config.queue_capacity,
                stats: TelemetryStats::default(),
                shutdown: false,
            }),
            Condvar::new(),
        ));

        let worker_state = state.clone();
        if let Err(_err) = thread::Builder::new()
            .name("cxdb-telemetry".into())
            .spawn(move || {
                worker_loop(worker_state, config);
            })
        {
            let (lock, _) = &*state;
            let mut guard = lock.lock().unwrap();
            guard.shutdown = true;
            guard.stats.dropped_error += 1;
            guard.stats.queue_len = 0;
        }

        Self { state }
    }

    /// Send a telemetry request.
    ///
    /// This method does not perform I/O. It may briefly block on the queue mutex.
    /// If the queue is full, the oldest request
    /// is dropped to make room for the new one.
    ///
    /// Returns `true` if the request was queued, `false` if it caused an
    /// overflow (oldest request was dropped).
    pub fn send(&self, req: AppendRequest) -> bool {
        let (lock, cvar) = &*self.state;
        let mut state = lock.lock().unwrap();

        if state.shutdown {
            return false;
        }

        let overflowed = if state.queue.len() >= state.capacity {
            // Drop oldest to make room for new
            state.queue.pop_front();
            state.stats.dropped_overflow += 1;
            true
        } else {
            false
        };

        state.queue.push_back(req);
        state.stats.queue_len = state.queue.len();

        // Notify worker thread
        cvar.notify_one();

        !overflowed
    }

    /// Get current telemetry statistics.
    pub fn stats(&self) -> TelemetryStats {
        let (lock, _) = &*self.state;
        let state = lock.lock().unwrap();
        state.stats.clone()
    }

    /// Shutdown the telemetry sender.
    ///
    /// This signals the worker thread to stop. Pending requests in the queue
    /// may or may not be processed depending on timing.
    pub fn shutdown(&self) {
        let (lock, cvar) = &*self.state;
        let mut state = lock.lock().unwrap();
        state.shutdown = true;
        cvar.notify_one();
    }
}

/// Worker loop that runs on the dedicated telemetry thread.
fn worker_loop(state: Arc<(Mutex<SharedState>, Condvar)>, config: TelemetryConfig) {
    let mut client: Option<Client> = None;

    loop {
        // Wait for work or shutdown
        let req = {
            let (lock, cvar) = &*state;
            let mut guard = lock.lock().unwrap();

            // Wait until there's work or shutdown
            while guard.queue.is_empty() && !guard.shutdown {
                guard = cvar.wait(guard).unwrap();
            }

            if guard.shutdown && guard.queue.is_empty() {
                return;
            }

            guard.queue.pop_front()
        };

        let Some(req) = req else {
            continue;
        };

        // Ensure we have a client connection
        if client.is_none() {
            client = try_connect(&config);
        }

        // Try to send
        if let Some(ref c) = client {
            let ctx = RequestContext::background();
            match c.append_turn(&ctx, &req) {
                Ok(_) => {
                    let (lock, _) = &*state;
                    let mut guard = lock.lock().unwrap();
                    guard.stats.sent += 1;
                    guard.stats.queue_len = guard.queue.len();
                }
                Err(err) => {
                    let should_reconnect = is_connection_error(&err);
                    if should_reconnect {
                        client = None;
                    }
                    let (lock, _) = &*state;
                    let mut guard = lock.lock().unwrap();
                    guard.stats.dropped_error += 1;
                    guard.stats.queue_len = guard.queue.len();

                    drop(guard);
                    if should_reconnect {
                        // Brief delay before retry to avoid tight loop
                        thread::sleep(config.reconnect_delay);
                    }
                }
            }
        } else {
            // No connection, drop the request
            let (lock, _) = &*state;
            let mut guard = lock.lock().unwrap();
            guard.stats.dropped_error += 1;
            guard.stats.queue_len = guard.queue.len();

            // Brief delay before retry
            drop(guard);
            thread::sleep(config.reconnect_delay);
        }
    }
}

/// Attempt to connect to CXDB.
fn try_connect(config: &TelemetryConfig) -> Option<Client> {
    let dial_fn = if config.use_tls { dial_tls } else { dial };
    dial_fn(&config.addr, config.client_opts.clone()).ok()
}

/// Builder for creating a TelemetrySender with custom options.
pub struct TelemetrySenderBuilder {
    config: TelemetryConfig,
}

impl TelemetrySenderBuilder {
    /// Create a new builder with the given server address.
    pub fn new(addr: impl Into<String>) -> Self {
        Self {
            config: TelemetryConfig {
                addr: addr.into(),
                ..Default::default()
            },
        }
    }

    /// Enable TLS for the connection.
    pub fn with_tls(mut self) -> Self {
        self.config.use_tls = true;
        self
    }

    /// Set the queue capacity (default: 512).
    pub fn with_queue_capacity(mut self, capacity: usize) -> Self {
        self.config.queue_capacity = capacity;
        self
    }

    /// Set the reconnect delay (default: 5 seconds).
    pub fn with_reconnect_delay(mut self, delay: Duration) -> Self {
        self.config.reconnect_delay = delay;
        self
    }

    /// Add a client option.
    pub fn with_client_option(mut self, opt: ClientOption) -> Self {
        self.config.client_opts.push(opt);
        self
    }

    /// Build and start the telemetry sender.
    pub fn build(self) -> TelemetrySender {
        TelemetrySender::start(self.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ENCODING_MSGPACK;

    #[test]
    fn queue_drops_oldest_when_full() {
        let state = Arc::new((
            Mutex::new(SharedState {
                queue: VecDeque::with_capacity(3),
                capacity: 3,
                stats: TelemetryStats::default(),
                shutdown: false,
            }),
            Condvar::new(),
        ));

        let sender = TelemetrySender { state };

        // Fill the queue
        for i in 0..3 {
            let req = AppendRequest {
                context_id: i,
                parent_turn_id: 0,
                type_id: "test".into(),
                type_version: 1,
                payload: vec![],
                idempotency_key: vec![],
                encoding: ENCODING_MSGPACK,
                compression: 0,
            };
            assert!(sender.send(req), "should not overflow for item {i}");
        }

        // Queue is now full, next send should overflow
        let req = AppendRequest {
            context_id: 100,
            parent_turn_id: 0,
            type_id: "test".into(),
            type_version: 1,
            payload: vec![],
            idempotency_key: vec![],
            encoding: ENCODING_MSGPACK,
            compression: 0,
        };
        assert!(!sender.send(req), "should overflow");

        let stats = sender.stats();
        assert_eq!(stats.dropped_overflow, 1);
        assert_eq!(stats.queue_len, 3);

        // Verify oldest was dropped (context_id 0) and newest is present (context_id 100)
        let (lock, _) = &*sender.state;
        let guard = lock.lock().unwrap();
        let ids: Vec<u64> = guard.queue.iter().map(|r| r.context_id).collect();
        assert_eq!(ids, vec![1, 2, 100]);
    }

    #[test]
    fn send_after_shutdown_returns_false() {
        let state = Arc::new((
            Mutex::new(SharedState {
                queue: VecDeque::with_capacity(10),
                capacity: 10,
                stats: TelemetryStats::default(),
                shutdown: false,
            }),
            Condvar::new(),
        ));

        let sender = TelemetrySender { state };
        sender.shutdown();

        let req = AppendRequest {
            context_id: 1,
            parent_turn_id: 0,
            type_id: "test".into(),
            type_version: 1,
            payload: vec![],
            idempotency_key: vec![],
            encoding: ENCODING_MSGPACK,
            compression: 0,
        };
        assert!(!sender.send(req));
    }

    #[test]
    fn builder_creates_sender() {
        // Just test that builder compiles and creates config correctly
        let builder = TelemetrySenderBuilder::new("localhost:9009")
            .with_tls()
            .with_queue_capacity(256)
            .with_reconnect_delay(Duration::from_secs(10));

        assert_eq!(builder.config.addr, "localhost:9009");
        assert!(builder.config.use_tls);
        assert_eq!(builder.config.queue_capacity, 256);
        assert_eq!(builder.config.reconnect_delay, Duration::from_secs(10));
    }
}
