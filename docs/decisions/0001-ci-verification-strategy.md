# ADR 0001: CI verification strategy and scope boundary

- Status: Accepted
- Date: 2026-05-13

## Context

The framework imposes strong invariants — allocation-free `APOProcess`,
no mutex in the realtime path, no global state, no blocking syscalls
from realtime code. `CLAUDE.md` lists six explicit prohibitions. The
question is which of these can be verified mechanically on
GitHub-hosted Windows runners, and which must fall back to local or
self-hosted testing.

A preliminary investigation surveyed three classes of tooling:

1. The Windows SDK / WDK utilities (`infverif`, `dumpbin`, `signtool`)
   and standard COM activation tooling (`regsvr32`, `CoCreateInstance`).
2. The Windows Audio Engine itself: loading APOs into `audiodg.exe`
   via `FxProperties` registry binding, requiring the Windows Audio
   Service and a real `MMDevice` endpoint.
3. Runtime instrumentation (`assert_no_alloc`, AddressSanitizer,
   ETW / WPR tracing).

Findings:

- **APOs are plain in-process COM servers.** The
  [official Microsoft documentation](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/implementing-audio-processing-objects)
  confirms the audio engine itself activates APOs via
  `CoCreateInstance`. A test harness can drive
  `Initialize → IsInputFormatSupported → LockForProcess → APOProcess →
  UnlockForProcess` from a Rust integration test without involving
  `audiodg.exe`. This is the equivalent of LADSPA's `applyplugin`
  but with no external SDK tool required.
- **GitHub-hosted Windows Server 2025 runners** include Visual Studio
  Enterprise 2022 and the Windows Driver Kit extension out of the box.
  `dumpbin`, `infverif`, `signtool`, `regsvr32` are all in PATH via
  the Developer Command Prompt environment. The Windows SDK version
  (10.1.26100.x, Windows 11 24H2) satisfies the Windows 11 AEC APO
  API requirement (23H2+).
- **`audiodg.exe`-level loading is not available on hosted runners.**
  Windows Server SKUs have the Windows Audio Service disabled by
  default, and hosted runners expose no physical or virtual audio
  endpoint to bind APOs to via `FxProperties`. There is no analogue
  of macOS's HAL plugin loading (where unsigned plugins load under
  `coreaudiod` on SIP-enabled hosted runners).
- **`assert_no_alloc`** is a global-allocator wrapper that fails
  allocations within a guarded scope. Inserting it into an
  integration test that calls `APOProcess` mechanically enforces
  prohibition 1 from `CLAUDE.md`.
- **AddressSanitizer** runs under `cargo test` on nightly Rust and
  surfaces FFI-boundary UB across the COM boundary. ThreadSanitizer
  on Windows has historically been less mature; we treat it as
  optional rather than required.
- **WHQL / EV signing** requires Microsoft HLK server submission with
  a paid certificate. CI cannot meaningfully perform this on every
  commit; it belongs to release procedures.

Industry baseline observed in adjacent Rust audio projects (`cpal`,
`wasapi-rs`, `windows-rs` consumers): CI typically stops at
`cargo build` and `cargo test`, with no virtual audio device setup and
no plugin lifecycle exercising. Adopting in-process COM activation
already puts this project above that baseline, and on par with the
deeper tiers of `tympan-aspl` and `tympan-ladspa`.

## Decision

CI verification is organised in four tiers. Each tier defines what
runs on which trigger and what is intentionally out of scope. The
operational details (commands, runner labels, fixtures) live in
[`docs/testing.md`](../testing.md); the tier boundaries below are
authoritative.

### Tier 1 — `static` (every PR push, target < 7 min)

- `cargo build --release --target x86_64-pc-windows-msvc --all-targets`
- `cargo test` for pure-Rust unit tests
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo doc --no-deps --document-private-items`
- A `git grep` for `static mut` in `src/` enforcing the no-global-state
  invariant
- `cargo deny` and project-specific clippy configuration enforcing
  prohibitions 2-5 from `CLAUDE.md`

This tier blocks merge.

### Tier 2 — `abi` (every PR push, target < 10 min)

Tier 1 plus:

- `dumpbin /exports` verifying `DllGetClassObject`, `DllCanUnloadNow`,
  `DllRegisterServer`, `DllUnregisterServer` are present and
  unmangled
- `infverif /v /w` over committed INF files
- `dumpbin /dependents` against an allow-list of acceptable Windows
  audio DLL dependencies
- Compile-time `static_assertions` for COM struct sizes
  (`WAVEFORMATEXTENSIBLE`, `APO_CONNECTION_PROPERTY`, etc.)
- `signtool sign` with a generated ad-hoc test certificate, then
  `signtool verify /pa` — validates the signing wiring without
  requiring a real cert

This tier blocks merge.

### Tier 3 — `lifecycle` (merge to `main` and nightly schedule, target < 25 min)

Tier 2 plus:

- `regsvr32 /s` per-user registration of each example APO cdylib
- A Rust integration test that `CoCreateInstance`s each registered
  CLSID and drives the full lifecycle (`Initialize` →
  `IsInputFormatSupported` → `LockForProcess` → `APOProcess` ×N →
  `UnlockForProcess`) against synthetic float32 buffers
- All `APOProcess` invocations run under an `assert_no_alloc` global
  allocator guard. Any allocation fails the build.
- Output assertions: no `NaN`, no `±Inf`, plus per-plugin analytic
  bounds (e.g. unit-gain APO must produce output bitwise-equal to
  input).
- The AEC variant runs the same sequence using
  `IApoAcousticEchoCancellation` with a synthetic auxiliary input
  stream, gated behind the `aec` cargo feature.
- An AddressSanitizer-enabled job (`RUSTFLAGS="-Zsanitizer=address"`,
  nightly) running the same fixtures. It runs as a pre-release audit
  on `v*.*.*` tag pushes and on manual dispatch — not on a daily
  schedule, which produced noise rather than signal since `main`
  rarely changes between releases. Non-blocking: it does not gate the
  release publish.

This tier does not block PR merge but its failure on `main` opens an
issue automatically (or notifies, depending on later infra choices).

### Tier 4 — out of CI scope

The following are *not* tested on GitHub-hosted runners. They are
documented in `docs/testing.md` (§ Tier 4) as a pre-release manual
checklist:

- Loading the APO into `audiodg.exe` via `FxProperties` binding on a
  real or virtual audio endpoint
- WHQL / EV signed driver flow
- Communications-mode application listening tests (Teams, Discord,
  etc.)
- xrun / glitch rates under sustained load (ETW / WPR capture)
- Behaviour across Windows Update audio-driver re-installation
- Long-running stability (jobs > 6 hours)

If a self-hosted Windows runner with a virtual audio device becomes
available later, these may be promoted into a Tier 4 CI workflow
without breaking changes to tiers 1-3.

## Consequences

Positive:

- Prohibitions 1-5 from `CLAUDE.md` become mechanically enforceable
  on every PR or nightly run.
- Tier 3's in-process activation catches the most common regression
  modes (CLSID misregistration, vtable layout drift, lifecycle
  ordering bugs, realtime-path allocations) on `main` and nightly
  without requiring any Windows infrastructure beyond a hosted
  runner.
- AddressSanitizer catches FFI-boundary UB before it ships, which is
  the hardest class of bug to debug under `audiodg.exe`.
- The strategy maps cleanly onto the sibling tympan crates'
  tier-numbering conventions, so contributors moving between crates
  see consistent CI semantics.

Negative:

- `audiodg.exe`-level integration bugs (interaction with the audio
  engine's format negotiation graph, processing-mode selection,
  endpoint property store) are not caught until manual verification
  or user reports.
- The framework cannot mechanically detect a regression that depends
  on a real `MMDevice` endpoint's behaviour (e.g. an APO that
  misbehaves only when bound to a Bluetooth A2DP endpoint).
- WHQL-related issues surface only at release-time signing, not on
  per-PR CI.

## Trigger for revisiting

Re-evaluate this strategy when any of the following holds:

- A self-hosted Windows runner with a virtual audio device (or with
  a real sound card) becomes part of the project's CI budget. At
  that point, Tier 4 is promoted into a workflow.
- A bug ships that would have been caught by `audiodg.exe`-level
  loading but was not caught by Tier 3 in-process activation. Document
  the case and decide whether to move audiodg-level testing into a
  scheduled job on a self-hosted runner.
- Microsoft adds a first-party APO test host (the Windows SDK does
  not currently ship one comparable to LADSPA's `applyplugin`); if
  that lands, evaluate whether it replaces or supplements the
  in-process lifecycle harness.
- ThreadSanitizer on Windows matures to the point where it can be
  used in Tier 3 reliably, in which case a multi-instance concurrent
  `APOProcess` test should be added.

## References

- Microsoft Learn: *Audio Processing Object Architecture*
  - <https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/audio-processing-object-architecture>
- Microsoft Learn: *Implementing Audio Processing Objects* (confirms
  CoCreateInstance-based activation by the audio engine)
  - <https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/implementing-audio-processing-objects>
- Microsoft Learn: *Windows 11 APIs for Audio Processing Objects*
  - <https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/windows-11-apis-for-audio-processing-objects>
- GitHub-hosted Windows Server 2025 runner image inventory
  - <https://github.com/actions/runner-images/blob/main/images/windows/Windows2025-Readme.md>
- `assert_no_alloc` crate
  - <https://docs.rs/assert_no_alloc>
- Sibling tympan CI strategies:
  - `tympan-ladspa` ADR 0005 (LADSPA, `applyplugin`-based Tier 2)
  - `tympan-aspl` `docs/testing.md` (macOS, `coreaudiod` HAL load
    Tier 3)
- Adjacent Rust audio CI baselines: `cpal`
  (<https://github.com/RustAudio/cpal/blob/master/.github/workflows/platforms.yml>),
  `nih-plug`, `rust-lv2`.
