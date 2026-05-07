package com.freeq.model

/**
 * Pure helpers for the unread-count + last-read-anchor state that
 * AppState mutates from event handlers. Extracted so the predicates
 * can be unit-tested independently of the AndroidViewModel surface.
 */
internal object UnreadTracker {
    /** Should an incoming message in `channel` bump the unread badge?
     *  Don't increment when the user is actively reading that channel,
     *  and don't increment for muted channels. */
    fun shouldIncrement(
        channel: String,
        activeChannel: String?,
        isMuted: Boolean,
    ): Boolean = activeChannel != channel && !isMuted

    /** Pick a message to anchor the "last read" position on when the
     *  user views a channel. Prefers the most recent real message —
     *  system join/part messages use random UUIDs that don't survive
     *  CHATHISTORY replay, so anchoring on them silently breaks
     *  cross-session unread tracking. Falls back to a system message
     *  if nothing real is in the buffer. */
    fun anchorMessage(messages: List<ChatMessage>): ChatMessage? =
        messages.lastOrNull { it.from.isNotEmpty() } ?: messages.lastOrNull()
}
