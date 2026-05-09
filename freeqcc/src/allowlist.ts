// Optional multi-DID allowlist with per-DID capability scopes.
//
// Format: ~/.freeqcc/allowlist.json
//   {
//     "allowed": [
//       { "did": "did:plc:...", "label": "alice", "actions": ["join", "privmsg"] },
//       { "did": "did:key:...", "label": "peer agent" }   // no actions = chat only
//     ]
//   }
//
// Owner is ALWAYS allowed and gets the full action set (OWNER_ACTIONS below).
// A non-owner with no allowlist entry can't dispatch the bot at all. A non-
// owner with an entry but no `actions` can chat with the bot but cannot drive
// IRC actions. A non-owner with `actions: ["join"]` can ask the bot to join
// channels but nothing else.
import { readFile, writeFile } from "node:fs/promises";
import { paths, ensureDir } from "./paths.js";

/** All IRC actions the daemon's control socket understands. Owner gets all. */
export const OWNER_ACTIONS: readonly string[] = [
  "join",
  "part",
  "privmsg",
  "notice",
  "nick",
];

export interface AllowlistEntry {
  did: string;
  label?: string;
  /** Action names this DID is allowed to invoke via the control socket.
   *  Empty / undefined = chat-only (no IRC actions). */
  actions?: string[];
}

interface AllowlistFile {
  allowed?: AllowlistEntry[];
}

export async function loadAllowlist(): Promise<AllowlistEntry[]> {
  let raw: string;
  try {
    raw = await readFile(paths.allowlist, "utf8");
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === "ENOENT") return [];
    return [];
  }
  try {
    const parsed = JSON.parse(raw) as AllowlistFile;
    return (parsed.allowed ?? [])
      .filter((e): e is AllowlistEntry => typeof e.did === "string" && e.did.length > 0)
      .map((e) => ({
        did: e.did,
        label: typeof e.label === "string" ? e.label : undefined,
        actions: Array.isArray(e.actions)
          ? e.actions.filter((a) => typeof a === "string")
          : [],
      }));
  } catch {
    return [];
  }
}

export async function saveAllowlist(entries: AllowlistEntry[]): Promise<void> {
  await ensureDir();
  const data: AllowlistFile = { allowed: entries };
  await writeFile(paths.allowlist, JSON.stringify(data, null, 2) + "\n", { mode: 0o600 });
}

/** True if this DID is the owner OR appears in the allowlist. */
export function isAllowed(senderDid: string, ownerDid: string, allowlist: AllowlistEntry[]): boolean {
  if (senderDid === ownerDid) return true;
  return allowlist.some((e) => e.did === senderDid);
}

/** Action set for a sender. Owner: all. Allowlisted: their entry's `actions`.
 *  Anyone else: empty (the gate refuses them anyway, but useful for symmetry). */
export function actionsFor(
  senderDid: string,
  ownerDid: string,
  allowlist: AllowlistEntry[],
): string[] {
  if (senderDid === ownerDid) return [...OWNER_ACTIONS];
  const entry = allowlist.find((e) => e.did === senderDid);
  return entry?.actions ?? [];
}
