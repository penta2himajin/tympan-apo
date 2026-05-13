//! COM wrapper bridging `AnyApoInstance` to the Windows
//! `IAudioProcessingObject` family.
//!
//! [`ApoInstanceCom`] holds an `Arc<dyn AnyApoInstance>` and
//! exposes it to the audio engine through the
//! [`IAudioProcessingObject`] vtable that `windows_core::implement`
//! generates from this type.
//!
//! ## Implementation status
//!
//! All `IAudioProcessingObject` / `IAudioProcessingObjectConfiguration`
//! / `IAudioProcessingObjectRT` methods are wired through to the
//! user APO via [`AnyApoInstance`]. The format-negotiation pair
//! routes through the [`crate::raw::media_type`] bridge.
//! `GetRegistrationProperties` synthesises the variable-length
//! `APO_REG_PROPERTIES` payload through
//! [`crate::raw::reg_properties::build_registration_properties`].

// The `windows_core::implement` proc-macro generates a sibling
// `*_Impl` struct without doc-comments; the crate-wide
// `#![deny(missing_docs)]` would otherwise reject the expansion.
#![allow(missing_docs)]

extern crate alloc;

use alloc::sync::Arc;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Media::Audio::Apo::{
    IAudioMediaType, IAudioProcessingObject, IAudioProcessingObjectConfiguration,
    IAudioProcessingObjectConfiguration_Impl, IAudioProcessingObjectRT,
    IAudioProcessingObjectRT_Impl, IAudioProcessingObject_Impl, IAudioSystemEffects,
    IAudioSystemEffects2, IAudioSystemEffects2_Impl, IAudioSystemEffects3,
    IAudioSystemEffects3_Impl, IAudioSystemEffects_Impl, APO_CONNECTION_DESCRIPTOR,
    APO_CONNECTION_PROPERTY, APO_REG_PROPERTIES, AUDIO_SYSTEMEFFECT, AUDIO_SYSTEMEFFECT_STATE,
    AUDIO_SYSTEMEFFECT_STATE_OFF, AUDIO_SYSTEMEFFECT_STATE_ON,
};
use windows_core::{implement, Ref, BOOL, GUID, HRESULT};

use crate::apo::SystemEffectState;

use crate::apo::ProcessInput;
use crate::buffer::{BufferFlags, CONNECTION_PROPERTY_SIGNATURE};
use crate::error::HResult;
use crate::format::Format;
use crate::instance::AnyApoInstance;
use crate::realtime::RealtimeContext;

/// COM-side carrier for an [`Arc<dyn AnyApoInstance>`](AnyApoInstance).
///
/// One of these is materialised per `IClassFactory::CreateInstance`
/// call. The carrier is what the audio engine sees as an
/// `IAudioProcessingObject*`; methods on the COM interface route
/// through this struct into the user's `ProcessingObject` via the
/// type-erased trait.
#[implement(
    IAudioProcessingObject,
    IAudioProcessingObjectConfiguration,
    IAudioProcessingObjectRT,
    IAudioSystemEffects,
    IAudioSystemEffects2,
    IAudioSystemEffects3
)]
pub struct ApoInstanceCom {
    instance: Arc<dyn AnyApoInstance>,
}

impl ApoInstanceCom {
    /// Wrap an existing instance for COM exposure.
    ///
    /// Called by the framework's class factory; users do not
    /// construct this directly. Increments the cdylib's
    /// outstanding-instance counter so `DllCanUnloadNow` reports
    /// `S_FALSE` while this object is live.
    #[must_use]
    pub fn new(instance: Arc<dyn AnyApoInstance>) -> Self {
        crate::raw::exports::outstanding_inc();
        Self { instance }
    }

    /// Borrow the underlying `AnyApoInstance`. Used by the
    /// future IAudioProcessingObjectConfiguration / RT wrappers.
    #[must_use]
    pub fn instance(&self) -> &Arc<dyn AnyApoInstance> {
        &self.instance
    }
}

impl Drop for ApoInstanceCom {
    fn drop(&mut self) {
        // Symmetric counterpart of the `outstanding_inc` in `new`.
        // Fires when the COM refcount on the wrapping `ComObject`
        // reaches zero and the box is freed.
        crate::raw::exports::outstanding_dec();
    }
}

impl IAudioProcessingObject_Impl for ApoInstanceCom_Impl {
    fn Reset(&self) -> windows_core::Result<()> {
        // The framework has no behavioural reset hook on the
        // ProcessingObject trait yet; users that need one will
        // override unlock_for_process. Treat Reset as a no-op.
        Ok(())
    }

    fn GetLatency(&self) -> windows_core::Result<i64> {
        // The trait does not yet expose latency; report zero
        // (matching the SYSVAD passthrough sample) until a
        // ProcessingObject::latency_hns method lands.
        Ok(0)
    }

    fn GetRegistrationProperties(&self) -> windows_core::Result<*mut APO_REG_PROPERTIES> {
        // The audio engine takes ownership of the returned buffer
        // and releases it with CoTaskMemFree. The builder allocates
        // with the matching CoTaskMemAlloc.
        crate::raw::reg_properties::build_registration_properties(self.instance.as_ref())
    }

    fn Initialize(&self, _cbdatasize: u32, _pbydata: *const u8) -> windows_core::Result<()> {
        // The framework's `AnyApoInstance::initialize` does not
        // currently consume user-supplied initialisation blobs;
        // delegate straight through.
        self.instance.initialize().map_err(|e| {
            windows_core::Error::new(HRESULT::from(e), "ProcessingObject::initialize failed")
        })
    }

    fn IsInputFormatSupported(
        &self,
        _poppositeformat: Ref<IAudioMediaType>,
        prequestedinputformat: Ref<IAudioMediaType>,
    ) -> windows_core::Result<IAudioMediaType> {
        // The framework currently ignores `poppositeformat` —
        // negotiation only consults the requested input format
        // and trusts the user's `ProcessingObject::is_input_format_supported`
        // for the verdict. A future revision can refine the
        // contract by surfacing the opposite format to the user.
        crate::raw::media_type::negotiate_format(
            self.instance.as_ref(),
            prequestedinputformat,
            crate::raw::media_type::NegotiationDirection::Input,
        )
    }

    fn IsOutputFormatSupported(
        &self,
        _poppositeformat: Ref<IAudioMediaType>,
        prequestedoutputformat: Ref<IAudioMediaType>,
    ) -> windows_core::Result<IAudioMediaType> {
        crate::raw::media_type::negotiate_format(
            self.instance.as_ref(),
            prequestedoutputformat,
            crate::raw::media_type::NegotiationDirection::Output,
        )
    }

    fn GetInputChannelCount(&self) -> windows_core::Result<u32> {
        // The audio engine queries this after LockForProcess.
        // Read the channel count out of the cached negotiated
        // input format; the cache is populated whenever the cell
        // is in `State::Locked` and cleared on `UnlockForProcess`.
        self.instance
            .locked_formats()
            .map(|f| u32::from(f.input.channels()))
            .ok_or_else(|| {
                windows_core::Error::new(
                    HRESULT::from(HResult::APOERR_NOT_LOCKED),
                    "GetInputChannelCount called outside of the Locked state",
                )
            })
    }
}

/// Extract a [`Format`] from a host-supplied
/// [`APO_CONNECTION_DESCRIPTOR`].
///
/// # Safety
///
/// `descriptor` must point to a `APO_CONNECTION_DESCRIPTOR` whose
/// `pFormat` is a valid `IAudioMediaType` (the audio engine
/// guarantees this in `LockForProcess`). The returned `Format`
/// holds a deep copy of the fields; no references survive the
/// call.
unsafe fn format_from_descriptor(
    descriptor: *const APO_CONNECTION_DESCRIPTOR,
) -> windows_core::Result<Format> {
    if descriptor.is_null() {
        return Err(windows_core::Error::new(
            HRESULT::from(HResult::E_POINTER),
            "APO_CONNECTION_DESCRIPTOR pointer was null",
        ));
    }
    // Safety: caller guarantees the pointer is valid and the
    // signature stamp is set by the audio engine.
    let descriptor = unsafe { &*descriptor };
    if descriptor.u32Signature != CONNECTION_PROPERTY_SIGNATURE {
        return Err(windows_core::Error::new(
            HRESULT::from(HResult::APOERR_INVALID_INPUT_DATA),
            "APO_CONNECTION_DESCRIPTOR.u32Signature did not match 'APOC'",
        ));
    }
    let Some(media_type) = descriptor.pFormat.as_ref() else {
        return Err(windows_core::Error::new(
            HRESULT::from(HResult::APOERR_FORMAT_NOT_SUPPORTED),
            "APO_CONNECTION_DESCRIPTOR.pFormat was None",
        ));
    };
    // Safety: media_type is a valid IAudioMediaType handed to us
    // by the audio engine. GetAudioFormat returns an interior
    // pointer the engine owns; we copy fields out via
    // Format::from_waveformatex.
    let wf_ptr = unsafe { media_type.GetAudioFormat() };
    if wf_ptr.is_null() {
        return Err(windows_core::Error::new(
            HRESULT::from(HResult::APOERR_FORMAT_NOT_SUPPORTED),
            "IAudioMediaType::GetAudioFormat returned null",
        ));
    }
    // Safety: GetAudioFormat returns a pointer to a WAVEFORMATEX
    // owned by the audio engine for the duration of the LockForProcess
    // call; we read from it once and the Format wrapper copies the
    // fields out.
    Ok(Format::from_waveformatex(unsafe { &*wf_ptr }))
}

impl IAudioProcessingObjectConfiguration_Impl for ApoInstanceCom_Impl {
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn LockForProcess(
        &self,
        u32numinputconnections: u32,
        ppinputconnections: *const *const APO_CONNECTION_DESCRIPTOR,
        u32numoutputconnections: u32,
        ppoutputconnections: *const *const APO_CONNECTION_DESCRIPTOR,
    ) -> windows_core::Result<()> {
        // The framework currently supports SISO APOs only — one
        // input connection, one output connection — matching the
        // architecture-doc constraint.
        if u32numinputconnections != 1 || u32numoutputconnections != 1 {
            return Err(windows_core::Error::new(
                HRESULT::from(HResult::APOERR_NUM_CONNECTIONS_INVALID),
                "framework supports exactly one input and one output connection",
            ));
        }
        if ppinputconnections.is_null() || ppoutputconnections.is_null() {
            return Err(windows_core::Error::new(
                HRESULT::from(HResult::E_POINTER),
                "connection-descriptor array pointer was null",
            ));
        }
        // Safety: the audio engine guarantees the arrays hold the
        // declared number of valid `APO_CONNECTION_DESCRIPTOR*`
        // entries, and the count of 1 was checked above.
        let input_desc = unsafe { *ppinputconnections };
        let output_desc = unsafe { *ppoutputconnections };

        // Safety: extracted descriptors are valid per the engine's
        // contract; format_from_descriptor performs its own null /
        // signature checks.
        let input_format = unsafe { format_from_descriptor(input_desc) }?;
        let output_format = unsafe { format_from_descriptor(output_desc) }?;

        self.instance
            .lock_for_process(&input_format, &output_format)
            .map_err(|e| {
                windows_core::Error::new(
                    HRESULT::from(e),
                    "ProcessingObject::lock_for_process failed",
                )
            })
    }

    fn UnlockForProcess(&self) -> windows_core::Result<()> {
        self.instance.unlock_for_process().map_err(|e| {
            windows_core::Error::new(
                HRESULT::from(e),
                "ProcessingObject::unlock_for_process failed",
            )
        })
    }
}

impl IAudioProcessingObjectRT_Impl for ApoInstanceCom_Impl {
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn APOProcess(
        &self,
        u32numinputconnections: u32,
        ppinputconnections: *const *const APO_CONNECTION_PROPERTY,
        u32numoutputconnections: u32,
        ppoutputconnections: *mut *mut APO_CONNECTION_PROPERTY,
    ) {
        // Defensive: write SILENT output and bail without
        // panicking if any precondition fails. APOProcess returns
        // no HRESULT — there is nowhere to report errors — so the
        // sole graceful degradation is to emit silence.
        let mark_output_silent = |frame_count: u32| {
            if u32numoutputconnections == 1 && !ppoutputconnections.is_null() {
                // Safety: count is 1 and pointer is non-null.
                let out_ptr = unsafe { *ppoutputconnections };
                if !out_ptr.is_null() {
                    // Safety: COM caller's APO_CONNECTION_PROPERTY*
                    // points to a writable slot.
                    unsafe {
                        (*out_ptr).u32ValidFrameCount = frame_count;
                        (*out_ptr).u32BufferFlags =
                            windows::Win32::Media::Audio::Apo::BUFFER_SILENT;
                    }
                }
            }
        };

        if u32numinputconnections != 1 || u32numoutputconnections != 1 {
            mark_output_silent(0);
            return;
        }
        if ppinputconnections.is_null() || ppoutputconnections.is_null() {
            mark_output_silent(0);
            return;
        }

        // Safety: count == 1 and pointers are non-null per the
        // checks above. The audio engine guarantees each entry
        // points to a valid APO_CONNECTION_PROPERTY.
        let in_ptr = unsafe { *ppinputconnections };
        let out_ptr = unsafe { *ppoutputconnections };
        if in_ptr.is_null() || out_ptr.is_null() {
            mark_output_silent(0);
            return;
        }
        // Safety: same.
        let in_prop = unsafe { &*in_ptr };
        let out_prop = unsafe { &mut *out_ptr };

        if in_prop.u32Signature != CONNECTION_PROPERTY_SIGNATURE
            || out_prop.u32Signature != CONNECTION_PROPERTY_SIGNATURE
        {
            mark_output_silent(0);
            return;
        }

        let Some(formats) = self.instance.locked_formats() else {
            mark_output_silent(0);
            return;
        };
        let channels = formats.input.channels() as usize;
        if channels == 0 || !formats.input.is_float() || formats.input.bits_per_sample() != 32 {
            // The framework's default ProcessingObject negotiation
            // only ever accepts pcm_float32; refuse anything else.
            mark_output_silent(0);
            return;
        }
        let frames = in_prop.u32ValidFrameCount as usize;
        let sample_count = match frames.checked_mul(channels) {
            Some(n) => n,
            None => {
                mark_output_silent(0);
                return;
            }
        };

        // Safety: the host guarantees the buffers hold at least
        // u32ValidFrameCount × channels float32 samples. We cap
        // by `frames` (which the host validated) and treat the
        // slices as Rust references for the duration of the
        // dispatch.
        let input_slice =
            unsafe { core::slice::from_raw_parts(in_prop.pBuffer as *const f32, sample_count) };
        let output_slice =
            unsafe { core::slice::from_raw_parts_mut(out_prop.pBuffer as *mut f32, sample_count) };

        let in_flags: BufferFlags = in_prop.u32BufferFlags.into();
        // Safety: we are on the audio engine's realtime thread —
        // APOProcess only runs after LockForProcess set state to
        // Locked, and the framework gates allocator use through
        // the `RealtimeContext` parameter.
        let rt = unsafe { RealtimeContext::new_unchecked() };

        let out_flags =
            match self
                .instance
                .process(&rt, ProcessInput::new(input_slice, in_flags), output_slice)
            {
                Ok(f) => f,
                Err(_) => {
                    mark_output_silent(in_prop.u32ValidFrameCount);
                    return;
                }
            };

        out_prop.u32ValidFrameCount = in_prop.u32ValidFrameCount;
        out_prop.u32BufferFlags = out_flags.into();
    }

    fn CalcInputFrames(&self, u32outputframecount: u32) -> u32 {
        // No resampling: one input frame yields one output frame.
        u32outputframecount
    }

    fn CalcOutputFrames(&self, u32inputframecount: u32) -> u32 {
        // No resampling: one input frame yields one output frame.
        u32inputframecount
    }
}

// IAudioSystemEffects (v1) is an empty marker interface. Listing it
// on the `#[implement(...)]` annotation makes `QueryInterface` for
// `IID_IAudioSystemEffects` resolve through the COM bridge; the
// trait carries no methods of its own.
impl IAudioSystemEffects_Impl for ApoInstanceCom_Impl {}

impl IAudioSystemEffects2_Impl for ApoInstanceCom_Impl {
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn GetEffectsList(
        &self,
        ppeffectsids: *mut *mut GUID,
        pceffects: *mut u32,
        _event: HANDLE,
    ) -> windows_core::Result<()> {
        // The Windows audio engine takes ownership of `*ppeffectsids`
        // (a `CoTaskMemAlloc`-backed buffer of GUIDs); the `event`
        // handle is one the APO can `SetEvent` on to indicate the
        // effect list has changed. The framework's current
        // `system_effects` snapshot is static between
        // `LockForProcess` cycles, so we ignore the event.
        if ppeffectsids.is_null() || pceffects.is_null() {
            return Err(windows_core::Error::new(
                HRESULT::from(HResult::E_POINTER),
                "GetEffectsList output pointers were null",
            ));
        }

        let effects = self.instance.system_effects();
        let count = effects.len();

        // Zero-effect case: write null pointer + zero count, return
        // S_OK. The audio engine treats this as "the APO has no
        // controllable effects".
        if count == 0 {
            // Safety: pointers were null-checked above.
            unsafe {
                *ppeffectsids = core::ptr::null_mut();
                *pceffects = 0;
            }
            return Ok(());
        }

        // Allocate one GUID per effect via CoTaskMemAlloc so the
        // audio engine can release it with CoTaskMemFree.
        let total_bytes = count
            .checked_mul(core::mem::size_of::<GUID>())
            .ok_or_else(|| {
                windows_core::Error::new(
                    HRESULT::from(HResult::E_OUTOFMEMORY),
                    "GetEffectsList size calculation overflowed",
                )
            })?;
        // Safety: combase.dll CoTaskMemAlloc returns null on OOM.
        let raw = unsafe { windows::Win32::System::Com::CoTaskMemAlloc(total_bytes) };
        if raw.is_null() {
            return Err(windows_core::Error::new(
                HRESULT::from(HResult::E_OUTOFMEMORY),
                "CoTaskMemAlloc returned null for GetEffectsList",
            ));
        }
        let list = raw.cast::<GUID>();
        // Safety: `list` points to `count` × size_of::<GUID>() bytes
        // of writable memory we just allocated; iteration is bounded
        // by `count`.
        unsafe {
            for (i, effect) in effects.iter().enumerate() {
                core::ptr::write(list.add(i), effect.id.into());
            }
            *ppeffectsids = list;
            *pceffects = count as u32;
        }
        Ok(())
    }
}

impl IAudioSystemEffects3_Impl for ApoInstanceCom_Impl {
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn GetControllableSystemEffectsList(
        &self,
        effects: *mut *mut AUDIO_SYSTEMEFFECT,
        numeffects: *mut u32,
        _event: HANDLE,
    ) -> windows_core::Result<()> {
        if effects.is_null() || numeffects.is_null() {
            return Err(windows_core::Error::new(
                HRESULT::from(HResult::E_POINTER),
                "GetControllableSystemEffectsList output pointers were null",
            ));
        }

        let user_effects = self.instance.system_effects();
        let count = user_effects.len();

        if count == 0 {
            // Safety: pointers were null-checked above.
            unsafe {
                *effects = core::ptr::null_mut();
                *numeffects = 0;
            }
            return Ok(());
        }

        let total_bytes = count
            .checked_mul(core::mem::size_of::<AUDIO_SYSTEMEFFECT>())
            .ok_or_else(|| {
                windows_core::Error::new(
                    HRESULT::from(HResult::E_OUTOFMEMORY),
                    "GetControllableSystemEffectsList size calculation overflowed",
                )
            })?;
        // Safety: combase.dll CoTaskMemAlloc returns null on OOM.
        let raw = unsafe { windows::Win32::System::Com::CoTaskMemAlloc(total_bytes) };
        if raw.is_null() {
            return Err(windows_core::Error::new(
                HRESULT::from(HResult::E_OUTOFMEMORY),
                "CoTaskMemAlloc returned null for GetControllableSystemEffectsList",
            ));
        }
        let list = raw.cast::<AUDIO_SYSTEMEFFECT>();
        // Safety: `list` points to `count` × size_of::<AUDIO_SYSTEMEFFECT>()
        // bytes; iteration is bounded by `count`.
        unsafe {
            for (i, eff) in user_effects.iter().enumerate() {
                core::ptr::write(
                    list.add(i),
                    AUDIO_SYSTEMEFFECT {
                        id: eff.id.into(),
                        canSetState: BOOL::from(eff.controllable),
                        state: match eff.state {
                            SystemEffectState::Off => AUDIO_SYSTEMEFFECT_STATE_OFF,
                            SystemEffectState::On => AUDIO_SYSTEMEFFECT_STATE_ON,
                        },
                    },
                );
            }
            *effects = list;
            *numeffects = count as u32;
        }
        Ok(())
    }

    fn SetAudioSystemEffectState(
        &self,
        effectid: &GUID,
        state: AUDIO_SYSTEMEFFECT_STATE,
    ) -> windows_core::Result<()> {
        // Reject IDs the APO never advertised — the audio engine
        // is not supposed to drive us into states for unknown
        // effects, and accepting them silently would hide a bug.
        let requested = crate::clsid::Clsid::from(*effectid);
        if !self
            .instance
            .system_effects()
            .iter()
            .any(|e| e.id == requested)
        {
            return Err(windows_core::Error::new(
                HRESULT::from(HResult::E_INVALIDARG),
                "SetAudioSystemEffectState: unknown effect id",
            ));
        }
        let cross = match state {
            AUDIO_SYSTEMEFFECT_STATE_OFF => SystemEffectState::Off,
            AUDIO_SYSTEMEFFECT_STATE_ON => SystemEffectState::On,
            other => {
                return Err(windows_core::Error::new(
                    HRESULT::from(HResult::E_INVALIDARG),
                    alloc::format!("SetAudioSystemEffectState: unknown state value {}", other.0),
                ));
            }
        };
        self.instance.set_system_effect_state(&requested, cross);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apo::{ApoCategory, ProcessInput, ProcessingObject};
    use crate::buffer::BufferFlags;
    use crate::clsid::Clsid;
    use crate::instance::ApoInstance;
    use crate::realtime::{RealtimeContext, State};

    struct Dummy;
    impl ProcessingObject for Dummy {
        const CLSID: Clsid = Clsid::from_u128(0xFEDCBA98_7654_3210_FEDC_BA9876543210);
        const NAME: &'static str = "dummy";
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

    fn new_com() -> ApoInstanceCom {
        ApoInstanceCom::new(Arc::new(ApoInstance::<Dummy>::new()))
    }

    #[test]
    fn new_holds_a_zero_refcount_instance() {
        let com = new_com();
        assert_eq!(com.instance().refcount(), 0);
        assert_eq!(com.instance().state(), State::Uninitialized);
    }

    #[test]
    fn instance_accessor_round_trips() {
        let com = new_com();
        let arc1 = com.instance();
        let arc2 = com.instance();
        // Both references address the same Arc.
        assert!(Arc::ptr_eq(arc1, arc2));
    }

    /// IAudioProcessingObject::IsInputFormatSupported routes through
    /// the COM vtable, into AnyApoInstance, and back out as a fresh
    /// IAudioMediaType. The end-to-end vtable hop is what we exercise
    /// here — the bridge's `negotiate_format` helper is covered by
    /// `crate::raw::media_type::tests` in isolation.
    #[test]
    fn is_input_format_supported_routes_float32_as_accept() {
        use crate::format::Format;
        use crate::raw::media_type::media_type_from_format;
        use windows::Win32::Media::Audio::Apo::IAudioProcessingObject;
        use windows_core::ComObject;

        let apo: IAudioProcessingObject = ComObject::new(new_com()).into_interface();
        let requested = media_type_from_format(&Format::pcm_float32(48_000, 1));
        // Safety: live IAudioProcessingObject vtable; opposite-format
        // pointer is allowed to be null per the audio engine
        // contract.
        let answered = unsafe { apo.IsInputFormatSupported(None, &requested) }.unwrap();
        // Safety: `answered` is live for the borrow.
        let wf = unsafe { &*answered.GetAudioFormat() };
        assert_eq!(
            Format::from_waveformatex(wf),
            Format::pcm_float32(48_000, 1)
        );
    }

    #[test]
    fn is_input_format_supported_suggests_float32_for_int16() {
        use crate::format::{Format, WAVE_FORMAT_IEEE_FLOAT};
        use crate::raw::media_type::media_type_from_format;
        use windows::Win32::Media::Audio::Apo::IAudioProcessingObject;
        use windows_core::ComObject;

        let apo: IAudioProcessingObject = ComObject::new(new_com()).into_interface();
        let requested = media_type_from_format(&Format::pcm_int16(48_000, 1));
        // Safety: live IAudioProcessingObject vtable.
        let answered = unsafe { apo.IsInputFormatSupported(None, &requested) }.unwrap();
        // Safety: `answered` is live for the borrow.
        let wf = unsafe { &*answered.GetAudioFormat() };
        let suggested = Format::from_waveformatex(wf);
        assert_eq!(suggested.format_tag(), WAVE_FORMAT_IEEE_FLOAT);
        assert_eq!(suggested.bits_per_sample(), 32);
        assert_eq!(suggested.sample_rate(), 48_000);
        assert_eq!(suggested.channels(), 1);
    }

    #[test]
    fn get_input_channel_count_errors_before_lock() {
        use windows::Win32::Media::Audio::Apo::IAudioProcessingObject;
        use windows_core::ComObject;

        let apo: IAudioProcessingObject = ComObject::new(new_com()).into_interface();
        // Safety: live IAudioProcessingObject vtable.
        let err = unsafe { apo.GetInputChannelCount() }.unwrap_err();
        assert_eq!(err.code(), HRESULT::from(HResult::APOERR_NOT_LOCKED));
    }

    #[test]
    fn get_input_channel_count_reports_locked_channel_count() {
        use crate::format::Format;

        let com = new_com();
        com.instance().initialize().unwrap();
        // 6-channel float32 input, mono output — exercises that the
        // method reads the input side specifically.
        com.instance()
            .lock_for_process(
                &Format::pcm_float32(48_000, 6),
                &Format::pcm_float32(48_000, 1),
            )
            .unwrap();
        // Direct call into the impl method via the macro-generated
        // _Impl Deref. Going through the vtable would also work,
        // but we already cover that hop in
        // `is_input_format_supported_routes_float32_as_accept`.
        use windows::Win32::Media::Audio::Apo::IAudioProcessingObject;
        use windows_core::ComObject;
        let apo: IAudioProcessingObject = ComObject::new(com).into_interface();
        // Safety: live IAudioProcessingObject vtable.
        let count = unsafe { apo.GetInputChannelCount() }.unwrap();
        assert_eq!(count, 6);
    }

    #[test]
    fn get_registration_properties_routes_through_builder() {
        use windows::Win32::Media::Audio::Apo::IAudioProcessingObject;
        use windows::Win32::System::Com::CoTaskMemFree;
        use windows_core::ComObject;

        let apo: IAudioProcessingObject = ComObject::new(new_com()).into_interface();
        // Safety: live IAudioProcessingObject vtable.
        let props = unsafe { apo.GetRegistrationProperties() }.unwrap();
        assert!(!props.is_null());
        // Safety: builder-produced live pointer; we read the CLSID
        // back through it before handing it to CoTaskMemFree.
        unsafe {
            assert_eq!(
                crate::clsid::Clsid::from((*props).clsid),
                <Dummy as ProcessingObject>::CLSID
            );
            CoTaskMemFree(Some(props.cast()));
        }
    }

    /// A `ProcessingObject` whose `Dummy` default produces an empty
    /// system-effect list. `GetEffectsList` should write a null
    /// pointer + zero count and return S_OK.
    #[test]
    fn get_effects_list_reports_no_effects_for_default_dummy() {
        use windows::Win32::Media::Audio::Apo::IAudioSystemEffects2;
        use windows_core::{ComObject, GUID};

        let api: IAudioSystemEffects2 = ComObject::new(new_com()).into_interface();
        let mut list: *mut GUID = 0xDEAD_BEEF as *mut GUID;
        let mut count: u32 = 0xDEAD_BEEF;
        // Safety: live IAudioSystemEffects2 vtable; the event
        // handle is a null `HANDLE` because the framework's
        // current implementation ignores it.
        unsafe {
            api.GetEffectsList(&mut list, &mut count, Default::default())
                .expect("GetEffectsList failed");
        }
        assert!(list.is_null(), "empty list expected; got {list:p}");
        assert_eq!(count, 0);
        // No CoTaskMemFree needed when the list is null. Dropping
        // the IAudioSystemEffects2 here releases the underlying
        // ApoInstanceCom via the COM refcount.
        drop(api);
    }

    /// A `ProcessingObject` with two advertised system effects.
    /// `GetEffectsList` should hand back a CoTaskMemAlloc'd GUID
    /// list of length 2 whose entries match the user's declaration.
    #[test]
    fn get_effects_list_reports_advertised_effects() {
        use crate::apo::SystemEffect;
        use crate::instance::ApoInstance;
        use windows::Win32::Media::Audio::Apo::IAudioSystemEffects2;
        use windows::Win32::System::Com::CoTaskMemFree;
        use windows_core::{ComObject, GUID};

        const FX_A: Clsid = Clsid::from_u128(0xAAAAAAAA_BBBB_CCCC_DDDD_EEEEEEEEEEEE);
        const FX_B: Clsid = Clsid::from_u128(0x11111111_2222_3333_4444_555555555555);

        struct TwoEffects;
        impl ProcessingObject for TwoEffects {
            const CLSID: Clsid = Clsid::from_u128(0x12121212_3434_5656_7878_9A9A9A9A9A9A);
            const NAME: &'static str = "two-effects";
            const COPYRIGHT: &'static str = "test";
            const CATEGORY: ApoCategory = ApoCategory::Sfx;
            fn new() -> Self {
                Self
            }
            fn system_effects(&self) -> &[SystemEffect] {
                const E: [SystemEffect; 2] = [SystemEffect::new(FX_A), SystemEffect::new(FX_B)];
                &E
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

        let com = ApoInstanceCom::new(Arc::new(ApoInstance::<TwoEffects>::new()));
        let api: IAudioSystemEffects2 = ComObject::new(com).into_interface();
        let mut list: *mut GUID = core::ptr::null_mut();
        let mut count: u32 = 0;
        // Safety: live IAudioSystemEffects2 vtable.
        unsafe {
            api.GetEffectsList(&mut list, &mut count, Default::default())
                .expect("GetEffectsList failed");
        }
        assert_eq!(count, 2);
        assert!(!list.is_null());
        // Safety: list is a CoTaskMemAlloc'd buffer of `count`
        // GUIDs we just received; read both entries, then release.
        unsafe {
            let a = *list;
            let b = *list.add(1);
            assert_eq!(Clsid::from(a), FX_A);
            assert_eq!(Clsid::from(b), FX_B);
            CoTaskMemFree(Some(list.cast()));
        }
    }

    /// IAudioSystemEffects3::GetControllableSystemEffectsList returns
    /// the same effect list as v2's GetEffectsList but with
    /// per-effect controllable / state fields populated from the
    /// user APO's declaration.
    #[test]
    fn get_controllable_system_effects_list_reports_user_state() {
        use crate::apo::{SystemEffect, SystemEffectState};
        use crate::instance::ApoInstance;
        use windows::Win32::Media::Audio::Apo::{
            IAudioSystemEffects3, AUDIO_SYSTEMEFFECT, AUDIO_SYSTEMEFFECT_STATE_OFF,
            AUDIO_SYSTEMEFFECT_STATE_ON,
        };
        use windows::Win32::System::Com::CoTaskMemFree;
        use windows_core::ComObject;

        const FX_A: Clsid = Clsid::from_u128(0xBBBBBBBB_CCCC_DDDD_EEEE_FFFFFFFFFFFF);
        const FX_B: Clsid = Clsid::from_u128(0x22222222_3333_4444_5555_666666666666);

        struct ControllableEffects;
        impl ProcessingObject for ControllableEffects {
            const CLSID: Clsid = Clsid::from_u128(0x34343434_5656_7878_9A9A_BCBCBCBCBCBC);
            const NAME: &'static str = "controllable";
            const COPYRIGHT: &'static str = "test";
            const CATEGORY: ApoCategory = ApoCategory::Sfx;
            fn new() -> Self {
                Self
            }
            fn system_effects(&self) -> &[SystemEffect] {
                const E: [SystemEffect; 2] = [
                    SystemEffect::new(FX_A)
                        .with_controllable(true)
                        .with_state(SystemEffectState::On),
                    SystemEffect::new(FX_B)
                        .with_controllable(false)
                        .with_state(SystemEffectState::Off),
                ];
                &E
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

        let api: IAudioSystemEffects3 =
            ComObject::new(ApoInstanceCom::new(Arc::new(ApoInstance::<
                ControllableEffects,
            >::new())))
            .into_interface();
        let mut list: *mut AUDIO_SYSTEMEFFECT = core::ptr::null_mut();
        let mut count: u32 = 0;
        // Safety: live IAudioSystemEffects3 vtable.
        unsafe {
            api.GetControllableSystemEffectsList(&mut list, &mut count, None)
                .expect("GetControllableSystemEffectsList failed");
        }
        assert_eq!(count, 2);
        assert!(!list.is_null());
        // Safety: list is a CoTaskMemAlloc'd buffer of `count`
        // AUDIO_SYSTEMEFFECTs; read both entries, then release.
        unsafe {
            let a = *list;
            let b = *list.add(1);
            assert_eq!(Clsid::from(a.id), FX_A);
            assert!(a.canSetState.as_bool());
            assert_eq!(a.state, AUDIO_SYSTEMEFFECT_STATE_ON);
            assert_eq!(Clsid::from(b.id), FX_B);
            assert!(!b.canSetState.as_bool());
            assert_eq!(b.state, AUDIO_SYSTEMEFFECT_STATE_OFF);
            CoTaskMemFree(Some(list.cast()));
        }
    }

    /// IAudioSystemEffects3::SetAudioSystemEffectState dispatches
    /// into the user's `set_system_effect_state` override. Unknown
    /// IDs yield E_INVALIDARG without touching user state.
    #[test]
    fn set_audio_system_effect_state_dispatches_and_rejects_unknown_ids() {
        use crate::apo::{SystemEffect, SystemEffectState};
        use crate::instance::ApoInstance;
        use core::cell::Cell;
        use windows::Win32::Media::Audio::Apo::{
            IAudioSystemEffects3, AUDIO_SYSTEMEFFECT_STATE_OFF, AUDIO_SYSTEMEFFECT_STATE_ON,
        };
        use windows_core::{ComObject, GUID};

        const FX: Clsid = Clsid::from_u128(0xABCDABCD_1234_5678_9ABC_DEF012345678);

        struct Toggleable {
            last: Cell<Option<(Clsid, SystemEffectState)>>,
        }
        impl ProcessingObject for Toggleable {
            const CLSID: Clsid = Clsid::from_u128(0x55555555_6666_7777_8888_999999999999);
            const NAME: &'static str = "toggleable";
            const COPYRIGHT: &'static str = "test";
            const CATEGORY: ApoCategory = ApoCategory::Sfx;
            fn new() -> Self {
                Self {
                    last: Cell::new(None),
                }
            }
            fn system_effects(&self) -> &[SystemEffect] {
                const E: [SystemEffect; 1] = [SystemEffect::new(FX).with_controllable(true)];
                &E
            }
            fn set_system_effect_state(&mut self, id: &Clsid, state: SystemEffectState) {
                self.last.set(Some((*id, state)));
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

        let inst = Arc::new(ApoInstance::<Toggleable>::new());
        let api: IAudioSystemEffects3 =
            ComObject::new(ApoInstanceCom::new(inst.clone())).into_interface();

        // Known effect → succeeds and forwards to the user.
        let id: GUID = FX.into();
        // Safety: live IAudioSystemEffects3 vtable.
        unsafe { api.SetAudioSystemEffectState(id, AUDIO_SYSTEMEFFECT_STATE_OFF) }
            .expect("SetAudioSystemEffectState(known, Off) failed");

        // Unknown effect → E_INVALIDARG; user state untouched.
        let unknown: GUID = Clsid::from_u128(0xDEAD_DEAD_DEAD_DEAD_DEAD_DEAD_DEAD_DEAD).into();
        let err = unsafe { api.SetAudioSystemEffectState(unknown, AUDIO_SYSTEMEFFECT_STATE_ON) }
            .expect_err("SetAudioSystemEffectState(unknown) should fail");
        assert_eq!(err.code(), HRESULT::from(HResult::E_INVALIDARG));
    }
}
