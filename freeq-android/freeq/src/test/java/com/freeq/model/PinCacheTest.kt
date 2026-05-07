package com.freeq.model

import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Pure-sync tests for PinCache. Only covers the synchronous API
 * (`isPinned`, `setAll`) — `addPin`/`removePin` post through
 * `Dispatchers.Main` which a plain JVM test can't drive without a
 * test-main dispatcher. Those paths are exercised by the integration
 * tests on a connected device.
 */
class PinCacheTest {

    @After
    fun tearDown() {
        // PinCache is a process-global object; clear between tests so
        // ordering doesn't leak state across them.
        PinCache.pins.clear()
    }

    @Test fun isPinned_false_for_unknown_channel() {
        assertFalse(PinCache.isPinned("#nope", "abc"))
    }

    @Test fun setAll_then_isPinned_roundtrips() {
        PinCache.setAll("#freeq", setOf("m1", "m2"))
        assertTrue(PinCache.isPinned("#freeq", "m1"))
        assertTrue(PinCache.isPinned("#freeq", "m2"))
        assertFalse(PinCache.isPinned("#freeq", "m3"))
    }

    @Test fun setAll_replaces_prior_set_does_not_merge() {
        // Server is the source of truth — a fresh /pins fetch must
        // overwrite, not union, otherwise unpinned messages silently
        // remain pinned in the UI until app restart.
        PinCache.setAll("#freeq", setOf("m1", "m2"))
        PinCache.setAll("#freeq", setOf("m3"))
        assertFalse(PinCache.isPinned("#freeq", "m1"))
        assertFalse(PinCache.isPinned("#freeq", "m2"))
        assertTrue(PinCache.isPinned("#freeq", "m3"))
    }

    @Test fun channel_lookup_is_case_insensitive() {
        // Channel names round-trip through different casings on the
        // wire (JOIN, PIN tag, REST API). The cache lowercases on
        // write so reads must match regardless of case.
        PinCache.setAll("#FreeQ", setOf("m1"))
        assertTrue(PinCache.isPinned("#freeq", "m1"))
        assertTrue(PinCache.isPinned("#FREEQ", "m1"))
        assertTrue(PinCache.isPinned("#FreeQ", "m1"))
    }

    @Test fun setAll_with_empty_clears_pins_for_channel() {
        PinCache.setAll("#freeq", setOf("m1"))
        PinCache.setAll("#freeq", emptySet())
        assertFalse(PinCache.isPinned("#freeq", "m1"))
    }

    @Test fun different_channels_are_independent() {
        PinCache.setAll("#a", setOf("m1"))
        PinCache.setAll("#b", setOf("m2"))
        assertTrue(PinCache.isPinned("#a", "m1"))
        assertFalse(PinCache.isPinned("#a", "m2"))
        assertTrue(PinCache.isPinned("#b", "m2"))
        assertFalse(PinCache.isPinned("#b", "m1"))
    }

    @Test fun isPinned_msgid_match_is_case_sensitive() {
        // Channel keys are lowercased; msgids are ULIDs and must
        // match exactly. Mismatched casing on a msgid is a bug, not
        // a normalization need.
        PinCache.setAll("#freeq", setOf("01HXYZ"))
        assertTrue(PinCache.isPinned("#freeq", "01HXYZ"))
        assertFalse(PinCache.isPinned("#freeq", "01hxyz"))
    }
}
