# hut_email

A small proxy that turns a simple HTTP API into emails sent through
[Tencent Cloud SES](https://www.tencentcloud.com/products/ses). It validates
recipients, applies per-recipient rate limits, signs requests with the
TC3-HMAC-SHA256 signature scheme, and hands them to a background worker that
performs the actual send. Clients never touch Tencent credentials or the SES
API directly.

The service is intended for internal (intranet) use and performs no
authentication of its own — gate it at the network layer if that matters.

- **Stack**: Rust, axum 0.8, tokio, reqwest (rustls), tracing
- **Port**: `39788` by default
- **Config**: environment variables (optionally from a `.env` file)

## Deployment

### Configuration

All settings come from environment variables. A `.env` file in the working
directory is loaded automatically (see [`.env.example`](.env.example)).

| Variable | Required | Default | Purpose |
| --- | --- | --- | --- |
| `TENCENTCLOUD_SECRET_ID` | yes | — | Tencent Cloud API secret id, used to sign SES requests |
| `TENCENTCLOUD_SECRET_KEY` | yes | — | Tencent Cloud API secret key, used to sign SES requests |
| `SES_FROM_ADDRESS` | yes | — | Sender address; must be verified in the SES console |
| `SES_ENDPOINT` | no | `ses.tencentcloudapi.com` | Tencent Cloud SES endpoint |
| `SES_REGION` | no | `ap-hongkong` | Tencent Cloud region |
| `LISTEN_ADDR` | no | `0.0.0.0:39788` | HTTP listen address |
| `RATE_LIMIT_MAX_PER_HOUR` | no | `20` | Max queued sends per hour per recipient address |
| `RUST_LOG` | no | `info` | Log filter (tracing `EnvFilter` syntax) |

### Run natively

```sh
cp .env.example .env   # then fill in real values
cargo run --release
```

Useful log levels:

```sh
RUST_LOG=hut_email=debug cargo run --release          # queue/config detail
RUST_LOG=info,tower_http=debug cargo run --release    # + per-request HTTP logs
```

### Run with Docker

The image is built in four stages with
[cargo-chef](https://github.com/LukeMathWalker/cargo-chef) so dependency
compilation is cached; the final stage is a minimal `alpine:3.21` image
running an unprivileged user. `docker compose` injects the secrets from the
local `.env` at runtime — they never enter the image.

```sh
docker compose up -d --build    # builds the image and starts the service
docker compose logs -f          # follow the tracing output
docker compose down             # stop
```

The container maps `39788:39788`. Log verbosity is controlled with
`RUST_LOG` in `.env` (or `environment` in `compose.yaml`).

### Release and deploy to production

Production has no reliable direct connection to GitHub, so releases are built
by CI and shipped over by hand from a bridge machine (one with access to both
GitHub and production):

1. Cut a release: `git tag vX.Y.Z && git push --tags`. The
   [`release` workflow](.github/workflows/release.yml) builds the image on a
   GitHub-hosted runner and attaches it (`docker save | gzip`) to a GitHub
   Release — no secrets are needed for this step.
2. From the bridge machine, run [`scripts/deploy.sh`](scripts/deploy.sh)
   `[vX.Y.Z]` (defaults to the latest release). It downloads the image via
   `gh`, copies it and `compose.yaml` to production over SSH, then loads and
   restarts the container there.

`.env` is created by hand directly on the production host (see
[`.env.example`](.env.example)) and never leaves it — it isn't part of the
release artifact or the deploy script, so secrets never transit GitHub or
the bridge machine.

## CLI

`examples/send_email_cli.rs` is a small client that posts to the service. It
sends the same "reset password" email that
[`assets/sample.sh`](assets/sample.sh) assembles by hand for the raw Tencent
SES API — but through this service, so the sender address and signing are
handled server-side.

```sh
cargo run --example send_email_cli -- <destination> <verification_code> [options]
```

| Option | Default | Purpose |
| --- | --- | --- |
| `<destination>` | required | Recipient email address |
| `<verification_code>` | required | Template `verification_code` field |
| `--subject` | `HU&T Email Verification Code` | Email subject |
| `--action` | `RESET YOUR PASSWORD` | Template `action` field |
| `--template-id` | `212086` | Tencent SES template id |
| `--url` | `http://127.0.0.1:39788` | Service base URL |

Example:

```sh
cargo run --example send_email_cli -- \
    user@example.com 9527-D2H3-J34G-2KPK
```

The client prints the request payload, the HTTP status, and the JSON response.
Exit codes: `0` on success, `1` on transport or API error, `2` on usage error.
When the service responds with a rate limit or overload, the client also
prints a retry hint. A release build of the binary can be produced with
`cargo build --release --examples` (`target/release/examples/send_email_cli`).

## API

There is one endpoint: `POST /send-email`.

The service validates and rate-limits the request, builds and signs the
underlying SES request, and pushes it onto the send queue — all before
responding. The background worker in `src/sender.rs` performs the real send
sequentially, so a `202` means *accepted for sending*, not *sent*. If the
queue is full, the request is shed with `503` instead of blocking.

### Request

```json
{
  "subject": "HU&T Email Verification Code",
  "destination": "user@example.com",
  "template_id": 212086,
  "template_data": {
    "action": "RESET YOUR PASSWORD",
    "verification_code": "9527-D2H3-J34G-2KPK"
  }
}
```

| Field | Type | Notes |
| --- | --- | --- |
| `subject` | string | Email subject |
| `destination` | string | Single recipient address; validated with a regex |
| `template_id` | number | Tencent SES template id |
| `template_data` | object | Optional (defaults to `{}`); serialized into SES's `TemplateData` string |

The sender address is fixed server-side (`SES_FROM_ADDRESS`); clients cannot
choose it. Use `curl` for a quick test:

```sh
curl -s -X POST http://127.0.0.1:39788/send-email \
  -H 'Content-Type: application/json' \
  -d '{"subject":"HU&T Email Verification Code","destination":"user@example.com","template_id":212086,"template_data":{"action":"RESET YOUR PASSWORD","verification_code":"9527-D2H3-J34G-2KPK"}}'
```

### Success

`202 Accepted`

```json
{
  "id": "a1b2c3d4-...",
  "status": "queued"
}
```

`id` is a UUID that the service also logs while sending, so a queued email can
be traced through the logs.

### Errors

Every error body carries `"status": "failed"`, so clients can branch on a
single field regardless of the HTTP status code.

| Status | Body | Meaning |
| --- | --- | --- |
| `400` | `{"message":"invalid destination address: ...","status":"failed"}` | Destination failed validation |
| `429` | `{"message":"rate limit exceeded for this recipient","try_again_sec":N,"status":"failed"}` | Per-recipient hourly quota exhausted; `try_again_sec` is when the next send should be allowed |
| `503` | `{"message":"server is busy, try again later","try_again_sec":30,"status":"failed"}` | Send queue full; the request was shed, retry later |
| `500` | `{"message":"Internal error","detail":"...","status":"failed"}` | Server-side failure (e.g. queue closed, signing error) |
