import {beforeEach,afterEach,describe,it,expect,vi} from 'vitest';
const mocks=vi.hoisted(()=>({handlers:new Map<string,(e:{payload:any})=>unknown>(),invoke:vi.fn()}));
vi.mock('@tauri-apps/api/core',()=>({invoke:mocks.invoke}));
vi.mock('@tauri-apps/api/event',()=>({listen:vi.fn(async(name:string,fn:(e:{payload:any})=>unknown)=>{mocks.handlers.set(name,fn);return()=>mocks.handlers.delete(name);})}));
import {attachCare} from '$lib/care/presentation';
const offer={delivery_id:'delivery',generation:2,interaction_epoch:1,text:'夜间休息提醒',expires_at:1234};
describe('care presentation leases',()=>{
  let care:Awaited<ReturnType<typeof attachCare>>|undefined;
  beforeEach(()=>{vi.useFakeTimers();mocks.handlers.clear();mocks.invoke.mockReset();mocks.invoke.mockResolvedValue(true);});
  afterEach(()=>{care?.destroy();care=undefined;vi.clearAllTimers();vi.useRealTimers();vi.unstubAllGlobals();});
  async function start(companionActive=()=>false){const show=vi.fn(),hide=vi.fn();care=await attachCare({show,hide,allowed:()=>true,companionActive});return{show,hide};}
  it('never presents an invalid or expired offer',async()=>{mocks.invoke.mockResolvedValue(false);const{show}=await start();await mocks.handlers.get('care-offer')?.({payload:offer});expect(show).not.toHaveBeenCalled();expect(mocks.invoke).toHaveBeenCalledWith('care_rejected',{deliveryId:'delivery',generation:2});});
  it('rechecks native activity after asynchronous validation without stopping native audio',async()=>{
    let busy=false;let resolve!:(value:boolean)=>void;
    mocks.invoke.mockImplementation((command:string)=>command==='care_validate_offer'?new Promise(r=>resolve=r):Promise.resolve(true));
    const show=vi.fn(),hide=vi.fn(),releasePreset=vi.fn();
    care=await attachCare({show,hide,releasePreset,allowed:()=>true,companionActive:()=>false,presetActive:()=>busy});
    const pending=mocks.handlers.get('care-offer')?.({payload:offer});busy=true;resolve(true);await pending;
    expect(show).not.toHaveBeenCalled();expect(releasePreset).not.toHaveBeenCalled();expect(hide).not.toHaveBeenCalled();
    expect(mocks.invoke.mock.calls.some(c=>c[0]==='care_presented')).toBe(false);
  });
  it('discards delayed validation after user activity cancelled its generation',async()=>{let resolve!:(v:boolean)=>void;mocks.invoke.mockImplementation((name:string)=>name==='care_validate_offer'?new Promise(r=>resolve=r):Promise.resolve(true));const{show}=await start();const pending=mocks.handlers.get('care-offer')?.({payload:offer});await mocks.handlers.get('care-cancel')?.({payload:{generation:3}});resolve(true);await pending;expect(show).not.toHaveBeenCalled();});
  it('keeps the current conversation bubble when care is rejected or cancelled',async()=>{const{show,hide}=await start(()=>true);await mocks.handlers.get('care-offer')?.({payload:offer});await mocks.handlers.get('care-cancel')?.({payload:{generation:3}});expect(show).not.toHaveBeenCalled();expect(hide).not.toHaveBeenCalled();});
  it('acknowledges only one display and suppresses replayed offers',async()=>{const{show}=await start();await mocks.handlers.get('care-offer')?.({payload:offer});await mocks.handlers.get('care-offer')?.({payload:offer});expect(show).toHaveBeenCalledOnce();expect(show.mock.calls[0][0]).toMatchObject({text:offer.text,plainText:true});expect(mocks.invoke.mock.calls.filter(c=>c[0]==='care_presented')).toHaveLength(1);expect(mocks.invoke.mock.calls.some(c=>/speech|audio|companion_stop/.test(c[0]))).toBe(false);});
  it('drops delayed care audio after a user request cancels the delivery',async()=>{
    const Audio=vi.fn();vi.stubGlobal('Audio',Audio);let finish!:(v:unknown)=>void;
    mocks.invoke.mockImplementation((name:string)=>name==='get_settings'?new Promise(r=>finish=r):Promise.resolve(true));
    await start();
    const pending=mocks.handlers.get('care-offer')?.({payload:{...offer,audio:'audio'}});
    await Promise.resolve();await Promise.resolve();
    await mocks.handlers.get('care-cancel')?.({payload:{generation:3}});finish({no_audio_mode:false,volume:1});await pending;
    expect(Audio).not.toHaveBeenCalled();
  });
  it('stops only the audio it owns when a care delivery is cancelled',async()=>{
    const paused=vi.fn();const owned={play:vi.fn(async()=>{}),pause:paused,src:'',volume:1,onended:null,onerror:null};
    vi.stubGlobal('Audio',vi.fn(function(){return owned;}));mocks.invoke.mockImplementation(async(name:string)=>name==='get_settings'?{no_audio_mode:false,volume:.5}:true);
    await start();await mocks.handlers.get('care-offer')?.({payload:{...offer,audio:'audio'}});
    expect(owned.play).toHaveBeenCalledOnce();await mocks.handlers.get('care-cancel')?.({payload:{generation:3}});expect(paused).toHaveBeenCalledOnce();
    expect(mocks.invoke.mock.calls.some(c=>c[0]==='companion_stop')).toBe(false);
  });

  function pairedAudio(){
    const owned={play:vi.fn(async()=>{}),pause:vi.fn(),src:'',volume:1,onended:null as null|(()=>void),onerror:null as null|(()=>void),onplaying:null as null|(()=>void)};
    vi.stubGlobal('Audio',vi.fn(function(){return owned;}));
    mocks.invoke.mockImplementation(async(name:string)=>name==='get_settings'?{no_audio_mode:false,volume:.5}:true);
    return owned;
  }
  it('shows and acknowledges paired care only when audio starts',async()=>{
    const owned=pairedAudio();const{show}=await start();
    await mocks.handlers.get('care-offer')?.({payload:{...offer,audio:'prepared'}});
    expect(show).not.toHaveBeenCalled();expect(mocks.invoke.mock.calls.some(c=>c[0]==='care_presented')).toBe(false);
    owned.onplaying?.();owned.onplaying?.();await Promise.resolve();
    expect(show).toHaveBeenCalledOnce();expect(mocks.invoke.mock.calls.filter(c=>c[0]==='care_presented')).toHaveLength(1);
    owned.onended?.();expect(show).toHaveBeenCalledOnce();
  });
  it('falls back immediately on playback failure and bounds a stalled decoder',async()=>{
    const owned=pairedAudio();owned.play.mockRejectedValueOnce(new Error('decode failed'));
    const{show}=await start();await mocks.handlers.get('care-offer')?.({payload:{...offer,audio:'bad'}});
    expect(show).toHaveBeenCalledOnce();
    await mocks.handlers.get('care-offer')?.({payload:{...offer,generation:3,audio:'stalled'}});
    expect(show).toHaveBeenCalledOnce();vi.advanceTimersByTime(1000);
    expect(show).toHaveBeenCalledTimes(2);expect(owned.pause).toHaveBeenCalled();
  });
  it('does not display cancelled care when a delayed playing callback arrives',async()=>{
    const owned=pairedAudio();const{show}=await start();
    await mocks.handlers.get('care-offer')?.({payload:{...offer,audio:'prepared'}});
    const lateStart=owned.onplaying;
    await mocks.handlers.get('care-cancel')?.({payload:{generation:3}});lateStart?.();vi.runAllTimers();
    expect(show).not.toHaveBeenCalled();expect(owned.pause).toHaveBeenCalledOnce();
  });
  it('shows text when muted during decoding and suppresses the late audio start',async()=>{
    const owned=pairedAudio();const{show}=await start();
    await mocks.handlers.get('care-offer')?.({payload:{...offer,audio:'prepared'}});
    const lateStart=owned.onplaying;
    await mocks.handlers.get('settings-change')?.({payload:{no_audio_mode:true}});lateStart?.();
    expect(show).toHaveBeenCalledOnce();expect(owned.pause).toHaveBeenCalledOnce();
  });

});
