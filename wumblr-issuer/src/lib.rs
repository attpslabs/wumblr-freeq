//! wumblr-issuer library surface.
//!
//! The actual service is in `main.rs`; this module exposes the signing
//! primitives so integration tests (and any future direct consumers) can
//! exercise credential issuance without spinning up the HTTP server.

pub mod credentials;
pub mod did_doc;
pub mod epds;
pub mod keys;
pub mod sessions;
