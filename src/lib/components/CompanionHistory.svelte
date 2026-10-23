<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import type { Turn, Observation } from '$lib/companion/types';
  let { turns, working = false, onForget, onChanged }: { turns: Turn[]; working?: boolean; onForget: (id: string) => void; onChanged: () => void } = $props();
  let observations = $state<Record<string, Observation>>({});
  let error = $state('');
  let preview = $state<{observation_id:string;dialogue_count:number;observation_count:number;history_epoch:number;preview_token:string} | null>(null);
  function domain(url: string) { try { return new URL(url).hostname; } catch { return ''; } }
  async function inspect(id: string) {
    try { const values = await invoke<Observation[]>('companion_get_observations', {ids:[id]}); if (values[0]) observations[id] = values[0]; }
    catch (cause) { error = String(cause); }
  }
  async function openSource(turnId: string, sourceId: string) {
    try { await invoke('companion_open_source', {turnId, sourceId}); } catch (cause) { error = String(cause); }
  }
  async function previewForget(id: string) {
    try { preview = await invoke('companion_forget_observation_preview', {id}); } catch (cause) { error = String(cause); }
  }
  async function forgetObservation() {
    if (!preview) return;
    try {
      await invoke('companion_forget_observation', {id:preview.observation_id,previewToken:preview.preview_token,expectedHistoryEpoch:preview.history_epoch});
      preview = null; observations = {}; onChanged();
    } catch (cause) { preview = null; error = `删除范围可能已变化，请重新预览。${String(cause)}`; }
  }
</script>

<section class="history" aria-label="最近对话与观察">
  <h3>最近的共同记忆</h3>
  {#if error}<p class="hint">{error}</p>{/if}
  {#if preview}<div class="confirm"><p>将遗忘这次观察，以及直接关联的 {preview.dialogue_count} 条对话、{preview.observation_count} 条观察。其他对话不在此次范围内。</p><button disabled={working} onclick={forgetObservation}>确认遗忘观察及相关对话</button><button onclick={() => preview = null}>取消</button></div>{/if}
  {#each turns as turn (turn.id)}
    <article>
      <small>{new Date(turn.created_at).toLocaleString()} · {turn.interaction_kind === 'screen_question' ? '带屏幕提问' : turn.source === 'screen' ? '观察发言' : turn.source === 'care' ? '自动关怀（仅聊天历史）' : '对话'}{turn.finish_kind === 'provider_limit' ? ' · 回答未完成' : turn.finish_kind === 'user_limit' ? ' · 已达用户字数上限' : ''}</small>
      {#if turn.user_text}<p>{turn.user_text}</p>{/if}<p><strong>桌宠：</strong>{turn.reply}</p>
      {#if turn.japanese}<p class="hint">{turn.japanese}</p>{/if}
      {#each turn.observation_refs ?? [] as ref (ref.observation_id)}
        <details ontoggle={(event) => { if ((event.currentTarget as HTMLDetailsElement).open && !observations[ref.observation_id]) void inspect(ref.observation_id); }}>
          <summary>相关屏幕观察</summary>
          {#if observations[ref.observation_id]}{@const observation = observations[ref.observation_id]}<p class="hint">{new Date(observation.observed_at).toLocaleString()} 的画面记录</p><p>{observation.summary}</p><button disabled={working} onclick={() => previewForget(ref.observation_id)}>忘记这次观察及相关对话</button>{:else}<p class="hint">正在读取，或此观察已被遗忘</p>{/if}
        </details>
      {/each}
      {#if turn.web_context?.sources?.length}
        <aside aria-label="本轮检索参考"><h4>本轮检索参考</h4><p class="hint">查询时间：{turn.web_context.completed_at ? new Date(turn.web_context.completed_at).toLocaleString() : '未知'}</p>
          {#each turn.web_context.sources as source (source.id)}<button class="source" onclick={() => openSource(turn.id, source.id)}>{source.title}<small>{domain(source.url)} · 资料日期：{source.published_at || source.date || '未知'}</small></button>{/each}
        </aside>
      {/if}
      <button disabled={working} title="遗忘本条正文与它创建的观察；其他已有对话正文保留" onclick={() => onForget(turn.id)}>忘记这条对话</button>
    </article>
  {/each}
</section>

<style>
  article { border-top:1px solid #e5dce0; padding:15px 0; overflow-wrap:anywhere; }
  p { white-space:pre-wrap; line-height:1.7; font-size:13px; }
  small,.hint { color:#7c6872; font-size:11px; }
  button { margin:5px 7px 3px 0; padding:7px 10px; border:1px solid #d5c6cd; border-radius:6px; color:inherit; background:transparent; font-size:12px; cursor:pointer; }
  button:disabled { opacity:.5; cursor:default; }
  details { margin:10px 0; padding:10px; background:#f5eff1; border-radius:6px; }
  summary { cursor:pointer; font-size:12px; }
  aside,.confirm { padding:12px; margin:12px 0; border:1px solid #dcd0d5; border-radius:8px; }
  h4 { margin:0; font-size:12px; }
  .source { display:block; width:100%; text-align:left; overflow-wrap:anywhere; }
  .source small { display:block; margin-top:4px; }
</style>
