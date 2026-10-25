import { beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { flushAsync } from './test-utils';
import type { CompanionView } from '$lib/companion/types';
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
import CompanionSettings from '$lib/components/CompanionSettings.svelte';

function fixture(): CompanionView {
  const empty = {base_url: '', model: '', has_key: false};
  return { bank_id: 'ema-companion-11111111-1111-4111-9111-111111111111', status: '就绪', pending: 0, busy: false, summary: '', settings: {
    schema_version: 1, input_enabled: true, input_collapsed: false, speech_enabled: true, observation_enabled: false, memory_enabled: true,
    chat: {base_url: 'https://api.deepseek.com', model: 'deepseek-flash', has_key: true}, vision: {...empty},
    speech: {base_url: 'https://api.fish.audio', model: 's2.1-pro-free', has_key: false},
    memory: {base_url: 'http://127.0.0.1:18088', model: '', has_key: false},
    voice_id: '', speech_speed: 1, observation_interval_secs: 300, capture_scope: 'foreground', capture_region: [0,0,800,600], excluded_windows: [], reply_max_chars: 120, nickname: '',
  }};
}
describe('companion settings boundaries', () => {
  beforeEach(() => { cleanup(); mocks.invoke.mockReset(); mocks.invoke.mockImplementation(async (command: string, args?: any) => {
    if (command === 'companion_get') return fixture();
    if (command === 'companion_character_get') return {id:'ema',display_name:'艾玛',persona:'当前测试人格',voice_locale:'ja',voice_id:'existing-voice'};
    if (command === 'companion_save') return {...fixture(), settings: args.settings};
    return [];
  }); });
  it('shows the bank as read-only and keeps configured API secrets out of text inputs', async () => {
    const page = render(CompanionSettings);
    await flushAsync();
    expect(page.getByText(fixture().bank_id)).toBeTruthy();
    expect(page.queryByDisplayValue(fixture().bank_id)).toBeNull();
    const password = page.container.querySelector<HTMLInputElement>('input[type="password"]');
    expect(password?.value).toBe('');
    expect(password?.placeholder).toContain('留空保持原密钥');
    const observation = page.getByLabelText('定时屏幕观察') as HTMLInputElement;
    expect(observation.checked).toBe(false);
  });
  it('saves a user reply limit without allowing a bank field to be submitted', async () => {
    const page = render(CompanionSettings);
    await flushAsync();
    expect(page.getByLabelText('回复字数上限')).toBeTruthy();
    await fireEvent.input(page.getByLabelText('回复字数上限'), {target: {value: '80'}});
    await fireEvent.click(page.getByRole('button', {name: '保存桌宠 AI 设置'}));
    await flushAsync();
    expect(mocks.invoke.mock.calls.some((call: unknown[]) => call[0] === 'companion_save')).toBe(true);
    const payload = mocks.invoke.mock.calls.find((call: unknown[]) => call[0] === 'companion_save')![1].settings;
    expect(payload.reply_max_chars).toBe(80);
    expect(payload).not.toHaveProperty('bank_id');
    expect(payload.observation_enabled).toBe(false);
  });
  it.each([0, 1500])('saves %i as the reply limit without the old 180-character ceiling', async limit => {
    const page = render(CompanionSettings);
    await flushAsync();
    const input = page.getByLabelText('回复字数上限');
    expect(input.getAttribute('max')).toBeNull();
    await fireEvent.input(input, { target: { value: String(limit) } });
    await fireEvent.click(page.getByRole('button', { name: '保存桌宠 AI 设置' }));
    await flushAsync();
    expect(mocks.invoke.mock.calls.find(call => call[0] === 'companion_save')![1].settings.reply_max_chars).toBe(limit);
  });
  it('rejects a negative reply limit before saving', async () => {
    const page = render(CompanionSettings);
    await flushAsync();
    await fireEvent.input(page.getByLabelText('回复字数上限'), { target: { value: '-1' } });
    await fireEvent.click(page.getByRole('button', { name: '保存桌宠 AI 设置' }));
    await flushAsync();
    expect(mocks.invoke.mock.calls.some(call => call[0] === 'companion_save')).toBe(false);
    expect(page.getByRole('status').textContent).toContain('非负整数');
  });
  it('defaults search on and keeps chat and vision thinking independent', async () => {
    const page=render(CompanionSettings);await flushAsync();
    expect((page.getByLabelText('允许按需网页搜索') as HTMLInputElement).checked).toBe(true);
    expect((page.getByLabelText('对话模型思考强度') as HTMLSelectElement).value).toBe('off');
    expect((page.getByLabelText('识图模型思考强度') as HTMLSelectElement).value).toBe('off');
    await fireEvent.change(page.getByLabelText('对话模型思考强度'),{target:{value:'high'}});
    await fireEvent.change(page.getByLabelText('识图模型思考接口类型'),{target:{value:'deepseek'}});
    await fireEvent.change(page.getByLabelText('识图模型思考强度'),{target:{value:'low'}});
    await fireEvent.click(page.getByRole('button',{name:'保存桌宠 AI 设置'}));await flushAsync();
    const settings=mocks.invoke.mock.calls.find(c=>c[0]==='companion_save')![1].settings;
    expect(settings.chat.thinking).toBe('high');expect(settings.vision.thinking).toBe('low');expect(settings.web_search.enabled).toBe(true);
  });
  it('persists an explicit choice to disable search',async()=>{
    const page=render(CompanionSettings);await flushAsync();await fireEvent.click(page.getByLabelText('允许按需网页搜索'));await fireEvent.click(page.getByRole('button',{name:'保存桌宠 AI 设置'}));await flushAsync();expect(mocks.invoke.mock.calls.find(c=>c[0]==='companion_save')![1].settings.web_search.enabled).toBe(false);
  });
  it('defaults uncertain search off for old settings and saves an explicit opt-in', async () => {
    const page = render(CompanionSettings); await flushAsync();
    const preference = page.getByLabelText('无法判断时默认搜索') as HTMLInputElement;
    expect(preference.checked).toBe(false);
    expect(preference.disabled).toBe(false);
    await fireEvent.click(preference);
    await fireEvent.click(page.getByRole('button',{name:'保存桌宠 AI 设置'})); await flushAsync();
    expect(mocks.invoke.mock.calls.find(c => c[0] === 'companion_save')![1].settings.web_search)
      .toMatchObject({enabled:true,search_when_uncertain:true});
  });
  it('disables the fallback control when search is off while preserving its saved preference', async () => {
    mocks.invoke.mockImplementation(async (command: string, args?: any) => {
      if (command === 'companion_get') { const view = fixture(); view.settings.web_search = {enabled:false,search_when_uncertain:true}; return view; }
      if (command === 'companion_save') return {...fixture(),settings:args.settings};
      return [];
    });
    const page = render(CompanionSettings); await flushAsync();
    const preference = page.getByLabelText('无法判断时默认搜索') as HTMLInputElement;
    expect(preference.checked).toBe(true);
    expect(preference.disabled).toBe(true);
    await fireEvent.click(page.getByLabelText('允许按需网页搜索'));
    expect(preference.disabled).toBe(false);
    await fireEvent.click(preference);
    await fireEvent.click(page.getByRole('button',{name:'保存桌宠 AI 设置'})); await flushAsync();
    expect(mocks.invoke.mock.calls.find(c => c[0] === 'companion_save')![1].settings.web_search)
      .toMatchObject({enabled:true,search_when_uncertain:false});
  });
  it('saves a different role persona and voice without overwriting the legacy voice configuration',async()=>{
    mocks.invoke.mockImplementation(async(command:string,args?:any)=>command==='companion_get'?fixture():command==='companion_character_get'?{id:'other',display_name:'小白',persona:'活泼的桌面伙伴',voice_locale:'zh',voice_id:'voice-other'}:command==='companion_save'?{...fixture(),settings:args.settings}:[]);
    const page=render(CompanionSettings);await flushAsync();expect(page.getByRole('button',{name:'保存桌宠 AI 设置'})).toBeTruthy();
    await fireEvent.input(page.getByLabelText('当前角色声音 ID'),{target:{value:'new-voice'}});await fireEvent.click(page.getByRole('button',{name:'保存桌宠 AI 设置'}));await flushAsync();
    const settings=mocks.invoke.mock.calls.find(c=>c[0]==='companion_save')![1].settings;expect(settings.characters.other).toMatchObject({persona:'活泼的桌面伙伴',voice_id:'new-voice',voice_locale:'zh'});expect(settings.voice_id).toBe('');
  });
  it('persists Japanese reading independently of a persona that requests Chinese replies',async()=>{
    mocks.invoke.mockImplementation(async(command:string,args?:any)=>command==='companion_get'?fixture():command==='companion_character_get'?{id:'ema',display_name:'艾玛',persona:'自然地用简体中文回复。',voice_locale:'zh',voice_id:'voice-ema'}:command==='companion_save'?{...fixture(),settings:args.settings}:[]);
    const page=render(CompanionSettings);await flushAsync();
    await fireEvent.change(page.getByLabelText('当前角色朗读语言'),{target:{value:'ja'}});
    await fireEvent.click(page.getByRole('button',{name:'保存桌宠 AI 设置'}));await flushAsync();
    const settings=mocks.invoke.mock.calls.find(c=>c[0]==='companion_save')![1].settings;
    expect(settings.characters.ema).toMatchObject({persona:'自然地用简体中文回复。',voice_locale:'ja'});
    expect((page.getByLabelText('当前角色朗读语言') as HTMLSelectElement).value).toBe('ja');
  });

  it('uses generic settings wording and removes the duplicate preset persona section', async () => {
    const page = render(CompanionSettings); await flushAsync();
    expect(page.getByRole('heading', {name:'桌宠 AI 设置'})).toBeTruthy();
    expect(page.queryByText('艾玛的人格')).toBeNull();
    expect(page.getByLabelText('当前角色人格提示词')).toBeTruthy();
    expect(page.getByText(/当前所有桌宠角色共用同一记忆 bank/)).toBeTruthy();
  });

  function voiceMock(options: { failUpload?: boolean; cancelled?: boolean } = {}) {
    const sample = {id:'a'.repeat(64),name:'recording.wav',size:48000,mime_type:'audio/wav'};
    let staged: typeof sample[] = [];
    mocks.invoke.mockImplementation(async (command: string, args?: any) => {
      if (command === 'companion_get') return fixture();
      if (command === 'companion_character_get') return {id:'other',display_name:'小白',persona:'友善',voice_locale:'zh',voice_id:'existing-voice'};
      if (command === 'companion_save') return {...fixture(),settings:args.settings};
      if (command === 'companion_voice_local_samples') return [...staged];
      if (command === 'companion_voice_samples') return [{name:'preset-one',candidate_text:'预设台词'}];
      if (command === 'companion_voice_local_audio') return 'data:audio/wav;base64,UklGRg==';
      if (command === 'get_settings') return {volume:0.7,no_audio_mode:false};
      if (command === 'companion_voice_import') { if (options.cancelled) return null; staged=[sample]; return [...staged]; }
      if (command === 'companion_voice_local_remove') { staged=[]; return []; }
      if (command === 'companion_voice_create') { if (options.failUpload) throw new Error('私有声音创建连接失败'); return 'new-private-voice'; }
      return [];
    });
    return sample;
  }

  async function openSpeech(page: { getByText: (text: string) => HTMLElement }) {
    await flushAsync();
    await fireEvent.click(page.getByText('生成语音'));
  }

  it('imports audio locally without saving settings or uploading and reloads it after reopening', async () => {
    voiceMock();
    const page = render(CompanionSettings); await openSpeech(page);
    await fireEvent.click(page.getByRole('button',{name:'导入音声到本地暂存'})); await flushAsync();
    expect(page.getByText('recording.wav')).toBeTruthy();
    expect(mocks.invoke.mock.calls.some(c => c[0] === 'companion_save' || c[0] === 'companion_voice_create')).toBe(false);
    expect(page.queryByRole('button',{name:'上传并创建私有声音'})).toBeNull();
    page.unmount();
    const reopened = render(CompanionSettings); await openSpeech(reopened);
    expect(reopened.getByText('recording.wav')).toBeTruthy();
    expect(mocks.invoke.mock.calls.filter(c => c[0] === 'companion_voice_local_samples').every(c => c[1].roleId === 'other')).toBe(true);
  });

  it('only uploads selected audio after explicit confirmation and lets the user keep staging it', async () => {
    const sample = voiceMock();
    const page = render(CompanionSettings); await openSpeech(page);
    await fireEvent.click(page.getByRole('button',{name:'导入音声到本地暂存'})); await flushAsync();
    await fireEvent.click(page.getByRole('checkbox', {name:/recording.wav/}));
    await fireEvent.click(page.getByRole('button',{name:'用所选素材建立私有声音'})); await flushAsync();
    expect(mocks.invoke.mock.calls.some(c => c[0] === 'companion_voice_create')).toBe(false);
    expect(page.getByText(/服务地址：https:\/\/api.fish.audio/)).toBeTruthy();
    await fireEvent.click(page.getByRole('button',{name:'继续暂存'}));
    expect(page.getByText('recording.wav')).toBeTruthy();
    await fireEvent.click(page.getByRole('button',{name:'用所选素材建立私有声音'}));
    await fireEvent.click(page.getByRole('button',{name:'上传并创建私有声音'})); await flushAsync();
    expect(mocks.invoke).toHaveBeenCalledWith('companion_voice_create',{roleId:'other',names:[],localIds:[sample.id]});
    expect(page.getByRole('status').textContent).toContain('私有声音已建立并绑定到小白');
    expect(page.getByText('recording.wav')).toBeTruthy();
  });

  it('keeps staged audio and the existing voice when upload fails', async () => {
    voiceMock({failUpload:true});
    const page = render(CompanionSettings); await openSpeech(page);
    await fireEvent.click(page.getByRole('button',{name:'导入音声到本地暂存'})); await flushAsync();
    await fireEvent.click(page.getByRole('checkbox', {name:/recording.wav/}));
    await fireEvent.click(page.getByRole('button',{name:'用所选素材建立私有声音'}));
    await fireEvent.click(page.getByRole('button',{name:'上传并创建私有声音'})); await flushAsync();
    expect(page.getByRole('status').textContent).toContain('连接失败');
    expect((page.getByLabelText('当前角色声音 ID') as HTMLInputElement).value).toBe('existing-voice');
    expect(page.getByText('recording.wav')).toBeTruthy();
    expect(mocks.invoke.mock.calls.some(c => c[0] === 'companion_voice_local_remove')).toBe(false);
  });

  it('removes staged audio locally and handles a cancelled file picker without uploading', async () => {
    const sample = voiceMock();
    const page = render(CompanionSettings); await openSpeech(page);
    await fireEvent.click(page.getByRole('button',{name:'导入音声到本地暂存'})); await flushAsync();
    await fireEvent.click(page.getByRole('button',{name:'移除本地音声 recording.wav'})); await flushAsync();
    expect(mocks.invoke).toHaveBeenCalledWith('companion_voice_local_remove',{roleId:'other',id:sample.id});
    expect(page.queryByText('recording.wav')).toBeNull();
    voiceMock({cancelled:true});
    await fireEvent.click(page.getByRole('button',{name:'导入音声到本地暂存'})); await flushAsync();
    expect(page.getByRole('status').textContent).toBe('已取消导入');
    expect(mocks.invoke.mock.calls.some(c => c[0] === 'companion_voice_create')).toBe(false);
  });

  it('previews local audio without requiring a saved speech key or uploading it', async () => {
    const sample = voiceMock();
    const play = vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue(undefined);
    const pause = vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(() => {});
    const page = render(CompanionSettings);
    try {
      await openSpeech(page);
      await fireEvent.click(page.getByRole('button',{name:'导入音声到本地暂存'})); await flushAsync();
      await fireEvent.click(page.getByRole('button',{name:'试听本地音声 recording.wav'})); await flushAsync();
      expect(mocks.invoke).toHaveBeenCalledWith('companion_voice_local_audio',{roleId:'other',id:sample.id});
      expect(play).toHaveBeenCalledOnce();
      expect(mocks.invoke.mock.calls.some(c => c[0] === 'companion_save' || c[0] === 'companion_voice_create')).toBe(false);
    } finally { page.unmount(); play.mockRestore(); pause.mockRestore(); }
  });

  it('uploads mixed preset and local selections for the current role', async () => {
    const sample = voiceMock();
    const page = render(CompanionSettings); await openSpeech(page);
    await fireEvent.click(page.getByRole('button',{name:'导入音声到本地暂存'})); await flushAsync();
    await fireEvent.click(page.getByRole('button',{name:'选择当前角色预设声音素材'})); await flushAsync();
    await fireEvent.click(page.getByRole('checkbox',{name:/recording.wav/}));
    await fireEvent.click(page.getByRole('checkbox',{name:'preset-one'}));
    await fireEvent.click(page.getByRole('button',{name:'用所选素材建立私有声音'}));
    await fireEvent.click(page.getByRole('button',{name:'上传并创建私有声音'})); await flushAsync();
    expect(mocks.invoke).toHaveBeenCalledWith('companion_voice_create',{roleId:'other',names:['preset-one'],localIds:[sample.id]});
  });

  it('requires a fresh confirmation if the speech service changes after review', async () => {
    voiceMock();
    const page = render(CompanionSettings); await openSpeech(page);
    await fireEvent.click(page.getByRole('button',{name:'导入音声到本地暂存'})); await flushAsync();
    await fireEvent.click(page.getByRole('checkbox',{name:/recording.wav/}));
    await fireEvent.click(page.getByRole('button',{name:'用所选素材建立私有声音'}));
    await fireEvent.input(page.getAllByLabelText('API 地址')[2],{target:{value:'https://speech.example'}});
    await fireEvent.click(page.getByRole('button',{name:'上传并创建私有声音'})); await flushAsync();
    expect(mocks.invoke.mock.calls.some(c => c[0] === 'companion_voice_create')).toBe(false);
    expect(page.getByRole('status').textContent).toContain('重新确认上传');
  });

});
