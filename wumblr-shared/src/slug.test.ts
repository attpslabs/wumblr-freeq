import assert from "node:assert/strict";
import { test } from "node:test";

import {
	isValidChannelName,
	isValidCommunitySlug,
} from "./slug.ts";

test("isValidCommunitySlug", () => {
	assert.equal(isValidCommunitySlug("solarpunk"), true);
	assert.equal(isValidCommunitySlug("a-b-c"), true);
	assert.equal(isValidCommunitySlug("wumblr"), true);
	assert.equal(isValidCommunitySlug("123-abc"), true);
	assert.equal(isValidCommunitySlug("abc"), true); // min length 3
	assert.equal(isValidCommunitySlug("a".repeat(21)), true); // max length 21

	assert.equal(isValidCommunitySlug("ab"), false); // too short
	assert.equal(isValidCommunitySlug("a".repeat(22)), false); // too long
	assert.equal(isValidCommunitySlug("Solarpunk"), false); // uppercase
	assert.equal(isValidCommunitySlug("solar_punk"), false); // underscore
	assert.equal(isValidCommunitySlug("solar punk"), false); // space
	assert.equal(isValidCommunitySlug(""), false);
});

test("isValidChannelName", () => {
	assert.equal(isValidChannelName("general"), true);
	assert.equal(isValidChannelName("off-topic"), true);
	assert.equal(isValidChannelName("a"), true); // min length 1
	assert.equal(isValidChannelName("a".repeat(32)), true); // max length 32

	assert.equal(isValidChannelName(""), false);
	assert.equal(isValidChannelName("a".repeat(33)), false);
	assert.equal(isValidChannelName("General"), false);
	assert.equal(isValidChannelName("🌱-garden"), false);
});
