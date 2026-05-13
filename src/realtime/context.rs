//! Zero-sized witness for the realtime context.

use core::marker::PhantomData;

/// Compile-time witness that the current call stack originates from
/// the audio engine's realtime thread.
///
/// `RealtimeContext` is zero-sized and cannot be constructed outside
/// of this crate. The framework's `APOProcess` harness creates a
/// reference and passes it to user code; any function safe to call
/// from the realtime path takes `&RealtimeContext`. Functions that
/// require heap allocation, blocking syscalls, or other non-realtime
/// operations simply do not accept this parameter, and therefore
/// cannot be invoked from the realtime path.
///
/// The `PhantomData<*const ()>` field makes the type `!Send` and
/// `!Sync`: a realtime witness from one thread must not be smuggled
/// to another thread, where the assumption that the caller is on the
/// audio engine's realtime thread would no longer hold.
#[derive(Debug)]
pub struct RealtimeContext {
    _not_send_sync: PhantomData<*const ()>,
}

impl RealtimeContext {
    /// Construct a new realtime witness.
    ///
    /// # Safety
    ///
    /// The caller must guarantee one of the following holds:
    ///
    /// 1. The current thread is the Windows audio engine's realtime
    ///    thread, i.e. the call stack originated from
    ///    `IAudioProcessingObjectRT::APOProcess`. The framework's
    ///    COM harness invokes this constructor exactly in that
    ///    position; user code receives the witness by reference
    ///    rather than constructing one of its own.
    /// 2. The caller is an integration test or benchmark driving the
    ///    framework's `process` path in-process, with full knowledge
    ///    that allocation- and lock-free guarantees only apply to
    ///    code paths reachable from the audio thread; tests that
    ///    perform allocations outside the realtime span are free to.
    ///
    /// Holding a `&RealtimeContext` does not in itself prevent
    /// allocation — it makes the requirement explicit at every
    /// call site that takes one. Mechanical enforcement lives in
    /// `tests/realtime_safety.rs` via the `assert_no_alloc`
    /// global-allocator guard.
    #[inline]
    pub unsafe fn new_unchecked() -> Self {
        Self {
            _not_send_sync: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use static_assertions::{assert_not_impl_any, const_assert_eq};

    const_assert_eq!(core::mem::size_of::<RealtimeContext>(), 0);
    assert_not_impl_any!(RealtimeContext: Send, Sync);

    #[test]
    fn constructor_is_callable() {
        // Smoke test: the framework's harness creates a witness via
        // `new_unchecked` before invoking user code. We exercise the
        // same path here to keep the constructor exercised by tests.
        let _ctx = unsafe { RealtimeContext::new_unchecked() };
    }
}
