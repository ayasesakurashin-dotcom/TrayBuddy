import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { isSendKey } from '$lib/companion/types';
import type { BubbleConfig } from '$lib/bubble/BubbleManager.svelte';
import { REPLY_READ_HOLD_MS, REPLY_STREAM_FLUSH_MS } from '$lib/companion/replyChunks';
const mocks = vi.hoisted(() => ({ handlers: new Map<string, (event: {payload: any}) => unknown>(), invoke: vi.fn(async (_command: string, _args?: unknown) => undefined) }));
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
  const cleanup: Array<() => void> = [];
  beforeEach(() => { vi.useFakeTimers(); mocks.handlers.clear(); mocks.invoke.mockClear(); });
  afterEach(() => { cleanup.splice(0).forEach(fn => fn()); vi.useRealTimers(); });
  async function create() {
    const ends: Array<(() => void) | undefined> = [];
    const starts: Array<(() => void) | undefined> = [];
    const audio = { stop: vi.fn(), playUrl: vi.fn(async (_: string, onEnd?: () => void, onStart?: () => void) => { ends.push(onEnd); starts.push(onStart); return true; }) };
    const show = vi.fn<(config: BubbleConfig) => void>();
    const hide = vi.fn();
    const releasePreset = vi.fn();
    const controls = await attachCompanion({audio: audio as any, show, hide, releasePreset, allowed: () => true});
    cleanup.push(controls.destroy);
    return {audio, show, hide, releasePreset, controls, start: (index = starts.length - 1) => starts[index]?.(), finish: (index = ends.length - 1) => ends[index]?.(), complete: () => show.mock.lastCall?.[0].onTextComplete?.()};
  }
  const dispatch = async (name: string, payload: unknown) => mocks.handlers.get(name)?.({payload});

  it('starts paired text on actual speech playback and never repeats it on duplicate packets', async () => {
    const {show, audio, start, finish, controls} = await create();
    const payload = {id:'paired',request:1,text:'一起开始的回复。',done:true,audio:'prepared'};
    await dispatch('companion-reply', payload);
    expect(show).not.toHaveBeenCalled();
    expect(controls.isActive()).toBe(true);
    expect(audio.playUrl).toHaveBeenCalledOnce();
    start(); start();
    expect(show.mock.lastCall?.[0].text).toBe(payload.text);
    await dispatch('companion-reply', payload);
    finish();
    expect(show).toHaveBeenCalledOnce();
    expect(audio.playUrl).toHaveBeenCalledOnce();
  });

  it('shows prepared text immediately if local playback fails', async () => {
    const {audio, show, controls} = await create();
    audio.playUrl.mockRejectedValueOnce(new Error('decoder failed'));
    await dispatch('companion-reply', {id:'failed',request:1,text:'语音失败也显示正文。',done:true,audio:'bad'});
    expect(show.mock.lastCall?.[0].text).toBe('语音失败也显示正文。');
    expect(controls.isActive()).toBe(true);
  });

  it('releases text if playback stalls or the user mutes while waiting', async () => {
    const {show, audio, start} = await create();
    await dispatch('companion-reply', {id:'stalled',request:1,text:'超时显示。',done:true,audio:'stalled'});
    expect(show).not.toHaveBeenCalled();
    vi.advanceTimersByTime(5000);
    expect(show.mock.lastCall?.[0].text).toBe('超时显示。');
    start(); expect(show).toHaveBeenCalledOnce();
    await dispatch('companion-reply', {id:'muted',request:2,text:'静音后显示。',done:true,audio:'wait'});
    expect(show).toHaveBeenCalledOnce();
    await dispatch('settings-change', {no_audio_mode:true});
    expect(show.mock.lastCall?.[0].text).toBe('静音后显示。');
    expect(audio.stop).toHaveBeenCalled();
    start(); expect(show).toHaveBeenCalledTimes(2);
  });

  it('does not resurrect paired text when stopped before audio starts', async () => {
    const {show, start, finish, controls} = await create();
    await dispatch('companion-reply', {id:'cancelled',request:1,text:'不应出现。',done:true,audio:'wait'});
    await dispatch('companion-stop', null);
    start(); finish(); vi.runAllTimers();
    expect(show).not.toHaveBeenCalled();
    expect(controls.isActive()).toBe(false);
  });

  it('does not flash buffered text when one settings update both mutes and hides the pet', async () => {
    const {show, start, controls} = await create();
    await dispatch('companion-reply', {id:'hidden',request:1,text:'不应显示。',done:true,audio:'wait'});
    await dispatch('settings-change', {no_audio_mode:true,show_character:false});
    start(); vi.runAllTimers();
    expect(show).not.toHaveBeenCalled();
    expect(controls.isActive()).toBe(false);
  });

  it('holds every page for four seconds after typing, with no replay on final text or TTS events', async () => {
    const {audio, show, controls, complete, finish} = await create();
    const text = '今'.repeat(175);
    await dispatch('companion-reply', {id: 'one', request: 1, text, done: true});
    expect(show.mock.calls.map(call => call[0].text)).toEqual(['今'.repeat(80)]);
    expect(show.mock.calls[0][0]).toMatchObject({duration: 4000, typeSpeed: 16, plainText: true});
    vi.advanceTimersByTime(10000); // Still typing: do not count this as reading time.
    expect(show).toHaveBeenCalledOnce();
    complete(); complete(); // Duplicate complete signals must not create two queue timers.
    vi.advanceTimersByTime(REPLY_READ_HOLD_MS - 1);
    expect(show).toHaveBeenCalledOnce();
    vi.advanceTimersByTime(1);
    expect(show).toHaveBeenCalledTimes(2);
    await dispatch('companion-audio', {id: 'one', request: 1, data: 'audio'});
    await dispatch('companion-audio', {id: 'one', request: 1, data: 'duplicate'});
    finish(); finish();
    await dispatch('companion-reply', {id: 'one', request: 1, text, done: true});
    expect(audio.playUrl).toHaveBeenCalledOnce();
    expect(show).toHaveBeenCalledTimes(2);
    complete(); vi.advanceTimersByTime(REPLY_READ_HOLD_MS);
    expect(show.mock.calls.map(call => call[0].text).join('')).toBe(text);
    expect(controls.isActive()).toBe(true);
    complete(); vi.advanceTimersByTime(REPLY_READ_HOLD_MS);
    expect(controls.isActive()).toBe(false);
  });

  it('starts an unfinished stream early, appends only the unseen tail, and protects preset bubbles until finished', async () => {
    const {show, releasePreset, controls, complete} = await create();
    const first = '这是正在生成的一段文字';
    const rest = '，后续内容继续抵达。'.repeat(16) + '这是最后的尾句。';
    await dispatch('companion-reply', {id: 'one', request: 1, text: first, done: false});
    expect(show).not.toHaveBeenCalled();
    expect(controls.isActive()).toBe(true);
    vi.advanceTimersByTime(REPLY_STREAM_FLUSH_MS);
    expect(show.mock.calls[0][0].text).toBe(first);
    complete(); vi.advanceTimersByTime(REPLY_READ_HOLD_MS);
    expect(controls.isActive()).toBe(true); // Model is still generating.
    await dispatch('companion-reply', {id: 'one', request: 1, text: first + rest.slice(0, 85), done: false});
    await dispatch('companion-reply', {id: 'one', request: 1, text: first + rest, done: false});
    await dispatch('companion-reply', {id: 'one', request: 1, text: first + rest, done: true});
    await dispatch('companion-reply', {id: 'one', request: 1, text: first + rest, done: true});
    for (let page = 0; page < 20 && controls.isActive(); page++) { complete(); vi.advanceTimersByTime(REPLY_READ_HOLD_MS); }
    expect(controls.isActive()).toBe(false);
    expect(show.mock.calls.map(call => call[0].text).join('')).toBe(first + rest);
    expect(releasePreset).toHaveBeenCalledOnce();
  });

  it('flushes the terminal partial tail after a stream error and releases its reading lock', async () => {
    const {show, controls, complete} = await create();
    const text = '已生成的内容应该保留';
    await dispatch('companion-reply', {id: 'one', request: 1, text, done: false});
    await dispatch('companion-reply', {id: 'one', request: 1, text, done: true, error: 'connection reset'});
    expect(show.mock.lastCall?.[0].text).toBe(text);
    complete(); vi.advanceTimersByTime(REPLY_READ_HOLD_MS);
    expect(controls.isActive()).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('keeps a tiny search prefix buffered past the flush timer and shows the complete answer once', async () => {
    const {show, controls, complete} = await create();
    const text = '查询资料显示，目前没有公布明确的发售日期，后续仍需要以官方公告为准。';
    await dispatch('companion-reply', {id:'search',request:1,text:'查询',done:false});
    vi.advanceTimersByTime(REPLY_STREAM_FLUSH_MS);
    expect(show).not.toHaveBeenCalled();
    expect(controls.isActive()).toBe(true);
    expect(vi.getTimerCount()).toBe(0); // Wait for data, not an endless flush loop.
    await dispatch('companion-reply', {id:'search',request:1,text,done:false});
    await dispatch('companion-reply', {id:'search',request:1,text,done:true});
    expect(show.mock.calls.map(call=>call[0].text)).toEqual([text]);
    complete();vi.advanceTimersByTime(REPLY_READ_HOLD_MS);
    expect(controls.isActive()).toBe(false);
  });

  it('shows a genuinely short terminal answer after buffering its partial prefix', async () => {
    const {show, controls, complete} = await create();
    await dispatch('companion-reply', {id:'short',request:1,text:'好的',done:false});
    vi.advanceTimersByTime(REPLY_STREAM_FLUSH_MS);
    expect(show).not.toHaveBeenCalled();
    await dispatch('companion-reply', {id:'short',request:1,text:'好的',done:true});
    expect(show.mock.lastCall?.[0].text).toBe('好的');
    complete();vi.advanceTimersByTime(REPLY_READ_HOLD_MS);
    expect(controls.isActive()).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('accepts late TTS after reading completes without showing the response again', async () => {
    const {audio, show, controls, complete, finish} = await create();
    await dispatch('companion-reply', {id: 'one', request: 1, text: '我在这里。', done: true});
    complete(); vi.advanceTimersByTime(REPLY_READ_HOLD_MS);
    expect(controls.isActive()).toBe(false);
    await dispatch('companion-audio', {id: 'one', request: 1, data: 'audio'});
    expect(controls.isActive()).toBe(true);
    finish();
    expect(controls.isActive()).toBe(false);
    expect(show).toHaveBeenCalledOnce();
    expect(audio.playUrl).toHaveBeenCalledOnce();
  });
  it('renews a request-tagged reading lease for each page and releases it only after the tail', async () => {
    const {controls, complete} = await create();
    await dispatch('companion-reply', {id: 'long', request: 7, text: '字'.repeat(90), done: true});
    expect(mocks.invoke).toHaveBeenCalledWith('companion_reading', {request: 7, active: true});
    complete(); vi.advanceTimersByTime(REPLY_READ_HOLD_MS);
    expect(mocks.invoke.mock.calls.filter(call => call[0] === 'companion_reading')).toEqual([
      ['companion_reading', {request: 7, active: true}],
      ['companion_reading', {request: 7, active: true}],
    ]);
    complete(); vi.advanceTimersByTime(REPLY_READ_HOLD_MS);
    expect(mocks.invoke).toHaveBeenLastCalledWith('companion_reading', {request: 7, active: false});
    expect(controls.isActive()).toBe(false);
  });
  it('keeps automatic observations deferred during long audio after text finishes', async () => {
    const {complete, finish} = await create();
    await dispatch('companion-reply', {id: 'long-audio', request: 9, text: '短文字，长语音。', done: true});
    await dispatch('companion-audio', {id: 'long-audio', request: 9, data: 'audio'});
    complete(); vi.advanceTimersByTime(35000);
    expect(mocks.invoke).toHaveBeenLastCalledWith('companion_reading', {request: 9, active: true});
    finish();
    expect(mocks.invoke).toHaveBeenCalledWith('companion_reading', {request: 9, active: false});
    expect(vi.getTimerCount()).toBe(0);
  });

  it('rejects a cancelled or different reply audio, including a late result', async () => {
    const {audio, controls, show} = await create();
    await dispatch('companion-reply', {id: 'one', request: 1, text: '今天也辛苦啦。'});
    await dispatch('companion-audio', {id: 'two', request: 2, data: 'invalid'});
    expect(audio.playUrl).not.toHaveBeenCalled();
    await dispatch('companion-stop', null);
    await dispatch('companion-audio', {id: 'one', request: 1, data: 'late'});
    await dispatch('companion-reply', {id: 'one', request: 1, text: '今天也辛苦啦。迟到的尾句', done: true});
    expect(audio.playUrl).not.toHaveBeenCalled();
    expect(show).toHaveBeenCalledOnce();
    expect(controls.isActive()).toBe(false);
  });
  it.each([
    ['companion-stop', null],
    ['settings-change', {silence_mode: true}],
    ['settings-change', {show_character: false}],
  ])('cancels playback on %s %j and ignores late callbacks and audio', async (name, payload) => {
    const {audio, show, controls, finish, complete, hide} = await create();
    await dispatch('companion-reply', {id: 'one', request: 1, text: '我在这里。'});
    await dispatch('companion-audio', {id: 'one', request: 1, data: 'audio'});
    expect(audio.playUrl).toHaveBeenCalledOnce();
    await dispatch(name, payload);
    expect(audio.stop).toHaveBeenCalledTimes(2);
    const calls = show.mock.calls.length; finish(); complete(); vi.runAllTimers();
    await dispatch('companion-audio', {id: 'one', request: 1, data: 'late'});
    expect(audio.playUrl).toHaveBeenCalledOnce();
    expect(show.mock.calls.length).toBe(calls); expect(controls.isActive()).toBe(false);
    expect(hide).toHaveBeenCalled(); expect(vi.getTimerCount()).toBe(0);
  });

  it('mutes audio while the remaining text keeps advancing', async () => {
    const {audio, show, controls, finish, complete} = await create();
    await dispatch('companion-reply', {id: 'one', request: 1, text: '我'.repeat(90), done: true});
    await dispatch('companion-audio', {id: 'one', request: 1, data: 'audio'});
    await dispatch('settings-change', {no_audio_mode: true});
    expect(audio.stop).toHaveBeenCalledTimes(2);
    finish(); complete(); vi.advanceTimersByTime(REPLY_READ_HOLD_MS);
    expect(show).toHaveBeenCalledTimes(2);
    expect(controls.isActive()).toBe(true);
    await dispatch('companion-audio', {id: 'one', request: 1, data: 'late'});
    expect(audio.playUrl).toHaveBeenCalledOnce();
    complete(); vi.advanceTimersByTime(REPLY_READ_HOLD_MS);
    expect(controls.isActive()).toBe(false);
  });

  it('ignores stale callbacks and older events when a new reply takes over', async () => {
    const {audio, show, controls, complete, finish} = await create();
    await dispatch('companion-reply', {id: 'one', request: 1, text: '旧'.repeat(90), done: true});
    const oldComplete = show.mock.lastCall?.[0].onTextComplete;
    await dispatch('companion-audio', {id: 'one', request: 1, data: 'audio'});
    await dispatch('companion-reply', {id: 'two', request: 2, text: '新回复', done: true});
    const calls = show.mock.calls.length;
    oldComplete?.(); finish(0);
    await dispatch('companion-reply', {id: 'one', request: 1, text: '旧最终回复', done: true});
    await dispatch('companion-audio', {id: 'one', request: 1, data: 'late'});
    expect(show).toHaveBeenCalledTimes(calls);
    expect(audio.playUrl).toHaveBeenCalledOnce();
    complete(); vi.advanceTimersByTime(REPLY_READ_HOLD_MS);
    expect(controls.isActive()).toBe(false);
  });

  it('continues reading when audio playback rejects', async () => {
    const {audio, show, controls, complete} = await create();
    audio.playUrl.mockRejectedValueOnce(new Error('audio unavailable'));
    await dispatch('companion-reply', {id: 'one', request: 1, text: '文'.repeat(90), done: true});
    await dispatch('companion-audio', {id: 'one', request: 1, data: 'audio'});
    complete(); vi.advanceTimersByTime(REPLY_READ_HOLD_MS);
    expect(show).toHaveBeenCalledTimes(2);
    complete(); vi.advanceTimersByTime(REPLY_READ_HOLD_MS);
    expect(controls.isActive()).toBe(false);
    expect(mocks.invoke).toHaveBeenCalledWith('companion_playback_finished');
  });

  it('cleans up listeners and both pending stream and reading timers on destruction', async () => {
    const {show, controls} = await create();
    await dispatch('companion-reply', {id: 'one', request: 1, text: '尚未完成的句子', done: false});
    expect(vi.getTimerCount()).toBe(1);
    controls.destroy();
    expect(mocks.handlers.size).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
    vi.runAllTimers();
    expect(show).not.toHaveBeenCalled();
    expect(controls.isActive()).toBe(false);
  });

  it('never starts a queue for a cancelled generation whose first packet had not arrived', async () => {
    const {show, controls} = await create();
    await dispatch('companion-stop', null);
    await dispatch('companion-reply', {id: 'unseen', request: 1, text: '迟到的取消结果', done: true, error: '已停止本次回复'});
    expect(show).not.toHaveBeenCalled();
    expect(controls.isActive()).toBe(false);
  });

  it('renders generated link labels as plain text', async () => {
    const {show, controls, finish} = await create();
    await dispatch('companion-reply', {id: 'one', request: 1, text: '[看看](javascript:bad)'});
    await dispatch('companion-audio', {id: 'one', request: 1, data: 'audio'});
    finish();
    expect(show.mock.calls.map(call => call[0].text)).toEqual(['看看']);
  });
});
