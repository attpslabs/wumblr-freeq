export const COMMUNITY_SLUG_RE = /^[a-z0-9-]{3,21}$/;
export const CHANNEL_NAME_RE = /^[a-z0-9-]{1,32}$/;

export function isValidCommunitySlug(s: string): boolean {
	return COMMUNITY_SLUG_RE.test(s);
}

export function isValidChannelName(s: string): boolean {
	return CHANNEL_NAME_RE.test(s);
}

export class InvalidSlugError extends Error {
	constructor(slug: string) {
		super(
			`Invalid community slug ${JSON.stringify(slug)}: must match ${COMMUNITY_SLUG_RE}`,
		);
		this.name = "InvalidSlugError";
	}
}

export class InvalidChannelNameError extends Error {
	constructor(name: string) {
		super(
			`Invalid channel name ${JSON.stringify(name)}: must match ${CHANNEL_NAME_RE}`,
		);
		this.name = "InvalidChannelNameError";
	}
}

export function assertCommunitySlug(s: string): void {
	if (!isValidCommunitySlug(s)) throw new InvalidSlugError(s);
}

export function assertChannelName(s: string): void {
	if (!isValidChannelName(s)) throw new InvalidChannelNameError(s);
}
