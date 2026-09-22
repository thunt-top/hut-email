mod api;
mod config;
mod email_map;
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

use crate::email_map::EmailMap;

#[derive(Clone)]
pub struct AppState {
    pub http_client: reqwest::Client,
    pub secret_id: Arc<str>,
    pub secret_key: Arc<str>,
    pub from_address: Arc<str>,
    pub endpoint: Arc<str>,
    pub region: Arc<str>,
    pub rate_limiter: Arc<RateLimiter<String, DashMapStateStore<String>, DefaultClock>>,
    pub email_map: Arc<EmailMap>,
    pub queue_tx: mpsc::Sender<sender::QueuedEmail>,
}

#[tokio::main]
async fn main() {
    // Tracing first: its verbosity comes from RUST_LOG (tracing's own
    // convention, read before anything else).
    init_tracing();
    dotenvy::dotenv().ok();

    let config_dir = env_or("CONFIG_DIR", "config");
    let cfg = config::Config::load(&config_dir)
        .unwrap_or_else(|err| panic!("invalid configuration: {err}"));

    let secret_id: Arc<str> = cfg.secret_id.clone().into();
    let secret_key: Arc<str> = cfg.secret_key.clone().into();
    let from_address: Arc<str> = cfg.from_address.clone().into();
    let endpoint: Arc<str> = cfg.endpoint.clone().into();
    let region: Arc<str> = cfg.region.clone().into();
    // LISTEN_ADDR stays env-overridable for orchestration (like PORT).
    let listen_addr = env_or("LISTEN_ADDR", &cfg.listen_addr);
    let max_per_hour = NonZeroU32::new(cfg.rate_limit_max_per_hour)
        .expect("rate_limit.max_per_hour must be a positive integer");

    let http_client = reqwest::Client::new();
    let (queue_tx, queue_rx) = mpsc::channel(256);

    let email_map_path = cfg
        .email_map_path
        .to_str()
        .expect("config dir must be valid UTF-8");
    let email_map = Arc::new(EmailMap::load(email_map_path));
    debug!(path = %cfg.email_map_path.display(), entries = email_map.len(), "email map loaded");

    debug!(%endpoint, %region, max_per_hour, "SES configuration");

    let state = AppState {
        http_client: http_client.clone(),
        secret_id,
        secret_key,
        from_address,
        endpoint,
        region,
        rate_limiter: Arc::new(RateLimiter::keyed(Quota::per_hour(max_per_hour))),
        email_map,
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

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
