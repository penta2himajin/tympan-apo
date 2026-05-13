//! Compile-time ABI invariants for the Windows COM types the
//! framework consumes.
//!
//! These asserts guard the layout-drift class of bugs that would
//! otherwise only surface at runtime under the audio engine: a
//! future `windows-rs` release that quietly changed the size or
//! alignment of `WAVEFORMATEX` / `APO_CONNECTION_PROPERTY` /
//! `APO_REG_PROPERTIES` would force a recompile-time failure here
//! rather than a hard-to-diagnose corruption at `audiodg.exe`
//! load.
//!
//! Microsoft's published layouts (per `audioenginebaseapo.h` and
//! `mmreg.h`) are the source of truth:
//!
//! - `WAVEFORMATEX` is `#[repr(C, packed(1))]`, 18 bytes, alignment 1.
//! - `APO_CONNECTION_PROPERTY` is `#[repr(C)]` with a leading
//!   `UINT_PTR` field; on a 64-bit target the size is 24 bytes
//!   (20 bytes of fields + 4 bytes tail padding) and alignment 8.
//! - `APO_REG_PROPERTIES` is `#[repr(C)]` with a `[GUID; 1]` tail
//!   that the builder in [`crate::raw::reg_properties`] grows past
//!   to emit a longer IID list; the published base size is 1092
//!   bytes.
//!
//! No runtime cost: each `const _: () = assert!(...)` is a const
//! evaluation that emits no symbol.

use windows::Win32::Media::Audio::Apo::{
    APO_BUFFER_FLAGS, APO_CONNECTION_PROPERTY, APO_REG_PROPERTIES,
};
use windows::Win32::Media::Audio::{WAVEFORMATEX, WAVEFORMATEXTENSIBLE};
use windows_core::GUID;

// WAVEFORMATEX: packed(1).
const _: () = assert!(core::mem::size_of::<WAVEFORMATEX>() == 18);
const _: () = assert!(core::mem::align_of::<WAVEFORMATEX>() == 1);

// WAVEFORMATEXTENSIBLE: packed(1); WAVEFORMATEX (18) + Samples
// union (2) + dwChannelMask (4) + SubFormat (16) = 40 bytes.
const _: () = assert!(core::mem::size_of::<WAVEFORMATEXTENSIBLE>() == 40);
const _: () = assert!(core::mem::align_of::<WAVEFORMATEXTENSIBLE>() == 1);

// APO_BUFFER_FLAGS is a `#[repr(transparent)]` newtype over i32,
// matching the C `enum APO_BUFFER_FLAGS` storage class.
const _: () = assert!(core::mem::size_of::<APO_BUFFER_FLAGS>() == 4);

// APO_CONNECTION_PROPERTY: `repr(C)` with a UINT_PTR first field. On
// 64-bit Windows the framework runs on, that pins struct alignment
// to 8 and pads the trailing u32 to a 24-byte total. The cdylib
// only links on x86_64 today; if a different pointer width becomes
// a target, this gate needs revisiting.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::size_of::<APO_CONNECTION_PROPERTY>() == 24);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::align_of::<APO_CONNECTION_PROPERTY>() == 8);

// GUID layout is the COM-wide invariant; assert it here so the
// `iidAPOInterfaceList` tail-write math in `reg_properties` rests
// on a checked base.
const _: () = assert!(core::mem::size_of::<GUID>() == 16);
const _: () = assert!(core::mem::align_of::<GUID>() == 4);

// APO_REG_PROPERTIES base size, computed from the published field
// list: 16 (clsid) + 4 (Flags) + 512 (szFriendlyName)
// + 512 (szCopyrightInfo) + 4 × 8 (eight u32 fields) + 16
// (iidAPOInterfaceList[0]) = 1092.
const _: () = assert!(core::mem::size_of::<APO_REG_PROPERTIES>() == 1092);
