use serde::{Deserialize, Serialize};
use crate::user::User;

#[derive(Serialize, Deserialize, Debug, Clone, Ord, PartialOrd, PartialEq, Eq)]
pub struct ReadyEvent {
    /// The API version used
    #[serde(rename = "v")]
    api_version: u8,
    /// The current logged-in user
    user: User,
    /// The session ID, used for resuming connections
    session_id: String,
    /// The Gateway URL for resuming connections
    resume_gateway_url: String,
    /// [Shard information](https://discord.com/developers/docs/topics/gateway#sharding) associated with this session, if sent when identifying
    shard: Option<Vec<i64>>
}