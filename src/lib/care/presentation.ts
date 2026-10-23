import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { BubbleConfig } from '$lib/bubble/BubbleManager.svelte';
import { ReplyChunkBuffer, REPLY_READ_HOLD_MS } from '$lib/companion/replyChunks';

interface Offer {
  delivery_id: string;
  generation: number;
  interaction_epoch: number;
  text: string;
  expires_at: number;
  audio?: string;
}

/** Owns care presentation only; a user conversation always takes priority. */
export async function attachCare(options: {
  show: (config: BubbleConfig) => void;
  hide: () => void;
  allowed: () => boolean;
  companionActive: () => boolean;
  presetActive?: () => boolean;
  releasePreset?: () => void;
}) {
  let active: Offer | null = null;
  let newest = 0;
  let disposed = false;
  let audioDisabled = false;
  let displayed = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let startTimer: ReturnType<typeof setTimeout> | null = null;
  let audio: HTMLAudioElement | null = null;
  let buffer = new ReplyChunkBuffer();
  let page: object | null = null;
  let textDone = false;
  let holdTimer: ReturnType<typeof setTimeout> | null = null;
  const cleanups: Array<() => void> = [];
  const track = (fn: () => void) => { if (disposed) fn(); else cleanups.push(fn); };
  const receipt = (command: string, offer: Offer) => invoke(command, {
    deliveryId: offer.delivery_id, generation: offer.generation,
  }).catch(() => false);
  const blocked = () => options.companionActive() || !!options.presetActive?.();
  const current = (offer: Offer) => !disposed && active === offer && newest === offer.generation
    && options.allowed() && !blocked();

  function stopAudio() {
    if (startTimer !== null) clearTimeout(startTimer);
    startTimer = null;
    const old = audio;
    audio = null;
    if (old) {
      old.onended = null; old.onerror = null; old.onplaying = null;
      old.pause(); old.src = '';
    }
  }
  function stop() {
    stopAudio();
    if (timer !== null) clearTimeout(timer);
    timer = null;
    if (holdTimer !== null) clearTimeout(holdTimer);
    holdTimer = null; page = null; textDone = false;
    buffer = new ReplyChunkBuffer();
    const old = active;
    active = null;
    if (old && displayed && !blocked()) options.hide();
    displayed = false;
    if (old) void receipt('care_finished', old);
  }
  function pump(offer: Offer) {
    if (!current(offer) || page) return;
    const text = buffer.take();
    if (text === null) {
      textDone = true;
      if (!audio) stop();
      return;
    }
    const token = {};
    page = token;
    let completed = false;
    options.show({text, duration: REPLY_READ_HOLD_MS, typeSpeed: 20, plainText: true, autoClose: false,
      onTextComplete: () => {
        if (completed || !current(offer) || page !== token) return;
        completed = true;
        holdTimer = setTimeout(() => {
          holdTimer = null;
          if (!current(offer) || page !== token) return;
          page = null; pump(offer);
        }, REPLY_READ_HOLD_MS);
      },
    });
  }
  function showText(offer: Offer) {
    if (!current(offer) || displayed) return;
    if (startTimer !== null) clearTimeout(startTimer);
    startTimer = null;
    displayed = true;
    buffer.append(offer.text, true);
    pump(offer);
    void receipt('care_presented', offer).then(accepted => {
      if (!accepted && active === offer) stop();
    });
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(stop, 60000);
  }
  function textFallback(offer: Offer) {
    if (!current(offer)) return;
    stopAudio();
    showText(offer);
    if (textDone) stop();
  }

  await listen<{generation: number}>('care-cancel', event => {
    newest = Math.max(newest, event.payload.generation); stop();
  }).then(track);
  await listen<Offer>('care-offer', async event => {
    const offer = event.payload;
    if (disposed || offer.generation <= newest || !options.allowed() || blocked()) {
      void receipt('care_rejected', offer); return;
    }
    newest = offer.generation;
    const valid = await receipt('care_validate_offer', offer);
    if (!valid || disposed || newest !== offer.generation || !options.allowed() || blocked()) {
      void receipt('care_rejected', offer); return;
    }
    stop(); active = offer;
    options.releasePreset?.();
    if (!offer.audio || audioDisabled) { showText(offer); return; }
    // Local decoding is bounded inside the backend's short presentation lease.
    startTimer = setTimeout(() => textFallback(offer), 1000);
    try {
      const settings = await invoke<{no_audio_mode: boolean; volume: number}>('get_settings');
      if (!current(offer)) return;
      if (displayed || settings.no_audio_mode || audioDisabled) { textFallback(offer); return; }
      const owned = new Audio(`data:audio/mpeg;base64,${offer.audio}`);
      audio = owned;
      owned.volume = Math.max(0, Math.min(1, settings.volume ?? 1));
      owned.onplaying = () => { if (audio === owned) showText(offer); };
      owned.onerror = () => { if (audio === owned) textFallback(offer); };
      owned.onended = () => { if (audio === owned) textFallback(offer); };
      await owned.play();
      if (!current(offer) && audio === owned) stopAudio();
    } catch { if (current(offer)) textFallback(offer); }
  }).then(track);
  await listen<{silence_mode?: boolean; show_character?: boolean; no_audio_mode?: boolean}>('settings-change', event => {
    if (event.payload.no_audio_mode !== undefined) audioDisabled = event.payload.no_audio_mode;
    if (event.payload.silence_mode || event.payload.show_character === false) stop();
    else if (audioDisabled && active) textFallback(active);
  }).then(track);
  return {stop, isActive: () => active !== null, destroy: () => {
    disposed = true; stop(); cleanups.forEach(fn => fn());
  }};
}
