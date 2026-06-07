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
export async function deriveGroupKey(
  channel: string,
  members: string[],
  epoch = 0,
): Promise<Uint8Array> {
  // Sort + dedup, matching Rust's Vec::sort + Vec::dedup (dedup removes only
  // *consecutive* duplicates, which after a sort means all duplicates).
  const sorted = [...members].sort();
  const deduped: string[] = [];
  for (const d of sorted) {
    if (deduped.length === 0 || deduped[deduped.length - 1] !== d) deduped.push(d);
  }

  const enc = new TextEncoder();
  // IKM: raw concatenation of the UTF-8 DID bytes.
  const ikmParts = deduped.map((d) => enc.encode(d));
  const ikmLen = ikmParts.reduce((n, p) => n + p.length, 0);
  const ikm = new Uint8Array(ikmLen);
  let off = 0;
  for (const p of ikmParts) {
    ikm.set(p, off);
    off += p.length;
  }

  const salt = new Uint8Array(
    await crypto.subtle.digest('SHA-256', enc.encode(channel.toLowerCase())),
  );
  const info = enc.encode(`freeq-e2ee-v2-${epoch}`);

  const baseKey = await (crypto.subtle.importKey as any)('raw', ikm, 'HKDF', false, ['deriveBits']);
  const bits = await (crypto.subtle as any).deriveBits(
    { name: 'HKDF', hash: 'SHA-256', salt, info },
    baseKey,
    256,
  );
  return new Uint8Array(bits);
}

/** Encrypt raw bytes → `nonce(12) ++ AES-256-GCM ciphertext+tag` (raw binary,
 *  no base64/text envelope). Mirrors `GroupKey::encrypt_bytes`. */
export async function encryptBytes(key: Uint8Array, plaintext: Uint8Array): Promise<Uint8Array> {
  const iv = crypto.getRandomValues(new Uint8Array(12));
  // `as any` on the crypto calls mirrors e2ee.ts — it sidesteps the strict
  // BufferSource typing for Uint8Array-backed views under TS 5.7+.
  const cryptoKey = await (crypto.subtle.importKey as any)('raw', key, { name: 'AES-GCM' }, false, [
    'encrypt',
  ]);
  const ct = new Uint8Array(
    await (crypto.subtle.encrypt as any)({ name: 'AES-GCM', iv }, cryptoKey, plaintext),
  );
  const blob = new Uint8Array(12 + ct.length);
  blob.set(iv, 0);
  blob.set(ct, 12);
  return blob;
}

/** Decrypt a nonce-prepended blob produced by `encryptBytes` (or the Rust
 *  `encrypt_bytes`). Throws on auth failure / truncation. */
export async function decryptBytes(key: Uint8Array, blob: Uint8Array): Promise<Uint8Array> {
  if (blob.length < 12) throw new Error('eimg: blob too short to contain a nonce');
  const iv = blob.subarray(0, 12);
  const ct = blob.subarray(12);
  const cryptoKey = await (crypto.subtle.importKey as any)('raw', key, { name: 'AES-GCM' }, false, [
    'decrypt',
  ]);
  return new Uint8Array(await (crypto.subtle.decrypt as any)({ name: 'AES-GCM', iv }, cryptoKey, ct));
}

export interface EimgUploadResult {
  imageId: string;
  /** Unix seconds at which the image 410s. */
  expiresAt: number;
}

/** A fetch outcome: decrypted bytes, or `gone` (expired/deleted, HTTP 410/404). */
export type EimgFetchResult = { found: Uint8Array } | { gone: true };

function eimgBase(origin: string): string {
  return `${origin.replace(/\/+$/, '')}/api/v1/eimg`;
}

/**
 * Encrypt `imageBytes` with the channel's group key and upload the ciphertext.
 *
 * Auth: relies on the active WebSocket session for `did` on the server (the
 * web client is logged in), so no upload token is sent. `contentType` is the
 * image MIME type. Returns the opaque `imageId` + `expiresAt`.
 */
export async function uploadEncryptedImage(
  origin: string,
  did: string,
  channel: string,
  members: string[],
  contentType: string,
  imageBytes: Uint8Array,
  epoch = 0,
): Promise<EimgUploadResult> {
  const key = await deriveGroupKey(channel, members, epoch);
  const ciphertext = await encryptBytes(key, imageBytes);

  const form = new FormData();
  form.append('file', new Blob([ciphertext as any], { type: contentType }), 'eimg');
  form.append('did', did);
  form.append('channel', channel);

  const resp = await fetch(eimgBase(origin), {
    method: 'POST',
    body: form,
    credentials: 'include',
  });
  if (!resp.ok) {
    throw new Error(`eimg upload failed: ${resp.status} ${await resp.text().catch(() => '')}`);
  }
  const json = (await resp.json()) as { image_id?: string; expires_at?: number };
  if (!json.image_id || typeof json.expires_at !== 'number') {
    throw new Error('eimg upload: malformed response');
  }
  return { imageId: json.image_id, expiresAt: json.expires_at };
}

/**
 * Fetch and decrypt an encrypted image. Returns `{ gone: true }` if the image
 * has expired (HTTP 410) or is absent (404).
 */
export async function fetchEncryptedImage(
  origin: string,
  imageId: string,
  did: string,
  channel: string,
  members: string[],
  epoch = 0,
): Promise<EimgFetchResult> {
  const url = `${eimgBase(origin)}/${encodeURIComponent(imageId)}?did=${encodeURIComponent(did)}`;
  const resp = await fetch(url, { credentials: 'include' });
  if (resp.status === 410 || resp.status === 404) return { gone: true };
  if (!resp.ok) {
    throw new Error(`eimg fetch failed: ${resp.status} ${await resp.text().catch(() => '')}`);
  }
  const ciphertext = new Uint8Array(await resp.arrayBuffer());
  const key = await deriveGroupKey(channel, members, epoch);
  const plaintext = await decryptBytes(key, ciphertext);
  return { found: plaintext };
}
