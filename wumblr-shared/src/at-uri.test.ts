import assert from "node:assert/strict";
import { test } from "node:test";

import {
	buildAtUri,
	communityOwnerFromUri,
	communityRkeyFromUri,
	InvalidAtUriError,
	parseAtUri,
	tryParseAtUri,
} from "./at-uri.ts";

test("parseAtUri: valid community URI", () => {
	const uri = "at://did:plc:abc234567xyz/com.wumblr.community/solarpunk";
	const parsed = parseAtUri(uri);
	assert.equal(parsed.authority, "did:plc:abc234567xyz");
	assert.equal(parsed.collection, "com.wumblr.community");
	assert.equal(parsed.rkey, "solarpunk");
});

test("parseAtUri: did:web supported", () => {
	const uri = "at://did:web:wumblr.com/com.wumblr.community/wumblr";
	const parsed = parseAtUri(uri);
	assert.equal(parsed.authority, "did:web:wumblr.com");
	assert.equal(parsed.rkey, "wumblr");
});

test("parseAtUri: throws on invalid", () => {
	assert.throws(() => parseAtUri("not-an-aturi"), InvalidAtUriError);
	assert.throws(() => parseAtUri("https://wumblr.com"), InvalidAtUriError);
	assert.throws(() => parseAtUri("at://handle.bsky.social/foo/bar"), InvalidAtUriError);
});

test("tryParseAtUri: returns null on invalid", () => {
	assert.equal(tryParseAtUri("nope"), null);
});

test("buildAtUri: composes correctly", () => {
	assert.equal(
		buildAtUri("did:plc:abc", "com.wumblr.community", "solarpunk"),
		"at://did:plc:abc/com.wumblr.community/solarpunk",
	);
});

test("buildAtUri round-trips with parseAtUri", () => {
	const built = buildAtUri("did:plc:xyz", "com.wumblr.community.member", "wumblr");
	const parsed = parseAtUri(built);
	assert.equal(parsed.authority, "did:plc:xyz");
	assert.equal(parsed.collection, "com.wumblr.community.member");
	assert.equal(parsed.rkey, "wumblr");
});

test("communityRkeyFromUri / communityOwnerFromUri", () => {
	const uri = "at://did:plc:alice/com.wumblr.community/solarpunk";
	assert.equal(communityRkeyFromUri(uri), "solarpunk");
	assert.equal(communityOwnerFromUri(uri), "did:plc:alice");
});
