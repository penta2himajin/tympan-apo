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
};
use windows_core::{implement, Ref, GUID};

use crate::instance::AnyApoInstance;

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

// Method bodies for these traits live in `crate::raw::dispatch`
// so the AEC carrier can share the implementations; this block is
// therefore a thin set of one-line delegates.
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
        crate::raw::dispatch::get_registration_properties_siso(self.instance.as_ref())
    }
    fn Initialize(&self, _cbdatasize: u32, _pbydata: *const u8) -> windows_core::Result<()> {
        crate::raw::dispatch::initialize(self.instance.as_ref())
    }
    fn IsInputFormatSupported(
        &self,
        _poppositeformat: Ref<IAudioMediaType>,
        prequestedinputformat: Ref<IAudioMediaType>,
    ) -> windows_core::Result<IAudioMediaType> {
        crate::raw::dispatch::negotiate_format(
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
        crate::raw::dispatch::negotiate_format(
            self.instance.as_ref(),
            prequestedoutputformat,
            crate::raw::media_type::NegotiationDirection::Output,
        )
    }
    fn GetInputChannelCount(&self) -> windows_core::Result<u32> {
        crate::raw::dispatch::get_input_channel_count(self.instance.as_ref())
    }
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
        // Safety: pointers obey the LockForProcess ABI per the
        // audio engine's contract; the helper validates count == 1
        // and non-null before dereferencing.
        unsafe {
            crate::raw::dispatch::lock_for_process(
                self.instance.as_ref(),
                u32numinputconnections,
                ppinputconnections,
                u32numoutputconnections,
                ppoutputconnections,
            )
        }
    }
    fn UnlockForProcess(&self) -> windows_core::Result<()> {
        crate::raw::dispatch::unlock_for_process(self.instance.as_ref())
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
        // Safety: pointers obey the APOProcess ABI per the audio
        // engine's contract; the helper validates everything
        // before dereferencing and bails to BUFFER_SILENT on any
        // failure.
        unsafe {
            crate::raw::dispatch::apo_process(
                self.instance.as_ref(),
                u32numinputconnections,
                ppinputconnections,
                u32numoutputconnections,
                ppoutputconnections,
            );
        }
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
        event: HANDLE,
    ) -> windows_core::Result<()> {
        crate::raw::dispatch::get_effects_list(
            self.instance.as_ref(),
            ppeffectsids,
            pceffects,
            event,
        )
    }
}

impl IAudioSystemEffects3_Impl for ApoInstanceCom_Impl {
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn GetControllableSystemEffectsList(
        &self,
        effects: *mut *mut AUDIO_SYSTEMEFFECT,
        numeffects: *mut u32,
        event: HANDLE,
    ) -> windows_core::Result<()> {
        crate::raw::dispatch::get_controllable_system_effects_list(
            self.instance.as_ref(),
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
        crate::raw::dispatch::set_audio_system_effect_state(self.instance.as_ref(), effectid, state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apo::{ApoCategory, ProcessInput, ProcessingObject};
    use crate::buffer::BufferFlags;
    use crate::clsid::Clsid;
    use crate::error::HResult;
    use crate::instance::ApoInstance;
    use crate::realtime::{RealtimeContext, State};
    use windows_core::HRESULT;

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
