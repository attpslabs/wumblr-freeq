# @wumblr/freeq-client

Thin TypeScript wrapper around [@freeq/sdk](https://github.com/attpslabs/wumblr-freeq/tree/main/freeq-sdk-js) for wumblr-specific orchestration:

1. POST a wumblr-issued [VerifiableCredential](https://github.com/attpslabs/wumblr-freeq/tree/main/wumblr-issuer) to freeq's `/api/v1/credentials/present` before any JOIN.
2. Open the IRC-over-WebSocket connection via the SDK's `FreeqClient`, SASL with `method="web-token"` and the broker-issued web token.
3. Once `ready`, JOIN the configured channels.

The wrapper doesn't fetch the credential or the web-token itself — the consumer (mobile/web app) is responsible for getting those from wumblr-backend (`GET /credentials/wumblr_member?community=…`) and the broker (`POST /session`).

## Install

```sh
pnpm add github:attpslabs/wumblr-freeq#path:wumblr-freeq-client
```

(The same git-subdir spec pattern used elsewhere in the wumblr stack.)

## Usage

```ts
import { WumblrFreeq } from "@wumblr/freeq-client";

const chat = new WumblrFreeq({
  wsUrl: "wss://irc.wumblr.com/",
  did: session.did,
  nick: session.nick,
  freeqWebToken: session.freeqWebToken,
  credential: credentialFromBackend,
  channels: ["#wumblr-general"],
});

chat.on("ready", () => {
  chat.say("#wumblr-general", "hello");
});
chat.on("message", (channel, msg) => {
  console.log(`[${channel}] ${msg.from}: ${msg.text}`);
});
chat.on("disconnected", (reason) => {
  console.warn("chat disconnected:", reason);
});

await chat.connect();
```

## Lifecycle

`connect()` is the one-shot entry point. It:
1. Presents the credential (fetch POST).
2. Opens the WS + SASL.
3. Resolves on `ready`, rejects on `authError` or pre-ready disconnect.

After `connect()` resolves, the configured `channels` are auto-joined by the SDK. Use `join()` / `say()` / `on()` for further interaction. Call `disconnect()` to tear down.

## Public API

| Method | Purpose |
|---|---|
| `new WumblrFreeq(opts)` | Construct. No I/O yet. |
| `connect()` | Present credential, open WS, complete SASL, auto-JOIN. |
| `join(channel)` | JOIN an additional channel post-ready. |
| `say(channel, text)` | Send a PRIVMSG. |
| `on(event, handler)` | Subscribe to `ready` / `message` / `join` / `part` / `authError` / `disconnected`. |
| `disconnect()` | Close the WebSocket. |
