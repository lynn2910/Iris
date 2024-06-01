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
use tracing::{error, trace};
use structures::gateway::{BotGatewayEndpoint, GatewayResponse, HelloWsMessage, IdentityGatewayMessage, IdentityProperties, op_codes, OpCode};
use crate::client::ShardPool;
use crate::events::EventHandler;
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
    pub shard_id: ShardID,
    /// The heartbeat sessions, contains the heartbeat task and a sender to stop the heartbeat
    heartbeat_session: Option<HeartbeatSession>,
    /// Channel used to send messages
    message_sender_channel: Option<UnboundedSender<ShardMessage>>,
    /// Channel used to receive messages sent by Discord
    message_received_channel: Option<UnboundedReceiver<GatewayResponse>>,
    /// Contains the HTTP client
    http_client: Arc<RestClient>,
    status: Arc<RwLock<ShardStatus>>,
    tasks: Option<ShardTasks>,
    event_handler: Arc<dyn EventHandler>
}

struct ShardTasks {
    message_sender_thread: JoinHandle<()>,
    message_sender_stop_signal: mpsc::Sender<()>,

    message_receiver_thread: JoinHandle<()>,
    message_receiver_stop_signal: mpsc::Sender<()>,
}

pub type ShardMessage = tokio_tungstenite::tungstenite::Message;


impl Shard {
    pub fn new(shard_id: ShardID, token: Arc<String>, event_handler: Arc<dyn EventHandler>) -> Self {
        let http_client = Arc::new(RestClient::new(token));

        Self {
            heartbeat_session: None,
            message_sender_channel: None,
            message_received_channel: None,
            status: Arc::new(RwLock::new(ShardStatus::Disconnected)),
            tasks: None,
            event_handler,
            http_client,
            shard_id
        }
    }

    fn parse_message<T: DeserializeOwned>(message: GatewayResponse) -> serde_json::Result<T> {
        serde_json::from_value(message.data)
    }

    fn parse_to_gateway_response(m: tokio_tungstenite::tungstenite::Message) -> serde_json::Result<GatewayResponse> {
        serde_json::from_reader(m.into_data().as_slice())
    }

    fn parse_text_to_gateway_response(m: &str) -> serde_json::Result<GatewayResponse> {
        serde_json::from_str(m)
    }

    pub async fn connect(
        &mut self,
        gateway: &BotGatewayEndpoint,
        intents: i64,
        shard_id: ShardID,
        shard_pool: ShardPool
    )
    {
        let (send_message_tx,            mut send_message_rx           ) = mpsc::unbounded::<ShardMessage>();
        let (send_message_stopper_tx,    mut send_message_stopper_rx   ) = mpsc::channel::<()>(1);
        let (_receive_message_tx,            receive_message_rx        ) = mpsc::unbounded::<GatewayResponse>();
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
        let events_cl = self.event_handler.clone();
        let receive_message_from_shard_task = tokio::spawn(async move {
            'receiver_loop: loop {
                tokio::select! {
                    _ = receive_message_stopper_rx.next() => {
                        receive_message_stopper_rx.close();
                        break;
                    }
                    Some(msg_received) = ws_read.next() => {
                        if msg_received.is_err() {
                            match msg_received.err().unwrap() {
                                tokio_tungstenite::tungstenite::Error::ConnectionClosed => {
                                    error!(target: "iris::sharding::shard::ws_receiver", "The connection had been closed normally.");
                                    *status_cl.write().await = ShardStatus::Disconnected;
                                    break 'receiver_loop;
                                },
                                tokio_tungstenite::tungstenite::Error::AlreadyClosed => {
                                    error!(target: "iris::sharding::shard::ws_receiver", "Cannot work with a closed connection.");
                                    *status_cl.write().await = ShardStatus::Disconnected;
                                    break 'receiver_loop;
                                },
                                tokio_tungstenite::tungstenite::Error::AttackAttempt => {
                                    panic!("Attack attempt on a shard's websocket detected.");
                                }
                                tokio_tungstenite::tungstenite::Error::Utf8 => {
                                    error!(target: "iris::sharding::shard::ws_receiver", "Wrong text encoding received from the websocket, wtf?");
                                }
                                tokio_tungstenite::tungstenite::Error::Io(err) => {
                                    error!(target: "iris::sharding::shard::ws_receiver", "IO error from the shard's websocket: {err:#?}");
                                    *status_cl.write().await = ShardStatus::Disconnected;
                                    break 'receiver_loop;
                                }
                                tokio_tungstenite::tungstenite::Error::Tls(err) => {
                                    error!(target: "iris::sharding::shard::ws_receiver", "TLS error from the shard's websocket: {err:#?}");
                                    *status_cl.write().await = ShardStatus::Disconnected;
                                    break 'receiver_loop;
                                }
                                tokio_tungstenite::tungstenite::Error::Capacity(err) => {
                                    error!(target: "iris::sharding::shard::ws_receiver", "Websocket buffer capacity exhausted: {err:#?}");
                                    *status_cl.write().await = ShardStatus::Disconnected;
                                    break 'receiver_loop;
                                }
                                tokio_tungstenite::tungstenite::Error::WriteBufferFull(err) => {
                                    error!(target: "iris::sharding::shard::ws_receiver", "Shard's websocket writing buffer full: {err:#?}");
                                    *status_cl.write().await = ShardStatus::Disconnected;
                                    break 'receiver_loop;
                                }
                                tokio_tungstenite::tungstenite::Error::Protocol(err) => {
                                    error!(target: "iris::sharding::shard::ws_receiver", "Protocol error from Discord's websocket: {err:#?}");
                                    *status_cl.write().await = ShardStatus::Disconnected;
                                    break 'receiver_loop;
                                }
                                _ => unimplemented!("An error was fired in the websocket shard receiver which should not be fired...")
                            }
                            return;
                        }
                        let msg_received = msg_received.unwrap();

                        match msg_received {
                            ShardMessage::Text(t) => Self::text_message_received(t, shard_id, &shard_pool, &events_cl).await,
                            ShardMessage::Ping(ping) => Self::ping_received(ping).await,
                            ShardMessage::Pong(pong) => Self::pong_received(pong).await,
                            ShardMessage::Close(closed) => {
                                trace!(target: "iris::sharding::shard::ws_receiver", "Close instruction received from the websocket");
                                if let Some(reason) = closed {
                                    trace!(target: "iris::sharding::shard::ws_receiver", "Code:   {}", reason.code);
                                    trace!(target: "iris::sharding::shard::ws_receiver", "Reason: {}", reason.reason);
                                }

                                *status_cl.write().await = ShardStatus::Disconnected;
                            }
                            _ => unimplemented!()
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

    async fn text_message_received(msg: String, shard_id: ShardID, shard_pool: &ShardPool, events: &Arc<dyn EventHandler>){
        let gateway_response = match Self::parse_text_to_gateway_response(msg.as_str()) {
            Ok(gr) => gr,
            Err(e) => {
                error!(target: "iris::sharding::shard", "Cannot parse the websocket's message to a GatewayResponse: {e:#?}");
                return;
            }
        };

        if gateway_response.event_name.is_some() {
            match gateway_response.event_name.as_ref().unwrap().as_str() {
                manage_events::READY_EVENT_NAME => manage_events::ready_event(shard_pool, shard_id, gateway_response, events).await,
                event_name => {
                    trace!(target: "iris::sharding::shard", "Unknown event name: {event_name:?}")
                }
            }
        }
    }
    async fn ping_received(_ping: Vec<u8>){
        println!("Ping received");
    }
    async fn pong_received(_ping: Vec<u8>){
        println!("Pong received");
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

mod manage_events {
    use std::sync::Arc;
    use tracing::error;
    use structures::gateway::GatewayResponse;
    use crate::client::ShardPool;
    use crate::events::{Context, EventHandler};
    use crate::sharding::shard::{Shard, ShardID};

    pub(super) const READY_EVENT_NAME: &str = "READY";
    pub(super) async fn ready_event(
        shard_pool: &ShardPool,
        shard_id: ShardID,
        data: GatewayResponse,
        events: &Arc<dyn EventHandler>
    )
    {
        let ctx = Context {
            shard_pool: shard_pool.clone(),
            shard_id
        };
        let events_cl = events.clone();
        
        tokio::task::spawn(async move {
            let parsed_data = Shard::parse_message(data);
            
            if let Err(e) = parsed_data {
                error!(target: "iris::sharding::shard::manage_events", "Invalid object given to the READY event: {e:#?}");
                return;
            }
            
            events_cl.ready(ctx, parsed_data.unwrap());
        });
    }
}