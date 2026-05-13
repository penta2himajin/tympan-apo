//! Minimal reference APO: copies input samples straight to output.
//!
//! Builds as a Windows `cdylib`. The four standard COM in-process
//! server entry points (`DllGetClassObject`, `DllCanUnloadNow`,
//! `DllRegisterServer`, `DllUnregisterServer`) are emitted by
//! `tympan_apo::register_apo!` and resolve through the framework's
//! class factory.
//!
//! ## CLSID
//!
//! `{1B7E5A4F-3D2C-4E89-9C1A-6A6F40C92E11}` — fixed for the example
//! so Tier 3 CI can register it under a stable key.
//!
//! ## Build
//!
//! ```bash
//! cargo build --release --target x86_64-pc-windows-msvc \
//!     --example passthrough
//! ```
//!
//! The output lands at
//! `target/x86_64-pc-windows-msvc/release/examples/passthrough.dll`.
//!
//! ## Use as a template
//!
//! Replace [`Passthrough::process`] with the desired DSP. The
//! framework's default `ProcessingObject::is_input_format_supported`
//! / `is_output_format_supported` already steer the negotiation
//! toward IEEE float32 — the bit depth the `&[f32]` / `&mut [f32]`
//! buffer parameters assume.

#![cfg(windows)]
// `register_apo!` emits a `pub static` and several `#[no_mangle]`
// extern functions at the crate root. Examples build as cdylibs
// here, so unmangled symbols are exactly what we want.
#![allow(missing_docs)]

use tympan_apo::realtime::RealtimeContext;
use tympan_apo::{ApoCategory, BufferFlags, Clsid, ProcessInput, ProcessingObject};

/// Reference APO that emits its input verbatim.
pub struct Passthrough;

impl ProcessingObject for Passthrough {
    const CLSID: Clsid = Clsid::from_u128(0x1B7E5A4F_3D2C_4E89_9C1A_6A6F40C92E11);
    const NAME: &'static str = "tympan-apo passthrough";
    const COPYRIGHT: &'static str = "tympan-apo example";
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

tympan_apo::register_apo!(Passthrough);
