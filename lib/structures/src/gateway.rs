use serde::{Deserialize, Serialize};

pub type OpCode = u16;

pub mod op_codes {
    use crate::gateway::OpCode;

    pub const HELLO: OpCode = 1;
    pub const IDENTITY_PAYLOAD: OpCode = 2;
}

/// The payload structure
/// 
/// It is used every time the api communicates with the client using the websocket.
/// 
/// See the [documentation](https://discord.com/developers/docs/topics/gateway-events#payload-structure)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GatewayResponse {
    /// The Gateway code
    ///
    /// See the [documentation](https://discord.com/developers/docs/topics/opcodes-and-status-codes#gateway-gateway-opcodes)
    pub op: OpCode,
    /// The event data. Can be any JSON value
    #[serde(rename="d")]
    pub data: serde_json::Value,
    /// The sequence number, if any
    ///
    /// Used for either:
    /// - [Resuming sessions](https://discord.com/developers/docs/topics/gateway#resuming)
    /// - [heartbeat](https://discord.com/developers/docs/topics/gateway#sending-heartbeats)
    #[serde(rename="s")]
    #[serde(default)]
    pub sequence_number: Option<i32>,
    /// The event name, if any
    #[serde(rename="t")]
    #[serde(default)]
    pub name: Option<String>
}

/// Contains the information in Get Gateway,
/// plus additional metadata that can help during the operation of large or sharded bots.
/// 
/// Unlike the Get Gateway, this route should not be cached for extended periods of time
/// as the value is not guaranteed to be the same per-call, and changes as the bot
/// joins/leaves guilds.*
/// 
/// See the [documentation](https://discord.com/developers/docs/topics/gateway#get-gateway-bot)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BotGatewayEndpoint {
    /// The URL that can be used to connect to the gateway
    pub url: String,
    /// The recommended number of shards to use when connecting
    pub shards: u64,
    /// The
    pub session_start_limit: SessionStartLimit
}

/// Contains information about the session and the limits
///
/// See the [documentation](https://discord.com/developers/docs/topics/gateway#session-start-limit-object)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionStartLimit {
    /// The total number of session starts the current user is allowed
    pub total: u64,
    /// The remaining number of session starts the current user is allowed
    pub remaining: u64,
    /// The number of milliseconds after which the limit resets
    pub reset_after: u128,
    /// The number of identify-requests allowed per 5 seconds
    pub max_concurrency: u64
}

/// Contains information given by the websocket at the connection
///
/// See the [documentation](https://discord.com/developers/docs/topics/gateway#hello-event)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HelloWsMessage {
    pub heartbeat_interval: u64
}


/// Contains information to identify the app, which will be given at each shard start
///
/// - See the [documentation](https://discord.com/developers/docs/topics/gateway#identifying)
/// - See also `IdentityGatewayMessage`
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IdentityProperties {
    pub os: String,
    pub browser: String,
    pub device: String,
}

/// Contains information to identify the app, which will be given at each shard start
///
/// See the [documentation](https://discord.com/developers/docs/topics/gateway#identifying)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IdentityGatewayMessage {
    pub token: String,
    pub intents: i64,
    pub properties: IdentityProperties
}