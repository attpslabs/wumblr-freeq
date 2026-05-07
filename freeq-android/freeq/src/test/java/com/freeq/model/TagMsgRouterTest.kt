package com.freeq.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * Tests for TAGMSG buffer-routing. Bugs in this rule cause edits /
 * deletes / reactions to apply to the wrong message or get silently
 * dropped — easy to miss in manual testing because the optimistic
 * client-side update masks them.
 */
class TagMsgRouterTest {

    @Test fun channel_target_routes_to_that_channel() {
        assertEquals("#freeq", TagMsgRouter.routeTo(target = "#freeq", from = "alice", selfNick = "me"))
    }

    @Test fun dm_target_routes_to_sender() {
        // The wire-level target on a DM is OUR nick (we're the recipient);
        // the buffer is named after the sender.
        assertEquals("alice", TagMsgRouter.routeTo(target = "me", from = "alice", selfNick = "me"))
    }

    @Test fun self_echo_returns_null() {
        // We already applied the action optimistically when the user
        // took it. Re-applying on the server echo would double the effect.
        assertNull(TagMsgRouter.routeTo(target = "#freeq", from = "me", selfNick = "me"))
        assertNull(TagMsgRouter.routeTo(target = "alice", from = "me", selfNick = "me"))
    }

    @Test fun self_echo_check_is_case_insensitive() {
        assertNull(TagMsgRouter.routeTo(target = "#freeq", from = "ME", selfNick = "me"))
        assertNull(TagMsgRouter.routeTo(target = "#freeq", from = "Me", selfNick = "me"))
    }

    @Test fun ampersand_local_channel_routes_as_dm() {
        // `&local` is technically an IRC channel prefix but freeq's
        // current code only treats `#` as channel-shaped. Any change
        // to that policy should update both BufferRouter and this
        // routing rule together — test pins the current behavior.
        assertEquals("alice", TagMsgRouter.routeTo(target = "&local", from = "alice", selfNick = "me"))
    }
}
