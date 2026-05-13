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
//! The shared SISO method bodies live in
//! [`crate::raw::dispatch`]; the trait impls below are thin
//! delegates so the two carriers stay in lock-step. AEC-specific
//! divergences are limited to `GetRegistrationProperties`
//! (advertises nine IIDs) and the three AEC trait impls.

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
};
use windows_core::{implement, Ref, GUID};

use crate::aec::AnyAecApoInstance;

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

// Method bodies for these traits live in `crate::raw::dispatch`
// so SISO and AEC carriers share the implementations; this block is
// a thin set of one-line delegates with the AEC-specific
// `GetRegistrationProperties` (advertises nine interfaces).
impl IAudioProcessingObject_Impl for AecApoInstanceCom_Impl {
    fn Reset(&self) -> windows_core::Result<()> {
        Ok(())
    }
    fn GetLatency(&self) -> windows_core::Result<i64> {
        Ok(0)
    }
    fn GetRegistrationProperties(&self) -> windows_core::Result<*mut APO_REG_PROPERTIES> {
        // AEC variant advertises nine interfaces (six SISO + three
        // AEC) — distinct from the SISO carrier's six-IID list.
        crate::aec::exports::build_aec_registration_properties(self.instance.as_any_apo_instance())
    }
    fn Initialize(&self, _cbdatasize: u32, _pbydata: *const u8) -> windows_core::Result<()> {
        crate::raw::dispatch::initialize(self.instance.as_any_apo_instance())
    }
    fn IsInputFormatSupported(
        &self,
        _poppositeformat: Ref<IAudioMediaType>,
        prequestedinputformat: Ref<IAudioMediaType>,
    ) -> windows_core::Result<IAudioMediaType> {
        crate::raw::dispatch::negotiate_format(
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
        crate::raw::dispatch::negotiate_format(
            self.instance.as_any_apo_instance(),
            prequestedoutputformat,
            crate::raw::media_type::NegotiationDirection::Output,
        )
    }
    fn GetInputChannelCount(&self) -> windows_core::Result<u32> {
        crate::raw::dispatch::get_input_channel_count(self.instance.as_any_apo_instance())
    }
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
        // Safety: pointers obey the LockForProcess ABI per the
        // audio engine's contract.
        unsafe {
            crate::raw::dispatch::lock_for_process(
                self.instance.as_any_apo_instance(),
                u32numinputconnections,
                ppinputconnections,
                u32numoutputconnections,
                ppoutputconnections,
            )
        }
    }
    fn UnlockForProcess(&self) -> windows_core::Result<()> {
        crate::raw::dispatch::unlock_for_process(self.instance.as_any_apo_instance())
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
        // Safety: pointers obey the APOProcess ABI per the audio
        // engine's contract; the helper validates everything
        // before dereferencing and bails to BUFFER_SILENT on any
        // failure.
        unsafe {
            crate::raw::dispatch::apo_process(
                self.instance.as_any_apo_instance(),
                u32numinputconnections,
                ppinputconnections,
                u32numoutputconnections,
                ppoutputconnections,
            );
        }
    }
    fn CalcInputFrames(&self, u32outputframecount: u32) -> u32 {
        u32outputframecount
    }
    fn CalcOutputFrames(&self, u32inputframecount: u32) -> u32 {
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
        event: HANDLE,
    ) -> windows_core::Result<()> {
        crate::raw::dispatch::get_effects_list(
            self.instance.as_any_apo_instance(),
            ppeffectsids,
            pceffects,
            event,
        )
    }
}

impl IAudioSystemEffects3_Impl for AecApoInstanceCom_Impl {
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn GetControllableSystemEffectsList(
        &self,
        effects: *mut *mut AUDIO_SYSTEMEFFECT,
        numeffects: *mut u32,
        event: HANDLE,
    ) -> windows_core::Result<()> {
        crate::raw::dispatch::get_controllable_system_effects_list(
            self.instance.as_any_apo_instance(),
            effects,
            numeffects,
            event,
        )
    }
    fn SetAudioSystemEffectState(
        &self,
        effectid: &GUID,
        state: AUDIO_SYSTEMEFFECT_STATE,
    ) -> windows_core::Result<()> {
        crate::raw::dispatch::set_audio_system_effect_state(
            self.instance.as_any_apo_instance(),
            effectid,
            state,
        )
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
        // call; the helper performs its own null / signature checks.
        unsafe {
            crate::raw::dispatch::aec_add_auxiliary_input(
                self.instance.as_ref(),
                dwinputid,
                cbdatasize,
                pbydata,
                pinputconnection,
            )
        }
    }

    fn RemoveAuxiliaryInput(&self, dwinputid: u32) -> windows_core::Result<()> {
        self.instance.remove_aux_input(dwinputid);
        Ok(())
    }

    fn IsInputFormatSupported(
        &self,
        prequestedinputformat: Ref<IAudioMediaType>,
    ) -> windows_core::Result<IAudioMediaType> {
        crate::raw::dispatch::aec_is_aux_format_supported(
            self.instance.as_ref(),
            prequestedinputformat,
        )
    }
}

impl IApoAuxiliaryInputRT_Impl for AecApoInstanceCom_Impl {
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn AcceptInput(&self, dwinputid: u32, pinputconnection: *const APO_CONNECTION_PROPERTY) {
        // Safety: caller-supplied pointer obeys the AcceptInput ABI;
        // the helper validates everything before dereferencing.
        unsafe {
            crate::raw::dispatch::aec_accept_input(
                self.instance.as_ref(),
                dwinputid,
                pinputconnection,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aec::{AecApoInstance, AecProcessingObject};
    use crate::apo::{ApoCategory, ProcessInput, ProcessingObject};
    use crate::buffer::{BufferFlags, CONNECTION_PROPERTY_SIGNATURE};
    use crate::clsid::Clsid;
    use crate::error::HResult;
    use crate::format::Format;
    use crate::realtime::RealtimeContext;
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
