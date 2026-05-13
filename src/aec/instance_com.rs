//! COM wrapper bridging [`AnyAecApoInstance`] to the Windows AEC
//! APO interface family.
//!
//! [`AecApoInstanceCom`] is the AEC counterpart of
//! `crate::raw::instance_com::ApoInstanceCom`:
//! it carries the same six SISO interfaces
//! (`IAudioProcessingObject` family + `IAudioSystemEffects`
//! v1/v2/v3) plus the three AEC-specific interfaces
//! (`IApoAcousticEchoCancellation`,
//! `IApoAuxiliaryInputConfiguration`, `IApoAuxiliaryInputRT`).
//!
//! The SISO method bodies are copy-pasted from
//! `crate::raw::instance_com::ApoInstanceCom` rather than
//! refactored through shared free functions; a follow-up PR can
//! collapse the two carriers if the maintenance cost warrants it.

// The `windows_core::implement` proc-macro generates a sibling
// `*_Impl` struct without doc-comments; the crate-wide
// `#![deny(missing_docs)]` would otherwise reject the expansion.
#![allow(missing_docs)]

extern crate alloc;

use alloc::sync::Arc;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Media::Audio::Apo::{
    IApoAcousticEchoCancellation, IApoAcousticEchoCancellation_Impl,
    IApoAuxiliaryInputConfiguration, IApoAuxiliaryInputConfiguration_Impl, IApoAuxiliaryInputRT,
    IApoAuxiliaryInputRT_Impl, IAudioMediaType, IAudioProcessingObject,
    IAudioProcessingObjectConfiguration, IAudioProcessingObjectConfiguration_Impl,
    IAudioProcessingObjectRT, IAudioProcessingObjectRT_Impl, IAudioProcessingObject_Impl,
    IAudioSystemEffects, IAudioSystemEffects2, IAudioSystemEffects2_Impl, IAudioSystemEffects3,
    IAudioSystemEffects3_Impl, IAudioSystemEffects_Impl, APO_CONNECTION_DESCRIPTOR,
    APO_CONNECTION_PROPERTY, APO_REG_PROPERTIES, AUDIO_SYSTEMEFFECT, AUDIO_SYSTEMEFFECT_STATE,
    AUDIO_SYSTEMEFFECT_STATE_OFF, AUDIO_SYSTEMEFFECT_STATE_ON,
};
use windows_core::{implement, Ref, BOOL, GUID, HRESULT};

use crate::aec::{AnyAecApoInstance, AuxiliaryInputBuffer};
use crate::apo::{ProcessInput, SystemEffectState};
use crate::buffer::{BufferFlags, CONNECTION_PROPERTY_SIGNATURE};
use crate::error::HResult;
use crate::format::Format;
use crate::realtime::RealtimeContext;

/// COM-side carrier for an `Arc<dyn AnyAecApoInstance>`.
///
/// One of these is materialised per `IClassFactory::CreateInstance`
/// call for an AEC APO. The carrier is what the audio engine sees
/// as an `IApoAcousticEchoCancellation*`; methods on the COM
/// interfaces route through this struct into the user's
/// [`crate::aec::AecProcessingObject`] via the type-erased trait.
#[implement(
    IAudioProcessingObject,
    IAudioProcessingObjectConfiguration,
    IAudioProcessingObjectRT,
    IAudioSystemEffects,
    IAudioSystemEffects2,
    IAudioSystemEffects3,
    IApoAcousticEchoCancellation,
    IApoAuxiliaryInputConfiguration,
    IApoAuxiliaryInputRT
)]
pub struct AecApoInstanceCom {
    instance: Arc<dyn AnyAecApoInstance>,
}

impl AecApoInstanceCom {
    /// Wrap an existing AEC instance for COM exposure.
    #[must_use]
    pub fn new(instance: Arc<dyn AnyAecApoInstance>) -> Self {
        crate::raw::exports::outstanding_inc();
        Self { instance }
    }

    /// Borrow the underlying `AnyAecApoInstance`.
    #[must_use]
    pub fn instance(&self) -> &Arc<dyn AnyAecApoInstance> {
        &self.instance
    }
}

impl Drop for AecApoInstanceCom {
    fn drop(&mut self) {
        // Symmetric counterpart of the `outstanding_inc` in `new`.
        // Fires when the COM refcount on the wrapping `ComObject`
        // reaches zero and the box is freed.
        crate::raw::exports::outstanding_dec();
    }
}

impl IAudioProcessingObject_Impl for AecApoInstanceCom_Impl {
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
        // AEC variant advertises nine interfaces (six SISO + three
        // AEC). The audio engine takes ownership of the returned
        // buffer and releases it with CoTaskMemFree.
        crate::aec::exports::build_aec_registration_properties(self.instance.as_any_apo_instance())
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
            self.instance.as_any_apo_instance(),
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
            self.instance.as_any_apo_instance(),
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
    // owned by the audio engine for the duration of the
    // LockForProcess call. `from_waveformatex_ptr` examines the
    // `cbSize` / `wFormatTag` markers to pick WAVEFORMATEX vs
    // WAVEFORMATEXTENSIBLE and copies the fields out.
    Ok(unsafe { Format::from_waveformatex_ptr(wf_ptr) })
}

impl IAudioProcessingObjectConfiguration_Impl for AecApoInstanceCom_Impl {
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

impl IAudioProcessingObjectRT_Impl for AecApoInstanceCom_Impl {
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
impl IAudioSystemEffects_Impl for AecApoInstanceCom_Impl {}

impl IAudioSystemEffects2_Impl for AecApoInstanceCom_Impl {
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

impl IAudioSystemEffects3_Impl for AecApoInstanceCom_Impl {
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

// IApoAcousticEchoCancellation (v1) is an empty marker interface;
// the engine consults it to opt the APO into the AEC slot but the
// interface itself carries no methods. Listing it on
// `#[implement(...)]` is what makes `QueryInterface` for
// `IID_IApoAcousticEchoCancellation` resolve through this struct.
impl IApoAcousticEchoCancellation_Impl for AecApoInstanceCom_Impl {}

impl IApoAuxiliaryInputConfiguration_Impl for AecApoInstanceCom_Impl {
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn AddAuxiliaryInput(
        &self,
        dwinputid: u32,
        cbdatasize: u32,
        pbydata: *const u8,
        pinputconnection: *const APO_CONNECTION_DESCRIPTOR,
    ) -> windows_core::Result<()> {
        // Safety: the audio engine guarantees pinputconnection is a
        // valid APO_CONNECTION_DESCRIPTOR for the duration of the
        // call; format_from_descriptor performs its own null /
        // signature checks.
        let format = unsafe { format_from_descriptor(pinputconnection) }?;
        let init_data: &[u8] = if cbdatasize == 0 || pbydata.is_null() {
            &[]
        } else {
            // Safety: caller guarantees pbydata is valid for at
            // least `cbdatasize` bytes.
            unsafe { core::slice::from_raw_parts(pbydata, cbdatasize as usize) }
        };
        self.instance
            .add_aux_input(dwinputid, &format, init_data)
            .map_err(|e| {
                windows_core::Error::new(
                    HRESULT::from(e),
                    "AecProcessingObject::add_aux_input failed",
                )
            })
    }

    fn RemoveAuxiliaryInput(&self, dwinputid: u32) -> windows_core::Result<()> {
        self.instance.remove_aux_input(dwinputid);
        Ok(())
    }

    fn IsInputFormatSupported(
        &self,
        prequestedinputformat: Ref<IAudioMediaType>,
    ) -> windows_core::Result<IAudioMediaType> {
        // Auxiliary-input format negotiation routes through the
        // user's `is_aux_format_supported`. We replicate the bridge
        // helper's structure inline so the call lands on
        // `is_aux_format_supported` rather than the primary-input
        // hook.
        let requested_format =
            crate::raw::media_type::format_from_media_type(prequestedinputformat)?;
        let decision = self.instance.is_aux_format_supported(&requested_format);
        match decision {
            crate::format::FormatNegotiation::Accept => Ok(
                crate::raw::media_type::media_type_from_format(&requested_format),
            ),
            crate::format::FormatNegotiation::Suggest(alt) => {
                Ok(crate::raw::media_type::media_type_from_format(&alt))
            }
            crate::format::FormatNegotiation::Reject => Err(windows_core::Error::new(
                HRESULT::from(HResult::APOERR_FORMAT_NOT_SUPPORTED),
                "AecProcessingObject rejected the requested aux format",
            )),
        }
    }
}

impl IApoAuxiliaryInputRT_Impl for AecApoInstanceCom_Impl {
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn AcceptInput(&self, dwinputid: u32, pinputconnection: *const APO_CONNECTION_PROPERTY) {
        // AcceptInput has no HRESULT return — there is nowhere to
        // report errors. Bail without panicking on any
        // precondition failure.
        if pinputconnection.is_null() {
            return;
        }
        // Safety: caller guarantees the property struct is valid
        // for the duration of the call.
        let prop = unsafe { &*pinputconnection };
        if prop.u32Signature != CONNECTION_PROPERTY_SIGNATURE {
            return;
        }
        // Use the primary input's locked format to infer the
        // channel count and verify the float32 sample width. A
        // future revision should track per-aux-input formats so
        // streams that differ from the primary (e.g. stereo
        // loopback against a mono mic) can be served correctly.
        let Some(formats) = self.instance.locked_formats() else {
            return;
        };
        let channels = formats.input.channels() as usize;
        if channels == 0 || !formats.input.is_float() || formats.input.bits_per_sample() != 32 {
            return;
        }
        let frames = prop.u32ValidFrameCount as usize;
        let Some(sample_count) = frames.checked_mul(channels) else {
            return;
        };
        // Safety: the host guarantees the buffer holds at least
        // `u32ValidFrameCount × channels` float32 samples.
        let samples =
            unsafe { core::slice::from_raw_parts(prop.pBuffer as *const f32, sample_count) };
        let flags: BufferFlags = prop.u32BufferFlags.into();
        // Safety: AcceptInput runs on the audio engine's realtime
        // thread, same as APOProcess.
        let rt = unsafe { RealtimeContext::new_unchecked() };
        self.instance.accept_aux_input(
            &rt,
            AuxiliaryInputBuffer {
                id: dwinputid,
                samples,
                flags,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aec::{AecApoInstance, AecProcessingObject};
    use crate::apo::{ApoCategory, ProcessingObject};
    use crate::clsid::Clsid;
    use core::cell::Cell;
    use windows_core::ComObject;

    struct AecDummy {
        aux_added: Cell<Option<u32>>,
    }
    impl ProcessingObject for AecDummy {
        const CLSID: Clsid = Clsid::from_u128(0xDDCCBBAA_FFEE_4433_2211_77665544DDEE);
        const NAME: &'static str = "aec dummy";
        const COPYRIGHT: &'static str = "test";
        const CATEGORY: ApoCategory = ApoCategory::Mfx;
        fn new() -> Self {
            Self {
                aux_added: Cell::new(None),
            }
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
    impl AecProcessingObject for AecDummy {
        fn add_aux_input(
            &mut self,
            id: u32,
            _format: &Format,
            _init_data: &[u8],
        ) -> Result<(), HResult> {
            self.aux_added.set(Some(id));
            Ok(())
        }
    }

    fn new_aec_com() -> AecApoInstanceCom {
        AecApoInstanceCom::new(Arc::new(AecApoInstance::<AecDummy>::new()))
    }

    #[test]
    fn new_aec_com_starts_with_zero_refcount() {
        let com = new_aec_com();
        assert_eq!(com.instance().refcount(), 0);
        assert_eq!(
            com.instance().state(),
            crate::realtime::State::Uninitialized
        );
    }

    /// Confirm QueryInterface for IApoAcousticEchoCancellation
    /// resolves through this object. The interface has no methods,
    /// so the cast itself is the verification.
    #[test]
    fn iapoacoustic_echo_cancellation_resolves_via_cast() {
        use windows::Win32::Media::Audio::Apo::IApoAcousticEchoCancellation;
        use windows_core::Interface;
        let apo: IApoAcousticEchoCancellation = ComObject::new(new_aec_com()).into_interface();
        let _unk: windows_core::IUnknown = apo.cast().unwrap();
    }

    #[test]
    fn add_remove_auxiliary_input_dispatches_to_user() {
        use crate::format::Format;
        use crate::raw::media_type::media_type_from_format;
        use core::mem::ManuallyDrop;
        use windows::Win32::Media::Audio::Apo::{
            IApoAuxiliaryInputConfiguration, APO_CONNECTION_BUFFER_TYPE_ALLOCATED,
        };

        let cfg: IApoAuxiliaryInputConfiguration = ComObject::new(new_aec_com()).into_interface();

        let format = Format::pcm_float32(48_000, 1);
        let media = media_type_from_format(&format);
        let desc = APO_CONNECTION_DESCRIPTOR {
            Type: APO_CONNECTION_BUFFER_TYPE_ALLOCATED,
            pBuffer: 0,
            u32MaxFrameCount: 256,
            pFormat: ManuallyDrop::new(Some(media)),
            u32Signature: CONNECTION_PROPERTY_SIGNATURE,
        };
        // Safety: descriptor is a stack local that lives through
        // the call.
        unsafe { cfg.AddAuxiliaryInput(42, &[1, 2, 3], &desc) }.expect("AddAuxiliaryInput failed");
        // Safety: live IApoAuxiliaryInputConfiguration.
        unsafe { cfg.RemoveAuxiliaryInput(42) }.expect("RemoveAuxiliaryInput failed");
    }
}
