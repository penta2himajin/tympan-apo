//! Reference APO: applies a fixed linear gain to every sample.
//!
//! Demonstrates how to extend the framework beyond the trivial
//! [`passthrough`](crate::passthrough) by retaining per-instance
//! processing state. The gain factor is stored in the `Gain` struct
//! and applied in-place during [`Gain::process`].
//!
//! ## CLSID
//!
//! `{2A6F3F19-3D2C-4E89-9C1A-6A6F40C92E12}` — sibling of the
//! passthrough CLSID, fixed so Tier 3 CI registers it under a
//! stable key.
//!
//! ## Build
//!
//! ```bash
//! cargo build --release --target x86_64-pc-windows-msvc \
//!     --example gain
//! ```
//!
//! The output lands at
//! `target/x86_64-pc-windows-msvc/release/examples/gain.dll`.
//!
//! ## Realtime considerations
//!
//! Multiplying by a scalar in [`Gain::process`] is allocation-free
//! and lock-free — the framework's realtime invariants are
//! preserved. The gain factor itself is a `f32` field on the APO
//! instance; reading it on the realtime path is a single load.
//!
//! For dynamic gain control surface this through
//! [`ProcessingObject::set_system_effect_state`](tympan_apo::ProcessingObject::set_system_effect_state)
//! or wire the value through an SPSC ring from a non-realtime
//! controller thread.

#![cfg(windows)]
// `register_apo!` emits a `pub static` and several `#[no_mangle]`
// extern functions at the crate root. Examples build as cdylibs
// here, so unmangled symbols are exactly what we want.
#![allow(missing_docs)]

use tympan_apo::realtime::RealtimeContext;
use tympan_apo::{ApoCategory, BufferFlags, Clsid, ProcessInput, ProcessingObject};

/// Reference APO that multiplies every sample by a fixed gain.
///
/// The gain factor is `0.5` (-6 dB) — quiet enough to be audibly
/// distinct from passthrough without overflow risk.
pub struct Gain {
    gain: f32,
}

impl ProcessingObject for Gain {
    const CLSID: Clsid = Clsid::from_u128(0x2A6F3F19_3D2C_4E89_9C1A_6A6F40C92E12);
    const NAME: &'static str = "tympan-apo gain";
    const COPYRIGHT: &'static str = "tympan-apo example";
    const CATEGORY: ApoCategory = ApoCategory::Sfx;

    fn new() -> Self {
        Self { gain: 0.5 }
    }

    fn process(
        &mut self,
        _rt: &RealtimeContext,
        input: ProcessInput<'_>,
        output: &mut [f32],
    ) -> BufferFlags {
        // Vector-style: write each scaled sample to the corresponding
        // output slot. Compiler autovectorises this on `x86_64-pc-windows-msvc`
        // with SSE2 (the default for the target).
        let g = self.gain;
        for (out, &inp) in output.iter_mut().zip(input.samples().iter()) {
            *out = inp * g;
        }
        input.flags()
    }
}

tympan_apo::register_apo!(Gain);
