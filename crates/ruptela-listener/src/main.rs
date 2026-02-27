use std::sync::Arc;
use clap::Parser;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::signal::unix::{signal, SignalKind};

mod commands;
mod crc;
mod normalize;
mod presence;
mod protocol;
mod server;

#[derive(Parser)]
#[command(about = "Ruptela GPS TCP listener")]
struct Args {
    /// Bind address
    #[arg(long, default_value = "0.0.0.0")]
    host: String,

    /// Bind port
    #[arg(long, default_value_t = 7700)]
    port: u16,

    /// Valkey/Redis connection URL
    #[arg(long, default_value = "redis://127.0.0.1:6379")]
    redis_url: String,

    /// Number of stream shards
    #[arg(long, default_value_t = 8)]
    shards: u64,

    /// Maximum number of concurrent device connections
    #[arg(long, default_value_t = 10_000)]
    max_connections: usize,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ruptela_listener=info".parse().unwrap()),
        )
        .init();

    let args = Args::parse();
    let addr = format!("{}:{}", args.host, args.port);
    let listener = TcpListener::bind(&addr).await.unwrap();

    let publisher = shared::publisher::Publisher::new(&args.redis_url, args.shards)
        .await
        .expect("Failed to connect to Valkey/Redis");
    // Extract the ConnectionManager before wrapping in Arc so it can be
    // cheaply cloned per-connection without going through Arc<Publisher>.
    let redis_manager = publisher.connection_manager();
    let publisher = Arc::new(publisher);

    let semaphore = Arc::new(Semaphore::new(args.max_connections));

    let mut sighup = signal(SignalKind::hangup()).expect("failed to register SIGHUP handler");

    tracing::info!(addr = %addr, "listening");

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((socket, peer_addr)) => {
                        let publisher = publisher.clone();
                        let redis_conn = redis_manager.clone();
                        let permit = Arc::clone(&semaphore).acquire_owned().await.unwrap();
                        tokio::spawn(async move {
                            server::handle_connection(socket, peer_addr, publisher, redis_conn).await;
                            drop(permit);
                        });
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "accept failed");
                    }
                }
            }
            _ = sighup.recv() => {
                tracing::info!("SIGHUP received, reloading configuration");
                break;
            }
        }
    }
}
