<script lang="ts">
  import {onMount} from 'svelte';
  import {invoke} from '@tauri-apps/api/core';
  import {listen} from '@tauri-apps/api/event';
  import type {CalendarData, SpecialDate} from '$lib/environment/types';
  let data=$state<CalendarData|null>(null), busy=$state(false), message=$state('');
  let country=$state('CN'), firstYear=$state(new Date().getFullYear()), lastYear=$state(new Date().getFullYear()+1);
  const empty=():SpecialDate=>({id:'',name:'',kind:'birthday',calendar:'gregorian',year:new Date().getFullYear(),month:1,day:1,yearly:true,leap_month:false,enabled:true,source:'manual'});
  let form=$state<SpecialDate>(empty());
  async function load(){data=await invoke<CalendarData>('care_calendar_get');}
  async function action(work:()=>Promise<unknown>){if(busy)return;busy=true;message='';try{await work();await load();}catch(e){message=String(e);}finally{busy=false;}}
  async function save(){await invoke('care_special_save',{date:{...$state.snapshot(form),year:form.yearly&&form.calendar==='gregorian'?null:form.year,leap_month:form.calendar==='lunar'&&form.leap_month,source:'manual'}});form=empty();message='已保存特殊日期';}
  onMount(()=>{void load().catch(e=>message=String(e));let disposed=false;let unlisten:(()=>void)|undefined;void listen('care-calendar-changed',()=>void load().catch(e=>message=String(e))).then(fn=>{if(disposed)fn();else unlisten=fn;});return()=>{disposed=true;unlisten?.();};});
</script>

<section aria-label="日历设置">
  <h4>日历设置</h4>
  <p>未导入日历时不触发节日关怀。导入的日期保存在本机，以后无需联网；个人特殊日期可独立使用。</p>
  <div class="row">
    <label>国家代码<input aria-label="日历国家代码" bind:value={country} maxlength="2" list="care-countries" /></label>
    <datalist id="care-countries"><option value="CN">中国</option><option value="US">美国</option><option value="JP">日本</option><option value="GB">英国</option><option value="DE">德国</option><option value="FR">法国</option><option value="AU">澳大利亚</option><option value="CA">加拿大</option></datalist>
    <label>起始年份<input aria-label="日历起始年份" type="number" bind:value={firstYear} min="1901" max="2100" /></label>
    <label>结束年份<input aria-label="日历结束年份" type="number" bind:value={lastYear} min={firstYear} max={Math.min(firstYear+4,2100)} /></label>
    <button disabled={busy} onclick={()=>action(async()=>{await invoke('care_calendar_import',{country,firstYear,lastYear});message='日历已导入并保存在本机';})}>{busy?'正在处理…':'联网导入日历'}</button>
  </div>
  <p>中国使用香港天文台公农历对照表，包含主要传统节日及节气；其他国家使用 Nager.Holidays 全国公众节日。部分国家或年份可能不受来源支持。调休上班日和普通周末不触发祝福；不推断用户是否放假。</p>
  {#if data}
    {#if !data.datasets.length}<p class="empty">尚未导入日历，节日关怀暂不启动。</p>{/if}
    {#each data.datasets as set (set.id)}
      <details><summary>{set.country} · {set.year} 年 · {set.events.length} 条节日/节气{set.lunar_days.length?' · 含农历对照':''}</summary>
        <p>导入时间：{new Date(set.imported_at).toLocaleString()}</p><p>来源：<a href={set.source} target="_blank" rel="noreferrer">查看资料来源</a></p>
        <div class="events">{#each set.events as event}<span>{event.date} · {event.name}{event.kind==='term'?'（节气）':''}</span>{/each}</div>
        <button disabled={busy} onclick={()=>action(()=>invoke('care_calendar_remove',{id:set.id}))}>移除此年度日历</button>
      </details>
    {/each}
    <h4>特殊日期与纪念日</h4>
    <p>生日和纪念日默认每年重复，放假等安排默认一次。未注明农历时按公历保存。日期独立保存在日历中，清空聊天不会删除这些日期。农历日期只在已导入对照表覆盖的年份触发；公历 2 月 29 日仅在闰年当天触发。</p>
    {#each data.special_dates as date (date.id)}
      <div class="date-entry"><div><strong>{date.name}</strong><p>{date.calendar==='lunar'?'农历':'公历'} {date.leap_month?'闰':''}{date.month} 月 {date.day} 日 · {date.yearly?'每年重复':`${date.year} 年一次`}{!date.enabled?' · 已暂停':''}</p></div>
        <button disabled={busy} onclick={()=>form={...date}}>修改</button><button disabled={busy} onclick={()=>action(()=>invoke('care_special_delete',{id:date.id}))}>删除</button></div>
    {/each}
    <div class="editor">
      <label>日期名称<input aria-label="特殊日期名称" bind:value={form.name} maxlength="80" placeholder="例如：我的生日、寒假开始" /></label>
      <div class="row"><label>类别<select bind:value={form.kind} onchange={e=>{form.kind=e.currentTarget.value;form.yearly=['birthday','anniversary'].includes(form.kind);}}><option value="birthday">生日</option><option value="anniversary">纪念日</option><option value="vacation">假期</option><option value="special">其他特殊日期</option></select></label>
        <label>历法<select aria-label="特殊日期历法" bind:value={form.calendar} onchange={()=>form.leap_month=false}><option value="gregorian">公历</option><option value="lunar">农历</option></select></label>
        <label>月<input aria-label="特殊日期月份" type="number" bind:value={form.month} min="1" max="12" /></label><label>日<input aria-label="特殊日期日" type="number" bind:value={form.day} min="1" max={form.calendar==='lunar'?30:31} /></label></div>
      <div class="row"><label><input type="checkbox" bind:checked={form.yearly} />每年重复</label>{#if !form.yearly}<label>年份<input aria-label="特殊日期年份" type="number" bind:value={form.year} min="1901" max="2100" /></label>{/if}{#if form.calendar==='lunar'}<label><input type="checkbox" bind:checked={form.leap_month} />农历闰月（仅匹配该闰月）</label>{/if}<label><input type="checkbox" bind:checked={form.enabled} />启用此日期</label></div>
      <div class="row"><button disabled={busy||!form.name.trim()} onclick={()=>action(save)}>{form.id?'保存修改':'添加特殊日期'}</button>{#if form.id}<button onclick={()=>form=empty()}>取消修改</button>{/if}</div>
    </div>
  {/if}
  {#if message}<p role="status">{message}</p>{/if}
</section>
<style>
  section{display:grid;gap:12px}h4,p{margin:0}p{font-size:12px;line-height:1.65;opacity:.75}.row,.date-entry{display:flex;flex-wrap:wrap;gap:8px;align-items:center}label{display:flex;flex-wrap:wrap;gap:6px;align-items:center;font-size:13px}input,select,button{font:inherit;color:inherit}input:not([type=checkbox]),select{padding:6px;border:1px solid #b8c3d0;border-radius:6px;background:transparent;max-width:100%}input[type=number]{width:85px}button{padding:7px 10px;border:1px solid #a9b8c9;border-radius:7px;background:transparent;cursor:pointer}button:disabled{opacity:.5;cursor:default}details,.editor,.date-entry{padding:12px;border:1px solid #dce1e8;border-radius:8px}.editor,.events{display:grid;gap:8px}.events{max-height:220px;overflow:auto;font-size:12px;margin:10px 0}.date-entry>div{flex:1}a{color:inherit}
</style>
