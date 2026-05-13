//! Lock-free single-producer / single-consumer ring buffer.
//!
//! The ring is intended for handing data between a non-realtime
//! producer (e.g. a worker thread draining diagnostic events) and a
//! realtime consumer (e.g. the `APOProcess` callback), or vice
//! versa. Both [`Producer::try_push`] and [`Consumer::try_pop`] are
//! wait-free, allocation-free, and never call into the kernel.
//!
//! ## Invariants
//!
//! - The ring permits exactly one producer thread and one consumer
//!   thread. The [`Producer`] and [`Consumer`] handles are `Send`
//!   but not `Sync`, so calling `try_push` from two threads
//!   simultaneously is a type-system error.
//! - Capacity is fixed at construction. Allocation happens once,
//!   inside [`spsc`]; afterwards the ring is heap-touch-free.
//! - Indices are unbounded `usize`s reduced modulo capacity only
//!   when indexing the storage, so the classic "full vs empty"
//!   ambiguity does not need a sacrificial slot.

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crossbeam_utils::CachePadded;

struct Shared<T> {
    buf: Box<[UnsafeCell<MaybeUninit<T>>]>,
    capacity: usize,
    // Producer writes `tail`, reads `head`. Consumer writes
    // `head`, reads `tail`. Separating them onto distinct cache
    // lines avoids false sharing between the two threads.
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
}

// Safety: the ring is only accessed through `Producer` and
// `Consumer`. Each side touches only one of the two atomics for
// writes, and the synchronisation between them is established via
// the Acquire/Release ordering on `head`/`tail`.
unsafe impl<T: Send> Send for Shared<T> {}
unsafe impl<T: Send> Sync for Shared<T> {}

/// Producer-side handle.
///
/// Owned by a single thread; can be sent across threads but not
/// shared by reference.
pub struct Producer<T> {
    shared: Arc<Shared<T>>,
    /// `*const ()` makes the handle `!Send` and `!Sync` by default.
    /// We then opt back into `Send` with a manual impl below; the
    /// lack of `Sync` is intentional and load-bearing for the SPSC
    /// invariant.
    _not_sync: PhantomData<*const ()>,
}

/// Consumer-side handle.
///
/// Owned by a single thread; can be sent across threads but not
/// shared by reference.
pub struct Consumer<T> {
    shared: Arc<Shared<T>>,
    _not_sync: PhantomData<*const ()>,
}

// We override `PhantomData<*const ()>`'s default `!Send` so that the
// handle can move between threads — the SPSC contract permits that.
// Sync is *not* re-implemented.
unsafe impl<T: Send> Send for Producer<T> {}
unsafe impl<T: Send> Send for Consumer<T> {}

impl<T> Producer<T> {
    /// Push a value into the ring. Returns `Err(value)` if the
    /// ring is full and the value could not be enqueued.
    ///
    /// Wait-free; safe to call from a realtime thread.
    #[inline]
    pub fn try_push(&self, value: T) -> Result<(), T> {
        let shared = &*self.shared;
        let tail = shared.tail.load(Ordering::Relaxed);
        let head = shared.head.load(Ordering::Acquire);
        if tail.wrapping_sub(head) == shared.capacity {
            return Err(value);
        }
        let idx = tail % shared.capacity;
        // Safety: the SPSC contract guarantees we are the sole
        // writer, and the index is exclusive to us until we
        // advance `tail` below.
        unsafe {
            (*shared.buf[idx].get()).write(value);
        }
        shared.tail.store(tail.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    /// Maximum number of in-flight items the ring can hold.
    #[inline]
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.shared.capacity
    }

    /// Returns `true` when [`Self::try_push`] would currently fail.
    ///
    /// May return stale information if the consumer pops a slot
    /// between this call and a subsequent `try_push`. The only
    /// guarantee is that if `is_full` returns `false`, the next
    /// `try_push` from this producer cannot fail.
    #[inline]
    #[must_use]
    pub fn is_full(&self) -> bool {
        let shared = &*self.shared;
        let tail = shared.tail.load(Ordering::Relaxed);
        let head = shared.head.load(Ordering::Acquire);
        tail.wrapping_sub(head) == shared.capacity
    }
}

impl<T> Consumer<T> {
    /// Pop a value from the ring. Returns `None` if the ring is
    /// currently empty.
    ///
    /// Wait-free; safe to call from a realtime thread.
    #[inline]
    #[must_use]
    pub fn try_pop(&self) -> Option<T> {
        let shared = &*self.shared;
        let head = shared.head.load(Ordering::Relaxed);
        let tail = shared.tail.load(Ordering::Acquire);
        if head == tail {
            return None;
        }
        let idx = head % shared.capacity;
        // Safety: the SPSC contract guarantees we are the sole
        // reader, and the slot was published by the producer's
        // Release on `tail` paired with our Acquire above.
        let value = unsafe { (*shared.buf[idx].get()).assume_init_read() };
        shared.head.store(head.wrapping_add(1), Ordering::Release);
        Some(value)
    }

    /// Maximum number of in-flight items the ring can hold.
    #[inline]
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.shared.capacity
    }

    /// Returns `true` when [`Self::try_pop`] would currently fail.
    ///
    /// May return stale information if the producer pushes a value
    /// between this call and a subsequent `try_pop`. The only
    /// guarantee is that if `is_empty` returns `false`, the next
    /// `try_pop` from this consumer cannot return `None`.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let shared = &*self.shared;
        let head = shared.head.load(Ordering::Relaxed);
        let tail = shared.tail.load(Ordering::Acquire);
        head == tail
    }
}

impl<T> Drop for Shared<T> {
    fn drop(&mut self) {
        // Drain any items still in the ring so their destructors
        // run. After `Producer` and `Consumer` are both dropped we
        // have unique access via `&mut self`, so the relaxed
        // loads are sufficient.
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Relaxed);
        let mut i = head;
        while i != tail {
            let idx = i % self.capacity;
            // Safety: the slot from `head..tail` is initialised
            // per the SPSC invariant.
            unsafe {
                (*self.buf[idx].get()).assume_init_drop();
            }
            i = i.wrapping_add(1);
        }
    }
}

/// Create a new SPSC ring with `capacity` slots.
///
/// Allocates `capacity` slots of backing storage on the heap. The
/// returned handles do not allocate further on their own.
///
/// # Panics
///
/// Panics if `capacity` is zero. A zero-capacity ring cannot hold
/// any items and is rejected at construction so realtime callers
/// do not have to defend against the degenerate case.
#[must_use]
pub fn spsc<T>(capacity: usize) -> (Producer<T>, Consumer<T>) {
    assert!(capacity > 0, "SPSC ring capacity must be non-zero");

    let mut buf = Vec::with_capacity(capacity);
    for _ in 0..capacity {
        buf.push(UnsafeCell::new(MaybeUninit::uninit()));
    }

    let shared = Arc::new(Shared {
        buf: buf.into_boxed_slice(),
        capacity,
        head: CachePadded::new(AtomicUsize::new(0)),
        tail: CachePadded::new(AtomicUsize::new(0)),
    });

    (
        Producer {
            shared: Arc::clone(&shared),
            _not_sync: PhantomData,
        },
        Consumer {
            shared,
            _not_sync: PhantomData,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use static_assertions::{assert_impl_all, assert_not_impl_any};

    // The ring is the bridge between exactly one producer thread
    // and exactly one consumer thread. Each handle must be `Send`
    // (it crosses thread boundaries) but neither may be `Sync`
    // (sharing a handle between threads would break SPSC).
    assert_impl_all!(Producer<u32>: Send);
    assert_impl_all!(Consumer<u32>: Send);
    assert_not_impl_any!(Producer<u32>: Sync);
    assert_not_impl_any!(Consumer<u32>: Sync);

    #[test]
    fn new_ring_is_empty_and_not_full() {
        let (p, c) = spsc::<u32>(4);
        assert!(c.is_empty());
        assert!(!p.is_full());
        assert_eq!(c.try_pop(), None);
    }

    #[test]
    fn push_then_pop_returns_same_value() {
        let (p, c) = spsc::<u32>(4);
        assert!(p.try_push(42).is_ok());
        assert_eq!(c.try_pop(), Some(42));
    }

    #[test]
    fn capacity_matches_constructor_argument() {
        let (p, c) = spsc::<u32>(8);
        assert_eq!(p.capacity(), 8);
        assert_eq!(c.capacity(), 8);
    }

    #[test]
    fn push_when_full_returns_back_the_value() {
        let (p, _c) = spsc::<u32>(2);
        assert!(p.try_push(1).is_ok());
        assert!(p.try_push(2).is_ok());
        assert!(p.is_full());
        assert_eq!(p.try_push(3), Err(3));
    }

    #[test]
    fn pop_when_empty_returns_none() {
        let (_p, c) = spsc::<u32>(2);
        assert_eq!(c.try_pop(), None);
    }

    #[test]
    fn fifo_order_is_preserved() {
        let (p, c) = spsc::<u32>(8);
        for i in 0..8 {
            assert!(p.try_push(i).is_ok());
        }
        for i in 0..8 {
            assert_eq!(c.try_pop(), Some(i));
        }
        assert!(c.is_empty());
    }

    #[test]
    fn ring_wraps_around_correctly() {
        // Capacity 3 ring. Push 2, pop 2, then push 3, pop 3. The
        // backing storage indices must wrap without losing values.
        let (p, c) = spsc::<u32>(3);
        assert!(p.try_push(1).is_ok());
        assert!(p.try_push(2).is_ok());
        assert_eq!(c.try_pop(), Some(1));
        assert_eq!(c.try_pop(), Some(2));

        assert!(p.try_push(3).is_ok());
        assert!(p.try_push(4).is_ok());
        assert!(p.try_push(5).is_ok());
        assert!(p.is_full());
        assert_eq!(c.try_pop(), Some(3));
        assert_eq!(c.try_pop(), Some(4));
        assert_eq!(c.try_pop(), Some(5));
        assert!(c.is_empty());
    }

    #[test]
    fn interleaved_push_and_pop_works() {
        let (p, c) = spsc::<u32>(2);
        for i in 0..100 {
            assert!(p.try_push(i).is_ok());
            assert_eq!(c.try_pop(), Some(i));
        }
        assert!(c.is_empty());
    }

    #[test]
    fn dropping_ring_drops_remaining_items() {
        // The destructor must run user-supplied Drop for any items
        // still in the ring. We observe this via a Drop counter.
        use std::sync::atomic::{AtomicUsize, Ordering};
        static DROPS: AtomicUsize = AtomicUsize::new(0);

        struct Counted;
        impl Drop for Counted {
            fn drop(&mut self) {
                DROPS.fetch_add(1, Ordering::Relaxed);
            }
        }

        DROPS.store(0, Ordering::Relaxed);
        {
            let (p, _c) = spsc::<Counted>(4);
            assert!(p.try_push(Counted).is_ok());
            assert!(p.try_push(Counted).is_ok());
            assert!(p.try_push(Counted).is_ok());
        }
        assert_eq!(DROPS.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn popped_items_are_not_double_dropped() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static DROPS: AtomicUsize = AtomicUsize::new(0);

        struct Counted;
        impl Drop for Counted {
            fn drop(&mut self) {
                DROPS.fetch_add(1, Ordering::Relaxed);
            }
        }

        DROPS.store(0, Ordering::Relaxed);
        {
            let (p, c) = spsc::<Counted>(4);
            assert!(p.try_push(Counted).is_ok());
            assert!(p.try_push(Counted).is_ok());
            // Pop one and let it drop here; one stays in the ring.
            let _popped = c.try_pop().unwrap();
        }
        assert_eq!(DROPS.load(Ordering::Relaxed), 2);
    }

    #[test]
    #[should_panic]
    fn zero_capacity_panics() {
        let _ = spsc::<u32>(0);
    }

    #[test]
    fn cross_thread_push_pop_preserves_order() {
        use std::thread;

        let (p, c) = spsc::<u32>(64);
        const N: u32 = 100_000;

        let producer = thread::spawn(move || {
            let mut next = 0;
            while next < N {
                if p.try_push(next).is_ok() {
                    next += 1;
                } else {
                    std::thread::yield_now();
                }
            }
        });

        let consumer = thread::spawn(move || {
            let mut expected = 0;
            while expected < N {
                match c.try_pop() {
                    Some(v) => {
                        assert_eq!(v, expected);
                        expected += 1;
                    }
                    None => std::thread::yield_now(),
                }
            }
        });

        producer.join().unwrap();
        consumer.join().unwrap();
    }
}
