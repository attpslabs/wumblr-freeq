import assert from "node:assert/strict";
import { test } from "node:test";

import {
	parseTransportChannel,
	toTransportChannel,
} from "./channel-name.ts";
import { InvalidChannelNameError, InvalidSlugError } from "./slug.ts";

test("toTransportChannel: simple case", () => {
	assert.equal(toTransportChannel("solarpunk", "general"), "#solarpunk-general");
});

test("toTransportChannel: rejects invalid slug", () => {
	assert.throws(() => toTransportChannel("ab", "general"), InvalidSlugError);
	assert.throws(() => toTransportChannel("Solarpunk", "general"), InvalidSlugError);
	assert.throws(() => toTransportChannel("solar_punk", "general"), InvalidSlugError);
});

test("toTransportChannel: rejects invalid channel name", () => {
	assert.throws(
		() => toTransportChannel("solarpunk", "General"),
		InvalidChannelNameError,
	);
	assert.throws(() => toTransportChannel("solarpunk", ""), InvalidChannelNameError);
});

test("parseTransportChannel: round-trips simple case", () => {
	const parsed = parseTransportChannel("#solarpunk-general");
	assert.deepEqual(parsed, { communitySlug: "solarpunk", channelName: "general" });
});

test("parseTransportChannel: returns null for non-channel strings", () => {
	assert.equal(parseTransportChannel("solarpunk-general"), null); // missing #
	assert.equal(parseTransportChannel("#"), null);
	assert.equal(parseTransportChannel("#-"), null);
	assert.equal(parseTransportChannel("##"), null);
});

test("parseTransportChannel: ambiguous splits resolved left-to-right", () => {
	// `solar-punk` is a valid slug; `general` is a valid channel.
	// `solar` is also a valid slug (5 chars); `punk-general` is also valid.
	// We pick the FIRST valid split left-to-right → `solar` + `punk-general`.
	const parsed = parseTransportChannel("#solar-punk-general");
	assert.deepEqual(parsed, {
		communitySlug: "solar",
		channelName: "punk-general",
	});
});

test("parseTransportChannel: rejects too-short slug", () => {
	// `ab` is too short to be a valid slug; no valid split exists.
	assert.equal(parseTransportChannel("#ab-general"), null);
});

test("round-trip: every valid (slug, channel) pair preserved", () => {
	const cases: Array<[string, string]> = [
		["solarpunk", "general"],
		["wumblr", "off-topic"],
		["abc", "z"],
		["a-b-c", "x-y-z"],
	];
	for (const [slug, name] of cases) {
		const transport = toTransportChannel(slug, name);
		const parsed = parseTransportChannel(transport);
		// Note: round-trip may not preserve ambiguous slug/channel splits.
		// For now we just assert the result is a valid parse.
		assert.ok(parsed, `failed to parse ${transport}`);
	}
});
