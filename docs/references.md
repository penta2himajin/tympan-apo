# References

Reference material consulted during design.

## Microsoft documentation

### Core APO documentation

- **Audio Processing Object Architecture**
  - <https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/audio-processing-object-architecture>
  - SFX / MFX / EFX categories, lifecycle, threading
- **Implementing Audio Processing Objects**
  - <https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/implementing-audio-processing-objects>
  - Required interfaces, base class usage, INF registration
- **Audio Signal Processing Modes**
  - <https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/audio-signal-processing-modes>
  - How APOs interact with the audio engine's processing modes
- **Deep Noise Suppression**
  - <https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/audio-signal-processing-modes#deep-noise-suppression>
  - Windows 11 24H2 system effect for AI-based noise suppression

### Windows 11 AEC APO API

- **Windows 11 APIs for Audio Processing Objects**
  - <https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/windows-11-apis-for-audio-processing-objects>
  - `IApoAcousticEchoCancellation`, `IApoAcousticEchoCancellation2`,
    `IApoAuxiliaryInputConfiguration`, `IApoAuxiliaryInputRT`
- **WASAPI loopback for AEC reference streams**
  - Covered in the above document; alternative to private-channel
    reference streams when those are not available

### Audio Engine context

- **AudioRenderEffectsManager**
  - <https://learn.microsoft.com/en-us/uwp/api/windows.media.audio.audiorendereffectsmanager>
  - Querying which effects are active on an endpoint
- **Audio Effects Discovery Sample**
  - Sample app from the Windows SDK demonstrating effect enumeration

## Reference implementations

### Microsoft Windows-driver-samples

- <https://github.com/microsoft/Windows-driver-samples>
- The canonical sample set, especially:
  - `audio/sysvad/EndpointsCommon/` — endpoint structure
  - `audio/sysvad/SwapAPO/` — channel-swap APO (simple SFX example)
  - `audio/sysvad/AecAPO/` — Windows 11 AEC APO sample
  - `audio/sysvad/KwsAPO/` — keyword spotter APO (loopback stripping)
- License: MIT

### Equalizer APO

- <https://sourceforge.net/projects/equalizerapo/>
- The most-deployed third-party APO
- License: GPL-2.0
- Demonstrates: system-wide parametric DSP, runtime configuration via
  text files, multi-channel EQ, VST host capability
- Notable as proof that third-party APOs can do far more than the
  Microsoft documentation suggests

### dechamps/APO

- <https://github.com/dechamps/APO>
- A community-maintained collection of notes on APO development
- Particularly useful for: registration mechanics, registry layout,
  the gap between documented and actual APO behaviour
- License: MIT-style

### NoiseTorch (Linux equivalent, for cross-reference)

- <https://github.com/noisetorch/NoiseTorch>
- Not an APO, but the closest open-source analogue for system-wide
  microphone noise suppression on Linux
- Useful as a reference for: user-facing UX, recovery strategies when
  audio pipelines reset, the case for in-process audio enhancement

## Related Rust crates

### COM bindings

- **windows** (Microsoft official)
  - <https://crates.io/crates/windows>
  - The official Rust bindings for the Windows API
  - Provides type definitions for `IAudioProcessingObject*` and
    related interfaces; `tympan-apo` builds on this
- **windows-sys** (lower-level)
  - <https://crates.io/crates/windows-sys>
  - Raw `extern "system"` bindings without the COM convenience layer

### Audio client-side (out of scope for tympan-apo, but relevant)

- **wasapi-rs**
  - <https://crates.io/crates/wasapi>
  - Friendly Rust wrapper around WASAPI for client-side audio
  - Use this to *capture* audio from devices, not to *process* it
    inside the audio engine
- **cpal**
  - <https://crates.io/crates/cpal>
  - Cross-platform client-side audio I/O

### Realtime / lock-free

- **crossbeam**
  - <https://crates.io/crates/crossbeam>
  - Lock-free data structures suitable for the realtime thread
- **atomic-waker**
  - <https://crates.io/crates/atomic-waker>
  - Cross-thread wake notification (non-blocking)

### General DSP

- **rustfft**: FFT used by spectral processing
- **biquad**: Standard biquad filters
- **realfft**: FFT optimized for real-valued signals

## Realtime audio programming background

- **Ross Bencina, "Real-time audio programming 101: time waits for nothing"**
  - <http://www.rossbencina.com/code/real-time-audio-programming-101-time-waits-for-nothing>
  - The canonical introduction to realtime audio constraints
- **Windows real-time scheduling**
  - APO threads run at `AVRT_PRIORITY_REALTIME` via the MMCSS service
  - The audio engine adjusts thread priorities; the APO should not
    attempt to modify scheduling itself

## Build, signing, and deployment

- **Code signing for audio drivers and APOs**
  - APO loading does not require WHQL signing (unlike kernel drivers)
  - But SmartScreen on installation prefers EV-code-signed binaries
  - <https://learn.microsoft.com/en-us/windows-hardware/drivers/install/code-signing-best-practices>
- **INF files for APO registration**
  - <https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/registering-an-apo>
  - The official mechanism; alternatives include direct `regsvr32`
    plus registry edits
- **Windows Update interaction**
  - Audio driver reinstallation can overwrite APO registration
  - The framework documents (but cannot prevent) this
  - Equalizer APO users report this is the single biggest operational
    issue
