export type ThinkingLevel = 'off' | 'low' | 'high' | 'max';
export interface Endpoint { base_url: string; model: string; api_key?: string; has_key: boolean; thinking?: ThinkingLevel; thinking_format?: 'auto' | 'deepseek' | 'reasoning_effort' }
export interface RoleSettings {display_name:string;persona:string;voice_locale:string;voice_id:string}
export interface RuntimeRole extends RoleSettings {id:string}
export interface CompanionSettings {
  characters?:Record<string,RoleSettings>;
  schema_version: number; input_enabled: boolean; input_collapsed: boolean; speech_enabled: boolean; observation_enabled: boolean; memory_enabled: boolean;
  chat: Endpoint; vision: Endpoint; speech: Endpoint; memory: Endpoint;
  voice_id: string; speech_speed: number; observation_interval_secs: number;
  capture_scope: string; capture_region: [number, number, number, number]; excluded_windows: string[];
  /** Total reply limit in Unicode characters; 0 leaves the model output uncapped by the app. */
  reply_max_chars: number; nickname: string;
  web_search?: { enabled: boolean };
}
export interface ObservationRef { observation_id: string; relation_kind: string; ordinal: number }
export interface Observation { id: string; observed_at: string; summary: string; visible_text?: string; source_label?: string; uncertainties?: string[] }
export interface SearchSource { id: string; title: string; url: string; published_at?: string | null; date?: string | null; retrieved_at?: string; excerpt?: string; evidence_kind?: string }
export interface WebSearchContext { status: string; attempted: boolean; completed_at?: string | null; sources: SearchSource[] }
export interface CompanionView { settings: CompanionSettings; bank_id: string; status: string; pending: number; busy: boolean; summary: string; history_epoch?: number; request?: number | null; last_observation?: Observation | null; character_name?: string; warning?: string | null; notice?: string | null }
export interface Turn { id: string; source: string; user_text: string; reply: string; japanese: string; created_at: string; interaction_kind?: string; finish_kind?: string; observation_refs?: ObservationRef[]; web_context?: WebSearchContext | null }
export type ScreenSelection = { mode: 'none' } | { mode: 'fresh'; target_token: string } | { mode: 'reference'; observation_ids: string[] };
export interface ChatSendPayload { client_message_id: string; expected_history_epoch: number; text: string; screen: ScreenSelection }
export interface PreparedScreenTarget { target_token: string; label: string; scope: 'foreground'; expires_at: string }
export interface SendOutcome { request: number | null; turn_id: string | null; result: 'completed' | 'failed' | 'cancelled'; text_committed: boolean; observation_ids: string[]; history_epoch: number; replayed: boolean; error_code?: string | null; warning_code?: string | null }
export interface RequestReceipt { client_message_id: string; history_epoch: number; state: 'pending' | 'text_committed' | 'finished' | 'interrupted'; request: number | null; turn_id: string | null; text_committed: boolean; outcome: SendOutcome | null }
export interface CompanionStatus { message: string; busy: boolean; request?: number | null; client_message_id?: string | null; kind?: string; stage?: string; event_seq?: number; warning?: string | null; notice?: string | null }
export function isSendKey(event: Pick<KeyboardEvent, 'key' | 'shiftKey' | 'isComposing' | 'keyCode'>): boolean {
  return event.key === 'Enter' && !event.shiftKey && !event.isComposing && event.keyCode !== 229;
}
