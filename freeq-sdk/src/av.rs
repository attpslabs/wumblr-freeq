//! AV call signaling — the IRC layer for freeq voice/video calls.
//!
//! A freeq AV call is coordinated over IRC: a participant sends an
//! `+freeq.at/av-start`, `av-join`, or `av-leave` TAGMSG, and the server
//! echoes `+freeq.at/av-state` TAGMSGs (carrying the session id) to the
//! channel. The actual media rides a separate MoQ transport — see the
//! `freeq-av` crate. This module is only the signaling.
//!
//! [`ClientHandle`](crate::client::ClientHandle) exposes `av_start`,
//! `av_join`, and `av_leave`; apply [`parse_av_state`] to incoming
//! [`Event::TagMsg`](crate::event::Event::TagMsg) tags to track a
//! channel's call state.

use std::collections::HashMap;

/// Generate a per-call instance id — 8 lowercase hex chars. Two devices
/// or agents signed in as the same DID get distinct instance ids so
/// their MoQ broadcast paths (`<session>/<nick>~<instance>`) don't
/// collide.
pub fn new_av_instance() -> String {
    format!("{:08x}", rand::random::<u32>())
}

/// Tags for an `+freeq.at/av-start` TAGMSG — open a new call.
pub fn av_start_tags(instance: &str, title: Option<&str>) -> HashMap<String, String> {
    let mut t = HashMap::new();
    t.insert("+freeq.at/av-start".into(), String::new());
    t.insert("+freeq.at/av-instance".into(), instance.to_string());
    if let Some(title) = title {
        t.insert("+freeq.at/av-title".into(), title.to_string());
    }
    t
}

/// Tags for an `+freeq.at/av-join` TAGMSG — join an existing call.
pub fn av_join_tags(session_id: &str, instance: &str) -> HashMap<String, String> {
    let mut t = HashMap::new();
    t.insert("+freeq.at/av-join".into(), String::new());
    t.insert("+freeq.at/av-id".into(), session_id.to_string());
    t.insert("+freeq.at/av-instance".into(), instance.to_string());
    t
}

/// Tags for an `+freeq.at/av-leave` TAGMSG — leave a call.
pub fn av_leave_tags(session_id: &str, instance: &str) -> HashMap<String, String> {
    let mut t = HashMap::new();
    t.insert("+freeq.at/av-leave".into(), String::new());
    t.insert("+freeq.at/av-id".into(), session_id.to_string());
    t.insert("+freeq.at/av-instance".into(), instance.to_string());
    t
}

/// What an `+freeq.at/av-state` TAGMSG reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvAction {
    /// A call was started in the channel.
    Started,
    /// A participant joined the call.
    Joined,
    /// A participant left.
    Left,
    /// The call ended (no participants remain).
    Ended,
}

/// A parsed `+freeq.at/av-state` TAGMSG — the server's broadcast of a
/// change to a channel's AV call.
#[derive(Debug, Clone)]
pub struct AvState {
    /// What happened.
    pub action: AvAction,
    /// The AV session id (the MoQ broadcast-path prefix).
    pub session_id: String,
    /// The nick whose action this was, when the server included it.
    pub actor: Option<String>,
    /// Active participant count after the change, when included.
    pub participants: Option<u32>,
    /// Call title, when included.
    pub title: Option<String>,
}

/// Parse the tags of an incoming TAGMSG as an [`AvState`]. Returns
/// `None` for any TAGMSG that isn't an `+freeq.at/av-state` broadcast,
/// so callers can apply it to every [`Event::TagMsg`](crate::event::Event::TagMsg)
/// unconditionally.
pub fn parse_av_state(tags: &HashMap<String, String>) -> Option<AvState> {
    let action = match tags.get("+freeq.at/av-state")?.as_str() {
        "started" => AvAction::Started,
        "joined" => AvAction::Joined,
        "left" => AvAction::Left,
        "ended" => AvAction::Ended,
        _ => return None,
    };
    Some(AvState {
        action,
        session_id: tags.get("+freeq.at/av-id")?.clone(),
        actor: tags.get("+freeq.at/av-actor").cloned(),
        participants: tags
            .get("+freeq.at/av-participants")
            .and_then(|p| p.parse().ok()),
        title: tags
            .get("+freeq.at/av-title")
            .filter(|t| !t.is_empty())
            .cloned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_is_8_hex() {
        let id = new_av_instance();
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn start_join_leave_tags() {
        let s = av_start_tags("abcd1234", Some("standup"));
        assert!(s.contains_key("+freeq.at/av-start"));
        assert_eq!(s["+freeq.at/av-instance"], "abcd1234");
        assert_eq!(s["+freeq.at/av-title"], "standup");

        let j = av_join_tags("01SESSION", "abcd1234");
        assert!(j.contains_key("+freeq.at/av-join"));
        assert_eq!(j["+freeq.at/av-id"], "01SESSION");
        assert_eq!(j["+freeq.at/av-instance"], "abcd1234");

        let l = av_leave_tags("01SESSION", "abcd1234");
        assert!(l.contains_key("+freeq.at/av-leave"));
        assert_eq!(l["+freeq.at/av-id"], "01SESSION");
    }

    #[test]
    fn parse_state_round_trip() {
        let mut tags = HashMap::new();
        tags.insert("+freeq.at/av-state".into(), "joined".into());
        tags.insert("+freeq.at/av-id".into(), "01SESSION".into());
        tags.insert("+freeq.at/av-actor".into(), "alice".into());
        tags.insert("+freeq.at/av-participants".into(), "3".into());
        let st = parse_av_state(&tags).expect("should parse");
        assert_eq!(st.action, AvAction::Joined);
        assert_eq!(st.session_id, "01SESSION");
        assert_eq!(st.actor.as_deref(), Some("alice"));
        assert_eq!(st.participants, Some(3));

        // A non-AV TAGMSG → None.
        let mut other = HashMap::new();
        other.insert("+typing".into(), "active".into());
        assert!(parse_av_state(&other).is_none());
    }
}
