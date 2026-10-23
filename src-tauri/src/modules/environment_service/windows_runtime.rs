//! Load optional Windows Runtime activation from the system directory.
//! Unsupported systems must disable location, never fail during process loading.
use std::{ffi::c_void, sync::OnceLock};
use windows::{
    core::{imp::IGenericFactory, Interface, GUID, HRESULT, HSTRING},
    Win32::{
        Foundation::HMODULE,
        System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32},
    },
};
static COMBASE: OnceLock<Option<usize>> = OnceLock::new();
static MTA: OnceLock<Option<usize>> = OnceLock::new();
fn library() -> Result<HMODULE, String> {
    COMBASE
        .get_or_init(|| unsafe {
            LoadLibraryExW(
                windows::core::w!("combase.dll"),
                None,
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
            .ok()
            .map(|m| m.0 as usize)
        })
        .map(|p| HMODULE(p as *mut c_void))
        .ok_or("device_unavailable".into())
}
fn initialize_mta(module: HMODULE) {
    MTA.get_or_init(|| unsafe {
        let pointer = GetProcAddress(module, windows::core::s!("CoIncrementMTAUsage"))?;
        let activate: unsafe extern "system" fn(*mut *mut c_void) -> HRESULT =
            std::mem::transmute(pointer);
        let mut cookie = std::ptr::null_mut();
        activate(&mut cookie).ok().ok()?;
        Some(cookie as usize)
    });
}
fn factory<T: Interface>(class: &str) -> Result<T, String> {
    let module = library()?;
    // One process MTA lease is sufficient for background workers. The system
    // releases its cookie and this loaded system library at process shutdown.
    initialize_mta(module);
    unsafe {
        let pointer = GetProcAddress(module, windows::core::s!("RoGetActivationFactory"))
            .ok_or("device_unavailable")?;
        let activate: unsafe extern "system" fn(
            *mut c_void,
            *const GUID,
            *mut *mut c_void,
        ) -> HRESULT = std::mem::transmute(pointer);
        let name = HSTRING::from(class);
        let mut result = std::ptr::null_mut();
        activate(std::mem::transmute_copy(&name), &T::IID, &mut result)
            .ok()
            .map_err(|_| "device_unavailable")?;
        if result.is_null() {
            return Err("device_unavailable".into());
        }
        Ok(T::from_raw(result))
    }
}
pub fn geolocator() -> Result<windows::Devices::Geolocation::Geolocator, String> {
    factory::<IGenericFactory>("Windows.Devices.Geolocation.Geolocator")?
        .ActivateInstance()
        .map_err(|_| "device_unavailable".into())
}
/// Must be called only from the foreground settings UI thread.
pub fn request_location_access() -> Result<
    windows::Foundation::IAsyncOperation<windows::Devices::Geolocation::GeolocationAccessStatus>,
    String,
> {
    let statics = factory::<windows::Devices::Geolocation::IGeolocatorStatics>(
        "Windows.Devices.Geolocation.Geolocator",
    )?;
    unsafe {
        let mut result = std::ptr::null_mut();
        (Interface::vtable(&statics).RequestAccessAsync)(statics.as_raw(), &mut result)
            .ok()
            .map_err(|_| "device_unavailable")?;
        if result.is_null() {
            return Err("device_unavailable".into());
        }
        Ok(windows::Foundation::IAsyncOperation::from_raw(result))
    }
}
pub fn location_capability(
) -> Result<windows::Security::Authorization::AppCapabilityAccess::AppCapability, String> {
    use windows::Security::Authorization::AppCapabilityAccess::{
        AppCapability, IAppCapabilityStatics,
    };
    let statics = factory::<IAppCapabilityStatics>(
        "Windows.Security.Authorization.AppCapabilityAccess.AppCapability",
    )?;
    unsafe {
        let name = HSTRING::from("location");
        let mut result = std::ptr::null_mut();
        (Interface::vtable(&statics).Create)(
            statics.as_raw(),
            std::mem::transmute_copy(&name),
            &mut result,
        )
        .ok()
        .map_err(|_| "device_unavailable")?;
        if result.is_null() {
            return Err("device_unavailable".into());
        }
        Ok(AppCapability::from_raw(result))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_activation_can_fail_without_preventing_process_start() {
        assert!(factory::<IGenericFactory>("TrayBuddy.Nonexistent.Optional.Runtime").is_err());
    }
}
