//! Atomic state machine for the APO lifecycle.
//!
//! The Windows audio engine drives an APO through a small finite
//! lifecycle:
//!
//! ```text
//!  Uninitialized ──Initialize──▶ Initialized ──LockForProcess──▶ Locked
//!                                    ▲                              │
//!                                    └───────UnlockForProcess───────┘
//! ```
//!
//! `APOProcess` is callable only while the state is [`State::Locked`].
//! Several of the framework's invariants (e.g. "buffer geometry is
//! pinned during processing") follow from the host obeying this
//! ordering, but the framework still verifies it: every transition
//! goes through [`StateCell`]'s atomic CAS, and bad transitions
//! surface as [`TransitionError`] rather than silently corrupting
//! state.

use core::sync::atomic::{AtomicU8, Ordering};

/// APO lifecycle state.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum State {
    /// Object exists but `Initialize` has not yet been called.
    Uninitialized = 0,
    /// `Initialize` has succeeded; `LockForProcess` has not been
    /// called (or has been undone by `UnlockForProcess`).
    Initialized = 1,
    /// `LockForProcess` has succeeded; `APOProcess` is callable.
    Locked = 2,
}

impl State {
    /// Returns `true` if `APOProcess` is legal in this state.
    #[inline]
    #[must_use]
    pub const fn allows_process(self) -> bool {
        matches!(self, State::Locked)
    }

    #[inline]
    const fn from_u8(value: u8) -> Self {
        // Safety: writers go through `StateCell`, which only stores
        // values produced by `State as u8`. The match is exhaustive
        // for the legal byte set; an unreachable arm guards against
        // future variants reaching this fn without the match being
        // updated.
        match value {
            0 => State::Uninitialized,
            1 => State::Initialized,
            2 => State::Locked,
            _ => panic!("StateCell stored an invalid State byte"),
        }
    }
}

/// Outcome of a failed transition.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct TransitionError {
    /// The state the caller assumed the cell was in.
    pub expected: State,
    /// The state the caller tried to transition into.
    pub attempted: State,
    /// The state actually observed in the cell.
    pub actual: State,
}

/// Atomic carrier for [`State`].
///
/// Holds the current lifecycle state and serialises transitions via
/// `compare_exchange`. Cheap to load from the realtime path
/// ([`Self::load`] is an `Acquire` load, no allocation, no kernel
/// involvement), and safe to share between threads.
#[derive(Debug)]
pub struct StateCell {
    state: AtomicU8,
}

impl StateCell {
    /// Construct a fresh cell in [`State::Uninitialized`].
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(State::Uninitialized as u8),
        }
    }

    /// Load the current state with `Acquire` ordering. Safe to call
    /// from the realtime path.
    #[inline]
    #[must_use]
    pub fn load(&self) -> State {
        State::from_u8(self.state.load(Ordering::Acquire))
    }

    /// `true` iff [`Self::load`] currently returns [`State::Locked`].
    #[inline]
    #[must_use]
    pub fn is_locked(&self) -> bool {
        self.load() == State::Locked
    }

    /// Transition `Uninitialized → Initialized`.
    #[inline]
    pub fn initialize(&self) -> Result<(), TransitionError> {
        self.transition(State::Uninitialized, State::Initialized)
    }

    /// Transition `Initialized → Locked`.
    #[inline]
    pub fn lock(&self) -> Result<(), TransitionError> {
        self.transition(State::Initialized, State::Locked)
    }

    /// Transition `Locked → Initialized`.
    #[inline]
    pub fn unlock(&self) -> Result<(), TransitionError> {
        self.transition(State::Locked, State::Initialized)
    }

    /// Unconditional reset to [`State::Uninitialized`]. Returns the
    /// state observed before the reset. Used by COM `Release` once
    /// the final reference is dropped, and by destructors that need
    /// to put the cell into a known terminal state regardless of
    /// what state it was in.
    #[inline]
    #[must_use]
    pub fn release(&self) -> State {
        State::from_u8(
            self.state
                .swap(State::Uninitialized as u8, Ordering::AcqRel),
        )
    }

    fn transition(&self, from: State, to: State) -> Result<(), TransitionError> {
        match self
            .state
            .compare_exchange(from as u8, to as u8, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => Ok(()),
            Err(actual_byte) => Err(TransitionError {
                expected: from,
                attempted: to,
                actual: State::from_u8(actual_byte),
            }),
        }
    }
}

impl Default for StateCell {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use static_assertions::assert_impl_all;

    assert_impl_all!(StateCell: Send, Sync);

    #[test]
    fn new_cell_starts_uninitialized() {
        let s = StateCell::new();
        assert_eq!(s.load(), State::Uninitialized);
        assert!(!s.is_locked());
    }

    #[test]
    fn default_matches_new() {
        assert_eq!(StateCell::default().load(), State::Uninitialized);
    }

    #[test]
    fn initialize_uninitialized_to_initialized() {
        let s = StateCell::new();
        assert!(s.initialize().is_ok());
        assert_eq!(s.load(), State::Initialized);
    }

    #[test]
    fn initialize_when_already_initialized_errors() {
        let s = StateCell::new();
        s.initialize().unwrap();
        let err = s.initialize().unwrap_err();
        assert_eq!(err.expected, State::Uninitialized);
        assert_eq!(err.attempted, State::Initialized);
        assert_eq!(err.actual, State::Initialized);
    }

    #[test]
    fn lock_requires_initialized() {
        let s = StateCell::new();
        let err = s.lock().unwrap_err();
        assert_eq!(err.expected, State::Initialized);
        assert_eq!(err.attempted, State::Locked);
        assert_eq!(err.actual, State::Uninitialized);

        s.initialize().unwrap();
        assert!(s.lock().is_ok());
        assert!(s.is_locked());
        assert!(s.load().allows_process());
    }

    #[test]
    fn lock_when_already_locked_errors() {
        let s = StateCell::new();
        s.initialize().unwrap();
        s.lock().unwrap();
        let err = s.lock().unwrap_err();
        assert_eq!(err.actual, State::Locked);
    }

    #[test]
    fn unlock_requires_locked() {
        let s = StateCell::new();
        let err = s.unlock().unwrap_err();
        assert_eq!(err.expected, State::Locked);
        assert_eq!(err.attempted, State::Initialized);
        assert_eq!(err.actual, State::Uninitialized);
    }

    #[test]
    fn unlock_returns_to_initialized() {
        let s = StateCell::new();
        s.initialize().unwrap();
        s.lock().unwrap();
        s.unlock().unwrap();
        assert_eq!(s.load(), State::Initialized);
    }

    #[test]
    fn release_from_any_state_resets() {
        let s = StateCell::new();
        assert_eq!(s.release(), State::Uninitialized);

        s.initialize().unwrap();
        assert_eq!(s.release(), State::Initialized);
        assert_eq!(s.load(), State::Uninitialized);

        s.initialize().unwrap();
        s.lock().unwrap();
        assert_eq!(s.release(), State::Locked);
        assert_eq!(s.load(), State::Uninitialized);
    }

    #[test]
    fn full_lifecycle_round_trip() {
        let s = StateCell::new();
        s.initialize().unwrap();
        s.lock().unwrap();
        s.unlock().unwrap();
        s.lock().unwrap();
        s.unlock().unwrap();
        let prior = s.release();
        assert_eq!(prior, State::Initialized);
        assert_eq!(s.load(), State::Uninitialized);
    }

    #[test]
    fn allows_process_only_in_locked() {
        assert!(!State::Uninitialized.allows_process());
        assert!(!State::Initialized.allows_process());
        assert!(State::Locked.allows_process());
    }

    #[test]
    fn concurrent_initialize_has_exactly_one_winner() {
        use std::sync::atomic::AtomicUsize;
        use std::sync::Arc;
        use std::thread;

        const THREADS: usize = 8;
        let s = Arc::new(StateCell::new());
        let wins = Arc::new(AtomicUsize::new(0));

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let s = Arc::clone(&s);
                let wins = Arc::clone(&wins);
                thread::spawn(move || {
                    if s.initialize().is_ok() {
                        wins.fetch_add(1, Ordering::Relaxed);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(wins.load(Ordering::Relaxed), 1);
        assert_eq!(s.load(), State::Initialized);
    }
}
