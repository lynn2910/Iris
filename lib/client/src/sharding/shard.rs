use std::sync::Arc;
use futures_channel::mpsc;
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures_util::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::json;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_tungstenite::connect_async;
use tracing::error;
use structures::gateway::{BotGatewayEndpoint, GatewayResponse, HelloWsMessage, IdentityGatewayMessage, IdentityProperties, op_codes, OpCode};
use crate::http::RestClient;
use crate::sharding::heartbeat::HeartbeatSession;

pub type ShardID = u64;

/// The status of the shard
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Default)]
#[repr(u8)]
pub enum ShardStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected
}

/// Represent a shard to the Discord API
///
/// It contains all threads, tasks and information about the shard
pub struct Shard {
    /// The heartbeat sessions, contains the heartbeat task and a sender to stop the heartbeat
    heartbeat_session: Option<HeartbeatSession>,
    /// Channel used to send messages
    message_sender_channel: Option<UnboundedSender<ShardMessage>>,
    /// Channel used to receive messages sent by Discord
    message_received_channel: Option<UnboundedReceiver<GatewayResponse>>,
    /// Contains the HTTP client
    http_client: Arc<RestClient>,
    status: Arc<RwLock<ShardStatus>>,
    tasks: Option<ShardTasks>
}

struct ShardTasks {
    message_sender_thread: JoinHandle<()>,
    message_sender_stop_signal: mpsc::Sender<()>,

    message_receiver_thread: JoinHandle<()>,
    message_receiver_stop_signal: mpsc::Sender<()>,
}

pub type ShardMessage = tokio_tungstenite::tungstenite::Message;


impl Shard {
    pub fn new(token: Arc<String>) -> Self {
        let http_client = Arc::new(RestClient::new(token));

        Self {
            heartbeat_session: None,
            message_sender_channel: None,
            message_received_channel: None,
            status: Arc::new(RwLock::new(ShardStatus::Disconnected)),
            tasks: None,
            http_client
        }
    }

    fn parse_message<T: DeserializeOwned>(message: GatewayResponse) -> serde_json::Result<T> {
        serde_json::from_value(message.data)
    }

    fn parse_to_gateway_response(m: tokio_tungstenite::tungstenite::Message) -> serde_json::Result<GatewayResponse> {
        serde_json::from_reader(m.into_data().as_slice())
    }

    pub async fn connect(&mut self, gateway: &BotGatewayEndpoint, intents: i64) {
        let (send_message_tx,            mut send_message_rx           ) = mpsc::unbounded::<ShardMessage>();
        let (send_message_stopper_tx,    mut send_message_stopper_rx   ) = mpsc::channel::<()>(1);
        let (receive_message_tx,             receive_message_rx        ) = mpsc::unbounded::<GatewayResponse>();
        let (receive_message_stopper_tx, mut receive_message_stopper_rx) = mpsc::channel::<()>(1);

        self.message_received_channel = Some(receive_message_rx);
        self.message_sender_channel = Some(send_message_tx.clone());


        *self.status.write().await = ShardStatus::Connecting;

        let (ws_stream, _) = match connect_async(gateway.url.clone()).await {
            Ok(d) => d,
            Err(err) => {
                error!(target: "ShardConnector", "Cannot connect the shard: {err:?}");
                return;
            }
        };

        let (mut ws_write, mut ws_read) = ws_stream.split();

        // get the heartbeat_interval
        let hello_message = {
            let raw = match ws_read.next().await {
                Some(maybe_m) => match maybe_m {
                    Ok(m) => Self::parse_to_gateway_response(m).expect("Wrong answer from the ws"), // fixme
                    Err(e) => {
                        error!(target: "ShardConnector", "Cannot get the hello message from the websocket: {e:?}");
                        *self.status.write().await = ShardStatus::Disconnected;
                        let _ = ws_write.close().await;
                        return;
                    }
                },
                None => {
                    error!(target: "ShardConnector", "Cannot get the hello message from the websocket");
                    *self.status.write().await = ShardStatus::Disconnected;
                    let _ = ws_write.close().await;
                    return;
                }
            };

            match Self::parse_message::<HelloWsMessage>(raw) {
                Ok(hello) => hello,
                Err(e) => {
                    error!(target: "ShardConnector", "Invalid Hello message from the websocket: {e:?}");
                    *self.status.write().await = ShardStatus::Disconnected;
                    let _ = ws_write.close().await;
                    return;
                }
            }
        };

        // Create the heartbeat, which will automatically start to run :)
        self.heartbeat_session = Some(
            HeartbeatSession::new(
                hello_message.heartbeat_interval,
                send_message_tx.clone()
            )
        );
        
        // After creating the heartbeat loop, we must send the identity-payload
        {
            let identity_payload = Self::encapsulate_payload(
                op_codes::IDENTITY_PAYLOAD,
                self.create_identity_payload_message(intents)
            );

            if let Err(e) = ws_write.send(identity_payload).await {
                error!(target: "ShardConnector", "Cannot send the identity payload, therefor the shard cannot be started: {e:?}");
                self.close_shard().await;
                let _ = ws_write.close().await;
                *self.status.write().await = ShardStatus::Disconnected;
                return;
            }
        }

        // The identity payload has been sent, we must create the tasks which will receive and send messages :o

        let status_cl = self.status.clone();
        let send_message_to_shard_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = send_message_stopper_rx.next() => {
                        send_message_stopper_rx.close();
                        break;
                    }
                    Some(msg_to_send) = send_message_rx.next() => {
                        println!("Message received, ready to send to the shard: {msg_to_send:?}");
                        if let Err(e) = ws_write.send(msg_to_send).await {
                            error!(target: "ShardSender", "Cannot send a message to the shard: {e:?}");
                            send_message_stopper_rx.close();
                            *status_cl.write().await = ShardStatus::Disconnected;
                            break;
                        }
                        println!("Message sent to the shard");
                    }
                }
            }
        });

        let status_cl = self.status.clone();
        let receive_message_from_shard_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = receive_message_stopper_rx.next() => {
                        receive_message_stopper_rx.close();
                        break;
                    }
                    Some(msg_received) = ws_read.next() => {
                        println!("Message received: {msg_received:#?}");

                        if msg_received.is_err() {}
                        let msg_received = msg_received.unwrap();

                        if !msg_received.is_text() {
                            println!("Not a text.");
                            continue;
                        }

                        let msg = match serde_json::from_str(msg_received.into_text().unwrap().as_str()) {
                            Ok(m) => m,
                            Err(e) => {
                                error!(target: "ShardReceiver", "Invalid message received by the shard: {e:?}");
                                continue;
                            }
                        };

                        if let Err(e) = receive_message_tx.unbounded_send(msg) {
                            error!(target: "ShardReceiver", "Cannot broadcast the message received by the shard: {e:?}");
                            receive_message_stopper_rx.close();
                            *status_cl.write().await = ShardStatus::Disconnected;
                            break;
                        }
                    }
                }
            }
        });

        self.tasks = Some(ShardTasks {
            message_sender_thread: send_message_to_shard_task,
            message_sender_stop_signal: send_message_stopper_tx,

            message_receiver_thread: receive_message_from_shard_task,
            message_receiver_stop_signal: receive_message_stopper_tx,
        });
        *self.status.write().await = ShardStatus::Connected;
    }

    pub async fn close_shard(&mut self){
        match self.heartbeat_session.as_mut() {
            Some(session) if !session.is_closed() => {
                if let Err(e) = session.stop() {
                    error!(target: "ShardDisconnector", "Cannot close the heartbeat session of the shard: {e:?}")
                }
            }
            _ => {}
        }

        if let Some(tasks) = self.tasks.as_mut() {
            if !tasks.message_sender_thread.is_finished() {
                if let Err(e) = tasks.message_sender_stop_signal.try_send(()) {
                    error!(target: "ShardDisconnector", "Cannot send the signal to stop the shard message sender task: {e:?}");
                    tasks.message_sender_thread.abort()
                } else {
                    tasks.message_sender_stop_signal.close_channel();
                }
            }

            if !tasks.message_receiver_thread.is_finished() {
                if let Err(e) = tasks.message_receiver_stop_signal.try_send(()) {
                    error!(target: "ShardDisconnector", "Cannot send the signal to stop the shard message receiver task: {e:?}");
                    tasks.message_receiver_thread.abort()
                } else {
                    tasks.message_receiver_stop_signal.close_channel();
                }
            }
        }

        if let Some(message_sender_channel) = self.message_sender_channel.as_mut() {
            if !message_sender_channel.is_closed() { message_sender_channel.close_channel() }
        }
    }

    /// This method will "encapsulate" the data inside a JSON that contains the given op code, used as a helper method for sending messages to the Discord Websocket
    pub(crate) fn encapsulate_payload<T: Serialize>(op_code: OpCode, d: T) -> ShardMessage {
        ShardMessage::Text(json!({
            "op": op_code,
            "d": d
        }).to_string())
    }

    fn create_identity_payload_message(&self, intents: i64) -> IdentityGatewayMessage {
        IdentityGatewayMessage {
            intents,
            token: self.http_client.get_token().to_string(),
            properties: IdentityProperties {
                os: std::env::consts::OS.to_string(),
                browser: crate::LIB_NAME.to_string(),
                device: crate::LIB_NAME.to_string(),
            },
        }
    }

    pub async fn get_status(&self) -> ShardStatus {
        *self.status.read().await
    }

    pub async fn is_connected(&self) -> bool {
        self.get_status().await == ShardStatus::Connected
    }
}
