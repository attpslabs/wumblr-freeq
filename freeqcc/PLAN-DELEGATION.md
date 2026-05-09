# Plan: Capability-Scoped Delegation

## Goal

Owner can grant a third-party DID a *narrowed* set of IRC actions. Example:
*"@friend.bsky.social can ask my bot to join channels but not change its nick."*

Today, every IRC action is owner-only. After this change, allowlisted DIDs each
have an `actions: string[]` of action names they're allowed to invoke. Owner
implicitly has all actions. Empty list (or no entry) = chat-only.

## Design

### Allowlist schema (extended)

`~/.freeqcc/allowlist.json`:
```json
{
  "allowed": [
    {
      "did": "did:plc:friend",
      "label": "alice",
      "actions": ["join", "privmsg"]
    },
    {
      "did": "did:plc:peer",
      "label": "peer agent",
      "actions": []
    }
  ]
}
```

- Missing `actions` → empty list → chat-only (current behavior).
- Listed actions are allowed; others rejected by control.ts.
- Owner action set is implicit; not configurable for v1.

### Token carries actions

`TokenContext` gains `actions: Set<string>`. Owner tokens get the full owner
set; allowlisted-DID tokens get `Set(allowlist[did].actions ?? [])`.

`control.ts` checks per-action:
- If `action` is in `ctx.actions` → run.
- Otherwise → `{ok:false, error:"action not granted"}`.

`isOwner` becomes a label only (used in log lines), not a gate.

### Live reload

Daemon `fs.watch`es `~/.freeqcc/allowlist.json` and re-parses on change.
Existing in-flight tokens keep their captured actions for safety; new tokens
use the new state.

### CLI surface

```
freeqcc grant <did> <action> [--label <label>]
  Add <action> to <did>'s allowed list. Creates the entry if new.
  Multiple grants = multiple commands or comma-separated <action>.

freeqcc revoke <did> [action]
  Remove a single action, or the whole entry if no action specified.

freeqcc grants
  List all allowlisted DIDs and their granted actions.
```

All CLI subcommands edit `~/.freeqcc/allowlist.json` atomically. The daemon's
fs.watch picks them up.

### Action vocabulary (v1)

Same as the agent-control plan: `join`, `part`, `privmsg`, `notice`, `nick`.

### Out of scope

- Phase-2 `AGENT APPROVE` server-side governance integration. The `creator_did`
  field in our delegation cert + the freeq server's `agent_capability_grants`
  table could plug together later. For now grants are local to the daemon.
- TTL on grants. Add later if needed.
- "Why" / audit trail for grants. Audit log of *invocations* already exists in
  control.ts log lines; could persist to a file later.

## Test plan

1. Start daemon as owner. Confirm `freeqcc grants` lists nothing.
2. `freeqcc grant did:key:zTEST join` — confirm it lands in allowlist.json.
3. Connect a probe IRC session authenticated as `did:key:zTEST`. DM yokota-bot
   `"please join #delegation-test"`.
4. Watch daemon: expect `[dispatch] zTEST → … (allowlisted)`, `[control]
   zTEST ran join ["#delegation-test"]`, `[reply] joined #delegation-test`.
5. From the same probe, DM `"send a notice to #freeq saying hi from a friend"`.
6. Watch daemon: expect `[control] denied: action not granted (notice for
   zTEST)`, and the bot's reply should politely decline.
7. `freeqcc revoke did:key:zTEST`. Probe DMs again — gets refusal (non-
   allowlisted).
