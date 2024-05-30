use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;
use futures_channel::mpsc;
use futures_channel::mpsc::SendError;
use serde_json::json;
use smol::stream::StreamExt;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep};
use tracing::{debug, error, trace};
use structures::gateway::op_codes;
use crate::sharding::shard::{Shard, ShardMessage};

/// Store the Handle of the heartbeat task and the received that can be used to stop it.
pub(super) struct HeartbeatSession {
    stop_sender_signal: mpsc::Sender<()>,
    task: JoinHandle<()>,
    pub(crate) last_heartbeat: Arc<RwLock<Option<Instant>>>
}

impl HeartbeatSession {
    pub(crate) fn is_closed(&self) -> bool {
        self.stop_sender_signal.is_closed() && self.task.is_finished()
    }

    /// Initiate the heartbeat loop interval
    pub(super) fn new(heartbeat_interval: u64, mut sender: mpsc::UnboundedSender<ShardMessage>) -> Self {
        let (tx, mut rx) = mpsc::channel::<()>(1);

        let last_heartbeat = Arc::new(RwLock::new(None));

        let last_heartbeat_cl = last_heartbeat.clone();
        let task = tokio::task::spawn(async move {
            loop {
                tokio::select! {
                    _ = rx.next() => {
                        // Stop signal received, the channel will be closed, and the loop will stop
                        rx.close();
                        break;
                    }
                    _ = sleep(Duration::from_millis(heartbeat_interval)) => {
                        if sender.is_closed() {
                            trace!(target: "HeartbeatShard", "The shard sender is closed, cannot send the ping. Ping thread will be killed.");
                            rx.close();
                            trace!(target: "HeartbeatShard", "The shard sender has been killed.");
                            break;
                        }

                        // Send a new heartbeat
                        trace!(target: "HeartbeatShard", "Sending heartbeat...");
                        let message = Self::create_ping_message(last_heartbeat_cl.read().await.deref());

                        if let Err(e) = sender.unbounded_send(message) {
                            error!(target: "HeartbeatShard", "Error while sending heartbeat: {:?}", e);
                            continue
                        }
                        *last_heartbeat_cl.write().await = Some(Instant::now());
                    }
                };
            };
        });

        HeartbeatSession {
            stop_sender_signal: tx,
            last_heartbeat, task,
        }
    }

    fn create_ping_message(last_heartbeat: &Option<Instant>) -> ShardMessage {
        Shard::encapsulate_payload(
            op_codes::HELLO,
            last_heartbeat.and_then(|lh| Some(lh.elapsed().as_millis()))
        )
    }

    /// Stop the heartbeat loop.
    /// The shard should use this method only internally when she is closed, and not the other way around.
    pub(super) fn stop(&mut self) -> Result<(), SendError>{
        self.stop_sender_signal.start_send(())?;
        self.task.abort();
        
        Ok(())
    }
}