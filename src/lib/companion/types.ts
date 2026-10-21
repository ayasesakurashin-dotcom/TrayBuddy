export interface Endpoint { base_url: string; model: string; api_key?: string; has_key: boolean }
export interface CompanionSettings {
  schema_version: number; input_enabled: boolean; input_collapsed: boolean; speech_enabled: boolean; observation_enabled: boolean; memory_enabled: boolean;
  chat: Endpoint; vision: Endpoint; speech: Endpoint; memory: Endpoint;
  voice_id: string; speech_speed: number; observation_interval_secs: number;
  capture_scope: string; capture_region: [number, number, number, number]; excluded_windows: string[];
  /** Total reply limit in Unicode characters; 0 leaves the model output uncapped by the app. */
  reply_max_chars: number; nickname: string;
}
export interface CompanionView { settings: CompanionSettings; bank_id: string; status: string; pending: number; busy: boolean; summary: string }
export interface Turn { id: string; source: string; user_text: string; reply: string; japanese: string; created_at: string }
export function isSendKey(event: Pick<KeyboardEvent, 'key' | 'shiftKey' | 'isComposing' | 'keyCode'>): boolean {
  return event.key === 'Enter' && !event.shiftKey && !event.isComposing && event.keyCode !== 229;
}
