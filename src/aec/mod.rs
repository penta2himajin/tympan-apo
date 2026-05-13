//! Windows 11 (23H2+) Acoustic Echo Cancellation APO support.
//!
//! Activates the AEC slot in the Windows audio engine's microphone
//! capture pipeline. Users implement [`AecProcessingObject`]
//! (extending [`ProcessingObject`]) and (in a follow-up PR)
//! register their cdylib with `register_aec_apo!` instead of the
//! standard `register_apo!` macro.
//!
//! ## Interfaces
//!
//! An AEC APO advertises three additional COM interfaces past the
//! standard six the SISO path uses:
//!
//! - `IApoAcousticEchoCancellation` — empty marker that opts the
//!   APO into the AEC slot.
//! - `IApoAuxiliaryInputConfiguration` — `AddAuxiliaryInput` /
//!   `RemoveAuxiliaryInput` / `IsInputFormatSupported` for the
//!   reference signal (loopback from the render endpoint).
//! - `IApoAuxiliaryInputRT` — realtime `AcceptInput` that delivers
//!   the aux signal samples buffer-by-buffer.
//!
//! ## Feature gate
//!
//! The whole module is gated on the `aec` cargo feature; enable it
//! by adding `tympan-apo = { version = "...", features = ["aec"] }`
//! to the consumer crate.

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::apo::ProcessInput;
use crate::buffer::BufferFlags;
use crate::error::HResult;
use crate::format::{Format, FormatNegotiation};
use crate::instance::{AnyApoInstance, ApoInstance, LockedFormats};
use crate::realtime::{RealtimeContext, State};
use crate::ProcessingObject;

/// Per-buffer auxiliary input handed to
/// [`AecProcessingObject::accept_aux_input`].
///
/// The Windows audio engine calls `AcceptInput` once per output
/// frame after the loopback render samples for that frame are
/// ready, passing them as interleaved float32. The `id` carries
/// the auxiliary input slot the engine previously announced via
/// `AddAuxiliaryInput`.
#[derive(Copy, Clone, Debug)]
pub struct AuxiliaryInputBuffer<'a> {
    /// Auxiliary-input slot ID, as set up by
    /// [`AecProcessingObject::add_aux_input`].
    pub id: u32,
    /// Interleaved float32 samples for this buffer.
    pub samples: &'a [f32],
    /// Status flags the engine stamped on the buffer.
    pub flags: BufferFlags,
}

/// User-implemented AEC APO.
///
/// Extends [`ProcessingObject`] with the aux-input lifecycle hooks
/// the AEC APO API requires. Implementors handle the reference
/// (loopback) signal in `accept_aux_input` and cancel it out of
/// `process` via whatever echo-cancellation algorithm they
/// implement.
///
/// Realtime safety: `accept_aux_input` runs on the audio engine's
/// realtime thread (the same one that calls `process`), so the same
/// allocation-free / lock-free constraints apply.
pub trait AecProcessingObject: ProcessingObject {
    /// Engine handshake: a new auxiliary input is being added.
    ///
    /// Called from a non-realtime thread before any
    /// `accept_aux_input` for the given `id`. The default
    /// implementation returns `Ok(())` — implementations that need
    /// per-input state should pre-allocate it here.
    fn add_aux_input(&mut self, id: u32, format: &Format, init_data: &[u8]) -> Result<(), HResult> {
        let _ = (id, format, init_data);
        Ok(())
    }

    /// Engine handshake: an auxiliary input is being removed.
    ///
    /// Called from a non-realtime thread; no further
    /// `accept_aux_input` calls for `id` will arrive. Default is a
    /// no-op.
    fn remove_aux_input(&mut self, id: u32) {
        let _ = id;
    }

    /// Decide whether `format` is acceptable for an auxiliary input.
    ///
    /// Default delegates to [`ProcessingObject::is_input_format_supported`]
    /// so AEC APOs that take the same format for primary and aux
    /// inputs do not need to override it.
    fn is_aux_format_supported(&self, format: &Format) -> FormatNegotiation {
        ProcessingObject::is_input_format_supported(self, format)
    }

    /// Realtime: receive one buffer of auxiliary-input samples.
    ///
    /// Called from the audio engine's realtime thread before the
    /// matching `process` invocation. Default is a no-op; AEC
    /// implementations stash the samples for cancellation in the
    /// subsequent `process` call.
    fn accept_aux_input(&mut self, rt: &RealtimeContext, input: AuxiliaryInputBuffer<'_>) {
        let _ = (rt, input);
    }
}

/// Type-erased view of an [`AecApoInstance`].
///
/// Mirrors [`AnyApoInstance`] but adds the AEC-specific methods
/// that the COM bridge dispatches through.
pub trait AnyAecApoInstance: AnyApoInstance {
    /// Forward `add_aux_input` to the user's `AecProcessingObject`.
    fn add_aux_input(&self, id: u32, format: &Format, init_data: &[u8]) -> Result<(), HResult>;
    /// Forward `remove_aux_input` to the user.
    fn remove_aux_input(&self, id: u32);
    /// Forward `is_aux_format_supported` to the user.
    fn is_aux_format_supported(&self, format: &Format) -> FormatNegotiation;
    /// Forward `accept_aux_input` to the user (realtime path).
    fn accept_aux_input(&self, rt: &RealtimeContext, input: AuxiliaryInputBuffer<'_>);
}

/// COM-side wrapper around a `T: AecProcessingObject`.
///
/// Built on top of [`ApoInstance<T>`] — the SISO state machine is
/// reused — with AEC-specific dispatch handled by the additional
/// trait impls below.
pub struct AecApoInstance<T: AecProcessingObject> {
    /// Inner SISO instance carrying the state machine and refcount.
    inner: ApoInstance<T>,
}

// Safety: same rationale as `ApoInstance`'s Sync impl; the audio
// engine serialises non-realtime calls against each other and
// against the realtime path through its own contract.
unsafe impl<T: AecProcessingObject> Sync for AecApoInstance<T> {}

impl<T: AecProcessingObject> AecApoInstance<T> {
    /// Construct a fresh AEC instance.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: ApoInstance::<T>::new(),
        }
    }

    /// Borrow the inner SISO instance.
    #[inline]
    #[must_use]
    pub fn inner(&self) -> &ApoInstance<T> {
        &self.inner
    }
}

impl<T: AecProcessingObject> Default for AecApoInstance<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: AecProcessingObject> AnyApoInstance for AecApoInstance<T> {
    #[inline]
    fn add_ref(&self) -> u32 {
        self.inner.add_ref()
    }
    #[inline]
    fn release(&self) -> u32 {
        self.inner.release()
    }
    #[inline]
    fn refcount(&self) -> u32 {
        self.inner.refcount()
    }
    #[inline]
    fn state(&self) -> State {
        self.inner.state()
    }
    #[inline]
    fn initialize(&self) -> Result<(), HResult> {
        self.inner.initialize()
    }
    #[inline]
    fn is_input_format_supported(&self, format: &Format) -> FormatNegotiation {
        self.inner.is_input_format_supported(format)
    }
    #[inline]
    fn is_output_format_supported(&self, format: &Format) -> FormatNegotiation {
        self.inner.is_output_format_supported(format)
    }
    #[inline]
    fn lock_for_process(&self, input: &Format, output: &Format) -> Result<(), HResult> {
        self.inner.lock_for_process(input, output)
    }
    #[inline]
    fn unlock_for_process(&self) -> Result<(), HResult> {
        self.inner.unlock_for_process()
    }
    #[inline]
    fn process(
        &self,
        rt: &RealtimeContext,
        input: ProcessInput<'_>,
        output: &mut [f32],
    ) -> Result<BufferFlags, HResult> {
        self.inner.process(rt, input, output)
    }
    #[inline]
    fn locked_formats(&self) -> Option<LockedFormats> {
        self.inner.locked_formats()
    }
    #[inline]
    fn clsid(&self) -> crate::Clsid {
        T::CLSID
    }
    #[inline]
    fn name(&self) -> &'static str {
        T::NAME
    }
    #[inline]
    fn copyright(&self) -> &'static str {
        T::COPYRIGHT
    }
    #[inline]
    fn category(&self) -> crate::ApoCategory {
        T::CATEGORY
    }
    #[inline]
    fn system_effects(&self) -> Vec<crate::SystemEffect> {
        self.inner.system_effects()
    }
    #[inline]
    fn set_system_effect_state(&self, id: &crate::Clsid, state: crate::SystemEffectState) {
        self.inner.set_system_effect_state(id, state);
    }
}

impl<T: AecProcessingObject> AnyAecApoInstance for AecApoInstance<T> {
    fn add_aux_input(&self, id: u32, format: &Format, init_data: &[u8]) -> Result<(), HResult> {
        // Safety: AddAuxiliaryInput is non-realtime and serialised
        // by the engine against process / accept_aux_input;
        // `&mut T` access through the inner UnsafeCell is sound
        // under that contract.
        let inner = unsafe { &mut *self.inner.inner_cell().get() };
        inner.add_aux_input(id, format, init_data)
    }

    fn remove_aux_input(&self, id: u32) {
        // Safety: same as add_aux_input.
        let inner = unsafe { &mut *self.inner.inner_cell().get() };
        inner.remove_aux_input(id);
    }

    fn is_aux_format_supported(&self, format: &Format) -> FormatNegotiation {
        // Safety: read-only access; non-realtime and serialised by
        // the engine against the realtime path.
        let inner = unsafe { &*self.inner.inner_cell().get() };
        inner.is_aux_format_supported(format)
    }

    fn accept_aux_input(&self, rt: &RealtimeContext, input: AuxiliaryInputBuffer<'_>) {
        // Safety: realtime path; the engine guarantees this is not
        // concurrent with non-realtime AddAuxiliaryInput etc.
        let inner = unsafe { &mut *self.inner.inner_cell().get() };
        inner.accept_aux_input(rt, input);
    }
}

/// Type-erased AEC instance constructor used by
/// `register_aec_apo!` to populate the
/// `AecApoVTable::create` slot.
#[doc(hidden)]
#[must_use]
pub fn make_aec_instance<T: AecProcessingObject + 'static>() -> Arc<dyn AnyAecApoInstance> {
    Arc::new(AecApoInstance::<T>::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apo::ApoCategory;
    use crate::clsid::Clsid;
    use core::cell::Cell;

    /// AEC implementor that records the engine-driven aux-input
    /// calls so tests can assert on the dispatch path.
    struct AuxTrace {
        added: Cell<Option<u32>>,
        removed: Cell<Option<u32>>,
        accepts: Cell<u32>,
        custom_aux_negotiation: Cell<bool>,
    }

    impl ProcessingObject for AuxTrace {
        const CLSID: Clsid = Clsid::from_u128(0x00112233_4455_6677_8899_AABBCCDDEEFF);
        const NAME: &'static str = "aec aux-trace";
        const COPYRIGHT: &'static str = "test";
        const CATEGORY: ApoCategory = ApoCategory::Mfx;
        fn new() -> Self {
            Self {
                added: Cell::new(None),
                removed: Cell::new(None),
                accepts: Cell::new(0),
                custom_aux_negotiation: Cell::new(false),
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

    impl AecProcessingObject for AuxTrace {
        fn add_aux_input(
            &mut self,
            id: u32,
            _format: &Format,
            _init_data: &[u8],
        ) -> Result<(), HResult> {
            self.added.set(Some(id));
            Ok(())
        }
        fn remove_aux_input(&mut self, id: u32) {
            self.removed.set(Some(id));
        }
        fn is_aux_format_supported(&self, format: &Format) -> FormatNegotiation {
            if self.custom_aux_negotiation.get() {
                // Distinguishable override response.
                FormatNegotiation::Reject
            } else {
                // Match what the trait default does: delegate to
                // the primary-input format negotiation.
                ProcessingObject::is_input_format_supported(self, format)
            }
        }
        fn accept_aux_input(&mut self, _rt: &RealtimeContext, _input: AuxiliaryInputBuffer<'_>) {
            self.accepts.set(self.accepts.get() + 1);
        }
    }

    fn rt() -> RealtimeContext {
        unsafe { RealtimeContext::new_unchecked() }
    }

    #[test]
    fn aec_instance_delegates_aux_lifecycle_calls() {
        let typed = AecApoInstance::<AuxTrace>::new();
        let f = Format::pcm_float32(48_000, 1);
        let rt = rt();
        let samples = [0.0_f32; 4];

        AnyAecApoInstance::add_aux_input(&typed, 9, &f, &[1, 2, 3]).unwrap();
        AnyAecApoInstance::accept_aux_input(
            &typed,
            &rt,
            AuxiliaryInputBuffer {
                id: 9,
                samples: &samples,
                flags: BufferFlags::VALID,
            },
        );
        AnyAecApoInstance::accept_aux_input(
            &typed,
            &rt,
            AuxiliaryInputBuffer {
                id: 9,
                samples: &samples,
                flags: BufferFlags::VALID,
            },
        );
        AnyAecApoInstance::remove_aux_input(&typed, 9);

        // Safety: typed is live and not aliased.
        let trace: &AuxTrace = unsafe { &*typed.inner.inner_cell().get() };
        assert_eq!(trace.added.get(), Some(9));
        assert_eq!(trace.removed.get(), Some(9));
        assert_eq!(trace.accepts.get(), 2);
    }

    #[test]
    fn make_aec_instance_yields_dyn_any_aec_apo_instance() {
        let inst: Arc<dyn AnyAecApoInstance> = make_aec_instance::<AuxTrace>();
        assert_eq!(AnyApoInstance::refcount(inst.as_ref()), 0);
        assert_eq!(AnyApoInstance::state(inst.as_ref()), State::Uninitialized);
    }

    #[test]
    fn aux_format_supported_defaults_to_input_format_supported() {
        let typed = AecApoInstance::<AuxTrace>::new();
        let f = Format::pcm_float32(48_000, 1);
        // Default path: AecProcessingObject::is_aux_format_supported
        // delegates to ProcessingObject::is_input_format_supported,
        // which Accepts float32.
        let result = AnyAecApoInstance::is_aux_format_supported(&typed, &f);
        assert_eq!(result, FormatNegotiation::Accept);
    }

    #[test]
    fn aux_format_supported_uses_override_when_provided() {
        let typed = AecApoInstance::<AuxTrace>::new();
        // Flip the override flag inside the AuxTrace.
        // Safety: not aliased.
        let trace: &AuxTrace = unsafe { &*typed.inner.inner_cell().get() };
        trace.custom_aux_negotiation.set(true);

        let f = Format::pcm_float32(48_000, 1);
        let result = AnyAecApoInstance::is_aux_format_supported(&typed, &f);
        assert_eq!(result, FormatNegotiation::Reject);
    }

    #[test]
    fn aec_instance_state_machine_inherits_from_apo_instance() {
        let typed = AecApoInstance::<AuxTrace>::new();
        assert_eq!(typed.state(), State::Uninitialized);
        assert_eq!(typed.refcount(), 0);
        typed.initialize().unwrap();
        assert_eq!(typed.state(), State::Initialized);
    }
}
