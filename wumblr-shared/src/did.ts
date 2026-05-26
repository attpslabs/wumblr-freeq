const DID_RE = /^did:[a-z]+:[a-zA-Z0-9._:%-]+$/;
const DID_PLC_RE = /^did:plc:[a-z2-7]{24}$/;

export function isValidDid(s: string): boolean {
	return DID_RE.test(s);
}

export function isPlcDid(s: string): boolean {
	return DID_PLC_RE.test(s);
}

/**
 * Last 6 characters of a DID, used as the deterministic suffix in
 * com.wumblr.community.membership.grant rkeys:
 *   `<community-slug>-<did-tail>` e.g. `solarpunk-3jl7m4`.
 */
export function didTail(did: string, length = 6): string {
	if (!isValidDid(did)) throw new Error(`Invalid DID: ${JSON.stringify(did)}`);
	return did.slice(-length);
}

export function grantRkey(communitySlug: string, memberDid: string): string {
	return `${communitySlug}-${didTail(memberDid)}`;
}
