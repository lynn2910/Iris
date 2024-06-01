use std::sync::Arc;
use tokio::sync::RwLock;

pub struct CacheInstance {
    
}

type LockedResource<T> = Arc<RwLock<T>>;