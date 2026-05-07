package com.freeq.model

/**
 * Pure decision logic for swapping IRC transport from WebSocket to plain
 * TCP after a failed connect. Lives outside AppState so it can be unit-
 * tested without instantiating an Android `ViewModel`.
 */
internal object TransportFallback {
    /** Should we swap from WS to TCP given the current Disconnected event?
     *
     *  Returns true when:
     *  - the disconnect reason names a WebSocket failure,
     *  - we haven't already swapped this attempt,
     *  - we have a saved session to fall back into,
     *  - and we know our nick.
     */
    fun shouldFallback(
        reason: String,
        transportFallbackUsed: Boolean,
        hasSavedSession: Boolean,
        nickIsEmpty: Boolean,
    ): Boolean = !transportFallbackUsed
            && reason.lowercase().contains("websocket")
            && hasSavedSession
            && !nickIsEmpty
}
