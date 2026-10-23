import type { EnvironmentSnapshot } from '$lib/environment/types';
export function environmentFixture(): EnvironmentSnapshot {
  return {
    schema_version:1,revision:1,now:1792670000,local_time:'2026-10-22T20:00:00+08:00',timezone:'+08:00',clock_revision:1,
    settings:{network_weather_enabled:true,location_mode:'fixed_city',fixed_city:null,fallback_city:null,device_location_enabled:false,ip_fallback_enabled:false},
    location:{mode:'fixed_city',key:null,city:null,source:null,confirmed:false,age_seconds:null,accuracy_m:null,expires_at:null,revision:1,record_revision:1,device_access:'not_requested',state:'idle',reason:'city_not_set'},
    weather:{data:null,validity:'unavailable',reason:'weather_unavailable',observed_at:null,received_at:null,expires_at:null,provider:'wttr.in',region:null,revision:1,refresh_state:'idle'},
    calendar:{local_date:'2026-10-22',region:'CN',dataset_version:'HKO-CN-2026-2030-v1',coverage:'covered',holidays:[]},
  };
}
