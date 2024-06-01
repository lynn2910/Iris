use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::FmtSubscriber;
use client::client::Client;
use client::events::Context;
use client::structures::events::ready::ReadyEvent;

const TOKEN: &str = include_str!("token");

#[derive(Default)]
struct EventHandler;

impl client::events::EventHandler for EventHandler {
    fn ready(&self, ctx: Context, ready: ReadyEvent) {
        println!("Shard {} is ready", ctx.shard_id);
        dbg!(&ready);
    }
}

#[tokio::main]
async fn main() {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(LevelFilter::TRACE)
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .expect("setting default subscriber failed");

    
    let client = Client::new(TOKEN, EventHandler::default());

    client.connect(1).await;
}
