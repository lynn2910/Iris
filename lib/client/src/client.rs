use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use reqwest::Method;
use tokio::sync::{Mutex, RwLock};
use tracing::{error, trace};
use structures::gateway::BotGatewayEndpoint;
use crate::events::EventHandler;
use crate::http::{HttpRequestBuilder, RestClient};
use crate::sharding::shard::{Shard, ShardID};

pub type ShardPool = Arc<RwLock<HashMap<ShardID, Shard>>>;

/// The Discord Client
///
/// It'll store everything, from the Shard Manager to the cache and so on.
pub struct Client {
    /// The token of the app
    token: Arc<String>,
    /// Contain all shards, stored by their ids
    shard_pool: ShardPool,
    /// Whether the "stop" instruction has been called. If this is false, the client will always try to restart the shards.
    is_closed: Arc<Mutex<bool>>,
    /// The number of shards
    pub shards_count: u64,
    /// Contains the global HTTP client for the application.
    ///
    /// Please note that each shard has his own http client to offer a better multitasking
    global_http_client: RestClient,
    /// The event handler
    event_handler: Arc<dyn EventHandler>
}

impl Client {
    /// Create a new client
    pub fn new<H: EventHandler + 'static>(token: impl Into<String>, event_handler: H) -> Self {
        let token = Arc::new(token.into());

        Self {
            global_http_client: RestClient::new(token.clone()),
            shard_pool: Arc::new(RwLock::new(HashMap::new())),
            is_closed: Arc::new(Mutex::new(false)),
            shards_count: 0,
            event_handler: Arc::new(event_handler),
            token
        }
    }

    /// Connect the client to the API
    ///
    /// This method is blocking and will return if the client crashes or if he's stopped manually
    pub async fn connect(self, intents: i64) {
        connect_client(self, intents).await
    }

    pub(crate) fn get_token_copy(&self) -> Arc<String> {
        self.token.clone()
    }

    pub(crate) fn get_event_handler_copy(&self) -> Arc<dyn EventHandler> {
        self.event_handler.clone()
    }

    pub(crate) fn get_shard_pool_clone(&self) -> ShardPool {
        self.shard_pool.clone()
    }
}

async fn get_client_informations(rest: &RestClient) -> reqwest::Result<BotGatewayEndpoint> {
    let request = rest.client.request(Method::GET, crate::url!("/{}/bot", "gateway"));

    let r = rest.send_request(
        HttpRequestBuilder::new(request).require_token(true)
    ).await;

    match r {
        Ok(res) => res.json().await,
        Err(e) => Err(e)
    }
}

async fn connect_client(client: Client, intents: i64){
    let client_informations = match get_client_informations(&client.global_http_client).await {
        Ok(bge) => bge,
        Err(e) => {
            error!(target: "IrisClient", "Cannot obtain the client's informations: {e}");
            return;
        },
    };

    #[cfg(feature = "verbose")]
    {
        trace!(target: "iris::client", "Websocket url is {}", client_informations.url);
        trace!(target: "iris::client", "Discord recommend {} shard(s)", client_informations.shards);
        trace!(target: "iris::client", "User is allowed to start {} session(s)", client_informations.session_start_limit.total);
        trace!(target: "iris::client", "User has {} allowed sessions remaining", client_informations.session_start_limit.remaining);
        trace!(target: "iris::client", "The session limit will reset after {}ms", client_informations.session_start_limit.reset_after);
        trace!(target: "iris::client", "User can start up to {} sessions simultaneously", client_informations.session_start_limit.max_concurrency);
    }

    let mut shard_pool = client.shard_pool.write().await;

    // Create the given number of shards
    for i in 0..client_informations.shards {
        shard_pool.insert(
            i,
            Shard::new(
                i,
                client.get_token_copy(),
                client.get_event_handler_copy()
            )
        );
    }
    drop(shard_pool);

    // Start a big ass blocking loop
    let interval = Duration::from_millis(crate::SHARD_WATCHER_INTERVAL);
    loop {
        let mut shard_pool = client.shard_pool.write().await;
        let is_client_closed = *client.is_closed.lock().await;

        for (k, shard) in shard_pool.iter_mut() {
            let is_connected = shard.is_connected().await;

            if !is_client_closed && !is_connected {
                trace!(target: "iris::client", "Connect the shard {k}");
                // first, clean everything
                shard.close_shard().await;

                // then, connect again the shard
                shard.connect(&client_informations, intents, *k, client.get_shard_pool_clone()).await;
                trace!(target: "iris::client", "Shard {k} is connected");
            } else if is_client_closed && is_connected {
                trace!(target: "iris::client", "Client closed; the shard {k} received the signal.");
                shard.close_shard().await;
            }
        }

        drop(shard_pool);
        tokio::time::sleep(interval).await;
    }
}
