export interface City { id:string; name:string; region:string; country:string; timezone:string; latitude:number; longitude:number }
export interface EnvironmentSettings { network_weather_enabled:boolean; location_mode:'fixed_city'|'automatic'; fixed_city:City|null; fallback_city:City|null; device_location_enabled:boolean; ip_fallback_enabled:boolean }
export interface EnvironmentSnapshot {
  schema_version:number; revision:number; now:number; local_time:string; timezone:string; clock_revision:number; settings:EnvironmentSettings;
  location:{ mode:string; key:string|null; city:string|null; source:string|null; confirmed:boolean; age_seconds:number|null; accuracy_m:number|null; expires_at:number|null; revision:number; record_revision:number; device_access:string; state:string; reason:string|null };
  weather:{data:{condition:string;condition_code:string;temperature:number;feels_like:number|null;humidity:number|null;wind_speed:number|null}|null;validity:string;reason:string|null;observed_at:number|null;valid_at?:number|null;data_kind?:string;received_at:number|null;expires_at:number|null;provider:string;region:string|null;revision:number;refresh_state:string};
  calendar:{local_date:string;region:string;dataset_version:string;coverage:string;holidays:string[]};
}
export interface CareSettings {schema_version:number;enabled:boolean;rain:boolean;night:boolean;holiday:boolean;legacy_weather:boolean;night_start:number;night_end:number;special_dates?:boolean;capture_dates?:boolean;solar_terms?:boolean;cold_threshold?:number;hot_threshold?:number}
export interface CareStatus {settings:CareSettings;reason:string|null;candidates:number;presenting:boolean}
const reasons:Record<string,string>={weather_disabled:'网络天气已关闭',resume_requires_refresh:'恢复后等待重新核验',location_unverified:'地点尚未确认或资料区域不匹配',observation_time_unknown:'天气来源没有可核验的完整观测时间',observation_in_future:'天气观测时间异常',weather_expired:'天气资料已过期',weather_unavailable:'尚无天气资料',city_not_set:'请先选择关注城市',device_location_expired:'设备位置已过期',device_denied:'Windows 未允许定位',device_unavailable:'设备定位暂不可用',device_timeout:'设备定位超时',device_accuracy_insufficient:'设备定位精度不足',location_sources_unavailable:'没有已允许的可用位置来源',ip_city_confirmation_required:'请确认网络估算的城市',network_unavailable:'暂时无法连接资料来源',provider_error:'资料来源暂不可用',ip_unavailable:'网络粗定位暂不可用',invalid_weather:'天气资料格式无法核验'};
export const explain=(reason:string|null|undefined)=>reason?(reasons[reason]??reason):'';
export const localTime=(timestamp:number|null)=>timestamp===null?'未知':new Date(timestamp*1000).toLocaleString();

export interface SpecialDate {id:string;name:string;kind:string;calendar:'gregorian'|'lunar';year:number|null;month:number;day:number;yearly:boolean;leap_month:boolean;enabled:boolean;source:string}
export interface CalendarDataset {id:string;country:string;year:number;source:string;imported_at:string;events:Array<{date:string;name:string;kind:string}>;lunar_days:Array<{date:string;month:number;day:number;leap:boolean}>}
export interface CalendarData {schema_version:number;revision:number;datasets:CalendarDataset[];special_dates:SpecialDate[]}
