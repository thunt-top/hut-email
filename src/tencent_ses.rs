//! Tencent Cloud SES `SendEmail` request builder, signed with TC3-HMAC-SHA256.
//! https://cloud.tencent.com/document/product/213/30654

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HOST, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Request};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

const SERVICE: &str = "ses";
const ACTION: &str = "SendEmail";
const VERSION: &str = "2020-10-02";
const ALGORITHM: &str = "TC3-HMAC-SHA256";

fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts a key of any length");
    mac.update(msg);
    mac.finalize().into_bytes().to_vec()
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Builds (but does not send) a signed `SendEmail` request against the
/// Tencent Cloud SES API.
///
/// `client` is reused so the caller keeps a single connection-pooled
/// `reqwest::Client` for the lifetime of the service. `secret_id`/`secret_key`
/// are the credentials from https://console.cloud.tencent.com/cam/capi,
/// `token` is the temporary credential token (empty string when using
/// long-term keys), `endpoint` is the API host (e.g.
/// "ses.tencentcloudapi.com"), `region` is the Tencent Cloud region (e.g.
/// "ap-hongkong"), and `payload` is the raw JSON request body (see
/// sample.sh for its shape). This only builds the request; sending it is
/// the caller's responsibility.
pub fn build_send_email_request(
    client: &Client,
    secret_id: &str,
    secret_key: &str,
    token: &str,
    endpoint: &str,
    region: &str,
    payload: &str,
) -> Result<Request, Box<dyn Error>> {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let date = DateTime::<Utc>::from_timestamp(timestamp as i64, 0)
        .ok_or("system time out of range")?
        .format("%Y-%m-%d")
        .to_string();

    // ************* Step 1: build the canonical request *************
    let canonical_headers = format!(
        "content-type:application/json; charset=utf-8\nhost:{endpoint}\nx-tc-action:{}\n",
        ACTION.to_lowercase(),
    );
    let signed_headers = "content-type;host;x-tc-action";
    let hashed_payload = sha256_hex(payload.as_bytes());
    let canonical_request =
        format!("POST\n/\n\n{canonical_headers}\n{signed_headers}\n{hashed_payload}");

    // ************* Step 2: build the string to sign *************
    let credential_scope = format!("{date}/{SERVICE}/tc3_request");
    let hashed_canonical_request = sha256_hex(canonical_request.as_bytes());
    let string_to_sign =
        format!("{ALGORITHM}\n{timestamp}\n{credential_scope}\n{hashed_canonical_request}");

    // ************* Step 3: compute the signature *************
    let secret_date = hmac_sha256(format!("TC3{secret_key}").as_bytes(), date.as_bytes());
    let secret_service = hmac_sha256(&secret_date, SERVICE.as_bytes());
    let secret_signing = hmac_sha256(&secret_service, b"tc3_request");
    let signature = hex::encode(hmac_sha256(&secret_signing, string_to_sign.as_bytes()));

    // ************* Step 4: build the Authorization header *************
    let authorization = format!(
        "{ALGORITHM} Credential={secret_id}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );

    // ************* Step 5: build the request *************
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_str(&authorization)?);
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    headers.insert(HOST, HeaderValue::from_str(endpoint)?);
    headers.insert(
        HeaderName::from_static("x-tc-action"),
        HeaderValue::from_static(ACTION),
    );
    headers.insert(
        HeaderName::from_static("x-tc-timestamp"),
        HeaderValue::from_str(&timestamp.to_string())?,
    );
    headers.insert(
        HeaderName::from_static("x-tc-version"),
        HeaderValue::from_static(VERSION),
    );
    headers.insert(
        HeaderName::from_static("x-tc-region"),
        HeaderValue::from_str(region)?,
    );
    headers.insert(
        HeaderName::from_static("x-tc-token"),
        HeaderValue::from_str(token)?,
    );

    let request = client
        .post(format!("https://{endpoint}"))
        .headers(headers)
        .body(payload.to_string())
        .build()?;

    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reference values cross-checked against the openssl commands in sample.sh:
    //   date="2026-08-13"; service="ses"; secret_key="testkey"; payload='{"a":1}'
    #[test]
    fn hashed_payload_matches_openssl() {
        assert_eq!(
            sha256_hex(br#"{"a":1}"#),
            "015abd7f5cc57a2dd94b7590f04ad8084273905ee33ec5cebeae62276a97f862"
        );
    }

    #[test]
    fn signing_key_chain_matches_openssl() {
        let secret_date = hmac_sha256(b"TC3testkey", b"2026-08-13");
        assert_eq!(
            hex::encode(&secret_date),
            "18d28e6f9c8dcba9c68598d4014f2d07f0b12afca4f999a36df440983b74fd69"
        );

        let secret_service = hmac_sha256(&secret_date, b"ses");
        assert_eq!(
            hex::encode(&secret_service),
            "fbfd49751abb89b74b99b8b4d9be35cbca79be967f02ce019a6bae374da4b008"
        );

        let secret_signing = hmac_sha256(&secret_service, b"tc3_request");
        assert_eq!(
            hex::encode(&secret_signing),
            "5b00674de5b902605ceb3c1c4b0ab84b3ec3f746dbe3480a56e79b4096002e6e"
        );
    }

    #[test]
    fn builds_request_with_expected_shape() {
        let request = build_send_email_request(
            &Client::new(),
            "AKIDxxxx",
            "testkey",
            "",
            "ses.tencentcloudapi.com",
            "ap-hongkong",
            r#"{"FromEmailAddress":"a@b.com","Subject":"s","Destination":["c@d.com"],"Template":{"TemplateID":1,"TemplateData":"{}"}}"#,
        )
        .unwrap();

        assert_eq!(request.method(), "POST");
        assert_eq!(request.url().as_str(), "https://ses.tencentcloudapi.com/");
        let headers = request.headers();
        assert_eq!(headers.get("x-tc-action").unwrap(), "SendEmail");
        assert_eq!(headers.get("x-tc-region").unwrap(), "ap-hongkong");
        assert_eq!(headers.get("x-tc-version").unwrap(), "2020-10-02");
        assert!(
            headers
                .get(AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("TC3-HMAC-SHA256 Credential=AKIDxxxx/")
        );
    }
}
