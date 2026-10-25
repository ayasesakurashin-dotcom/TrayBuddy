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
  view = { character_name: '艾玛', bank_id: 'ema-companion-test', status: '就绪', pending: 0, busy: false, summary: '', settings: {
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

describe('screen questions and request receipts', () => {
  it('uses a generic fallback and refreshes the active role without losing the shared draft', async () => {
    view.character_name = undefined;
    localStorage.setItem('ema-companion-draft', '继续上次的话题');
    const page = render(CompanionInput); await flushAsync();
    expect(page.getByLabelText('给桌面伙伴的消息')).toBeTruthy();
    view.character_name = '小白';
    mocks.handlers.get('companion-settings-changed')?.({payload:{}}); await flushAsync();
    expect((page.getByLabelText('给小白的消息') as HTMLTextAreaElement).value).toBe('继续上次的话题');
  });
  const completed = {request:8,turn_id:'turn-8',result:'completed',text_committed:true,observation_ids:['observation-8'],history_epoch:0,replayed:false};
  it('keeps date receipts in the bottom-left status through reply and voice stages, then clears them on a new send', async () => {
    const page = render(CompanionInput); await flushAsync();
    const notice = '已保存特殊日期：每年公历10月25日生日。';
    const sendStatus = async (event_seq:number, message:string, extra:Record<string,unknown>={}) => {
      mocks.handlers.get('companion-status')?.({payload:{request:8,event_seq,message,busy:true,notice,...extra}});
      await flushAsync();
    };
    await sendStatus(1,'正在生成回复…');
    expect(page.getByRole('status').textContent).toBe(notice);
    expect(page.getByRole('status').getAttribute('title')).toBe(notice);
    await sendStatus(2,'正在准备回复语音…');
    await sendStatus(3,'就绪',{busy:false});
    expect(page.getByRole('status').textContent).toBe(notice);
    await fireEvent.input(page.getByLabelText('给艾玛的消息'),{target:{value:'接下来聊别的'}});
    await fireEvent.click(page.getByRole('button',{name:'发送'})); await flushAsync();
    expect(page.queryByText(notice)).toBeNull();
  });
  it('preserves higher priority failure warnings and rejects stale date notices', async () => {
    const page = render(CompanionInput); await flushAsync();
    mocks.handlers.get('companion-status')?.({payload:{request:9,event_seq:1,message:'就绪',busy:false,warning:'本次调用搜索工具失败',notice:'已保存特殊日期'}});
    await flushAsync();
    expect(page.getByRole('status').textContent).toBe('本次调用搜索工具失败');
    mocks.handlers.get('companion-status')?.({payload:{request:8,event_seq:99,message:'就绪',busy:false,notice:'迟到的保存提示'}});
    await flushAsync();
    expect(page.queryByText('迟到的保存提示')).toBeNull();
  });
  it('restores a date receipt when input opens collapsed', async () => {
    view.notice='已保存特殊日期：生日';view.settings.input_collapsed=true;
    const page=render(CompanionInput);await flushAsync();
    const entry=page.getByRole('button',{name:'展开聊天输入框'});
    expect(entry.getAttribute('title')).toBe(view.notice);
    expect(entry.textContent).toContain('查看本次提示');
  });
  it('prepares a target without sending, then attaches it to the original question exactly once', async () => {
    view.settings.vision = {base_url:'https://vision.example',model:'vision',has_key:true};
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === 'companion_get') return structuredClone(view);
      if (command === 'companion_prepare_screen') return {target_token:'ticket',label:'编辑器窗口',scope:'foreground',expires_at:'2099-01-01'};
      if (command === 'companion_send') return completed;
    });
    const page=render(CompanionInput); await flushAsync();
    await fireEvent.input(page.getByLabelText('给艾玛的消息'),{target:{value:'这个报错怎么解决？'}});
    await fireEvent.click(page.getByRole('button',{name:'带上当前屏幕'})); await flushAsync();
    expect(mocks.invoke.mock.calls.filter(call=>call[0]==='companion_send')).toHaveLength(0);
    expect(page.getByText('发送时观察：编辑器窗口')).toBeTruthy();
    await fireEvent.click(page.getByRole('button',{name:'发送'})); await flushAsync();
    const sends=mocks.invoke.mock.calls.filter(call=>call[0]==='companion_send');
    expect(sends).toHaveLength(1);
    expect(sends[0][1].payload).toMatchObject({text:'这个报错怎么解决？',expected_history_epoch:0,screen:{mode:'fresh',target_token:'ticket'}});
    expect((page.getByLabelText('给艾玛的消息') as HTMLTextAreaElement).value).toBe('');
    expect(page.queryByText('发送时观察：编辑器窗口')).toBeNull();
  });

  it('keeps a newly edited draft when the previous send finishes', async () => {
    let finish!: (value: unknown)=>void;
    mocks.invoke.mockImplementation(async (command:string)=>{
      if(command==='companion_get')return structuredClone(view);
      if(command==='companion_send')return new Promise(resolve=>finish=resolve);
    });
    const page=render(CompanionInput);await flushAsync();
    const input=page.getByLabelText('给艾玛的消息');
    await fireEvent.input(input,{target:{value:'第一条问题'}});
    await fireEvent.click(page.getByRole('button',{name:'发送'}));await flushAsync();
    await fireEvent.input(input,{target:{value:'还没发送的第二条'}});
    finish(completed);await flushAsync();
    expect((input as HTMLTextAreaElement).value).toBe('还没发送的第二条');
    expect(localStorage.getItem('ema-companion-draft')).toBe('还没发送的第二条');
  });

  it('shows search failure in the status area and preserves the question for an explicit retry', async () => {
    mocks.invoke.mockImplementation(async(command:string)=>{
      if(command==='companion_get')return structuredClone(view);
      if(command==='companion_send')return {...completed,result:'failed',text_committed:false,turn_id:null,error_code:'search_timeout',warning_code:'search_timeout'};
    });
    const page=render(CompanionInput);await flushAsync();
    await fireEvent.input(page.getByLabelText('给艾玛的消息'),{target:{value:'请查证这个版本的发布日期'}});
    await fireEvent.click(page.getByRole('button',{name:'发送'}));await flushAsync();
    expect(page.getByRole('status').textContent).toContain('本次调用搜索工具失败');
    expect((page.getByLabelText('给艾玛的消息') as HTMLTextAreaElement).value).toBe('请查证这个版本的发布日期');
    expect(mocks.invoke.mock.calls.filter(call=>call[0]==='companion_send')).toHaveLength(1);
    expect(mocks.invoke.mock.calls.some(call=>call[0]==='companion_speech_preview')).toBe(false);
  });

  it('uses a committed receipt after transport failure without resending or recapturing', async () => {
    mocks.invoke.mockImplementation(async(command:string)=>{
      if(command==='companion_get')return structuredClone(view);
      if(command==='companion_send')throw new Error('transport lost');
      if(command==='companion_request_receipt')return {client_message_id:'saved',history_epoch:0,state:'finished',request:8,turn_id:'turn-8',text_committed:true,outcome:completed};
    });
    const page=render(CompanionInput);await flushAsync();
    await fireEvent.input(page.getByLabelText('给艾玛的消息'),{target:{value:'已经保存的问题'}});
    await fireEvent.click(page.getByRole('button',{name:'发送'}));await flushAsync();
    expect((page.getByLabelText('给艾玛的消息') as HTMLTextAreaElement).value).toBe('');
    expect(mocks.invoke.mock.calls.filter(call=>call[0]==='companion_send')).toHaveLength(1);
    expect(mocks.invoke.mock.calls.some(call=>call[0]==='companion_prepare_screen')).toBe(false);
  });

  it('rejects stale request status after a newer request became active', async () => {
    const page=render(CompanionInput);await flushAsync();
    mocks.handlers.get('companion-status')?.({payload:{request:9,event_seq:3,message:'正在查找资料…',busy:true}});
    mocks.handlers.get('companion-status')?.({payload:{request:8,event_seq:999,message:'旧请求就绪',busy:false}});
    await flushAsync();
    expect(page.getByRole('status').textContent).toBe('正在查找资料…');
    expect(page.getByRole('button',{name:'停止'})).toBeTruthy();
  });
  it('recovers a committed send after window recreation without restoring a screenshot action',async()=>{
    localStorage.setItem('ema-companion-draft','已提交的问题');localStorage.setItem('companion-pending-input','old-receipt');localStorage.setItem('companion-pending-text','已提交的问题');
    mocks.invoke.mockImplementation(async(command:string)=>{if(command==='companion_get')return structuredClone(view);if(command==='companion_request_receipt')return{client_message_id:'old-receipt',history_epoch:0,state:'finished',request:null,turn_id:'turn-8',text_committed:true,outcome:completed};});
    const page=render(CompanionInput);await flushAsync();expect((page.getByLabelText('给艾玛的消息') as HTMLTextAreaElement).value).toBe('');expect(localStorage.getItem('companion-pending-input')).toBeNull();expect(mocks.invoke.mock.calls.some(c=>c[0]==='companion_send'||c[0]==='companion_prepare_screen')).toBe(false);
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
