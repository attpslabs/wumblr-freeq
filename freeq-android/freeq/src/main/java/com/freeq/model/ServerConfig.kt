package com.freeq.model

import com.freeq.BuildConfig

/**
 * Central server configuration. Defaults are baked in per build flavor;
 * see `productFlavors` in `freeq/build.gradle.kts`.
 */
object ServerConfig {
    var ircServer: String = BuildConfig.IRC_SERVER
    var authBrokerBase: String = BuildConfig.AUTH_BROKER_BASE

    val apiBaseUrl: String
        get() = "https://" + ircServer.substringBefore(":")
    val wssServer: String
        get() = "wss://" + ircServer.substringBefore(":") + "/irc"
}
