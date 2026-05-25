# wumblr-backend

Rust + Axum. Product API for wumblr — approval queue, freeq credential issuer, glue between mobile clients, freeq-auth-broker, and wumblr-appview.

**M1 scope (this commit):** scaffolded, runs, serves health + OAuth client metadata + JWKS (empty placeholder) + DID documents. No OAuth logic, no DB, no credential signing yet — those land in subsequent M1 steps and M2.

## Run

```sh
cargo run -p wumblr-backend
```

Defaults:
- Binds `0.0.0.0:8787`
- Reports public origin as `http://127.0.0.1:8787` (overridable for production)
- Broker URL `http://127.0.0.1:3080` (unused in step 2)

Override with env vars (or CLI flags):

```sh
WUMBLR_LISTEN=0.0.0.0:8787 \
WUMBLR_PUBLIC_ORIGIN=https://wumblr.com \
WUMBLR_BROKER_URL=https://broker.wumblr.com \
WUMBLR_ADMIN_DIDS=did:plc:abc... \
cargo run -p wumblr-backend
```

## Endpoints (current)

| Path | Purpose |
| --- | --- |
| `GET /health` | Liveness + version |
| `GET /oauth-client-metadata.json` | ATProto OAuth 2.0 client metadata (web + native redirects, DPoP-bound) |
| `GET /jwks.json` | OAuth client JWKS — **empty placeholder until M1 step 4** |
| `GET /.well-known/did.json` | `did:web:wumblr.com` DID document |
| `GET /verify/.well-known/did.json` | `did:web:wumblr.com:verify` — freeq credential issuer (verification methods empty until M2) |

## Smoke test

```sh
curl -s http://127.0.0.1:8787/health | jq
curl -s http://127.0.0.1:8787/oauth-client-metadata.json | jq
curl -s http://127.0.0.1:8787/.well-known/did.json | jq
```

## Plan refs

- §2 (backend role + endpoints)
- §6.5 (lexicons backend reads/writes)
- §11 (risk: OAuth-client-sharing, resolved by freeq-auth-broker)
