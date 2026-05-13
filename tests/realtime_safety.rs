//! Mechanical enforcement of `CLAUDE.md` prohibition #1 —
//! `APOProcess` and everything reachable from it must be
//! allocation-free.
//!
//! This integration test installs
//! [`assert_no_alloc::AllocDisabler`] as the test crate's global
//! allocator and drives `ApoInstance<Passthrough>` through a full
//! lifecycle. The `process` calls themselves run inside an
//! [`assert_no_alloc::assert_no_alloc`] guard, which aborts the
//! test if any allocation traverses the global allocator hook
//! during that span.
//!
//! ## Why the rlib path
//!
//! `[tier3_lifecycle]` loads the *built* `passthrough.dll` and
//! drives the COM lifecycle through `LoadLibrary` —
//! `assert_no_alloc` is incompatible with that path because the
//! cdylib has its own `__rust_alloc` symbol independent of the
//! test crate. The rlib path here shares a single link unit with
//! the test, so the global allocator hook is observable from the
//! framework's `process` dispatch.
//!
//! [tier3_lifecycle]: ../tier3_lifecycle/index.html

#![cfg(windows)]

use assert_no_alloc::{assert_no_alloc, AllocDisabler};
use std::sync::Arc;

use tympan_apo::format::Format;
use tympan_apo::instance::{AnyApoInstance, ApoInstance};
use tympan_apo::realtime::RealtimeContext;
use tympan_apo::{ApoCategory, BufferFlags, Clsid, ProcessInput, ProcessingObject};

#[global_allocator]
static A: AllocDisabler = AllocDisabler;

struct Passthrough;

impl ProcessingObject for Passthrough {
    const CLSID: Clsid = Clsid::from_u128(0x1B7E5A4F_3D2C_4E89_9C1A_6A6F40C92E11);
    const NAME: &'static str = "tympan-apo passthrough (rlib)";
    const COPYRIGHT: &'static str = "tympan-apo test";
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

#[test]
fn process_dispatch_is_allocation_free() {
    let inst: Arc<dyn AnyApoInstance> = Arc::new(ApoInstance::<Passthrough>::new());
    inst.initialize().unwrap();

    let format = Format::pcm_float32(48_000, 1);
    inst.lock_for_process(&format, &format).unwrap();

    const FRAMES: usize = 256;
    let input: Vec<f32> = (0..FRAMES)
        .map(|i| (i as f32 / FRAMES as f32) * 2.0 - 1.0)
        .collect();
    let mut output: Vec<f32> = vec![0.0; FRAMES];

    // Safety: realtime witness is constructed via the crate's
    // crate-private `new_unchecked`; the framework's `process` path
    // requires it. This integration test is pure logic, not a real
    // audio-thread invocation.
    let rt = unsafe { RealtimeContext::new_unchecked() };

    const ITERATIONS: usize = 64;
    assert_no_alloc(|| {
        for _ in 0..ITERATIONS {
            let flags = inst
                .process(
                    &rt,
                    ProcessInput::new(&input, BufferFlags::VALID),
                    &mut output,
                )
                .unwrap();
            // Mirror the audio engine's behaviour of asserting the
            // returned flags inside the realtime span — any
            // allocation in that assertion path would itself fail
            // the test.
            assert_eq!(flags, BufferFlags::VALID);
        }
    });

    // Verify outside the assert_no_alloc guard so the assertion
    // helpers themselves are allowed to allocate.
    for (&s_in, &s_out) in input.iter().zip(output.iter()) {
        assert!(s_out.is_finite());
        assert_eq!(s_out.to_bits(), s_in.to_bits());
    }

    inst.unlock_for_process().unwrap();
}
