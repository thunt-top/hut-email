use reqwest::{Client, Request};
use tokio::sync::mpsc;
use tracing::{error, info};
use uuid::Uuid;

/// A request that has already been built and signed, waiting to be sent.
pub struct QueuedEmail {
    pub id: Uuid,
    pub recipient: String,
    pub request: Request,
}

/// Consumes queued, pre-built requests and actually sends them. Runs for
/// the lifetime of the service; the channel closing (all senders dropped)
/// ends the loop.
pub async fn run(client: Client, mut queue: mpsc::Receiver<QueuedEmail>) {
    while let Some(item) = queue.recv().await {
        let QueuedEmail {
            id,
            recipient,
            request,
        } = item;
        match client.execute(request).await {
            Ok(response) => log_ses_response(id, &recipient, response).await,
            Err(err) => {
                error!(id = %id, recipient = %recipient, error = %err, "send failed");
            }
        }
    }
}

// Tencent Cloud's API always answers HTTP 200 (even for auth failures or
// rejected templates) and reports errors inside the JSON body instead:
// `{"Response":{"Error":{"Code":...,"Message":...}}}`. HTTP status alone
// can't tell success from failure here.
#[tracing::instrument(skip(response), fields(id = %id, recipient = %recipient))]
async fn log_ses_response(id: Uuid, recipient: &str, response: reqwest::Response) {
    let http_status = response.status();
    let body = match response.text().await {
        Ok(body) => body,
        Err(err) => {
            error!(
                http_status = %http_status,
                error = %err,
                "sent but failed to read the response body"
            );
            return;
        }
    };

    let error = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("Response")?.get("Error").cloned());

    match error {
        None => {
            info!(http_status = %http_status, "sent");
        }
        Some(error) => {
            error!(
                http_status = %http_status,
                ses_error = %error,
                "SES rejected the request"
            );
        }
    }
}
