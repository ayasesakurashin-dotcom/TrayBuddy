import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { AudioManager } from '$lib/audio/AudioManager';
import type { BubbleConfig } from '$lib/bubble/BubbleManager.svelte';
import { ReplyChunkBuffer, REPLY_READ_HOLD_MS, REPLY_STREAM_FLUSH_MS } from './replyChunks';

interface Reply {
  id: string;
  text: string;
  source?: string;
  request: number;
  /** Older backends sent only one complete reply without this flag. */
  done?: boolean;
  error?: string;
}

interface Reading {
  id: string;
  request: number;
  buffer: ReplyChunkBuffer;
  page: object | null;
  audioReceived: boolean;
  reading: boolean;
}

export async function attachCompanion(options: {
  audio: AudioManager;
  show: (config: BubbleConfig) => void;
  hide?: () => void;
  releasePreset: () => void;
  allowed: () => boolean;
}) {
  let active: Reading | null = null;
  let newestRequest = -1;
  let ownsAudio = false;
  let audioDisabled = false;
  let disposed = false;
  let holdTimer: ReturnType<typeof setTimeout> | null = null;
  let flushTimer: ReturnType<typeof setTimeout> | null = null;
  let audioLeaseTimer: ReturnType<typeof setInterval> | null = null;
  const cleanups: Array<() => void> = [];
  const track = (fn: () => void) => { if (disposed) fn(); else cleanups.push(fn); };
  const playbackFinished = () => { void invoke('companion_playback_finished').catch(() => {}); };
  function readingLease(current: Reading, reading: boolean) {
    current.reading = reading;
    void invoke('companion_reading', {request: current.request, active: reading || ownsAudio}).catch(() => {});
  }

  function clearTimers() {
    if (holdTimer !== null) clearTimeout(holdTimer);
    if (flushTimer !== null) clearTimeout(flushTimer);
    holdTimer = null;
    flushTimer = null;
  }

  function stopAudio() {
    if (!ownsAudio) return;
    // Invalidate ownership before stop(), which may call the completion callback.
    ownsAudio = false;
    if (audioLeaseTimer !== null) clearInterval(audioLeaseTimer);
    audioLeaseTimer = null;
    options.audio.stop();
    if (active) readingLease(active, active.reading);
    playbackFinished();
  }

  function stop() {
    clearTimers();
    stopAudio();
    if (active) readingLease(active, false);
    active = null;
    options.hide?.();
  }

  function pump(current: Reading, force = false) {
    if (disposed || active !== current || current.page) return;
    if (!options.allowed()) { stop(); return; }
    let text = current.buffer.take(force);
    while (text !== null && !text.trim()) text = current.buffer.take(force);
    if (text === null) {
      if (current.buffer.done) readingLease(current, false);
      // Give the first packets a chance to grow, then start an unfinished
      // sentence without waiting for the complete model response.
      if (!current.buffer.done && current.buffer.hasPending && flushTimer === null) {
        flushTimer = setTimeout(() => {
          flushTimer = null;
          pump(current, true);
        }, REPLY_STREAM_FLUSH_MS);
      }
      return;
    }
    if (flushTimer !== null) clearTimeout(flushTimer);
    flushTimer = null;
    const page = {};
    current.page = page;
    // Renew a short backend lease for each page, so automatic observations
    // cannot interrupt a long reply after the model has finished generating.
    readingLease(current, true);
    let completed = false;
    options.show({
      text,
      duration: REPLY_READ_HOLD_MS,
      typeSpeed: 16,
      plainText: true,
      onTextComplete: () => {
        if (completed || disposed || active !== current || current.page !== page) return;
        completed = true;
        // Reading time starts after the last character is actually visible,
        // including when the user clicks to finish the typewriter animation.
        holdTimer = setTimeout(() => {
          holdTimer = null;
          if (active !== current) return;
          current.page = null;
          pump(current);
        }, REPLY_READ_HOLD_MS);
      },
    });
  }

  await listen<Reply>('companion-reply', event => {
    const payload = event.payload;
    if (disposed || !options.allowed() || payload.request < newestRequest) return;
    // Cancellation can finish before even the first text packet is delivered.
    // Its terminal error must not create a brand new reading queue after stop.
    if (payload.error && (!active || active.request !== payload.request || active.id !== payload.id)) return;
    if (!active || active.request !== payload.request || active.id !== payload.id) {
      // A cancelled request cannot resurrect itself through a delayed final
      // event, and previous requests cannot replace the current reading queue.
      if (payload.request <= newestRequest) return;
      newestRequest = payload.request;
      stop();
      active = { id: payload.id, request: payload.request, buffer: new ReplyChunkBuffer(), page: null, audioReceived: false, reading: false };
      options.audio.stop();
      options.releasePreset();
    }
    if (active.buffer.append(payload.text, payload.done !== false)) pump(active);
  }).then(track);

  await listen<{id: string; data: string; request: number}>('companion-audio', async event => {
    const payload = event.payload;
    const current = active;
    if (disposed || audioDisabled || !options.allowed() || !current || current.id !== payload.id || current.request !== payload.request || current.audioReceived) return;
    current.audioReceived = true;
    ownsAudio = true;
    readingLease(current, current.reading);
    audioLeaseTimer = setInterval(() => {
      if (active === current && ownsAudio) readingLease(current, current.reading);
    }, 10000);
    let finished = false;
    const finish = () => {
      if (finished || disposed || active !== current || !ownsAudio) return;
      finished = true;
      ownsAudio = false;
      if (audioLeaseTimer !== null) clearInterval(audioLeaseTimer);
      audioLeaseTimer = null;
      readingLease(current, current.reading);
      playbackFinished();
    };
    try {
      if (!await options.audio.playUrl(`data:audio/mpeg;base64,${payload.data}`, finish)) finish();
    } catch {
      finish();
    }
    // Audio arrival/completion never changes the reading cursor or shows text.
  }).then(track);

  await listen('companion-stop', stop).then(track);
  await listen<{no_audio_mode?: boolean; silence_mode?: boolean; show_character?: boolean}>('settings-change', event => {
    if (event.payload.no_audio_mode !== undefined) {
      audioDisabled = event.payload.no_audio_mode;
      if (audioDisabled) stopAudio();
    }
    if (event.payload.silence_mode || event.payload.show_character === false) stop();
  }).then(track);

  return {
    isActive: () => ownsAudio || !!(active && (!active.buffer.done || active.buffer.hasPending || active.page)),
    stop,
    destroy: () => { disposed = true; stop(); cleanups.forEach(fn => fn()); },
  };
}
