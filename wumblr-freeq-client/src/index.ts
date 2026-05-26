/**
 * @wumblr/freeq-client — thin wrapper around @freeq/sdk for wumblr-specific
 * orchestration: present a wumblr-issued VerifiableCredential to freeq before
 * JOIN, attach the broker-issued web token to SASL.
 *
 * Flow on connect():
 *   1. POST credential to <serverOrigin>/api/v1/credentials/present
 *      so freeq's policy engine has it before any JOIN gates it.
 *   2. Open the WebSocket via @freeq/sdk's FreeqClient, SASL with
 *      method=web-token and the token in `token`.
 *   3. Once `ready`, the consumer can call join() / say() / on().
 *
 * The wrapper does NOT itself fetch the credential or the web-token —
 * the consumer (mobile/web app) is responsible for getting those from
 * wumblr-backend (`GET /credentials/wumblr_member`) and the broker
 * (`POST /session`) respectively.
 */

import { FreeqClient } from "@freeq/sdk";
import type { FreeqEvents, Message } from "@freeq/sdk";

export interface VerifiableCredential {
	type: "FreeqCredential/v1";
	issuer: string;
	subject: string;
	credential_type: string;
	claims: Record<string, unknown>;
	issued_at: string;
	expires_at?: string;
	signature: string;
}

export interface WumblrFreeqOptions {
	/** WebSocket URL — e.g. `wss://irc.wumblr.com/`. */
	wsUrl: string;
	/** HTTP origin of the same freeq server for REST calls (credential present, etc.).
	 *  E.g. `https://irc.wumblr.com`. If omitted, derived from `wsUrl` (wss→https, ws→http). */
	serverOrigin?: string;
	/** User's DID (e.g. `did:plc:abc…`). Used as SASL subject. */
	did: string;
	/** IRC nick — typically the user's handle minus its domain. */
	nick: string;
	/** One-time web token from broker `POST /session`. Single-use, 5min TTL server-side. */
	freeqWebToken: string;
	/** A wumblr-issued VerifiableCredential. Will be POSTed to /api/v1/credentials/present
	 *  before connecting so it's available when JOIN is attempted. */
	credential: VerifiableCredential;
	/** Channels to JOIN automatically once SASL succeeds. */
	channels?: string[];
}

/** Public event surface. Mirrors the relevant subset of FreeqEvents. */
export type WumblrFreeqEventMap = {
	ready: () => void;
	message: (channel: string, msg: Message) => void;
	join: (channel: string, nick: string) => void;
	part: (channel: string, nick: string) => void;
	authError: (err: string) => void;
	disconnected: (reason: string) => void;
	/** Fires after requestHistory() completes; `messages` is chronological. */
	historyBatch: (channel: string, messages: Message[]) => void;
};

export class WumblrFreeq {
	private readonly options: WumblrFreeqOptions;
	private readonly serverOrigin: string;
	private readonly client: FreeqClient;

	constructor(options: WumblrFreeqOptions) {
		this.options = options;
		this.serverOrigin = options.serverOrigin ?? deriveHttpOrigin(options.wsUrl);
		// Build the inner SDK client immediately so consumers can register
		// event listeners via `.on()` BEFORE calling connect(). The actual
		// network dial is deferred to connect().
		this.client = new FreeqClient({
			url: this.options.wsUrl,
			nick: this.options.nick,
			channels: this.options.channels,
			sasl: {
				method: "web-token",
				token: this.options.freeqWebToken,
				did: this.options.did,
				pdsUrl: "",
			},
		});
	}

	/**
	 * Connect end-to-end:
	 *   1. POST credential to freeq's /api/v1/credentials/present.
	 *   2. Open WebSocket + SASL (the SDK auto-joins options.channels on ready).
	 *
	 * Resolves on `ready`, rejects on `authError` or pre-ready disconnect.
	 */
	async connect(): Promise<void> {
		await this.presentCredential();

		return new Promise<void>((resolve, reject) => {
			let settled = false;
			this.client.once("ready", () => {
				if (settled) return;
				settled = true;
				resolve();
			});
			this.client.once("authError", (err: string) => {
				if (settled) return;
				settled = true;
				reject(new Error(`auth error: ${err}`));
			});
			this.client.once("disconnected", (reason: string) => {
				if (settled) return;
				settled = true;
				reject(new Error(`websocket disconnected before ready: ${reason}`));
			});

			this.client.connect();
		});
	}

	/** POST our credential to freeq so it's stored before any JOIN gates it. */
	private async presentCredential(): Promise<void> {
		const url = `${this.serverOrigin}/api/v1/credentials/present`;
		const res = await fetch(url, {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ credential: this.options.credential }),
		});
		if (!res.ok) {
			throw new Error(`present credential: ${res.status} ${await res.text()}`);
		}
		const body = (await res.json()) as { status: string; error?: string | null };
		if (body.status !== "accepted") {
			throw new Error(`present credential rejected: ${body.error ?? body.status}`);
		}
	}

	join(channel: string): void {
		this.client.join(channel);
	}

	say(channel: string, text: string): void {
		this.client.sendMessage(channel, text);
	}

	/** Request CHATHISTORY for a channel. Results arrive via the `historyBatch` event. */
	requestHistory(channel: string, count = 100): void {
		this.client.requestHistory({
			target: channel,
			mode: "latest",
			count,
		});
	}

	on<K extends keyof WumblrFreeqEventMap>(
		event: K,
		handler: WumblrFreeqEventMap[K],
	): void {
		// Pass-through to the SDK. The SDK's event signature for `message`
		// matches ours; we narrow to the subset we expose.
		this.client.on(event as keyof FreeqEvents, handler as never);
	}

	disconnect(): void {
		this.client.disconnect();
	}
}

function deriveHttpOrigin(wsUrl: string): string {
	if (wsUrl.startsWith("wss://")) {
		return "https://" + new URL(wsUrl).host;
	}
	if (wsUrl.startsWith("ws://")) {
		return "http://" + new URL(wsUrl).host;
	}
	throw new Error(`unsupported ws URL: ${wsUrl}`);
}
