//! COM Class Identifier.
//!
//! Audio Processing Objects are identified by a 128-bit COM
//! `CLSID` (a `GUID`). Each implementor of `ProcessingObject`
//! declares a `CLSID` constant; the framework's class factory
//! consults it during `DllGetClassObject` to decide whether the
//! host's request can be satisfied.
//!
//! This module exposes a cross-platform [`Clsid`] type so that
//! framework users can author and unit-test their CLSIDs on hosts
//! other than Windows. The type is `#[repr(C)]` and field-for-field
//! compatible with `windows_core::GUID`; explicit conversions are
//! provided behind `#[cfg(windows)]`.

use core::fmt;

/// COM Class Identifier — a 128-bit globally-unique identifier.
///
/// Layout-compatible with `windows_core::GUID`. The display
/// format is the canonical dash-separated hexadecimal form
/// `{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}` so error messages and
/// debug output match what `regedit` and other Windows tooling
/// show.
#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct Clsid {
    /// First 8 hexadecimal digits.
    pub data1: u32,
    /// First group of 4 hexadecimal digits.
    pub data2: u16,
    /// Second group of 4 hexadecimal digits.
    pub data3: u16,
    /// Third group of 4 plus final 12 hexadecimal digits, in
    /// network byte order.
    pub data4: [u8; 8],
}

impl Clsid {
    /// The nil CLSID (`{00000000-0000-0000-0000-000000000000}`).
    /// COM rejects this value as `CLASS_E_CLASSNOTAVAILABLE`; it
    /// is exposed only as a sentinel.
    pub const NIL: Self = Self::from_parts(0, 0, 0, [0; 8]);

    /// Build a CLSID from its four canonical fields.
    #[inline]
    #[must_use]
    pub const fn from_parts(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Self {
        Self {
            data1,
            data2,
            data3,
            data4,
        }
    }

    /// Build a CLSID from a single big-endian 128-bit integer.
    ///
    /// Useful for declaring a CLSID inline:
    ///
    /// ```
    /// use tympan_apo::Clsid;
    /// const MY_APO: Clsid =
    ///     Clsid::from_u128(0x12345678_1234_5678_1234_567812345678);
    /// ```
    #[inline]
    #[must_use]
    pub const fn from_u128(value: u128) -> Self {
        let b = value.to_be_bytes();
        Self {
            data1: u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            data2: u16::from_be_bytes([b[4], b[5]]),
            data3: u16::from_be_bytes([b[6], b[7]]),
            data4: [b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]],
        }
    }

    /// Project this CLSID into a single big-endian 128-bit integer.
    /// Inverse of [`Self::from_u128`].
    #[inline]
    #[must_use]
    pub const fn to_u128(&self) -> u128 {
        u128::from_be_bytes([
            (self.data1 >> 24) as u8,
            (self.data1 >> 16) as u8,
            (self.data1 >> 8) as u8,
            self.data1 as u8,
            (self.data2 >> 8) as u8,
            self.data2 as u8,
            (self.data3 >> 8) as u8,
            self.data3 as u8,
            self.data4[0],
            self.data4[1],
            self.data4[2],
            self.data4[3],
            self.data4[4],
            self.data4[5],
            self.data4[6],
            self.data4[7],
        ])
    }

    /// `true` iff this is [`Self::NIL`].
    #[inline]
    #[must_use]
    pub const fn is_nil(&self) -> bool {
        self.to_u128() == 0
    }
}

impl fmt::Debug for Clsid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Clsid({self})")
    }
}

impl fmt::Display for Clsid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
            self.data1,
            self.data2,
            self.data3,
            self.data4[0],
            self.data4[1],
            self.data4[2],
            self.data4[3],
            self.data4[4],
            self.data4[5],
            self.data4[6],
            self.data4[7],
        )
    }
}

#[cfg(windows)]
impl From<windows_core::GUID> for Clsid {
    #[inline]
    fn from(g: windows_core::GUID) -> Self {
        Self {
            data1: g.data1,
            data2: g.data2,
            data3: g.data3,
            data4: g.data4,
        }
    }
}

#[cfg(windows)]
impl From<Clsid> for windows_core::GUID {
    #[inline]
    fn from(c: Clsid) -> Self {
        Self {
            data1: c.data1,
            data2: c.data2,
            data3: c.data3,
            data4: c.data4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nil_is_nil() {
        assert!(Clsid::NIL.is_nil());
        assert_eq!(Clsid::NIL.to_u128(), 0);
    }

    #[test]
    fn non_nil_is_not_nil() {
        assert!(!Clsid::from_u128(1).is_nil());
        assert!(!Clsid::from_parts(0, 0, 0, [0, 0, 0, 0, 0, 0, 0, 1]).is_nil());
    }

    #[test]
    fn from_u128_round_trips() {
        let raw = 0x12345678_9abc_def0_1234_56789abcdef0_u128;
        let c = Clsid::from_u128(raw);
        assert_eq!(c.to_u128(), raw);
    }

    #[test]
    fn from_parts_matches_explicit_fields() {
        let c = Clsid::from_parts(0xDEAD_BEEF, 0x1234, 0x5678, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(c.data1, 0xDEAD_BEEF);
        assert_eq!(c.data2, 0x1234);
        assert_eq!(c.data3, 0x5678);
        assert_eq!(c.data4, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn from_u128_matches_known_byte_order() {
        // Reference value: the canonical CLSID for
        // `IUnknown` is {00000000-0000-0000-C000-000000000046}.
        let c = Clsid::from_u128(0x00000000_0000_0000_C000_000000000046);
        assert_eq!(c.data1, 0);
        assert_eq!(c.data2, 0);
        assert_eq!(c.data3, 0);
        assert_eq!(c.data4, [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46]);
    }

    #[test]
    fn display_format_is_canonical() {
        let c = Clsid::from_u128(0x12345678_9abc_def0_a1b2_c3d4e5f60718);
        assert_eq!(format!("{c}"), "{12345678-9ABC-DEF0-A1B2-C3D4E5F60718}");
    }

    #[test]
    fn debug_wraps_display() {
        let c = Clsid::from_u128(1);
        assert_eq!(
            format!("{c:?}"),
            "Clsid({00000000-0000-0000-0000-000000000001})"
        );
    }

    #[test]
    fn equality_is_structural() {
        let a = Clsid::from_u128(0x42);
        let b = Clsid::from_u128(0x42);
        let c = Clsid::from_u128(0x43);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[cfg(windows)]
    #[test]
    fn from_windows_guid_preserves_bytes() {
        let g = windows_core::GUID {
            data1: 0xDEAD_BEEF,
            data2: 0x1234,
            data3: 0x5678,
            data4: [1, 2, 3, 4, 5, 6, 7, 8],
        };
        let c: Clsid = g.into();
        assert_eq!(c.data1, 0xDEAD_BEEF);
        assert_eq!(c.data2, 0x1234);
        assert_eq!(c.data3, 0x5678);
        assert_eq!(c.data4, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[cfg(windows)]
    #[test]
    fn windows_guid_round_trips() {
        let g = windows_core::GUID {
            data1: 0x12345678,
            data2: 0x9abc,
            data3: 0xdef0,
            data4: [0xa1, 0xb2, 0xc3, 0xd4, 0xe5, 0xf6, 0x07, 0x18],
        };
        let c: Clsid = g.into();
        let g2: windows_core::GUID = c.into();
        assert_eq!(g, g2);
    }

    #[cfg(windows)]
    #[test]
    fn clsid_layout_matches_windows_guid() {
        // The two structs must be ABI-compatible so a future raw
        // module can transmute pointers when needed.
        use core::mem::{align_of, size_of};
        assert_eq!(size_of::<Clsid>(), size_of::<windows_core::GUID>());
        assert_eq!(align_of::<Clsid>(), align_of::<windows_core::GUID>());
    }
}
