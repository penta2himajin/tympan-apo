# Architecture

*Read this in other languages: [日本語](ja/architecture.md).*

This document describes the framework's implemented architecture: the
module layout, the four-layer model, and the core abstractions users
implement against. The "In scope" feature set from
[`overview.md`](overview.md) is complete; see
[`decisions/0001-ci-verification-strategy.md`](decisions/0001-ci-verification-strategy.md)
and [`testing.md`](testing.md) for the verification strategy.

## Module layout

The framework crate is an `rlib` only. The four `Dll*` COM entry
points are emitted into the *consumer* crate's root by the
`register_apo!` / `register_aec_apo!` macros, so the framework itself
does not produce a `cdylib` — that avoids the parallel-link race
(`rust-lang/cargo#6313`) that a dual `rlib` + `cdylib` artefact would
hit. Each reference APO under `examples/` is its own `cdylib`.

```
tympan-apo/
├── src/
│   ├── lib.rs            # Re-exports; public API surface
│   ├── apo.rs            # ProcessingObject trait, ProcessInput,
│   │                     #   ApoCategory, SystemEffect
│   ├── instance.rs       # ApoInstance<T> + AnyApoInstance: the
│   │                     #   framework-side lifecycle wrapper
│   ├── buffer.rs         # BufferFlags, ConnectionProperty
│   ├── format.rs         # Format, FormatNegotiation,
│   │                     #   WAVEFORMATEX(TENSIBLE) conversions
│   ├── error.rs          # HResult wrapper + APO HRESULT constants
│   ├── clsid.rs          # Clsid (cross-platform GUID)
│   ├── inf.rs            # INF file generator
│   ├── fx_properties.rs  # FxProperties endpoint-binding helpers
│   ├── macros.rs         # register_apo! / register_aec_apo!
│   ├── raw/              # Low-level COM bindings (Windows-only)
│   │   ├── mod.rs
│   │   ├── abi.rs            # Compile-time ABI invariants
│   │   ├── class_factory.rs  # ApoVTable + ApoClassFactory
│   │   ├── instance_com.rs   # ApoInstanceCom: IAudioProcessingObject
│   │   │                     #   family + IAudioSystemEffects v1/v2/v3
│   │   ├── dispatch.rs       # Shared COM method bodies
│   │   ├── media_type.rs     # IAudioMediaType <-> Format bridge
│   │   ├── reg_properties.rs # APO_REG_PROPERTIES payload builder
│   │   ├── register.rs       # HKCU CLSID registry write/clear
│   │   └── exports.rs        # Dll* dispatch helpers
│   ├── realtime/         # Realtime-safe primitives (cross-platform)
│   │   ├── mod.rs
│   │   ├── context.rs    # RealtimeContext marker type
│   │   ├── ring.rs       # Lock-free SPSC ring buffer
│   │   ├── state.rs      # StateCell lifecycle state machine
│   │   └── refcount.rs   # Atomic COM-style refcount
│   └── aec/              # Windows 11 AEC APO support
│       │                 #   (Windows + `aec` feature)
│       ├── mod.rs            # AecProcessingObject, AecApoInstance<T>,
│       │                     #   AnyAecApoInstance, AuxiliaryInputBuffer
│       ├── class_factory.rs  # AecApoVTable + AecApoClassFactory
│       ├── instance_com.rs   # AecApoInstanceCom: the nine AEC IIDs
│       └── exports.rs        # AEC Dll* dispatch helpers
├── examples/
│   ├── passthrough.rs    # Trivial APO: copies input to output
│   ├── gain.rs           # Fixed linear gain; per-instance state
│   └── aec_scaffold.rs   # AEC APO skeleton (requires `aec` feature)
└── tests/
    ├── realtime_safety.rs    # assert_no_alloc guard on the RT path
    ├── register_apo.rs       # Macro-emitted export wiring
    ├── tier3_lifecycle.rs    # In-process COM activation (SISO)
    └── tier3_aec_lifecycle.rs# In-process COM activation (AEC)
```

## Layer model

Four conceptual layers, isolated by module boundary.

### Layer 1: `raw` — COM bindings

Windows-only (`#[cfg(windows)]`).

- Sole consumer of the `windows` / `windows-core` crates' APO
  interface types and the sole owner of `windows_core::implement`-based
  vtable construction.
- `instance_com::ApoInstanceCom` bridges `Arc<dyn AnyApoInstance>` to
  the `IAudioProcessingObject` family
  (`IAudioProcessingObject`, `IAudioProcessingObjectConfiguration`,
  `IAudioProcessingObjectRT`) plus `IAudioSystemEffects` v1/v2/v3.
- `dispatch` hoists the COM method bodies into free functions over
  `&dyn AnyApoInstance` so the SISO and AEC carriers stay in lock-step
  without copy-pasted impls.
- `class_factory` exposes `ApoVTable` (a CLSID + metadata + creator
  fn) and the `IClassFactory` that mints instances from it.
- `exports` supplies the reusable bodies the macro-emitted `Dll*`
  entry points call into; `register` writes the
  `HKCU\Software\Classes\CLSID\{…}` subtree; `reg_properties` builds
  the variable-length `APO_REG_PROPERTIES` payload; `media_type`
  bridges `IAudioMediaType` to `Format`; `abi` holds compile-time
  `size_of` / `align_of` assertions guarding `windows-rs` layout drift.

Users of `tympan-apo` are not expected to touch this module. It is
`pub` for advanced users and the framework's own test harness.

### Layer 2: `realtime` — zero-allocation primitives

Cross-platform — the realtime invariants do not depend on Windows
APIs, and unit-testing them on any host is more valuable than gating
them behind `#[cfg(windows)]`.

- No allocator use, no `std::sync::Mutex`, no `std::collections`.
- `RealtimeContext` — a zero-sized marker required as a parameter for
  any function safe to call from the realtime `APOProcess` path. It
  cannot be constructed by user code (the framework hands one out by
  reference from its `process` harness), so its presence in a call
  stack is a compile-time witness of realtime safety.
- `ring` — a lock-free single-producer / single-consumer ring buffer.
  `Producer` / `Consumer` are `Send` but not `Sync`; capacity is fixed
  at construction, so `try_push` / `try_pop` are wait-free and
  heap-touch-free.
- `state` — `StateCell`, the atomic lifecycle state machine
  (`Uninitialized → Initialized → Locked`), with bad transitions
  surfaced as `TransitionError` rather than silent corruption.
- `refcount` — `Refcount`, the wait-free atomic counter behind the
  COM `IUnknown` `AddRef` / `Release` contract.

### Layer 3: Public API — safe, idiomatic

This is the layer the large majority of users interact with. It lives
in the crate root and the cross-platform modules `apo`, `buffer`,
`clsid`, `error`, `format`, `instance`, `inf`, and `fx_properties`.

- `ProcessingObject` — the trait users implement (see below).
- `ApoInstance<T>` / `AnyApoInstance` — the framework-side wrapper that
  combines a `StateCell`, a `Refcount`, and an `UnsafeCell<T>` into the
  single object handed to the audio engine. `AnyApoInstance` is the
  type-erased view the COM bridge dispatches through.
- `Format` / `FormatNegotiation` — PCM stream description and the
  Accept / Suggest negotiation result.
- `ProcessInput` / `BufferFlags` / `ConnectionProperty` — the
  per-buffer payload and host flag words.
- `Clsid` / `HResult` — cross-platform GUID and HRESULT value types,
  layout-compatible with their `windows-core` counterparts.

### Layer 4: `aec` — Windows 11 AEC APO support

Gated on `#[cfg(all(windows, feature = "aec"))]` so non-AEC plugins do
not pull in the Windows 11 SDK surface.

- `AecProcessingObject` — extension trait over `ProcessingObject`
  adding the auxiliary-input lifecycle hooks (`add_aux_input`,
  `remove_aux_input`, `is_aux_format_supported`, `accept_aux_input`).
- `AecApoInstance<T>` / `AnyAecApoInstance` — the AEC wrapper, built on
  top of `ApoInstance<T>` so the SISO state machine is reused.
- `AuxiliaryInputBuffer` — the per-buffer reference-signal payload
  delivered to `accept_aux_input` on the realtime thread.
- `class_factory` / `instance_com` / `exports` — the AEC counterparts
  of the `raw` carriers. `AecApoInstanceCom` advertises nine COM
  interfaces: the six SISO interfaces plus
  `IApoAcousticEchoCancellation`, `IApoAuxiliaryInputConfiguration`,
  and `IApoAuxiliaryInputRT`.

## Core abstractions

### `ProcessingObject`

The top-level trait implemented by consumers. Each implementor is one
CLSID-identified APO. The framework's COM harness constructs the type
via `new`, drives the format-negotiation / `LockForProcess` /
`APOProcess` / `UnlockForProcess` sequence, and routes the audio
engine's calls into the trait methods.

```text
pub trait ProcessingObject: Sized + Send {
    const CLSID: Clsid;
    const NAME: &'static str;
    const COPYRIGHT: &'static str;
    const CATEGORY: ApoCategory;          // Sfx / Mfx / Efx

    fn new() -> Self;

    // Format negotiation — defaults accept any IEEE-float32 stream
    // and Suggest a float32 alternative for anything else.
    fn is_input_format_supported(&self, format: &Format) -> FormatNegotiation { … }
    fn is_output_format_supported(&self, format: &Format) -> FormatNegotiation { … }

    // System-effect enumeration / toggling (IAudioSystemEffects2/3).
    // Defaults: no enumerable effects, no-op toggle.
    fn system_effects(&self) -> &[SystemEffect] { &[] }
    fn set_system_effect_state(&mut self, id: &Clsid, state: SystemEffectState) { … }

    // Lifecycle. Pre-allocate in lock_for_process; release in unlock.
    fn lock_for_process(&mut self, input: &Format, output: &Format)
        -> Result<(), HResult> { Ok(()) }
    fn unlock_for_process(&mut self) {}

    // Realtime: allocation-free, lock-free, no syscalls.
    fn process(
        &mut self,
        rt: &RealtimeContext,
        input: ProcessInput<'_>,
        output: &mut [f32],
    ) -> BufferFlags;
}
```

`process` is the only required method past `new` and the associated
constants; everything else has a sensible default. The return value
becomes the `u32BufferFlags` of the host's output
`APO_CONNECTION_PROPERTY`.

The framework emits the COM in-process server entry points via a
macro:

```text
tympan_apo::register_apo!(MyApo);
```

This expands, in the calling crate's root, to the `ApoVTable` static,
a one-entry registry, and the four `#[no_mangle]` `Dll*` exports
(`DllGetClassObject`, `DllCanUnloadNow`, `DllRegisterServer`,
`DllUnregisterServer`) wired to the dispatch helpers in `raw::exports`.
It must be called exactly once per `cdylib` because the emitted
symbols have fixed names.

### `Format` and format negotiation

`Format` mirrors `WAVEFORMATEX` plus the `WAVEFORMATEXTENSIBLE`
extension (channel mask, valid-bits-per-sample, sub-format). Typed
constructors (`pcm_int16`, `pcm_int24`, `pcm_int32`, `pcm_float32`,
`pcm_float64`) produce the base variant; `with_extensible` opts into
the extensible wire format and fills a default channel mask.
`raw::media_type` converts to and from the host's `IAudioMediaType`.

```text
fn is_input_format_supported(&self, format: &Format) -> FormatNegotiation {
    if format.sample_rate() == 48_000 && format.channels() == 1 {
        FormatNegotiation::Accept
    } else {
        FormatNegotiation::Suggest(Format::pcm_float32(48_000, 1))
    }
}
```

### `RealtimeContext`

A zero-sized marker that compile-checks realtime safety. The framework
passes one by reference from its `APOProcess` harness into
`ProcessingObject::process`; it has no fields and no user-reachable
constructor (tests use the crate-private `new_unchecked`).

### `aec::AecProcessingObject`

Extension trait for AEC APOs. Adds the auxiliary-input (reference
stream) lifecycle on top of `ProcessingObject`:

```text
pub trait AecProcessingObject: ProcessingObject {
    fn add_aux_input(&mut self, id: u32, format: &Format, init_data: &[u8])
        -> Result<(), HResult> { Ok(()) }
    fn remove_aux_input(&mut self, id: u32) {}
    fn is_aux_format_supported(&self, format: &Format) -> FormatNegotiation { … }
    fn accept_aux_input(&mut self, rt: &RealtimeContext, input: AuxiliaryInputBuffer<'_>) {}
}
```

All four methods have defaults, so an implementor overrides only what
its echo-cancellation algorithm needs. `accept_aux_input` runs on the
realtime thread and carries the same allocation-free / lock-free
constraints as `process`.

## Cross-cutting concerns

### CLSID allocation

APOs are identified by COM Class IDs. `Clsid` is a cross-platform,
`#[repr(C)]`, GUID-layout-compatible type with `from_u128` /
`from_parts` constructors so authors can declare and unit-test CLSIDs
on any host. `Clsid::NIL` is the sentinel COM rejects as
`CLASS_E_CLASSNOTAVAILABLE`.

### Registration

Three layers of registration helper, increasing in platform
specificity:

- `raw::register` — `DllRegisterServer` / `DllUnregisterServer` write
  and clear the `HKCU\Software\Classes\CLSID\{…}` subtree, so
  `regsvr32 /n /i:user` works without administrative privilege.
- `inf` — `generate(&InfConfig)` emits a minimal INF for production
  drops that integrate with the Windows componentization model.
- `fx_properties` — binds a registered CLSID to a specific audio
  endpoint by writing the `FxProperties` subtree under
  `HKLM\…\MMDevices\Audio`. Requires elevation.

### Realtime logging

Realtime code cannot log via `tracing` or `log` (both allocate). The
`realtime::ring` SPSC buffer is the substrate for the "log from the
realtime thread, drain off-thread" pattern: push a small `Copy` event
from `process`, drain it from a non-realtime thread.

## Resolved design decisions

The questions that were open during the design phase have since been
settled:

- **Aggregation.** APOs are single-input single-output (with optional
  aux inputs in AEC mode). The framework enforces SISO at the type
  level and the class factory rejects aggregation with
  `CLASS_E_NOAGGREGATION`.
- **Minimum Windows version.** MSRV is Rust 1.80, matching the
  `windows` crate. The non-AEC path targets Windows 10+; the `aec`
  feature targets Windows 11 23H2+ and is gated so non-AEC builds do
  not require the newer SDK.
- **AEC reference stream.** The reference (loopback) signal is
  delivered through `IApoAuxiliaryInputRT::AcceptInput` and surfaced
  to user code as `AuxiliaryInputBuffer`. The framework does not open
  its own WASAPI loopback.
- **Signal-processing modes.** APOs declare their slot via
  `ApoCategory` (`Sfx` / `Mfx` / `Efx`); they are otherwise
  mode-agnostic at the framework layer.
- **Dynamic effect on/off.** `IAudioSystemEffects2` /
  `IAudioSystemEffects3` are implemented: `ProcessingObject::system_effects`
  advertises the effect list and `set_system_effect_state` receives
  the engine's toggle calls.

## Known limitations

- `IApoAuxiliaryInputRT::AcceptInput` infers the aux buffer geometry
  from the primary input's locked format. AEC APOs whose aux input
  uses a different format than the primary input need explicit
  per-aux-input format tracking, which is not yet implemented.
- `raw::reg_properties` advertises a fixed interface list per carrier
  (three SISO IIDs, or nine for the AEC carrier). Widening it for an
  APO with a bespoke interface set would require a code change.
- Tier 4 verification — driving a real `audiodg.exe` — cannot run on
  GitHub-hosted runners; it is a manual / self-hosted step. See
  [`testing.md`](testing.md).
