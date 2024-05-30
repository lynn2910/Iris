use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::FmtSubscriber;
use client::client::Client;

const TOKEN: &str = include_str!("token");

#[tokio::main]
async fn main() {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(LevelFilter::TRACE)
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .expect("setting default subscriber failed");

    
    let client = Client::new(TOKEN);

    client.connect(1).await;
}
