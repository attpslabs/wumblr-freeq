// @vitest-environment jsdom
/**
 * Failing tests for a serious hang-up bug: leaving a web voice call
 * leaves the microphone capturing. The browser mic indicator stays on
 * and the user is, for all practical purposes, "still in the call,
 * still being listened to" after pressing hang-up.
 *
 * Root cause: CallPanel's cleanup() calls `pub.setAttribute('source','')`.
 * The real <moq-publish> (moq publish/src/element.ts) only accepts a
 * `source` of camera / screen / file / null. Its attributeChangedCallback
 * THROWS on an empty string — and crucially it throws *before* updating
 * its internal `source` state. A custom-element reaction exception is
 * reported, not propagated, so cleanup() runs to completion and the
 * element is even removed from the DOM — but because `source` was never
 * cleared, the component never closes its capture sources, and the
 * getUserMedia microphone track stays live until garbage collection.
 *
 * `<moq-publish>` releases the mic only when `source` is cleared with a
 * value it accepts (`removeAttribute('source')` → `null`), or on GC.
 * Setting `muted` does NOT release it — mute keeps the track live so
 * unmute is instant.
 *
 * The bare moq-publish stub in CallPanel.test.tsx models none of this,
 * which is why the bug slipped past the existing suite. This file uses
 * a faithful stub.
 *
 * Fix target: cleanup() must release the publish source through a path
 * the component honours — `pub.removeAttribute('source')` — so the
 * Microphone source is closed and the mic track stopped.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, fireEvent, act, cleanup as rtlCleanup } from '@testing-library/react';
import { useStore } from '../store';
import {
  __setClientForTests,
  __resetAvInstanceForTests,
  joinAvSession,
} from '../irc/client';

vi.mock('../lib/moq-loader', () => ({
  loadMoqComponents: vi.fn(() => Promise.resolve()),
  isMoqLoaded: () => true,
}));
vi.mock('../lib/profiles', () => ({
  getCachedProfile: () => null,
  prefetchProfiles: vi.fn(),
  fetchProfile: vi.fn(),
}));

// ── Faithful <moq-publish> stub ─────────────────────────────────
// Models the real component's `source` contract and microphone
// lifecycle:
//  - source = camera/screen/file → acquires a live mic track
//  - source = null (removeAttribute) → closes capture, stops the track
//  - source = '' (or other) → throws *before* clearing source state, so
//    the capture is NEVER closed (this is the bug)
//  - disconnecting / muting do NOT stop the track (faithful to moq:
//    the connection closes but the capture sources keep running)
class FakeMoqPublish extends HTMLElement {
  static observedAttributes = ['url', 'name', 'muted', 'invisible', 'source'];
  micTrack = {
    stop: vi.fn(),
    readyState: 'inactive' as 'inactive' | 'live' | 'ended',
  };

  attributeChangedCallback(name: string, _old: string | null, value: string | null) {
    if (name !== 'source') return;
    if (value === 'camera' || value === 'screen' || value === 'file') {
      this.micTrack.readyState = 'live'; // source acquired → mic capturing
    } else if (value === null) {
      // removeAttribute('source'): the component clears `source` and
      // closes its capture sources (Source.Microphone.close()).
      if (this.micTrack.readyState === 'live') {
        this.micTrack.readyState = 'ended';
        this.micTrack.stop();
      }
    } else {
      // Empty string etc.: the real component throws here, BEFORE it
      // updates `source` — so the capture is never closed. The reaction
      // exception is reported, not propagated; the caller continues.
      throw new Error(`Invalid source: ${value}`);
    }
  }
  // No disconnectedCallback teardown of the mic — faithful to moq:
  // removing the element stops the broadcast connection, not the
  // getUserMedia capture.
}

if (!customElements.get('moq-publish')) {
  customElements.define('moq-publish', FakeMoqPublish);
}
if (!customElements.get('moq-watch')) {
  customElements.define('moq-watch', class extends HTMLElement {});
}

// ── harness ─────────────────────────────────────────────────────

function makeMediaStream(): MediaStream {
  const tracks = [{ stop: vi.fn(), kind: 'audio' as const }];
  return {
    getTracks: () => tracks,
    getAudioTracks: () => tracks,
    getVideoTracks: () => [],
  } as unknown as MediaStream;
}

function resetStore() {
  useStore.getState().reset();
  useStore.setState({
    nick: 'me',
    avSessions: new Map(),
    activeAvSession: null,
    avAudioActive: false,
    avMuted: false,
    avCameraOn: false,
    authDid: 'did:plc:me',
  });
}

function makeSession(id = 'sess-1') {
  return {
    id,
    channel: '#room',
    createdBy: 'did:plc:me',
    createdByNick: 'me',
    participants: new Map(),
    state: 'active' as const,
    startedAt: new Date(),
  };
}

function setupClient(nick: string) {
  const raw = vi.fn();
  __setClientForTests({ nick, raw } as any);
  return raw;
}

function setupSession() {
  useStore.getState().updateAvSession(makeSession('sess-1'));
  joinAvSession('#room', 'sess-1');
  useStore.getState().setAvAudioActive(true);
}

function mockSessionsApi() {
  vi.spyOn(globalThis, 'fetch').mockImplementation(
    async () =>
      new Response(JSON.stringify({ participants: [] }), { status: 200 }) as any,
  );
}

beforeEach(() => {
  resetStore();
  __resetAvInstanceForTests();
  setupClient('me');
  Object.defineProperty(globalThis.navigator, 'mediaDevices', {
    value: {
      getUserMedia: vi.fn(() => Promise.resolve(makeMediaStream())),
      enumerateDevices: vi.fn(() => Promise.resolve([])),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    },
    configurable: true,
  });
});

afterEach(() => {
  rtlCleanup();
  __setClientForTests(null);
  vi.restoreAllMocks();
});

import { CallPanel } from './CallPanel';

async function flush(times = 4) {
  for (let i = 0; i < times; i++) {
    await act(async () => {
      await Promise.resolve();
    });
  }
}

describe('CallPanel — hang-up must stop the microphone (serious bug)', () => {
  it('hanging up stops the microphone capture — the mic must not keep listening', async () => {
    setupSession();
    mockSessionsApi();

    const { container } = render(<CallPanel />);
    await flush();

    const pub = container.querySelector('moq-publish') as FakeMoqPublish;
    expect(pub).toBeTruthy();
    // The call is live → the publish element holds a live mic track.
    expect(pub.micTrack.readyState).toBe('live');

    const leaveBtn = container.querySelector(
      'button[title="Leave call"]',
    ) as HTMLButtonElement;
    expect(leaveBtn).toBeTruthy();

    await act(async () => {
      fireEvent.click(leaveBtn);
    });

    // The whole point: after hang-up the microphone must be released.
    expect(pub.micTrack.stop).toHaveBeenCalled();
    expect(pub.micTrack.readyState).toBe('ended');
  });

  it('unmounting the call panel stops the microphone capture', async () => {
    setupSession();
    mockSessionsApi();

    const { container, unmount } = render(<CallPanel />);
    await flush();

    const pub = container.querySelector('moq-publish') as FakeMoqPublish;
    expect(pub.micTrack.readyState).toBe('live');

    await act(async () => {
      unmount();
    });

    expect(pub.micTrack.stop).toHaveBeenCalled();
    expect(pub.micTrack.readyState).toBe('ended');
  });

  it('hang-up fully ends the call: panel closed, av-leave sent, mic stopped', async () => {
    const raw = setupClient('me'); // fresh spy for the wire assertion
    setupSession();
    mockSessionsApi();

    const { container } = render(<CallPanel />);
    await flush();
    const pub = container.querySelector('moq-publish') as FakeMoqPublish;

    const leaveBtn = container.querySelector(
      'button[title="Leave call"]',
    ) as HTMLButtonElement;
    await act(async () => {
      fireEvent.click(leaveBtn);
    });

    // These already work today:
    expect(document.querySelector('moq-publish')).toBeNull();
    expect(useStore.getState().avAudioActive).toBe(false);
    const sawLeave = raw.mock.calls.some(([line]: [string]) =>
      String(line).includes('+freeq.at/av-leave'),
    );
    expect(sawLeave).toBe(true);
    // …but this is the serious bug — the mic is still capturing:
    expect(pub.micTrack.stop).toHaveBeenCalled();
  });
});
