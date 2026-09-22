//! Example CLI client for the `hut_email` service.
//!
//! Sends the same "reset password" email that `assets/sample.sh` assembles by
//! hand for the raw Tencent SES API, but through the local HTTP endpoint:
//! the service fills in the sender address (`SES_FROM_ADDRESS`) and signs the
//! SES request itself, so the client only supplies subject, destination,
//! template id, and template data.
//!
//! ```text
//! Usage:
//!   cargo run --example send_email_cli -- <destination> <verification_code> [options]
//!
//! Usage:
//!   cargo run --example send_email_cli -- <destination> <verification_code> [options]
//!
//! Options:
//!   --subject        Email subject             [default: HU&T Email Verification Code]
//!   --action         Template \"action\" field   [default: RESET YOUR PASSWORD]
//!   --template-id    Tencent SES template ID   [default: 212086]
//!   --url            Service base URL          [default: http://127.0.0.1:39788]
//!
//! ```

use serde_json::{Value, json};
use std::time::Duration;

const DEFAULT_SUBJECT: &str = "HU&T Email Verification Code";
const DEFAULT_ACTION: &str = "RESET YOUR PASSWORD";
const DEFAULT_TEMPLATE_ID: u64 = 212086;
const DEFAULT_URL: &str = "http://127.0.0.1:39788";

const USAGE: &str = "\
Usage:
  cargo run --example send_email_cli -- <destination> <verification_code> [options]

Options:
  --subject        Email subject             [default: HU&T Email Verification Code]
  --action         Template \"action\" field   [default: RESET YOUR PASSWORD]
  --template-id    Tencent SES template ID   [default: 212086]
  --url            Service base URL          [default: http://127.0.0.1:39788]";

struct Args {
    destination: String,
    verification_code: String,
    subject: String,
    action: String,
    template_id: u64,
    base_url: String,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);

    let mut destination = None;
    let mut verification_code = None;
    let mut subject = DEFAULT_SUBJECT.to_string();
    let mut action = DEFAULT_ACTION.to_string();
    let mut template_id = DEFAULT_TEMPLATE_ID;
    let mut base_url = DEFAULT_URL.to_string();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--subject" => {
                subject = args.next().ok_or("missing value after --subject")?;
            }
            "--action" => {
                action = args.next().ok_or("missing value after --action")?;
            }
            "--template-id" => {
                template_id = args
                    .next()
                    .ok_or("missing value after --template-id")?
                    .parse()
                    .map_err(|_| "--template-id must be a positive integer".to_string())?;
            }
            "--url" => {
                base_url = args.next().ok_or("missing value after --url")?;
            }
            _ if arg.starts_with("--") => return Err(format!("unknown option: {arg}")),
            _ if destination.is_none() => destination = Some(arg),
            _ if verification_code.is_none() => verification_code = Some(arg),
            _ => return Err(format!("unexpected extra argument: {arg}")),
        }
    }

    Ok(Args {
        destination: destination.ok_or("missing <destination>")?,
        verification_code: verification_code.ok_or("missing <verification_code>")?,
        subject,
        action,
        template_id,
        base_url,
    })
}

#[tokio::main]
async fn main() {
    let args = match parse_args() {
        Ok(args) => args,
        Err(err) => {
            eprintln!("error: {err}\n");
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };

    // Same shape as the `TencentSendEmailPayload` in src/api.rs, minus the
    // sender address, which the service derives from `SES_FROM_ADDRESS`.
    let payload = json!({
        "subject": args.subject,
        "destination": args.destination,
        "template_id": args.template_id,
        "template_data": {
            "action": args.action,
            "verification_code": args.verification_code,
        },
    });

    let endpoint = format!("{}/send-email", args.base_url.trim_end_matches('/'));
    println!("POST {endpoint}");
    println!(
        "{}",
        serde_json::to_string_pretty(&payload).expect("payload is serializable")
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("failed to build HTTP client");

    let response = match client.post(&endpoint).json(&payload).send().await {
        Ok(response) => response,
        Err(err) => {
            eprintln!("error: could not reach {endpoint}: {err}");
            std::process::exit(1);
        }
    };

    let status = response.status();
    let body: Value = match response.json().await {
        Ok(body) => body,
        Err(err) => {
            eprintln!("error: got {status} but could not decode the response body: {err}");
            std::process::exit(1);
        }
    };

    println!("{status}");
    println!(
        "{}",
        serde_json::to_string_pretty(&body).expect("response body is serializable")
    );

    if status.is_success() {
        std::process::exit(0);
    }

    if let Some(sec) = body.get("try_again_sec").and_then(Value::as_u64) {
        eprintln!("hint: retry in {sec}s");
    }
    std::process::exit(1);
}
