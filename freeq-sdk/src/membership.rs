//! Opt-in channel membership tracking, **keyed by DID**.
//!
//! freeq treats the AT Protocol **DID** as the durable identity and the nick as
//! a display alias permanently owned by that DID (the server enforces nick
//! ownership and reclaims a nick by DID on reconnect). For access control and
//! key derivation — e.g. deriving a channel's [`crate::e2ee_did::GroupKey`] for
//! the ephemeral image store — what matters is the *set of member DIDs*, not
//! nicks.
//!
//! This helper is **opt-in and stateless toward the network**: feed it the
//! [`Event`]s your read loop already produces via [`Membership::apply`], then
//! query [`Membership::member_dids`]. It does not talk to the server.
//!
//! # Model
//!
//! - The public state is `channel → {DID}`.
//! - A nick→DID index is kept *internally* only to translate nick-based removal
//!   events (PART / KICK / QUIT, which carry a nick, not a DID) back to a DID.
//! - **NICK changes are a no-op for the DID set** — the DID is unchanged, so the
//!   member set (and any derived key) is stable across renames. Only the
//!   internal nick index is updated.
//!
//! # Limitations
//!
//! - Only members whose `JOIN` carried an `account` (DID) are tracked. Guests
//!   (account `*`) are ignored. The initial `NAMES`/`353` roster carries nicks
//!   only (no DIDs), so pre-existing members aren't seen until they re-`JOIN`
//!   or the tracker is started from an empty channel. This is intentional —
//!   membership accumulates forward from when tracking begins.

use std::collections::{HashMap, HashSet};

use crate::event::Event;

/// Tracks channel membership as a set of DIDs, updated from [`Event`]s.
#[derive(Debug, Default, Clone)]
pub struct Membership {
    /// channel (lowercased) → set of member DIDs.
    by_channel: HashMap<String, HashSet<String>>,
    /// channel (lowercased) → (nick → DID), to resolve nick-based removals.
    nick_index: HashMap<String, HashMap<String, String>>,
}

impl Membership {
    /// Create an empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    fn key(channel: &str) -> String {
        channel.to_lowercase()
    }

    /// Feed one event into the tracker. Events that don't affect membership are
    /// ignored, so it's safe to forward *every* event from your read loop.
    pub fn apply(&mut self, event: &Event) {
        match event {
            Event::Joined {
                channel,
                nick,
                account,
            } => {
                // Only DID-bearing joins are tracked (guests have account = None).
                if let Some(did) = account {
                    let ch = Self::key(channel);
                    self.by_channel
                        .entry(ch.clone())
                        .or_default()
                        .insert(did.clone());
                    self.nick_index
                        .entry(ch)
                        .or_default()
                        .insert(nick.clone(), did.clone());
                }
            }
            Event::Parted { channel, nick } => {
                self.remove_nick(channel, nick);
            }
            Event::Kicked { channel, nick, .. } => {
                self.remove_nick(channel, nick);
            }
            Event::UserQuit { nick, .. } => {
                // QUIT isn't scoped to a channel — drop the nick everywhere.
                let channels: Vec<String> = self.nick_index.keys().cloned().collect();
                for ch in channels {
                    self.remove_nick(&ch, nick);
                }
            }
            Event::NickChanged { old_nick, new_nick } => {
                // The DID set is unchanged; only re-key the nick index. Do it in
                // every channel the old nick appears in.
                for idx in self.nick_index.values_mut() {
                    if let Some(did) = idx.remove(old_nick) {
                        idx.insert(new_nick.clone(), did);
                    }
                }
            }
            _ => {}
        }
    }

    /// Remove a member by nick from one channel (resolving nick → DID via the
    /// internal index).
    fn remove_nick(&mut self, channel: &str, nick: &str) {
        let ch = Self::key(channel);
        if let Some(idx) = self.nick_index.get_mut(&ch) {
            if let Some(did) = idx.remove(nick) {
                if let Some(set) = self.by_channel.get_mut(&ch) {
                    set.remove(&did);
                    if set.is_empty() {
                        self.by_channel.remove(&ch);
                    }
                }
            }
            if idx.is_empty() {
                self.nick_index.remove(&ch);
            }
        }
    }

    /// The member DIDs of a channel (sorted + deduped, ready for
    /// [`GroupKey::derive`](crate::e2ee_did::GroupKey::derive)). Empty if the
    /// channel is untracked.
    pub fn member_dids(&self, channel: &str) -> Vec<String> {
        let mut dids: Vec<String> = self
            .by_channel
            .get(&Self::key(channel))
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        dids.sort();
        dids.dedup();
        dids
    }

    /// Whether a DID is currently a tracked member of the channel.
    pub fn contains(&self, channel: &str, did: &str) -> bool {
        self.by_channel
            .get(&Self::key(channel))
            .is_some_and(|s| s.contains(did))
    }

    /// Forget a channel entirely (e.g. when the local user parts it).
    pub fn clear_channel(&mut self, channel: &str) {
        let ch = Self::key(channel);
        self.by_channel.remove(&ch);
        self.nick_index.remove(&ch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(channel: &str, nick: &str, account: Option<&str>) -> Event {
        Event::Joined {
            channel: channel.to_string(),
            nick: nick.to_string(),
            account: account.map(|s| s.to_string()),
        }
    }

    #[test]
    fn join_accumulates_dids() {
        let mut m = Membership::new();
        m.apply(&joined("#chan", "alice", Some("did:plc:alice")));
        m.apply(&joined("#chan", "bob", Some("did:plc:bob")));
        assert_eq!(
            m.member_dids("#chan"),
            vec!["did:plc:alice".to_string(), "did:plc:bob".to_string()]
        );
    }

    #[test]
    fn guest_join_without_did_is_ignored() {
        let mut m = Membership::new();
        m.apply(&joined("#chan", "guest1", None));
        assert!(m.member_dids("#chan").is_empty());
    }

    #[test]
    fn channel_match_is_case_insensitive() {
        let mut m = Membership::new();
        m.apply(&joined("#Chan", "alice", Some("did:plc:alice")));
        assert!(m.contains("#chan", "did:plc:alice"));
        assert_eq!(m.member_dids("#CHAN").len(), 1);
    }

    #[test]
    fn part_removes_the_dids_member() {
        let mut m = Membership::new();
        m.apply(&joined("#chan", "alice", Some("did:plc:alice")));
        m.apply(&joined("#chan", "bob", Some("did:plc:bob")));
        m.apply(&Event::Parted {
            channel: "#chan".into(),
            nick: "alice".into(),
        });
        assert_eq!(m.member_dids("#chan"), vec!["did:plc:bob".to_string()]);
    }

    #[test]
    fn kick_removes_member() {
        let mut m = Membership::new();
        m.apply(&joined("#chan", "alice", Some("did:plc:alice")));
        m.apply(&Event::Kicked {
            channel: "#chan".into(),
            nick: "alice".into(),
            by: "op".into(),
            reason: "bye".into(),
        });
        assert!(m.member_dids("#chan").is_empty());
    }

    #[test]
    fn quit_removes_member_from_all_channels() {
        let mut m = Membership::new();
        m.apply(&joined("#a", "alice", Some("did:plc:alice")));
        m.apply(&joined("#b", "alice", Some("did:plc:alice")));
        m.apply(&Event::UserQuit {
            nick: "alice".into(),
            reason: "gone".into(),
        });
        assert!(m.member_dids("#a").is_empty());
        assert!(m.member_dids("#b").is_empty());
    }

    #[test]
    fn nick_change_is_a_noop_for_did_set() {
        let mut m = Membership::new();
        m.apply(&joined("#chan", "alice", Some("did:plc:alice")));
        m.apply(&Event::NickChanged {
            old_nick: "alice".into(),
            new_nick: "alice2".into(),
        });
        // DID set unchanged...
        assert_eq!(m.member_dids("#chan"), vec!["did:plc:alice".to_string()]);
        // ...and a subsequent PART under the NEW nick still resolves to the DID.
        m.apply(&Event::Parted {
            channel: "#chan".into(),
            nick: "alice2".into(),
        });
        assert!(m.member_dids("#chan").is_empty());
    }

    #[test]
    fn member_dids_is_sorted_and_deduped() {
        let mut m = Membership::new();
        m.apply(&joined("#chan", "charlie", Some("did:plc:charlie")));
        m.apply(&joined("#chan", "alice", Some("did:plc:alice")));
        // Same DID re-joining under a different session/nick doesn't duplicate.
        m.apply(&joined("#chan", "alice_phone", Some("did:plc:alice")));
        assert_eq!(
            m.member_dids("#chan"),
            vec!["did:plc:alice".to_string(), "did:plc:charlie".to_string()]
        );
    }

    #[test]
    fn clear_channel_forgets_everything() {
        let mut m = Membership::new();
        m.apply(&joined("#chan", "alice", Some("did:plc:alice")));
        m.clear_channel("#chan");
        assert!(m.member_dids("#chan").is_empty());
    }

    #[test]
    fn unrelated_events_are_ignored() {
        let mut m = Membership::new();
        m.apply(&Event::Message {
            from: "alice".into(),
            target: "#chan".into(),
            text: "hi".into(),
            tags: HashMap::new(),
        });
        assert!(m.member_dids("#chan").is_empty());
    }
}
