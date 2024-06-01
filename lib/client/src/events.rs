use structures::events::ready::ReadyEvent;
use crate::client::ShardPool;
use crate::sharding::shard::ShardID;

#[async_trait::async_trait]
pub trait EventHandler: Send + Sync {
    fn ready(&self, _ctx: Context, _d: ReadyEvent) {}
}

pub struct Context {
    pub shard_pool: ShardPool,
    pub shard_id: ShardID
}