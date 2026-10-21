<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { isSendKey, type CompanionView } from '$lib/companion/types';
  let draft = $state('');
  let busy = $state(false);
  let status = $state('和艾玛聊聊');
  let configured = $state(false);
  let layoutReady = $state(false);
  let enabled = $state(true);
  let collapsed = $state(false);
  let toggling = $state(false);
  let toggleError = $state('');
  let composing = false;
  let input = $state<HTMLTextAreaElement>();
  let destroyed = false;
  let layoutRevision = 0;
  let refreshRevision = 0;
  async function refresh() {
    const request = ++refreshRevision;
    const revision = layoutRevision;
    try { const view = await invoke<CompanionView>('companion_get'); if (destroyed || request !== refreshRevision) return;
      configured = view.settings.chat.has_key; busy = view.busy;
      enabled = view.settings.input_enabled;
      if (!toggling && revision === layoutRevision) collapsed = view.settings.input_collapsed;
      status = configured ? view.status : '请先在设置中配置对话 API';
    } catch (error) { if (!destroyed && request === refreshRevision) status = String(error); }
    finally { if (!destroyed && request === refreshRevision) layoutReady = true; }
  }
  async function toggleInput() {
    if (toggling) return;
    toggling = true; toggleError = ''; layoutRevision++;
    const previous = collapsed;
    // Paint the compact control before the native window shrinks. Otherwise
    // WebView can briefly leave a clipped textarea inside the collapsed bounds.
    if (!previous) { collapsed = true; await tick(); }
    try {
      const next = await invoke<boolean>('companion_set_input_collapsed', { collapsed: !previous });
      if (destroyed) return;
      collapsed = next; composing = false;
      await tick();
      if (collapsed) void invoke('companion_blur').catch(() => {});
      else input?.focus();
    } catch (error) { collapsed = previous; toggleError = String(error); status = toggleError; }
    finally { layoutRevision++; toggling = false; }
  }
  async function send() {
    if (busy || !configured || !draft.trim()) return;
    const text = draft.trim(); busy = true;
    try {
      await invoke('companion_send', { text });
      if (draft.trim() === text) { draft = ''; localStorage.removeItem('ema-companion-draft'); }
    } catch (error) { status = String(error); }
    finally { busy = false; }
  }
  function keydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      input?.blur(); void invoke('companion_blur').catch(() => {}); return;
    }
    if (!composing && isSendKey(event)) { event.preventDefault(); void send(); }
  }
  onMount(() => {
    draft = localStorage.getItem('ema-companion-draft') || '';
    const unlisteners: Array<() => void> = [];
    const track = (fn: () => void) => { if (destroyed) fn(); else unlisteners.push(fn); };
    void refresh();
    void listen<{message: string; busy: boolean}>('companion-status', event => {
      status = event.payload.message; busy = event.payload.busy;
    }).then(track);
    void listen('companion-settings-changed', () => { void refresh(); }).then(track);
    return () => { destroyed = true; unlisteners.forEach(fn => fn()); };
  });
</script>

{#if layoutReady && enabled}
<div class="input-window">
{#if collapsed}
  <button class="collapsed-entry" aria-label="展开聊天输入框" aria-expanded="false" aria-controls="companion-entry" title={toggleError || '展开聊天输入框'} disabled={toggling} onclick={toggleInput}>
    <span>{toggleError ? '重试展开' : '和艾玛聊聊'}</span><span aria-hidden="true">⌄</span>
  </button>
{:else}
<div class="composer">
  <div class="entry" id="companion-entry">
    <textarea bind:this={input} bind:value={draft} aria-label="给艾玛的消息" placeholder={configured ? '想和艾玛说些什么？' : '在设置中连接模型后，就能聊天啦'} maxlength="4000" rows="1"
      onkeydown={keydown} oncompositionstart={() => composing = true} oncompositionend={() => composing = false}
      oninput={() => localStorage.setItem('ema-companion-draft', draft)}></textarea>
    {#if busy}
      <button class="stop" onclick={() => invoke('companion_stop').catch(error => status = String(error))} title="停止回复">停止</button>
    {:else}
      <button disabled={!configured || !draft.trim()} onclick={send} title="Enter 发送">发送</button>
    {/if}
  </div>
  <div class="footer">
    <div class="status" role="status">{status}</div>
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
  .collapse:hover { background: #eedfe3; }
  .collapsed-entry { box-sizing: border-box; display: flex; align-items: center; justify-content: center; gap: 10px; width: 100%; height: 100%; border: 1px solid #d9c9c9; border-radius: 15px; padding: 0 10px; background: #fff9f7; color: #57434a; white-space: nowrap; overflow: hidden; }
  button:focus-visible { outline: 2px solid #b97790; outline-offset: -2px; }
  @media (prefers-color-scheme: dark) { .composer, .collapsed-entry { background: #292328; color: #f1e5e8; border-color: #61505a; } .status { color: #bca5af; } .collapse:hover { background: #49353f; } }
</style>
