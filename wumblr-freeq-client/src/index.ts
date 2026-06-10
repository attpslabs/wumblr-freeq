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
import {
	uploadEncryptedImage,
	fetchEncryptedImage,
	type EimgUploadResult,
	type EimgFetchResult,
} from "@freeq/sdk/eimg";

export type { EimgUploadResult, EimgFetchResult };

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
	/** Per-channel (lowercased IRC name) member DID set, fed by the SDK's
	 *  member events. Used to derive the eimg group key. The local user's own
	 *  DID is always included. */
	private readonly memberDids = new Map<string, Set<string>>();

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
		this.setupMemberTracking();
	}

	/** Maintain a per-channel member DID set from the SDK's member events.
	 *  The SDK tracks nick↔DID globally and emits per-channel join/leave/list
	 *  events; we accumulate the DIDs per channel here so the eimg path can
	 *  derive the channel's group key. */
	private setupMemberTracking(): void {
		const ch = (channel: string) => channel.toLowerCase();
		const setFor = (channel: string): Set<string> => {
			const key = ch(channel);
			let s = this.memberDids.get(key);
			if (!s) {
				s = new Set<string>();
				this.memberDids.set(key, s);
			}
			// Always include our own DID — sender and recipients must derive the
			// key from the identical member set.
			s.add(this.options.did);
			return s;
		};
		this.client.on("memberJoined", (channel, member) => {
			if (member.did) setFor(channel).add(member.did);
		});
		this.client.on("membersList", (channel, members) => {
			const s = setFor(channel);
			for (const m of members) if (m.did) s.add(m.did);
		});
		this.client.on("memberLeft", (channel, nick) => {
			const did = this.client.getDidForNick(nick);
			if (did) this.memberDids.get(ch(channel))?.delete(did);
		});
		this.client.on("membersCleared", (channel) => {
			// Reset to just our own DID; the fresh roster will repopulate it.
			this.memberDids.set(ch(channel), new Set<string>([this.options.did]));
		});
	}

	/** The current member DIDs of a channel (sorted), for eimg key derivation.
	 *  Always includes the local user's DID. `channel` is the IRC channel name
	 *  (e.g. `#general`). */
	membersOf(channel: string): string[] {
		const s = this.memberDids.get(channel.toLowerCase());
		const out = s ? [...s] : [this.options.did];
		out.sort();
		return out;
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

	/**
	 * Encrypt an image with the channel's group key and upload the ciphertext to
	 * the ephemeral image store (`/api/v1/eimg`). The server only ever sees
	 * ciphertext; images are hard-deleted 24h after upload.
	 *
	 * Member DIDs are resolved from the tracked roster ([`membersOf`](#membersOf))
	 * unless `members` is passed explicitly. The key is derived from this set, so
	 * sender and recipients must agree on it (Phase A: a member who joins AFTER
	 * upload can't decrypt — the set is snapshot at upload). `epoch` is fixed at 0
	 * (no rotation until the MLS phase).
	 */
	uploadEncryptedImage(
		channel: string,
		contentType: string,
		imageBytes: Uint8Array,
		members?: string[],
		epoch = 0,
	): Promise<EimgUploadResult> {
		return uploadEncryptedImage(
			this.serverOrigin,
			this.options.did,
			channel,
			members ?? this.membersOf(channel),
			contentType,
			imageBytes,
			epoch,
		);
	}

	/**
	 * Fetch and decrypt an ephemeral image. Resolves to `{ gone: true }` if the
	 * image has expired (24h) or been deleted. Member DIDs default to the tracked
	 * roster unless passed explicitly.
	 */
	fetchEncryptedImage(
		imageId: string,
		channel: string,
		members?: string[],
		epoch = 0,
	): Promise<EimgFetchResult> {
		return fetchEncryptedImage(
			this.serverOrigin,
			imageId,
			this.options.did,
			channel,
			members ?? this.membersOf(channel),
			epoch,
		);
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
