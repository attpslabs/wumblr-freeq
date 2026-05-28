import adminsJson from "../lexicons/com/wumblr/admins.json" with { type: "json" };
import memberJson from "../lexicons/com/wumblr/member.json" with { type: "json" };
import membershipProofJson from "../lexicons/com/wumblr/membershipProof.json" with { type: "json" };
import profileJson from "../lexicons/com/wumblr/profile.json" with { type: "json" };

export const lexicons = {
	profile: profileJson,
	admins: adminsJson,
	member: memberJson,
	membershipProof: membershipProofJson,
} as const;

export const NSIDS = {
	profile: "com.wumblr.profile",
	admins: "com.wumblr.admins",
	member: "com.wumblr.member",
	membershipProof: "com.wumblr.membershipProof",
} as const;

export type JoinMode = "open" | "invite";

export interface AdminEntry {
	did: string;
	addedAt: string;
}
