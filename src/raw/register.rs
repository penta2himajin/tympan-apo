//! Registry helpers for COM in-process server self-registration.
//!
//! Writes the `HKCU\Software\Classes\CLSID\{<clsid>}` subtree that
//! the Windows COM activator consults when resolving
//! `CoCreateInstance(<clsid>, ...)`:
//!
//! - `<clsid-key>\(default)` — friendly name
//! - `<clsid-key>\InprocServer32\(default)` — absolute DLL path
//! - `<clsid-key>\InprocServer32\ThreadingModel` — `"Both"`
//!   (the framework's `ApoClassFactory` is reentrant and the
//!   activator may legitimately call into it from either an STA
//!   or MTA apartment)
//!
//! [`write_registry`] populates the subtree; [`clear_registry`]
//! deletes it. Both helpers operate exclusively under
//! `HKEY_CURRENT_USER` so `regsvr32 /n /i:user` can drive them
//! without administrative privilege.
//!
//! The framework intentionally does not touch the audio-engine
//! `FxProperties` registry binding here — `FxProperties` lives
//! under per-endpoint keys, requires admin, and is not what
//! `DllRegisterServer` is responsible for.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows_core::PCWSTR;

use crate::clsid::Clsid;
use crate::raw::class_factory::ApoVTable;

/// Write the CLSID subtree for a single APO under
/// `HKEY_CURRENT_USER`. Convenience wrapper around
/// [`write_registry_with`] for the SISO `ApoVTable`.
pub fn write_registry(vtable: &ApoVTable, dll_path_wide: &[u16]) -> windows_core::Result<()> {
    write_registry_with(vtable.clsid, vtable.name, dll_path_wide)
}

/// Write the CLSID subtree under `HKEY_CURRENT_USER` from the
/// minimum metadata: CLSID + friendly name + DLL path.
///
/// Used by both the SISO `write_registry` and the AEC
/// `aec_dll_register_server_dispatch` to avoid duplicating the
/// registry-write logic across the two vtable types.
pub fn write_registry_with(
    clsid: Clsid,
    name: &str,
    dll_path_wide: &[u16],
) -> windows_core::Result<()> {
    let clsid_path = clsid_subkey(&clsid);
    let key = create_subkey(HKEY_CURRENT_USER, &clsid_path)?;
    set_default_value(key, name);
    close(key);

    let inproc_path = inproc_subkey(&clsid);
    let inproc = create_subkey(HKEY_CURRENT_USER, &inproc_path)?;
    let set_path = set_default_wide(inproc, dll_path_wide);
    let set_threading = set_named_value(inproc, "ThreadingModel", "Both");
    close(inproc);
    set_path?;
    set_threading?;

    Ok(())
}

/// Delete the CLSID subtree under `HKEY_CURRENT_USER`.
///
/// Idempotent: returns `Ok(())` whether the subtree existed or not.
pub fn clear_registry(clsid: &Clsid) -> windows_core::Result<()> {
    let path = clsid_subkey(clsid);
    let path_wide = encode_wide(&path);
    // Safety: HKEY_CURRENT_USER is a sentinel constant; path_wide
    // is a valid null-terminated UTF-16 string for the duration of
    // the call.
    let err = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(path_wide.as_ptr())) };
    if err.is_ok() || err == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(err.into())
    }
}

fn clsid_subkey(clsid: &Clsid) -> String {
    let mut s = String::with_capacity(64);
    let _ = write!(s, "Software\\Classes\\CLSID\\{clsid}");
    s
}

fn inproc_subkey(clsid: &Clsid) -> String {
    let mut s = String::with_capacity(80);
    let _ = write!(s, "Software\\Classes\\CLSID\\{clsid}\\InprocServer32");
    s
}

fn encode_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

fn create_subkey(parent: HKEY, path: &str) -> windows_core::Result<HKEY> {
    let path_wide = encode_wide(path);
    let mut key = HKEY::default();
    // Safety: path_wide is a valid null-terminated UTF-16 string
    // for the duration of the call; we hand it to RegCreateKeyExW
    // via PCWSTR. All output parameters are stack locals.
    let err = unsafe {
        RegCreateKeyExW(
            parent,
            PCWSTR(path_wide.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
    };
    err.ok().map(|()| key)
}

fn close(key: HKEY) {
    // Safety: key is a live handle from create_subkey.
    let _ = unsafe { RegCloseKey(key) };
}

fn set_default_value(key: HKEY, value: &str) {
    let wide = encode_wide(value);
    // Safety: wide is a valid null-terminated UTF-16 buffer; bytes
    // length below stays within its bounds.
    let _ = unsafe { RegSetValueExW(key, PCWSTR::null(), None, REG_SZ, Some(wide_bytes(&wide))) };
}

fn set_default_wide(key: HKEY, wide: &[u16]) -> windows_core::Result<()> {
    // Safety: wide is a null-terminated UTF-16 buffer supplied by
    // the caller; wide_bytes reinterprets it as a byte slice for
    // RegSetValueExW.
    unsafe { RegSetValueExW(key, PCWSTR::null(), None, REG_SZ, Some(wide_bytes(wide))).ok() }
}

fn set_named_value(key: HKEY, name: &str, value: &str) -> windows_core::Result<()> {
    let name_wide = encode_wide(name);
    let value_wide = encode_wide(value);
    // Safety: both buffers live until the end of this function;
    // bytes length stays within bounds.
    unsafe {
        RegSetValueExW(
            key,
            PCWSTR(name_wide.as_ptr()),
            None,
            REG_SZ,
            Some(wide_bytes(&value_wide)),
        )
        .ok()
    }
}

fn wide_bytes(w: &[u16]) -> &[u8] {
    // Safety: u16 → u8 reinterpretation is well-defined as long as
    // the length is doubled. REG_SZ expects the byte count
    // including the trailing null terminator, which `encode_wide`
    // already appended.
    unsafe { core::slice::from_raw_parts(w.as_ptr().cast::<u8>(), core::mem::size_of_val(w)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clsid_subkey_uses_canonical_dash_separated_brace_form() {
        let c = Clsid::from_u128(0x12345678_1234_5678_1234_567812345678);
        assert_eq!(
            clsid_subkey(&c),
            "Software\\Classes\\CLSID\\{12345678-1234-5678-1234-567812345678}"
        );
    }

    #[test]
    fn inproc_subkey_extends_the_clsid_path() {
        let c = Clsid::from_u128(0xAABBCCDD_EEFF_0011_2233_445566778899);
        assert_eq!(
            inproc_subkey(&c),
            "Software\\Classes\\CLSID\\{AABBCCDD-EEFF-0011-2233-445566778899}\\InprocServer32"
        );
    }

    #[test]
    fn encode_wide_null_terminates() {
        let w = encode_wide("ab");
        assert_eq!(w, [b'a' as u16, b'b' as u16, 0]);
    }

    #[test]
    fn wide_bytes_doubles_length() {
        let w = [u16::from(b'a'), u16::from(b'b'), 0];
        let b = wide_bytes(&w);
        assert_eq!(b.len(), 6);
    }
}
