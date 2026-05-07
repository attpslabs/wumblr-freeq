package com.freeq.model

/**
 * Routing predicate for IRCv3 TAGMSG dispatch (typing, edit, delete,
 * react). Pulled out of `AndroidEventHandler.onEvent` so the
 * self-echo + channel-vs-DM resolution rule lives in one place.
 */
internal object TagMsgRouter {
    /**
     * Resolves which buffer a TAGMSG should be applied to.
     *
     * Returns the buffer name — a channel like `#freeq` or a peer nick —
     * or `null` when the TAGMSG is our own echo coming back through the
     * server, in which case the dispatch site must ignore it (we already
     * applied the action optimistically on send).
     *
     * Rules:
     * - Self-echo (sender matches our own nick, case-insensitive) ⇒ null.
     * - `target` starts with `#` ⇒ it's a channel TAGMSG; route to that
     *   channel.
     * - Otherwise ⇒ it's a DM TAGMSG. Note: the IRC `target` for a DM
     *   TAGMSG is our own nick (the recipient), so the buffer is named
     *   after the sender (`from`).
     */
    fun routeTo(target: String, from: String, selfNick: String): String? {
        if (from.equals(selfNick, ignoreCase = true)) return null
        return if (target.startsWith("#")) target else from
    }
}
