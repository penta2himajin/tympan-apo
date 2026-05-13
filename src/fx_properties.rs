//! Endpoint-binding helpers for the audio engine's `FxProperties`
//! registry surface.
//!
//! Once an APO's CLSID is registered (via `regsvr32` /
//! `DllRegisterServer` at the HKCU scope, or via an INF at the
//! HKLM scope), the audio engine still needs to know **which
//! endpoint** to attach it to. That association lives under
//!
//! ```text
//! HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\
//!   <Capture|Render>\{<endpoint-guid>}\FxProperties
//! ```
//!
//! with a `REG_BINARY` value name of the form
//! `{<property-fmtid>},<pid>`. The value bytes are a serialised
//! `PROPVARIANT` whose VT is `VT_CLSID` and whose payload is the
//! APO CLSID.
//!
//! This module exposes three layers:
//!
//! 1. [`fx_properties_key_path`] / [`fx_property_value_name`]
//!    return the canonical registry paths and value names. Useful
//!    for installer toolchains that generate REG or INF files
//!    without touching the registry directly.
//! 2. [`serialize_clsid_property`] returns the 24-byte
//!    `REG_BINARY` payload for a `VT_CLSID` value.
//! 3. `write_endpoint_binding` / `clear_endpoint_binding`
//!    (Windows-only) perform the actual registry writes via
//!    `RegSetValueExW` / `RegDeleteValueW`. They require admin
//!    because the `MMDevices\Audio` subtree lives under HKLM and
//!    is owned by `TrustedInstaller`.
//!
//! ## Why this is fragile
//!
//! The audio engine caches endpoint properties; writes to
//! `FxProperties` may not take effect until either a reboot or a
//! `Restart-Service AudioSrv`. The robust path for production
//! deployments is an INF file (see [`crate::inf`]) processed by
//! the Windows driver toolchain — which handles invalidation
//! automatically. The runtime helpers here are intended for
//! development, CI, and one-off installer scripts.

extern crate alloc;

use alloc::format;
use alloc::string::String;

use crate::clsid::Clsid;

/// Which side of the audio pipeline an endpoint belongs to.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum EndpointKind {
    /// Microphone / capture endpoint.
    Capture,
    /// Speaker / render endpoint.
    Render,
}

impl EndpointKind {
    /// Registry-path leaf — `"Capture"` or `"Render"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Capture => "Capture",
            Self::Render => "Render",
        }
    }
}

/// `FMTID_FX` — common GUID for the legacy FX property keys
/// (`PKEY_FX_*_Clsid` family). The individual property is
/// distinguished by a `pid`.
pub const FMTID_FX: Clsid = Clsid::from_u128(0xD04E05A6_594B_4FB6_A80D_01AF5EED7D1D);

/// Well-known `pid` values inside [`FMTID_FX`]. Modern Windows
/// versions support more keys; the framework exposes the legacy
/// triple that maps directly to the [`ApoCategory`] variants.
///
/// [`ApoCategory`]: crate::ApoCategory
pub mod pid {
    /// `PKEY_FX_PreMixCLSID` — stream effect (SFX) slot.
    pub const PRE_MIX: u32 = 0;
    /// `PKEY_FX_PostMixCLSID` — mode effect (MFX) slot.
    pub const POST_MIX: u32 = 1;
    /// `PKEY_FX_EndpointCLSID` — endpoint effect (EFX) slot.
    pub const ENDPOINT: u32 = 2;
}

/// Map an [`ApoCategory`] to the matching `FMTID_FX` pid.
///
/// [`ApoCategory`]: crate::ApoCategory
#[must_use]
pub const fn pid_for_category(category: crate::ApoCategory) -> u32 {
    match category {
        crate::ApoCategory::Sfx => pid::PRE_MIX,
        crate::ApoCategory::Mfx => pid::POST_MIX,
        crate::ApoCategory::Efx => pid::ENDPOINT,
    }
}

/// Canonical registry key path for an endpoint's `FxProperties`
/// subtree.
///
/// Returns the path as a Windows-style `\`-separated string so it
/// can be passed straight to `RegCreateKeyExW` after wide-string
/// conversion.
#[must_use]
pub fn fx_properties_key_path(kind: EndpointKind, endpoint_id: &Clsid) -> String {
    format!(
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\MMDevices\\Audio\\{}\\{}\\FxProperties",
        kind.as_str(),
        endpoint_id
    )
}

/// Canonical value name for a single `FxProperties` property.
///
/// The audio engine uses `{<fmtid>},<pid>` (no spaces) — e.g.
/// `{D04E05A6-594B-4FB6-A80D-01AF5EED7D1D},0` for the SFX slot.
#[must_use]
pub fn fx_property_value_name(fmtid: &Clsid, pid: u32) -> String {
    format!("{fmtid},{pid}")
}

/// Serialise a CLSID as a `VT_CLSID` `PROPVARIANT` body for
/// `REG_BINARY` storage.
///
/// Layout matches the in-process `PROPVARIANT` struct that the
/// audio engine's property reader deserialises:
///
/// ```text
/// offset 0:  u32  vt      (little-endian VT_CLSID = 72)
/// offset 4:  u32  reserved (zero)
/// offset 8:  GUID body    (16 bytes)
/// ```
///
/// Total 24 bytes.
#[must_use]
pub fn serialize_clsid_property(clsid: &Clsid) -> [u8; 24] {
    const VT_CLSID: u32 = 72;
    let mut buf = [0u8; 24];
    buf[0..4].copy_from_slice(&VT_CLSID.to_le_bytes());
    // reserved bytes [4..8] stay zero
    buf[8..12].copy_from_slice(&clsid.data1.to_le_bytes());
    buf[12..14].copy_from_slice(&clsid.data2.to_le_bytes());
    buf[14..16].copy_from_slice(&clsid.data3.to_le_bytes());
    buf[16..24].copy_from_slice(&clsid.data4);
    buf
}

#[cfg(windows)]
pub use windows_impl::{clear_endpoint_binding, write_endpoint_binding};

#[cfg(windows)]
mod windows_impl {
    extern crate alloc;
    use alloc::vec::Vec;

    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE,
        KEY_WRITE, REG_BINARY, REG_OPTION_NON_VOLATILE,
    };
    use windows_core::PCWSTR;

    use crate::clsid::Clsid;
    use crate::error::HResult;

    use super::{
        fx_properties_key_path, fx_property_value_name, serialize_clsid_property, EndpointKind,
    };

    /// Write a CLSID `FxProperties` binding for an endpoint.
    ///
    /// Opens (or creates) the per-endpoint `FxProperties` key under
    /// `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\
    /// Audio\<kind>\{endpoint}` and writes a `REG_BINARY` value
    /// named `{fmtid},pid` whose payload is the
    /// [`serialize_clsid_property`] encoding of `apo_clsid`.
    ///
    /// Requires the calling process to run elevated; HKLM writes
    /// to the `MMDevices\Audio` subtree are governed by the
    /// `TrustedInstaller` ACL on most Windows installs.
    pub fn write_endpoint_binding(
        kind: EndpointKind,
        endpoint_id: &Clsid,
        fmtid: &Clsid,
        pid: u32,
        apo_clsid: &Clsid,
    ) -> windows_core::Result<()> {
        let path = fx_properties_key_path(kind, endpoint_id);
        let key = create_subkey(&path)?;
        let value_name = fx_property_value_name(fmtid, pid);
        let value_wide: Vec<u16> = value_name
            .encode_utf16()
            .chain(core::iter::once(0))
            .collect();
        let payload = serialize_clsid_property(apo_clsid);
        // Safety: value_wide is null-terminated UTF-16 for the
        // duration of the call; payload is a 24-byte stack array;
        // both stay alive past RegSetValueExW.
        let err = unsafe {
            RegSetValueExW(
                key,
                PCWSTR(value_wide.as_ptr()),
                None,
                REG_BINARY,
                Some(&payload),
            )
        };
        close(key);
        err.ok()
    }

    /// Remove a previously-written `FxProperties` binding.
    /// Idempotent — `ERROR_FILE_NOT_FOUND` is swallowed.
    pub fn clear_endpoint_binding(
        kind: EndpointKind,
        endpoint_id: &Clsid,
        fmtid: &Clsid,
        pid: u32,
    ) -> windows_core::Result<()> {
        let path = fx_properties_key_path(kind, endpoint_id);
        let key = match create_subkey(&path) {
            Ok(k) => k,
            Err(e) if e.code() == HResult::E_INVALIDARG.into() => return Ok(()),
            Err(e) => return Err(e),
        };
        let value_name = fx_property_value_name(fmtid, pid);
        let value_wide: Vec<u16> = value_name
            .encode_utf16()
            .chain(core::iter::once(0))
            .collect();
        // Safety: value_wide is null-terminated UTF-16.
        let err = unsafe { RegDeleteValueW(key, PCWSTR(value_wide.as_ptr())) };
        close(key);
        if err.is_ok() || err == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(err.into())
        }
    }

    fn create_subkey(path: &str) -> windows_core::Result<HKEY> {
        let path_wide: Vec<u16> = path.encode_utf16().chain(core::iter::once(0)).collect();
        let mut key = HKEY::default();
        // Safety: path_wide is a valid null-terminated UTF-16
        // string; KEY_WRITE requires elevation under the
        // MMDevices\Audio subtree.
        let err = unsafe {
            RegCreateKeyExW(
                HKEY_LOCAL_MACHINE,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ApoCategory;

    #[test]
    fn endpoint_kind_as_str_matches_microsoft_naming() {
        assert_eq!(EndpointKind::Capture.as_str(), "Capture");
        assert_eq!(EndpointKind::Render.as_str(), "Render");
    }

    #[test]
    fn fx_properties_key_path_is_canonical() {
        let endpoint = Clsid::from_u128(0x12345678_1234_5678_1234_567812345678);
        let path = fx_properties_key_path(EndpointKind::Capture, &endpoint);
        assert_eq!(
            path,
            "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\MMDevices\\Audio\\Capture\\\
             {12345678-1234-5678-1234-567812345678}\\FxProperties"
        );
    }

    #[test]
    fn fx_property_value_name_uses_brace_comma_format() {
        let fmtid = FMTID_FX;
        let name = fx_property_value_name(&fmtid, pid::POST_MIX);
        assert_eq!(name, "{D04E05A6-594B-4FB6-A80D-01AF5EED7D1D},1");
    }

    #[test]
    fn pid_for_category_maps_to_legacy_triple() {
        assert_eq!(pid_for_category(ApoCategory::Sfx), pid::PRE_MIX);
        assert_eq!(pid_for_category(ApoCategory::Mfx), pid::POST_MIX);
        assert_eq!(pid_for_category(ApoCategory::Efx), pid::ENDPOINT);
    }

    #[test]
    fn serialize_clsid_property_emits_vt_clsid_layout() {
        let c = Clsid::from_u128(0x01020304_0506_0708_090A_0B0C0D0E0F10);
        let bytes = serialize_clsid_property(&c);
        // VT_CLSID = 72 in little-endian.
        assert_eq!(&bytes[0..4], &[72, 0, 0, 0]);
        // Reserved.
        assert_eq!(&bytes[4..8], &[0, 0, 0, 0]);
        // GUID body: data1 LE, data2 LE, data3 LE, data4 in order.
        assert_eq!(&bytes[8..12], &0x01020304u32.to_le_bytes());
        assert_eq!(&bytes[12..14], &0x0506u16.to_le_bytes());
        assert_eq!(&bytes[14..16], &0x0708u16.to_le_bytes());
        assert_eq!(
            &bytes[16..24],
            &[0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10]
        );
    }

    #[test]
    fn serialize_clsid_property_total_length_is_24_bytes() {
        let c = Clsid::from_u128(0);
        let bytes = serialize_clsid_property(&c);
        assert_eq!(bytes.len(), 24);
    }
}
