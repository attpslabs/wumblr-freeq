package com.freeq.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.Date

class UnreadTrackerTest {

    private fun msg(id: String, from: String = "alice", ts: Long = 0L) = ChatMessage(
        id = id,
        from = from,
        text = "x",
        isAction = false,
        timestamp = Date(ts),
    )

    private fun system(id: String, ts: Long = 0L) = msg(id, from = "", ts = ts)

    // ── shouldIncrement ──

    @Test fun increments_when_inactive_and_unmuted() {
        assertTrue(UnreadTracker.shouldIncrement("#a", activeChannel = "#b", isMuted = false))
        assertTrue(UnreadTracker.shouldIncrement("#a", activeChannel = null, isMuted = false))
    }

    @Test fun does_not_increment_when_channel_is_active() {
        // The user is reading it now — no badge needed.
        assertFalse(UnreadTracker.shouldIncrement("#a", activeChannel = "#a", isMuted = false))
    }

    @Test fun does_not_increment_when_muted() {
        assertFalse(UnreadTracker.shouldIncrement("#a", activeChannel = "#b", isMuted = true))
    }

    @Test fun muted_takes_precedence_over_inactive_match() {
        assertFalse(UnreadTracker.shouldIncrement("#a", activeChannel = null, isMuted = true))
    }

    // ── anchorMessage ──

    @Test fun anchor_returns_last_real_message() {
        val msgs = listOf(msg("a", ts = 1), msg("b", ts = 2), msg("c", ts = 3))
        assertEquals("c", UnreadTracker.anchorMessage(msgs)?.id)
    }

    @Test fun anchor_skips_trailing_system_messages() {
        // System messages (empty `from`) get random UUIDs; anchoring on
        // one strands cross-session unread tracking after CHATHISTORY
        // replay because the id won't survive the replay.
        val msgs = listOf(
            msg("real", ts = 1),
            system("sys-1", ts = 2),
            system("sys-2", ts = 3),
        )
        assertEquals("real", UnreadTracker.anchorMessage(msgs)?.id)
    }

    @Test fun anchor_falls_back_to_system_when_no_real_messages() {
        // Genuinely empty channel with only join/parts — better than null.
        val msgs = listOf(system("sys-1", ts = 1), system("sys-2", ts = 2))
        assertEquals("sys-2", UnreadTracker.anchorMessage(msgs)?.id)
    }

    @Test fun anchor_returns_null_for_empty_list() {
        assertNull(UnreadTracker.anchorMessage(emptyList()))
    }
}
