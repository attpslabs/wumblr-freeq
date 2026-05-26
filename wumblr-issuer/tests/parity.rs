//! Cross-crate parity test: a credential we sign here must verify under
//! freeq-server's `policy::credentials::verify_credential_signature` byte-for-byte.
//!
//! If this test ever fails, our `VerifiableCredential` struct or JCS
//! canonicalization has drifted from freeq-server's — every cred we issue
//! will be rejected by freeq.

use ed25519_dalek::SigningKey;
use serde_json::json;

#[test]
fn issued_credential_verifies_under_freeq_server() {
    // Deterministic key — never use a fixed seed in prod.
    let seed = [42u8; 32];
    let signing_key = SigningKey::from_bytes(&seed);
    let pubkey_bytes = signing_key.verifying_key().to_bytes();

    let our_cred = wumblr_issuer::credentials::sign(
        "did:web:wumblr.com:verify",
        "did:plc:abc123example",
        "wumblr_member:wumblr",
        json!({ "community": "wumblr" }),
        3600,
        &signing_key,
    )
    .expect("issuer signs credential");

    // Re-serialize through freeq-server's VerifiableCredential type to make sure
    // the JSON round-trips into the same shape, then verify the signature.
    let json_bytes = serde_json::to_vec(&our_cred).unwrap();
    let freeq_cred: freeq_server::policy::types::VerifiableCredential =
        serde_json::from_slice(&json_bytes).expect("freeq-server parses our credential");

    // Sanity: round-tripping preserved fields.
    assert_eq!(freeq_cred.credential_type_tag, "FreeqCredential/v1");
    assert_eq!(freeq_cred.issuer, "did:web:wumblr.com:verify");
    assert_eq!(freeq_cred.subject, "did:plc:abc123example");
    assert_eq!(freeq_cred.credential_type, "wumblr_member:wumblr");
    assert!(!freeq_cred.signature.is_empty());

    let verified = freeq_server::policy::credentials::verify_credential_signature(
        &freeq_cred,
        &pubkey_bytes,
    )
    .expect("verify call returns Ok");

    assert!(verified, "freeq-server must verify our signature");
}
