import {
	assertChannelName,
	assertCommunitySlug,
	isValidChannelName,
	isValidCommunitySlug,
} from "./slug.ts";

/**
 * Map between display channel name and IRC transport channel name.
 *
 * Display:   `general`           (what users see in the UI)
 * Transport: `#solarpunk-general` (what wumblr-freeq actually uses)
 *
 * The community-rkey prefix isolates channels across communities and is
 * what wumblr-freeq's policy machinery keys off of. Single source of
 * truth — never construct transport channel names ad-hoc elsewhere.
 */

const TRANSPORT_PREFIX = "#";

export function toTransportChannel(
	communitySlug: string,
	channelName: string,
): string {
	assertCommunitySlug(communitySlug);
	assertChannelName(channelName);
	return `${TRANSPORT_PREFIX}${communitySlug}-${channelName}`;
}

export interface ParsedTransportChannel {
	communitySlug: string;
	channelName: string;
}

export function parseTransportChannel(
	transport: string,
): ParsedTransportChannel | null {
	if (!transport.startsWith(TRANSPORT_PREFIX)) return null;
	const rest = transport.slice(TRANSPORT_PREFIX.length);
	// Split on first '-': community-slug is everything up to the first '-'
	// that yields a valid slug + valid channel name. The slug regex allows
	// internal dashes too, so we must try every split position and pick
	// the first valid pair (left-to-right).
	for (let i = 1; i < rest.length; i++) {
		if (rest[i] !== "-") continue;
		const slug = rest.slice(0, i);
		const name = rest.slice(i + 1);
		if (isValidCommunitySlug(slug) && isValidChannelName(name)) {
			return { communitySlug: slug, channelName: name };
		}
	}
	return null;
}
