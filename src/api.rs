use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use governor::clock::{Clock, QuantaClock, QuantaInstant, Reference};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc::error::TrySendError;
use tracing::{debug, error, warn};
use uuid::Uuid;

use crate::sender::QueuedEmail;
use crate::{AppState, tencent_ses, validate};

#[derive(Deserialize)]
pub struct SendEmailRequest {
    pub subject: String,
    pub destination: String,
    pub template_id: u64,
    #[serde(default)]
    pub template_data: serde_json::Value,
}

#[derive(Serialize)]
pub struct SendEmailResponse {
    pub id: Uuid,
    pub status: &'static str,
}

#[derive(Serialize)]
struct TencentTemplate {
    #[serde(rename = "TemplateID")]
    template_id: u64,
    #[serde(rename = "TemplateData")]
    template_data: String,
}

#[derive(Serialize)]
struct TencentSendEmailPayload {
    #[serde(rename = "FromEmailAddress")]
    from_email_address: String,
    #[serde(rename = "Subject")]
    subject: String,
    #[serde(rename = "Destination")]
    destination: Vec<String>,
    #[serde(rename = "Template")]
    template: TencentTemplate,
}

pub enum ApiError {
    InvalidEmail(String),
    RateLimited(u64),
    /// The send queue is full: the service is shedding load rather than
    /// making clients wait indefinitely behind a slow SES upstream.
    Overloaded,
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, mut body) = match self {
            ApiError::InvalidEmail(addr) => (
                StatusCode::BAD_REQUEST,
                json!({
                      "message": format!("invalid destination address: {addr}"),
                }),
            ),
            ApiError::RateLimited(sec) => (
                StatusCode::TOO_MANY_REQUESTS,
                json!({
                      "message": "rate limit exceeded for this recipient",
                      "try_again_sec": sec, // Prompt the user to try again later. Release server resource fast.
                }),
            ),
            ApiError::Overloaded => (
                StatusCode::SERVICE_UNAVAILABLE,
                json!(
                    {
                      "message": "mail service is busy, try again later",
                      "try_again_sec": 30, // Prompt the user to try again later. Release server resource fast.
                    }
                ),
            ),
            ApiError::Internal(message) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({
                    "message": "Internal error",
                    "detail": message,
                }),
            ),
        };
        // Every error carries the same top-level status so clients can
        // branch on a single field regardless of the HTTP status code.
        body["status"] = json!("failed");

        if status.is_server_error() {
            error!(%status, body = %body, "request failed");
        } else {
            warn!(%status, body = %body, "request rejected");
        }

        (status, Json(body)).into_response()
    }
}

/// Validates, normalizes and rate-limits the request, builds the signed
/// SES request, hands it off to the send queue, and responds — all before
/// the email is actually sent. The destination is normalized (and possibly
/// remapped via the email map) before rate limiting; the background worker
/// in `sender` does the real send. If the queue is full the request is shed
/// with `503 Service Unavailable` instead of blocking until a slot frees up.
#[tracing::instrument(
    name = "send_email",
    skip_all,
    fields(destination = %req.destination, template_id = req.template_id)
)]
pub async fn send_email(
    State(state): State<AppState>,
    Json(req): Json<SendEmailRequest>,
) -> Result<(StatusCode, Json<SendEmailResponse>), ApiError> {
    if !validate::is_valid_email(&req.destination) {
        return Err(ApiError::InvalidEmail(req.destination));
    }

    // Normalization includes the optional address mapping: rate limits key
    // on the resolved address, which is also where the email actually goes.
    if let Some(target) = state.email_map.redirect_for(&req.destination) {
        debug!(target, "recipient redirected by email map");
    }
    let destination = state.email_map.resolve(&req.destination);

    // We own the domain, and all valid receive address under the domain should
    // have been redirected to some real email service.
    if destination.ends_with("thunt.top") {
        return Err(ApiError::InvalidEmail(req.destination));
    }

    if let Err(ratelimited) = state.rate_limiter.check_key(&destination) {
        let possible_time: QuantaInstant = ratelimited.earliest_possible();
        // Nanos from now until the next conforming request, rounded up to
        // whole seconds. Saturates to 0 if the window has already passed.
        let possible_since_now: u64 = possible_time
            .duration_since(QuantaClock::default().now())
            .as_u64()
            .div_ceil(1_000_000_000);
        return Err(ApiError::RateLimited(possible_since_now));
    }

    let template_data = serde_json::to_string(&req.template_data)
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    let payload = TencentSendEmailPayload {
        from_email_address: state.from_address.to_string(),
        subject: req.subject,
        destination: vec![destination.clone()],
        template: TencentTemplate {
            template_id: req.template_id,
            template_data,
        },
    };
    let payload_json =
        serde_json::to_string(&payload).map_err(|err| ApiError::Internal(err.to_string()))?;

    let request = tencent_ses::build_send_email_request(
        &state.http_client,
        &state.secret_id,
        &state.secret_key,
        "",
        &state.endpoint,
        &state.region,
        &payload_json,
    )
    .map_err(|err| ApiError::Internal(err.to_string()))?;

    let id = Uuid::new_v4();
    match state.queue_tx.try_send(QueuedEmail {
        id,
        recipient: destination,
        request,
    }) {
        Ok(()) => {}
        // Load shedding: fail fast with 503 rather than waiting for a free
        // slot, so the client learns immediately that the email was not queued.
        Err(TrySendError::Full(_)) => return Err(ApiError::Overloaded),
        Err(TrySendError::Closed(_)) => {
            return Err(ApiError::Internal("send queue is closed".to_string()));
        }
    }
    debug!(id = %id, "queued for sending");

    Ok((
        StatusCode::ACCEPTED,
        Json(SendEmailResponse {
            id,
            status: "queued",
        }),
    ))
}
