<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { isSendKey, type CompanionView, type ScreenSelection, type PreparedScreenTarget,
    type ChatSendPayload, type SendOutcome, type RequestReceipt, type CompanionStatus, type Observation } from '$lib/companion/types';
  let draft = $state('');
  let busy = $state(false);
  let status = $state('就绪');
  let warning = $state('');
  let notice = $state('');
  let configured = $state(false);
  let visionConfigured = $state(false);
  let layoutReady = $state(false);
  let enabled = $state(true);
  let collapsed = $state(false);
  let toggling = $state(false);
  let toggleError = $state('');
  let preparing = $state(false);
  let screen = $state<ScreenSelection>({mode: 'none'});
  let screenLabel = $state('');
  let screenError = $state(false);
  let lastObservation = $state<Observation | null>(null);
  let characterName = $state('艾玛');
  let epoch = 0;
  let composing = false;
  let input = $state<HTMLTextAreaElement>();
  let destroyed = false;
  let layoutRevision = 0;
  let refreshRevision = 0;
  let textRevision = 0;
  let selectionRevision = 0;
  let sendRevision = 0;
  let currentRequest = 0;
  let lastEventSeq = 0;
  let activeClient = '';
  let activityTimer: ReturnType<typeof setTimeout> | undefined;
  let receiptTimer: ReturnType<typeof setTimeout> | undefined;
  const draftKey = 'ema-companion-draft';
  const receiptKey = 'companion-pending-input';
  const pendingTextKey = 'companion-pending-text';
  function errorMessage(error: unknown): string {
    const code = String(error);
    const messages: Record<string, string> = {
      context_recent_only: '本轮使用近期对话，较早原文仍保留', history_changed: '记录已变化，请重新发送', request_in_progress: '这条消息仍在处理中',
      request_id_conflict: '发送编号冲突，请重试', screen_not_configured: '请先在设置中配置识图模型',
      screen_target_unavailable: '找不到可观察的应用窗口', screen_target_expired: '屏幕选择已过期，请重新选择',
      screen_target_changed: '目标窗口已变化，请重新选择', screen_excluded: '该窗口已被排除',
      screen_capture_failed: '未能获取画面，可以重试或仅发送文字', screen_unusable: '画面信息不足，可以重试或仅发送文字',
      screen_sensitive: '画面可能包含敏感内容，本次已停止', screen_vision_failed: '识图未完成，可以重试或仅发送文字',
      screen_vision_invalid: '识图结果无效，可以重试或仅发送文字', search_failed: '本次调用搜索工具失败',
      search_route_failed: '本次搜索判断失败，未进行联网核查', search_empty: '本次搜索没有找到可用资料',
    };
    return messages[code] || (code.startsWith('route_') ? '本次搜索判断失败，未进行联网核查' : code.startsWith('search_') ? '本次调用搜索工具失败，未完成资料核查' : code === 'provider_limit' ? '模型已到输出上限，可以继续提问' : code === 'speech_failed' ? '文字已回复，语音未完成' : code);
  }
  async function refresh() {
    const request = ++refreshRevision;
    const revision = layoutRevision;
    try {
      const view = await invoke<CompanionView>('companion_get');
      if (destroyed || request !== refreshRevision) return;
      configured = view.settings.chat.has_key;
      visionConfigured = !!(view.settings.vision.has_key && view.settings.vision.base_url && view.settings.vision.model);
      if (!activeClient) busy = view.busy;
      enabled = view.settings.input_enabled;
      epoch = view.history_epoch ?? 0;
      currentRequest = Math.max(currentRequest, view.request ?? 0);
      lastObservation = view.last_observation ?? null;
      characterName = view.character_name || '艾玛';
      if (!toggling && revision === layoutRevision) collapsed = view.settings.input_collapsed;
      if (!activeClient) status = configured ? view.status : '请先在设置中配置对话 API';
      if (view.warning) warning = view.warning;
      if (!activeClient && (view.request ?? 0) >= currentRequest && view.notice !== undefined) notice = view.notice || '';
    } catch (error) { if (!destroyed && request === refreshRevision) status = errorMessage(error); }
    finally { if (!destroyed && request === refreshRevision) layoutReady = true; }
  }
  function updateLayout() {
    void invoke('companion_set_input_layout', { expanded: screen.mode !== 'none' || screenError }).catch(() => {});
  }
  function activity() {
    void invoke('care_interaction_activity', { active: true }).catch(() => {});
    clearTimeout(activityTimer);
    activityTimer = setTimeout(() => void invoke('care_interaction_activity', { active: false }).catch(() => {}), 15000);
  }
  function editDraft() {
    textRevision++;
    localStorage.setItem(draftKey, draft);
    activity();
  }
  function clearScreen() {
    if (busy) return;
    selectionRevision++; screen = {mode: 'none'}; screenLabel = ''; screenError = false; updateLayout();
  }
  async function prepareScreen() {
    if (busy || preparing) return;
    if (!visionConfigured) { status = '请先在设置中配置识图模型'; return; }
    const revision = ++selectionRevision;
    preparing = true; status = '正在准备屏幕…';
    try {
      const target = await invoke<PreparedScreenTarget>('companion_prepare_screen');
      if (destroyed || revision !== selectionRevision) return;
      screen = {mode: 'fresh', target_token: target.target_token};
      screenLabel = `发送时观察：${target.label}`; screenError = false;
      status = '发送时获取画面'; updateLayout();
    } catch (error) { if (revision === selectionRevision) { screen = {mode:'none'}; screenLabel = ''; status = errorMessage(error); updateLayout(); } }
    finally { if (revision === selectionRevision) preparing = false; }
  }
  function referenceLast() {
    if (busy || !lastObservation) return;
    selectionRevision++;
    screen = {mode:'reference', observation_ids:[lastObservation.id]};
    screenLabel = `引用观察 · ${new Date(lastObservation.observed_at).toLocaleString()}`;
    screenError = false; updateLayout();
  }
  async function toggleInput() {
    if (toggling) return;
    toggling = true; toggleError = ''; layoutRevision++;
    const previous = collapsed;
    if (!previous) { collapsed = true; await tick(); }
    try {
      const next = await invoke<boolean>('companion_set_input_collapsed', { collapsed: !previous });
      if (destroyed) return;
      collapsed = next; composing = false;
      await tick();
      if (collapsed) void invoke('companion_blur').catch(() => {});
      else { input?.focus(); if (screen.mode === 'fresh') void prepareScreen(); }
    } catch (error) { collapsed = previous; toggleError = errorMessage(error); status = toggleError; }
    finally { layoutRevision++; toggling = false; }
  }
  function applyOutcome(outcome: SendOutcome, text: string, textRev: number, selectionRev: number) {
    epoch = outcome.history_epoch;
    if (outcome.warning_code) warning = errorMessage(outcome.warning_code);
    if (outcome.text_committed) {
      if (textRevision === textRev && draft.trim() === text) { draft = ''; localStorage.removeItem(draftKey); }
      if (selectionRevision === selectionRev) { screen = {mode:'none'}; screenLabel = ''; screenError = false; updateLayout(); }
    } else if (outcome.error_code) {
      status = errorMessage(outcome.error_code); screenError = screen.mode === 'fresh'; updateLayout();
    }
  }
  async function recoverReceipt(id: string, snapshot?: { text: string; textRev: number; selectionRev: number }): Promise<boolean> {
    const receipt = await invoke<RequestReceipt | null>('companion_request_receipt', { clientMessageId: id });
    if (destroyed || activeClient && activeClient !== id) return false;
    if (receipt?.text_committed && snapshot) {
      applyOutcome(receipt.outcome ?? {request:receipt.request,turn_id:receipt.turn_id,result:'completed',text_committed:true,observation_ids:[],history_epoch:receipt.history_epoch,replayed:true}, snapshot.text, snapshot.textRev, snapshot.selectionRev);
    }
    if (receipt && (receipt.state === 'pending' || receipt.state === 'text_committed')) {
      busy = true; activeClient = id; currentRequest = Math.max(currentRequest, receipt.request ?? 0);
      receiptTimer = setTimeout(() => { void recoverReceipt(id, snapshot).catch(() => { status = '尚未确认发送结果，请稍后重试'; }); }, 1000);
      return true;
    }
    if (receipt?.outcome && snapshot) applyOutcome(receipt.outcome, snapshot.text, snapshot.textRev, snapshot.selectionRev);
    activeClient = ''; localStorage.removeItem(receiptKey); localStorage.removeItem(pendingTextKey); await refresh();
    return false;
  }
  async function send() {
    if (busy || preparing || !configured || !draft.trim()) return;
    const text = draft.trim();
    const textRev = textRevision, selectionRev = selectionRevision, run = ++sendRevision;
    const payload: ChatSendPayload = {client_message_id: crypto.randomUUID(),expected_history_epoch:epoch,text,screen:$state.snapshot(screen)};
    activeClient = payload.client_message_id; localStorage.setItem(receiptKey, activeClient); localStorage.setItem(pendingTextKey, text);
    busy = true; warning = ''; notice = ''; screenError = false; lastEventSeq = 0; activity();
    let uncertain = false;
    try {
      const outcome = await invoke<SendOutcome>('companion_send', { payload });
      if (!destroyed && run === sendRevision) applyOutcome(outcome, text, textRev, selectionRev);
    } catch (error) {
      if (destroyed || run !== sendRevision) return;
      status = errorMessage(error); warning = status;
      try { uncertain = await recoverReceipt(payload.client_message_id, {text,textRev,selectionRev}); }
      catch { uncertain = true; status = '尚未确认发送结果，请保持此窗口；可以停止当前请求'; receiptTimer = setTimeout(() => { void recoverReceipt(payload.client_message_id, {text,textRev,selectionRev}).catch(() => {}); }, 1500); }
      screenError = payload.screen.mode === 'fresh'; updateLayout();
    } finally {
      if (!destroyed && run === sendRevision && !uncertain) {
        activeClient = ''; busy = false; localStorage.removeItem(receiptKey); localStorage.removeItem(pendingTextKey);
        void refresh();
      }
    }
  }
  async function retryScreen() { await prepareScreen(); if (screen.mode === 'fresh') await send(); }
  function textOnly() { clearScreen(); void send(); }
  function keydown(event: KeyboardEvent) {
    if (event.key === 'Escape') { input?.blur(); void invoke('companion_blur').catch(() => {}); return; }
    if (!composing && isSendKey(event)) { event.preventDefault(); void send(); }
  }
  onMount(() => {
    draft = localStorage.getItem(draftKey) || '';
    const unlisteners: Array<() => void> = [];
    const track = (fn: () => void) => { if (destroyed) fn(); else unlisteners.push(fn); };
    void refresh(); updateLayout();
    const previousId = localStorage.getItem(receiptKey);
    if (previousId) { activeClient = previousId; busy = true; void recoverReceipt(previousId,{text:localStorage.getItem(pendingTextKey) || '',textRev:textRevision,selectionRev:selectionRevision}).catch(() => status = '尚未确认上次发送结果'); }
    void listen<CompanionStatus>('companion-status', event => {
      const value = event.payload;
      if (value.request != null && value.request < currentRequest) return;
      if (activeClient && value.client_message_id && activeClient !== value.client_message_id) return;
      if (value.request === currentRequest && value.event_seq != null && value.event_seq <= lastEventSeq) return;
      if ((value.request ?? currentRequest) > currentRequest) { lastEventSeq = 0; warning = ''; notice = ''; }
      currentRequest = value.request ?? currentRequest; lastEventSeq = value.event_seq ?? lastEventSeq + 1;
      status = value.message; busy = value.busy;
      if (value.warning) warning = value.warning;
      if (value.notice !== undefined) notice = value.notice || '';
    }).then(track);
    void listen('companion-settings-changed', () => { if (screen.mode !== 'none') clearScreen(); void refresh(); }).then(track);
    void listen('companion-history-changed', () => { void refresh(); }).then(track);
    const refocus = () => { if (screen.mode === 'fresh' && !busy) void prepareScreen(); };
    window.addEventListener('focus', refocus);
    return () => { destroyed = true; clearTimeout(activityTimer); clearTimeout(receiptTimer); window.removeEventListener('focus',refocus); unlisteners.forEach(fn => fn()); };
  });
</script>

{#if layoutReady && enabled}
<div class="input-window">
{#if collapsed}
  <button class="collapsed-entry" aria-label="展开聊天输入框" aria-expanded="false" aria-controls="companion-entry" title={warning || notice || toggleError || status || '展开聊天输入框'} disabled={toggling} onclick={toggleInput}>
    <span>{toggleError ? '重试展开' : warning || notice ? '查看本次提示' : `和${characterName}聊聊`}</span><span aria-hidden="true">⌄</span>
  </button>
{:else}
<div class="composer">
  <div class="entry" id="companion-entry">
    <textarea bind:this={input} bind:value={draft} aria-label={`给${characterName}的消息`} placeholder={configured ? `想和${characterName}说些什么？` : '在设置中连接模型后，就能聊天啦'} maxlength="4000" rows="1"
      onkeydown={keydown} onfocus={activity} oncompositionstart={() => { composing = true; activity(); }} oncompositionend={() => composing = false}
      oninput={editDraft}></textarea>
    {#if busy}
      <button class="stop" onclick={() => invoke('companion_stop').catch(error => status = String(error))} title="停止回复">停止</button>
    {:else}
      <button disabled={!configured || preparing || !draft.trim()} onclick={send} title="Enter 发送">发送</button>
    {/if}
  </div>
  {#if screen.mode !== 'none' || screenError}
    <div class="attachment">
      {#if screenError}<button disabled={busy || preparing} onclick={retryScreen}>重试带上屏幕</button><button disabled={busy} onclick={textOnly}>仅发送文字</button>
      {:else}<span title={screenLabel}>{screenLabel}</span><button aria-label="移除屏幕引用" disabled={busy} onclick={clearScreen}>×</button>{/if}
    </div>
  {/if}
  <div class="footer">
    <div class="status" role="status" title={warning || notice || status}>{warning || notice || status}</div>
    <button class="screen-toggle" aria-label="带上当前屏幕" title={visionConfigured ? '发送时观察最近的应用窗口' : '请先配置识图模型'} aria-pressed={screen.mode === 'fresh'} disabled={busy || preparing || !visionConfigured} onclick={() => screen.mode === 'fresh' ? clearScreen() : prepareScreen()}>屏幕</button>
    {#if lastObservation && screen.mode === 'none'}<button class="screen-toggle" aria-label="引用刚才的观察" title="引用刚才的观察，不重新截图" disabled={busy || preparing} onclick={referenceLast}>引用</button>{/if}
    <button class="collapse" aria-label="收起聊天输入框" aria-expanded="true" aria-controls="companion-entry" title="收起聊天输入框" disabled={toggling} onclick={toggleInput}><span aria-hidden="true">⌃</span> 收起</button>
  </div>
</div>
{/if}
</div>
{/if}

<style>
  :global(html), :global(body) { margin: 0; width: 100%; height: 100%; background: transparent !important; overflow: hidden; font-family: 'Segoe UI', 'Microsoft YaHei', sans-serif; }
  .input-window { width: 100vw; height: 100vh; overflow: hidden; }
  .composer { box-sizing: border-box; width: 100%; height: 100%; padding: 7px 10px; border: 1px solid #d9c9c9; border-radius: 14px; background: #fff9f7; color: #57434a; overflow: hidden; }
  .entry { display: flex; gap: 7px; align-items: center; }
  textarea { flex: 1; min-width: 0; height: 34px; max-height: 34px; padding: 4px 0; border: 0; outline: 0; resize: none; overflow-x: hidden; overflow-y: auto; font: inherit; font-size: 13px; background: transparent; color: inherit; }
  textarea::placeholder { color: #927f85; }
  button { border: 0; border-radius: 8px; padding: 7px 10px; background: #8b4d62; color: white; cursor: pointer; font-size: 12px; }
  button:disabled { opacity: .4; cursor: default; }
  .stop { background: #64555b; }
  .footer { display: flex; align-items: center; gap: 6px; margin-top: 3px; }
  .status { flex: 1; min-width: 0; font-size: 10px; color: #8c747e; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .collapse { flex: none; background: transparent; color: inherit; padding: 2px 4px; font-size: 11px; }
  .attachment { display:flex; align-items:center; gap:5px; height:26px; font-size:10px; }
  .attachment span { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; flex:1; }
  .attachment button, .screen-toggle { background:transparent; color:inherit; padding:2px 4px; font-size:10px; flex:none; }
  .screen-toggle[aria-pressed=true] { background:#eedfe3; }
  .collapse:hover { background: #eedfe3; }
  .collapsed-entry { box-sizing: border-box; display: flex; align-items: center; justify-content: center; gap: 10px; width: 100%; height: 100%; border: 1px solid #d9c9c9; border-radius: 15px; padding: 0 10px; background: #fff9f7; color: #57434a; white-space: nowrap; overflow: hidden; }
  button:focus-visible { outline: 2px solid #b97790; outline-offset: -2px; }
  @media (prefers-color-scheme: dark) { .composer, .collapsed-entry { background: #292328; color: #f1e5e8; border-color: #61505a; } .status { color: #bca5af; } .collapse:hover { background: #49353f; } }
</style>
