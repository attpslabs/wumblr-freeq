# wumblr-deploy/

Self-contained Docker deployment for the public wumblr-freeq stack.

This is separate from upstream's [`deploy/`](../deploy/), which is for bare-metal systemd-based single-server deploys of freeq alone. `wumblr-deploy/` brings up the four-service wumblr stack (freeq + broker + issuer + nginx) and is the deployment surface for wumblr.com.

| Service | Binary             | Purpose                                                      |
|---------|--------------------|--------------------------------------------------------------|
| freeq   | `freeq-server`     | IRC-over-WebSocket; verifies VerifiableCredentials on JOIN.  |
| broker  | `freeq-auth-broker`| ATProto OAuth orchestrator; manages user sessions.           |
| issuer  | `wumblr-issuer`    | Signs `wumblr_member:<community>` VCs over PDS-public state. |
| nginx   | -                  | TLS-terminating reverse proxy for all four hostnames.        |

All three Rust services build from the same image (`wumblr-freeq:latest`); broker and issuer override `ENTRYPOINT`. One [`Dockerfile`](Dockerfile), three binaries, ~12-minute cold cargo build (then layer-cached).

## Routes served

| Public URL                                            | Service | Purpose                                  |
|-------------------------------------------------------|---------|------------------------------------------|
| `https://api.wumblr.com/verify/.well-known/did.json`  | issuer  | did:web issuer DID document              |
| `https://api.wumblr.com/credentials/*`                | issuer  | VerifiableCredential issuance            |
| `https://api.wumblr.com/*`                            | backend | OAuth-glue + session bridge              |
| `https://auth.wumblr.com/*`                           | broker  | OAuth login/callback/session             |
| `wss://irc.wumblr.com/*`                              | freeq   | IRC-over-WebSocket transport             |

## Bring up

```sh
docker compose \
  -f /opt/wumblr/wumblr-freeq/wumblr-deploy/docker-compose.yml \
  --env-file /opt/wumblr/.env \
  up -d --build
```

Single compose file covers everything — all four Rust services (freeq, broker, issuer, backend) build from one image and run alongside nginx. No external repo overlays needed.

## Required environment

`--env-file` must provide:

| Variable                       | Purpose                                                                                                          |
|--------------------------------|------------------------------------------------------------------------------------------------------------------|
| `BROKER_SHARED_SECRET`         | HMAC secret between broker and freeq-server (≥32 chars; `openssl rand -hex 32`)                                  |
| `WUMBLR_ISSUER_PRIVKEY_B64`    | Ed25519 issuer seed, 32 bytes base64url-no-pad. PERMANENT — rotating invalidates every issued credential.        |
| `WUMBLR_ISSUER_SHARED_SECRET`  | HMAC secret between wumblr-backend and the issuer (≥32 chars)                                                    |
| `WUMBLR_ISSUER_DID`            | (optional) override issuer DID. Default: `did:web:api.wumblr.com:verify`                                         |
| `WUMBLR_BROKER_PUBLIC_URL`     | (optional) override broker public URL. Default: `https://auth.wumblr.com`                                        |
| `WUMBLR_OPER_DIDS`             | (optional) comma-separated OPER DIDs for freeq-server                                                            |

Compose will refuse to start if `BROKER_SHARED_SECRET`, `WUMBLR_ISSUER_PRIVKEY_B64`, or `WUMBLR_ISSUER_SHARED_SECRET` are unset — `${VAR:?...}` substitution errors loudly.

## TLS

Mount a TLS cert + key at `/opt/wumblr/tls/cert.pem` + `/opt/wumblr/tls/key.pem` (referenced by the `nginx` service). For Cloudflare-proxied deployments, an origin cert with SAN `*.wumblr.com, wumblr.com` covers all three subdomains.

## Volumes

| Volume        | Used by | Contents                                              |
|---------------|---------|-------------------------------------------------------|
| `freeq-data`  | freeq   | SQLite DB, message-signing key, MOTD                  |
| `broker-data` | broker  | Encrypted OAuth-session SQLite                        |

Issuer is stateless (key loaded from env at boot).

## Rebuilding after a code change

```sh
cd /opt/wumblr/wumblr-freeq
git pull
docker compose \
  -f wumblr-deploy/docker-compose.yml \
  [-f /opt/wumblr/wumblr/infra/docker-compose.yml] \
  --env-file /opt/wumblr/.env up -d --build
```

Cold cargo build takes ~12 minutes; subsequent layer-cached builds with only the issuer changed take ~30 seconds.
