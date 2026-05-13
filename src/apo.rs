//! Top-level Audio Processing Object surface.
//!
//! This module will host the `ProcessingObject` trait and lifecycle
//! types that users implement. The trait itself, along with the
//! `process()` IO types, lands once the realtime IO buffer
//! abstractions exist; for now only the supporting enums are here.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_are_distinct() {
        assert_ne!(ApoCategory::Sfx, ApoCategory::Mfx);
        assert_ne!(ApoCategory::Mfx, ApoCategory::Efx);
        assert_ne!(ApoCategory::Sfx, ApoCategory::Efx);
    }
}
