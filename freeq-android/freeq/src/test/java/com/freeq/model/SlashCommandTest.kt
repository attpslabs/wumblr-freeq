package com.freeq.model

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Tests for the slash-command parser. Compose-bar bugs that silently
 * eat user input land here.
 */
class SlashCommandTest {

    @Test fun parses_join_with_channel() {
        assertEquals(
            SlashCommand.Join("#freeq"),
            SlashCommandParser.parse("/join #freeq"),
        )
    }

    @Test fun join_with_no_arg_is_empty() {
        assertEquals(SlashCommand.Empty, SlashCommandParser.parse("/join"))
        assertEquals(SlashCommand.Empty, SlashCommandParser.parse("/join "))
    }

    @Test fun part_and_leave_both_resolve_to_PartActive() {
        assertEquals(SlashCommand.PartActive, SlashCommandParser.parse("/part"))
        assertEquals(SlashCommand.PartActive, SlashCommandParser.parse("/leave"))
        // Either with an arg also resolves to PartActive — the dispatch
        // site uses appState.activeChannel.value, not the argument.
        assertEquals(SlashCommand.PartActive, SlashCommandParser.parse("/part #ignored"))
    }

    @Test fun parses_nick_change() {
        assertEquals(SlashCommand.Nick("newhandle"), SlashCommandParser.parse("/nick newhandle"))
    }

    @Test fun nick_with_no_arg_is_empty() {
        assertEquals(SlashCommand.Empty, SlashCommandParser.parse("/nick"))
    }

    @Test fun parses_me_action() {
        assertEquals(SlashCommand.Me("waves hello"), SlashCommandParser.parse("/me waves hello"))
    }

    @Test fun parses_msg_with_target_and_text() {
        assertEquals(
            SlashCommand.Msg("alice", "hi there"),
            SlashCommandParser.parse("/msg alice hi there"),
        )
    }

    @Test fun msg_without_text_is_empty() {
        assertEquals(SlashCommand.Empty, SlashCommandParser.parse("/msg alice"))
        assertEquals(SlashCommand.Empty, SlashCommandParser.parse("/msg"))
    }

    @Test fun parses_topic() {
        assertEquals(
            SlashCommand.Topic("the new topic"),
            SlashCommandParser.parse("/topic the new topic"),
        )
    }

    @Test fun unknown_command_falls_through_to_raw() {
        // Forwarded as a raw IRC line for the SDK to handle.
        assertEquals(
            SlashCommand.Raw("invite alice #freeq"),
            SlashCommandParser.parse("/invite alice #freeq"),
        )
        assertEquals(
            SlashCommand.Raw("whois alice"),
            SlashCommandParser.parse("/whois alice"),
        )
    }

    @Test fun command_lookup_is_case_insensitive() {
        assertEquals(SlashCommand.Join("#x"), SlashCommandParser.parse("/JOIN #x"))
        assertEquals(SlashCommand.Join("#x"), SlashCommandParser.parse("/Join #x"))
    }

    @Test fun empty_or_lone_slash_input_is_empty() {
        assertEquals(SlashCommand.Empty, SlashCommandParser.parse("/"))
    }

    @Test fun preserves_trailing_arguments_with_spaces() {
        // /me's argument should keep its inner whitespace, not be tokenized.
        assertEquals(
            SlashCommand.Me("does  the  weird   thing"),
            SlashCommandParser.parse("/me does  the  weird   thing"),
        )
    }
}
