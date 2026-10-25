<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import type { LocalVoiceSample, RuntimeRole } from '$lib/companion/types';

  let { role, serviceUrl, working, run, prepareUpload, afterUpload, play, report }: {
    role: RuntimeRole;
    serviceUrl: string;
    working: boolean;
    run: (fn: () => Promise<unknown>) => Promise<void>;
    prepareUpload: () => Promise<void>;
    afterUpload: () => Promise<void>;
    play: (data: string) => Promise<unknown>;
    report: (message: string) => void;
  } = $props();

  let localSamples = $state<LocalVoiceSample[]>([]);
  let presetSamples = $state<Array<{ name: string; candidate_text: string }>>([]);
  let selectedLocal = $state<string[]>([]);
  let selectedPresets = $state<string[]>([]);
  let loading = $state(true);
  let loadFailed = $state(false);
  let confirmation = $state<{ roleId: string; roleName: string; url: string; localIds: string[]; names: string[] } | null>(null);
  const selectedCount = $derived(selectedLocal.length + selectedPresets.length);

  $effect(() => {
    const id = role.id;
    let disposed = false;
    localSamples = []; presetSamples = []; selectedLocal = []; selectedPresets = []; confirmation = null;
    loading = true; loadFailed = false;
    void invoke<LocalVoiceSample[]>('companion_voice_local_samples', { roleId: id })
      .then(samples => { if (!disposed) localSamples = samples; })
      .catch(error => { if (!disposed) { loadFailed = true; report(String(error)); } })
      .finally(() => { if (!disposed) loading = false; });
    return () => { disposed = true; };
  });

  async function importAudio() {
    const roleId = role.id;
    const samples = await invoke<LocalVoiceSample[] | null>('companion_voice_import', { roleId });
    if (role.id !== roleId) return '角色已切换，请查看对应角色的暂存素材';
    if (!samples) return '已取消导入';
    localSamples = samples; confirmation = null; loadFailed = false;
    return '音声已暂存在本机，尚未上传；关闭设置或重启后仍可使用';
  }

  async function removeAudio(id: string) {
    const roleId = role.id;
    const samples = await invoke<LocalVoiceSample[]>('companion_voice_local_remove', { roleId, id });
    if (role.id !== roleId) return '已移除对应角色的本地素材';
    localSamples = samples;
    selectedLocal = selectedLocal.filter(value => value !== id);
    confirmation = null;
    return '已移除本地素材；已创建的声音仍可使用';
  }

  async function loadPresets() {
    const roleId = role.id;
    const samples = await invoke<Array<{ name: string; candidate_text: string }>>('companion_voice_samples', { roleId });
    if (role.id !== roleId) return '角色已切换，请重新选择素材';
    presetSamples = samples; selectedPresets = []; confirmation = null;
    return samples.length ? '可先试听，再选择清晰自然的语音片段' : '当前角色没有可用的预设音声，可以导入本地录音';
  }

  function reviewUpload() {
    confirmation = { roleId: role.id, roleName: role.display_name, url: serviceUrl,
      localIds: [...selectedLocal], names: [...selectedPresets] };
  }

  async function createVoice() {
    const selection = confirmation;
    if (!selection) return;
    if (role.id !== selection.roleId || serviceUrl !== selection.url) {
      confirmation = null;
      throw new Error('角色或语音服务已变化，请重新确认上传');
    }
    await prepareUpload();
    const id = await invoke<string>('companion_voice_create', {
      roleId: selection.roleId, names: selection.names, localIds: selection.localIds,
    });
    confirmation = null;
    await afterUpload();
    return `私有声音已建立并绑定到${selection.roleName}：${id}。本地素材已保留`;
  }

  function sizeLabel(size: number) { return `${(size / 1_000_000).toFixed(2)} MB`; }
</script>

<section aria-label="声音素材">
  <h3>声音素材</h3>
  <p class="hint">本地导入仅暂存到当前角色，支持 WAV、MP3；关闭设置或重启后保留。每个角色最多 8 段，单段 8MB、总量 20MB。选择并确认上传后，才会向已配置的语音服务提交素材。</p>
  <button disabled={working || loading || loadFailed} onclick={() => run(importAudio)}>导入音声到本地暂存</button>
  <button disabled={working} onclick={() => run(loadPresets)}>选择当前角色预设声音素材</button>
  {#if loading}<p class="hint">正在读取本地暂存…</p>
  {:else if !localSamples.length && !loadFailed}<p class="hint">当前角色暂无本地暂存音声。</p>{/if}
  {#if localSamples.length}
    <div class="samples" aria-label="本地暂存音声">
      {#each localSamples as sample (sample.id)}
        <div class="sample">
          <label><input type="checkbox" disabled={working} bind:group={selectedLocal} value={sample.id} onchange={() => confirmation = null} /><span>{sample.name}<small>本地暂存 · {sizeLabel(sample.size)}</small></span></label>
          <div><button disabled={working} aria-label={`试听本地音声 ${sample.name}`} onclick={() => run(async () => play(await invoke<string>('companion_voice_local_audio', { roleId: role.id, id: sample.id })))}>试听原音</button><button disabled={working} aria-label={`移除本地音声 ${sample.name}`} onclick={() => run(() => removeAudio(sample.id))}>移除</button></div>
        </div>
      {/each}
    </div>
  {/if}
  {#if presetSamples.length}
    <div class="samples" aria-label="角色预设音声">
      {#each presetSamples as sample (sample.name)}
        <div class="sample"><label><input type="checkbox" disabled={working} bind:group={selectedPresets} value={sample.name} onchange={() => confirmation = null} />{sample.name}</label><button disabled={working} onclick={() => run(async () => play(await invoke<string>('companion_voice_sample_audio', { roleId: role.id, name: sample.name })))}>试听原音</button></div>
      {/each}
    </div>
  {/if}
  <p class="hint">已选 {selectedCount} 段，创建时合计选择 1—8 段、总量不超过 20MB。服务端会识别录音文本；角色台词不作为录音逐字稿。</p>
  <button disabled={working || selectedCount === 0 || selectedCount > 8} onclick={reviewUpload}>用所选素材建立私有声音</button>
  {#if confirmation}
    <div class="confirm" role="group" aria-label="确认上传声音素材">
      <p>将为{confirmation.roleName}向已配置的 Fish Audio 服务上传 {confirmation.localIds.length + confirmation.names.length} 段素材，建立私有声音。创建成功后替换该角色的声音 ID，本地素材继续保留；失败时保留原声音。</p>
      <p class="destination">服务地址：{confirmation.url}</p>
      <ul>{#each localSamples.filter(sample => confirmation?.localIds.includes(sample.id)) as sample}<li>{sample.name}（本地暂存）</li>{/each}{#each confirmation.names as name}<li>{name}（角色预设）</li>{/each}</ul>
      <button disabled={working} onclick={() => run(createVoice)}>上传并创建私有声音</button>
      <button disabled={working} onclick={() => confirmation = null}>继续暂存</button>
    </div>
  {/if}
</section>

<style>
  h3 { font-size: 13px; margin: 18px 0 8px; }
  .hint, small { color: #7c6872; line-height: 1.7; font-size: 12px; }
  button { margin: 5px 7px 3px 0; padding: 8px 12px; border: 1px solid #d5c6cd; border-radius: 6px; color: inherit; background: transparent; font-size: 12px; cursor: pointer; }
  button:disabled { opacity: .5; cursor: default; }
  .samples { max-height: 230px; overflow-y: auto; margin: 12px 0; border: 1px solid #dcd0d5; padding: 8px; border-radius: 6px; }
  .sample { display: flex; align-items: center; justify-content: space-between; gap: 8px; font-size: 12px; }
  .sample label { display: flex; align-items: center; gap: 6px; min-width: 0; overflow-wrap: anywhere; }
  .sample div { flex-shrink: 0; } small { display: block; font-size: 10px; }
  .confirm { border-left: 2px solid #854b60; padding: 1px 12px 10px; font-size: 12px; line-height: 1.7; }
  .destination { overflow-wrap: anywhere; }
  @media (max-width: 450px) { .sample { align-items: flex-start; flex-direction: column; padding-bottom: 8px; } }
  @media (prefers-color-scheme: dark) { .hint, small { color: #bba6b1; } .samples { border-color: #66545e; } }
</style>
