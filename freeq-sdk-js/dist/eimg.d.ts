/**
 * Ephemeral encrypted image (eimg) support for the freeq web/RN client.
 *
 * Images are encrypted client-side with the channel's ENC2 group key before
 * upload, so the freeq server (a blind broker) and the backing spaces service
 * only ever hold ciphertext. Images are hard-deleted 24h after upload.
 *
 * This module MUST stay byte-compatible with the Rust SDK
 * (`freeq-sdk/src/e2ee_did.rs` `GroupKey` + `freeq-sdk/src/eimg.rs`):
 * `deriveGroupKey` reproduces `GroupKey::derive`, and `encryptBytes` produces
 * the same nonce-prepended layout as `GroupKey::encrypt_bytes`. The
 * cross-implementation parity vector is asserted in `eimg.test.ts`.
 *
 * Key agreement (Phase A): `epoch` is fixed at 0 (no rotation yet — that comes
 * with OpenMLS later). Decryption only succeeds when sender and recipient derive
 * the key from the *same* member DID set; see the caveat in the Rust
 * `membership.rs` docs (the NAMES roster carries nicks only).
 */
/** Derive the channel's ENC2 group AES-256-GCM key from member DIDs + epoch.
 *
 * Mirrors `GroupKey::derive` exactly:
 *   sorted = dedup(sort(members))
 *   ikm    = concat(utf8(did) for did in sorted)        // no separator
 *   salt   = SHA-256(utf8(channel.toLowerCase()))
 *   key    = HKDF-SHA256(ikm, salt, "freeq-e2ee-v2-{epoch}", 32 bytes)
 *
 * Returns the raw 32-byte key.
 */
export declare function deriveGroupKey(channel: string, members: string[], epoch?: number): Promise<Uint8Array>;
/** Encrypt raw bytes → `nonce(12) ++ AES-256-GCM ciphertext+tag` (raw binary,
 *  no base64/text envelope). Mirrors `GroupKey::encrypt_bytes`. */
export declare function encryptBytes(key: Uint8Array, plaintext: Uint8Array): Promise<Uint8Array>;
/** Decrypt a nonce-prepended blob produced by `encryptBytes` (or the Rust
 *  `encrypt_bytes`). Throws on auth failure / truncation. */
export declare function decryptBytes(key: Uint8Array, blob: Uint8Array): Promise<Uint8Array>;
export interface EimgUploadResult {
    imageId: string;
    /** Unix seconds at which the image 410s. */
    expiresAt: number;
}
/** A fetch outcome: decrypted bytes, or `gone` (expired/deleted, HTTP 410/404). */
export type EimgFetchResult = {
    found: Uint8Array;
} | {
    gone: true;
};
/**
 * Encrypt `imageBytes` with the channel's group key and upload the ciphertext.
 *
 * Auth: relies on the active WebSocket session for `did` on the server (the
 * web client is logged in), so no upload token is sent. `contentType` is the
 * image MIME type. Returns the opaque `imageId` + `expiresAt`.
 */
export declare function uploadEncryptedImage(origin: string, did: string, channel: string, members: string[], contentType: string, imageBytes: Uint8Array, epoch?: number): Promise<EimgUploadResult>;
/**
 * Fetch and decrypt an encrypted image. Returns `{ gone: true }` if the image
 * has expired (HTTP 410) or is absent (404).
 */
export declare function fetchEncryptedImage(origin: string, imageId: string, did: string, channel: string, members: string[], epoch?: number): Promise<EimgFetchResult>;
//# sourceMappingURL=eimg.d.ts.map