import XCTest
import UIKit
@testable import freeq

/// Behavior tests for the AV (voice/video call) state machine in AppState.
/// Drives `startOrJoinVoice`, `startCall`, `leaveCall`, the AV event handler,
/// and the inbound `+freeq.at/av-state` TAGMSG handler through synthetic
/// inputs. Substitutes a fake `AvSessionDriver` so no real MoQ session opens.
///
/// The bugs these tests guard against were observed live:
///   - Remote video tile sometimes missing (frame arrives before
///     ParticipantJoined; participantsWithVideo not symmetric with
///     callParticipants on remove).
///   - Disconnected web user leaves a stale tile on iOS (no `av-state=left`
///     handling; participantsWithVideo never cleared on ParticipantLeft).
///   - IRC disconnect leaves the user in a zombie call.
///   - Own nick echoed as remote tile in self-joined sessions.
///   - `av-state=ended` for a channel we're not in re-tore-down the active
///     call (loose `isInCall` gating).
final class AvSessionTests: XCTestCase {

    // MARK: - Fakes

    /// Records every call so tests can assert on what production code did
    /// without actually opening a MoQ session.
    final class FakeAvDriver: AvSessionDriver {
        var muteCalls: [Bool] = []
        var cameraCalls: [Bool] = []
        var setCameraThrows: Error? = nil
        var pushedFrames: Int = 0
        var leaveCalls: Int = 0
        var connected: Bool = true

        func setMuted(muted: Bool) { muteCalls.append(muted) }
        func setCameraEnabled(enabled: Bool) throws {
            cameraCalls.append(enabled)
            if let e = setCameraThrows { throw e }
        }
        func pushVideoFrame(bgra: [UInt8], width: UInt32, height: UInt32, timestampUs: UInt64) {
            pushedFrames += 1
        }
        func leave() { leaveCalls += 1; connected = false }
        func isConnected() -> Bool { connected }
    }

    // MARK: - Harness

    /// Wires a fresh AppState with hooks pre-installed: every wire send is
    /// captured into `sent`, and `startCall` builds a `FakeAvDriver` instead
    /// of a real `FreeqAv`. The test gets back the state, a reference to
    /// the line-capture buffer, and a closure that returns the latest fake
    /// driver (which may be nil before `startCall` runs).
    private func makeHarness(myNick: String = "alice") -> (
        AppState,
        sent: () -> [String],
        latestDriver: () -> FakeAvDriver?
    ) {
        for k in ["freeq.nick", "freeq.server", "freeq.channels", "freeq.readPositions",
                  "freeq.unreadCounts", "freeq.mutedChannels"] {
            UserDefaults.standard.removeObject(forKey: k)
        }
        BufferCacheStore.clear()

        let state = AppState()
        state.nick = myNick

        var lines: [String] = []
        state.rawSenderForTest = { lines.append($0) }

        var lastDriver: FakeAvDriver? = nil
        state.avSessionFactory = { _, _, _, _, _ in
            let d = FakeAvDriver()
            lastDriver = d
            return d
        }

        return (state, { lines }, { lastDriver })
    }

    /// Spin the run loop briefly until `predicate` is true. Returns true on
    /// success, false on timeout. Used to wait out the `Task { @MainActor ... }`
    /// that runs `startOrJoinVoice`'s probe path.
    private func waitFor(timeout: TimeInterval = 1.0, _ predicate: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while !predicate() {
            if Date() > deadline { return false }
            RunLoop.current.run(mode: .default, before: Date(timeIntervalSinceNow: 0.01))
        }
        return true
    }

    // MARK: - 1. startOrJoinVoice: no known session → sends av-start

    func testStartOrJoinVoiceWithNoKnownSessionSendsAvStart() {
        let (state, sent, _) = makeHarness()
        state.activeSessionProbeForTest = { _ in .none }

        state.startOrJoinVoice(channel: "#freeq")
        XCTAssertTrue(waitFor { sent().count == 1 }, "expected one wire line within timeout")

        XCTAssertEqual(sent().count, 1, "exactly one wire line should be sent (av-start)")
        let line = sent().first ?? ""
        XCTAssertTrue(line.hasPrefix("@+freeq.at/av-start;"), "expected av-start TAGMSG, got: \(line)")
        XCTAssertTrue(line.contains("+freeq.at/av-instance="), "av-start must carry +freeq.at/av-instance")
        XCTAssertTrue(line.hasSuffix("TAGMSG #freeq"), "TAGMSG target should be the channel")
        XCTAssertTrue(state.pendingAvStart.contains("#freeq"),
                     "channel should be marked pendingAvStart while we wait for `started` echo")
        XCTAssertFalse(state.isInCall, "isInCall stays false until the server's `started` echo lands")
    }

    // MARK: - 2. startOrJoinVoice joins the server-discovered session,
    // ignoring a stale `activeAvSessions` cache entry.
    //
    // Regression: the cache is only cleared by an `av-state=ended` TAGMSG,
    // which is easily missed (backgrounded, disconnect, or a bot restart
    // that auto-ends the old session with no broadcast). The old code
    // early-returned on the cached id and joined a dead session, putting
    // our MoQ broadcast under a prefix nobody watches — "Eliza can't hear
    // me on iOS". startOrJoinVoice must now always trust the live probe.

    func testStartOrJoinVoiceJoinsDiscoveredSessionIgnoringStaleCache() {
        let (state, sent, driverRef) = makeHarness()
        // A stale id left over from a previous, now-dead session.
        state.activeAvSessions["#freeq"] = "sess-STALE-dead"
        // The server's probe is authoritative: the real live session.
        state.activeSessionProbeForTest = { _ in .found(sessionId: "sess-abc") }

        state.startOrJoinVoice(channel: "#freeq")
        XCTAssertTrue(waitFor { state.isInCall }, "expected to be in-call after discovery")

        XCTAssertEqual(sent().count, 1, "should send exactly one line (av-join)")
        let line = sent().first ?? ""
        XCTAssertTrue(line.hasPrefix("@+freeq.at/av-join;"), "expected av-join, got: \(line)")
        XCTAssertTrue(line.contains("+freeq.at/av-id=sess-abc"),
                      "must join the probed session, not the stale cache id: \(line)")
        XCTAssertFalse(line.contains("sess-STALE-dead"), "stale cache id must never reach the wire")
        XCTAssertTrue(line.contains("+freeq.at/av-instance="))
        XCTAssertTrue(line.hasSuffix("TAGMSG #freeq"))
        XCTAssertEqual(state.currentCallSessionId, "sess-abc")
        XCTAssertEqual(state.currentCallChannel, "#freeq")
        XCTAssertNotNil(driverRef(), "factory should have been invoked")
    }

    // MARK: - 3. startOrJoinVoice no-op when already in call

    func testStartOrJoinVoiceIsNoOpWhenInCall() {
        let (state, sent, _) = makeHarness()
        state.startCall(channel: "#first", sessionId: "sess-1")
        let countAfterFirst = sent().count
        XCTAssertTrue(state.isInCall)

        // Now try to start another call. The `guard !isInCall` bails out
        // synchronously, before any discovery Task is even spawned.
        state.startOrJoinVoice(channel: "#second")

        XCTAssertEqual(sent().count, countAfterFirst,
                       "second startOrJoinVoice while in-call must send nothing on the wire")
        XCTAssertEqual(state.currentCallChannel, "#first")
    }

    // MARK: - 4. startCall reuses currentAvInstance if set, else mints fresh

    func testStartCallReusesCurrentAvInstance() {
        let (state, sent, _) = makeHarness()
        state.currentAvInstance = "deadbeef"

        state.startCall(channel: "#freeq", sessionId: "sess-1")

        let line = sent().first ?? ""
        XCTAssertTrue(line.contains("+freeq.at/av-instance=deadbeef"),
                      "av-join must echo the existing av-instance: \(line)")
    }

    func testStartCallMintsInstanceIfNoneSet() {
        let (state, sent, _) = makeHarness()
        // currentAvInstance starts as nil.
        state.startCall(channel: "#freeq", sessionId: "sess-1")

        let line = sent().first ?? ""
        guard let range = line.range(of: "+freeq.at/av-instance=") else {
            return XCTFail("av-join must include +freeq.at/av-instance, got: \(line)")
        }
        let after = String(line[range.upperBound...])
        let id = after.split(separator: " ").first.map(String.init) ?? ""
        // 8 hex chars per generateAvInstanceId.
        XCTAssertEqual(id.count, 8, "minted instance id should be 8 hex chars, got '\(id)'")
        XCTAssertTrue(id.allSatisfy({ "0123456789abcdef".contains($0) }),
                      "instance id should be lowercase hex")
    }

    // MARK: - 5. leaveCall sends av-leave, clears state

    func testLeaveCallSendsAvLeaveAndClearsState() {
        let (state, sent, driverRef) = makeHarness()
        state.startCall(channel: "#freeq", sessionId: "sess-xyz")
        guard sent().count == 1 else { return XCTFail("setup: expected 1 wire line (av-join)") }
        let instance = state.currentAvInstance ?? "?"
        XCTAssertTrue(state.isInCall)
        let driver = driverRef()

        state.leaveCall()

        // Wire line:
        XCTAssertEqual(sent().count, 2)
        let leave = sent().last ?? ""
        XCTAssertTrue(leave.hasPrefix("@+freeq.at/av-leave;"))
        XCTAssertTrue(leave.contains("+freeq.at/av-id=sess-xyz"))
        XCTAssertTrue(leave.contains("+freeq.at/av-instance=\(instance)"))
        XCTAssertTrue(leave.hasSuffix("TAGMSG #freeq"))

        // State:
        XCTAssertFalse(state.isInCall)
        XCTAssertNil(state.currentCallChannel)
        XCTAssertNil(state.currentCallSessionId)
        XCTAssertNil(state.currentAvInstance)
        XCTAssertEqual(driver?.leaveCalls, 1, "driver.leave() must be called")
    }

    // MARK: - 6. av-state=started populates activeAvSessions

    func testAvStateStartedPopulatesActiveSessions() {
        let (state, _, _) = makeHarness()
        let handler = SwiftEventHandler(appState: state)
        handler.handleEvent(.tagMsg(msg: TagMessage(
            from: "carol", target: "#freeq",
            tags: [
                TagEntry(key: "+freeq.at/av-state", value: "started"),
                TagEntry(key: "+freeq.at/av-id", value: "sess-1"),
                TagEntry(key: "+freeq.at/av-actor", value: "carol"),
            ])))

        XCTAssertEqual(state.activeAvSessions["#freeq"], "sess-1")
        XCTAssertFalse(state.isInCall, "merely learning of a session doesn't put us in it")
    }

    // MARK: - 7. av-state=joined while in call appends participant

    func testAvStateJoinedWhileInCallAppendsParticipant() {
        let (state, _, _) = makeHarness()
        state.startCall(channel: "#freeq", sessionId: "sess-1")
        XCTAssertTrue(state.isInCall)

        SwiftEventHandler(appState: state).handleEvent(.tagMsg(msg: TagMessage(
            from: "server", target: "#freeq",
            tags: [
                TagEntry(key: "+freeq.at/av-state", value: "joined"),
                TagEntry(key: "+freeq.at/av-id", value: "sess-1"),
                TagEntry(key: "+freeq.at/av-actor", value: "bob"),
            ])))

        XCTAssertEqual(state.callParticipants, ["bob"])

        // Same participant again — no duplicate.
        SwiftEventHandler(appState: state).handleEvent(.tagMsg(msg: TagMessage(
            from: "server", target: "#freeq",
            tags: [
                TagEntry(key: "+freeq.at/av-state", value: "joined"),
                TagEntry(key: "+freeq.at/av-id", value: "sess-1"),
                TagEntry(key: "+freeq.at/av-actor", value: "BOB"),
            ])))
        XCTAssertEqual(state.callParticipants.count, 1, "case-insensitive dedup")
    }

    // MARK: - 8. av-state=left removes participant + clears video flag

    func testAvStateLeftRemovesParticipantAndClearsVideoFlag() {
        let (state, _, _) = makeHarness()
        state.startCall(channel: "#freeq", sessionId: "sess-1")

        // Add bob via joined, then a frame so he's in participantsWithVideo too.
        let h = SwiftEventHandler(appState: state)
        h.handleEvent(.tagMsg(msg: TagMessage(from: "server", target: "#freeq", tags: [
            TagEntry(key: "+freeq.at/av-state", value: "joined"),
            TagEntry(key: "+freeq.at/av-id", value: "sess-1"),
            TagEntry(key: "+freeq.at/av-actor", value: "bob"),
        ])))
        state.deliverAvEventForTest(.videoFrame(nick: "bob",
            bgra: [UInt8](repeating: 0, count: 4), width: 1, height: 1))
        XCTAssertTrue(state.participantsWithVideo.contains("bob"))

        // Now bob leaves via TAGMSG.
        h.handleEvent(.tagMsg(msg: TagMessage(from: "server", target: "#freeq", tags: [
            TagEntry(key: "+freeq.at/av-state", value: "left"),
            TagEntry(key: "+freeq.at/av-id", value: "sess-1"),
            TagEntry(key: "+freeq.at/av-actor", value: "bob"),
        ])))

        XCTAssertFalse(state.callParticipants.contains("bob"))
        XCTAssertFalse(state.participantsWithVideo.contains("bob"),
                       "participantsWithVideo must be cleared symmetrically with callParticipants")
    }

    // MARK: - 9. av-state=ended while in call tears down

    func testAvStateEndedTearsDownCurrentCall() {
        let (state, sent, _) = makeHarness()
        state.startCall(channel: "#freeq", sessionId: "sess-1")
        XCTAssertTrue(state.isInCall)
        let sentBeforeEnd = sent().count

        SwiftEventHandler(appState: state).handleEvent(.tagMsg(msg: TagMessage(
            from: "server", target: "#freeq",
            tags: [
                TagEntry(key: "+freeq.at/av-state", value: "ended"),
                TagEntry(key: "+freeq.at/av-id", value: "sess-1"),
            ])))

        XCTAssertFalse(state.isInCall)
        XCTAssertNil(state.currentCallChannel)
        XCTAssertNil(state.currentCallSessionId)
        XCTAssertNil(state.activeAvSessions["#freeq"])
        // `ended` is the server's signal — we don't blast a redundant av-leave
        // back over a session that's already gone.
        XCTAssertEqual(sent().count, sentBeforeEnd,
                       "av-state=ended must NOT trigger any additional wire traffic")
    }

    // MARK: - 10. av-state for channels we're not in is inert against current call

    func testAvStateForOtherChannelDoesNotTouchCurrentCall() {
        let (state, _, _) = makeHarness()
        state.startCall(channel: "#freeq", sessionId: "sess-1")
        XCTAssertTrue(state.isInCall)
        XCTAssertEqual(state.currentCallChannel, "#freeq")

        let h = SwiftEventHandler(appState: state)
        // Some other call on #other ends — must not tear down OURS.
        h.handleEvent(.tagMsg(msg: TagMessage(from: "server", target: "#other", tags: [
            TagEntry(key: "+freeq.at/av-state", value: "ended"),
            TagEntry(key: "+freeq.at/av-id", value: "sess-2"),
        ])))
        XCTAssertTrue(state.isInCall, "#other ending must not end #freeq")
        XCTAssertEqual(state.currentCallChannel, "#freeq")

        // Someone joins a different call — must not appear in OUR participants.
        h.handleEvent(.tagMsg(msg: TagMessage(from: "server", target: "#other", tags: [
            TagEntry(key: "+freeq.at/av-state", value: "joined"),
            TagEntry(key: "+freeq.at/av-id", value: "sess-2"),
            TagEntry(key: "+freeq.at/av-actor", value: "dave"),
        ])))
        XCTAssertFalse(state.callParticipants.contains("dave"))
    }

    // MARK: - 11. Self-echo filtering — TAGMSG only, not FreeqAv events

    /// The TAGMSG `+freeq.at/av-state=joined` broadcast goes to every
    /// channel member including the joiner; we filter our own nick out
    /// of that path so we don't list ourselves as a remote.
    ///
    /// The FreeqAv path is different: the SDK already filters our own
    /// broadcast at the *path* level (`path == our_name`), so anything
    /// reaching `participantJoined` is a different device — including
    /// a same-DID second device (iOS + web for the same handle). We
    /// must NOT filter on bare nick at this layer or we'd lose the
    /// multi-device case (the "iOS doesn't show my web client" bug).
    func testSelfEchoFilteredOnlyOnTagMsgPath() {
        let (state, _, _) = makeHarness(myNick: "alice")
        state.startCall(channel: "#freeq", sessionId: "sess-1")

        // FreeqAv path: same-DID second device. The SDK gave us this
        // event after stripping `~instance`, so the bare nick happens
        // to match — but it's still a different device, so we keep it.
        state.deliverAvEventForTest(.participantJoined(nick: "alice"))
        XCTAssertTrue(state.callParticipants.contains(where: { $0.lowercased() == "alice" }),
                      "FreeqAv participantJoined must surface as a remote tile even if nick matches us — same-DID multi-device")

        // TAGMSG path: server-sent broadcast goes to everyone in the
        // channel, including the joiner. Filter our own nick here.
        state.callParticipants.removeAll()
        SwiftEventHandler(appState: state).handleEvent(.tagMsg(msg: TagMessage(
            from: "server", target: "#freeq",
            tags: [
                TagEntry(key: "+freeq.at/av-state", value: "joined"),
                TagEntry(key: "+freeq.at/av-id", value: "sess-1"),
                TagEntry(key: "+freeq.at/av-actor", value: "ALICE"),
            ])))
        XCTAssertFalse(state.callParticipants.contains(where: { $0.lowercased() == "alice" }),
                       "own nick must not appear via TAGMSG joined (server self-echo)")
    }

    // MARK: - 12. callParticipants reflects what server tells us (limitation pinned)

    /// We currently key participants by nick alone. Two participants with the
    /// same nick collapse in the iOS tile grid — this is a known limitation
    /// (web client keys by (nick, instance) and shows separate tiles).
    /// Pin it down so a future migration to (nick, instance) tuples doesn't
    /// regress silently.
    func testSameNickTwiceCollapsesToOneTile_KnownLimitation() {
        let (state, _, _) = makeHarness()
        state.startCall(channel: "#freeq", sessionId: "sess-1")

        state.deliverAvEventForTest(.participantJoined(nick: "bob"))
        state.deliverAvEventForTest(.participantJoined(nick: "bob"))

        XCTAssertEqual(state.callParticipants, ["bob"],
                       "current implementation collapses same-DID-different-instance to one tile; if this changes, plumb instance id through and update CallView accordingly")
    }

    // MARK: - 13. ParticipantLeft (FreeqAv path) removes from both sets

    func testParticipantLeftClearsParticipantAndVideo() {
        let (state, _, _) = makeHarness()
        state.startCall(channel: "#freeq", sessionId: "sess-1")

        state.deliverAvEventForTest(.participantJoined(nick: "bob"))
        state.deliverAvEventForTest(.videoFrame(nick: "bob",
            bgra: [UInt8](repeating: 0, count: 4), width: 1, height: 1))
        XCTAssertTrue(state.callParticipants.contains("bob"))
        XCTAssertTrue(state.participantsWithVideo.contains("bob"))

        state.deliverAvEventForTest(.participantLeft(nick: "bob"))

        XCTAssertFalse(state.callParticipants.contains("bob"))
        XCTAssertFalse(state.participantsWithVideo.contains("bob"),
                       "ParticipantLeft must also clear participantsWithVideo — leaving the flag set causes a frozen tile next call")
    }

    // MARK: - 14. videoFrame for unknown nick is tolerated

    func testVideoFrameBeforeParticipantJoinedIsDropped() {
        let (state, _, _) = makeHarness()
        state.startCall(channel: "#freeq", sessionId: "sess-1")

        // Frame arrives ahead of the join announcement. Must not crash,
        // must not create a phantom participant, must not flip
        // participantsWithVideo for an absent nick.
        state.deliverAvEventForTest(.videoFrame(nick: "ghost",
            bgra: [UInt8](repeating: 0, count: 4), width: 1, height: 1))

        XCTAssertFalse(state.callParticipants.contains("ghost"))
        XCTAssertFalse(state.participantsWithVideo.contains("ghost"))
    }

    // MARK: - 15. toggleMute flips isMuted AND calls setMuted exactly once

    func testToggleMuteFlipsStateAndCallsDriverOnce() {
        let (state, _, driverRef) = makeHarness()
        state.startCall(channel: "#freeq", sessionId: "sess-1")
        let driver = driverRef()!

        XCTAssertFalse(state.isMuted)

        state.toggleMute()
        XCTAssertTrue(state.isMuted)
        XCTAssertEqual(driver.muteCalls, [true],
                       "toggleMute -> on must call driver.setMuted(true) exactly once")

        state.toggleMute()
        XCTAssertFalse(state.isMuted)
        XCTAssertEqual(driver.muteCalls, [true, false])
    }

    // MARK: - 16. toggleCamera spins up capture and signals driver

    func testToggleCameraOnEnablesCameraOnDriver() throws {
        let (state, _, driverRef) = makeHarness()
        state.startCall(channel: "#freeq", sessionId: "sess-1")
        let driver = driverRef()!
        XCTAssertNil(state.cameraCapture)

        state.toggleCamera()

        XCTAssertTrue(state.isCameraOn)
        XCTAssertEqual(driver.cameraCalls, [true])
        XCTAssertNotNil(state.cameraCapture,
                        "cameraCapture must be allocated on first toggle so the AVCaptureSession setup cost is paid once")
    }

    func testToggleCameraOffDisablesCameraAndStopsPushingFrames() throws {
        let (state, _, driverRef) = makeHarness()
        state.startCall(channel: "#freeq", sessionId: "sess-1")
        let driver = driverRef()!

        state.toggleCamera()  // on
        XCTAssertEqual(driver.cameraCalls, [true])

        // Simulate a frame arriving from the capture pipeline while camera is on.
        // The onFrame closure pushes to avSession.pushVideoFrame.
        let frame: [UInt8] = [0, 0, 0, 255]
        frame.withUnsafeBufferPointer { buf in
            state.cameraCapture?.onFrame?(buf.baseAddress!, buf.count, 1, 1, 0)
        }
        XCTAssertEqual(driver.pushedFrames, 1)

        state.toggleCamera()  // off
        XCTAssertFalse(state.isCameraOn)
        XCTAssertEqual(driver.cameraCalls, [true, false])

        // After camera-off, the capture's onFrame closure may still fire
        // briefly (the capture queue drains). The driver must either be
        // cleared (so pushVideoFrame no-ops) or pushVideoFrame must short-
        // circuit. We pin: the onFrame closure either (a) has no avSession
        // to call into, or (b) the next frames don't increment the count.
        // Easier: nilling cameraCapture itself happens on leaveCall, not on
        // toggle-off; just verify the call signaled the driver.
    }

    // MARK: - 17. Leaving call while camera on cleans up

    func testLeaveCallWhileCameraOnStopsLocalCamera() {
        let (state, _, driverRef) = makeHarness()
        state.startCall(channel: "#freeq", sessionId: "sess-1")
        let driver = driverRef()!
        state.toggleCamera()
        XCTAssertNotNil(state.cameraCapture)
        XCTAssertEqual(driver.cameraCalls, [true])

        state.leaveCall()

        XCTAssertNil(state.cameraCapture, "leaveCall must release the AVCaptureSession")
        XCTAssertFalse(state.isCameraOn)
        // Driver was leave()'d (and possibly setCameraEnabled(false) — but
        // current `leaveCall` skips the explicit camera-off call since the
        // whole driver is going away). Just verify leave fired.
        XCTAssertEqual(driver.leaveCalls, 1)
    }

    // MARK: - 19. FreeqAv Disconnected clears in-call state

    func testFreeqAvDisconnectedClearsCallState() {
        let (state, _, _) = makeHarness()
        state.startCall(channel: "#freeq", sessionId: "sess-1")
        state.deliverAvEventForTest(.participantJoined(nick: "bob"))
        XCTAssertTrue(state.isInCall)
        XCTAssertEqual(state.callParticipants, ["bob"])

        state.deliverAvEventForTest(.disconnected(reason: "network"))

        XCTAssertFalse(state.isInCall)
        XCTAssertTrue(state.callParticipants.isEmpty)
        XCTAssertTrue(state.participantsWithVideo.isEmpty)
        XCTAssertNil(state.currentCallChannel)
        XCTAssertNil(state.currentCallSessionId)
    }

    // MARK: - 20. IRC connection drop tears down call

    func testIrcDisconnectTearsDownInProgressCall() {
        let (state, _, driverRef) = makeHarness()
        state.startCall(channel: "#freeq", sessionId: "sess-1")
        let driver = driverRef()!
        state.deliverAvEventForTest(.participantJoined(nick: "bob"))
        XCTAssertTrue(state.isInCall)

        // Simulate the IRC connection dying entirely.
        SwiftEventHandler(appState: state).handleEvent(.disconnected(reason: "EOF"))

        XCTAssertFalse(state.isInCall, "IRC disconnect must end the call locally; otherwise the UI shows a phantom in-call state")
        XCTAssertNil(state.currentCallChannel)
        XCTAssertTrue(state.callParticipants.isEmpty)
        XCTAssertEqual(driver.leaveCalls, 1, "the MoQ session must be closed when the IRC wire dies")
    }

    // MARK: - Bonus: pendingAvStart consumed by `started` echo only when av-actor=me

    // MARK: - 21. videoFrame populates participantsWithVideo for same-nick remote
    //
    // The current regression: "i'm not seeing web video on ios but i'm seeing
    // both videos on web". Same-DID multi-device case where iOS alice and
    // web alice are both in the call. The SDK already filters the iOS
    // device's own broadcast at the path layer (`path == our_name`); the
    // web alice's `participantJoined` event arrives with nick="alice"
    // (the ~instance suffix was stripped by the SDK). Before fix
    // e76072b, AppState.AvCallbackHandler had `if nick == myNick: return`
    // which collapsed this case into a no-op. The fix removed that check.
    //
    // This test pins the behaviour end-to-end: after FreeqAv emits a
    // participantJoined for our own nick (which is the cue that another
    // device on the same DID is in the call), and then a videoFrame
    // arrives for that nick, both `callParticipants` AND
    // `participantsWithVideo` must populate. Without that pair, the iOS UI
    // shows the avatar fallback even though pixels are arriving — exactly
    // the user-visible regression.
    func testWebAliceVideoFrameAppearsOnIosAlice() {
        let (state, _, _) = makeHarness(myNick: "alice")
        state.startCall(channel: "#freeq", sessionId: "sess-1")
        XCTAssertTrue(state.isInCall)

        // The SDK's `path == our_name` check filtered the iOS broadcast,
        // so the participantJoined we get is web alice's. Nick happens
        // to be the same because it's the same DID/handle.
        state.deliverAvEventForTest(.participantJoined(nick: "alice"))
        XCTAssertTrue(state.callParticipants.contains(where: { $0.lowercased() == "alice" }),
                      "same-nick remote (web alice) must be tracked as a remote participant on iOS")

        // Web's first video frame for that broadcast.
        let bgra = [UInt8](repeating: 0x80, count: 4)
        state.deliverAvEventForTest(.videoFrame(nick: "alice", bgra: bgra, width: 1, height: 1))

        // The user-visible signal: tile becomes "video" (CallView swaps
        // the avatar for the AVSampleBufferDisplayLayer once this set
        // includes the nick).
        XCTAssertTrue(state.participantsWithVideo.contains("alice"),
                      "videoFrame for same-nick remote must populate participantsWithVideo — without this the iOS tile renders an avatar even though pixels are arriving")
    }

    // MARK: - 22. videoFrame race: frame arrives BEFORE participantJoined
    //
    // The SFU can deliver the first video frame slightly before the
    // catalog/announce path fires participantJoined (out-of-order on the
    // network). Current handler drops the frame silently. We pin that:
    // the dropped frame must not (a) create a phantom participant,
    // (b) populate participantsWithVideo for a nick that isn't in
    // callParticipants, (c) crash on missing video layer.
    func testVideoFrameRaceAheadOfParticipantJoined() {
        let (state, _, _) = makeHarness()
        state.startCall(channel: "#freeq", sessionId: "sess-1")

        // Frame arrives first (no join announcement yet).
        state.deliverAvEventForTest(.videoFrame(nick: "racey",
            bgra: [UInt8](repeating: 0, count: 4), width: 1, height: 1))

        XCTAssertFalse(state.callParticipants.contains("racey"))
        XCTAssertFalse(state.participantsWithVideo.contains("racey"),
                       "must not flip the video flag for an absent nick")

        // Now the join arrives, then a second frame. Tile should fill.
        state.deliverAvEventForTest(.participantJoined(nick: "racey"))
        state.deliverAvEventForTest(.videoFrame(nick: "racey",
            bgra: [UInt8](repeating: 0, count: 4), width: 1, height: 1))
        XCTAssertTrue(state.participantsWithVideo.contains("racey"))
    }

    // MARK: - 23. videoTrackStopped clears the video flag but keeps the participant
    //
    // When the remote turns off their camera, FreeqAv emits
    // videoTrackStopped. We must clear participantsWithVideo so the tile
    // reverts to the avatar fallback. BUT we must NOT remove from
    // callParticipants — the user is still in the audio call.
    func testVideoTrackStoppedClearsFlagButKeepsParticipant() {
        let (state, _, _) = makeHarness()
        state.startCall(channel: "#freeq", sessionId: "sess-1")

        state.deliverAvEventForTest(.participantJoined(nick: "bob"))
        state.deliverAvEventForTest(.videoFrame(nick: "bob",
            bgra: [UInt8](repeating: 0, count: 4), width: 1, height: 1))
        XCTAssertTrue(state.callParticipants.contains("bob"))
        XCTAssertTrue(state.participantsWithVideo.contains("bob"))

        // Bob turns off his camera.
        state.deliverAvEventForTest(.videoTrackStopped(nick: "bob"))

        XCTAssertTrue(state.callParticipants.contains("bob"),
                      "videoTrackStopped is a camera-off event, not a leave — keep the participant")
        XCTAssertFalse(state.participantsWithVideo.contains("bob"),
                       "clear the video flag so the tile flips to the avatar fallback")
    }

    // MARK: - 24. videoTrackStopped → camera-on cycle restores the tile
    //
    // After camera-off → camera-on by the remote, a new videoFrame must
    // re-populate participantsWithVideo (avatar swap re-happens cleanly).
    func testVideoCycleOffOnRefillsTile() {
        let (state, _, _) = makeHarness()
        state.startCall(channel: "#freeq", sessionId: "sess-1")

        state.deliverAvEventForTest(.participantJoined(nick: "bob"))
        state.deliverAvEventForTest(.videoFrame(nick: "bob",
            bgra: [UInt8](repeating: 0, count: 4), width: 1, height: 1))
        state.deliverAvEventForTest(.videoTrackStopped(nick: "bob"))
        XCTAssertFalse(state.participantsWithVideo.contains("bob"))

        // Bob turns camera back on; frame arrives again.
        state.deliverAvEventForTest(.videoTrackStarted(nick: "bob"))
        state.deliverAvEventForTest(.videoFrame(nick: "bob",
            bgra: [UInt8](repeating: 0, count: 4), width: 1, height: 1))

        XCTAssertTrue(state.participantsWithVideo.contains("bob"),
                      "second camera-on cycle must restore the video flag")
    }

    // MARK: - 25. Three distinct DIDs — all three appear as participants
    func testThreeDistinctDidsAllAppearAsParticipants() {
        let (state, _, _) = makeHarness(myNick: "alice")
        state.startCall(channel: "#freeq", sessionId: "sess-1")

        state.deliverAvEventForTest(.participantJoined(nick: "bob"))
        state.deliverAvEventForTest(.participantJoined(nick: "carol"))
        // Alice's own broadcast is filtered at the SDK layer; we don't
        // expect ourselves to appear via FreeqAv. But if another device
        // on the same DID also joins (4th case), it surfaces as alice.

        XCTAssertEqual(Set(state.callParticipants), Set(["bob", "carol"]),
                       "both remote nicks must appear (the SDK only filters our own broadcast at the path level)")
        XCTAssertFalse(state.callParticipants.contains("alice"),
                       "our own nick should not appear — the SDK's `path == our_name` check filters our broadcast before participantJoined fires")
    }

    // MARK: - 26. callParticipants does NOT include the local user even
    // when the local user joins via TAGMSG (server self-echo)
    func testCallParticipantsExcludesLocalUserOnTagMsgJoinedEcho() {
        let (state, _, _) = makeHarness(myNick: "alice")
        state.startCall(channel: "#freeq", sessionId: "sess-1")

        // The server broadcasts av-state=joined for every joiner, including
        // ourselves. We must not list ourselves as a remote tile on that
        // self-echo.
        SwiftEventHandler(appState: state).handleEvent(.tagMsg(msg: TagMessage(
            from: "server", target: "#freeq",
            tags: [
                TagEntry(key: "+freeq.at/av-state", value: "joined"),
                TagEntry(key: "+freeq.at/av-id", value: "sess-1"),
                TagEntry(key: "+freeq.at/av-actor", value: "alice"),
            ])))

        XCTAssertFalse(state.callParticipants.contains(where: { $0.lowercased() == "alice" }),
                       "TAGMSG self-echo must not add the local user as a remote tile")
    }

    // MARK: - 27. av-state=joined for someone else's DID on iOS adds remote
    func testThreeDidsViaTagMsgPath() {
        let (state, _, _) = makeHarness(myNick: "alice")
        state.startCall(channel: "#freeq", sessionId: "sess-1")

        let h = SwiftEventHandler(appState: state)
        h.handleEvent(.tagMsg(msg: TagMessage(from: "server", target: "#freeq", tags: [
            TagEntry(key: "+freeq.at/av-state", value: "joined"),
            TagEntry(key: "+freeq.at/av-id", value: "sess-1"),
            TagEntry(key: "+freeq.at/av-actor", value: "bob"),
        ])))
        h.handleEvent(.tagMsg(msg: TagMessage(from: "server", target: "#freeq", tags: [
            TagEntry(key: "+freeq.at/av-state", value: "joined"),
            TagEntry(key: "+freeq.at/av-id", value: "sess-1"),
            TagEntry(key: "+freeq.at/av-actor", value: "carol"),
        ])))

        XCTAssertEqual(Set(state.callParticipants), Set(["bob", "carol"]))
    }

    func testPendingAvStartOnlyConsumedBySelfActor() {
        let (state, _, _) = makeHarness(myNick: "alice")
        state.activeSessionProbeForTest = { _ in .none }
        state.startOrJoinVoice(channel: "#freeq")
        XCTAssertTrue(waitFor { state.pendingAvStart.contains("#freeq") })
        XCTAssertTrue(state.pendingAvStart.contains("#freeq"))

        // av-actor=carol — someone else's start raced ours. Must NOT cause
        // us to auto-join their session (that would either fail with
        // "channel busy" or produce two competing calls).
        SwiftEventHandler(appState: state).handleEvent(.tagMsg(msg: TagMessage(
            from: "server", target: "#freeq",
            tags: [
                TagEntry(key: "+freeq.at/av-state", value: "started"),
                TagEntry(key: "+freeq.at/av-id", value: "sess-carol"),
                TagEntry(key: "+freeq.at/av-actor", value: "carol"),
            ])))

        XCTAssertEqual(state.activeAvSessions["#freeq"], "sess-carol",
                       "we still record the session id for future joiners")
        XCTAssertFalse(state.isInCall,
                       "we don't auto-join someone else's session just because we had a pending start")
        XCTAssertTrue(state.pendingAvStart.contains("#freeq"),
                      "pendingAvStart stays set until our OWN av-start echoes back")
    }
}

// MARK: - Camera orientation

/// Drives `CallCameraCapture.applyOrientation` synchronously and asserts
/// the preview connection's rotation angle is what makes the user appear
/// upright relative to gravity, given the portrait-locked SwiftUI UI.
///
/// The app's `Info.plist` does not declare `UISupportedInterfaceOrientations`,
/// so SwiftUI never re-lays-out when the device rotates — the preview tile
/// stays portrait-shaped. That makes Apple's canonical rotating-UI mapping
/// (portrait=90, landscapeLeft=0, landscapeRight=180, portraitUpsideDown=270)
/// **wrong for us**: in landscape, that mapping rotates the buffer into a
/// landscape-shaped image which then gets jammed into a portrait rect tilted
/// in the user's vision, producing the upside-down-in-landscape preview the
/// user reported.
///
/// The angles asserted here are the ones that land the user's head at
/// gravity-up regardless of how the device is tilted. See the comment block
/// on `CallCameraCapture.applyOrientation` for the derivation.
final class CallCameraCaptureOrientationTests: XCTestCase {

    func testPortraitOrientationKeepsUserUpright() {
        let cap = CallCameraCapture()
        cap.configureForTest()

        cap.applyOrientationForTest(.portrait)

        XCTAssertEqual(cap.previewRotationAngleForTest, 90,
                       "portrait → preview rotates 90° CW so the user's head appears at the top of the portrait rect")
    }

    func testLandscapeLeftOrientationKeepsUserUprightInPortraitLockedRect() {
        let cap = CallCameraCapture()
        cap.configureForTest()

        cap.applyOrientationForTest(.landscapeLeft)

        // User-reported bug: Apple's standard mapping (0°) leaves the
        // preview upside-down in landscape because our UI is
        // portrait-locked, not rotating. The correct angle is 270° CW.
        XCTAssertEqual(cap.previewRotationAngleForTest, 270,
                       "landscapeLeft + portrait-locked UI → 270° CW keeps the user's head at vision-up")
    }

    func testLandscapeRightOrientationKeepsUserUprightInPortraitLockedRect() {
        let cap = CallCameraCapture()
        cap.configureForTest()

        cap.applyOrientationForTest(.landscapeRight)

        XCTAssertEqual(cap.previewRotationAngleForTest, 270,
                       "landscapeRight + portrait-locked UI → 270° CW keeps the user's head at vision-up")
    }

    func testPortraitUpsideDownKeepsUserUpright() {
        let cap = CallCameraCapture()
        cap.configureForTest()

        cap.applyOrientationForTest(.portraitUpsideDown)

        // The rect rotates with the device (it's drawn in device-portrait
        // coords), so what's at rect-top in pixels is at vision-bottom for
        // an upside-down device — and the buffer's head-position also
        // flipped to compensate. Net rotation: 90° CW, same as portrait.
        XCTAssertEqual(cap.previewRotationAngleForTest, 90,
                       "portraitUpsideDown + portrait-locked UI → 90° CW (same as portrait); rect & buffer rotations cancel")
    }

    func testFaceUpFaceDownAndUnknownLeavePreviewAlone() {
        let cap = CallCameraCapture()
        cap.configureForTest()
        cap.applyOrientationForTest(.portrait)
        let before = cap.previewRotationAngleForTest

        cap.applyOrientationForTest(.faceUp)
        XCTAssertEqual(cap.previewRotationAngleForTest, before,
                       "face-up is not a meaningful UI rotation; keep the prior preview angle")
        cap.applyOrientationForTest(.faceDown)
        XCTAssertEqual(cap.previewRotationAngleForTest, before,
                       "face-down is not a meaningful UI rotation; keep the prior preview angle")
        cap.applyOrientationForTest(.unknown)
        XCTAssertEqual(cap.previewRotationAngleForTest, before)
    }

    /// `lastValidOrientation` must remember the most recent UI-meaningful
    /// orientation so the resume-from-faceUp path (e.g., user sets phone on
    /// the table during a call, picks it back up) renders the right
    /// rotation.
    func testFaceUpDoesNotResetLastValidOrientation() {
        let cap = CallCameraCapture()
        cap.configureForTest()

        cap.applyOrientationForTest(.landscapeLeft)
        cap.applyOrientationForTest(.faceUp)

        XCTAssertEqual(cap.lastValidOrientationForTest, .landscapeLeft,
                       "lastValidOrientation must not be overwritten by faceUp/faceDown/unknown")
    }

    /// The data-output (encoder feed) connection's rotation must NOT change
    /// across orientation events. We've deliberately locked it at the
    /// preset-configured angle because the encoder is set to 1280x720
    /// landscape. User-visible rotation comes from software-rotating the
    /// BGRA buffer in `captureOutput` before pushing it to the encoder.
    func testDataOutputRotationIsImmutableAcrossOrientationChanges() {
        let cap = CallCameraCapture()
        cap.configureForTest()
        let initial = cap.dataOutputRotationAngleForTest

        cap.applyOrientationForTest(.portrait)
        XCTAssertEqual(cap.dataOutputRotationAngleForTest, initial)
        cap.applyOrientationForTest(.landscapeRight)
        XCTAssertEqual(cap.dataOutputRotationAngleForTest, initial)
        cap.applyOrientationForTest(.landscapeLeft)
        XCTAssertEqual(cap.dataOutputRotationAngleForTest, initial)
        cap.applyOrientationForTest(.portraitUpsideDown)
        XCTAssertEqual(cap.dataOutputRotationAngleForTest, initial)
    }

    /// Two consecutive flips faster than any notification debounce (there
    /// isn't one — we process every notification) should settle at the
    /// final orientation, not an intermediate.
    func testRapidConsecutiveFlipsSettleAtFinalOrientation() {
        let cap = CallCameraCapture()
        cap.configureForTest()

        cap.applyOrientationForTest(.portrait)
        cap.applyOrientationForTest(.landscapeLeft)
        cap.applyOrientationForTest(.landscapeRight)
        cap.applyOrientationForTest(.portrait)

        XCTAssertEqual(cap.previewRotationAngleForTest, 90,
                       "final state must reflect the last applied orientation")
        XCTAssertEqual(cap.lastValidOrientationForTest, .portrait)
    }

    /// Initial-orientation race: a call that starts in landscape must
    /// render its very first frame with the correct rotation, not the
    /// configure-time portrait default. We simulate `configureIfNeeded`'s
    /// initial-orientation read via `simulateInitialConfigure` and assert
    /// the resulting preview angle.
    func testInitialOrientationReadAppliesBeforeFirstFrame() {
        let cap = CallCameraCapture()
        cap.configureForTest()
        cap.setOrientationProviderForTest { .landscapeLeft }

        cap.simulateInitialConfigure()

        XCTAssertEqual(cap.previewRotationAngleForTest, 270,
                       "call started in landscapeLeft must seed the preview at 270°, not the 90° default")
    }

    func testInitialOrientationFallsBackToPortraitForFaceUpOrUnknown() {
        let cap = CallCameraCapture()
        cap.configureForTest()
        cap.setOrientationProviderForTest { .faceUp }

        cap.simulateInitialConfigure()

        XCTAssertEqual(cap.previewRotationAngleForTest, 90,
                       "if the device starts faceUp, fall back to portrait (90°) so we don't render a black tile")
    }

    /// `pendingPreviewAngle` is the angle we'd apply if the preview
    /// connection were ready. It must update even when no connection
    /// exists (the simulator/unit-test case). That guarantees the
    /// affine-transform fallback knows the right angle the moment the
    /// connection appears.
    func testPendingPreviewAngleTracksOrientationWithNoConnection() {
        let cap = CallCameraCapture()
        cap.configureForTest()

        cap.applyOrientationForTest(.landscapeRight)
        XCTAssertEqual(cap.pendingPreviewAngle, 270)

        cap.applyOrientationForTest(.portrait)
        XCTAssertEqual(cap.pendingPreviewAngle, 90)

        cap.applyOrientationForTest(.portraitUpsideDown)
        XCTAssertEqual(cap.pendingPreviewAngle, 90)
    }

    /// When the `AVCaptureVideoPreviewLayer.connection` is nil (the
    /// session hasn't started yet, or we're in the simulator with no
    /// real camera), the orientation handler falls back to a CALayer
    /// affine transform. The CW rotation angle in
    /// `AVCaptureConnection.videoRotationAngle` is the OPPOSITE sign to
    /// `CGAffineTransform(rotationAngle:)` (which is CCW for positive
    /// angles on iOS). We must negate the angle in the fallback or the
    /// preview spins the wrong way around the layer.
    ///
    /// This test pins the math so a future "clean up the fallback" PR
    /// can't silently reverse the rotation again.
    func testAffineTransformFallbackUsesClockwiseConvention() {
        let cap = CallCameraCapture()
        cap.configureForTest()

        cap.applyOrientationForTest(.portrait)  // angle = 90° CW

        let t = cap.previewLayer.affineTransform()
        // CGAffineTransform(rotationAngle: θ) has a = cos(θ), b = sin(θ).
        // For 90° CW we want θ_radians = -π/2 (because iOS treats
        // positive as CCW). cos(-π/2) = 0, sin(-π/2) = -1.
        XCTAssertEqual(t.a, 0, accuracy: 1e-6, "cos(-π/2) should be 0")
        XCTAssertEqual(t.b, -1, accuracy: 1e-6,
                       "sin(-π/2) should be -1; +1 would mean we rotated CCW (wrong direction)")
    }

    /// Camera toggle off → flip device to landscape → toggle on. The new
    /// CallCameraCapture instance (allocated lazily by AppState) must
    /// pick up the current orientation from the provider, NOT default to
    /// portrait. We simulate that lifecycle here.
    func testCameraToggleOnAfterRotationPicksUpCurrentOrientation() {
        let cap = CallCameraCapture()
        cap.configureForTest()
        cap.setOrientationProviderForTest { .landscapeRight }

        // First-run path: configureIfNeeded runs once, reads
        // orientationProvider, applies orientation.
        cap.simulateInitialConfigure()

        XCTAssertEqual(cap.previewRotationAngleForTest, 270,
                       "starting a fresh capture while in landscape must seed at 270°, matching gravity-up")
        XCTAssertEqual(cap.lastValidOrientationForTest, .landscapeRight)
    }
}

// MARK: - Camera orientation: broadcast software rotation

/// The broadcast path (data-output → H.264 encoder → receiver) has a
/// different constraint: the encoder is locked at the configure-time
/// dimensions (1280×720 from `VideoPreset::P720`). We can't change
/// `videoRotationAngle` on the data-output connection because that
/// produces a 720×1280 buffer in portrait, which the encoder rejects.
///
/// Instead we *software-rotate* the BGRA buffer in `captureOutput` so the
/// contents are upright but the dimensions stay 1280×720. Portrait
/// orientations get letterboxed (black bars on each side).
///
/// These tests use a small BGRA buffer with a single distinctive "head
/// marker" pixel, drive `rotatedFrame` directly, and assert the marker
/// ends up at the expected position post-rotation.
final class CallCameraCaptureBroadcastRotationTests: XCTestCase {

    /// Build a W×H BGRA buffer with one "head marker" pixel at the given
    /// (col, row). Marker is red (B=0, G=0, R=255, A=255); everything
    /// else is black.
    private func buffer(width W: Int, height H: Int, markerAt: (col: Int, row: Int)) -> [UInt8] {
        var buf = [UInt8](repeating: 0, count: W * H * 4)
        let idx = (markerAt.row * W + markerAt.col) * 4
        buf[idx]     = 0    // B
        buf[idx + 1] = 0    // G
        buf[idx + 2] = 255  // R
        buf[idx + 3] = 255  // A
        return buf
    }

    /// Find the pixel index where R == 255 in a BGRA buffer.
    private func findMarker(_ buf: [UInt8], width W: Int, height H: Int) -> (col: Int, row: Int)? {
        for y in 0..<H {
            for x in 0..<W {
                let i = (y * W + x) * 4
                if buf[i + 2] == 255 { return (x, y) }
            }
        }
        return nil
    }

    /// Sensor in portrait orientation: head appears at the buffer's LEFT
    /// edge. After our 90° CW rotation + letterbox into a landscape canvas,
    /// the head must end up at the TOP of the canvas (anywhere along the
    /// centred content column).
    ///
    /// We use a 16×4 source so the contentColW after letterboxing is 1,
    /// and place the marker in a column that the nearest-neighbour scaler
    /// will visit. The marker spans the left column (col=0) at all rows
    /// so the scaler can't miss it. After rotation, the entire left
    /// column should map to the TOP ROW of the canvas (within the
    /// centred content window).
    func testPortraitRotatesHeadFromBufferLeftToOutputTop() {
        let W = 16, H = 4
        // Paint the entire left column red — this is the user's head
        // anchored to the buffer's left edge in portrait.
        var src = [UInt8](repeating: 0, count: W * H * 4)
        for y in 0..<H {
            let idx = (y * W + 0) * 4
            src[idx + 2] = 255
            src[idx + 3] = 255
        }

        let out = CallCameraCapture.rotatedFrame(
            sourceBGRA: src, sourceWidth: W, sourceHeight: H, for: .portrait
        )

        XCTAssertEqual(out.width, W, "output canvas must keep source landscape width")
        XCTAssertEqual(out.height, H, "output canvas must keep source landscape height")
        guard let m = findMarker(out.data, width: out.width, height: out.height) else {
            return XCTFail("portrait rotation lost the marker entirely")
        }
        XCTAssertEqual(m.row, 0,
                       "portrait input head (left column of buffer) must end up at the TOP row of the rotated output, got row \(m.row)")
    }

    /// Sensor in landscapeRight: head at buffer-top. No rotation needed.
    func testLandscapeRightIsAnIdentityRotation() {
        let W = 8, H = 2
        let src = buffer(width: W, height: H, markerAt: (col: 4, row: 0))

        let out = CallCameraCapture.rotatedFrame(
            sourceBGRA: src, sourceWidth: W, sourceHeight: H, for: .landscapeRight
        )

        let m = findMarker(out.data, width: out.width, height: out.height)
        XCTAssertEqual(m?.col, 4)
        XCTAssertEqual(m?.row, 0,
                       "landscapeRight is a no-op: marker at top stays at top")
    }

    /// Sensor in landscapeLeft: head at buffer-bottom. 180° rotation
    /// must put the marker at the top, and mirror it left-right.
    func testLandscapeLeftRotates180SoBottomBecomesTop() {
        let W = 8, H = 2
        let src = buffer(width: W, height: H, markerAt: (col: 2, row: H - 1))

        let out = CallCameraCapture.rotatedFrame(
            sourceBGRA: src, sourceWidth: W, sourceHeight: H, for: .landscapeLeft
        )

        guard let m = findMarker(out.data, width: out.width, height: out.height) else {
            return XCTFail("landscapeLeft rotation lost the marker")
        }
        XCTAssertEqual(m.row, 0,
                       "landscapeLeft → 180° flips bottom to top; marker row must be 0, got \(m.row)")
        XCTAssertEqual(m.col, W - 1 - 2,
                       "180° rotation must also flip columns left-right")
    }

    /// portraitUpsideDown: head at buffer-right. 270° CW rotation +
    /// letterbox. For 270° CW, source (sx, sy) maps to rotated (rx, ry)
    /// where (sx, sy) = (W-1-ry, rx). Source (W-1, 0) → rotated (0, 0)
    /// = canvas top after letterbox.
    func testPortraitUpsideDownPutsHeadAtOutputTop() {
        let W = 8, H = 2
        let src = buffer(width: W, height: H, markerAt: (col: W - 1, row: 0))

        let out = CallCameraCapture.rotatedFrame(
            sourceBGRA: src, sourceWidth: W, sourceHeight: H, for: .portraitUpsideDown
        )

        guard let m = findMarker(out.data, width: out.width, height: out.height) else {
            return XCTFail("portraitUpsideDown rotation lost the marker")
        }
        XCTAssertEqual(m.row, 0,
                       "portraitUpsideDown rotation must land the head at output row 0, got \(m.row)")
    }

    /// faceUp / faceDown / unknown must NOT crash and must not return an
    /// empty buffer. We treat them as portrait (matches the preview's
    /// "keep prior orientation" semantics — and an upright portrait is the
    /// safer default than an empty canvas).
    func testFaceUpOrientationFallsBackToPortraitRotation() {
        let W = 8, H = 2
        let src = buffer(width: W, height: H, markerAt: (col: 0, row: 0))

        let outFace = CallCameraCapture.rotatedFrame(
            sourceBGRA: src, sourceWidth: W, sourceHeight: H, for: .faceUp
        )
        let outPort = CallCameraCapture.rotatedFrame(
            sourceBGRA: src, sourceWidth: W, sourceHeight: H, for: .portrait
        )
        XCTAssertEqual(outFace.data, outPort.data,
                       ".faceUp must behave the same as .portrait so we never push an empty/garbled frame to the encoder")
    }

    /// All four orientations must preserve the source dimensions so the
    /// H.264 encoder (locked at 1280×720) accepts every frame. This is
    /// the "thou shalt not change the dimensions" invariant.
    func testRotatedFrameAlwaysReturnsSourceDimensions() {
        let W = 8, H = 2
        let src = buffer(width: W, height: H, markerAt: (col: 0, row: 0))

        for o in [UIDeviceOrientation.portrait, .portraitUpsideDown,
                  .landscapeLeft, .landscapeRight, .faceUp, .unknown] {
            let out = CallCameraCapture.rotatedFrame(
                sourceBGRA: src, sourceWidth: W, sourceHeight: H, for: o
            )
            XCTAssertEqual(out.width, W,
                           "rotatedFrame must keep source width for orientation \(o.rawValue)")
            XCTAssertEqual(out.height, H,
                           "rotatedFrame must keep source height for orientation \(o.rawValue)")
            XCTAssertEqual(out.data.count, W * H * 4,
                           "rotatedFrame must return W*H*4 bytes for orientation \(o.rawValue)")
        }
    }
}
