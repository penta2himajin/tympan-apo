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
//! This PR lands the macro plumbing — the wrapper struct, the
//! `#[implement(IAudioProcessingObject)]` annotation, and the
//! seven trait methods. Several method bodies are still stubs
//! returning `E_NOTIMPL` while the format-negotiation /
//! registration-properties translation layers are designed in
//! follow-ups; the trivially-correct ones (`Reset`, `GetLatency`,
//! `GetInputChannelCount`) are wired up.

// The `windows_core::implement` proc-macro generates a sibling
// `*_Impl` struct without doc-comments; the crate-wide
// `#![deny(missing_docs)]` would otherwise reject the expansion.
#![allow(missing_docs)]

extern crate alloc;

use alloc::sync::Arc;

use windows::Win32::Media::Audio::Apo::{
    IAudioMediaType, IAudioProcessingObject, IAudioProcessingObject_Impl, APO_REG_PROPERTIES,
};
use windows_core::{implement, Ref, HRESULT};

use crate::error::HResult;
use crate::instance::AnyApoInstance;

/// COM-side carrier for an [`Arc<dyn AnyApoInstance>`](AnyApoInstance).
///
/// One of these is materialised per `IClassFactory::CreateInstance`
/// call. The carrier is what the audio engine sees as an
/// `IAudioProcessingObject*`; methods on the COM interface route
/// through this struct into the user's `ProcessingObject` via the
/// type-erased trait.
#[implement(IAudioProcessingObject)]
pub struct ApoInstanceCom {
    instance: Arc<dyn AnyApoInstance>,
}

impl ApoInstanceCom {
    /// Wrap an existing instance for COM exposure.
    ///
    /// Called by the framework's class factory; users do not
    /// construct this directly.
    #[must_use]
    pub fn new(instance: Arc<dyn AnyApoInstance>) -> Self {
        Self { instance }
    }

    /// Borrow the underlying `AnyApoInstance`. Used by the
    /// future IAudioProcessingObjectConfiguration / RT wrappers.
    #[must_use]
    pub fn instance(&self) -> &Arc<dyn AnyApoInstance> {
        &self.instance
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
        // APO_REG_PROPERTIES has a variable-length IID list at
        // the tail and needs a CoTaskMemAlloc-allocated buffer.
        // Implementing it correctly requires the
        // ApoVTable.{name,copyright,clsid} accessor work that
        // will land alongside the IUnknown wrapper for the class
        // factory.
        Err(windows_core::Error::new(
            HRESULT::from(HResult::E_NOTIMPL),
            "GetRegistrationProperties not yet implemented",
        ))
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
        _prequestedinputformat: Ref<IAudioMediaType>,
    ) -> windows_core::Result<IAudioMediaType> {
        Err(windows_core::Error::new(
            HRESULT::from(HResult::E_NOTIMPL),
            "IAudioMediaType <-> Format bridge not yet implemented",
        ))
    }

    fn IsOutputFormatSupported(
        &self,
        _poppositeformat: Ref<IAudioMediaType>,
        _prequestedoutputformat: Ref<IAudioMediaType>,
    ) -> windows_core::Result<IAudioMediaType> {
        Err(windows_core::Error::new(
            HRESULT::from(HResult::E_NOTIMPL),
            "IAudioMediaType <-> Format bridge not yet implemented",
        ))
    }

    fn GetInputChannelCount(&self) -> windows_core::Result<u32> {
        // The audio engine queries this after LockForProcess;
        // the framework can answer once the locked input format
        // is cached on AnyApoInstance. Stubbed until then.
        Err(windows_core::Error::new(
            HRESULT::from(HResult::APOERR_NOT_LOCKED),
            "GetInputChannelCount called before LockForProcess wiring lands",
        ))
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
}
