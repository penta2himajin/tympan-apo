//! PCM audio stream format and format negotiation.
//!
//! [`Format`] mirrors the fields of the Windows `WAVEFORMATEX`
//! structure plus the `WAVEFORMATEXTENSIBLE` extension (channel
//! mask, valid-bits-per-sample, sub-format GUID). The layout is
//! plain Rust so the type is constructible and inspectable from
//! cross-platform code; conversions to and from the FFI structures
//! live under `#[cfg(windows)]`.
//!
//! ## Extensible vs. base
//!
//! `WAVEFORMATEXTENSIBLE` is the canonical wire format for any
//! stream with more than two channels, a bit depth other than 8 or
//! 16, or an explicit channel-position mask. The base
//! `WAVEFORMATEX` covers everything else. The framework's typed
//! constructors (`pcm_int16`, `pcm_float32`, ...) produce the base
//! variant; opt into the extensible variant with
//! [`Format::with_extensible`] (which also fills in a default
//! `channel_mask` based on the channel count).

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
/// APO: format tag, channel count, sample rate, bit depth, and (for
/// the extensible variant) channel mask plus sub-format GUID. The
/// derived fields (`block_align`, `avg_bytes_per_sec`) are computed
/// at construction from the inputs.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Format {
    /// Logical PCM type: `WAVE_FORMAT_PCM` or `WAVE_FORMAT_IEEE_FLOAT`.
    /// The wire `wFormatTag` switches to `WAVE_FORMAT_EXTENSIBLE`
    /// when the [`Self::extensible`] flag is set; this field
    /// remembers the original choice so the sub-format GUID
    /// resolves correctly.
    format_tag: u16,
    channels: u16,
    samples_per_sec: u32,
    avg_bytes_per_sec: u32,
    block_align: u16,
    bits_per_sample: u16,
    /// `WAVEFORMATEXTENSIBLE::Samples::wValidBitsPerSample`. Zero
    /// means "same as `bits_per_sample`" and only matters for the
    /// extensible variant.
    valid_bits_per_sample: u16,
    /// `WAVEFORMATEXTENSIBLE::dwChannelMask`. Zero means "engine
    /// picks the default for `channels`".
    channel_mask: u32,
    /// When `true`, the format crosses the COM boundary as
    /// `WAVEFORMATEXTENSIBLE` (`wFormatTag = WAVE_FORMAT_EXTENSIBLE`,
    /// `cbSize = 22`) and carries the channel mask + sub-format
    /// extension. When `false`, the format uses the base
    /// `WAVEFORMATEX` (`cbSize = 0`).
    extensible: bool,
}

impl Format {
    /// Construct a format directly from its fields.
    ///
    /// Prefer the typed constructors (`pcm_int16`, `pcm_float32`,
    /// ...) when modelling a standard PCM stream. This raw
    /// constructor is intended for round-tripping through
    /// `WAVEFORMATEX` and for tests. Initialises the extension
    /// fields (`valid_bits_per_sample`, `channel_mask`) to zero.
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
            valid_bits_per_sample: 0,
            channel_mask: 0,
            extensible: false,
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

    /// `true` if this is the extensible variant — the wire format
    /// uses `WAVE_FORMAT_EXTENSIBLE` and `cbSize == 22` to surface
    /// `channel_mask` and `valid_bits_per_sample` over the wire.
    /// The logical PCM / float distinction stays in `format_tag`
    /// and resolves to the sub-format GUID at conversion time.
    #[inline]
    #[must_use]
    pub const fn is_extensible(&self) -> bool {
        self.extensible
    }

    /// `WAVEFORMATEXTENSIBLE::dwChannelMask` value (zero if
    /// unspecified — the audio engine picks a default for the
    /// channel count).
    #[inline]
    #[must_use]
    pub const fn channel_mask(&self) -> u32 {
        self.channel_mask
    }

    /// `WAVEFORMATEXTENSIBLE::Samples::wValidBitsPerSample` —
    /// effective precision when the container is wider than the
    /// sample (e.g. 24-bit-in-32-bit). Zero means "same as
    /// `bits_per_sample`".
    #[inline]
    #[must_use]
    pub const fn valid_bits_per_sample(&self) -> u16 {
        self.valid_bits_per_sample
    }

    /// Promote a base `WAVEFORMATEX`-style format to the extensible
    /// wire variant.
    ///
    /// Flips the `extensible` flag to `true` and fills in the
    /// channel-position mask via [`default_channel_mask`] when
    /// `channel_mask` is currently zero. The `format_tag` field
    /// stays as `WAVE_FORMAT_PCM` / `WAVE_FORMAT_IEEE_FLOAT` so
    /// the sub-format GUID can still be resolved; only the
    /// over-the-wire `wFormatTag` changes (to
    /// `WAVE_FORMAT_EXTENSIBLE`).
    #[inline]
    #[must_use]
    pub const fn with_extensible(mut self) -> Self {
        self.extensible = true;
        if self.channel_mask == 0 {
            self.channel_mask = default_channel_mask(self.channels);
        }
        if self.valid_bits_per_sample == 0 {
            self.valid_bits_per_sample = self.bits_per_sample;
        }
        self
    }

    /// Override `channel_mask`. Useful for declaring custom
    /// channel layouts; pass zero to clear back to the default.
    #[inline]
    #[must_use]
    pub const fn with_channel_mask(mut self, mask: u32) -> Self {
        self.channel_mask = mask;
        self
    }

    /// Override `valid_bits_per_sample`. Pass zero to clear back
    /// to "same as `bits_per_sample`".
    #[inline]
    #[must_use]
    pub const fn with_valid_bits_per_sample(mut self, bits: u16) -> Self {
        self.valid_bits_per_sample = bits;
        self
    }
}

/// Default `WAVEFORMATEXTENSIBLE::dwChannelMask` for a given channel
/// count, matching the Microsoft "consumer convention" layouts.
/// Returns `0` for unusual channel counts that have no canonical
/// layout (the caller should provide an explicit mask via
/// [`Format::with_channel_mask`]).
#[must_use]
pub const fn default_channel_mask(channels: u16) -> u32 {
    // KSAUDIO_SPEAKER constants from ksmedia.h.
    const SPEAKER_FRONT_LEFT: u32 = 0x1;
    const SPEAKER_FRONT_RIGHT: u32 = 0x2;
    const SPEAKER_FRONT_CENTER: u32 = 0x4;
    const SPEAKER_LOW_FREQUENCY: u32 = 0x8;
    const SPEAKER_BACK_LEFT: u32 = 0x10;
    const SPEAKER_BACK_RIGHT: u32 = 0x20;
    const SPEAKER_SIDE_LEFT: u32 = 0x200;
    const SPEAKER_SIDE_RIGHT: u32 = 0x400;
    match channels {
        1 => SPEAKER_FRONT_CENTER,
        2 => SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT,
        // 5.1 with LFE
        6 => {
            SPEAKER_FRONT_LEFT
                | SPEAKER_FRONT_RIGHT
                | SPEAKER_FRONT_CENTER
                | SPEAKER_LOW_FREQUENCY
                | SPEAKER_BACK_LEFT
                | SPEAKER_BACK_RIGHT
        }
        // 7.1 with LFE
        8 => {
            SPEAKER_FRONT_LEFT
                | SPEAKER_FRONT_RIGHT
                | SPEAKER_FRONT_CENTER
                | SPEAKER_LOW_FREQUENCY
                | SPEAKER_BACK_LEFT
                | SPEAKER_BACK_RIGHT
                | SPEAKER_SIDE_LEFT
                | SPEAKER_SIDE_RIGHT
        }
        _ => 0,
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
            valid_bits_per_sample: 0,
            channel_mask: 0,
            extensible: false,
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
            wFormatTag: if self.extensible {
                WAVE_FORMAT_EXTENSIBLE
            } else {
                self.format_tag
            },
            nChannels: self.channels,
            nSamplesPerSec: self.samples_per_sec,
            nAvgBytesPerSec: self.avg_bytes_per_sec,
            nBlockAlign: self.block_align,
            wBitsPerSample: self.bits_per_sample,
            cbSize: if self.extensible { 22 } else { 0 },
        }
    }

    /// Construct a [`Format`] from a Windows
    /// `WAVEFORMATEXTENSIBLE`.
    ///
    /// Copies the base fields plus `dwChannelMask`,
    /// `wValidBitsPerSample`, and resolves the sub-format GUID
    /// back to a logical `format_tag` value (`WAVE_FORMAT_PCM` /
    /// `WAVE_FORMAT_IEEE_FLOAT`). The wire `wFormatTag` of the
    /// extensible struct itself is always `WAVE_FORMAT_EXTENSIBLE`;
    /// the framework records that via the `extensible` flag.
    #[must_use]
    pub fn from_waveformatextensible(
        wfx: &windows::Win32::Media::Audio::WAVEFORMATEXTENSIBLE,
    ) -> Self {
        let base = wfx.Format;
        // Safety: WAVEFORMATEXTENSIBLE_0 is a `Copy` packed union;
        // the field-level read happens through the value-context
        // idiom so we never form a reference into the packed layout.
        let valid_bits = unsafe { wfx.Samples.wValidBitsPerSample };
        // Resolve the sub-format GUID back to the logical PCM /
        // IEEE_FLOAT distinction. Anything we do not recognise
        // falls back to PCM.
        const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: windows_core::GUID =
            windows_core::GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
        let sub: windows_core::GUID = wfx.SubFormat;
        let logical_tag = if sub == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
            WAVE_FORMAT_IEEE_FLOAT
        } else {
            WAVE_FORMAT_PCM
        };
        Self {
            format_tag: logical_tag,
            channels: { base.nChannels },
            samples_per_sec: { base.nSamplesPerSec },
            avg_bytes_per_sec: { base.nAvgBytesPerSec },
            block_align: { base.nBlockAlign },
            bits_per_sample: { base.wBitsPerSample },
            valid_bits_per_sample: valid_bits,
            channel_mask: { wfx.dwChannelMask },
            extensible: true,
        }
    }

    /// Project this [`Format`] into a Windows
    /// `WAVEFORMATEXTENSIBLE`.
    ///
    /// The wire `wFormatTag` is `WAVE_FORMAT_EXTENSIBLE` and the
    /// `cbSize` is 22, regardless of the logical
    /// [`Self::format_tag`]. The sub-format GUID is resolved from
    /// the logical `format_tag` (PCM →
    /// `KSDATAFORMAT_SUBTYPE_PCM`, IEEE_FLOAT →
    /// `KSDATAFORMAT_SUBTYPE_IEEE_FLOAT`).
    #[must_use]
    pub fn to_waveformatextensible(&self) -> windows::Win32::Media::Audio::WAVEFORMATEXTENSIBLE {
        use windows::Win32::Media::Audio::{WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0};
        let base = windows::Win32::Media::Audio::WAVEFORMATEX {
            // Over the wire the extensible variant always carries
            // WAVE_FORMAT_EXTENSIBLE; the logical tag lives in
            // the SubFormat GUID below.
            wFormatTag: WAVE_FORMAT_EXTENSIBLE,
            nChannels: self.channels,
            nSamplesPerSec: self.samples_per_sec,
            nAvgBytesPerSec: self.avg_bytes_per_sec,
            nBlockAlign: self.block_align,
            wBitsPerSample: self.bits_per_sample,
            // `cbSize` for WAVEFORMATEXTENSIBLE is always 22 (the
            // size of the extension past WAVEFORMATEX).
            cbSize: 22,
        };
        let samples = WAVEFORMATEXTENSIBLE_0 {
            wValidBitsPerSample: if self.valid_bits_per_sample == 0 {
                self.bits_per_sample
            } else {
                self.valid_bits_per_sample
            },
        };
        WAVEFORMATEXTENSIBLE {
            Format: base,
            Samples: samples,
            dwChannelMask: self.channel_mask,
            SubFormat: self.sub_format_guid(),
        }
    }

    /// Sub-format GUID for the extensible variant.
    ///
    /// Resolves from the logical [`Self::format_tag`]:
    /// `WAVE_FORMAT_PCM` → `KSDATAFORMAT_SUBTYPE_PCM`,
    /// `WAVE_FORMAT_IEEE_FLOAT` → `KSDATAFORMAT_SUBTYPE_IEEE_FLOAT`.
    /// Other values fall back to PCM.
    fn sub_format_guid(&self) -> windows_core::GUID {
        // `Win32_Media_KernelStreaming` exposes the PCM GUID;
        // `KSDATAFORMAT_SUBTYPE_IEEE_FLOAT` lives in the Multimedia
        // module which we do not enable, so we hard-code its value
        // (matches ksmedia.h).
        const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: windows_core::GUID =
            windows_core::GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
        if self.is_int_pcm() {
            windows::Win32::Media::KernelStreaming::KSDATAFORMAT_SUBTYPE_PCM
        } else {
            KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
        }
    }

    /// Read a [`Format`] from a Windows `WAVEFORMATEX` pointer,
    /// detecting the extensible variant via the `cbSize` /
    /// `wFormatTag` markers and re-reading as
    /// `WAVEFORMATEXTENSIBLE` when present.
    ///
    /// # Safety
    ///
    /// `wf` must point to a valid `WAVEFORMATEX`; if its `cbSize`
    /// is at least 22, the bytes past the base struct must form a
    /// valid `WAVEFORMATEXTENSIBLE` (the audio engine guarantees
    /// this when the format tag is `WAVE_FORMAT_EXTENSIBLE`).
    #[must_use]
    pub unsafe fn from_waveformatex_ptr(
        wf: *const windows::Win32::Media::Audio::WAVEFORMATEX,
    ) -> Self {
        // Safety: caller guarantees `wf` is a valid WAVEFORMATEX.
        let base = unsafe { &*wf };
        if { base.cbSize } >= 22 && { base.wFormatTag } == WAVE_FORMAT_EXTENSIBLE {
            // Safety: when cbSize >= 22 and the tag is extensible,
            // the audio engine guarantees the bytes past the base
            // struct continue with the WAVEFORMATEXTENSIBLE
            // extension.
            let wfx = wf as *const windows::Win32::Media::Audio::WAVEFORMATEXTENSIBLE;
            Self::from_waveformatextensible(unsafe { &*wfx })
        } else {
            Self::from_waveformatex(base)
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

    #[test]
    fn waveformatextensible_is_40_bytes_packed_one() {
        // WAVEFORMATEX (18) + Samples union (2) + dwChannelMask (4)
        // + SubFormat (16) = 40, all packed(1).
        assert_eq!(
            core::mem::size_of::<windows::Win32::Media::Audio::WAVEFORMATEXTENSIBLE>(),
            40
        );
        assert_eq!(
            core::mem::align_of::<windows::Win32::Media::Audio::WAVEFORMATEXTENSIBLE>(),
            1
        );
    }

    #[test]
    fn to_waveformatextensible_sets_wire_tag_and_subformat_for_float32() {
        let f = Format::pcm_float32(48_000, 8).with_extensible();
        let wfx = f.to_waveformatextensible();
        // The wire wFormatTag inside WAVEFORMATEXTENSIBLE.Format is
        // always WAVE_FORMAT_EXTENSIBLE.
        assert_eq!({ wfx.Format.wFormatTag }, WAVE_FORMAT_EXTENSIBLE);
        assert_eq!({ wfx.Format.cbSize }, 22);
        assert_eq!({ wfx.Format.nChannels }, 8);
        // SubFormat = KSDATAFORMAT_SUBTYPE_IEEE_FLOAT.
        const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: windows_core::GUID =
            windows_core::GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
        assert_eq!({ wfx.SubFormat }, KSDATAFORMAT_SUBTYPE_IEEE_FLOAT);
        // wValidBitsPerSample defaults to bits_per_sample.
        assert_eq!(unsafe { wfx.Samples.wValidBitsPerSample }, 32);
        // dwChannelMask is the 7.1 default for 8 channels.
        assert!({ wfx.dwChannelMask } != 0);
    }

    #[test]
    fn to_waveformatextensible_sets_pcm_subformat_for_int16() {
        let f = Format::pcm_int16(48_000, 2).with_extensible();
        let wfx = f.to_waveformatextensible();
        assert_eq!(
            { wfx.SubFormat },
            windows::Win32::Media::KernelStreaming::KSDATAFORMAT_SUBTYPE_PCM
        );
    }

    #[test]
    fn extensible_round_trips_through_waveformatextensible() {
        let original = Format::pcm_float32(48_000, 6)
            .with_extensible()
            .with_valid_bits_per_sample(24);
        let wfx = original.to_waveformatextensible();
        let parsed = Format::from_waveformatextensible(&wfx);
        // Logical fields preserved.
        assert!(parsed.is_extensible());
        assert_eq!(parsed.format_tag(), WAVE_FORMAT_IEEE_FLOAT);
        assert_eq!(parsed.channels(), original.channels());
        assert_eq!(parsed.sample_rate(), original.sample_rate());
        assert_eq!(parsed.channel_mask(), original.channel_mask());
        assert_eq!(parsed.valid_bits_per_sample(), 24);
    }

    #[test]
    fn from_waveformatex_ptr_picks_extensible_when_cbsize_22() {
        let f = Format::pcm_float32(48_000, 8).with_extensible();
        let wfx = f.to_waveformatextensible();
        // Treat &wfx as &WAVEFORMATEX — the framework's COM bridge
        // sees only the WAVEFORMATEX prefix from GetAudioFormat.
        let prefix: *const windows::Win32::Media::Audio::WAVEFORMATEX =
            core::ptr::addr_of!(wfx.Format);
        // Safety: prefix points to the WAVEFORMATEX prefix of a
        // live WAVEFORMATEXTENSIBLE.
        let parsed = unsafe { Format::from_waveformatex_ptr(prefix) };
        assert!(parsed.is_extensible());
        assert_eq!(parsed.channels(), 8);
        assert_eq!(parsed.format_tag(), WAVE_FORMAT_IEEE_FLOAT);
    }

    #[test]
    fn from_waveformatex_ptr_keeps_base_when_cbsize_zero() {
        let f = Format::pcm_float32(48_000, 2);
        let wf = f.to_waveformatex();
        let ptr: *const windows::Win32::Media::Audio::WAVEFORMATEX = &wf;
        // Safety: ptr points to a live WAVEFORMATEX with cbSize=0.
        let parsed = unsafe { Format::from_waveformatex_ptr(ptr) };
        assert!(!parsed.is_extensible());
        assert_eq!(parsed.format_tag(), WAVE_FORMAT_IEEE_FLOAT);
    }

    #[test]
    fn default_channel_mask_returns_known_layouts() {
        assert_eq!(default_channel_mask(1), 0x4); // FRONT_CENTER
        assert_eq!(default_channel_mask(2), 0x3); // FL | FR
        assert_eq!(default_channel_mask(6), 0x3F); // 5.1
        assert_eq!(default_channel_mask(8), 0x63F); // 7.1
        assert_eq!(default_channel_mask(3), 0); // unusual count
    }
}
