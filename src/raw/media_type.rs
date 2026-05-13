//! Bridge between the Windows [`IAudioMediaType`] COM interface and
//! the framework's [`Format`] value type.
//!
//! The audio engine exchanges PCM stream descriptions with an APO
//! through `IAudioMediaType` instances. To answer
//! `IAudioProcessingObject::IsInputFormatSupported` and its output
//! counterpart, the framework needs to:
//!
//! 1. Read a `Format` out of a host-supplied `IAudioMediaType`
//!    (see [`format_from_media_type`]).
//! 2. Surface a `Format` back to the host as a fresh
//!    `IAudioMediaType` (see [`media_type_from_format`]).
//!
//! `IAudioMediaType` carries more than just a `WAVEFORMATEX` —
//! `IsCompressedFormat`, `IsEqual`, and `GetUncompressedAudioFormat`
//! are also part of the interface — but the audio engine's
//! negotiation path consults `GetAudioFormat` first and never needs
//! the heavier surface for the formats this framework returns
//! (uncompressed PCM only). The wrapper therefore reports
//! `IsCompressedFormat=FALSE` and stubs out the rest with
//! `E_NOTIMPL`. If a future feature needs the full surface the stubs
//! can be filled in without changing callers.
//!
//! ## Negotiation HRESULT mapping
//!
//! Native APOs distinguish *accept* from *suggest* by returning
//! `S_OK` vs `S_FALSE`; the `windows-rs` `Result<IAudioMediaType>`
//! signature collapses both into the `Ok` arm and always writes the
//! returned interface to the engine's out-pointer. The audio engine
//! reads the returned format regardless, so the contract is preserved
//! in practice: on `Accept` the framework echoes the requested format
//! back, and on `Suggest` it hands over the alternative. Callers that
//! receive an identical format treat it as acceptance.

// The `windows_core::implement` proc-macro generates a sibling
// `*_Impl` struct without doc-comments; the crate-wide
// `#![deny(missing_docs)]` would otherwise reject the expansion.
#![allow(missing_docs)]

extern crate alloc;

use windows::Win32::Media::Audio::Apo::{
    IAudioMediaType, IAudioMediaType_Impl, UNCOMPRESSEDAUDIOFORMAT,
};
use windows::Win32::Media::Audio::WAVEFORMATEX;
use windows_core::{implement, ComObject, Ref, BOOL, HRESULT};

use crate::error::HResult;
use crate::format::{Format, FormatNegotiation};
use crate::instance::AnyApoInstance;

/// In-process `IAudioMediaType` carrier owning a `WAVEFORMATEX`.
///
/// Returned from the framework's `IsInputFormatSupported` /
/// `IsOutputFormatSupported` answers; the audio engine reads the
/// underlying format back through `IAudioMediaType::GetAudioFormat`.
#[implement(IAudioMediaType)]
pub struct FormatMediaType {
    wf: WAVEFORMATEX,
}

impl FormatMediaType {
    /// Construct a [`FormatMediaType`] holding the `WAVEFORMATEX`
    /// projection of `format`.
    #[must_use]
    pub fn new(format: &Format) -> Self {
        Self {
            wf: format.to_waveformatex(),
        }
    }
}

impl IAudioMediaType_Impl for FormatMediaType_Impl {
    fn IsCompressedFormat(&self) -> windows_core::Result<BOOL> {
        // The framework only models PCM (integer / float) formats.
        Ok(BOOL::from(false))
    }

    fn IsEqual(&self, _piaudiotype: Ref<IAudioMediaType>) -> windows_core::Result<u32> {
        // The audio engine does not call IsEqual on the formats
        // we return; surface E_NOTIMPL so that the omission is
        // explicit if a future caller does start relying on it.
        Err(windows_core::Error::new(
            HRESULT::from(HResult::E_NOTIMPL),
            "FormatMediaType::IsEqual is not part of the bridge surface",
        ))
    }

    fn GetAudioFormat(&self) -> *mut WAVEFORMATEX {
        // Interior pointer into the wrapper's owned WAVEFORMATEX.
        // The audio engine only reads through the pointer during
        // the lifetime of the wrapper's IAudioMediaType reference.
        core::ptr::addr_of!(self.wf) as *mut WAVEFORMATEX
    }

    fn GetUncompressedAudioFormat(
        &self,
        _puncompressedaudioformat: *mut UNCOMPRESSEDAUDIOFORMAT,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::new(
            HRESULT::from(HResult::E_NOTIMPL),
            "FormatMediaType::GetUncompressedAudioFormat is not part of the bridge surface",
        ))
    }
}

/// Construct an [`IAudioMediaType`] backed by a fresh
/// [`FormatMediaType`].
///
/// Wraps the user-supplied [`Format`] in a `WAVEFORMATEX`-bearing
/// COM object that the audio engine can pass straight back into
/// `LockForProcess`.
#[must_use]
pub fn media_type_from_format(format: &Format) -> IAudioMediaType {
    ComObject::new(FormatMediaType::new(format)).into_interface()
}

/// Read a [`Format`] out of an `IAudioMediaType` reference handed in
/// by the audio engine.
///
/// Returns `E_POINTER` when the reference is null, and
/// `APOERR_FORMAT_NOT_SUPPORTED` when `IAudioMediaType::GetAudioFormat`
/// returns a null `WAVEFORMATEX` pointer. The returned `Format`
/// holds a deep copy of the underlying fields; no references into
/// the host-owned `WAVEFORMATEX` survive the call.
pub fn format_from_media_type(media: Ref<'_, IAudioMediaType>) -> windows_core::Result<Format> {
    let Some(mt) = media.as_ref() else {
        return Err(windows_core::Error::new(
            HRESULT::from(HResult::E_POINTER),
            "IAudioMediaType reference was null",
        ));
    };
    // Safety: the COM caller hands us a valid IAudioMediaType for
    // the duration of the call. GetAudioFormat returns an interior
    // pointer into the engine's owned WAVEFORMATEX.
    let wf_ptr = unsafe { mt.GetAudioFormat() };
    if wf_ptr.is_null() {
        return Err(windows_core::Error::new(
            HRESULT::from(HResult::APOERR_FORMAT_NOT_SUPPORTED),
            "IAudioMediaType::GetAudioFormat returned null",
        ));
    }
    // Safety: GetAudioFormat returned non-null; the pointee is
    // valid for the duration of the call. Format::from_waveformatex
    // copies the fields out.
    Ok(Format::from_waveformatex(unsafe { &*wf_ptr }))
}

/// Which direction of negotiation a bridge call is servicing.
///
/// Lets [`negotiate_format`] route to the matching
/// [`AnyApoInstance`] method without duplicating the surrounding
/// translation logic.
#[derive(Copy, Clone, Debug)]
pub enum NegotiationDirection {
    /// Servicing `IAudioProcessingObject::IsInputFormatSupported`.
    Input,
    /// Servicing `IAudioProcessingObject::IsOutputFormatSupported`.
    Output,
}

/// Translate a host-supplied requested-format `IAudioMediaType`
/// through the user APO's [`FormatNegotiation`] decision and surface
/// the result as a fresh `IAudioMediaType`.
///
/// - [`FormatNegotiation::Accept`] → returns the requested format
///   echoed back to the engine; see the module-level note on the
///   `S_OK` / `S_FALSE` collapse.
/// - [`FormatNegotiation::Suggest`] → returns the suggested format.
/// - [`FormatNegotiation::Reject`] → returns
///   `APOERR_FORMAT_NOT_SUPPORTED`.
pub fn negotiate_format(
    instance: &dyn AnyApoInstance,
    requested: Ref<'_, IAudioMediaType>,
    direction: NegotiationDirection,
) -> windows_core::Result<IAudioMediaType> {
    let requested_format = format_from_media_type(requested)?;
    let decision = match direction {
        NegotiationDirection::Input => instance.is_input_format_supported(&requested_format),
        NegotiationDirection::Output => instance.is_output_format_supported(&requested_format),
    };
    match decision {
        FormatNegotiation::Accept => Ok(media_type_from_format(&requested_format)),
        FormatNegotiation::Suggest(alt) => Ok(media_type_from_format(&alt)),
        FormatNegotiation::Reject => Err(windows_core::Error::new(
            HRESULT::from(HResult::APOERR_FORMAT_NOT_SUPPORTED),
            "ProcessingObject rejected the requested format with no alternative",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apo::{ApoCategory, ProcessInput, ProcessingObject};
    use crate::buffer::BufferFlags;
    use crate::clsid::Clsid;
    use crate::format::WAVE_FORMAT_IEEE_FLOAT;
    use crate::instance::ApoInstance;
    use crate::realtime::RealtimeContext;
    use alloc::sync::Arc;

    struct AcceptFloat32;
    impl ProcessingObject for AcceptFloat32 {
        const CLSID: Clsid = Clsid::from_u128(0x11111111_2222_3333_4444_555555555555);
        const NAME: &'static str = "accept-float32";
        const COPYRIGHT: &'static str = "test";
        const CATEGORY: ApoCategory = ApoCategory::Sfx;
        fn new() -> Self {
            Self
        }
        fn process(
            &mut self,
            _rt: &RealtimeContext,
            input: ProcessInput<'_>,
            output: &mut [f32],
        ) -> BufferFlags {
            output.copy_from_slice(input.samples());
            input.flags()
        }
    }

    struct RejectEverything;
    impl ProcessingObject for RejectEverything {
        const CLSID: Clsid = Clsid::from_u128(0x66666666_7777_8888_9999_AAAAAAAAAAAA);
        const NAME: &'static str = "reject-everything";
        const COPYRIGHT: &'static str = "test";
        const CATEGORY: ApoCategory = ApoCategory::Sfx;
        fn new() -> Self {
            Self
        }
        fn is_input_format_supported(&self, _format: &Format) -> FormatNegotiation {
            FormatNegotiation::Reject
        }
        fn is_output_format_supported(&self, _format: &Format) -> FormatNegotiation {
            FormatNegotiation::Reject
        }
        fn process(
            &mut self,
            _rt: &RealtimeContext,
            _input: ProcessInput<'_>,
            output: &mut [f32],
        ) -> BufferFlags {
            output.fill(0.0);
            BufferFlags::SILENT
        }
    }

    fn read_format(media: &IAudioMediaType) -> Format {
        // Safety: media is a live IAudioMediaType returned by the
        // bridge under test; GetAudioFormat returns an interior
        // pointer valid for the borrow.
        unsafe {
            let wf = media.GetAudioFormat();
            assert!(!wf.is_null());
            Format::from_waveformatex(&*wf)
        }
    }

    #[test]
    fn format_media_type_round_trips_via_get_audio_format() {
        let f = Format::pcm_float32(48_000, 1);
        let media = media_type_from_format(&f);
        let echoed = read_format(&media);
        assert_eq!(echoed, f);
    }

    #[test]
    fn format_media_type_reports_uncompressed() {
        let f = Format::pcm_int16(44_100, 2);
        let media = media_type_from_format(&f);
        // Safety: live interface returned by the bridge.
        let compressed = unsafe { media.IsCompressedFormat() }.unwrap();
        assert!(!compressed.as_bool());
    }

    #[test]
    fn format_from_media_type_reads_back_the_requested_format() {
        let requested = media_type_from_format(&Format::pcm_int16(48_000, 1));
        let r = Ref::from(&requested);
        let parsed = format_from_media_type(r).unwrap();
        assert_eq!(parsed, Format::pcm_int16(48_000, 1));
    }

    #[test]
    fn format_from_media_type_rejects_null_reference() {
        let r: Ref<'_, IAudioMediaType> = Ref::default();
        let err = format_from_media_type(r).unwrap_err();
        assert_eq!(err.code(), HRESULT::from(HResult::E_POINTER));
    }

    #[test]
    fn negotiate_format_accept_echoes_requested_for_float32() {
        let inst: Arc<dyn AnyApoInstance> = Arc::new(ApoInstance::<AcceptFloat32>::new());
        let requested = media_type_from_format(&Format::pcm_float32(48_000, 1));
        let r = Ref::from(&requested);
        let answer = negotiate_format(inst.as_ref(), r, NegotiationDirection::Input).unwrap();
        assert_eq!(read_format(&answer), Format::pcm_float32(48_000, 1));
    }

    #[test]
    fn negotiate_format_suggest_returns_float32_alternative_for_int16() {
        let inst: Arc<dyn AnyApoInstance> = Arc::new(ApoInstance::<AcceptFloat32>::new());
        let requested = media_type_from_format(&Format::pcm_int16(48_000, 1));
        let r = Ref::from(&requested);
        let answer = negotiate_format(inst.as_ref(), r, NegotiationDirection::Input).unwrap();
        let suggested = read_format(&answer);
        assert_eq!(suggested.format_tag(), WAVE_FORMAT_IEEE_FLOAT);
        assert_eq!(suggested.bits_per_sample(), 32);
        assert_eq!(suggested.sample_rate(), 48_000);
        assert_eq!(suggested.channels(), 1);
    }

    #[test]
    fn negotiate_format_output_direction_routes_through_is_output() {
        // Mirror the input test on the output direction to make
        // sure the discriminant wiring is correct.
        let inst: Arc<dyn AnyApoInstance> = Arc::new(ApoInstance::<AcceptFloat32>::new());
        let requested = media_type_from_format(&Format::pcm_float32(44_100, 2));
        let r = Ref::from(&requested);
        let answer = negotiate_format(inst.as_ref(), r, NegotiationDirection::Output).unwrap();
        assert_eq!(read_format(&answer), Format::pcm_float32(44_100, 2));
    }

    #[test]
    fn negotiate_format_reject_surfaces_apoerr_format_not_supported() {
        let inst: Arc<dyn AnyApoInstance> = Arc::new(ApoInstance::<RejectEverything>::new());
        let requested = media_type_from_format(&Format::pcm_float32(48_000, 1));
        let r = Ref::from(&requested);
        let err = negotiate_format(inst.as_ref(), r, NegotiationDirection::Input).unwrap_err();
        assert_eq!(
            err.code(),
            HRESULT::from(HResult::APOERR_FORMAT_NOT_SUPPORTED)
        );
    }

    #[test]
    fn negotiate_format_propagates_null_requested_as_e_pointer() {
        let inst: Arc<dyn AnyApoInstance> = Arc::new(ApoInstance::<AcceptFloat32>::new());
        let r: Ref<'_, IAudioMediaType> = Ref::default();
        let err = negotiate_format(inst.as_ref(), r, NegotiationDirection::Input).unwrap_err();
        assert_eq!(err.code(), HRESULT::from(HResult::E_POINTER));
    }
}
