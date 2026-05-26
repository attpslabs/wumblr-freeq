FROM rust:1.95-slim-bookworm AS builder
WORKDIR /src

RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./

COPY freeq-server/      freeq-server/
COPY freeq-sdk/         freeq-sdk/
COPY freeq-sdk-ffi/     freeq-sdk-ffi/
COPY freeq-tui/         freeq-tui/
COPY freeq-bots/        freeq-bots/
COPY freeq-auth-broker/ freeq-auth-broker/
COPY freeq-windows-core/ freeq-windows-core/
COPY freeq-bot-id/      freeq-bot-id/
COPY freeq-av-client/   freeq-av-client/
COPY freeq-av/          freeq-av/
COPY freeq-agent-kit/   freeq-agent-kit/
COPY freeq-eliza/       freeq-eliza/
COPY wumblr-issuer/     wumblr-issuer/
COPY wumblr-backend/    wumblr-backend/

RUN cargo build --release \
    -p freeq-server \
    -p freeq-auth-broker \
    -p wumblr-issuer \
    -p wumblr-backend


FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home wumblr \
    && mkdir -p /data && chown wumblr:wumblr /data

COPY --from=builder /src/target/release/freeq-server      /usr/local/bin/freeq-server
COPY --from=builder /src/target/release/freeq-auth-broker /usr/local/bin/freeq-auth-broker
COPY --from=builder /src/target/release/wumblr-issuer     /usr/local/bin/wumblr-issuer
COPY --from=builder /src/target/release/wumblr-backend    /usr/local/bin/wumblr-backend

VOLUME /data
USER wumblr

ENV RUST_LOG=info
EXPOSE 3080 3090 6667 8080 8787

ENTRYPOINT ["freeq-server"]
