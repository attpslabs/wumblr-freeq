/**
 * Minimal AT-URI parser/builder for wumblr's `com.wumblr.*` records.
 *
 * Format: at://<authority>/<collection>/<rkey>
 * Authority is always a DID for our records (we never use handle-based
 * AT-URIs since handles can change).
 */

export interface AtUri {
	authority: string; // did:plc:... or did:web:...
	collection: string; // com.wumblr.community, etc.
	rkey: string;
}

const AT_URI_RE =
	/^at:\/\/(did:[a-z]+:[a-zA-Z0-9._:%-]+)\/([a-zA-Z0-9.-]+)\/([a-zA-Z0-9._:~-]+)$/;

export class InvalidAtUriError extends Error {
	constructor(uri: string) {
		super(`Invalid AT-URI: ${JSON.stringify(uri)}`);
		this.name = "InvalidAtUriError";
	}
}

export function parseAtUri(uri: string): AtUri {
	const m = AT_URI_RE.exec(uri);
	if (!m) throw new InvalidAtUriError(uri);
	return {
		authority: m[1]!,
		collection: m[2]!,
		rkey: m[3]!,
	};
}

export function tryParseAtUri(uri: string): AtUri | null {
	const m = AT_URI_RE.exec(uri);
	if (!m) return null;
	return {
		authority: m[1]!,
		collection: m[2]!,
		rkey: m[3]!,
	};
}

export function buildAtUri(
	authority: string,
	collection: string,
	rkey: string,
): string {
	return `at://${authority}/${collection}/${rkey}`;
}

/**
 * The community's "stable identifier" within wumblr is the AT-URI of its
 * com.wumblr.community record. We also expose the rkey alone since that
 * is what gets embedded in IRC channel names and freeq credentials.
 */
export function communityRkeyFromUri(uri: string): string {
	return parseAtUri(uri).rkey;
}

export function communityOwnerFromUri(uri: string): string {
	return parseAtUri(uri).authority;
}
