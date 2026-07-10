//! Audio buffer types — flags and connection properties.
//!
//! Mirrors the small set of types from `audioenginebaseapo.h` that
//! the COM harness needs to interpret when forwarding host buffers
//! to user [`crate::ProcessingObject::process`] implementations.
//! Defined cross-platform so the realtime-safety properties can be
//! unit-tested on any host.

/// Status of an audio buffer crossing the APO boundary.
///
/// The enum values match the Windows audio engine's
/// `APO_BUFFER_FLAGS` definitions in `audioenginebaseapo.h`:
///
/// | Variant | Value | Meaning |
/// |---|---|---|
/// | [`Self::INVALID`] | `0` | Buffer contents undefined; ignore. |
/// | [`Self::VALID`] | `1` | Buffer contains valid audio data. |
/// | [`Self::SILENT`] | `2` | Buffer represents pure silence and may be skipped by the APO without producing audible artefacts. |
///
/// Defined as a `#[repr(transparent)]` newtype rather than an enum
/// so callers can round-trip the raw `i32` returned by the audio
/// engine without panicking on out-of-range values.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct BufferFlags(pub i32);

impl BufferFlags {
    /// Buffer contents are undefined.
    pub const INVALID: Self = Self(0);
    /// Buffer contains valid audio data.
    pub const VALID: Self = Self(1);
    /// Buffer is entirely silence; the APO is free to skip
    /// computation for this buffer.
    pub const SILENT: Self = Self(2);

    /// `true` iff this is exactly [`Self::VALID`].
    #[inline]
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.0 == Self::VALID.0
    }

    /// `true` iff this is exactly [`Self::SILENT`].
    #[inline]
    #[must_use]
    pub const fn is_silent(self) -> bool {
        self.0 == Self::SILENT.0
    }

    /// `true` iff this is exactly [`Self::INVALID`].
    #[inline]
    #[must_use]
    pub const fn is_invalid(self) -> bool {
        self.0 == Self::INVALID.0
    }
}

impl Default for BufferFlags {
    /// Defaults to [`Self::INVALID`], matching the Windows audio
    /// engine's `APO_BUFFER_FLAGS::default()`.
    #[inline]
    fn default() -> Self {
        Self::INVALID
    }
}

/// Audio buffer descriptor handed to / from `APOProcess`.
///
/// Cross-platform mirror of the Windows
/// `APO_CONNECTION_PROPERTY` C struct. The framework's COM harness
/// translates one of these per input and per output connection
/// when dispatching into [`crate::ProcessingObject::process`].
///
/// `buffer` is a raw address (`usize`) to match Windows's
/// `UINT_PTR pBuffer`. Higher-level code that wraps the COM
/// harness will turn this into a typed slice; doing so here is
/// premature because the `Format` negotiated between the audio
/// engine and the APO determines the element type and layout.
///
/// `signature` carries the `'APOC'` magic the host stamps onto
/// every connection (see
/// [`CONNECTION_PROPERTY_SIGNATURE`]); the harness checks it
/// before trusting the rest of the struct.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct ConnectionProperty {
    /// Raw address of the audio buffer (`pBuffer`).
    pub buffer: usize,
    /// Number of audio frames containing valid data
    /// (`u32ValidFrameCount`).
    pub valid_frame_count: u32,
    /// Buffer status flags (`u32BufferFlags`).
    pub flags: BufferFlags,
    /// `'APOC'` magic for tamper detection (`u32Signature`). The
    /// COM harness rejects buffers whose signature does not
    /// match [`CONNECTION_PROPERTY_SIGNATURE`].
    pub signature: u32,
}

impl ConnectionProperty {
    /// Construct an empty descriptor: null buffer, zero frames,
    /// `INVALID` flags, and the correct signature.
    #[inline]
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            buffer: 0,
            valid_frame_count: 0,
            flags: BufferFlags::INVALID,
            signature: CONNECTION_PROPERTY_SIGNATURE,
        }
    }
}

/// `'APOC'` — magic value the Windows audio engine stamps on every
/// `APO_CONNECTION_PROPERTY::u32Signature` field.
///
/// In big-endian byte order the bytes spell `A`, `P`, `O`, `C`.
/// The Windows audio header (`audioenginebaseapo.h`) defines the
/// constant via the four-character literal `'APOC'`, which MSVC
/// evaluates as `0x4150_4F43`.
pub const CONNECTION_PROPERTY_SIGNATURE: u32 = u32::from_be_bytes(*b"APOC");

#[cfg(windows)]
impl From<windows::Win32::Media::Audio::Apo::APO_BUFFER_FLAGS> for BufferFlags {
    #[inline]
    fn from(value: windows::Win32::Media::Audio::Apo::APO_BUFFER_FLAGS) -> Self {
        Self(value.0)
    }
}

#[cfg(windows)]
impl From<BufferFlags> for windows::Win32::Media::Audio::Apo::APO_BUFFER_FLAGS {
    #[inline]
    fn from(value: BufferFlags) -> Self {
        Self(value.0)
    }
}

#[cfg(windows)]
impl From<windows::Win32::Media::Audio::Apo::APO_CONNECTION_PROPERTY> for ConnectionProperty {
    #[inline]
    fn from(value: windows::Win32::Media::Audio::Apo::APO_CONNECTION_PROPERTY) -> Self {
        Self {
            buffer: value.pBuffer,
            valid_frame_count: value.u32ValidFrameCount,
            flags: value.u32BufferFlags.into(),
            signature: value.u32Signature,
        }
    }
}

#[cfg(windows)]
impl From<ConnectionProperty> for windows::Win32::Media::Audio::Apo::APO_CONNECTION_PROPERTY {
    #[inline]
    fn from(value: ConnectionProperty) -> Self {
        Self {
            pBuffer: value.buffer,
            u32ValidFrameCount: value.valid_frame_count,
            u32BufferFlags: value.flags.into(),
            u32Signature: value.signature,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_flag_constants_match_microsoft_values() {
        assert_eq!(BufferFlags::INVALID.0, 0);
        assert_eq!(BufferFlags::VALID.0, 1);
        assert_eq!(BufferFlags::SILENT.0, 2);
    }

    #[test]
    fn buffer_flag_predicates_classify_correctly() {
        assert!(BufferFlags::VALID.is_valid());
        assert!(!BufferFlags::VALID.is_silent());
        assert!(!BufferFlags::VALID.is_invalid());

        assert!(BufferFlags::SILENT.is_silent());
        assert!(!BufferFlags::SILENT.is_valid());

        assert!(BufferFlags::INVALID.is_invalid());
        assert!(!BufferFlags::INVALID.is_valid());
    }

    #[test]
    fn buffer_flags_default_is_invalid() {
        assert_eq!(BufferFlags::default(), BufferFlags::INVALID);
    }

    #[test]
    fn buffer_flags_unknown_value_round_trips() {
        // Out-of-range values must round-trip without panic so
        // hosts that introduce new flags do not crash this
        // framework on observation.
        let f = BufferFlags(99);
        assert!(!f.is_valid());
        assert!(!f.is_silent());
        assert!(!f.is_invalid());
        assert_eq!(f.0, 99);
    }

    #[test]
    fn connection_property_empty_has_signature() {
        let cp = ConnectionProperty::empty();
        assert_eq!(cp.signature, CONNECTION_PROPERTY_SIGNATURE);
        assert_eq!(cp.buffer, 0);
        assert_eq!(cp.valid_frame_count, 0);
        assert!(cp.flags.is_invalid());
    }

    #[test]
    fn signature_value_spells_apoc_in_be_bytes() {
        assert_eq!(CONNECTION_PROPERTY_SIGNATURE, 0x4150_4F43);
        let bytes = CONNECTION_PROPERTY_SIGNATURE.to_be_bytes();
        assert_eq!(&bytes, b"APOC");
    }

    #[cfg(windows)]
    #[test]
    fn buffer_flags_round_trip_through_windows_type() {
        use windows::Win32::Media::Audio::Apo::APO_BUFFER_FLAGS;
        for f in [
            BufferFlags::INVALID,
            BufferFlags::VALID,
            BufferFlags::SILENT,
            BufferFlags(42),
        ] {
            let w: APO_BUFFER_FLAGS = f.into();
            let back: BufferFlags = w.into();
            assert_eq!(f, back);
        }
    }

    #[cfg(windows)]
    #[test]
    fn buffer_flags_constants_match_windows_constants() {
        use windows::Win32::Media::Audio::Apo::{BUFFER_INVALID, BUFFER_SILENT, BUFFER_VALID};
        assert_eq!(BufferFlags::INVALID, BUFFER_INVALID.into());
        assert_eq!(BufferFlags::VALID, BUFFER_VALID.into());
        assert_eq!(BufferFlags::SILENT, BUFFER_SILENT.into());
    }

    #[cfg(windows)]
    #[test]
    fn connection_property_round_trips_through_windows_type() {
        use windows::Win32::Media::Audio::Apo::APO_CONNECTION_PROPERTY;
        let cp = ConnectionProperty {
            buffer: 0xDEAD_BEEF,
            valid_frame_count: 1024,
            flags: BufferFlags::VALID,
            signature: CONNECTION_PROPERTY_SIGNATURE,
        };
        let w: APO_CONNECTION_PROPERTY = cp.into();
        assert_eq!(w.pBuffer, 0xDEAD_BEEF);
        assert_eq!(w.u32ValidFrameCount, 1024);
        assert_eq!(w.u32BufferFlags.0, 1);
        assert_eq!(w.u32Signature, CONNECTION_PROPERTY_SIGNATURE);

        let back: ConnectionProperty = w.into();
        assert_eq!(cp, back);
    }
}
