import assert from "node:assert/strict";
import { test } from "node:test";

import { didTail, grantRkey, isPlcDid, isValidDid } from "./did.ts";

test("isValidDid", () => {
	assert.equal(isValidDid("did:plc:abcdefghijklmnopqrstuv2x"), true);
	assert.equal(isValidDid("did:web:wumblr.com"), true);
	assert.equal(isValidDid("did:web:wumblr.com:verify"), true);
	assert.equal(isValidDid("alice.bsky.social"), false);
	assert.equal(isValidDid(""), false);
});

test("isPlcDid", () => {
	assert.equal(isPlcDid("did:plc:abcdefghijklmnopqrstuv2x"), true);
	assert.equal(isPlcDid("did:web:wumblr.com"), false);
});

test("didTail: default length 6", () => {
	const tail = didTail("did:plc:abcdefghijklmnopqrstuv2x");
	assert.equal(tail, "stuv2x");
	assert.equal(tail.length, 6);
});

test("didTail: custom length", () => {
	assert.equal(didTail("did:plc:abcdefghijklmnopqrstuv2x", 4), "uv2x");
	assert.equal(didTail("did:plc:abcdefghijklmnopqrstuv2x", 8), "qrstuv2x");
});

test("grantRkey: deterministic", () => {
	const rkey = grantRkey("solarpunk", "did:plc:abcdefghijklmnopqrstuv2x");
	assert.match(rkey, /^solarpunk-[a-z0-9]{6}$/);
	// Same inputs → same output
	const rkey2 = grantRkey("solarpunk", "did:plc:abcdefghijklmnopqrstuv2x");
	assert.equal(rkey, rkey2);
});
