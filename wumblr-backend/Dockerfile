# Multi-stage build for wumblr-backend.
#
# Builder stage compiles the release binary against the Cargo workspace.
# Runtime stage is a slim Debian with only what the binary needs (libssl,
# ca-certificates for HTTPS calls to PDSes + freeq-auth-broker).

FROM rust:1.95-slim-bookworm AS builder
WORKDIR /build

# System deps for any C bindings transitively pulled in.
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy the workspace manifest first to maximize layer caching.
COPY Cargo.toml Cargo.lock ./
COPY apps/backend/Cargo.toml apps/backend/Cargo.toml

# Pre-cache deps by building a stub.
RUN mkdir -p apps/backend/src \
    && echo 'fn main() { println!("stub"); }' > apps/backend/src/main.rs \
    && cargo build --release -p wumblr-backend \
    && rm -rf apps/backend/src target/release/deps/wumblr_backend*

# Now copy the real source and build for real.
COPY apps/backend/src apps/backend/src
RUN cargo build --release -p wumblr-backend


FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home wumblr

COPY --from=builder /build/target/release/wumblr-backend /usr/local/bin/wumblr-backend

USER wumblr
EXPOSE 8787

# Defaults intended to be overridden by docker-compose env vars.
ENV WUMBLR_LISTEN=0.0.0.0:8787 \
    RUST_LOG=wumblr_backend=info,tower_http=info

ENTRYPOINT ["/usr/local/bin/wumblr-backend"]
