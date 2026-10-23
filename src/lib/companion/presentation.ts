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
  /** Complete reply and prepared speech arrive as one presentation. */
  audio?: string;
}

interface Reading {
  id: string;
  request: number;
  buffer: ReplyChunkBuffer;
  page: object | null;
  audioReceived: boolean;
  reading: boolean;
  waitingAudio: boolean;
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
  let audioStartTimer: ReturnType<typeof setTimeout> | null = null;
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
    if (audioStartTimer !== null) clearTimeout(audioStartTimer);
    audioStartTimer = null;
    options.audio.stop();
    if (active) {
      readingLease(active, active.reading);
      if (active.waitingAudio) { active.waitingAudio = false; pump(active); }
    }
    playbackFinished();
  }

  function stop() {
    clearTimers();
    const previous = active;
    active = null;
    stopAudio();
    if (previous) readingLease(previous, false);
    options.hide?.();
  }

  function pump(current: Reading, force = false) {
    if (disposed || active !== current || current.page || current.waitingAudio) return;
    if (!options.allowed()) { stop(); return; }
    let text = current.buffer.take(force);
    while (text !== null && !text.trim()) text = current.buffer.take(force);
    if (text === null) {
      options.hide?.();
      if (current.buffer.done) readingLease(current, false);
      // Give the first packets a chance to grow, then start an unfinished
      // sentence without waiting for the complete model response.
      if (!force && !current.buffer.done && current.buffer.hasPending && flushTimer === null) {
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
      autoClose: false,
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

  await listen<Reply>('companion-reply', async event => {
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
      active = { id: payload.id, request: payload.request, buffer: new ReplyChunkBuffer(), page: null, audioReceived: false, reading: false, waitingAudio: false };
      options.audio.stop();
      options.releasePreset();
    }
    const current = active;
    const paired = !!payload.audio && !current.audioReceived && !audioDisabled;
    if (paired) current.waitingAudio = true;
    if (current.buffer.append(payload.text, payload.done !== false)) pump(current);
    if (paired) await playAudio(current, payload.audio!);
  }).then(track);

  async function playAudio(current: Reading, data: string) {
    if (disposed || audioDisabled || !options.allowed() || active !== current || current.audioReceived) return;
    current.audioReceived = true;
    ownsAudio = true;
    readingLease(current, current.reading);
    audioLeaseTimer = setInterval(() => {
      if (active === current && ownsAudio) readingLease(current, current.reading);
    }, 10000);
    let finished = false;
    const startText = () => {
      if (disposed || active !== current || !ownsAudio || !options.allowed()) return;
      if (audioStartTimer !== null) clearTimeout(audioStartTimer);
      audioStartTimer = null;
      if (current.waitingAudio) { current.waitingAudio = false; pump(current); }
    };
    const finish = () => {
      if (finished || disposed || active !== current || !ownsAudio) return;
      finished = true;
      ownsAudio = false;
      if (audioLeaseTimer !== null) clearInterval(audioLeaseTimer);
      audioLeaseTimer = null;
      if (audioStartTimer !== null) clearTimeout(audioStartTimer);
      audioStartTimer = null;
      if (current.waitingAudio) { current.waitingAudio = false; pump(current); }
      readingLease(current, current.reading);
      playbackFinished();
    };
    // A local decoder that never starts must not hold the text indefinitely.
    if (current.waitingAudio) audioStartTimer = setTimeout(stopAudio, 5000);
    try {
      if (!await options.audio.playUrl(`data:audio/mpeg;base64,${data}`, finish, startText)) finish();
    } catch {
      finish();
    }
    // Legacy late audio does not rewind a reading queue that already started.
  }
  await listen<{id: string; data: string; request: number}>('companion-audio', async event => {
    const payload = event.payload;
    const current = active;
    if (!current || current.id !== payload.id || current.request !== payload.request) return;
    await playAudio(current, payload.data);
  }).then(track);

  await listen('companion-stop', stop).then(track);
  await listen<{no_audio_mode?: boolean; silence_mode?: boolean; show_character?: boolean}>('settings-change', event => {
    if (event.payload.silence_mode || event.payload.show_character === false) {
      if (event.payload.no_audio_mode !== undefined) audioDisabled = event.payload.no_audio_mode;
      stop(); return;
    }
    if (event.payload.no_audio_mode !== undefined) {
      audioDisabled = event.payload.no_audio_mode;
      if (audioDisabled) stopAudio();
    }
  }).then(track);

  return {
    isActive: () => ownsAudio || !!(active && (active.waitingAudio || !active.buffer.done || active.buffer.hasPending || active.page)),
    stop,
    destroy: () => { disposed = true; stop(); cleanups.forEach(fn => fn()); },
  };
}
