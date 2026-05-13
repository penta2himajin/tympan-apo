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
    /// The caller must guarantee that the current thread is the
    /// Windows audio engine's realtime thread (i.e. the call stack
    /// originated from `IAudioProcessingObjectRT::APOProcess`). The
    /// constructor is `pub(crate)` so that only the framework's COM
    /// harness can satisfy this contract; user code receives the
    /// witness by reference rather than constructing one.
    #[inline]
    #[allow(dead_code)] // wired up once the raw COM harness lands
    pub(crate) unsafe fn new_unchecked() -> Self {
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
