# M2 closeout

**Status:** shipped 2026-05-26.

**User-visible ship gate met:** opened `https://wumblr.com` in a fresh
incognito window → Sign in with Bluesky → consented on bsky.social → landed
on the wumblr AppShell with the `WB` community pinned → clicked into it →
auto-JOIN succeeded against `#wumblr-general` → message history backfilled →
sent and persisted a real message through the full freeq → broker → backend
→ issuer → freeq verification path.

**Deferred to M3** (not required for the user-visible flow):
- Setting an actual channel policy on `#wumblr-general` to enforce credential
  gating (the credential is being signed and presented; freeq just isn't
  *requiring* it yet).
- Two-dev test (one dev, one message — not yet two devs exchanging).

Both are pure ops/IRC work — no code changes — and naturally fold into M3
when community-creation lands.

## What lived through M2 — the actual final architecture

M2 wound up much larger in scope than originally planned (~10 commits became
~25) because the underlying architecture took two meaningful pivots. Final
shape:

### Two repos, one org

- **`attpslabs/wumblr`** (private) — Expo/RN frontend ONLY. Mobile + web app,
  branding, marketing. Deployed via Cloudflare Pages from `main`.
- **`attpslabs/wumblr-freeq`** (public, MIT) — everything else. Started life
  as a wumblr-branding fork of `chad/freeq`; absorbed all the backend code
  during the M2 reshape. Now contains:
    - `freeq-server/`, `freeq-auth-broker/`, `freeq-sdk/`, `freeq-sdk-js/` — upstream freeq components
    - `wumblr-issuer/` — Ed25519 VerifiableCredential signer (new in M2)
    - `wumblr-backend/` — OAuth-glue session bridge (moved from wumblr in M2)
    - `wumblr-lexicons/`, `wumblr-shared/` — TS packages (moved from wumblr in M2)
    - `wumblr-freeq-client/` — TypeScript SDK wrapper for the chat client (new in M2)
    - `wumblr-deploy/` — single docker-compose for the whole stack (new in M2)

The mobile app imports `@wumblr/{lexicons,shared,freeq-client}` from the
public repo via pnpm's `github:owner/repo#path:subdir` git-subdir spec, so
Cloudflare Pages builds don't need the public repo cloned alongside.

### Five services, one VPS, one image

`wumblr-deploy/docker-compose.yml` runs five containers:

| Service | Public URL                            | Image                  | Persists       |
|---------|---------------------------------------|------------------------|----------------|
| backend | `https://api.wumblr.com/*`            | `wumblr-freeq:latest`  | none (in-memory sessions) |
| issuer  | `https://api.wumblr.com/verify/*` + `/credentials/*` | same image, different entrypoint | none (stateless) |
| broker  | `https://auth.wumblr.com/*`           | same image             | `broker-data` (encrypted OAuth sessions) |
| freeq   | `wss://irc.wumblr.com/irc`            | same image             | `freeq-data` (SQLite) |
| nginx   | :443 / :80                            | nginx:1.27-alpine      | -              |

All four Rust services build from one image (`wumblr-freeq:latest`); broker,
issuer, and backend override `ENTRYPOINT` from the default `freeq-server`.

### OAuth: broker-led, single dance

M1 had wumblr-backend orchestrating OAuth via `@atproto/oauth-client-browser`
and pushing the resulting session blob to a `MockBroker`. M2 discovered
freeq-auth-broker IS the OAuth orchestrator (runs `/auth/login` → bsky.social
→ `/auth/callback` itself) — and that the two architectures couldn't
interoperate. So M2 ripped out the browser-client OAuth and switched to
broker-led redirect:

1. User clicks "Sign in with Bluesky" at `wumblr.com`
2. Browser redirects to `https://auth.wumblr.com/auth/login?handle=...&return_to=https://wumblr.com/auth/callback`
3. Broker bounces through bsky.social and back; returns to wumblr.com with
   `#oauth=<base64-encoded-json>` in the URL fragment
4. wumblr.com's `/auth/callback` decodes the fragment, extracts the
   `broker_token`, POSTs to backend `/session/exchange`
5. Backend calls broker's `/session` (server-to-server), gets `{did, handle,
   freeq_web_token, nick}`, mints an opaque `wb-…` bearer, returns it
6. Mobile stores the bearer + freeq web-token in `localStorage`

### Credentials: VerifiableCredentials signed by wumblr-issuer

For the chat path:

1. Frontend calls backend `/credentials/wumblr_member?community=wumblr`
   with the wumblr bearer
2. Backend resolves the bearer → DID, then HMAC-signs a request to issuer
   (internal, on the docker network)
3. Issuer signs a `FreeqCredential/v1` with Ed25519 (key from
   `WUMBLR_ISSUER_PRIVKEY_B64`, JCS canonicalization), returns the VC
4. Frontend POSTs the VC to freeq's `/api/v1/credentials/present`
5. Frontend opens WebSocket to `wss://irc.wumblr.com/irc`, SASL with
   `method=web-token` and the broker-issued token in the `signature` field
6. Frontend `JOIN #wumblr-general` — freeq policy engine checks for the
   credential (when policy is set), or allows unconditionally (current state)

Cross-crate parity is unit-tested:
`wumblr-issuer/tests/parity.rs` round-trips a credential signed by the issuer
and verifies it under `freeq_server::policy::credentials::verify_credential_signature`.
If that passes, every credential we mint will verify on the freeq side.

### Mobile UI: AppShell + sidebar + auto-connect

`apps/mobile/app/`:
- `index.tsx` — sign-in CTA when signed-out; AppShell + welcome when signed-in
- `login.tsx` — handle input → redirects to broker
- `auth/callback.tsx` — parses `#oauth=` fragment, calls `exchangeBrokerToken`
- `c/[community].tsx` — AppShell + ChatView for `/c/<community>`. Auto-connects
  on mount.

`apps/mobile/src/components/`:
- `AppShell.tsx` — `flex-row` layout: 64px `CommunitySidebar` + flex-1 main
- `CommunitySidebar.tsx` — Discord-style pinned communities (currently
  hardcoded to `[wumblr]`), user avatar footer that taps to sign out
- `ChatView.tsx` — channel header + `FlatList` of messages + send input

`apps/mobile/src/state/`:
- `session/SessionProvider.tsx` — wb-bearer, persisted to localStorage,
  validated against backend `/me` on mount
- `session/storage.ts` — localStorage helper + one-shot eviction of legacy
  `@atproto-oauth-client` IndexedDB from M1
- `chat/ChatProvider.tsx` — wraps `WumblrFreeq`, manages connect lifecycle,
  in-memory messages with CHATHISTORY backfill on JOIN, optimistic echo
- `backend/client.ts` — fetch wrappers for backend routes

## Key technical decisions made during M2

### Open/closed boundary went through three formulations before settling

1. First: "PDS-touching code is public" — too narrow; left wumblr-backend
   in the closed repo even though nothing about it is wumblr.com-specific.
2. Second: "protocol-public, product-closed" — slightly better but the
   line was blurry. Where do you draw "protocol"?
3. Final: **"frontend closed, everything else public."** Simple, unambiguous,
   easy to apply mechanically. Anything that's not literally Expo/RN code
   or wumblr branding lives in the public repo.

That decision drove the `apps/backend/` → `wumblr-freeq/wumblr-backend/`
move (with `git filter-repo` preserving the M1 commit history).

### Domain split: api / auth / irc

- `api.wumblr.com` (orange-cloud) — backend + issuer share this hostname.
  nginx splits: `/verify/*` and `/credentials/*` → issuer, everything else →
  backend.
- `auth.wumblr.com` (orange-cloud) — broker. Browser navigates here for the
  OAuth dance.
- `irc.wumblr.com` (orange-cloud) — freeq WebSocket transport. Originally
  gray-cloud for IRC idle-timeout reasons, flipped to orange after the
  Cloudflare-origin-cert-isn't-browser-trusted issue.

### Repo move from daveselfsurf to attpslabs

Started as `daveselfsurf/wumblr`. Moved to `attpslabs/wumblr` (private)
mid-M2 via GitHub Transfer Ownership. Lockstep clean. Sets up the
collaborator-friendly shape for the attpslabs org.

### Build artifact: freeq-sdk-js/dist/ committed

pnpm git-subdir installs don't run build scripts. Consumers depending on
`@freeq/sdk` via the git-subdir spec need `dist/` already in the tree.
Force-added it on the wumblr branch; upstream `chad/freeq` still gitignores
it. When `@freeq/sdk` gets republished to npm, this hack goes away.

### CORS layer placement matters in axum

The upstream `freeq-server/src/web.rs` applied its `CorsLayer` inline before
several `.merge(...)` calls — meaning the merged-in policy routes (notably
`/api/v1/credentials/present`) never received CORS headers. Browsers
blocked the OPTIONS preflight. M2 restructured this so the layer is
applied to `final_app` after all merges; now every route gets it.

### IPv4-only DNS in the freeq container

The freeq container's `tokio::net::lookup_host` returned both A and AAAA
records for `api.wumblr.com` (Cloudflare advertises both), and tried v6
first — but the docker default-bridge network has no public v6 routing,
so the connect timed out. Fixed in `wumblr-deploy/docker-compose.yml` via
`dns_opt: [single-request, no-aaaa]` on the freeq service. glibc 2.31+
respects this; runtime is glibc 2.36.

### did:web resolution rule

freeq-sdk follows did:web spec strictly: `did:web:api.wumblr.com:verify`
resolves to `https://api.wumblr.com/verify/did.json` (NOT
`/verify/.well-known/did.json`). Only DIDs without path components use
`.well-known/`. M2's issuer serves both paths to be forward-compatible.

### WebSocket bug — wrong path

mobile passed `wss://irc.wumblr.com/` to the SDK, but freeq listens at
`/irc`. Fixed by updating the default WS URL in `ChatProvider.tsx` to
`wss://irc.wumblr.com/irc`. Caught only during browser smoke-test; no
unit test would have surfaced it.

### Other production bugs caught only at deploy time

- nginx caches upstream IPs at start; container rebuilds change the
  backend's IP and nginx 502s until restart. Workaround: always
  `restart nginx` after `up -d --build`.
- `WumblrFreeq.on()` originally threw "connect() before calling other
  methods" if called before `connect()` — common pattern is registering
  handlers *before* connect. Fixed by constructing the inner FreeqClient
  in the wrapper's constructor.

## Lessons that inform M3

1. **Three commit sequences will start over-budget by 3x.** M2 was scoped
   at 10 commits; took 25. Almost all of the inflation came from operational
   gotchas (CORS scope, DNS v6, did:web path, freeq path mismatch) — each
   one was a 5-30 minute fix individually. Budget M3 with the same fudge.
2. **Cross-repo refactors are real work.** The boundary B reshape took
   roughly half a day across `git filter-repo`, cross-repo references,
   CI/CD, and dependency reshuffling. Worth it once; not again for a while.
3. **Cloudflare Pages + git-subdir packages are stable** once the artifacts
   are committed. Future M3+ frontend work just merges to `main` and Pages
   redeploys.
4. **freeq's policy/credentials substrate is solid**. Once the parity test
   passed, every credential we minted verified. The bugs were all *around*
   credentials (DNS, paths, CORS), never *in* the cryptographic core.

## What M3 starts with

### M2 leftovers (small)
- Set a real channel policy on `#wumblr-general` to enforce
  `wumblr_member:wumblr`. ~15 min on the VPS.
- Two-dev test with two real bsky accounts in #wumblr-general.

### M3 proper
- `wumblr-appview` — Rust + Axum, indexes `com.wumblr.*` records from the
  bsky Jetstream firehose into Postgres
- Real community creation flow: write `com.wumblr.community` record to user
  PDS; backend reads it via the appview for the credential gate
- Per-community channel switcher in the mobile sidebar (today's sidebar is
  one hardcoded entry)
- Replace ChatProvider's "messages are a single in-memory list" with
  per-channel + per-community state

## Commits in M2 (chronological, both repos)

### attpslabs/wumblr (closed, frontend)
```
…       repo-move + git-subdir spec switch
…       mobile: switch web sign-in to broker-led OAuth redirect
…       mobile: chat UI — Connect-to-chat button + #wumblr-general JOIN
…       mobile: fix chat WS URL — freeq listens at /irc, not /
…       mobile: bump @wumblr/freeq-client lockfile to constructor-fix
…       mobile: AppShell with permanent left sidebar + community routing
…       mobile: backfill chat history on JOIN via CHATHISTORY
…       slim to frontend-only (delete apps/backend/, packages/*, infra/)
```

### attpslabs/wumblr-freeq (public, everything else)
```
67893bf wumblr: MOTD + broker ALLOWED_ORIGINS for wumblr.com
a32ca86 wumblr-issuer: new crate signs wumblr_member:<community> VCs
e5bde39 wumblr-deploy: self-contained Docker stack for the public services
bc0f819 import wumblr-backend + lexicons + shared from daveselfsurf/wumblr
2fb0c5e wumblr-backend: integrate into workspace and deploy pipeline
fcf7c2a freeq-auth-broker: allow wumblr.com return_to URLs
9f3813d wumblr-deploy: drop stale 'closed-product' overlay references
930e5bf wumblr-backend: proxy credential issuance through to wumblr-issuer
d2eb28a nginx: route /credentials/* to backend, not issuer
06d43ee freeq-sdk-js: commit dist/ for git-distributed consumers
ceb9f8e wumblr-freeq-client: TS wrapper around @freeq/sdk
6526c2f wumblr-freeq-client: build inner FreeqClient in constructor
1de52a2 freeq-server: allow wumblr.com CORS origins
68f79b5 freeq-server: apply CorsLayer to fully-merged router, not partial
aad9dfc wumblr-deploy: force IPv4-only DNS in the freeq container
906eacb issuer: serve DID doc at /verify/did.json (did:web spec)
fb22a8d wumblr-freeq-client: expose requestHistory + historyBatch event
```
