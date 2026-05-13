//! COM in-process server entry points.
//!
//! Every APO `.dll` exports the four standard COM in-process server
//! functions below. The Windows audio engine resolves these by name
//! at load time (Tier 2 verification checks they are present and
//! unmangled via `dumpbin /exports`).
//!
//! Current behaviour: the entry points are wired up at the ABI
//! boundary but report no-class-available / no-op. The
//! `IClassFactory` machinery, registry writes, and ref-count
//! bookkeeping land in later commits.

use core::ffi::c_void;

use windows_core::{GUID, HRESULT};

/// `S_OK = 0x00000000`.
const S_OK: HRESULT = HRESULT(0);

/// `S_FALSE = 0x00000001`. Returned by `DllCanUnloadNow` while the
/// DLL still has live objects.
const S_FALSE: HRESULT = HRESULT(1);

/// `CLASS_E_CLASSNOTAVAILABLE = 0x80040111`. Returned by
/// `DllGetClassObject` when the requested CLSID is unknown.
const CLASS_E_CLASSNOTAVAILABLE: HRESULT = HRESULT(0x80040111_u32 as i32);

/// COM class object factory entry point.
///
/// # Safety
///
/// Called by COM. `rclsid` and `riid` must point to valid `GUID`s
/// and `ppv` must point to a writable pointer slot. The function
/// follows the standard COM contract: on `CLASS_E_CLASSNOTAVAILABLE`
/// the slot is zeroed before returning.
#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(
    _rclsid: *const GUID,
    _riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if !ppv.is_null() {
        unsafe {
            *ppv = core::ptr::null_mut();
        }
    }
    CLASS_E_CLASSNOTAVAILABLE
}

/// Returns `S_OK` if the DLL has no outstanding object references
/// and may be unloaded, otherwise `S_FALSE`.
///
/// # Safety
///
/// Called by COM. The placeholder implementation never reports the
/// DLL as unloadable so the host keeps it loaded; once the
/// reference counter is wired in, this will consult it.
#[no_mangle]
pub unsafe extern "system" fn DllCanUnloadNow() -> HRESULT {
    // Stub: with no live objects yet there is nothing to count, but
    // returning S_FALSE is the safer placeholder while
    // `DllGetClassObject` cannot actually hand out objects.
    S_FALSE
}

/// Self-registration entry point invoked by `regsvr32.exe`.
///
/// # Safety
///
/// Called by `regsvr32`. The current implementation does not touch
/// the registry; once `registration` lands it will write the CLSID
/// keys under `HKCU\Software\Classes\CLSID\{...}` (per-user) or
/// `HKLM\...` (machine, with admin).
#[no_mangle]
pub unsafe extern "system" fn DllRegisterServer() -> HRESULT {
    S_OK
}

/// Inverse of `DllRegisterServer`.
///
/// # Safety
///
/// Called by `regsvr32 /u`. Mirrors `DllRegisterServer`'s stub
/// behaviour for now.
#[no_mangle]
pub unsafe extern "system" fn DllUnregisterServer() -> HRESULT {
    S_OK
}
