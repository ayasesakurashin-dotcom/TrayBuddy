import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { AudioManager } from '$lib/audio/AudioManager';
import type { BubbleConfig } from '$lib/bubble/BubbleManager.svelte';

export async function attachCompanion(options: {
  audio: AudioManager;
  show: (config: BubbleConfig) => void;
  releasePreset: () => void;
  allowed: () => boolean;
}) {
  let until = 0;
  let readUntil = 0;
  let active: {id: string; text: string; request: number} | null = null;
  let ownsAudio = false;
  let disposed = false;
  const cleanups: Array<() => void> = [];
  const track = (fn: () => void) => { if (disposed) fn(); else cleanups.push(fn); };
  function stop() {
    active = null; until = 0; readUntil = 0;
    if (ownsAudio) { ownsAudio = false; options.audio.stop(); }
    void invoke('companion_playback_finished').catch(() => {});
  }
  await listen<{id: string; text: string; source: string; request: number}>('companion-reply', event => {
    if (!options.allowed()) return;
    stop(); active = {...event.payload, text: event.payload.text.replace(/\[([^\]]+)\]\([^)]+\)/g, '$1')};
    until = Date.now() + Math.min(18000, Math.max(5500, Array.from(active.text).length * 100));
    readUntil = until;
    options.audio.stop(); options.releasePreset();
    // Companion text is plain text. Do not create clickable model-generated URLs.
    const text = active.text;
    options.show({text, duration: until - Date.now(), typeSpeed: 22});
  }).then(track);
  await listen<{id: string; data: string; request: number}>('companion-audio', async event => {
    const payload = event.payload;
    if (!options.allowed() || !active || active.id !== payload.id || active.request !== payload.request) return;
    const current = active;
    ownsAudio = true; until = Date.now() + 60000;
    options.show({text: current.text, duration: 60000, typeSpeed: 1});
    await options.audio.playUrl(`data:audio/mpeg;base64,${payload.data}`, () => {
      if (active?.id === current.id) {
        ownsAudio = false; until = Math.max(readUntil, Date.now() + 3500);
        options.show({text: current.text, duration: until - Date.now(), typeSpeed: 1});
        void invoke('companion_playback_finished').catch(() => {});
      }
    });
  }).then(track);
  await listen('companion-stop', stop).then(track);
  // Yield to DSH's existing system-level notification, without touching DSH itself.
  await listen('dsh-notify', stop).then(track);
  await listen<{no_audio_mode?: boolean; silence_mode?: boolean; show_character?: boolean}>('settings-change', event => {
    if (event.payload.no_audio_mode || event.payload.silence_mode || event.payload.show_character === false) stop();
  }).then(track);
  return { isActive: () => Date.now() < until, stop, destroy: () => { disposed = true; stop(); cleanups.forEach(fn => fn()); } };
}
