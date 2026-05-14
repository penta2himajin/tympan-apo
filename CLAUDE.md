# tympan-apo

## Overview

Rust framework for writing Windows Audio Processing Objects (APOs).
The library provides safe abstractions over the APO COM interfaces so
that Rust applications can implement system-effect audio processors
running inside the Windows Audio Engine without using C++.

Detailed design lives under @docs/overview.md and @docs/architecture.md.

## Project Structure

```
src/                     # Public API (high-level, safe)
src/raw/                 # Low-level FFI: COM interface bindings
src/realtime/            # Realtime-safe primitives (lock-free, alloc-free)
src/aec/                 # Windows 11 AEC APO API support
examples/                # Reference APOs (passthrough, gain, aec_scaffold)
tests/                   # Integration tests
docs/                    # Design and references
.github/                 # Issue/PR templates, CI workflows
```

## Development Setup

Required toolchain:

- Rust 1.80+ (matches the `windows` crate's minimum)
- Windows 10 SDK or Windows 11 SDK (depending on AEC APO API usage)
- MSVC toolchain (Visual Studio 2022 or Build Tools)
- Administrator access on the target machine for APO installation
  and registration

For AEC APO API features:

- Windows 11 SDK (build 22000+)
- Target machine running Windows 11

## Build & Test

```bash
cargo build --release --target x86_64-pc-windows-msvc
cargo test
```

The build produces a `cdylib` (`.dll`). To install an APO into the
Windows Audio Engine:

```powershell
# As Administrator
regsvr32.exe target\release\my_apo.dll
# Then update the device's FxProperties registry entries (see docs/architecture.md)
```

Verification: in Sound Settings, the affected device should advertise the
custom effect via the Effects page.

## Development Principles

- **Realtime safety is non-negotiable.** The `APOProcess` callback
  executes on the Windows audio engine's realtime thread. Code in this
  path must be allocation-free, lock-free, and free of system calls.
  Use the `realtime` module primitives.
- **COM done right.** APOs are COM objects with specific aggregation,
  threading model, and IUnknown requirements. The `raw` module
  encapsulates COM bookkeeping; users implement high-level traits.
- **Single-input single-output.** APOs are SISO objects (with optional
  auxiliary inputs in Windows 11 AEC APO mode). The framework enforces
  this at the type level.
- **Format negotiation matters.** APOs negotiate input/output formats
  with the audio engine via `IsInputFormatSupported` /
  `IsOutputFormatSupported`. The framework provides format-matching
  helpers and defaults that work for most use cases.
- **No global state.** APO instances are first-class objects; the
  framework never relies on `static mut` or singletons.

## Architectural Boundaries

- `raw` module is the only place that links to Windows audio COM
  interfaces (`AudioEngineBaseAPO.h`, `audioenginebaseapo.idl`).
- `realtime` module never allocates and never returns `Result` values
  containing `String` or other heap types. Errors are represented as
  `HRESULT` values.
- `aec` module isolates Windows 11 AEC APO API surface (`IApoAux*`,
  `IApoAcousticEchoCancellation*`) so non-AEC plugins do not pull in
  Windows 11 SDK requirements.
- Public API surface lives in `lib.rs` and re-exports from internal
  modules.
- `examples/` plugins must build to `cdylib`. Non-DLL examples belong
  in `tests/` or as doc-tests.

## Prohibitions

1. Do not allocate memory in any function called from `APOProcess` or
   its transitive callees. Pre-allocate buffers during `LockForProcess`.
2. Do not call `std::sync::Mutex::lock()` from realtime code paths.
   Use lock-free primitives (`crossbeam`, atomics) instead.
3. Do not introduce dependencies on async runtimes (`tokio`,
   `async-std`). This is a sync, realtime-oriented library.
4. Do not depend on external C libraries beyond what Windows provides
   (`audioenginebaseapo.lib`, `propsys.lib`, etc.).
5. Do not expose `unsafe fn` in the public API without a clearly
   documented safety contract. Internal `unsafe` is encapsulated
   behind safe wrappers.
6. Do not call any function that might block or wait on I/O from
   realtime code (no `println!`, no file I/O, no allocator calls).

## Git Conventions

- Scoped Conventional Commits: `feat(raw):`, `fix(realtime):`,
  `feat(aec):`, `docs(arch):`.
- Scopes follow the module structure: `raw`, `realtime`, `api`, `aec`,
  `examples`, `docs`, `meta` (CI, README, license).
- Breaking changes use `!` notation and require a corresponding entry
  in `docs/decisions/` (when that directory exists).
- PRs link a handoff issue with `Closes #N` or `Refs #N`.

## Session Handoff

Long-running workstreams use GitHub issues for cross-session continuity.
See @docs/handoff-protocol.md for the full protocol.

- Label: `session-handoff`
- One issue per workstream (not per session)
- On session start, read the relevant handoff issue and confirm the
  **Next action** with the user before executing.
