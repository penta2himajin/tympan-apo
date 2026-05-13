//! PCM audio stream format and format negotiation.
//!
//! [`Format`] mirrors the fields of the Windows `WAVEFORMATEX`
//! structure but uses a plain Rust layout, so it is constructible
//! and inspectable from cross-platform code. Conversion routines to
//! and from the FFI structure live under `#[cfg(windows)]`.
//!
//! Only the `WAVEFORMATEX` subset of fields is exposed here. The
//! `WAVEFORMATEXTENSIBLE` extension (channel mask, sub-format GUID)
//! will be modelled in a follow-up once the public API surrounding
//! channel layouts is settled.

/// `WAVE_FORMAT_PCM` — integer PCM.
pub const WAVE_FORMAT_PCM: u16 = 0x0001;
/// `WAVE_FORMAT_IEEE_FLOAT` — IEEE 754 floating-point PCM.
pub const WAVE_FORMAT_IEEE_FLOAT: u16 = 0x0003;
/// `WAVE_FORMAT_EXTENSIBLE` — indicates that a
/// `WAVEFORMATEXTENSIBLE` structure follows the base `WAVEFORMATEX`.
pub const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

/// PCM audio stream format.
///
/// Holds the parameters that the audio engine negotiates with an
/// APO: format tag, channel count, sample rate, and bit depth. The
/// derived fields (`block_align`, `avg_bytes_per_sec`) are computed
/// at construction from the inputs.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Format {
    format_tag: u16,
    channels: u16,
    samples_per_sec: u32,
    avg_bytes_per_sec: u32,
    block_align: u16,
    bits_per_sample: u16,
}

impl Format {
    /// Construct a format directly from its fields.
    ///
    /// Prefer the typed constructors (`pcm_int16`, `pcm_float32`,
    /// ...) when modelling a standard PCM stream. This raw
    /// constructor is intended for round-tripping through
    /// `WAVEFORMATEX` and for tests.
    #[must_use]
    pub const fn from_raw(
        format_tag: u16,
        channels: u16,
        samples_per_sec: u32,
        bits_per_sample: u16,
    ) -> Self {
        let block_align = channels * (bits_per_sample / 8);
        let avg_bytes_per_sec = samples_per_sec * block_align as u32;
        Self {
            format_tag,
            channels,
            samples_per_sec,
            avg_bytes_per_sec,
            block_align,
            bits_per_sample,
        }
    }

    /// 16-bit signed integer PCM (`WAVE_FORMAT_PCM`).
    #[must_use]
    pub const fn pcm_int16(sample_rate: u32, channels: u16) -> Self {
        Self::from_raw(WAVE_FORMAT_PCM, channels, sample_rate, 16)
    }

    /// 24-bit signed integer PCM packed into 3-byte containers
    /// (`WAVE_FORMAT_PCM`).
    #[must_use]
    pub const fn pcm_int24(sample_rate: u32, channels: u16) -> Self {
        Self::from_raw(WAVE_FORMAT_PCM, channels, sample_rate, 24)
    }

    /// 32-bit signed integer PCM (`WAVE_FORMAT_PCM`).
    #[must_use]
    pub const fn pcm_int32(sample_rate: u32, channels: u16) -> Self {
        Self::from_raw(WAVE_FORMAT_PCM, channels, sample_rate, 32)
    }

    /// 32-bit IEEE float PCM (`WAVE_FORMAT_IEEE_FLOAT`). This is the
    /// canonical format negotiated between the Windows audio engine
    /// and most APOs.
    #[must_use]
    pub const fn pcm_float32(sample_rate: u32, channels: u16) -> Self {
        Self::from_raw(WAVE_FORMAT_IEEE_FLOAT, channels, sample_rate, 32)
    }

    /// 64-bit IEEE float PCM (`WAVE_FORMAT_IEEE_FLOAT`).
    #[must_use]
    pub const fn pcm_float64(sample_rate: u32, channels: u16) -> Self {
        Self::from_raw(WAVE_FORMAT_IEEE_FLOAT, channels, sample_rate, 64)
    }

    /// `wFormatTag` field — one of `WAVE_FORMAT_PCM`,
    /// `WAVE_FORMAT_IEEE_FLOAT`, or `WAVE_FORMAT_EXTENSIBLE`.
    #[inline]
    #[must_use]
    pub const fn format_tag(&self) -> u16 {
        self.format_tag
    }

    /// `nChannels` field.
    #[inline]
    #[must_use]
    pub const fn channels(&self) -> u16 {
        self.channels
    }

    /// `nSamplesPerSec` field, in hertz.
    #[inline]
    #[must_use]
    pub const fn sample_rate(&self) -> u32 {
        self.samples_per_sec
    }

    /// `wBitsPerSample` field.
    #[inline]
    #[must_use]
    pub const fn bits_per_sample(&self) -> u16 {
        self.bits_per_sample
    }

    /// `nBlockAlign` field — bytes per audio frame (one sample
    /// across all channels).
    #[inline]
    #[must_use]
    pub const fn block_align(&self) -> u16 {
        self.block_align
    }

    /// `nAvgBytesPerSec` field — `sample_rate * block_align`.
    #[inline]
    #[must_use]
    pub const fn avg_bytes_per_sec(&self) -> u32 {
        self.avg_bytes_per_sec
    }

    /// `true` if this is an IEEE-float PCM stream.
    #[inline]
    #[must_use]
    pub const fn is_float(&self) -> bool {
        self.format_tag == WAVE_FORMAT_IEEE_FLOAT
    }

    /// `true` if this is an integer PCM stream.
    #[inline]
    #[must_use]
    pub const fn is_int_pcm(&self) -> bool {
        self.format_tag == WAVE_FORMAT_PCM
    }
}

/// Outcome of `ProcessingObject::is_input_format_supported` (and
/// its output counterpart) — to be defined in [`crate::apo`].
///
/// Mirrors the three return paths defined by
/// `IAudioProcessingObject::IsInputFormatSupported`:
///
/// - [`Self::Accept`] — the proposed format is acceptable as-is.
/// - [`Self::Suggest`] — the proposed format is not acceptable, but
///   the named alternative is.
/// - [`Self::Reject`] — the APO cannot work with this format and
///   has no alternative to suggest.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum FormatNegotiation {
    /// Format is acceptable; the audio engine should adopt it.
    Accept,
    /// Format is not acceptable; suggest this alternative.
    Suggest(Format),
    /// Format is not acceptable and no alternative is available.
    Reject,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_float32_48k_mono_has_expected_fields() {
        let f = Format::pcm_float32(48_000, 1);
        assert_eq!(f.format_tag(), WAVE_FORMAT_IEEE_FLOAT);
        assert_eq!(f.channels(), 1);
        assert_eq!(f.sample_rate(), 48_000);
        assert_eq!(f.bits_per_sample(), 32);
        assert_eq!(f.block_align(), 4);
        assert_eq!(f.avg_bytes_per_sec(), 48_000 * 4);
        assert!(f.is_float());
        assert!(!f.is_int_pcm());
    }

    #[test]
    fn pcm_int16_44k1_stereo_has_expected_fields() {
        let f = Format::pcm_int16(44_100, 2);
        assert_eq!(f.format_tag(), WAVE_FORMAT_PCM);
        assert_eq!(f.channels(), 2);
        assert_eq!(f.sample_rate(), 44_100);
        assert_eq!(f.bits_per_sample(), 16);
        assert_eq!(f.block_align(), 4);
        assert_eq!(f.avg_bytes_per_sec(), 44_100 * 4);
        assert!(f.is_int_pcm());
        assert!(!f.is_float());
    }

    #[test]
    fn pcm_int24_48k_mono_block_align_is_3() {
        let f = Format::pcm_int24(48_000, 1);
        assert_eq!(f.block_align(), 3);
        assert_eq!(f.avg_bytes_per_sec(), 48_000 * 3);
    }

    #[test]
    fn pcm_float64_48k_5_1_block_align_is_48() {
        let f = Format::pcm_float64(48_000, 6);
        assert_eq!(f.block_align(), 48);
        assert_eq!(f.avg_bytes_per_sec(), 48_000 * 48);
    }

    #[test]
    fn negotiation_variants_distinguish() {
        let a = FormatNegotiation::Accept;
        let s = FormatNegotiation::Suggest(Format::pcm_float32(48_000, 1));
        let r = FormatNegotiation::Reject;
        assert_ne!(a, s);
        assert_ne!(s, r);
        assert_ne!(a, r);
    }
}
