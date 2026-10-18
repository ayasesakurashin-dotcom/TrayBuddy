<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import type { CompanionView, CompanionSettings, Turn } from '$lib/companion/types';
  let view = $state<CompanionView | null>(null);
  let config = $state<CompanionSettings | null>(null);
  let message = $state('');
  let working = $state(false);
  let history = $state<Turn[]>([]);
  let showHistory = $state(false);
  let confirmClear = $state(false);
  let excluded = $state('');
  let samples = $state<Array<{name: string; candidate_text: string}>>([]);
  let selectedSamples = $state<string[]>([]);
  let confirmVoiceUpload = $state(false);
  let previewAudio: HTMLAudioElement | null = null;
  const endpoints = [{ id: 'chat', title: '对话模型', hint: 'DeepSeek V4.1 Flash 使用 deepseek-flash' }, { id: 'vision', title: '识图模型', hint: '独立配置；需要支持图片输入' }, { id: 'speech', title: '生成语音', hint: 'Fish Audio · 中文气泡，日语声音' }] as const;
  async function load() {
    const next = await invoke<CompanionView>('companion_get');
    view = next; config = next.settings;
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
    config.excluded_windows = excluded.split('\n').map(value => value.trim()).filter(Boolean);
    view = await invoke<CompanionView>('companion_save', { settings: $state.snapshot(config) });
    config = view.settings;
    for (const kind of ['chat', 'vision', 'speech', 'memory'] as const) config[kind].api_key = '';
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
  async function loadSamples() { samples = await invoke('companion_voice_samples'); return '可先试听，再选择清晰自然的日语片段'; }
  async function createVoice() {
    await persist();
    const id = await invoke<string>('companion_voice_create', { names: $state.snapshot(selectedSamples) });
    await load(); confirmVoiceUpload = false; return `私有声音已建立：${id}`;
  }
  onMount(() => {
    let stopped = false; const cleanup: Array<() => void> = [];
    void load().catch(error => message = String(error));
    void listen('companion-settings-changed', () => { if (!working) void load().catch(error => message = String(error)); }).then(fn => stopped ? fn() : cleanup.push(fn));
    void listen<{message: string}>('companion-status', event => { if (view) view.status = event.payload.message; }).then(fn => stopped ? fn() : cleanup.push(fn));
    return () => { stopped = true; previewAudio?.pause(); cleanup.forEach(fn => fn()); };
  });
</script>

<section class="companion-settings" aria-label="艾玛 AI 设置">
  <header><span class="eyebrow">EMA COMPANION</span><h2>和艾玛一起，让日常多一点回应</h2><p>短句聊天、持续记忆，以及熟悉的日语声音。</p></header>
  {#if config && view}
    <div class="toggles">
      <label><input type="checkbox" bind:checked={config.input_enabled} />显示对话输入框</label>
      <label><input type="checkbox" bind:checked={config.speech_enabled} />AI 回复语音</label>
      <label><input type="checkbox" bind:checked={config.observation_enabled} />定时屏幕观察</label>
    </div>
    <div class="fields two">
      <label>希望艾玛怎么称呼你<input bind:value={config.nickname} maxlength="40" placeholder="留空则自然称呼" /></label>
      <label>回复字数上限<input type="number" bind:value={config.reply_max_chars} min="40" max="180" /></label>
    </div>
    <p class="hint">日常以 1—3 个短句回应，默认最多 120 字。开关保存后生效，也可在右键菜单快速切换。</p>
    {#each endpoints as endpoint}
      <details open={endpoint.id === 'chat'}>
        <summary>{endpoint.title}<span>{config[endpoint.id].has_key ? '密钥已配置' : '待配置'}</span></summary>
        <p class="hint">{endpoint.hint}</p>
        <div class="fields">
          <label>API 地址<input bind:value={config[endpoint.id].base_url} placeholder="https://…" autocomplete="off" /></label>
          <label>模型 ID<input bind:value={config[endpoint.id].model} autocomplete="off" /></label>
          <label>API 密钥<input type="password" bind:value={config[endpoint.id].api_key} placeholder={config[endpoint.id].has_key ? '已安全保存；留空保持原密钥' : '填写 API Key'} autocomplete="new-password" /></label>
        </div>
        {#if endpoint.id === 'speech'}
          <div class="fields two"><label>艾玛的私有声音 ID<input bind:value={config.voice_id} placeholder="reference_id" /></label><label>语速<input type="number" bind:value={config.speech_speed} step="0.05" min="0.6" max="1.5" /></label></div>
          <p class="hint">使用现有日语语音建立私有声音。气泡文本可能并非录音逐字稿，参考素材需先核对。</p>
          <button disabled={working} onclick={() => action(loadSamples)}>选择 Ema 预设声音素材</button>
          <button disabled={working} onclick={() => action(async () => { await persist(); return play(await invoke<string>('companion_speech_preview')); })}>试听合成声音</button>
          {#if samples.length}<div class="samples">{#each samples as sample}<div class="sample"><label><input type="checkbox" bind:group={selectedSamples} value={sample.name} />{sample.name}</label><button disabled={working} onclick={() => action(async () => play(await invoke<string>('companion_voice_sample_audio', { name: sample.name })))}>试听原音</button></div>{/each}</div><p class="hint">选择 1—8 段。上传后使用服务端语音识别，不把气泡文本当作录音逐字稿。</p><button disabled={working || selectedSamples.length === 0 || selectedSamples.length > 8} onclick={() => confirmVoiceUpload = true}>用所选素材建立私有声音</button>{/if}
          {#if confirmVoiceUpload}<div class="confirm"><p>将向已配置的 Fish Audio 服务上传 {selectedSamples.length} 段所选 Ema 日语素材，建立私有声音。</p><button disabled={working} onclick={() => action(createVoice)}>上传并创建私有声音</button><button onclick={() => confirmVoiceUpload = false}>取消</button></div>{/if}
        {/if}
        <button disabled={working} onclick={() => action(() => test(endpoint.id))}>保存并测试连接</button>
      </details>
    {/each}
    <details>
      <summary>持续记忆<span>专用 bank · 与其他工作区隔离</span></summary>
      <label class="check"><input type="checkbox" bind:checked={config.memory_enabled} />启用 Hindsight 长期记忆</label>
      <div class="fields"><label>本机 Hindsight 地址<input bind:value={config.memory.base_url} /></label><label>鉴权密钥（如有）<input type="password" bind:value={config.memory.api_key} placeholder="留空保持原密钥" autocomplete="new-password" /></label></div>
      <p class="bank"><strong>艾玛专用 bank（只读）</strong><code>{view.bank_id}</code></p>
      <p class="hint">由本机身份自动生成并验证所有权，不能填入编程助手或其他工作区的 bank。长期记忆待同步：{view.pending} 条。聊天原文保存在本机；当前 Hindsight 的记忆抽取会调用其配置的模型 API。</p>
      <button disabled={working} onclick={() => action(() => test('memory'))}>保存并检查记忆连接</button>
      <button disabled={working} onclick={() => action(recent)}>查看最近记录</button>
      <button disabled={working} onclick={() => action(exportHistory)}>导出全部记录</button>
      <button class="danger" disabled={working} onclick={() => confirmClear = !confirmClear}>清空艾玛记忆</button>
      {#if confirmClear}<div class="confirm"><p>将删除艾玛的本地记录，并清理她的专用 bank。其他 bank 不受影响。</p><button class="danger" disabled={working} onclick={() => action(() => forget(null))}>确认清空艾玛记忆</button><button onclick={() => confirmClear = false}>取消</button></div>{/if}
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
    <details><summary>艾玛的人格<span>完整背景 · 温柔明朗</span></summary><p class="hint">开朗、体贴，怕寂寞，也害怕被讨厌；有自己的判断，愿意认真帮助你。允许讨论完整剧情，区分各条路线。日语自称「ボク」，通过共同记忆逐渐熟悉你。</p></details>
    <footer><button class="primary" disabled={working} onclick={() => action(async () => { await persist(); return '艾玛 AI 设置已保存'; })}>{working ? '处理中…' : '保存艾玛 AI 设置'}</button><button onclick={() => action(() => invoke('companion_stop'))}>停止当前回复和语音</button><p role="status">{message || view.status}</p></footer>
    {#if showHistory}<section class="history"><h3>最近的共同记忆</h3>{#each history as turn (turn.id)}<article><small>{new Date(turn.created_at).toLocaleString()} · {turn.source === 'screen' ? '屏幕观察' : '对话'}</small><p>{turn.user_text}</p><p><strong>艾玛：</strong>{turn.reply}</p>{#if turn.japanese}<p class="hint">{turn.japanese}</p>{/if}<button disabled={working} onclick={() => action(() => forget(turn.id))}>忘记这条</button></article>{/each}<button onclick={() => showHistory = false}>收起记录</button></section>{/if}
  {:else}<p role="status">{message || '正在读取艾玛 AI 设置…'}</p>{/if}
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
  .history article { border-bottom: 1px solid #e5dce0; padding: 14px 0; font-size: 13px; overflow-wrap: anywhere; white-space: pre-wrap; } small { color: #88707c; } h3 { font-size: 15px; }
  .samples { max-height: 230px; overflow-y: auto; margin: 12px 0; border: 1px solid #dcd0d5; padding: 8px; border-radius: 6px; } .sample { display: flex; align-items: center; justify-content: space-between; font-size: 12px; } .sample label { display: flex; gap: 6px; align-items: center; }
  @media (max-width: 540px) { .companion-settings { padding: 18px; } .two { grid-template-columns: 1fr; } summary span { float: none; display: block; margin: 7px 0 0 16px; } }
  @media (prefers-color-scheme: dark) { .companion-settings { background: #29262b; color: #eee4e8; border-color: #54474f; } input:not([type=checkbox]), select, textarea { background: #343036; border-color: #66545e; } details, .toggles { border-color: #54474f; } .hint, header p, summary span, footer p, small { color: #bba6b1; } .bank { background: #373037; } }
</style>
