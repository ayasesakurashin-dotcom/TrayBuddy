import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import type { BubbleConfig } from '$lib/bubble/BubbleManager.svelte';
const mocks = vi.hoisted(() => ({handlers:new Map<string,(event:{payload:any})=>unknown>(),invoke:vi.fn()}));
vi.mock('@tauri-apps/api/core',()=>({invoke:mocks.invoke}));
vi.mock('@tauri-apps/api/event',()=>({listen:vi.fn(async(name:string,fn:(event:{payload:any})=>unknown)=>{
  mocks.handlers.set(name,fn);return()=>mocks.handlers.delete(name);
})}));
import BubbleManager from '$lib/bubble/BubbleManager.svelte';
import { attachCompanion } from '$lib/companion/presentation';
import { attachCare } from '$lib/care/presentation';

describe('complete companion reply through actual bubble components',()=>{
  let controls:Awaited<ReturnType<typeof attachCompanion>>|undefined;
  let care:Awaited<ReturnType<typeof attachCare>>|undefined;
  beforeEach(()=>{vi.useFakeTimers();mocks.handlers.clear();mocks.invoke.mockReset();mocks.invoke.mockResolvedValue(null);});
  afterEach(()=>{controls?.destroy();controls=undefined;care?.destroy();care=undefined;cleanup();vi.clearAllTimers();vi.useRealTimers();});
  it('automatically displays every page of a voiced reply without manually invoking completion callbacks',async()=>{
    const page=render(BubbleManager);
    await tick();await Promise.resolve();await tick();
    const shown:string[]=[];
    const show=(config:BubbleConfig)=>{shown.push(config.text);page.component.show(config);};
    const audio={stop:vi.fn(),playUrl:vi.fn(async(_url:string,_end?:()=>void,start?:()=>void)=>{start?.();return true;})};
    controls=await attachCompanion({audio:audio as any,show,hide:()=>page.component.hide(),releasePreset:()=>{},allowed:()=>true});
    const text='这是需要完整显示的第一部分。'.repeat(6)+'第二部分应继续出现在新的气泡。'.repeat(6)+'最后一页也必须完整显示。';
    await mocks.handlers.get('companion-reply')?.({payload:{id:'complete',request:1,text,done:true,audio:'prepared'}});
    for(let n=0;n<30;n++){await vi.advanceTimersByTimeAsync(1000);await tick();}
    expect(shown.join('')).toBe(text);
    expect(audio.playUrl).toHaveBeenCalledOnce();
  });
  it('finishes a long reply after voice ends early and a preset close timer would previously compete',async()=>{
    const page=render(BubbleManager);await tick();await Promise.resolve();await tick();
    const shown:string[]=[];
    let finishVoice:(()=>void)|undefined;
    const audio={stop:vi.fn(),playUrl:vi.fn(async(_url:string,end?:()=>void,start?:()=>void)=>{finishVoice=end;start?.();return true;})};
    controls=await attachCompanion({audio:audio as any,show:config=>{shown.push(config.text);expect(config.autoClose).toBe(false);page.component.show(config);},hide:()=>page.component.hide(),releasePreset:()=>{},allowed:()=>true});
    const text='这一段在实际组件中重复出现，仍需逐页显示。'.repeat(18)+'这是必须保留的最后一句。';
    await mocks.handlers.get('companion-reply')?.({payload:{id:'early-end',request:1,text,done:true,audio:'prepared'}});
    await vi.advanceTimersByTimeAsync(1500);finishVoice?.();
    for(let n=0;n<60;n++){await vi.advanceTimersByTimeAsync(1000);await tick();}
    expect(shown.join('')).toBe(text);
    expect(controls.isActive()).toBe(false);
    expect(page.component.isShowing()).toBe(false);
  });
  it('shows all care pages even if its complete speech ends before text pagination finishes',async()=>{
    mocks.invoke.mockImplementation(async command=>command==='get_settings'?{no_audio_mode:false,volume:1}:true);
    const owned={play:vi.fn(async()=>{}),pause:vi.fn(),src:'',volume:1,onplaying:null as null|(()=>void),onended:null as null|(()=>void),onerror:null as null|(()=>void)};
    vi.stubGlobal('Audio',vi.fn(function(){return owned;}));
    try{
      const page=render(BubbleManager);await tick();await Promise.resolve();await tick();
      const shown:string[]=[];
      care=await attachCare({show:config=>{shown.push(config.text);expect(Array.from(config.text).length).toBeLessThanOrEqual(80);page.component.show(config);},hide:()=>page.component.hide(),allowed:()=>true,companionActive:()=>false});
      const text='这是自动关怀需要完整显示的内容。'.repeat(7);
      await mocks.handlers.get('care-offer')?.({payload:{delivery_id:'care',generation:1,interaction_epoch:1,text,expires_at:1234,audio:'prepared'}});
      expect(shown).toHaveLength(0);owned.onplaying?.();
      await vi.advanceTimersByTimeAsync(1000);owned.onended?.();
      for(let n=0;n<20;n++){await vi.advanceTimersByTimeAsync(1000);await tick();}
      expect(shown.join('')).toBe(text);expect(shown.length).toBeGreaterThan(1);
      expect(care.isActive()).toBe(false);
      expect(mocks.invoke.mock.calls.filter(call=>call[0]==='care_presented')).toHaveLength(1);
    }finally{vi.unstubAllGlobals();}
  });
});
