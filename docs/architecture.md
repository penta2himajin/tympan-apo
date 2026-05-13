# Architecture

*Read this in other languages: [日本語](ja/architecture.md).*

This document describes the planned architecture. Implementation has not
begun. Details may change as design feedback accumulates.

## Module layout

```
tympan-apo/
├── src/
│   ├── lib.rs            # Re-exports; public API surface
│   ├── apo.rs            # ProcessingObject trait, lifecycle
│   ├── format.rs         # WAVEFORMATEX helpers, format negotiation
│   ├── property.rs       # IPropertyStore wrappers
│   ├── registration.rs   # CLSID + INF + registry helpers
│   ├── raw/              # Low-level: COM interface bindings via `windows`
│   │   ├── mod.rs
│   │   ├── interfaces.rs # IAudioProcessingObject* trait wiring
│   │   ├── hresult.rs    # APO-specific HRESULT codes
│   │   └── class.rs      # IClassFactory boilerplate
│   ├── realtime/         # Realtime-safe primitives
│   │   ├── mod.rs
│   │   ├── context.rs    # RealtimeContext marker type
│   │   ├── ring.rs       # Lock-free SPSC ring buffer
│   │   └── state.rs      # Atomic state machine helpers
│   └── aec/              # Windows 11 AEC APO support
│       ├── mod.rs
│       ├── auxiliary.rs  # IApoAuxiliaryInput* support
│       └── reference.rs  # Reference-stream handling (WASAPI loopback)
├── examples/
│   ├── passthrough/      # Trivial APO that copies input to output
│   ├── gain/             # Linear gain APO
│   └── aec-scaffold/     # AEC APO skeleton (no real DSP)
└── tests/
    └── ...               # Integration tests
```

## Layer model

Four conceptual layers, isolated by module boundary:

### Layer 1: `raw` — COM bindings

- Sole consumer of the `windows` crate's APO interface types
- Sole owner of `implement!`-based vtable construction
- Provides direct mappings of `IAudioProcessingObject`,
  `IAudioProcessingObjectRT`, `IAudioProcessingObjectConfiguration`,
  `IAudioSystemEffects3`, and the AEC APO interfaces

Users of `tympan-apo` should not need to touch this module. It exists
for the framework's internal use and for advanced users who need to
bypass the higher-level abstractions.

### Layer 2: `realtime` — zero-allocation primitives

- No allocator usage
- No `std::sync::Mutex`, no `std::collections::HashMap`
- Lock-free SPSC ring buffers (built on `crossbeam-utils`)
- Atomic state machines for plugin lifecycle
- A `RealtimeContext` zero-sized marker that:
  - Is required as a parameter for any function safe to call from
    `APOProcess`
  - Cannot be constructed outside the framework
  - Acts as a compile-time witness of realtime safety

This layer's invariant: any function reachable from `APOProcess` must
accept `&RealtimeContext` and contain no heap operations.

### Layer 3: Public API — safe, idiomatic

- `ProcessingObject` trait
- `Format`, `PropertyStore`, `ConfigurationContext` types
- Lifetime-bounded references to host-owned buffers (APO_CONNECTION_PROPERTY)
- Result types for fallible operations during initialization

This is the layer 95% of users will interact with.

### Layer 4: `aec` — Windows 11 AEC APO support

- Implements the auxiliary-input pattern required by AEC APOs
- Wraps `IApoAuxiliaryInputRT` for realtime reference-stream access
- Helpers for the WASAPI loopback path used when private channels are
  not available
- Optional: gated behind a `aec` cargo feature so non-AEC plugins
  don't pull in Windows 11 SDK requirements

## Core abstractions

### `ProcessingObject`

The top-level trait implemented by consumers. Maps to the APO COM
lifecycle.

```text
trait ProcessingObject: Sized {
    const CLSID: GUID;
    const NAME: &'static str;
    const COPYRIGHT: &'static str;
    const CATEGORY: ApoCategory;  // Sfx / Mfx / Efx

    fn new() -> Self;

    fn is_input_format_supported(
        &self,
        format: &Format,
    ) -> FormatNegotiation;

    fn lock_for_process(
        &mut self,
        input: &Format,
        output: &Format,
    ) -> Result<(), HResult>;

    fn process(
        &mut self,
        rt: &RealtimeContext,
        input: ApoInput,
        output: ApoOutput,
    );

    fn unlock_for_process(&mut self) {}
}
```

The framework provides COM object construction and class factory
registration as a macro:

```text
tympan_apo::register_apo!(MyApo);
```

This expands to the `DllGetClassObject` entry point and the
`IClassFactory` implementation that COM uses to instantiate the APO.

### `Format` and format negotiation

APOs negotiate sample rate, channel count, and bit depth with the
audio engine. The framework provides a `Format` wrapper around
`WAVEFORMATEX`/`WAVEFORMATEXTENSIBLE`:

```text
fn is_input_format_supported(
    &self,
    format: &Format,
) -> FormatNegotiation {
    if format.sample_rate() == 48_000 && format.channels() == 1 {
        FormatNegotiation::Accept
    } else {
        FormatNegotiation::Suggest(
            Format::pcm_float32(48_000, 1),
        )
    }
}
```

### `RealtimeContext`

Identical purpose to its counterpart in sibling tympan crates: a
zero-sized marker that compile-checks realtime safety. Instances are
passed by reference from the framework's `APOProcess` harness to user
code. They have no fields and no way to be constructed from user code.

### `aec::AecProcessingObject`

Extension trait for AEC APOs. Adds support for the auxiliary input
(reference stream from the render endpoint):

```text
trait AecProcessingObject: ProcessingObject {
    fn process_aec(
        &mut self,
        rt: &RealtimeContext,
        microphone: ApoInput,
        reference: ApoAuxiliaryInput,
        output: ApoOutput,
    );
}
```

The framework handles registration of the auxiliary input with the
audio engine and the timestamp alignment of microphone and reference
streams.

## Cross-cutting concerns

### CLSID allocation

APOs are identified by COM Class IDs (GUIDs). Authors must generate
a unique GUID per APO. The framework provides:

- Compile-time validation that `CLSID` is non-zero and not a
  well-known Microsoft GUID
- A `tympan-apo-genclsid` build-script helper that generates a fresh
  GUID for new plugins

### Registration

The framework provides INF-file templates and PowerShell snippets for:

- Registering the COM class (`regsvr32`)
- Associating the APO with a target endpoint's `FxProperties` registry
  entry
- Cleaning up on uninstallation

Users must run installation steps as Administrator. The framework does
not attempt to elevate privileges itself.

### Realtime logging

Realtime code cannot log via `tracing` or `log` (both allocate).
The `realtime` module provides a lock-free log queue for capturing
diagnostic events from `APOProcess`. A separate non-realtime thread
(spawned during `LockForProcess`) drains the queue.

## Open questions

Resolved during design phase:

- [ ] How to handle aggregation? COM APOs are inherently single-input
  single-output (with optional aux input). The framework should
  enforce this at the type level rather than relying on runtime
  checks.
- [ ] What is the minimum supported Windows version? Windows 10 21H2
  is reasonable; Windows 11 22H2+ for AEC APO. Should non-AEC APOs
  support older versions?
- [ ] How to handle the WASAPI loopback path for AEC reference
  streams? The audio engine can provide a reference via private
  channels or the APO can open its own loopback. The framework needs
  to abstract over both.
- [ ] How to interact with the audio engine's signal processing modes
  (Raw, Default, Communications, Speech, etc.)? Should APOs declare
  which modes they support, or remain mode-agnostic?
- [ ] Should the framework support the `IAudioSystemEffects2`
  notification pattern for dynamic effect on/off via
  `IAudioSystemEffectsControl`?

These will be resolved before implementation begins. Decisions will be
recorded in `docs/decisions/` (to be created).
