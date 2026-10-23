<script lang="ts">
  import { onMount } from 'svelte';
  import CareCalendarSettings from './CareCalendarSettings.svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { explain, localTime, type City, type EnvironmentSnapshot, type CareStatus, type CareSettings } from '$lib/environment/types';
  let { diagnostic = false } = $props<{diagnostic?:boolean}>();
  let snapshot=$state<EnvironmentSnapshot|null>(null);
  let care=$state<CareStatus|null>(null);
  let query=$state('');let cities=$state<City[]>([]);let target=$state<'fixed_city'|'fallback_city'>('fixed_city');
  let busy=$state(false);let message=$state('');let showCalendar=$state(false);
  const sourceNames:Record<string,string>={manual_fixed:'固定关注城市',manual_fallback:'备用关注城市',device:'Windows 设备定位',ip_approximate:'根据网络估算（粗略）'};
  async function refresh(){try{const [s,c]=await Promise.all([invoke<EnvironmentSnapshot>('environment_get_snapshot'),invoke<CareStatus>('care_get_status')]);if(!snapshot||s.revision>=snapshot.revision)snapshot=s;care=c;}catch(e){message=String(e);}}
  async function action(work:()=>Promise<unknown>){if(busy)return;busy=true;message='';try{await work();await refresh();}catch(e){message=explain(String(e));}finally{busy=false;}}
  async function saveEnvironment(changes:Record<string,unknown>){if(!snapshot)return;const settings={schema_version:1,calendar_region:'CN',...snapshot.settings,...changes};snapshot=await invoke<EnvironmentSnapshot>('environment_save_settings',{settings});}
  async function saveCare(changes:Partial<CareSettings>){if(!care)return;care=await invoke<CareStatus>('care_save_settings',{settings:{...care.settings,...changes}});}
  function timeInput(minutes:number){return `${Math.floor(minutes/60).toString().padStart(2,'0')}:${(minutes%60).toString().padStart(2,'0')}`;}
  function minutes(value:string){const[h,m]=value.split(':').map(Number);return h*60+m;}
  onMount(()=>{void refresh();let cancelled=false;let cleanup:(()=>void)|undefined;void listen<EnvironmentSnapshot>('environment-updated',event=>{if(event.payload.schema_version!==1)return;if(!snapshot||event.payload.revision>=snapshot.revision)snapshot=event.payload;}).then(fn=>{if(cancelled)fn();else cleanup=fn;});const timer=setInterval(()=>void refresh(),30000);return()=>{cancelled=true;cleanup?.();clearInterval(timer);};});
</script>

<section class="environment-settings" aria-label="环境与自动关怀">
  <h3>环境与自动关怀</h3>
  {#if snapshot}
    <p class="muted datetime-section">本机时间：{snapshot.local_time} · 日历：{snapshot.calendar.coverage==='not_imported'?'尚未导入':snapshot.calendar.region||'待设置'}</p>
    {#if !diagnostic}
      <label><input type="checkbox" checked={snapshot.settings.network_weather_enabled} disabled={busy} onchange={e=>action(()=>saveEnvironment({network_weather_enabled:e.currentTarget.checked}))}/> 允许获取网络天气</label>
      <label>地点模式 <select value={snapshot.settings.location_mode} disabled={busy} onchange={e=>action(()=>saveEnvironment({location_mode:e.currentTarget.value}))}><option value="fixed_city">固定关注城市</option><option value="automatic">自动位置</option></select></label>
      <p>固定城市：{snapshot.settings.fixed_city?.name??'尚未设置'}{#if snapshot.settings.fixed_city} · {snapshot.settings.fixed_city.region} · {snapshot.settings.fixed_city.country}{/if}</p>
      {#if snapshot.settings.location_mode==='automatic'}
        <p>备用城市：{snapshot.settings.fallback_city?.name??'尚未设置'}</p>
        <label><input type="checkbox" checked={snapshot.settings.device_location_enabled} disabled={busy} onchange={e=>{const enabled=e.currentTarget.checked;void action(async()=>{await saveEnvironment({device_location_enabled:enabled});if(enabled)await invoke('environment_request_location_access');});}}/> 允许使用 Windows 设备定位</label>
        <p class="muted">只进行一次读取及有限刷新，不保存精确坐标或移动轨迹。系统权限：{snapshot.location.device_access}</p>
        {#if snapshot.settings.device_location_enabled && snapshot.location.device_access!=='allowed'}<button disabled={busy} onclick={()=>action(()=>invoke('environment_request_location_access'))}>重新申请系统定位权限</button>{/if}
        <label><input type="checkbox" checked={snapshot.settings.ip_fallback_enabled} disabled={busy} onchange={e=>action(()=>saveEnvironment({ip_fallback_enabled:e.currentTarget.checked}))}/> 设备定位和备用城市均不可用时，允许通过 IP 估算城市</label>
        <p class="muted">关闭设备定位后，已开启的 IP 兜底仍可能获取粗略城市。首次估计或城市变化后需要确认。</p>
      {/if}
      <div class="city-search"><label>选择城市 <select bind:value={target}><option value="fixed_city">固定城市</option><option value="fallback_city">备用城市</option></select></label><input aria-label="城市名称" placeholder="输入城市名后查询" bind:value={query} maxlength="80"/><button disabled={busy||query.trim().length<2} onclick={()=>action(async()=>{cities=await invoke<City[]>('environment_search_cities',{query});if(!cities.length)message='没有找到城市，请换一种写法。';})}>查询城市</button></div>
      {#if cities.length}<div class="city-results">{#each cities as city}<button disabled={busy} onclick={()=>action(async()=>{await saveEnvironment({[target]:city});cities=[];})}>{city.name} · {city.region} · {city.country}</button>{/each}</div>{/if}
      <p class="muted">城市查询使用 Open-Meteo 地名目录；天气优先使用 wttr.in，资料无法核验时使用 Open-Meteo 当前天气估算，并标明来源。备用接口仅接收约公里级的天气坐标。选择的城市是关注地点，不代表你实际身在当地。</p>
    {/if}
    <div class="status"><strong>{snapshot.location.city??'地点待设置'}</strong><span>{sourceNames[snapshot.location.source??'']??'位置不可用'}{snapshot.location.confirmed?' · 已核验':' · 未核验'}</span><p>{explain(snapshot.location.reason)}</p>
      {#if snapshot.location.age_seconds!==null}<small>位置更新：{Math.floor(snapshot.location.age_seconds/60)} 分钟前{#if snapshot.location.accuracy_m!==null} · 精度范围约 {Math.ceil(snapshot.location.accuracy_m)} 米{/if}</small>{/if}
      {#if !diagnostic&&snapshot.location.source==='ip_approximate'&&!snapshot.location.confirmed&&snapshot.location.key}<button disabled={busy} onclick={()=>action(()=>invoke('environment_confirm_ip_city',{key:snapshot!.location.key,revision:snapshot!.location.revision}))}>确认此网络估算城市</button>{/if}
    </div>
    <div class="status"><strong>天气资料 · {{fresh:'当前可用',stale:'已过期',unverified:'尚未核验',unavailable:'不可用'}[snapshot.weather.validity]??snapshot.weather.validity}</strong>
      {#if snapshot.weather.data}<p>{snapshot.weather.region??snapshot.location.city??'关注地区'}：{snapshot.weather.data.condition} · {snapshot.weather.data.temperature}°C</p>{/if}
      <p>{explain(snapshot.weather.reason)}</p><small>来源：{snapshot.weather.provider}{#if snapshot.weather.data_kind==='model_current'} · 当前天气估算（非实况观测）{/if}<br/>{snapshot.weather.data_kind==='model_current'?'估算对应时间':'观测时间'}：{localTime(snapshot.weather.valid_at??snapshot.weather.observed_at)}<br/>获取时间：{localTime(snapshot.weather.received_at)}<br/>刷新状态：{snapshot.weather.refresh_state}</small>
      <p class="muted">只有地点匹配、资料时间可靠且未过期的资料才支持天气对话和关怀。天气估算与实况观测会分别标明。</p>
      <div class="actions"><button class="refresh" disabled={busy||!snapshot.settings.network_weather_enabled} onclick={()=>action(()=>invoke('environment_refresh_weather'))}>刷新天气</button><button disabled={busy} onclick={()=>action(()=>invoke('environment_refresh_location'))}>更新位置</button></div>
    </div>
    {#if care&&!diagnostic}<div class="care"><h4>自动关怀</h4><label><input type="checkbox" checked={care.settings.enabled} disabled={busy} onchange={e=>action(()=>saveCare({enabled:e.currentTarget.checked}))}/> 开启自动关怀</label><p class="muted">由触发器提供事实，模型按当前角色人格生成关怀；开启 AI 回复语音时同步合成声音。编辑、聊天和阅读回复优先。开启后接管原角色关怀；关闭时两套关怀均关闭。</p>
      <div class="checks">{#each [{key:'rain',label:'天气现象（雨、雪、雾、沙尘等）'},{key:'night',label:'晚间休息'},{key:'holiday',label:'导入日历的节日祝福'},{key:'legacy_weather',label:'冷热关怀'},{key:'special_dates',label:'特殊日期与纪念日'}] as item}<label><input type="checkbox" checked={care.settings[item.key as keyof CareSettings]===true} disabled={busy} onchange={e=>action(()=>saveCare({[item.key]:e.currentTarget.checked}))}/>{item.label}</label>{/each}</div>
      <label>晚间时段 <input type="time" value={timeInput(care.settings.night_start)} disabled={busy} onchange={e=>action(()=>saveCare({night_start:minutes(e.currentTarget.value)}))}/> 至 <input type="time" value={timeInput(care.settings.night_end)} disabled={busy} onchange={e=>action(()=>saveCare({night_end:minutes(e.currentTarget.value)}))}/></label><p class="muted">支持跨午夜；每个夜间一次。所有关怀至少间隔 45 分钟；天气、节日、晚间、特殊日期四类各自滚动 24 小时最多 2 条。特殊日期优先，其余随机选择。</p><div class="checks"><label>偏冷阈值 ≤ <input aria-label="偏冷温度阈值" type="number" value={care.settings.cold_threshold??5} min="-60" max="59" disabled={busy} onchange={e=>action(()=>saveCare({cold_threshold:Number(e.currentTarget.value)}))}/> °C</label><label>偏热阈值 ≥ <input aria-label="偏热温度阈值" type="number" value={care.settings.hot_threshold??30} min="-59" max="60" disabled={busy} onchange={e=>action(()=>saveCare({hot_threshold:Number(e.currentTarget.value)}))}/> °C</label></div>
      <label><input type="checkbox" checked={care.settings.solar_terms??false} disabled={busy} onchange={e=>action(()=>saveCare({solar_terms:e.currentTarget.checked}))}/> 日历已导入时，也启用二十四节气关怀</label>
      <label><input type="checkbox" checked={care.settings.capture_dates??true} disabled={busy} onchange={e=>action(()=>saveCare({capture_dates:e.currentTarget.checked}))}/> 从对话中保存明确自述的特殊日期</label>
      <p class="muted">明确自述会自动保存并告知；含糊或冲突时先询问。已显示的关怀加入聊天历史，不自动写入长期记忆；生日等用户自述事实遵循现有长期记忆开关。天气资料须包含可核验依据，来源未提供的沙尘等现象不会猜测。</p>
      <button onclick={()=>showCalendar=!showCalendar}>{showCalendar?'收起日历设置':'日历设置'}</button>{#if showCalendar}<CareCalendarSettings />{/if}
      <p>{care.reason??'等待合适的关怀时机'}</p>
    </div>{/if}
  {:else}<p>正在读取环境设置…</p>{/if}
  {#if message}<p class="message" role="status">{message}</p>{/if}
</section>
<style>
  .environment-settings{display:grid;gap:12px;padding:16px;border:1px solid #dce1e8;border-radius:12px;color:inherit;background:var(--bg-primary,transparent)}h3,h4,p{margin:0}h4{font-size:15px}.muted,small{font-size:12px;line-height:1.65;opacity:.72}label{display:flex;gap:8px;align-items:center;flex-wrap:wrap;font-size:13px}input,select,button{font:inherit}input:not([type=checkbox]),select{border:1px solid #b8c3d0;border-radius:6px;padding:6px;max-width:100%;color:inherit;background:transparent}button{border:1px solid #a9b8c9;border-radius:7px;padding:7px 10px;background:transparent;color:inherit;cursor:pointer}button:disabled{opacity:.5;cursor:default}.status,.care{border-top:1px solid #dce1e8;padding-top:12px;display:grid;gap:8px}.status span{font-size:12px}.city-search,.actions,.checks{display:flex;gap:8px;flex-wrap:wrap}.city-results{display:grid;gap:6px;max-height:180px;overflow:auto}.message{color:#b45309;font-size:13px}.care{gap:12px}
</style>
