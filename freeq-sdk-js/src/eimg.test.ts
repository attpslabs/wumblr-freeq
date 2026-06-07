/**
 * eimg crypto tests — and the CROSS-IMPLEMENTATION PARITY VECTOR.
 *
 * `EXPECTED_KEY_HEX` must stay identical to `GROUP_KEY_PARITY_HEX` in the Rust
 * SDK (`freeq-sdk/src/e2ee_did.rs`, test `group_key_derivation_vector`). If
 * either side's GroupKey derivation drifts, one of these tests breaks and the
 * two implementations must be re-synced — otherwise cross-client image
 * decryption silently fails.
 */
import { describe, it, expect } from 'vitest';
import { deriveGroupKey, encryptBytes, decryptBytes } from './eimg';

function toHex(b: Uint8Array): string {
  return Array.from(b)
    .map((x) => x.toString(16).padStart(2, '0'))
    .join('');
}

// Must equal Rust GROUP_KEY_PARITY_HEX for derive("#Secret", [bob,alice,bob], 0).
const EXPECTED_KEY_HEX =
  'f3a95c43ef7245faee31bfde76b2e7de50de309c1ee801042ca22c90138900a7';

describe('eimg GroupKey parity', () => {
  it('derives the byte-identical key as the Rust SDK (cross-impl vector)', async () => {
    // Same inputs as the Rust vector: unsorted + duplicate + mixed-case channel,
    // to exercise sort/dedup/lowercase identically on both sides.
    const key = await deriveGroupKey('#Secret', ['did:plc:bob', 'did:plc:alice', 'did:plc:bob'], 0);
    expect(toHex(key)).toBe(EXPECTED_KEY_HEX);
  });

  it('is order- and duplicate-independent (sort+dedup)', async () => {
    const a = await deriveGroupKey('#chan', ['did:plc:b', 'did:plc:a'], 0);
    const b = await deriveGroupKey('#chan', ['did:plc:a', 'did:plc:b', 'did:plc:a'], 0);
    expect(toHex(a)).toBe(toHex(b));
  });

  it('is channel-case-insensitive', async () => {
    const a = await deriveGroupKey('#Chan', ['did:plc:a'], 0);
    const b = await deriveGroupKey('#chan', ['did:plc:a'], 0);
    expect(toHex(a)).toBe(toHex(b));
  });

  it('differs by epoch and by member set', async () => {
    const base = toHex(await deriveGroupKey('#chan', ['did:plc:a'], 0));
    expect(toHex(await deriveGroupKey('#chan', ['did:plc:a'], 1))).not.toBe(base);
    expect(toHex(await deriveGroupKey('#chan', ['did:plc:a', 'did:plc:b'], 0))).not.toBe(base);
  });
});

describe('eimg encryptBytes/decryptBytes', () => {
  it('roundtrips arbitrary bytes', async () => {
    const key = await deriveGroupKey('#secret', ['did:plc:alice', 'did:plc:bob'], 0);
    const img = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0, 1, 2, 255, 254]);
    const blob = await encryptBytes(key, img);
    // nonce(12) ++ ciphertext+tag(16) → strictly larger, not a text envelope.
    expect(blob.length).toBeGreaterThan(12 + img.length);
    const got = await decryptBytes(key, blob);
    expect(Array.from(got)).toEqual(Array.from(img));
  });

  it('fails to decrypt under a different member set', async () => {
    const k1 = await deriveGroupKey('#chan', ['did:plc:a', 'did:plc:b'], 0);
    const k2 = await deriveGroupKey('#chan', ['did:plc:a', 'did:plc:c'], 0);
    const blob = await encryptBytes(k1, new Uint8Array([1, 2, 3]));
    await expect(decryptBytes(k2, blob)).rejects.toBeTruthy();
  });

  it('fails on a tampered blob', async () => {
    const key = await deriveGroupKey('#chan', ['did:plc:a'], 0);
    const blob = await encryptBytes(key, new Uint8Array([1, 2, 3, 4]));
    blob[blob.length - 1] ^= 0xff;
    await expect(decryptBytes(key, blob)).rejects.toBeTruthy();
  });

  it('rejects a truncated blob (< 12 bytes)', async () => {
    const key = await deriveGroupKey('#chan', ['did:plc:a'], 0);
    await expect(decryptBytes(key, new Uint8Array([1, 2, 3]))).rejects.toThrow(/too short/);
  });

  it('roundtrips empty plaintext', async () => {
    const key = await deriveGroupKey('#chan', ['did:plc:a'], 0);
    const blob = await encryptBytes(key, new Uint8Array(0));
    expect(Array.from(await decryptBytes(key, blob))).toEqual([]);
  });
});
