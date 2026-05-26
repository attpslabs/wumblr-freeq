/**
 * FreeqClient — event-driven IRC client with AT Protocol identity and E2EE.
 *
 * Usage:
 *   const client = new FreeqClient({ url: 'wss://irc.freeq.at/irc', nick: 'mybot' });
 *   client.on('message', (channel, msg) => console.log(`${msg.from}: ${msg.text}`));
 *   client.connect();
 */
import { EventEmitter } from './events.js';
import type { AvSession, FreeqClientOptions, SaslCredentials, TransportState, PinnedMessage, WhoisInfo, HistoryOptions, EmitEventOptions, HeartbeatHandle } from './types.js';
export declare class FreeqClient extends EventEmitter {
    private transport;
    private _nick;
    private _authDid;
    /** Bearer token usable for `/agent/tools/*` HTTP calls. Populated
     *  from the server-emitted `NOTICE * :API-BEARER <session_id>` that
     *  fires immediately after SASL success. Bots use this to call
     *  diagnostic tools as themselves instead of as anonymous. */
    private _apiBearer;
    private _connectionState;
    private _registered;
    private opts;
    private ackedCaps;
    private sasl;
    private skipBrokerRefresh;
    private guestFallbackCount;
    /** Set when SASL was attempted and 904 was received. Suppresses any
     *  subsequent registration completion as a guest, and blocks outgoing
     *  PRIVMSGs that would silently leak under the guest identity. */
    private _saslFailed;
    /** Channels the server has flagged +E. Used to block plaintext sends
     *  when we don't (yet) have the passphrase, so messages don't leak
     *  unencrypted into a channel the rest of the room expects encrypted. */
    private _encryptedChannels;
    /** Current AWAY reason, or null if not away. Re-asserted on
     *  reconnect so the wire and UI states don't diverge after the
     *  server forgets us during the disconnect. */
    private _currentAway;
    private autoJoinChannels;
    private _joinedChannels;
    private backgroundWhois;
    private echoPlaintextCache;
    private batches;
    private pendingAwayReason;
    private _avSessions;
    private _activeAvSession;
    /** Lowercase nick → DID. Populated from numeric 330 (WHOIS) and from
     *  inbound `+freeq.at/account` tags. */
    private _nickToDid;
    /** DID → lowercase nick. Reverse cache for AGENT PAUSE/REVOKE which
     *  take nicks, not DIDs. */
    private _didToNick;
    /** Accumulating WHOIS info per nick. Multiple WHOIS numerics fire
     *  incrementally (311/312/319/330/671/673); we collect until 318
     *  (RPL_ENDOFWHOIS) and resolve the requestWhois() Promise. */
    private _whoisBuffer;
    /** Pending requestWhois() Promise resolvers, keyed by lowercase nick. */
    private _pendingWhois;
    /** Random-suffix nick collision retry counter. */
    private _nickCollisionRetries;
    /** Background heartbeat loop handle (set by startHeartbeat()). */
    private _agentHeartbeatTimer;
    /** Recently-seen coordination event IDs (TAGMSG + companion PRIVMSG carry
     *  the same eventId; we fire `coordinationEvent` only once per pair). */
    private _seenCoordinationEvents;
    constructor(opts: FreeqClientOptions);
    /** Current IRC nickname. */
    get nick(): string;
    /** Authenticated AT Protocol DID, or null if guest. */
    get authDid(): string | null;
    /** Bearer token for `/agent/tools/*` HTTP calls. Set automatically
     *  on SASL success; null while unauthenticated. Use as
     *  `Authorization: Bearer <client.apiBearer>` to make diagnostic
     *  calls as the same identity the IRC session is bound to. */
    get apiBearer(): string | null;
    /** Current connection state. */
    get connectionState(): TransportState;
    /** Whether IRC registration is complete (001 received). */
    get registered(): boolean;
    /** Set of channels we're currently in (lowercase). */
    get joinedChannels(): ReadonlySet<string>;
    /** Active AV sessions. */
    get avSessions(): ReadonlyMap<string, AvSession>;
    /** Active AV session ID we're participating in. */
    get activeAvSession(): string | null;
    /** Server origin for API calls. */
    get serverOrigin(): string;
    /** Connect to the IRC server. */
    connect(): void;
    /** Wait for the WebSocket send buffer to drain. Returns when
     *  `bufferedAmount` reaches 0 (or the WS is no longer open), or after
     *  `maxMs` (default 2000ms). Call before `disconnect()` if you need
     *  outbound messages (PRESENCE=offline, QUIT, etc.) to actually reach
     *  the server before the socket closes. */
    flush(maxMs?: number): Promise<void>;
    /** Disconnect from the server. */
    disconnect(): void;
    /** Force an immediate reconnect. */
    reconnect(): void;
    /** Set SASL credentials (call before connect, or before reconnect). */
    setSaslCredentials(creds: SaslCredentials): void;
    /** Send a message to a channel or user. */
    sendMessage(target: string, text: string, multiline?: boolean): void;
    /** Send a reply to a specific message. */
    sendReply(target: string, replyToMsgId: string, text: string, multiline?: boolean): void;
    /** Edit a previously sent message. */
    sendEdit(target: string, originalMsgId: string, newText: string, multiline?: boolean): void;
    /** Send a message with Markdown formatting. */
    sendMarkdown(target: string, text: string): void;
    /** Delete a message. */
    sendDelete(target: string, msgId: string): void;
    /** React to a message with an emoji. */
    sendReaction(target: string, emoji: string, msgId?: string): void;
    /** Remove our previous reaction to a message. */
    sendUnreact(target: string, emoji: string, msgId: string): void;
    /** Join a channel. */
    join(channel: string): void;
    /** Leave a channel. */
    part(channel: string): void;
    /** Set a channel's topic. */
    setTopic(channel: string, topic: string): void;
    /** Set a channel or user mode. */
    setMode(channel: string, mode: string, arg?: string): void;
    /** Kick a user from a channel. */
    kick(channel: string, nick: string, reason?: string): void;
    /** Invite a user to a channel. */
    invite(channel: string, nick: string): void;
    /** Set or clear away status. */
    setAway(reason?: string): void;
    /** Fire a WHOIS and resolve with parsed info when 318 (RPL_ENDOFWHOIS)
     *  arrives. Renamed from `whois()` — that name remains as a deprecated
     *  alias for one release. */
    requestWhois(nick: string, opts?: {
        timeoutMs?: number;
    }): Promise<WhoisInfo>;
    /** @deprecated Use `requestWhois(nick)` (returns `Promise<WhoisInfo>`).
     *  Kept for one release; calling this still fires the `whois` event
     *  on each numeric, same as before. */
    whois(nick: string): void;
    /** Request chat history for a target (channel or DM partner).
     *
     *  `opts.mode` selects:
     *    - 'latest' — most recent N messages
     *    - 'before' — N messages before `opts.msgid`
     *    - 'after'  — N messages after `opts.msgid`
     */
    requestHistory(opts: HistoryOptions): void;
    /** @deprecated Use the `HistoryOptions` form. The two-arg form is kept
     *  for backwards compatibility with freeq-app. */
    requestHistory(channel: string, before?: string): void;
    /** Request CHATHISTORY TARGETS — list of recent conversation targets
     *  (channels + DM partners with recent activity).
     *  Each result fires `historyTarget(target, timestamp?)`. */
    requestHistoryTargets(limit?: number): void;
    /** @deprecated Use `requestHistoryTargets(limit)`. CHATHISTORY TARGETS
     *  returns channels too, not just DMs; the original name was misleading.
     *  Kept for one release. */
    requestDmTargets(limit?: number): void;
    /** Pin a message. */
    pin(channel: string, msgid: string): void;
    /** Unpin a message. */
    unpin(channel: string, msgid: string): void;
    /** Send a raw IRC command. */
    raw(line: string): void;
    /** Set a channel encryption passphrase (ENC1). */
    setChannelEncryption(channel: string, passphrase: string): Promise<void>;
    /** Remove channel encryption. */
    removeChannelEncryption(channel: string): void;
    /** Initialize E2EE for DMs (called automatically after SASL success). */
    initializeE2EE(did: string): Promise<void>;
    /** Get the E2EE safety number for a DM partner. */
    getSafetyNumber(remoteDid: string): Promise<string | null>;
    /** Fetch pinned messages for a channel via REST API.
     *  Returns the fetched pins; also fires the `pins` event for any
     *  subscribers. Returns an empty array on failure. */
    fetchPins(channel: string): Promise<PinnedMessage[]>;
    private onTransportStateChange;
    private didForNick;
    /** Resolve nick to DID — set by the app layer for E2EE support. */
    nickToDid: ((nick: string) => string | undefined) | null;
    private resolveNickToDid;
    /** Parse a `+freeq.at/event=*` TAGMSG/PRIVMSG and emit `coordinationEvent`.
     *  De-dupes by eventId so the paired TAGMSG + companion PRIVMSG fire
     *  the event only once. */
    private emitCoordinationEvent;
    private signedPrivmsg;
    private cacheEchoPlaintext;
    private handleLine;
    private handleCap;
    private handleAuthenticate;
    private handleAvSessionState;
    /** Send IRC QUIT. Closes the session cleanly on the server side. */
    quit(reason?: string): void;
    /** JOIN multiple channels at once (comma-separated wire form). */
    joinMany(channels: string[]): void;
    /** PRIVMSG with arbitrary IRCv3 tags. Caller-managed escaping is handled
     *  by the SDK's format() helper. */
    sendTagged(target: string, text: string, tags: Record<string, string>): void;
    /** TAGMSG (tags-only, no body) to a target. */
    sendTagmsg(target: string, tags: Record<string, string>): void;
    /** Send a media attachment (image/audio/video URL with metadata).
     *  Server side stores the media tags; rich clients render the embed. */
    sendMedia(target: string, media: {
        url: string;
        mime?: string;
        alt?: string;
        width?: number;
        height?: number;
        durationMs?: number;
        sizeBytes?: number;
        fallback?: string;
    }): void;
    /** Attach link-preview metadata to a message. */
    sendLinkPreview(target: string, preview: {
        url: string;
        title?: string;
        description?: string;
        imageUrl?: string;
    }): void;
    /** Send a message and await the server-assigned msgid via echo-message.
     *  Resolves with the msgid the server stamps on the echo. Requires
     *  `echo-message` cap (negotiated by default). Timeouts after 5s. */
    sendAndAwaitEcho(target: string, text: string, tags?: Record<string, string>): Promise<string>;
    /** Send a threaded reply (alias for sendReply, named to match Rust SDK
     *  `reply_in_thread`). */
    sendReplyInThread(target: string, parentMsgId: string, text: string): void;
    /** Start a typing indicator in a target (channel or DM). */
    startTyping(target: string): void;
    /** Stop a typing indicator. */
    stopTyping(target: string): void;
    /** Sync lookup: nick → DID. Returns undefined if unknown.
     *  Auto-populated from WHOIS 330, JOIN account tags, and ACCOUNT notify. */
    getDidForNick(nick: string): string | undefined;
    /** Sync lookup: DID → current nick. Returns undefined if unknown.
     *  Needed for AGENT PAUSE/REVOKE which take nicks, not DIDs. */
    getNickForDid(did: string): string | undefined;
    /** Declare actor_class for this session. Class is one of:
     *  'agent' | 'external_agent' | 'human'. Broadcast to shared channels. */
    registerAgent(actorClass: 'agent' | 'external_agent' | 'human'): void;
    /** Submit a provenance declaration (JSON value, base64url-encoded on
     *  the wire). For agents, typically a FreeqBotDelegation/v1 cert. */
    submitProvenance(provenance: unknown): void;
    /** Update structured agent presence (state, status, task). */
    setPresence(state: string, status?: string, task?: string): void;
    /** Send a single heartbeat. */
    sendHeartbeat(state: string, ttlSeconds: number): void;
    /** Start a background heartbeat loop at the given interval (ms).
     *  TTL is set to 2× interval per Rust SDK convention. */
    startHeartbeat(intervalMs: number): HeartbeatHandle;
    /** Request approval from channel ops for a capability use. */
    requestApproval(channel: string, capability: string, resource?: string): void;
    /** Op-only. Pause target agent — expects PRESENCE=paused within 10s. */
    pauseAgent(nick: string, reason?: string): void;
    /** Op-only. Resume a paused agent. */
    resumeAgent(nick: string): void;
    /** Op-only. Revoke capabilities + force disconnect. */
    revokeAgent(nick: string, reason?: string): void;
    /** Op approval response. */
    approveAgent(nick: string, capability: string): void;
    /** Op denial response. */
    denyAgent(nick: string, capability: string, reason?: string): void;
    /** Emit a coordination event as paired TAGMSG (for storage) +
     *  companion PRIVMSG (for rich-client rendering). Returns the
     *  server-stored event ID. */
    emitEvent(channel: string, eventType: string, payload: unknown, opts?: EmitEventOptions): string;
    /** Sugar over `emitEvent` for `task_request`. Returns the task ID. */
    createTask(channel: string, description: string): string;
    /** Sugar for `task_update` — progress update on a task. */
    updateTask(channel: string, taskId: string, phase: string, summary: string): void;
    /** Sugar for `task_complete`. */
    completeTask(channel: string, taskId: string, summary: string, url?: string): void;
    /** Sugar for `task_failed`. */
    failTask(channel: string, taskId: string, error: string): void;
    /** Sugar for `evidence_attach` — attach evidence to a task. */
    attachEvidence(channel: string, taskId: string, evidenceType: string, summary: string, url?: string): void;
    /** Submit an agent manifest (base64-encoded TOML). */
    submitManifest(tomlContent: string): void;
    /** Spawn a child agent in a channel. */
    spawnAgent(channel: string, nick: string, capabilities: string[], ttlSeconds?: number, taskRef?: string): void;
    /** Despawn a child agent (parent only). */
    despawnAgent(nick: string): void;
    /** Send a message attributed to a spawned child agent. */
    sendAsChild(childNick: string, channel: string, text: string): void;
    /** Submit a spend record for the current action.
     *  (Server emits a `budget_exceeded` governance TAGMSG to us if this
     *  spend pushes us past the per-agent budget cap.) */
    submitSpend(channel: string, amount: number, unit: string, description: string, taskRef?: string): void;
    /** Set a per-agent budget on a channel (op only). */
    setBudget(channel: string, maxAmount: number, unit: string, period: string, sponsorDid: string): void;
    /** Query channel budget state (server replies with snapshot). */
    requestBudget(channel: string): void;
}
//# sourceMappingURL=client.d.ts.map