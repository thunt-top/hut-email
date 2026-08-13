mod api;
mod sender;
mod tencent_ses;
mod validate;

use std::{num::NonZeroU32, sync::Arc};

use axum::Router;
use axum::routing::post;
use governor::{Quota, RateLimiter, clock::DefaultClock, state::keyed::DashMapStateStore};
use tokio::sync::mpsc;
use tower_http::trace::TraceLayer;
use tracing::{debug, info};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
pub struct AppState {
    pub http_client: reqwest::Client,
    pub secret_id: Arc<str>,
    pub secret_key: Arc<str>,
    pub from_address: Arc<str>,
    pub endpoint: Arc<str>,
    pub region: Arc<str>,
    pub rate_limiter: Arc<RateLimiter<String, DashMapStateStore<String>, DefaultClock>>,
    pub queue_tx: mpsc::Sender<sender::QueuedEmail>,
}

#[tokio::main]
async fn main() {
    init_tracing();
    dotenvy::dotenv().ok();
    let secret_id: Arc<str> = require_env("TENCENTCLOUD_SECRET_ID").into();
    let secret_key: Arc<str> = require_env("TENCENTCLOUD_SECRET_KEY").into();
    let from_address: Arc<str> = require_env("SES_FROM_ADDRESS").into();
    let endpoint: Arc<str> = env_or("SES_ENDPOINT", "ses.tencentcloudapi.com").into();
    let region: Arc<str> = env_or("SES_REGION", "ap-hongkong").into();
    let listen_addr = env_or("LISTEN_ADDR", "0.0.0.0:39788");

    let max_per_hour: NonZeroU32 = env_or("RATE_LIMIT_MAX_PER_HOUR", "20")
        .parse()
        .ok()
        .and_then(NonZeroU32::new)
        .expect("RATE_LIMIT_MAX_PER_HOUR must be a positive integer");

    let http_client = reqwest::Client::new();
    let (queue_tx, queue_rx) = mpsc::channel(256);

    debug!(%endpoint, %region, max_per_hour, "SES configuration");

    let state = AppState {
        http_client: http_client.clone(),
        secret_id,
        secret_key,
        from_address,
        endpoint,
        region,
        rate_limiter: Arc::new(RateLimiter::keyed(Quota::per_hour(max_per_hour))),
        queue_tx,
    };

    // Sends already-built requests off the queue; runs independently of
    // request handlers, which return as soon as a request is queued.
    tokio::spawn(sender::run(http_client, queue_rx));

    let app = Router::new()
        .route("/send-email", post(api::send_email))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&listen_addr)
        .await
        .unwrap_or_else(|err| panic!("failed to bind {listen_addr}: {err}"));
    info!(%listen_addr, "hut_email listening");
    axum::serve(listener, app).await.expect("server error");
}

/// Log output is configured with `RUST_LOG` (or `RUST_LOG`-style directives),
/// defaulting to `info` when unset. See `tracing_subscriber::EnvFilter` for
/// the directive syntax, e.g. `RUST_LOG=hut_email=debug` or
/// `RUST_LOG=hut_email=debug,tower_http=debug`.
fn init_tracing() {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .init();
}

fn require_env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("missing required env var {key}"))
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
