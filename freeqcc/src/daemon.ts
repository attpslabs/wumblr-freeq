// Long-lived freeqcc daemon: load identity + owner + delegation,
// connect to freeq, listen for DMs, dispatch owner DMs to claude.
//
// Phase 5 wires connect + announce + heartbeat. The DM gate + claude
// dispatch (phase 6) hangs off the returned client.
import { loadOrCreateIdentity } from "./identity.js";
import { loadOrPromptOwner } from "./owner.js";
import { loadOrMintDelegation } from "./delegation.js";
import { connect, type Connected } from "./connect.js";

export interface DaemonOptions {
  /** IRC nick. If omitted, derives from owner handle: `<owner>-agent` (truncated). */
  nick?: string;
  serverUrl?: string;
}

/** Default nick: `<owner-handle>-agent`, truncated to fit IRC nick limits. */
function deriveDefaultNick(handle: string): string {
  const base = handle.replace(/[^a-zA-Z0-9.-]/g, "").toLowerCase();
  const proposed = `${base}-agent`;
  // Most IRC servers cap nicks at 32; freeq is permissive but keep it sane.
  return proposed.length > 30 ? proposed.slice(0, 30) : proposed;
}

export async function runDaemon(opts: DaemonOptions = {}): Promise<Connected> {
  const agent = await loadOrCreateIdentity();
  const owner = await loadOrPromptOwner();
  const delegation = await loadOrMintDelegation({ agent, owner });
  const nick = opts.nick ?? deriveDefaultNick(owner.handle);

  console.log("─── freeqcc daemon ───");
  console.log(`agent DID:      ${agent.did}${agent.isFresh ? " (fresh)" : ""}`);
  console.log(`owner:          @${owner.handle} (${owner.did})`);
  console.log(`delegation:     ${delegation.signature ? "signed" : "unsigned (v1.0)"}`);
  console.log(`server:         ${opts.serverUrl ?? "wss://irc.freeq.at/irc"}`);
  console.log(`nick:           ${nick}`);
  console.log("──────────────────────");

  const conn = await connect({
    identity: agent,
    owner,
    delegation,
    nick,
    serverUrl: opts.serverUrl,
  });

  console.log(`✓ connected as ${conn.nick}`);
  console.log(`  DM @${conn.nick} from @${owner.handle} to talk to your local Claude Code.`);

  // Phase 6 will install the owner-DID gate + claude dispatch here.
  // For now: just log incoming DMs so we can confirm the round-trip works.
  conn.client.on("message", (channel: string, msg: { from: string; text: string; isSelf?: boolean }) => {
    if (msg.isSelf) return;
    if (channel.startsWith("#") || channel.startsWith("&")) return; // ignore channel msgs
    console.log(`[DM] ${msg.from} → ${conn.nick}: ${msg.text}`);
  });

  // Graceful shutdown on SIGINT/SIGTERM
  const shutdown = async (sig: string): Promise<void> => {
    console.log(`\n[${sig}] shutting down...`);
    await conn.stop(`signal ${sig}`);
    process.exit(0);
  };
  process.once("SIGINT", () => void shutdown("SIGINT"));
  process.once("SIGTERM", () => void shutdown("SIGTERM"));

  return conn;
}
