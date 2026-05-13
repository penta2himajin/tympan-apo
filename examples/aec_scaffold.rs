//! Reference AEC APO scaffold: copies the microphone input
//! straight to the output (no actual echo cancellation) but wires
//! up the full AEC interface surface so the audio engine can pick
//! it up as an AEC APO candidate.
//!
//! This example is the AEC counterpart of `passthrough.rs`. It
//! demonstrates the `AecProcessingObject` trait and the
//! `register_aec_apo!` macro; real AEC implementations would store
//! the loopback signal from [`AecScaffold::accept_aux_input`] and
//! cancel it in [`AecScaffold::process`].
//!
//! ## CLSID
//!
//! `{3D5C9E2D-3D2C-4E89-9C1A-6A6F40C92E13}` — sibling of the
//! passthrough/gain CLSIDs so Tier 3 CI can register it under a
//! stable key.
//!
//! ## Build
//!
//! ```bash
//! cargo build --release --target x86_64-pc-windows-msvc \
//!     --features aec --example aec_scaffold
//! ```
//!
//! Requires the `aec` cargo feature. The framework's `aec` module
//! is gated on the feature, so `register_aec_apo!` only resolves
//! when the feature is enabled.

#![cfg(all(windows, feature = "aec"))]
// `register_aec_apo!` emits a `pub static` and the four `Dll*`
// `#[no_mangle]` entry points at the crate root; examples build
// as cdylibs so the unmangled symbols are exactly what we want.
#![allow(missing_docs)]

use tympan_apo::aec::{AecProcessingObject, AuxiliaryInputBuffer};
use tympan_apo::error::HResult;
use tympan_apo::realtime::RealtimeContext;
use tympan_apo::{ApoCategory, BufferFlags, Clsid, Format, ProcessInput, ProcessingObject};

/// Reference AEC APO: passthrough on the primary input, no
/// cancellation; the aux-input hooks just count calls so the
/// example demonstrates the trait surface.
pub struct AecScaffold {
    /// Trivial counter for `accept_aux_input` invocations. Real
    /// AEC implementations would store the loopback samples here.
    aux_calls: u32,
}

impl ProcessingObject for AecScaffold {
    const CLSID: Clsid = Clsid::from_u128(0x3D5C9E2D_3D2C_4E89_9C1A_6A6F40C92E13);
    const NAME: &'static str = "tympan-apo aec scaffold";
    const COPYRIGHT: &'static str = "tympan-apo example";
    // AEC APOs sit in the MFX slot of the engine's capture
    // pipeline, processing per-endpoint per-mode audio.
    const CATEGORY: ApoCategory = ApoCategory::Mfx;

    fn new() -> Self {
        Self { aux_calls: 0 }
    }

    fn process(
        &mut self,
        _rt: &RealtimeContext,
        input: ProcessInput<'_>,
        output: &mut [f32],
    ) -> BufferFlags {
        // Passthrough — a real AEC would subtract the cached
        // reference signal from the input here.
        output.copy_from_slice(input.samples());
        input.flags()
    }
}

impl AecProcessingObject for AecScaffold {
    fn add_aux_input(
        &mut self,
        _id: u32,
        _format: &Format,
        _init_data: &[u8],
    ) -> Result<(), HResult> {
        // Real AEC would allocate per-aux-input state here. We
        // accept any incoming aux input.
        Ok(())
    }

    fn remove_aux_input(&mut self, _id: u32) {
        // No-op for the scaffold.
    }

    fn accept_aux_input(&mut self, _rt: &RealtimeContext, _input: AuxiliaryInputBuffer<'_>) {
        // Count the engine's deliveries of the reference signal.
        // Realtime-safe: pure scalar increment, no allocation.
        self.aux_calls = self.aux_calls.wrapping_add(1);
    }
}

tympan_apo::register_aec_apo!(AecScaffold);
