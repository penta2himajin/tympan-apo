# Overview

*Read this in other languages: [日本語](ja/overview.md).*

## Purpose

`tympan-apo` is a Rust framework for implementing Windows
**Audio Processing Objects** (APOs) — the COM-based system-effect
plugins that run inside the Windows Audio Engine (`audiodg.exe`) and
apply digital signal processing to audio streams flowing through
specific devices.

The goal is to enable Rust applications to:

- Implement Stream Effect (SFX) APOs that process per-application audio
- Implement Mode Effect (MFX) APOs that process audio mapped to a
  specific mode (e.g. communications, media)
- Implement Acoustic Echo Cancellation APOs using the Windows 11
  AEC APO API (the official slot for AEC and adjacent processing in
  the microphone capture pipeline)
- Build noise-suppression, voice-effect, or general microphone
  enhancement plugins that work system-wide for any application using
  the affected device

… without writing C++.

## Why this exists

The APO architecture is defined in C++ COM headers
(`AudioEngineBaseAPO.h`, `audioenginebaseapo.idl`). The standard
implementation path inherits from `CBaseAudioProcessingObject` and is
illustrated by Microsoft's SYSVAD sample and the open-source Equalizer
APO project.

There is no first-party Rust binding or framework. Existing options
for Rust developers are:

| Approach | Status | Trade-off |
|---|---|---|
| Hand-rolled COM in Rust via `windows` crate | Possible, complex | Hundreds of lines per APO; manual IUnknown bookkeeping |
| C++ wrapper + Rust core via FFI | Hybrid | Build complexity; loses pure-Rust appeal |
| Direct use of SYSVAD sample as template | C++ only | No Rust path |

This framework fills the Rust gap. It encapsulates the COM bookkeeping,
realtime safety concerns, and Windows-specific quirks behind safe Rust
traits.

## Scope

### In scope

- APO COM object infrastructure (IUnknown, IClassFactory, registration)
- Required interfaces: `IAudioProcessingObject`,
  `IAudioProcessingObjectConfiguration`, `IAudioProcessingObjectRT`,
  `IAudioSystemEffects` (and v2/v3 variants)
- SFX and MFX APO categories
- AEC APO support via `IApoAcousticEchoCancellation`,
  `IApoAcousticEchoCancellation2`, `IApoAuxiliaryInputConfiguration`,
  `IApoAuxiliaryInputRT` (Windows 11 23H2+)
- Format negotiation helpers (sample rate, channel count, bit depth)
- Property store wrappers for APO configuration
- Realtime-safe primitives (lock-free ring buffers, atomic state
  helpers)
- Registration helpers (CLSID assignment, INF file generation,
  registry write helpers for the FxProperties location)
- Example APOs: minimal passthrough, simple gain, reference AEC
  scaffold

### Out of scope

- Endpoint Effect (EFX) APOs — same APIs apply, but EFX scope is
  inherently entire-device and warrants additional consideration
- Kernel-mode WDM audio drivers (not APOs at all; entirely different
  programming model)
- Audio Driver Foundation (used by hardware vendors to ship complete
  driver stacks)
- DAW plugin formats (VST3, ASIO) — different APIs
- Signal-processing algorithms (DSP, ML) — these belong in consumer
  crates that depend on `tympan-apo`

## Naming

*Tympan* refers to the tympanal organ — a membrane-based hearing organ
on the abdomen of moths in families such as Pyralidae and Noctuidae.
The organ evolved as a defence against bat echolocation: it captures
ultrasound and converts vibration into neural signals via attached
chordotonal receptors.

The analogy:

- A tympanal organ sits between the outside world and the moth's
  nervous system, converting one physical domain (air pressure) into
  another (nerve impulses).
- `tympan-apo` sits between the Windows Audio Engine and user-space
  Rust code, converting one programming domain (COM, IUnknown,
  realtime APOProcess callbacks) into another (safe Rust types,
  ownership, lifetimes).

The second word `apo` is Microsoft's abbreviation for Audio Processing
Object.

## Status

**Design phase.** As of the initial commit:

- No source code in `src/`
- API design documented in [`architecture.md`](architecture.md)
- Reference material gathered in [`references.md`](references.md)

Implementation will begin once the API design is reviewed and
stabilised.

## Target audience

- Rust developers building Windows audio applications that need
  system-wide audio effects without requiring users to install virtual
  audio devices
- Plugin authors targeting the Windows Audio Engine
- Researchers prototyping audio processing pipelines that need to
  integrate at the Windows Audio Engine layer

Not intended for:

- Application-level audio playback (use `cpal`, `wasapi-rs`, or the
  `windows` crate directly)
- DAW-specific plugin formats (VST3, ASIO) — those use entirely
  different APIs
- Cross-platform plugin formats (LADSPA, LV2 are Linux-centric)

## Comparison to alternatives

### vs. `windows` crate (raw COM)

The Microsoft-maintained `windows` crate provides Rust bindings to the
entire Windows API including APO interfaces. Building an APO with it
directly is possible but requires:

- Hand-implementing `IUnknown`, `IClassFactory`, and the COM lifetime
  protocol
- Manual vtable construction or extensive use of the `implement!`
  macro
- Encyclopaedic knowledge of which methods are realtime-safe and which
  are not

`tympan-apo` builds on `windows` (it has to) but provides a higher-level
abstraction so users implement a `ProcessingObject` trait rather than a
full COM interface set.

### vs. Equalizer APO

[Equalizer APO](https://sourceforge.net/projects/equalizerapo/) is the
best-known open-source APO. It is implemented in C++ and provides
system-wide DSP via a parametric pipeline configured at runtime.
It demonstrates that the APO architecture supports general-purpose
DSP, but is C++-only and not designed as a framework for other
plugins.

`tympan-apo` is the closest analogue in spirit: enabling third-party
DSP via the APO mechanism, but in Rust and as a reusable library.

### vs. SYSVAD AEC sample

Microsoft's [Windows-driver-samples](https://github.com/microsoft/Windows-driver-samples)
repository includes an AEC APO sample in `audio/sysvad/`. It is the
canonical reference for the Windows 11 AEC APO API. `tympan-apo`
adopts the same API surface but exposes it through Rust idioms.

## Registration and deployment

APOs are COM in-process servers. Deployment involves:

1. Building the APO as a `cdylib` producing a `.dll`
2. Registering the COM class with `regsvr32` (writes CLSID entries to
   `HKLM\SOFTWARE\Classes\CLSID\{...}`)
3. Associating the APO with a target audio endpoint via registry edits
   under
   `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Capture\{device-guid}\FxProperties`
4. Restarting the Windows audio service or rebooting

The framework provides build-script helpers to generate INF files
covering these registration steps, and PowerShell snippets users can
invoke during installation.

### Caveats

- Windows Update may overwrite the APO registration when reinstalling
  audio drivers. This is a known limitation of third-party APOs
  (Equalizer APO users encounter it routinely). The framework cannot
  prevent this, but documents recovery strategies.
- Code signing: an EV code-signing certificate avoids SmartScreen
  warnings on installation but is not strictly required for APO
  loading. The audio engine accepts unsigned APOs as long as the
  CLSID is registered.
