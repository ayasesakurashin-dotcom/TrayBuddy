import {beforeEach,describe,it,expect,vi} from 'vitest';
import {cleanup,fireEvent,render} from '@testing-library/svelte';
import {flushAsync} from './test-utils';
import {environmentFixture} from './environment-fixture';
const mocks=vi.hoisted(()=>({invoke:vi.fn(),handlers:new Map<string,(e:{payload:any})=>unknown>()}));
vi.mock('@tauri-apps/api/core',()=>({invoke:mocks.invoke}));
vi.mock('@tauri-apps/api/event',()=>({listen:vi.fn(async(name:string,handler:(e:{payload:any})=>unknown)=>{mocks.handlers.set(name,handler);return()=>mocks.handlers.delete(name);})}));
import EnvironmentSettings from '$lib/components/EnvironmentSettings.svelte';
let snapshot=environmentFixture();
const care={settings:{schema_version:1,enabled:false,rain:true,night:true,holiday:true,legacy_weather:true,night_start:1380,night_end:120},reason:'自动关怀已关闭',candidates:0,presenting:false};
beforeEach(()=>{cleanup();snapshot=environmentFixture();mocks.invoke.mockReset();mocks.handlers.clear();mocks.invoke.mockImplementation(async(command:string,args?:any)=>{
  if(command==='environment_get_snapshot')return structuredClone(snapshot);
  if(command==='care_get_status')return structuredClone(care);
  if(command==='environment_save_settings'){snapshot.settings=args.settings;snapshot.revision++;return structuredClone(snapshot);}
  if(command==='environment_refresh_weather')return structuredClone(snapshot);
  return null;
});});
describe('independent environment settings',()=>{
  it('loads without requesting location, weather or a model',async()=>{const page=render(EnvironmentSettings);await flushAsync();expect(page.getByText('地点待设置')).toBeTruthy();expect(mocks.invoke.mock.calls.map(c=>c[0]).sort()).toEqual(['care_get_status','environment_get_snapshot']);expect((page.getByLabelText('开启自动关怀') as HTMLInputElement).checked).toBe(false);});
  it('requests Windows permission only after the user explicitly enables device location',async()=>{snapshot.settings.location_mode='automatic';const page=render(EnvironmentSettings);await flushAsync();await fireEvent.click(page.getByLabelText('允许使用 Windows 设备定位'));await flushAsync();const commands=mocks.invoke.mock.calls.map(c=>c[0]);expect(commands.indexOf('environment_request_location_access')).toBeGreaterThan(commands.indexOf('environment_save_settings'));expect(mocks.invoke.mock.calls.find(c=>c[0]==='environment_save_settings')![1].settings).toMatchObject({device_location_enabled:true,ip_fallback_enabled:false});});
  it('keeps IP fallback independent when device location is disabled',async()=>{snapshot.settings.location_mode='automatic';snapshot.settings.ip_fallback_enabled=true;snapshot.settings.device_location_enabled=true;snapshot.location.device_access='allowed';const page=render(EnvironmentSettings);await flushAsync();await fireEvent.click(page.getByLabelText('允许使用 Windows 设备定位'));await flushAsync();expect(mocks.invoke.mock.calls.find(c=>c[0]==='environment_save_settings')![1].settings).toMatchObject({device_location_enabled:false,ip_fallback_enabled:true});expect(mocks.invoke.mock.calls.some(c=>c[0]==='environment_request_location_access')).toBe(false);});
  it('marks old weather and uses the shared refresh command',async()=>{snapshot.weather.validity='stale';snapshot.weather.reason='weather_expired';const page=render(EnvironmentSettings);await flushAsync();expect(page.getByText('天气资料 · 已过期')).toBeTruthy();await fireEvent.click(page.getByRole('button',{name:'刷新天气'}));await flushAsync();expect(mocks.invoke).toHaveBeenCalledWith('environment_refresh_weather');expect(mocks.invoke.mock.calls.some(c=>c[0]==='get_weather_info')).toBe(false);});
  it('shows a model valid time and source without claiming a measured observation',async()=>{snapshot.weather={...snapshot.weather,validity:'fresh',reason:null,provider:'open-meteo.com',data_kind:'model_current',valid_at:1792736100,observed_at:null,data:{condition:'晴',condition_code:'113',temperature:15.4,feels_like:13.3,humidity:55,wind_speed:7.6},region:'北京'};const page=render(EnvironmentSettings);await flushAsync();expect(page.getByText('天气资料 · 当前可用')).toBeTruthy();expect(page.getByText(/当前天气估算（非实况观测）/)).toBeTruthy();expect(page.getByText(/估算对应时间/)).toBeTruthy();expect(page.queryByText(/观测时间：未知/)).toBeNull();expect(page.queryByText('天气来源没有可核验的完整观测时间')).toBeNull();});
});
