//! Top-level Audio Processing Object surface.
//!
//! Users of the framework implement [`ProcessingObject`] for a
//! type carrying the per-instance state of their APO. The
//! framework's COM harness will (in a follow-up PR) construct an
//! instance via [`ProcessingObject::new`], drive the lifecycle, and
//! forward audio buffers into [`ProcessingObject::process`].

use crate::buffer::BufferFlags;
use crate::clsid::Clsid;
use crate::error::HResult;
use crate::format::{Format, FormatNegotiation};
use crate::realtime::RealtimeContext;

/// Per-buffer input handed to [`ProcessingObject::process`].
///
/// Borrows an interleaved float32 sample buffer from the host and
/// carries the [`BufferFlags`] the host stamped on it. Both
/// fields are accessed through const fns so the wrapper is
/// allocation-free and realtime-safe.
#[derive(Copy, Clone, Debug)]
pub struct ProcessInput<'a> {
    samples: &'a [f32],
    flags: BufferFlags,
}

impl<'a> ProcessInput<'a> {
    /// Wrap a sample slice and the host's flag word.
    ///
    /// The framework's COM harness will construct one of these per
    /// `APOProcess` invocation; tests construct them directly.
    #[inline]
    #[must_use]
    pub const fn new(samples: &'a [f32], flags: BufferFlags) -> Self {
        Self { samples, flags }
    }

    /// Interleaved float32 samples — `frame_count * channel_count`
    /// elements long.
    #[inline]
    #[must_use]
    pub const fn samples(&self) -> &'a [f32] {
        self.samples
    }

    /// Flags the host stamped on this buffer.
    #[inline]
    #[must_use]
    pub const fn flags(&self) -> BufferFlags {
        self.flags
    }

    /// Convenience: `true` iff [`Self::flags`] is
    /// [`BufferFlags::SILENT`].
    #[inline]
    #[must_use]
    pub const fn is_silent(&self) -> bool {
        self.flags.is_silent()
    }
}

/// On/off state of a [`SystemEffect`].
///
/// Mirrors the Windows `AUDIO_SYSTEMEFFECT_STATE` enumeration:
/// `Off = 0`, `On = 1`. The framework converts between the two
/// representations at the COM boundary so the user-facing API
/// stays cross-platform.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Default)]
pub enum SystemEffectState {
    /// Effect is currently off; the user-side `process` may skip
    /// the per-effect work.
    Off,
    /// Effect is currently on; the user-side `process` should
    /// apply the effect normally.
    #[default]
    On,
}

/// One system effect this APO advertises to the audio engine via
/// `IAudioSystemEffects2::GetEffectsList` and (when this APO
/// supports per-effect toggling) `IAudioSystemEffects3::GetControllableSystemEffectsList`.
///
/// The Windows audio engine surfaces these in the Sound Settings
/// UI; users can see the effect by name (resolved via the
/// per-effect ID in the audio property store) and — for effects
/// marked `controllable` — toggle each independently.
///
/// The framework's default `ProcessingObject::system_effects`
/// returns an empty slice, so an APO advertises no enumerable
/// effects unless it overrides the method. That matches the
/// historical behaviour of a v1-only `IAudioSystemEffects` marker.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct SystemEffect {
    /// Unique identifier for this effect within the APO. The audio
    /// engine pairs this with the friendly name in the per-endpoint
    /// `AudioSystemEffects_PropertyStore` to render the UI.
    pub id: Clsid,
    /// `true` if the audio engine may call
    /// `SetAudioSystemEffectState` on this effect at runtime.
    /// `false` means the effect is always on and the Sound Settings
    /// UI hides the toggle.
    pub controllable: bool,
    /// Initial state of the effect, also surfaced through
    /// `GetControllableSystemEffectsList`.
    pub state: SystemEffectState,
}

impl SystemEffect {
    /// Construct an effect descriptor from its unique ID. Defaults
    /// to non-controllable, `On` state — the v1/v2 behaviour where
    /// effects are always-on markers in the discovery list.
    #[inline]
    #[must_use]
    pub const fn new(id: Clsid) -> Self {
        Self {
            id,
            controllable: false,
            state: SystemEffectState::On,
        }
    }

    /// Builder-style: mark this effect as user-controllable.
    #[inline]
    #[must_use]
    pub const fn with_controllable(mut self, controllable: bool) -> Self {
        self.controllable = controllable;
        self
    }

    /// Builder-style: set the initial state.
    #[inline]
    #[must_use]
    pub const fn with_state(mut self, state: SystemEffectState) -> Self {
        self.state = state;
        self
    }
}

/// Category of an Audio Processing Object, as exposed via
/// `IAudioSystemEffects` / `IAudioSystemEffects3`.
///
/// The audio engine instantiates an APO once per (endpoint, mode)
/// combination, and the category selects where in the per-stream
/// processing graph the APO sits.
///
/// - [`Self::Sfx`] — Stream Effect: per-application processing, runs
///   before the engine mixes streams together. Used for
///   per-application volume, ducking, or stream-specific effects.
/// - [`Self::Mfx`] — Mode Effect: per-endpoint, per-mode processing,
///   applied to the mixed stream for a specific audio mode
///   (Communications, Media, etc.).
/// - [`Self::Efx`] — Endpoint Effect: applied to the entire endpoint
///   regardless of mode. Inherently device-wide; ship with care.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub enum ApoCategory {
    /// Stream effect — per-application processing.
    Sfx,
    /// Mode effect — per-endpoint, per-mode processing.
    Mfx,
    /// Endpoint effect — per-endpoint, mode-agnostic processing.
    Efx,
}

/// User-implemented Audio Processing Object.
///
/// Each implementor represents one CLSID-identified APO with a
/// distinct name, category, and processing behaviour. The
/// framework's COM harness instantiates the type via
/// [`Self::new`], drives the format-negotiation /
/// `LockForProcess` / `APOProcess` / `UnlockForProcess` sequence,
/// and routes the audio engine's calls into the corresponding
/// trait methods.
///
/// ## Default format negotiation
///
/// The default [`Self::is_input_format_supported`] /
/// [`Self::is_output_format_supported`] implementations accept any
/// IEEE-float32 stream and suggest a float32 alternative for
/// anything else. This matches the canonical Windows audio engine
/// negotiation and is the format the [`Self::process`] callback's
/// `&[f32]` / `&mut [f32]` parameters assume.
///
/// Implementors that want to handle integer PCM or other formats
/// directly should override these methods and use [`Format`]'s
/// accessors to do their own typed slicing inside `process`.
///
/// ## Realtime safety
///
/// [`Self::process`] takes a [`RealtimeContext`] reference. Any
/// helper function callable from `process` should also accept
/// `&RealtimeContext`, which makes its presence in the call stack
/// visible at compile time. The realtime path must be
/// allocation-free and lock-free per `CLAUDE.md` prohibitions 1
/// and 2.
pub trait ProcessingObject: Sized + Send {
    /// CLSID under which the audio engine and `regsvr32` identify
    /// this APO. Must be unique per implementor.
    const CLSID: Clsid;

    /// Human-readable APO name. Surfaced in `Sound Settings` and
    /// elsewhere in the Windows audio UI.
    const NAME: &'static str;

    /// Copyright notice carried in the registered class metadata.
    const COPYRIGHT: &'static str;

    /// Category controlling where in the per-stream processing
    /// graph the APO sits — see [`ApoCategory`].
    const CATEGORY: ApoCategory;

    /// Construct a fresh APO instance.
    ///
    /// Called by the framework's class factory once per
    /// `CoCreateInstance` invocation from the audio engine. Heap
    /// allocation is allowed here; it is *not* allowed inside
    /// [`Self::process`].
    fn new() -> Self;

    /// Decide whether `format` is acceptable as an input format.
    ///
    /// The default implementation accepts any IEEE-float32 stream
    /// and suggests `pcm_float32(format.sample_rate(),
    /// format.channels())` otherwise.
    fn is_input_format_supported(&self, format: &Format) -> FormatNegotiation {
        default_float32_negotiation(format)
    }

    /// Decide whether `format` is acceptable as an output format.
    ///
    /// The default implementation mirrors
    /// [`Self::is_input_format_supported`].
    fn is_output_format_supported(&self, format: &Format) -> FormatNegotiation {
        default_float32_negotiation(format)
    }

    /// List of system effects this APO advertises to the audio
    /// engine via `IAudioSystemEffects2::GetEffectsList`.
    ///
    /// The default returns an empty slice — the APO is registered
    /// but exposes no granular effects in the Sound Settings UI.
    /// Implementors that want per-effect toggles should override
    /// this with a slice borrowed from `&self`.
    ///
    /// Called from non-realtime threads; allocation is permitted
    /// in implementations that need it (though the default's
    /// constant slice is allocation-free).
    fn system_effects(&self) -> &[SystemEffect] {
        &[]
    }

    /// Toggle the state of one of this APO's advertised effects.
    ///
    /// Called by the audio engine through
    /// `IAudioSystemEffects3::SetAudioSystemEffectState` whenever
    /// the user flips an effect toggle in the Sound Settings UI.
    /// The framework dispatches into this method only for effects
    /// the user advertised with `controllable: true`; if `id` does
    /// not match any advertised effect the framework returns
    /// `E_INVALIDARG` and does not invoke the method.
    ///
    /// The default implementation is a no-op. Implementors that
    /// want to react to state changes (skip processing when off,
    /// reload internal state, etc.) override this method.
    ///
    /// Called from a non-realtime thread and may race with the
    /// realtime `process` callback. Implementors that read effect
    /// state from `process` should mediate via atomics or a
    /// realtime-safe primitive.
    fn set_system_effect_state(&mut self, id: &Clsid, state: SystemEffectState) {
        let _ = (id, state);
    }

    /// Prepare for processing under the supplied input/output
    /// formats.
    ///
    /// Called once between `Initialize` and the first
    /// [`Self::process`] invocation. This is where implementors
    /// should pre-allocate internal buffers; allocation in
    /// [`Self::process`] is prohibited.
    ///
    /// Returning an [`HResult`] failure aborts lock and surfaces
    /// to the audio engine as an `IsInitialized=FALSE` state.
    fn lock_for_process(&mut self, input: &Format, output: &Format) -> Result<(), HResult> {
        let _ = (input, output);
        Ok(())
    }

    /// Release any resources acquired during
    /// [`Self::lock_for_process`].
    ///
    /// Always paired with a prior successful `lock_for_process`.
    /// Allocator use is allowed.
    fn unlock_for_process(&mut self) {}

    /// Process one audio buffer.
    ///
    /// Realtime-critical: must be allocation-free, lock-free, and
    /// must not call into the kernel. Reachable callees should
    /// take `&RealtimeContext` to make the constraint visible
    /// throughout the call graph.
    ///
    /// `input` carries the host's input samples and the
    /// [`BufferFlags`] the host stamped on the buffer (the APO is
    /// free to short-circuit when [`ProcessInput::is_silent`] is
    /// `true`). `output` is the interleaved float32 buffer to
    /// write into; the same length as `input.samples()` (the
    /// framework enforces this before dispatching).
    ///
    /// The return value becomes the `u32BufferFlags` field of the
    /// host's output `APO_CONNECTION_PROPERTY` — typically
    /// [`BufferFlags::VALID`] for normal audio, or
    /// [`BufferFlags::SILENT`] when the APO knows it wrote pure
    /// silence and the engine may skip downstream work.
    fn process(
        &mut self,
        rt: &RealtimeContext,
        input: ProcessInput<'_>,
        output: &mut [f32],
    ) -> BufferFlags;
}

#[inline]
fn default_float32_negotiation(format: &Format) -> FormatNegotiation {
    if format.is_float() && format.bits_per_sample() == 32 {
        FormatNegotiation::Accept
    } else {
        FormatNegotiation::Suggest(Format::pcm_float32(format.sample_rate(), format.channels()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference implementor used by the trait's unit tests.
    /// Copies input straight to output frame-by-frame.
    struct Passthrough;

    impl ProcessingObject for Passthrough {
        const CLSID: Clsid = Clsid::from_u128(0xCAFEBABE_DEAD_BEEF_1234_56789ABCDEF0);
        const NAME: &'static str = "tympan-apo passthrough";
        const COPYRIGHT: &'static str = "test fixture";
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
    fn variants_are_distinct() {
        assert_ne!(ApoCategory::Sfx, ApoCategory::Mfx);
        assert_ne!(ApoCategory::Mfx, ApoCategory::Efx);
        assert_ne!(ApoCategory::Sfx, ApoCategory::Efx);
    }

    #[test]
    fn associated_constants_round_trip() {
        assert_eq!(Passthrough::NAME, "tympan-apo passthrough");
        assert_eq!(Passthrough::COPYRIGHT, "test fixture");
        assert_eq!(Passthrough::CATEGORY, ApoCategory::Sfx);
        assert!(!Passthrough::CLSID.is_nil());
    }

    #[test]
    fn default_input_format_accepts_float32_at_any_rate_channels() {
        let apo = Passthrough::new();
        for (rate, ch) in [(48_000, 1), (44_100, 2), (96_000, 6), (192_000, 8)] {
            assert_eq!(
                apo.is_input_format_supported(&Format::pcm_float32(rate, ch)),
                FormatNegotiation::Accept,
                "float32 {rate} Hz × {ch} ch must be accepted",
            );
        }
    }

    #[test]
    fn default_input_format_suggests_float32_for_int_pcm() {
        let apo = Passthrough::new();
        for fmt in [
            Format::pcm_int16(48_000, 2),
            Format::pcm_int24(44_100, 1),
            Format::pcm_int32(96_000, 4),
        ] {
            match apo.is_input_format_supported(&fmt) {
                FormatNegotiation::Suggest(suggested) => {
                    assert!(suggested.is_float(), "suggestion must be float");
                    assert_eq!(suggested.bits_per_sample(), 32);
                    assert_eq!(suggested.sample_rate(), fmt.sample_rate());
                    assert_eq!(suggested.channels(), fmt.channels());
                }
                other => panic!("expected Suggest for {fmt:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn default_input_format_suggests_float32_for_float64() {
        // Even float-but-wrong-width formats must be steered to
        // float32.
        let apo = Passthrough::new();
        let f = Format::pcm_float64(48_000, 1);
        match apo.is_input_format_supported(&f) {
            FormatNegotiation::Suggest(s) => {
                assert!(s.is_float());
                assert_eq!(s.bits_per_sample(), 32);
            }
            other => panic!("expected Suggest, got {other:?}"),
        }
    }

    #[test]
    fn default_output_negotiation_matches_input() {
        let apo = Passthrough::new();
        for fmt in [
            Format::pcm_float32(48_000, 1),
            Format::pcm_int16(44_100, 2),
            Format::pcm_float64(96_000, 6),
        ] {
            assert_eq!(
                apo.is_input_format_supported(&fmt),
                apo.is_output_format_supported(&fmt),
            );
        }
    }

    #[test]
    fn default_lock_for_process_succeeds() {
        let mut apo = Passthrough::new();
        let fmt = Format::pcm_float32(48_000, 1);
        assert!(apo.lock_for_process(&fmt, &fmt).is_ok());
    }

    #[test]
    fn default_unlock_is_callable() {
        let mut apo = Passthrough::new();
        apo.unlock_for_process();
    }

    #[test]
    fn process_runs_against_a_synthetic_buffer() {
        // The realtime witness can be constructed in tests via
        // the crate-private `new_unchecked` constructor; this is
        // the only path that bypasses the contract, and it is
        // permitted here because the test exercises pure logic,
        // not realtime-thread-dependent behaviour.
        let mut apo = Passthrough::new();
        let samples = [0.1_f32, -0.2, 0.3, -0.4, 0.5, -0.6, 0.7, -0.8];
        let mut output = [0.0_f32; 8];
        let rt = unsafe { RealtimeContext::new_unchecked() };
        let out_flags = apo.process(
            &rt,
            ProcessInput::new(&samples, BufferFlags::VALID),
            &mut output,
        );
        assert_eq!(output, samples);
        assert_eq!(out_flags, BufferFlags::VALID);
    }

    #[test]
    fn process_input_exposes_samples_and_flags() {
        let samples = [1.0_f32, 2.0, 3.0];
        let input = ProcessInput::new(&samples, BufferFlags::SILENT);
        assert_eq!(input.samples(), &samples);
        assert_eq!(input.flags(), BufferFlags::SILENT);
        assert!(input.is_silent());
    }

    #[test]
    fn process_input_is_not_silent_when_flag_is_valid() {
        let samples = [0.0_f32];
        let input = ProcessInput::new(&samples, BufferFlags::VALID);
        assert!(!input.is_silent());
    }

    #[test]
    fn process_passes_through_input_flags() {
        // Passthrough's implementation returns the input flags
        // verbatim — verify each variant survives the round-trip.
        let mut apo = Passthrough::new();
        let rt = unsafe { RealtimeContext::new_unchecked() };
        let samples = [0.5_f32; 4];
        for f in [
            BufferFlags::VALID,
            BufferFlags::SILENT,
            BufferFlags::INVALID,
        ] {
            let mut output = [0.0_f32; 4];
            let out = apo.process(&rt, ProcessInput::new(&samples, f), &mut output);
            assert_eq!(out, f);
        }
    }
}
