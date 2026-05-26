//! freeq-auth-broker client (stubbed in M1 step 4).
//!
//! When wired up in M2, this module will:
//!  - POST OAuth-session blobs to `<broker>/sessions`, receiving an opaque ID.
//!  - GET `<broker>/whoami` with a bearer to resolve a session → `did + handle + profile`.
//!  - POST DPoP-proxied PDS writes via `<broker>/xrpc/com.atproto.repo.createRecord`.
//!
//! For step 4 we ship a `Mock` impl that records what *would* have been sent
//! and returns synthesized DIDs. This unblocks the M1 ship-gate
//! ("log in with Bluesky → see DID on Home") without needing a running broker.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhoAmI {
    pub did: String,
    pub handle: Option<String>,
}

/// Capability the backend needs from the broker. Real HTTP impl lands in M2.
pub trait BrokerClient: Send + Sync {
    /// Resolve a session bearer → DID/handle. Returns `None` when unknown.
    fn whoami(&self, session_id: &str) -> Result<Option<WhoAmI>>;

    /// Store an OAuth-session blob (as serialized by `@atproto/oauth-client-expo`)
    /// and return an opaque session_id the client can present on subsequent requests.
    fn register_session(&self, did: &str, session_blob: Value) -> Result<String>;
}

/// In-memory mock used in M1. Real `HttpBroker` impl lands in M2.
///
/// Behavior:
///  - `register_session` records the session blob and returns
///    `wumblr-session-<rand>`. The DID is taken from the call's `did` arg,
///    which the RN client extracts from the `OAuthSession.did` returned by
///    `@atproto/oauth-client-expo.signIn()`.
///  - `whoami` looks up the registered session and returns the DID.
///  - All state is process-local — restarting wumblr-backend invalidates
///    every session. Fine for dev; real broker has SQLite persistence.
#[derive(Default)]
pub struct MockBroker {
    inner: Arc<Mutex<MockState>>,
}

#[derive(Default)]
struct MockState {
    sessions: std::collections::HashMap<String, MockSession>,
    next_id: u64,
}

#[derive(Clone)]
struct MockSession {
    did: String,
    #[allow(dead_code)] // we'll inspect this in tests once they land
    blob: Value,
}

impl MockBroker {
    pub fn new() -> Self {
        Self::default()
    }
}

impl BrokerClient for MockBroker {
    fn whoami(&self, session_id: &str) -> Result<Option<WhoAmI>> {
        let inner = self.inner.lock().unwrap();
        Ok(inner.sessions.get(session_id).map(|s| WhoAmI {
            did: s.did.clone(),
            handle: None,
        }))
    }

    fn register_session(&self, did: &str, session_blob: Value) -> Result<String> {
        let mut inner = self.inner.lock().unwrap();
        inner.next_id += 1;
        let session_id = format!("wumblr-session-{}", inner.next_id);
        inner.sessions.insert(
            session_id.clone(),
            MockSession {
                did: did.to_string(),
                blob: session_blob,
            },
        );
        Ok(session_id)
    }
}
