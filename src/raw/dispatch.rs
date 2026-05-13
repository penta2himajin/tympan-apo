//! Shared dispatch helpers for the COM-side method bodies that
//! both [`ApoInstanceCom`](crate::raw::instance_com::ApoInstanceCom)
//! and [`AecApoInstanceCom`](crate::aec::instance_com::AecApoInstanceCom)
//! delegate into.
//!
//! Why a separate module: the `windows_core::implement` proc-macro
//! generates a per-struct `*_Impl` and binds trait impls to that
//! specific `_Impl`. The two carriers therefore each need their
//! own `impl IAudioProcessingObject_Impl for *_Impl { ... }` block,
//! but the **method bodies** are identical (both end up dispatching
//! through `&dyn AnyApoInstance`). Hoisting those bodies into free
//! functions here keeps the per-carrier impl blocks down to
//! one-line delegates.
//!
//! The functions take `&dyn AnyApoInstance` rather than a generic
//! `&I` so the AEC carrier can pass its `&dyn AnyAecApoInstance`
//! upcast via `AnyAecApoInstance::as_any_apo_instance`.

extern crate alloc;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Media::Audio::Apo::{
    IAudioMediaType, APO_CONNECTION_DESCRIPTOR, APO_CONNECTION_PROPERTY, APO_REG_PROPERTIES,
    AUDIO_SYSTEMEFFECT, AUDIO_SYSTEMEFFECT_STATE, AUDIO_SYSTEMEFFECT_STATE_OFF,
    AUDIO_SYSTEMEFFECT_STATE_ON, BUFFER_SILENT,
};
use windows_core::{Ref, BOOL, GUID, HRESULT};

use crate::apo::{ProcessInput, SystemEffectState};
use crate::buffer::{BufferFlags, CONNECTION_PROPERTY_SIGNATURE};
use crate::error::HResult;
use crate::format::Format;
use crate::instance::AnyApoInstance;
use crate::raw::media_type::NegotiationDirection;
#[cfg(feature = "aec")]
use crate::raw::media_type::{format_from_media_type, media_type_from_format};
use crate::raw::reg_properties::build_registration_properties;
use crate::realtime::RealtimeContext;

/// `IAudioProcessingObject::Initialize` body. Routes through
/// [`AnyApoInstance::initialize`].
pub(crate) fn initialize(instance: &dyn AnyApoInstance) -> windows_core::Result<()> {
    instance.initialize().map_err(|e| {
        windows_core::Error::new(HRESULT::from(e), "ProcessingObject::initialize failed")
    })
}

/// `IAudioProcessingObject::GetRegistrationProperties` body —
/// SISO variant. The AEC carrier uses
/// [`crate::aec::exports::build_aec_registration_properties`]
/// directly to advertise the nine-IID interface list.
pub(crate) fn get_registration_properties_siso(
    instance: &dyn AnyApoInstance,
) -> windows_core::Result<*mut APO_REG_PROPERTIES> {
    build_registration_properties(instance)
}

/// `IAudioProcessingObject::IsInputFormatSupported` /
/// `IsOutputFormatSupported` body. The opposite-format pointer is
/// currently ignored.
pub(crate) fn negotiate_format(
    instance: &dyn AnyApoInstance,
    requested: Ref<'_, IAudioMediaType>,
    direction: NegotiationDirection,
) -> windows_core::Result<IAudioMediaType> {
    crate::raw::media_type::negotiate_format(instance, requested, direction)
}

/// `IAudioProcessingObject::GetInputChannelCount` body. Reads the
/// cached locked input format.
pub(crate) fn get_input_channel_count(instance: &dyn AnyApoInstance) -> windows_core::Result<u32> {
    instance
        .locked_formats()
        .map(|f| u32::from(f.input.channels()))
        .ok_or_else(|| {
            windows_core::Error::new(
                HRESULT::from(HResult::APOERR_NOT_LOCKED),
                "GetInputChannelCount called outside of the Locked state",
            )
        })
}

/// `IAudioProcessingObjectConfiguration::LockForProcess` body.
/// Validates the SISO 1×1 connection-count contract, parses the
/// descriptor pair, and forwards to [`AnyApoInstance::lock_for_process`].
///
/// # Safety
///
/// `ppinputconnections` and `ppoutputconnections` must be either
/// null or point to arrays of `APO_CONNECTION_DESCRIPTOR*` of
/// length matching the count parameters; the audio engine
/// guarantees this in `LockForProcess`.
pub(crate) unsafe fn lock_for_process(
    instance: &dyn AnyApoInstance,
    u32numinputconnections: u32,
    ppinputconnections: *const *const APO_CONNECTION_DESCRIPTOR,
    u32numoutputconnections: u32,
    ppoutputconnections: *const *const APO_CONNECTION_DESCRIPTOR,
) -> windows_core::Result<()> {
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
    // Safety: count was checked == 1 and pointers are non-null.
    let input_desc = unsafe { *ppinputconnections };
    let output_desc = unsafe { *ppoutputconnections };
    // Safety: descriptors guaranteed valid by the engine.
    let input_format = unsafe { format_from_descriptor(input_desc) }?;
    let output_format = unsafe { format_from_descriptor(output_desc) }?;
    instance
        .lock_for_process(&input_format, &output_format)
        .map_err(|e| {
            windows_core::Error::new(
                HRESULT::from(e),
                "ProcessingObject::lock_for_process failed",
            )
        })
}

/// `IAudioProcessingObjectConfiguration::UnlockForProcess` body.
pub(crate) fn unlock_for_process(instance: &dyn AnyApoInstance) -> windows_core::Result<()> {
    instance.unlock_for_process().map_err(|e| {
        windows_core::Error::new(
            HRESULT::from(e),
            "ProcessingObject::unlock_for_process failed",
        )
    })
}

/// `IAudioProcessingObjectRT::APOProcess` body. Slices the
/// host-supplied float32 buffers and dispatches into
/// [`AnyApoInstance::process`]. Emits `BUFFER_SILENT` on every
/// failure path because `APOProcess` has no `HRESULT` return.
///
/// # Safety
///
/// Pointers must obey the `APOProcess` ABI: `ppinputconnections`
/// and `ppoutputconnections` either null or arrays of
/// `APO_CONNECTION_PROPERTY*` of the declared lengths.
pub(crate) unsafe fn apo_process(
    instance: &dyn AnyApoInstance,
    u32numinputconnections: u32,
    ppinputconnections: *const *const APO_CONNECTION_PROPERTY,
    u32numoutputconnections: u32,
    ppoutputconnections: *mut *mut APO_CONNECTION_PROPERTY,
) {
    let mark_output_silent = |frame_count: u32| {
        if u32numoutputconnections == 1 && !ppoutputconnections.is_null() {
            // Safety: count is 1 and pointer is non-null.
            let out_ptr = unsafe { *ppoutputconnections };
            if !out_ptr.is_null() {
                // Safety: COM caller's writable slot.
                unsafe {
                    (*out_ptr).u32ValidFrameCount = frame_count;
                    (*out_ptr).u32BufferFlags = BUFFER_SILENT;
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
    // Safety: count == 1 and pointers non-null per the checks.
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

    let Some(formats) = instance.locked_formats() else {
        mark_output_silent(0);
        return;
    };
    let channels = formats.input.channels() as usize;
    if channels == 0 || !formats.input.is_float() || formats.input.bits_per_sample() != 32 {
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

    // Safety: host guarantees pBuffer holds at least
    // u32ValidFrameCount × channels float32 samples.
    let input_slice =
        unsafe { core::slice::from_raw_parts(in_prop.pBuffer as *const f32, sample_count) };
    let output_slice =
        unsafe { core::slice::from_raw_parts_mut(out_prop.pBuffer as *mut f32, sample_count) };

    let in_flags: BufferFlags = in_prop.u32BufferFlags.into();
    // Safety: APOProcess runs on the realtime thread (engine
    // contract); the witness is valid for the duration of the call.
    let rt = unsafe { RealtimeContext::new_unchecked() };

    let out_flags =
        match instance.process(&rt, ProcessInput::new(input_slice, in_flags), output_slice) {
            Ok(f) => f,
            Err(_) => {
                mark_output_silent(in_prop.u32ValidFrameCount);
                return;
            }
        };

    out_prop.u32ValidFrameCount = in_prop.u32ValidFrameCount;
    out_prop.u32BufferFlags = out_flags.into();
}

/// `IAudioSystemEffects2::GetEffectsList` body. Allocates a
/// `CoTaskMemAlloc`-backed GUID list per-effect from the user APO's
/// snapshot.
pub(crate) fn get_effects_list(
    instance: &dyn AnyApoInstance,
    ppeffectsids: *mut *mut GUID,
    pceffects: *mut u32,
    _event: HANDLE,
) -> windows_core::Result<()> {
    if ppeffectsids.is_null() || pceffects.is_null() {
        return Err(windows_core::Error::new(
            HRESULT::from(HResult::E_POINTER),
            "GetEffectsList output pointers were null",
        ));
    }

    let effects = instance.system_effects();
    let count = effects.len();

    if count == 0 {
        // Safety: pointers null-checked above.
        unsafe {
            *ppeffectsids = core::ptr::null_mut();
            *pceffects = 0;
        }
        return Ok(());
    }

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
    // Safety: list points to count × size_of::<GUID>() bytes.
    unsafe {
        for (i, effect) in effects.iter().enumerate() {
            core::ptr::write(list.add(i), effect.id.into());
        }
        *ppeffectsids = list;
        *pceffects = count as u32;
    }
    Ok(())
}

/// `IAudioSystemEffects3::GetControllableSystemEffectsList` body.
pub(crate) fn get_controllable_system_effects_list(
    instance: &dyn AnyApoInstance,
    effects_out: *mut *mut AUDIO_SYSTEMEFFECT,
    numeffects: *mut u32,
    _event: HANDLE,
) -> windows_core::Result<()> {
    if effects_out.is_null() || numeffects.is_null() {
        return Err(windows_core::Error::new(
            HRESULT::from(HResult::E_POINTER),
            "GetControllableSystemEffectsList output pointers were null",
        ));
    }

    let user_effects = instance.system_effects();
    let count = user_effects.len();

    if count == 0 {
        // Safety: pointers null-checked above.
        unsafe {
            *effects_out = core::ptr::null_mut();
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
    // Safety: list points to count × size_of::<AUDIO_SYSTEMEFFECT>() bytes.
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
        *effects_out = list;
        *numeffects = count as u32;
    }
    Ok(())
}

/// `IAudioSystemEffects3::SetAudioSystemEffectState` body. Rejects
/// IDs the APO never advertised.
pub(crate) fn set_audio_system_effect_state(
    instance: &dyn AnyApoInstance,
    effectid: &GUID,
    state: AUDIO_SYSTEMEFFECT_STATE,
) -> windows_core::Result<()> {
    let requested = crate::clsid::Clsid::from(*effectid);
    if !instance.system_effects().iter().any(|e| e.id == requested) {
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
    instance.set_system_effect_state(&requested, cross);
    Ok(())
}

/// `IAudioMediaType::IsEqual` returns `E_NOTIMPL` from the bridge —
/// re-exported here for symmetry with the other helpers though
/// neither carrier currently uses it (the bridge only constructs
/// `FormatMediaType` instances and never has the engine call back
/// into them).
///
/// Reserved for future shared use; intentionally not currently
/// referenced by either carrier.
#[allow(dead_code)]
pub(crate) fn iaudiomediatype_isequal_notimpl() -> windows_core::Result<u32> {
    Err(windows_core::Error::new(
        HRESULT::from(HResult::E_NOTIMPL),
        "IsEqual not implemented",
    ))
}

/// Extract a [`Format`] from a host-supplied
/// [`APO_CONNECTION_DESCRIPTOR`].
///
/// Re-exposed from [`crate::raw::instance_com`] so the AEC bridge
/// (which lives in a different module tree) can reach it without
/// the carrier struct in scope.
///
/// # Safety
///
/// `descriptor` must point to a valid `APO_CONNECTION_DESCRIPTOR`;
/// the audio engine guarantees this in `LockForProcess` and
/// `AddAuxiliaryInput`. The returned `Format` holds a deep copy
/// of the underlying fields; no references survive the call.
pub(crate) unsafe fn format_from_descriptor(
    descriptor: *const APO_CONNECTION_DESCRIPTOR,
) -> windows_core::Result<Format> {
    if descriptor.is_null() {
        return Err(windows_core::Error::new(
            HRESULT::from(HResult::E_POINTER),
            "APO_CONNECTION_DESCRIPTOR pointer was null",
        ));
    }
    // Safety: caller guarantees valid pointer.
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
    // Safety: media_type valid for the call duration.
    let wf_ptr = unsafe { media_type.GetAudioFormat() };
    if wf_ptr.is_null() {
        return Err(windows_core::Error::new(
            HRESULT::from(HResult::APOERR_FORMAT_NOT_SUPPORTED),
            "IAudioMediaType::GetAudioFormat returned null",
        ));
    }
    // Safety: GetAudioFormat returns a non-null WAVEFORMATEX pointer.
    Ok(unsafe { Format::from_waveformatex_ptr(wf_ptr) })
}

/// AEC `IApoAuxiliaryInputConfiguration::AddAuxiliaryInput` body.
/// Lives here rather than in the AEC module so the dispatch
/// helpers are co-located.
///
/// # Safety
///
/// `pinputconnection` must be a valid `APO_CONNECTION_DESCRIPTOR*`
/// (the audio engine guarantees this); `pbydata`/`cbdatasize`
/// must describe a valid byte slice (the framework treats null +
/// zero-size as "no init data").
#[cfg(feature = "aec")]
pub(crate) unsafe fn aec_add_auxiliary_input(
    instance: &dyn crate::aec::AnyAecApoInstance,
    dwinputid: u32,
    cbdatasize: u32,
    pbydata: *const u8,
    pinputconnection: *const APO_CONNECTION_DESCRIPTOR,
) -> windows_core::Result<()> {
    // Safety: caller-guaranteed valid descriptor.
    let format = unsafe { format_from_descriptor(pinputconnection) }?;
    let init_data: &[u8] = if cbdatasize == 0 || pbydata.is_null() {
        &[]
    } else {
        // Safety: caller guarantees pbydata is valid for at least
        // cbdatasize bytes.
        unsafe { core::slice::from_raw_parts(pbydata, cbdatasize as usize) }
    };
    instance
        .add_aux_input(dwinputid, &format, init_data)
        .map_err(|e| {
            windows_core::Error::new(
                HRESULT::from(e),
                "AecProcessingObject::add_aux_input failed",
            )
        })
}

/// AEC `IApoAuxiliaryInputConfiguration::IsInputFormatSupported`
/// body. Routes through [`crate::aec::AnyAecApoInstance::is_aux_format_supported`].
#[cfg(feature = "aec")]
pub(crate) fn aec_is_aux_format_supported(
    instance: &dyn crate::aec::AnyAecApoInstance,
    prequestedinputformat: Ref<'_, IAudioMediaType>,
) -> windows_core::Result<IAudioMediaType> {
    let requested_format = format_from_media_type(prequestedinputformat)?;
    let decision = instance.is_aux_format_supported(&requested_format);
    match decision {
        crate::format::FormatNegotiation::Accept => Ok(media_type_from_format(&requested_format)),
        crate::format::FormatNegotiation::Suggest(alt) => Ok(media_type_from_format(&alt)),
        crate::format::FormatNegotiation::Reject => Err(windows_core::Error::new(
            HRESULT::from(HResult::APOERR_FORMAT_NOT_SUPPORTED),
            "AecProcessingObject rejected the requested aux format",
        )),
    }
}

/// AEC `IApoAuxiliaryInputRT::AcceptInput` body. Slices the
/// host-supplied float32 buffer and dispatches into the user's
/// realtime aux-input hook. Bails silently on every precondition
/// failure (the COM method has no `HRESULT` to surface errors).
///
/// # Safety
///
/// `pinputconnection` either null or a valid
/// `APO_CONNECTION_PROPERTY*` (engine contract).
#[cfg(feature = "aec")]
pub(crate) unsafe fn aec_accept_input(
    instance: &dyn crate::aec::AnyAecApoInstance,
    dwinputid: u32,
    pinputconnection: *const APO_CONNECTION_PROPERTY,
) {
    if pinputconnection.is_null() {
        return;
    }
    // Safety: caller guarantees valid pointer for the call.
    let prop = unsafe { &*pinputconnection };
    if prop.u32Signature != CONNECTION_PROPERTY_SIGNATURE {
        return;
    }
    // Aux buffer geometry currently inherits the primary input's
    // locked format; per-aux-input formats are a future
    // refinement.
    let Some(formats) = instance.locked_formats() else {
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
    // Safety: host guarantees the buffer holds at least
    // u32ValidFrameCount × channels float32 samples.
    let samples = unsafe { core::slice::from_raw_parts(prop.pBuffer as *const f32, sample_count) };
    let flags: BufferFlags = prop.u32BufferFlags.into();
    // Safety: AcceptInput runs on the realtime thread.
    let rt = unsafe { RealtimeContext::new_unchecked() };
    instance.accept_aux_input(
        &rt,
        crate::aec::AuxiliaryInputBuffer {
            id: dwinputid,
            samples,
            flags,
        },
    );
}
