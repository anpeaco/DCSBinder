//! Enumerate attached game controllers via Windows `DirectInput 8`.
//!
//! See `ADR-004`. This module is `#[cfg(windows)]`-gated by its parent.
//!
//! Note: this is the only place in the project that uses `unsafe`. Every other
//! crate runs under `#![deny(unsafe_code)]`. The FFI calls below are scoped to
//! enumeration and have no long-lived raw pointers.

#![allow(unsafe_code)]

use std::ffi::c_void;

use windows::core::{Interface, GUID};
use windows::Win32::Devices::HumanInterfaceDevice::{
    DirectInput8Create, IDirectInput8W, DI8DEVCLASS_GAMECTRL, DIDEVICEINSTANCEW,
    DIEDFL_ATTACHEDONLY, DIRECTINPUT_VERSION,
};
use windows::Win32::Foundation::{BOOL, HMODULE};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;

use super::guid::Guid;

/// `DIENUM_CONTINUE` (1) typed as `BOOL` so the callback's return type matches.
const CONTINUE: BOOL = BOOL(1);

/// One currently-attached game controller as `DirectInput` sees it.
///
/// `instance_guid` is the GUID that appears in DCS filenames (e.g.
/// `MFDLeft {4E50F3B0-2309-11ee-8015-444553540000}.diff.lua`). `product_guid`
/// is shared across multiple physical copies of the same controller model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveDevice {
    pub instance_guid: Guid,
    pub product_guid: Guid,
    pub product_name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum EnumError {
    #[error("CoInitializeEx failed: {0}")]
    CoInit(String),
    #[error("DirectInput8Create failed: {0}")]
    Create(String),
    #[error("IDirectInput8::EnumDevices failed: {0}")]
    Enum(String),
    #[error("GetModuleHandle(NULL) failed: {0}")]
    ModuleHandle(String),
}

/// Enumerate all currently-attached game controllers via `DirectInput`.
///
/// Returns an empty `Vec` if no controllers are attached; returns an `Err` only
/// if `DirectInput` itself fails to initialize.
pub fn enumerate() -> Result<Vec<LiveDevice>, EnumError> {
    // SAFETY: We call into COM / DirectInput. CoInitializeEx is matched with
    // CoUninitialize. DirectInput8Create's out-parameter is wrapped in an
    // `IDirectInput8W` smart pointer that drops correctly. The enumeration
    // callback only reads a `*const` provided by DirectInput for the duration
    // of one call and writes through a `*mut Vec<LiveDevice>` whose Vec lives
    // on this thread's stack for the entire EnumDevices call.
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_err() {
            return Err(EnumError::CoInit(format!("{hr:?}")));
        }

        let result = enumerate_inner();
        CoUninitialize();
        result
    }
}

unsafe fn enumerate_inner() -> Result<Vec<LiveDevice>, EnumError> {
    let hinst: HMODULE =
        GetModuleHandleW(None).map_err(|e| EnumError::ModuleHandle(e.message()))?;

    let mut di_ptr: *mut c_void = std::ptr::null_mut();
    let iid: GUID = IDirectInput8W::IID;
    DirectInput8Create(
        hinst.into(),
        DIRECTINPUT_VERSION,
        std::ptr::from_ref(&iid),
        std::ptr::addr_of_mut!(di_ptr).cast(),
        None,
    )
    .map_err(|e| EnumError::Create(e.message()))?;

    if di_ptr.is_null() {
        return Err(EnumError::Create("DirectInput8Create returned null".into()));
    }
    let di: IDirectInput8W = IDirectInput8W::from_raw(di_ptr);

    let mut devices: Vec<LiveDevice> = Vec::new();
    let ctx = std::ptr::addr_of_mut!(devices).cast::<c_void>();

    di.EnumDevices(
        DI8DEVCLASS_GAMECTRL,
        Some(enum_callback),
        ctx,
        DIEDFL_ATTACHEDONLY,
    )
    .map_err(|e| EnumError::Enum(e.message()))?;

    Ok(devices)
}

unsafe extern "system" fn enum_callback(lpddi: *mut DIDEVICEINSTANCEW, pvref: *mut c_void) -> BOOL {
    if lpddi.is_null() || pvref.is_null() {
        return CONTINUE;
    }
    let inst = &*lpddi;
    let devices = &mut *(pvref.cast::<Vec<LiveDevice>>());

    let product_name = wide_str_to_string(&inst.tszProductName);
    devices.push(LiveDevice {
        instance_guid: guid_from_windows(inst.guidInstance),
        product_guid: guid_from_windows(inst.guidProduct),
        product_name,
    });

    CONTINUE
}

fn guid_from_windows(g: GUID) -> Guid {
    let mut bytes = [0u8; 16];
    bytes[0..4].copy_from_slice(&g.data1.to_be_bytes());
    bytes[4..6].copy_from_slice(&g.data2.to_be_bytes());
    bytes[6..8].copy_from_slice(&g.data3.to_be_bytes());
    bytes[8..16].copy_from_slice(&g.data4);
    Guid::from_bytes(bytes)
}

fn wide_str_to_string(wide: &[u16]) -> String {
    let len = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..len])
}
