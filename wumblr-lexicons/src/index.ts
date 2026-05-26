import communityJson from "../lexicons/com/wumblr/community.json" with { type: "json" };
import memberJson from "../lexicons/com/wumblr/community/member.json" with { type: "json" };
import grantJson from "../lexicons/com/wumblr/community/membership/grant.json" with { type: "json" };
import channelJson from "../lexicons/com/wumblr/community/channel.json" with { type: "json" };
import requestJson from "../lexicons/com/wumblr/community/request.json" with { type: "json" };

export const lexicons = {
	community: communityJson,
	communityMember: memberJson,
	communityMembershipGrant: grantJson,
	communityChannel: channelJson,
	communityRequest: requestJson,
} as const;

export const NSIDS = {
	community: "com.wumblr.community",
	communityMember: "com.wumblr.community.member",
	communityMembershipGrant: "com.wumblr.community.membership.grant",
	communityChannel: "com.wumblr.community.channel",
	communityRequest: "com.wumblr.community.request",
} as const;

export type CommunityType = "public" | "restricted" | "private" | "mature";
export type CommunityRole = "owner" | "mod" | "member";
export type ChannelKind = "text" | "voice" | "video";
export type RequestStatus = "pending" | "approved" | "rejected";

export const TOPICS = [
	"anime-manga",
	"art",
	"business-finance",
	"collectibles",
	"education-culture",
	"fashion-beauty",
	"food-cooking",
	"games",
	"health",
	"home-garden",
	"humanities",
	"identity-culture",
	"internet-culture",
	"memes",
	"music",
	"nature-outdoors",
	"news-politics",
	"places-travel",
	"pop-culture",
	"qa-trivia",
	"reading-writing",
	"science",
	"sports",
	"technology",
	"vehicles",
	"wellness",
	"adult",
	"mature",
] as const;
export type Topic = (typeof TOPICS)[number];
