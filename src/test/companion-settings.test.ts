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
    await fireEvent.click(page.getByRole('button', {name: '保存艾玛 AI 设置'}));
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
    await fireEvent.click(page.getByRole('button', { name: '保存艾玛 AI 设置' }));
    await flushAsync();
    expect(mocks.invoke.mock.calls.find(call => call[0] === 'companion_save')![1].settings.reply_max_chars).toBe(limit);
  });
  it('rejects a negative reply limit before saving', async () => {
    const page = render(CompanionSettings);
    await flushAsync();
    await fireEvent.input(page.getByLabelText('回复字数上限'), { target: { value: '-1' } });
    await fireEvent.click(page.getByRole('button', { name: '保存艾玛 AI 设置' }));
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
    await fireEvent.click(page.getByRole('button',{name:'保存艾玛 AI 设置'}));await flushAsync();
    const settings=mocks.invoke.mock.calls.find(c=>c[0]==='companion_save')![1].settings;
    expect(settings.chat.thinking).toBe('high');expect(settings.vision.thinking).toBe('low');expect(settings.web_search.enabled).toBe(true);
  });
  it('persists an explicit choice to disable search',async()=>{
    const page=render(CompanionSettings);await flushAsync();await fireEvent.click(page.getByLabelText('允许按需网页搜索'));await fireEvent.click(page.getByRole('button',{name:'保存艾玛 AI 设置'}));await flushAsync();expect(mocks.invoke.mock.calls.find(c=>c[0]==='companion_save')![1].settings.web_search.enabled).toBe(false);
  });
  it('saves a different role persona and voice without overwriting the legacy voice configuration',async()=>{
    mocks.invoke.mockImplementation(async(command:string,args?:any)=>command==='companion_get'?fixture():command==='companion_character_get'?{id:'other',display_name:'小白',persona:'活泼的桌面伙伴',voice_locale:'zh',voice_id:'voice-other'}:command==='companion_save'?{...fixture(),settings:args.settings}:[]);
    const page=render(CompanionSettings);await flushAsync();expect(page.getByRole('button',{name:'保存小白 AI 设置'})).toBeTruthy();
    await fireEvent.input(page.getByLabelText('当前角色声音 ID'),{target:{value:'new-voice'}});await fireEvent.click(page.getByRole('button',{name:'保存小白 AI 设置'}));await flushAsync();
    const settings=mocks.invoke.mock.calls.find(c=>c[0]==='companion_save')![1].settings;expect(settings.characters.other).toMatchObject({persona:'活泼的桌面伙伴',voice_id:'new-voice',voice_locale:'zh'});expect(settings.voice_id).toBe('');
  });
  it('persists Japanese reading independently of a persona that requests Chinese replies',async()=>{
    mocks.invoke.mockImplementation(async(command:string,args?:any)=>command==='companion_get'?fixture():command==='companion_character_get'?{id:'ema',display_name:'艾玛',persona:'自然地用简体中文回复。',voice_locale:'zh',voice_id:'voice-ema'}:command==='companion_save'?{...fixture(),settings:args.settings}:[]);
    const page=render(CompanionSettings);await flushAsync();
    await fireEvent.change(page.getByLabelText('当前角色朗读语言'),{target:{value:'ja'}});
    await fireEvent.click(page.getByRole('button',{name:'保存艾玛 AI 设置'}));await flushAsync();
    const settings=mocks.invoke.mock.calls.find(c=>c[0]==='companion_save')![1].settings;
    expect(settings.characters.ema).toMatchObject({persona:'自然地用简体中文回复。',voice_locale:'ja'});
    expect((page.getByLabelText('当前角色朗读语言') as HTMLSelectElement).value).toBe('ja');
  });

});
