//! `HRESULT` wrapper and APO-specific error constants.
//!
//! COM uses the 32-bit `HRESULT` value as its universal success /
//! failure indicator. Negative values are failures, non-negative
//! values are successes (with `S_OK` and `S_FALSE` being the two
//! common ones). The audio engine extends this with a set of APO
//! facility codes (`FACILITY_AUDIO`) documented in
//! `audioenginebaseapo.h`.
//!
//! `HResult` is a thin `#[repr(transparent)]` wrapper so it can be
//! used interchangeably with the raw `i32` at the FFI boundary,
//! including in `extern "system"` return positions.

use core::fmt;

/// COM `HRESULT`.
///
/// Layout-compatible with `i32` and with the `windows-core` crate's
/// `HRESULT(pub i32)` type. Conversions to and from
/// `windows_core::HRESULT` are provided on Windows.
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct HResult(pub i32);

impl HResult {
    /// `S_OK` — success, no further information.
    pub const S_OK: Self = Self(0);
    /// `S_FALSE` — success with a "false" semantic (e.g. operation
    /// completed but the queried condition does not hold).
    pub const S_FALSE: Self = Self(1);

    /// `E_NOTIMPL` — function is not implemented.
    pub const E_NOTIMPL: Self = Self(0x8000_4001_u32 as i32);
    /// `E_NOINTERFACE` — `QueryInterface` could not produce the
    /// requested interface.
    pub const E_NOINTERFACE: Self = Self(0x8000_4002_u32 as i32);
    /// `E_POINTER` — invalid (typically null) pointer argument.
    pub const E_POINTER: Self = Self(0x8000_4003_u32 as i32);
    /// `E_FAIL` — unspecified failure.
    pub const E_FAIL: Self = Self(0x8000_4005_u32 as i32);
    /// `E_UNEXPECTED` — catastrophic failure (last-resort code).
    pub const E_UNEXPECTED: Self = Self(0x8000_FFFF_u32 as i32);
    /// `E_OUTOFMEMORY` — allocation failure.
    pub const E_OUTOFMEMORY: Self = Self(0x8007_000E_u32 as i32);
    /// `E_INVALIDARG` — one or more arguments are invalid.
    pub const E_INVALIDARG: Self = Self(0x8007_0057_u32 as i32);

    /// `CLASS_E_CLASSNOTAVAILABLE` — `DllGetClassObject` does not
    /// recognise the requested CLSID.
    pub const CLASS_E_CLASSNOTAVAILABLE: Self = Self(0x8004_0111_u32 as i32);
    /// `CLASS_E_NOAGGREGATION` — class refuses aggregation.
    pub const CLASS_E_NOAGGREGATION: Self = Self(0x8004_0110_u32 as i32);

    /// `APOERR_INVALID_INPUT_DATA` — input data does not match the
    /// format negotiated during `LockForProcess`.
    pub const APOERR_INVALID_INPUT_DATA: Self = Self(0x8889_0001_u32 as i32);
    /// `APOERR_FORMAT_NOT_SUPPORTED` — proposed format is not
    /// supported by this APO.
    pub const APOERR_FORMAT_NOT_SUPPORTED: Self = Self(0x8889_0008_u32 as i32);
    /// `APOERR_INVALID_API_VERSION` — caller is requesting an
    /// unsupported APO API revision.
    pub const APOERR_INVALID_API_VERSION: Self = Self(0x8889_0007_u32 as i32);
    /// `APOERR_NUM_CONNECTIONS_INVALID` — the number of input or
    /// output connections is not supported.
    pub const APOERR_NUM_CONNECTIONS_INVALID: Self = Self(0x8889_000B_u32 as i32);
    /// `APOERR_NOT_LOCKED` — operation is only valid between
    /// `LockForProcess` and `UnlockForProcess`.
    pub const APOERR_NOT_LOCKED: Self = Self(0x8889_000A_u32 as i32);
    /// `APOERR_ALREADY_LOCKED` — APO is already locked.
    pub const APOERR_ALREADY_LOCKED: Self = Self(0x8889_0006_u32 as i32);

    /// Returns `true` if the underlying value is non-negative.
    #[inline]
    #[must_use]
    pub const fn is_ok(self) -> bool {
        self.0 >= 0
    }

    /// Returns `true` if the underlying value is negative.
    #[inline]
    #[must_use]
    pub const fn is_err(self) -> bool {
        self.0 < 0
    }

    /// Converts to a `Result<(), Self>`, mapping the success codes
    /// (`S_OK`, `S_FALSE`, and any other non-negative value) to `Ok`.
    #[inline]
    pub const fn ok(self) -> Result<(), Self> {
        if self.is_ok() {
            Ok(())
        } else {
            Err(self)
        }
    }
}

impl fmt::Debug for HResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HResult(0x{:08X})", self.0 as u32)
    }
}

impl fmt::Display for HResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HRESULT 0x{:08X}", self.0 as u32)
    }
}

impl From<i32> for HResult {
    #[inline]
    fn from(value: i32) -> Self {
        Self(value)
    }
}

impl From<HResult> for i32 {
    #[inline]
    fn from(value: HResult) -> Self {
        value.0
    }
}

#[cfg(windows)]
impl From<windows_core::HRESULT> for HResult {
    #[inline]
    fn from(value: windows_core::HRESULT) -> Self {
        Self(value.0)
    }
}

#[cfg(windows)]
impl From<HResult> for windows_core::HRESULT {
    #[inline]
    fn from(value: HResult) -> Self {
        Self(value.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_codes_classify_correctly() {
        assert!(HResult::S_OK.is_ok());
        assert!(!HResult::S_OK.is_err());
        assert!(HResult::S_FALSE.is_ok());
        assert!(!HResult::S_FALSE.is_err());
    }

    #[test]
    fn failure_codes_classify_correctly() {
        assert!(HResult::E_FAIL.is_err());
        assert!(!HResult::E_FAIL.is_ok());
        assert!(HResult::E_INVALIDARG.is_err());
        assert!(HResult::APOERR_FORMAT_NOT_SUPPORTED.is_err());
        assert!(HResult::APOERR_INVALID_INPUT_DATA.is_err());
        assert!(HResult::CLASS_E_CLASSNOTAVAILABLE.is_err());
    }

    #[test]
    fn ok_method_converts_to_result() {
        assert_eq!(HResult::S_OK.ok(), Ok(()));
        assert_eq!(HResult::S_FALSE.ok(), Ok(()));
        assert_eq!(HResult::E_FAIL.ok(), Err(HResult::E_FAIL));
    }

    #[test]
    fn raw_constant_values_match_microsoft_definitions() {
        // Sanity-check a handful of well-known constants against
        // `winerror.h` / `audioenginebaseapo.h`.
        assert_eq!(HResult::E_NOTIMPL.0 as u32, 0x8000_4001);
        assert_eq!(HResult::E_POINTER.0 as u32, 0x8000_4003);
        assert_eq!(HResult::E_OUTOFMEMORY.0 as u32, 0x8007_000E);
        assert_eq!(HResult::E_INVALIDARG.0 as u32, 0x8007_0057);
        assert_eq!(HResult::CLASS_E_CLASSNOTAVAILABLE.0 as u32, 0x8004_0111);
    }

    #[test]
    fn debug_formatting_is_hex() {
        let s = format!("{:?}", HResult::E_INVALIDARG);
        assert_eq!(s, "HResult(0x80070057)");
    }
}
