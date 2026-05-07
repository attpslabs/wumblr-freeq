package com.freeq.model

/**
 * A parsed slash-command from the compose box. Pulled out of
 * `ComposeBar.handleCommand` so the parser is testable as pure logic
 * and the dispatch site stays a small `when`.
 */
internal sealed interface SlashCommand {
    data class Join(val channel: String) : SlashCommand
    object PartActive : SlashCommand
    data class Nick(val newNick: String) : SlashCommand
    data class Me(val text: String) : SlashCommand
    data class Msg(val target: String, val text: String) : SlashCommand
    data class Topic(val text: String) : SlashCommand
    /** Unknown command: forwarded to the server as a raw IRC line. */
    data class Raw(val line: String) : SlashCommand
    /** Recognized command but the required argument was missing or
     *  malformed (e.g. `/msg` with no target). The dispatch site treats
     *  this as a silent no-op. */
    object Empty : SlashCommand
}

internal object SlashCommandParser {
    fun parse(input: String): SlashCommand {
        val withoutSlash = input.removePrefix("/")
        val parts = withoutSlash.split(" ", limit = 2)
        val cmd = parts.firstOrNull()?.lowercase()
        if (cmd.isNullOrEmpty()) return SlashCommand.Empty
        val arg = parts.getOrNull(1)?.takeIf { it.isNotEmpty() }
        return when (cmd) {
            "join" -> arg?.let { SlashCommand.Join(it) } ?: SlashCommand.Empty
            "part", "leave" -> SlashCommand.PartActive
            "nick" -> arg?.let { SlashCommand.Nick(it) } ?: SlashCommand.Empty
            "me" -> arg?.let { SlashCommand.Me(it) } ?: SlashCommand.Empty
            "msg" -> {
                val msgParts = (arg ?: "").split(" ", limit = 2)
                if (msgParts.size == 2 && msgParts[0].isNotEmpty() && msgParts[1].isNotEmpty())
                    SlashCommand.Msg(msgParts[0], msgParts[1])
                else SlashCommand.Empty
            }
            "topic" -> arg?.let { SlashCommand.Topic(it) } ?: SlashCommand.Empty
            else -> SlashCommand.Raw(withoutSlash)
        }
    }
}
