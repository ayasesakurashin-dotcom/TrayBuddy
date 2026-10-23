import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { WindowCore, WindowCoreBindings } from '$lib/animation/WindowCore';
import type { BubbleConfig } from '$lib/bubble/BubbleManager.svelte';

const mocks = vi.hoisted(() => ({
  handlers: new Map<string, Array<(event: {payload: any}) => unknown>>(),
  invoke: vi.fn<(command: string, args?: unknown) => Promise<unknown>>(),
  audio: {stop: vi.fn(), play: vi.fn(async () => true), playUrl: vi.fn(async () => true), destroy: vi.fn()},
  window: {setPosition: vi.fn(), scaleFactor: vi.fn(async () => 1), outerPosition: vi.fn(async () => ({x: 0, y: 0}))},
}));

// Exercise the real runtime and companion reader; replace only its host APIs.
vi.unmock('$lib/animation/WindowCore');
vi.mock('@tauri-apps/api/core', () => ({invoke: mocks.invoke}));
vi.mock('@tauri-apps/api/event', () => ({
  emit: vi.fn(async () => {}),
  listen: vi.fn(async (name: string, handler: (event: {payload: any}) => unknown) => {
    mocks.handlers.set(name, [...(mocks.handlers.get(name) ?? []), handler]);
    return () => mocks.handlers.set(name, (mocks.handlers.get(name) ?? []).filter(fn => fn !== handler));
  }),
}));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => mocks.window,
  cursorPosition: async () => ({x: 0, y: 0}),
  LogicalPosition: class { constructor(public x: number, public y: number) {} },
}));
vi.mock('$lib/i18n', () => ({
  initI18n: async () => {}, destroyI18n: () => {}, onLangChange: () => () => {}, t: (key: string) => key,
}));
vi.mock('$lib/audio/AudioManager', () => ({
  getAudioManager: async () => mocks.audio, clearAudioCache: () => {}, resetAudioManagerInstance: () => {},
}));
vi.mock('$lib/trigger/TriggerManager', () => ({
  getTriggerManager: () => ({destroy: () => {}}), resetTriggerManagerInstance: () => {},
}));

import { createWindowCore } from '$lib/animation/WindowCore';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return {promise, resolve};
}

const dispatch = async (name: string, payload: unknown) => {
  await Promise.all((mocks.handlers.get(name) ?? []).map(handler => handler({payload})));
};

describe('WindowCore companion bubble ownership', () => {
  let core: WindowCore | undefined;
  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
    mocks.audio.play.mockImplementation(async () => true);
    mocks.handlers.clear();
    mocks.invoke.mockImplementation(async command => {
      switch (command) {
        case 'get_settings': return {show_character: true, show_border: false, silence_mode: false, animation_scale: 1, lang: 'zh', nickname: '用户'};
        case 'get_saved_window_position': return [null, null];
        case 'get_usage_stats': return {first_login: null, total_usage_seconds: 0, session_uptime_seconds: 0};
        default: return null;
      }
    });
  });
  afterEach(() => { core?.destroy(); core = undefined; vi.clearAllTimers(); vi.useRealTimers(); });

  async function init() {
    const show = vi.fn<(config: BubbleConfig) => void>();
    const hide = vi.fn();
    // BubbleManager reports false while replacing one bubble with the next.
    const manager = {show, hide, isShowing: vi.fn(() => false)};
    const values = {
      getShowCharacter: () => true, getSilenceMode: () => false, getShowModDataPanel: () => false,
      getShowBorder: () => false, getUserNickname: () => '用户',
    };
    const bindings = new Proxy(values, {
      get: (target, key) => Reflect.get(target, key) ?? vi.fn(),
    }) as unknown as WindowCoreBindings;
    core = createWindowCore({
      bindings,
      refs: {getCharacterCanvas: () => null, getBorderCanvas: () => null, getBubbleManager: () => manager as any},
      callbacks: {playAnimation: vi.fn(async () => true)},
    });
    await core.init();
    expect(mocks.handlers.get('state-change')).toHaveLength(1);
    expect(mocks.handlers.get('companion-reply')).toHaveLength(1);
    return {show, hide, manager};
  }

  function nativeResources() {
    const original=mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation(async(command,args)=>{
      if(command==='get_text_by_name')return {text:`原有活动：${(args as {name:string}).name}`,duration:3};
      if(command==='care_validate_offer'||command==='care_presented')return true;
      return original(command,args);
    });
  }
  const careOffer=(generation:number)=>({delivery_id:`care-${generation}`,generation,interaction_epoch:1,text:'自动关怀正文',expires_at:9999});
  const nativeState=(name:string)=>({state:{name,persistent:false,anima:'',audio:name,text:name,priority:0},play_once:true});

  it.each(['night','random1_1'])('keeps native %s text/audio and rejects care until the activity finishes',async name=>{
    const {show}=await init();nativeResources();
    let finishAudio:(()=>void)|undefined;
    mocks.audio.play.mockImplementation(async(_name?:unknown,end?:()=>void)=>{finishAudio=end;return true;});
    await dispatch('state-change',nativeState(name));
    expect(mocks.audio.play).toHaveBeenCalledWith(name,expect.any(Function));
    expect(show.mock.lastCall?.[0].text).toBe(`原有活动：${name}`);
    await dispatch('care-offer',careOffer(10));
    expect(show).toHaveBeenCalledOnce();
    expect(mocks.invoke.mock.calls.some(call=>call[0]==='care_presented')).toBe(false);
    finishAudio?.();core!.handleBubbleClose();
    await dispatch('care-offer',careOffer(11));
    expect(show.mock.lastCall?.[0].text).toBe('自动关怀正文');
    expect(mocks.invoke.mock.calls.filter(call=>call[0]==='care_presented')).toHaveLength(1);
    expect(mocks.invoke.mock.calls.some(call=>['companion_send','companion_observe','companion_reading'].includes(call[0]))).toBe(false);
  });

  it('lets a native activity preempt care without late cancellation hiding its bubble',async()=>{
    const {show,hide}=await init();nativeResources();
    await dispatch('care-offer',careOffer(1));
    expect(show.mock.lastCall?.[0].text).toBe('自动关怀正文');
    await dispatch('state-change',nativeState('random4_1'));
    expect(mocks.audio.play).toHaveBeenCalledWith('random4_1',expect.any(Function));
    expect(show.mock.lastCall?.[0].text).toBe('原有活动：random4_1');
    const hides=hide.mock.calls.length;
    await dispatch('care-cancel',{generation:2});
    expect(hide).toHaveBeenCalledTimes(hides);
    await dispatch('care-offer',careOffer(3));
    expect(show).toHaveBeenCalledTimes(2);
    // Only the genuine care shown before the event was acknowledged.
    expect(mocks.invoke.mock.calls.filter(call=>call[0]==='care_presented')).toHaveLength(1);
    expect(mocks.invoke).toHaveBeenCalledWith('care_finished',{deliveryId:'care-1',generation:1});
  });

  it.each([true, false])('does not overwrite or hide AI text when a pending preset resolves (preset bubble=%s)', async branch_show_bubble => {
    const {show, hide} = await init();
    const requested = deferred<void>();
    const text = deferred<{text: string; duration: number}>();
    const original = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation(async (command, args) => {
      if (command === 'get_text_by_name') { requested.resolve(); return text.promise; }
      return original(command, args);
    });
    const preset = dispatch('state-change', {
      state: {name: 'preset', persistent: false, anima: '', audio: '', text: 'greeting', priority: 0, branch_show_bubble},
      play_once: true,
    });
    await requested.promise;
    await dispatch('companion-reply', {id: 'reply', request: 1, text: 'AI 回复仍然需要保持完整的阅读时间。', done: false});
    expect(show.mock.calls.map(call => call[0].text)).toEqual(['AI 回复仍然需要保持完整的阅读时间。']);
    const hidesBeforePresetCompletes = hide.mock.calls.length;
    text.resolve({text: '迟到的预设气泡', duration: 3});
    await preset;
    expect(show).toHaveBeenCalledOnce();
    expect(hide).toHaveBeenCalledTimes(hidesBeforePresetCompletes);
  });

  it('ignores AI tool activation during a page switch and resumes tool notices after reading finishes', async () => {
    const {show, manager} = await init();
    await dispatch('companion-reply', {id: 'reply', request: 1, text: '读'.repeat(90), done: true});
    show.mock.lastCall?.[0].onTextComplete?.();
    await vi.advanceTimersByTimeAsync(4000);
    expect(show).toHaveBeenCalledTimes(2);
    expect(manager.isShowing()).toBe(false); // The replacement bubble is in its switch gap.
    await dispatch('ai-tool-activated', {window_name: 'Editor', model: 'model'});
    expect(show).toHaveBeenCalledTimes(2);
    show.mock.lastCall?.[0].onTextComplete?.();
    await vi.advanceTimersByTimeAsync(4000);
    await dispatch('ai-tool-activated', {window_name: 'Editor', model: 'model'});
    expect(show).toHaveBeenCalledTimes(3);
    expect(show.mock.lastCall?.[0].text).toContain('Editor');
  });
});
