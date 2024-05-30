mod sharding;
pub mod client;
mod http;


pub const API_URL: &str = "https://discord.com/api/v10/";
pub const LIB_NAME: &str = "IrisClient.rs";
/// The interval between each checkup of the shards. Default to 500ms
pub const SHARD_WATCHER_INTERVAL: u64 = 500;