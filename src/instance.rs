//! Framework-side wrapper around a user [`ProcessingObject`].
//!
//! Combines a [`StateCell`] (lifecycle), a [`Refcount`] (COM
//! ownership), and an `UnsafeCell<T>` (user state) into the single
//! object the future COM class-factory will hand to the audio
//! engine. Every host-driven entry point — `Initialize`,
//! `LockForProcess`, `APOProcess`, `UnlockForProcess`, and the
//! IUnknown ref-counting methods — projects through here before
//! reaching the user's [`ProcessingObject`] methods.
//!
//! ## Threading
//!
//! The Windows audio engine serialises all non-realtime calls on
//! one thread, and `APOProcess` runs on the realtime thread only
//! while the cell is [`State::Locked`]. State transitions go
//! through `compare_exchange`, so the realtime path is allowed to
//! see a stable T as long as the host obeys its own contract.
//! `UnsafeCell<T>` is what we use to expose `&mut T` to the
//! method dispatch under that contract.
//!
//! `ApoInstance<T>` is `Sync` even though `T: ProcessingObject`
//! is only `Send`: the framework guarantees that exactly one of
//! the wrappers' methods touches `T` at any moment, and the rest
//! of the struct is composed of atomic primitives.

use core::cell::UnsafeCell;

use crate::apo::{ProcessInput, ProcessingObject};
use crate::buffer::BufferFlags;
use crate::error::HResult;
use crate::format::{Format, FormatNegotiation};
use crate::realtime::{RealtimeContext, Refcount, State, StateCell};

/// COM-side wrapper around a `T: ProcessingObject`.
///
/// Owns the user's APO instance and tracks its lifecycle (state +
/// refcount). Constructed by the framework's class factory and
/// handed to the audio engine as `IAudioProcessingObject*`. Users
/// do not interact with this type directly.
pub struct ApoInstance<T: ProcessingObject> {
    inner: UnsafeCell<T>,
    state: StateCell,
    refcount: Refcount,
}

// Safety: see the module-level doc-comment. The framework's COM
// dispatch ensures exactly one method touches `inner` at a time,
// and the lifecycle CAS serialises lifecycle vs process access.
unsafe impl<T: ProcessingObject> Sync for ApoInstance<T> {}

impl<T: ProcessingObject> ApoInstance<T> {
    /// Construct a fresh instance in the
    /// [`State::Uninitialized`] state with refcount 0.
    ///
    /// Calls `T::new` to materialise the user's APO state; heap
    /// allocation is permitted here.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: UnsafeCell::new(T::new()),
            state: StateCell::new(),
            refcount: Refcount::new(),
        }
    }

    /// Current lifecycle state.
    #[inline]
    #[must_use]
    pub fn state(&self) -> State {
        self.state.load()
    }

    /// Current reference count.
    #[inline]
    #[must_use]
    pub fn refcount(&self) -> u32 {
        self.refcount.count()
    }

    /// Increment the COM reference count and return the new
    /// value. Delegates to [`Refcount::add_ref`].
    #[inline]
    pub fn add_ref(&self) -> u32 {
        self.refcount.add_ref()
    }

    /// Decrement the COM reference count and return the new
    /// value. Delegates to [`Refcount::release`].
    #[inline]
    pub fn release(&self) -> u32 {
        self.refcount.release()
    }

    /// Transition `Uninitialized → Initialized`.
    ///
    /// Surfaces a [`HResult::APOERR_ALREADY_LOCKED`] when the
    /// state is not [`State::Uninitialized`] (matching the
    /// Windows audio engine's behaviour for double-Initialize).
    pub fn initialize(&self) -> Result<(), HResult> {
        self.state
            .initialize()
            .map_err(|_| HResult::APOERR_ALREADY_LOCKED)
    }

    /// Delegate to [`ProcessingObject::is_input_format_supported`].
    ///
    /// Read-only access to `T`; callable in any state.
    pub fn is_input_format_supported(&self, format: &Format) -> FormatNegotiation {
        // Safety: read-only access is sound while no &mut alias
        // is in flight. The framework's dispatch only emits &mut
        // aliases via lock/unlock/process; this method is mutually
        // exclusive with those by the host's serialisation
        // contract.
        let inner = unsafe { &*self.inner.get() };
        inner.is_input_format_supported(format)
    }

    /// Delegate to [`ProcessingObject::is_output_format_supported`].
    pub fn is_output_format_supported(&self, format: &Format) -> FormatNegotiation {
        let inner = unsafe { &*self.inner.get() };
        inner.is_output_format_supported(format)
    }

    /// Transition `Initialized → Locked` and call
    /// [`ProcessingObject::lock_for_process`] on the user.
    ///
    /// Rolls the state machine back to `Initialized` if the
    /// user's `lock_for_process` returns an error, so the engine
    /// can retry without first calling `UnlockForProcess`.
    pub fn lock_for_process(&self, input: &Format, output: &Format) -> Result<(), HResult> {
        self.state.lock().map_err(|err| match err.actual {
            State::Uninitialized => HResult::APOERR_NOT_LOCKED,
            State::Initialized => HResult::E_FAIL, // unreachable in practice
            State::Locked => HResult::APOERR_ALREADY_LOCKED,
        })?;

        // Safety: `state.lock()` succeeded, so the host must not
        // be holding another alias to `inner`. Lock + process do
        // not race with each other.
        let inner = unsafe { &mut *self.inner.get() };
        match inner.lock_for_process(input, output) {
            Ok(()) => Ok(()),
            Err(e) => {
                // Roll back the state machine so the engine can
                // retry from Initialized.
                let _ = self.state.unlock();
                Err(e)
            }
        }
    }

    /// Call [`ProcessingObject::unlock_for_process`] on the user
    /// and transition `Locked → Initialized`.
    pub fn unlock_for_process(&self) -> Result<(), HResult> {
        if self.state.load() != State::Locked {
            return Err(HResult::APOERR_NOT_LOCKED);
        }
        // Safety: state == Locked means no other thread is in
        // process(); host serialises lock/unlock with process.
        let inner = unsafe { &mut *self.inner.get() };
        inner.unlock_for_process();
        self.state
            .unlock()
            .map_err(|_| HResult::APOERR_NOT_LOCKED)?;
        Ok(())
    }

    /// Forward an audio buffer into the user's
    /// [`ProcessingObject::process`].
    ///
    /// Realtime-callable. Fails with
    /// [`HResult::APOERR_NOT_LOCKED`] when the cell is not
    /// currently `Locked`; on success returns whatever
    /// [`BufferFlags`] the user reports.
    pub fn process(
        &self,
        rt: &RealtimeContext,
        input: ProcessInput<'_>,
        output: &mut [f32],
    ) -> Result<BufferFlags, HResult> {
        if !self.state.is_locked() {
            return Err(HResult::APOERR_NOT_LOCKED);
        }
        // Safety: state == Locked and the host serialises process
        // against lock/unlock. No allocation, no kernel calls in
        // this dispatch.
        let inner = unsafe { &mut *self.inner.get() };
        Ok(inner.process(rt, input, output))
    }
}

impl<T: ProcessingObject> Default for ApoInstance<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apo::{ApoCategory, ProcessInput, ProcessingObject};
    use crate::buffer::BufferFlags;
    use crate::clsid::Clsid;
    use crate::error::HResult;
    use crate::format::Format;
    use crate::realtime::{RealtimeContext, State};
    use core::cell::Cell;
    use static_assertions::assert_impl_all;

    /// Reference Passthrough implementor. Carries a Cell so that
    /// tests can observe whether each lifecycle hook fired.
    struct Trace {
        lock_seen: Cell<Option<(u32, u32)>>, // (input rate, output rate)
        unlock_seen: Cell<u32>,
        process_seen: Cell<u32>,
        lock_should_fail: Cell<bool>,
    }

    impl ProcessingObject for Trace {
        const CLSID: Clsid = Clsid::from_u128(0x01234567_89AB_CDEF_0123_456789ABCDEF);
        const NAME: &'static str = "tympan-apo trace";
        const COPYRIGHT: &'static str = "test fixture";
        const CATEGORY: ApoCategory = ApoCategory::Sfx;

        fn new() -> Self {
            Self {
                lock_seen: Cell::new(None),
                unlock_seen: Cell::new(0),
                process_seen: Cell::new(0),
                lock_should_fail: Cell::new(false),
            }
        }

        fn lock_for_process(&mut self, input: &Format, output: &Format) -> Result<(), HResult> {
            if self.lock_should_fail.get() {
                return Err(HResult::APOERR_FORMAT_NOT_SUPPORTED);
            }
            self.lock_seen
                .set(Some((input.sample_rate(), output.sample_rate())));
            Ok(())
        }

        fn unlock_for_process(&mut self) {
            self.unlock_seen.set(self.unlock_seen.get() + 1);
        }

        fn process(
            &mut self,
            _rt: &RealtimeContext,
            input: ProcessInput<'_>,
            output: &mut [f32],
        ) -> BufferFlags {
            self.process_seen.set(self.process_seen.get() + 1);
            output.copy_from_slice(input.samples());
            input.flags()
        }
    }

    assert_impl_all!(ApoInstance<Trace>: Sync);

    fn rt() -> RealtimeContext {
        // The realtime witness can be constructed in tests via
        // the crate-private new_unchecked path. Pure logic tests
        // do not depend on the real audio-thread guarantees.
        unsafe { RealtimeContext::new_unchecked() }
    }

    #[test]
    fn new_starts_uninitialized_with_zero_refcount() {
        let apo = ApoInstance::<Trace>::new();
        assert_eq!(apo.state(), State::Uninitialized);
        assert_eq!(apo.refcount(), 0);
    }

    #[test]
    fn default_matches_new() {
        let apo: ApoInstance<Trace> = ApoInstance::default();
        assert_eq!(apo.state(), State::Uninitialized);
        assert_eq!(apo.refcount(), 0);
    }

    #[test]
    fn add_ref_release_delegate_to_refcount() {
        let apo = ApoInstance::<Trace>::new();
        assert_eq!(apo.add_ref(), 1);
        assert_eq!(apo.add_ref(), 2);
        assert_eq!(apo.refcount(), 2);
        assert_eq!(apo.release(), 1);
        assert_eq!(apo.release(), 0);
    }

    #[test]
    fn initialize_transitions_to_initialized() {
        let apo = ApoInstance::<Trace>::new();
        assert!(apo.initialize().is_ok());
        assert_eq!(apo.state(), State::Initialized);
    }

    #[test]
    fn double_initialize_returns_apoerr_already_locked() {
        let apo = ApoInstance::<Trace>::new();
        apo.initialize().unwrap();
        assert_eq!(apo.initialize(), Err(HResult::APOERR_ALREADY_LOCKED));
    }

    #[test]
    fn lock_requires_initialized() {
        let apo = ApoInstance::<Trace>::new();
        let f = Format::pcm_float32(48_000, 1);
        assert_eq!(
            apo.lock_for_process(&f, &f),
            Err(HResult::APOERR_NOT_LOCKED)
        );
        assert_eq!(apo.state(), State::Uninitialized);
    }

    #[test]
    fn lock_for_process_transitions_and_forwards_to_user() {
        let apo = ApoInstance::<Trace>::new();
        apo.initialize().unwrap();
        let input = Format::pcm_float32(48_000, 1);
        let output = Format::pcm_float32(44_100, 2);
        apo.lock_for_process(&input, &output).unwrap();
        assert_eq!(apo.state(), State::Locked);

        // The Trace inner observed the formats verbatim.
        let trace = unsafe { &*apo.inner.get() };
        assert_eq!(trace.lock_seen.get(), Some((48_000, 44_100)));
    }

    #[test]
    fn lock_failure_rolls_state_back_to_initialized() {
        let apo = ApoInstance::<Trace>::new();
        apo.initialize().unwrap();

        // Arm the failure mode.
        unsafe { &*apo.inner.get() }.lock_should_fail.set(true);

        let f = Format::pcm_float32(48_000, 1);
        assert_eq!(
            apo.lock_for_process(&f, &f),
            Err(HResult::APOERR_FORMAT_NOT_SUPPORTED)
        );
        // State machine rolled back, host can retry.
        assert_eq!(apo.state(), State::Initialized);
    }

    #[test]
    fn unlock_for_process_returns_to_initialized() {
        let apo = ApoInstance::<Trace>::new();
        apo.initialize().unwrap();
        let f = Format::pcm_float32(48_000, 1);
        apo.lock_for_process(&f, &f).unwrap();
        apo.unlock_for_process().unwrap();
        assert_eq!(apo.state(), State::Initialized);

        let trace = unsafe { &*apo.inner.get() };
        assert_eq!(trace.unlock_seen.get(), 1);
    }

    #[test]
    fn unlock_without_lock_fails() {
        let apo = ApoInstance::<Trace>::new();
        assert_eq!(apo.unlock_for_process(), Err(HResult::APOERR_NOT_LOCKED));
    }

    #[test]
    fn process_requires_locked_state() {
        let apo = ApoInstance::<Trace>::new();
        let samples = [0.0_f32; 4];
        let mut output = [0.0_f32; 4];
        let rt = rt();
        let result = apo.process(
            &rt,
            ProcessInput::new(&samples, BufferFlags::VALID),
            &mut output,
        );
        assert_eq!(result, Err(HResult::APOERR_NOT_LOCKED));
    }

    #[test]
    fn process_after_lock_returns_user_flags_and_copies_samples() {
        let apo = ApoInstance::<Trace>::new();
        apo.initialize().unwrap();
        let f = Format::pcm_float32(48_000, 1);
        apo.lock_for_process(&f, &f).unwrap();

        let samples = [0.1_f32, -0.2, 0.3, -0.4];
        let mut output = [0.0_f32; 4];
        let rt = rt();
        let out = apo
            .process(
                &rt,
                ProcessInput::new(&samples, BufferFlags::SILENT),
                &mut output,
            )
            .unwrap();
        assert_eq!(out, BufferFlags::SILENT);
        assert_eq!(output, samples);

        let trace = unsafe { &*apo.inner.get() };
        assert_eq!(trace.process_seen.get(), 1);
    }

    #[test]
    fn full_lifecycle_round_trip() {
        let apo = ApoInstance::<Trace>::new();
        apo.initialize().unwrap();
        let f = Format::pcm_float32(48_000, 1);
        apo.lock_for_process(&f, &f).unwrap();

        let samples = [0.5_f32; 4];
        let mut output = [0.0_f32; 4];
        let rt = rt();
        for _ in 0..3 {
            apo.process(
                &rt,
                ProcessInput::new(&samples, BufferFlags::VALID),
                &mut output,
            )
            .unwrap();
        }
        apo.unlock_for_process().unwrap();
        assert_eq!(apo.state(), State::Initialized);

        // Lock-process-unlock can repeat.
        apo.lock_for_process(&f, &f).unwrap();
        apo.unlock_for_process().unwrap();
        assert_eq!(apo.state(), State::Initialized);

        let trace = unsafe { &*apo.inner.get() };
        assert_eq!(trace.process_seen.get(), 3);
        assert_eq!(trace.unlock_seen.get(), 2);
    }

    #[test]
    fn is_input_format_supported_uses_user_default() {
        let apo = ApoInstance::<Trace>::new();
        let f = Format::pcm_float32(48_000, 1);
        assert_eq!(
            apo.is_input_format_supported(&f),
            crate::format::FormatNegotiation::Accept
        );
        let f = Format::pcm_int16(48_000, 1);
        match apo.is_input_format_supported(&f) {
            crate::format::FormatNegotiation::Suggest(s) => {
                assert!(s.is_float());
                assert_eq!(s.bits_per_sample(), 32);
            }
            other => panic!("expected Suggest, got {other:?}"),
        }
    }
}
