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

#[cfg(windows)]
impl Format {
    /// Construct a [`Format`] from a Windows
    /// [`WAVEFORMATEX`](windows::Win32::Media::Audio::WAVEFORMATEX).
    ///
    /// Only the base `WAVEFORMATEX` fields are copied; `cbSize` and
    /// any trailing extension bytes (as used by
    /// `WAVEFORMATEXTENSIBLE`) are ignored. To round-trip an
    /// extensible format, deal with the extension explicitly before
    /// or after calling this routine.
    ///
    /// `WAVEFORMATEX` is `#[repr(C, packed(1))]`, so this function
    /// performs the field copies through the `{ ... }` value-context
    /// idiom rather than taking references into the packed layout.
    #[must_use]
    pub fn from_waveformatex(wf: &windows::Win32::Media::Audio::WAVEFORMATEX) -> Self {
        Self {
            format_tag: { wf.wFormatTag },
            channels: { wf.nChannels },
            samples_per_sec: { wf.nSamplesPerSec },
            avg_bytes_per_sec: { wf.nAvgBytesPerSec },
            block_align: { wf.nBlockAlign },
            bits_per_sample: { wf.wBitsPerSample },
        }
    }

    /// Project this [`Format`] into a Windows
    /// [`WAVEFORMATEX`](windows::Win32::Media::Audio::WAVEFORMATEX).
    ///
    /// `cbSize` is zero, matching plain
    /// `WAVE_FORMAT_PCM` / `WAVE_FORMAT_IEEE_FLOAT` streams with no
    /// trailing extension data.
    #[must_use]
    pub fn to_waveformatex(&self) -> windows::Win32::Media::Audio::WAVEFORMATEX {
        windows::Win32::Media::Audio::WAVEFORMATEX {
            wFormatTag: self.format_tag,
            nChannels: self.channels,
            nSamplesPerSec: self.samples_per_sec,
            nAvgBytesPerSec: self.avg_bytes_per_sec,
            nBlockAlign: self.block_align,
            wBitsPerSample: self.bits_per_sample,
            cbSize: 0,
        }
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

#[cfg(all(test, windows))]
mod windows_conv_tests {
    use super::*;
    use windows::Win32::Media::Audio::WAVEFORMATEX;

    #[test]
    fn windows_waveformatex_is_18_bytes_packed_one() {
        // Sanity-check the windows crate's representation. If
        // Microsoft ever changes WAVEFORMATEX's layout, the
        // conversion routines need a closer look.
        assert_eq!(core::mem::size_of::<WAVEFORMATEX>(), 18);
        assert_eq!(core::mem::align_of::<WAVEFORMATEX>(), 1);
    }

    #[test]
    fn pcm_float32_48k_mono_round_trips() {
        let f = Format::pcm_float32(48_000, 1);
        let wf = f.to_waveformatex();
        assert_eq!({ wf.wFormatTag }, WAVE_FORMAT_IEEE_FLOAT);
        assert_eq!({ wf.nChannels }, 1);
        assert_eq!({ wf.nSamplesPerSec }, 48_000);
        assert_eq!({ wf.nAvgBytesPerSec }, 48_000 * 4);
        assert_eq!({ wf.nBlockAlign }, 4);
        assert_eq!({ wf.wBitsPerSample }, 32);
        assert_eq!({ wf.cbSize }, 0);

        let f2 = Format::from_waveformatex(&wf);
        assert_eq!(f, f2);
    }

    #[test]
    fn every_typed_constructor_round_trips() {
        for f in [
            Format::pcm_int16(44_100, 2),
            Format::pcm_int24(48_000, 1),
            Format::pcm_int32(96_000, 4),
            Format::pcm_float32(48_000, 1),
            Format::pcm_float64(192_000, 8),
        ] {
            let wf = f.to_waveformatex();
            let f2 = Format::from_waveformatex(&wf);
            assert_eq!(f, f2, "round-trip failed for {f:?}");
        }
    }

    #[test]
    fn from_waveformatex_preserves_all_base_fields() {
        let wf = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_PCM,
            nChannels: 2,
            nSamplesPerSec: 44_100,
            nAvgBytesPerSec: 44_100 * 4,
            nBlockAlign: 4,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        let f = Format::from_waveformatex(&wf);
        assert_eq!(f.format_tag(), WAVE_FORMAT_PCM);
        assert_eq!(f.channels(), 2);
        assert_eq!(f.sample_rate(), 44_100);
        assert_eq!(f.avg_bytes_per_sec(), 44_100 * 4);
        assert_eq!(f.block_align(), 4);
        assert_eq!(f.bits_per_sample(), 16);
    }

    #[test]
    fn from_waveformatex_ignores_cbsize() {
        // The trailing extension bytes are out of scope for
        // `Format`. We make sure a non-zero cbSize does not affect
        // the resulting `Format`.
        let wf = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE,
            nChannels: 6,
            nSamplesPerSec: 48_000,
            nAvgBytesPerSec: 48_000 * 24,
            nBlockAlign: 24,
            wBitsPerSample: 32,
            cbSize: 22, // sizeof(WAVEFORMATEXTENSIBLE) - sizeof(WAVEFORMATEX)
        };
        let f = Format::from_waveformatex(&wf);
        assert_eq!(f.format_tag(), WAVE_FORMAT_EXTENSIBLE);
        assert_eq!(f.channels(), 6);
        assert_eq!(f.sample_rate(), 48_000);
        // `Format::to_waveformatex` zeroes cbSize again, which is
        // the documented behaviour for the lossy round-trip.
        assert_eq!({ f.to_waveformatex().cbSize }, 0);
    }
}
