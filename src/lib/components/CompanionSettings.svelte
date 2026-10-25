<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import CompanionHistory from './CompanionHistory.svelte';
  import EnvironmentSettings from './EnvironmentSettings.svelte';
  import CompanionVoiceLibrary from './CompanionVoiceLibrary.svelte';
  import type { CompanionView, CompanionSettings, Turn, Endpoint, RuntimeRole } from '$lib/companion/types';
  let view = $state<CompanionView | null>(null);
  let config = $state<CompanionSettings | null>(null);
  let message = $state('');
  let character=$state<RuntimeRole|null>(null);
  let working = $state(false);
  let history = $state<Turn[]>([]);
  let showHistory = $state(false);
  let confirmClear = $state(false);
  let showEnvironment = $state(false);
  let excluded = $state('');
  let previewAudio: HTMLAudioElement | null = null;
  const endpoints = [{ id: 'chat', title: '对话模型', hint: 'DeepSeek V4.1 Flash 使用 deepseek-flash' }, { id: 'vision', title: '识图模型', hint: '独立配置；需要支持图片输入' }, { id: 'speech', title: '生成语音', hint: 'Fish Audio · 跟随当前角色的朗读设置' }] as const;
  async function load() {
    const next = await invoke<CompanionView>('companion_get');
    view = next; config = next.settings;
    const role=await invoke<RuntimeRole>('companion_character_get').catch(()=>null);
    character=role&&typeof role.id==='string'&&typeof role.persona==='string'&&typeof role.voice_locale==='string'?role:null;
    config.web_search ??= {enabled:true};
    config.web_search.search_when_uncertain ??= false;
    for (const kind of ['chat','vision'] as const) {config[kind].thinking ??= 'off'; config[kind].thinking_format ??= 'auto';}
    excluded = config.excluded_windows.join('\n');
    for (const kind of ['chat', 'vision', 'speech', 'memory'] as const) config[kind].api_key = '';
  }
  async function action(fn: () => Promise<unknown>) {
    working = true;
    try { const result = await fn(); message = typeof result === 'string' ? result : '已完成'; }
    catch (error) { message = String(error); }
    finally { working = false; }
  }
  async function persist() {
    if (!config) return;
    const replyLimit = Number(config.reply_max_chars ?? 0);
    if (!Number.isSafeInteger(replyLimit) || replyLimit < 0) throw new Error('回复字数上限须为非负整数，0 表示不限制');
    config.reply_max_chars = replyLimit;
    if(character){config.characters??={};config.characters[character.id]={display_name:character.display_name,persona:character.persona,voice_locale:character.voice_locale,voice_id:character.voice_id};}
    config.excluded_windows = excluded.split('\n').map(value => value.trim()).filter(Boolean);
    view = await invoke<CompanionView>('companion_save', { settings: $state.snapshot(config) });
    config = view.settings;
    for (const kind of ['chat', 'vision', 'speech', 'memory'] as const) config[kind].api_key = '';
  }
  function supportsThinking(endpoint: Endpoint) {
    if (endpoint.thinking_format && endpoint.thinking_format !== 'auto') return true;
    try { return new URL(endpoint.base_url).hostname === 'api.deepseek.com' && ['deepseek-flash','deepseek-v4-pro'].includes(endpoint.model); } catch { return false; }
  }
  async function test(kind: string) { await persist(); return invoke<string>('companion_test', { kind }); }
  async function recent() { history = await invoke<Turn[]>('companion_history'); showHistory = true; }
  async function forget(id: string | null) { await invoke('companion_forget', { id }); await recent(); confirmClear = false; return '本机已遗忘；远端清理完成前不会检索旧记忆'; }
  async function exportHistory() {
    return invoke<string>('companion_export');
  }
  async function play(data: string) {
    const settings = await invoke<{volume: number; no_audio_mode: boolean}>('get_settings');
    if (settings.no_audio_mode) return '请先关闭全局静音再试听';
    previewAudio?.pause(); previewAudio = new Audio(data); previewAudio.volume = settings.volume;
    await previewAudio.play(); return '正在试听';
  }
  onMount(() => {
    let stopped = false; const cleanup: Array<() => void> = [];
    void load().catch(error => message = String(error));
    void listen('companion-settings-changed', () => { if (!working) void load().catch(error => message = String(error)); }).then(fn => stopped ? fn() : cleanup.push(fn));
    void listen('companion-history-changed', () => { if (showHistory) void recent().catch(error => message = String(error)); }).then(fn => stopped ? fn() : cleanup.push(fn));
    void listen<{message: string}>('companion-status', event => { if (view) view.status = event.payload.message; }).then(fn => stopped ? fn() : cleanup.push(fn));
    return () => { stopped = true; previewAudio?.pause(); cleanup.forEach(fn => fn()); };
  });
</script>

<section class="companion-settings" aria-label="桌宠 AI 设置">
  <header><span class="eyebrow">DESKTOP COMPANION</span><h2>桌宠 AI 设置</h2><p>所有角色共用 AI 功能与接口；人格、朗读语言和声音按角色保存。</p></header>
  {#if config && view}
    <div class="toggles">
      <label><input type="checkbox" bind:checked={config.input_enabled} />显示对话输入框</label>
      <label><input type="checkbox" bind:checked={config.speech_enabled} />AI 回复语音</label>
      <label><input type="checkbox" bind:checked={config.observation_enabled} />定时屏幕观察</label>
    </div>
    <div class="fields two">
      <label>希望桌宠怎么称呼你<input bind:value={config.nickname} maxlength="40" placeholder="留空则自然称呼" /></label>
      <label>回复字数上限<input type="number" bind:value={config.reply_max_chars} min="0" step="1" placeholder="0 表示不限制" /></label>
    </div>
    <p class="hint">0 表示不限制总字数，也可填写任意正整数设置上限。回复按语意分片，每片最多 80 字，完整显示后停留 4 秒。模型服务自身的输出容量仍适用。开关保存后生效，也可在右键菜单快速切换。</p>
    {#if character}<details><summary>当前角色人格与声音<span>{character.display_name} · {character.id}</span></summary>
      <div class="fields"><label>角色称呼<input bind:value={character.display_name} maxlength="80" /></label><label>人格提示词<textarea aria-label="当前角色人格提示词" bind:value={character.persona} rows="8" maxlength="12000"></textarea></label></div>
      <div class="fields two"><label>朗读语言<select aria-label="当前角色朗读语言" bind:value={character.voice_locale}><option value="original">跟随回复原文</option><option value="zh">中文</option><option value="ja">日语</option><option value="en">英语</option></select></label></div>
      <p class="hint">聊天、屏幕观察与自动关怀共同使用当前角色的人格与声音。切换桌宠时自动读取对应角色的配置，新角色使用通用默认人格。修改后点击底部保存。</p>
    </details>{/if}
    <details>
      <summary>网页搜索<span>默认开启 · 需要时查询</span></summary>
      {#if config.web_search}
        <label class="check"><input type="checkbox" bind:checked={config.web_search.enabled} />允许按需网页搜索</label>
        <label class="check"><input type="checkbox" bind:checked={config.web_search.search_when_uncertain} disabled={!config.web_search.enabled} />无法判断时默认搜索</label>
      {/if}
      <p class="hint">默认只在明确需要核查公开资料时搜索。开启“无法判断时默认搜索”后，模型不确定是否需要搜索时也会尝试查询；无法提炼公开查询关键词时先询问。关闭网页搜索或本次明确要求不联网时，不会搜索。此偏好保存后生效。</p>
      <p class="hint">首版使用官方 DeepSeek 对话配置；其他服务仍可正常聊天。搜索失败会在输入框状态区提示，提示不会进入回复气泡或语音。</p>
    </details>
    {#each endpoints as endpoint}
      <details open={endpoint.id === 'chat'}>
        <summary>{endpoint.title}<span>{config[endpoint.id].has_key ? '密钥已配置' : '待配置'}</span></summary>
        <p class="hint">{endpoint.hint}</p>
        <div class="fields">
          <label>API 地址<input bind:value={config[endpoint.id].base_url} placeholder="https://…" autocomplete="off" /></label>
          <label>模型 ID<input bind:value={config[endpoint.id].model} autocomplete="off" /></label>
          <label>API 密钥<input type="password" bind:value={config[endpoint.id].api_key} placeholder={config[endpoint.id].has_key ? '已安全保存；留空保持原密钥' : '填写 API Key'} autocomplete="new-password" /></label>
        </div>
        {#if endpoint.id !== 'speech'}
          <div class="fields two">
            <label>{endpoint.title}思考强度<select aria-label={`${endpoint.title}思考强度`} bind:value={config[endpoint.id].thinking} disabled={!supportsThinking(config[endpoint.id])}><option value="off">关闭（默认）</option><option value="low">低</option><option value="high">高</option><option value="max">最高</option></select></label>
            <label>思考接口类型<select aria-label={`${endpoint.title}思考接口类型`} bind:value={config[endpoint.id].thinking_format}><option value="auto">自动识别</option><option value="deepseek">DeepSeek 兼容</option><option value="reasoning_effort">OpenAI 兼容</option></select></label>
          </div>
          <p class="hint">聊天与识图分别保存，默认关闭思考。自动识别仅向已适配模型发送参数；使用其他服务时，请根据服务文档选择兼容类型。模型可能不支持部分档位或关闭思考，此时需调整配置。搜索判断、后台摘要和语音翻译保持非思考。</p>
        {/if}
        {#if endpoint.id === 'speech'}
          <div class="fields two">{#if character}<label>当前角色声音 ID<input aria-label="当前角色声音 ID" bind:value={character.voice_id} placeholder="reference_id；留空则只显示文字" /></label>{/if}<label>语速<input type="number" bind:value={config.speech_speed} step="0.05" min="0.6" max="1.5" /></label></div>
          <button disabled={working} onclick={() => action(async () => { await persist(); return play(await invoke<string>('companion_speech_preview')); })}>试听合成声音</button>
          {#if character}<CompanionVoiceLibrary role={character} serviceUrl={config.speech.base_url} {working} run={action} prepareUpload={persist} afterUpload={load} {play} report={value => message = value} />{:else}<p class="hint">角色配置未加载，请重新打开设置后管理声音素材。</p>{/if}
        {/if}
        <button disabled={working} onclick={() => action(() => test(endpoint.id))}>保存并测试连接</button>
      </details>
    {/each}
    <details>
      <summary>持续记忆<span>专用 bank · 与其他工作区隔离</span></summary>
      <label class="check"><input type="checkbox" bind:checked={config.memory_enabled} />启用 Hindsight 长期记忆</label>
      <div class="fields"><label>本机 Hindsight 地址<input bind:value={config.memory.base_url} /></label><label>鉴权密钥（如有）<input type="password" bind:value={config.memory.api_key} placeholder="留空保持原密钥" autocomplete="new-password" /></label></div>
      <p class="bank"><strong>桌宠专用 bank（只读）</strong><code>{view.bank_id}</code></p>
      <p class="hint">由本机身份自动生成并验证所有权，不能填入编程助手或其他工作区的 bank。长期记忆待同步：{view.pending} 条。聊天原文保存在本机；当前 Hindsight 的记忆抽取会调用其配置的模型 API。</p>
      <p class="hint">当前所有桌宠角色共用同一记忆 bank 和连续对话历史；切换角色会保留已有记忆与会话。</p>
      <button disabled={working} onclick={() => action(() => test('memory'))}>保存并检查记忆连接</button>
      <button disabled={working} onclick={() => action(recent)}>查看最近记录</button>
      <button disabled={working} onclick={() => action(exportHistory)}>导出全部记录</button>
      <button class="danger" disabled={working} onclick={() => confirmClear = !confirmClear}>清空桌宠记忆</button>
      {#if confirmClear}<div class="confirm"><p>将删除桌宠的本地记录，并清理应用专用 bank。该记录供本机桌宠角色共用，其他应用的 bank 不受影响。</p><button class="danger" disabled={working} onclick={() => action(() => forget(null))}>确认清空桌宠记忆</button><button onclick={() => confirmClear = false}>取消</button></div>{/if}
      {#if view.summary}<p class="hint">关系与话题摘要：{view.summary}</p>{/if}
    </details>
    <details>
      <summary>屏幕观察<span>默认关闭 · 每 5 分钟</span></summary>
      <div class="fields two">
        <label>间隔（秒）<input type="number" bind:value={config.observation_interval_secs} min="30" max="86400" /></label>
        <label>观察范围<select bind:value={config.capture_scope}><option value="foreground">当前前台窗口</option><option value="screen">鼠标所在显示器</option><option value="region">指定屏幕区域</option></select></label>
      </div>
      {#if config.capture_scope === 'region'}<div class="region">{#each ['X', 'Y', '宽度', '高度'] as label, index}<label>{label}<input type="number" bind:value={config.capture_region[index]} /></label>{/each}</div>{/if}
      <label class="fields">排除窗口（标题关键词，每行一个）<textarea bind:value={excluded} rows="3" placeholder="银行&#10;密码管理器"></textarea></label>
      <p class="hint">开启后会把选定范围的截图发送给你配置的识图 API，原图不落盘。锁屏、免打扰和无效画面会跳过；对话进行时延后。观察摘要与对话使用同一专用 bank。</p>
      <button disabled={working} onclick={() => action(async () => { await persist(); await invoke('companion_observe'); return '已完成本次观察'; })}>保存并立即观察一次</button>
    </details>
    <footer><button class="primary" disabled={working} onclick={() => action(async () => { await persist(); return '桌宠 AI 设置已保存'; })}>{working ? '处理中…' : '保存桌宠 AI 设置'}</button><button onclick={() => action(() => invoke('companion_stop'))}>停止当前回复和语音</button><p role="status">{message || view.status}</p></footer>
    <details ontoggle={(event) => { if ((event.currentTarget as HTMLDetailsElement).open) showEnvironment = true; }}><summary>环境与自动关怀<span>地点 · 天气 · 提醒</span></summary>{#if showEnvironment}<EnvironmentSettings />{/if}</details>
    {#if showHistory}<CompanionHistory turns={history} {working} onForget={(id) => void action(() => forget(id))} onChanged={() => void recent()} /><button onclick={() => showHistory = false}>收起记录</button>{/if}
  {:else}<p role="status">{message || '正在读取桌宠 AI 设置…'}</p>{/if}
</section>

<style>
  .companion-settings { max-width: 570px; margin: 24px auto; padding: 28px; border: 1px solid #dcd5d7; border-radius: 12px; background: #fffcfa; color: #3f3339; font-family: 'Segoe UI', 'Microsoft YaHei', sans-serif; }
  .eyebrow { font-size: 10px; font-weight: 700; letter-spacing: .16em; color: #8b4d62; }
  h2 { font-size: 20px; line-height: 1.5; margin: 7px 0; } header p { font-size: 13px; color: #7c6872; margin: 6px 0 22px; }
  .toggles { display: flex; flex-wrap: wrap; gap: 12px; padding: 12px 0 20px; border-bottom: 1px solid #e9e0e3; font-size: 13px; } .toggles label, .check { display: flex; align-items: center; gap: 5px; }
  .fields { display: grid; gap: 12px; margin: 16px 0; } .two { grid-template-columns: 1fr 1fr; } .fields label { display: grid; gap: 6px; font-size: 12px; }
  input:not([type=checkbox]), select, textarea { box-sizing: border-box; width: 100%; border: 1px solid #dcd0d5; border-radius: 6px; padding: 9px; background: #fff; color: inherit; font: inherit; font-size: 13px; min-width: 0; }
  input:focus, textarea:focus, select:focus { outline: 2px solid #b8899a; outline-offset: 1px; }
  .hint { color: #7c6872; line-height: 1.7; font-size: 12px; } details { border-bottom: 1px solid #e5dce0; padding: 17px 0; } summary { cursor: pointer; font-weight: 600; font-size: 14px; } summary span { float: right; font-size: 11px; font-weight: 400; color: #8a7680; }
  button { margin: 5px 7px 3px 0; padding: 8px 12px; border: 1px solid #d5c6cd; border-radius: 6px; color: inherit; background: transparent; font-size: 12px; cursor: pointer; } button:disabled { opacity: .5; cursor: default; } .primary { background: #854b60; color: white; border-color: #854b60; } .danger { color: #a63c43; }
  footer { padding-top: 20px; } footer p { line-height: 1.6; font-size: 12px; color: #806770; } .bank { padding: 12px; background: #f4edef; font-size: 11px; overflow-wrap: anywhere; } code { display: block; margin-top: 8px; } .region { display: grid; grid-template-columns: repeat(4,1fr); gap: 8px; font-size: 12px; } .confirm { border-left: 2px solid #a63c43; padding: 1px 12px 10px; font-size: 12px; }

  @media (max-width: 540px) { .companion-settings { padding: 18px; } .two { grid-template-columns: 1fr; } summary span { float: none; display: block; margin: 7px 0 0 16px; } }
  @media (prefers-color-scheme: dark) { .companion-settings { background: #29262b; color: #eee4e8; border-color: #54474f; } input:not([type=checkbox]), select, textarea { background: #343036; border-color: #66545e; } details, .toggles { border-color: #54474f; } .hint, header p, summary span, footer p { color: #bba6b1; } .bank { background: #373037; } }
</style>
