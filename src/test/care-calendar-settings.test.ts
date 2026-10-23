import {beforeEach,describe,it,expect,vi} from 'vitest';
import {cleanup,fireEvent,render} from '@testing-library/svelte';
import {flushAsync} from './test-utils';
const mocks=vi.hoisted(()=>({invoke:vi.fn()}));
vi.mock('@tauri-apps/api/core',()=>({invoke:mocks.invoke}));
vi.mock('@tauri-apps/api/event',()=>({listen:vi.fn(async()=>()=>{})}));
import CalendarSettings from '$lib/components/CareCalendarSettings.svelte';
beforeEach(()=>{cleanup();mocks.invoke.mockReset();mocks.invoke.mockImplementation(async(command:string)=>command==='care_calendar_get'?{schema_version:1,revision:1,datasets:[],special_dates:[]}:null);});
describe('care calendar settings',()=>{
  it('uses no network merely by opening calendar settings',async()=>{
    const page=render(CalendarSettings);await flushAsync();expect(page.getByText('尚未导入日历，节日关怀暂不启动。')).toBeTruthy();expect(mocks.invoke.mock.calls.map(c=>c[0])).toEqual(['care_calendar_get']);
  });
  it('imports only the country and years explicitly selected',async()=>{
    const page=render(CalendarSettings);await flushAsync();await fireEvent.input(page.getByLabelText('日历国家代码'),{target:{value:'JP'}});await fireEvent.input(page.getByLabelText('日历起始年份'),{target:{value:'2026'}});await fireEvent.input(page.getByLabelText('日历结束年份'),{target:{value:'2027'}});await fireEvent.click(page.getByRole('button',{name:'联网导入日历'}));await flushAsync();expect(mocks.invoke).toHaveBeenCalledWith('care_calendar_import',{country:'JP',firstYear:2026,lastYear:2027});
  });
  it('stores a manual birthday as an annual Gregorian date without inventing a birth year',async()=>{
    const page=render(CalendarSettings);await flushAsync();await fireEvent.input(page.getByLabelText('特殊日期名称'),{target:{value:'我的生日'}});await fireEvent.input(page.getByLabelText('特殊日期月份'),{target:{value:'10'}});await fireEvent.input(page.getByLabelText('特殊日期日'),{target:{value:'25'}});await fireEvent.click(page.getByRole('button',{name:'添加特殊日期'}));await flushAsync();expect(mocks.invoke).toHaveBeenCalledWith('care_special_save',{date:expect.objectContaining({name:'我的生日',month:10,day:25,calendar:'gregorian',yearly:true,year:null,source:'manual'})});
  });
  it('defaults a vacation to one occurrence',async()=>{
    const page=render(CalendarSettings);await flushAsync();await fireEvent.change(page.getByLabelText('类别'),{target:{value:'vacation'}});await flushAsync();expect((page.getByLabelText('每年重复') as HTMLInputElement).checked).toBe(false);expect(page.getByLabelText('特殊日期年份')).toBeTruthy();
  });
});
