import { beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { flushAsync } from './test-utils';
import type { CompanionView } from '$lib/companion/types';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  handlers: new Map<string, (event: { payload: any }) => unknown>(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async (name: string, handler: (event: { payload: any }) => unknown) => {
  mocks.handlers.set(name, handler);
  return () => mocks.handlers.delete(name);
}) }));
import CompanionInput from '../routes/companion_input/+page.svelte';

let view: CompanionView;
beforeEach(() => {
  cleanup(); localStorage.clear(); mocks.handlers.clear(); mocks.invoke.mockReset();
  const endpoint = { base_url: '', model: '', has_key: false };
  view = { bank_id: 'ema-companion-test', status: '就绪', pending: 0, busy: false, summary: '', settings: {
    schema_version: 1, input_enabled: true, input_collapsed: false, speech_enabled: true, observation_enabled: false, memory_enabled: true,
    chat: { ...endpoint, has_key: true }, vision: { ...endpoint }, speech: { ...endpoint }, memory: { ...endpoint },
    voice_id: '', speech_speed: 1, observation_interval_secs: 300, capture_scope: 'foreground', capture_region: [0, 0, 800, 600],
    excluded_windows: [], reply_max_chars: 120, nickname: '',
  } };
  mocks.invoke.mockImplementation(async (command: string, args?: { collapsed: boolean }) => {
    if (command === 'companion_get') return structuredClone(view);
    if (command === 'companion_set_input_collapsed') {
      view.settings.input_collapsed = args!.collapsed;
      return args!.collapsed;
    }
  });
});

describe('manual companion input collapse', () => {
  it('keeps the draft and focuses the input when explicitly expanded again', async () => {
    const page = render(CompanionInput);
    await flushAsync();
    await fireEvent.input(page.getByLabelText('给艾玛的消息'), { target: { value: '还没说完的话' } });
    await fireEvent.click(page.getByRole('button', { name: '收起聊天输入框' }));
    await flushAsync();
    expect(page.queryByLabelText('给艾玛的消息')).toBeNull();
    expect(localStorage.getItem('ema-companion-draft')).toBe('还没说完的话');
    expect(mocks.invoke).toHaveBeenCalledWith('companion_set_input_collapsed', { collapsed: true });
    await fireEvent.click(page.getByRole('button', { name: '展开聊天输入框' }));
    await flushAsync();
    const input = page.getByLabelText('给艾玛的消息') as HTMLTextAreaElement;
    expect(input.value).toBe('还没说完的话');
    expect(document.activeElement).toBe(input);
    expect(mocks.invoke).toHaveBeenCalledWith('companion_set_input_collapsed', { collapsed: false });
    expect(mocks.invoke.mock.calls.some((call: unknown[]) => call[0] === 'companion_stop' || call[0] === 'companion_send')).toBe(false);
  });

  it('restores the saved collapsed choice when the window is created again', async () => {
    const first = render(CompanionInput);
    await flushAsync();
    await fireEvent.click(first.getByRole('button', { name: '收起聊天输入框' }));
    await flushAsync();
    first.unmount();
    const reopened = render(CompanionInput);
    await flushAsync();
    expect(reopened.getByRole('button', { name: '展开聊天输入框' }).getAttribute('aria-expanded')).toBe('false');
    expect(reopened.queryByLabelText('给艾玛的消息')).toBeNull();
  });

  it('keeps a manual choice when an older settings request finishes late', async () => {
    const page = render(CompanionInput);
    await flushAsync();
    let finishLoad!: (value: CompanionView) => void;
    mocks.invoke.mockImplementationOnce(() => new Promise<CompanionView>(resolve => finishLoad = resolve));
    mocks.handlers.get('companion-settings-changed')?.({ payload: {} });
    const oldView = structuredClone(view);
    await fireEvent.click(page.getByRole('button', { name: '收起聊天输入框' }));
    await flushAsync();
    finishLoad(oldView);
    await flushAsync();
    expect(page.getByRole('button', { name: '展开聊天输入框' })).toBeTruthy();
    expect(page.queryByLabelText('给艾玛的消息')).toBeNull();
  });

  it('does not expand or cancel an ongoing reply on background status changes', async () => {
    view.settings.input_collapsed = true; view.busy = true;
    const page = render(CompanionInput);
    await flushAsync();
    mocks.handlers.get('companion-status')?.({ payload: { message: '正在生成回复', busy: true } });
    await flushAsync();
    expect(page.queryByLabelText('给艾玛的消息')).toBeNull();
    await fireEvent.click(page.getByRole('button', { name: '展开聊天输入框' }));
    await flushAsync();
    expect(page.getByRole('button', { name: '停止' })).toBeTruthy();
    expect(mocks.invoke.mock.calls.some((call: unknown[]) => call[0] === 'companion_stop')).toBe(false);
  });

  it('keeps the input and draft available if saving the collapsed choice fails', async () => {
    const page = render(CompanionInput);
    await flushAsync();
    await fireEvent.input(page.getByLabelText('给艾玛的消息'), { target: { value: '保留草稿' } });
    mocks.invoke.mockRejectedValueOnce('保存失败');
    await fireEvent.click(page.getByRole('button', { name: '收起聊天输入框' }));
    await flushAsync();
    expect((page.getByLabelText('给艾玛的消息') as HTMLTextAreaElement).value).toBe('保留草稿');
    expect(page.getByRole('status').textContent).toBe('保存失败');
    expect(page.queryByRole('button', { name: '展开聊天输入框' })).toBeNull();
  });

  it('waits for the saved layout before rendering into a possibly collapsed native window', async () => {
    let finishLoad!: (value: CompanionView) => void;
    mocks.invoke.mockImplementationOnce(() => new Promise<CompanionView>(resolve => finishLoad = resolve));
    const page = render(CompanionInput);
    expect(page.queryByLabelText('给艾玛的消息')).toBeNull();
    view.settings.input_collapsed = true;
    finishLoad(structuredClone(view));
    await flushAsync();
    expect(page.getByRole('button', { name: '展开聊天输入框' })).toBeTruthy();
    expect(page.queryByLabelText('给艾玛的消息')).toBeNull();
  });

  it('removes all input content when disabled and restores its draft when enabled', async () => {
    const page = render(CompanionInput);
    await flushAsync();
    await fireEvent.input(page.getByLabelText('给艾玛的消息'), { target: { value: '隐藏时保留草稿' } });
    view.settings.input_enabled = false;
    mocks.handlers.get('companion-settings-changed')?.({ payload: {} });
    await flushAsync();
    expect(page.queryByLabelText('给艾玛的消息')).toBeNull();
    expect(page.queryByRole('button')).toBeNull();
    view.settings.input_enabled = true;
    mocks.handlers.get('companion-settings-changed')?.({ payload: {} });
    await flushAsync();
    expect((page.getByLabelText('给艾玛的消息') as HTMLTextAreaElement).value).toBe('隐藏时保留草稿');
  });

  it('does not let an older refresh restore content after disabling the input', async () => {
    const page = render(CompanionInput);
    await flushAsync();
    const oldView = structuredClone(view);
    let finishLoad!: (value: CompanionView) => void;
    mocks.invoke.mockImplementationOnce(() => new Promise<CompanionView>(resolve => finishLoad = resolve));
    mocks.handlers.get('companion-settings-changed')?.({ payload: {} });
    view.settings.input_enabled = false;
    mocks.handlers.get('companion-settings-changed')?.({ payload: {} });
    await flushAsync();
    finishLoad(oldView);
    await flushAsync();
    expect(page.queryByLabelText('给艾玛的消息')).toBeNull();
    expect(page.queryByRole('button')).toBeNull();
  });

  it('renders the compact control before the collapse command finishes resizing the native window', async () => {
    const page = render(CompanionInput);
    await flushAsync();
    let finishToggle!: (value: boolean) => void;
    mocks.invoke.mockImplementationOnce(() => new Promise<boolean>(resolve => finishToggle = resolve));
    await fireEvent.click(page.getByRole('button', { name: '收起聊天输入框' }));
    await flushAsync();
    expect(page.queryByLabelText('给艾玛的消息')).toBeNull();
    expect((page.getByRole('button', { name: '展开聊天输入框' }) as HTMLButtonElement).disabled).toBe(true);
    finishToggle(true);
    await flushAsync();
    expect((page.getByRole('button', { name: '展开聊天输入框' }) as HTMLButtonElement).disabled).toBe(false);
  });
});
