import { describe, it, expect, vi, beforeEach } from 'vitest';
import { isSendKey } from '$lib/companion/types';
const mocks = vi.hoisted(() => ({ handlers: new Map<string, (event: {payload: any}) => unknown>(), invoke: vi.fn(async () => undefined) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async (name: string, fn: (event: {payload: any}) => unknown) => { mocks.handlers.set(name, fn); return () => mocks.handlers.delete(name); }) }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
import { attachCompanion } from '$lib/companion/presentation';

describe('companion input', () => {
  it('never sends an IME confirmation or a Shift+Enter newline', () => {
    const event = { key: 'Enter', shiftKey: false, isComposing: false, keyCode: 13 };
    expect(isSendKey(event)).toBe(true);
    expect(isSendKey({...event, isComposing: true})).toBe(false);
    expect(isSendKey({...event, keyCode: 229})).toBe(false);
    expect(isSendKey({...event, shiftKey: true})).toBe(false);
  });
});
describe('companion presentation', () => {
  beforeEach(() => { mocks.handlers.clear(); mocks.invoke.mockClear(); });
  async function create() {
    const audio = { stop: vi.fn(), playUrl: vi.fn(async (_: string, onEnd?: () => void) => { end = onEnd; return true; }) };
    let end: (() => void) | undefined;
    const show = vi.fn();
    const controls = await attachCompanion({audio: audio as any, show, releasePreset: vi.fn(), allowed: () => true});
    return {audio, show, controls, finish: () => end?.()};
  }
  const dispatch = async (name: string, payload: unknown) => mocks.handlers.get(name)?.({payload});
  it('rejects a cancelled or different reply audio, including a late result', async () => {
    const {audio, controls} = await create();
    await dispatch('companion-reply', {id: 'one', request: 1, text: '今天也辛苦啦。'});
    await dispatch('companion-audio', {id: 'two', request: 2, data: 'invalid'});
    expect(audio.playUrl).not.toHaveBeenCalled();
    await dispatch('companion-stop', null);
    await dispatch('companion-audio', {id: 'one', request: 1, data: 'late'});
    expect(audio.playUrl).not.toHaveBeenCalled();
    expect(controls.isActive()).toBe(false); controls.destroy();
  });
  it.each([
    ['companion-stop', null],
    ['settings-change', {no_audio_mode: true}],
    ['settings-change', {silence_mode: true}],
    ['settings-change', {show_character: false}],
  ])('cancels playback on %s %j and ignores late callbacks and audio', async (name, payload) => {
    const {audio, show, controls, finish} = await create();
    await dispatch('companion-reply', {id: 'one', request: 1, text: '我在这里。'});
    await dispatch('companion-audio', {id: 'one', request: 1, data: 'audio'});
    expect(audio.playUrl).toHaveBeenCalledOnce();
    await dispatch(name, payload);
    expect(audio.stop).toHaveBeenCalledTimes(2);
    const calls = show.mock.calls.length; finish();
    await dispatch('companion-audio', {id: 'one', request: 1, data: 'late'});
    expect(audio.playUrl).toHaveBeenCalledOnce();
    expect(show.mock.calls.length).toBe(calls); expect(controls.isActive()).toBe(false); controls.destroy();
  });
  it('renders generated link labels as plain text', async () => {
    const {show, controls, finish} = await create();
    await dispatch('companion-reply', {id: 'one', request: 1, text: '[看看](javascript:bad)'});
    await dispatch('companion-audio', {id: 'one', request: 1, data: 'audio'});
    finish();
    expect(show.mock.calls.every((call: Array<{text: string}>) => call[0].text === '看看')).toBe(true); controls.destroy();
  });
});
