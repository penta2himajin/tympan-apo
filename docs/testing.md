# Testing and CI

*Read this in other languages: [日本語](ja/testing.md).*

This document describes the testing and continuous-integration strategy
for `tympan-apo`, including what can be verified automatically on
GitHub-hosted Windows runners, what requires manual or self-hosted
execution, and the constraints imposed by the Windows Audio Engine
architecture.

The decision itself is recorded in
[`docs/decisions/0001-ci-verification-strategy.md`](decisions/0001-ci-verification-strategy.md).
This document is the operational reference for how the tiers run.

## Tiered verification strategy

Verification is organised in four tiers by depth and environment
requirements. Each tier subsumes the previous one. Lower tiers run on
every pull request; higher tiers run on schedule or on demand.

### Tier 1: Static and unit verification

Standard Rust toolchain checks runnable on any GitHub-hosted Windows
runner.

| Check | Command | Purpose |
|---|---|---|
| Build | `cargo build --release --target x86_64-pc-windows-msvc --all-targets` | Compilation across crate features |
| Test | `cargo test` | Unit tests for logic that does not require COM activation |
| Lint | `cargo clippy --all-targets -- -D warnings` | Including project-specific realtime-safety lints |
| Format | `cargo fmt --check` | Style consistency |
| Doc | `cargo doc --no-deps --document-private-items` | Documentation coverage and rustdoc errors |
| No global state | `! git grep -nE 'static\s+mut' -- src/` | Mechanical enforcement of `CLAUDE.md` rule and ADR |

Required on every pull request. Total time: 3-7 minutes on
`windows-2025` / `windows-latest`.

### Tier 2: DLL and COM ABI verification

Verify that the built `cdylib` has the structural properties required
to be a loadable APO COM in-process server.

| Check | Tool | Purpose |
|---|---|---|
| Exported entry points | `dumpbin /exports target\release\*.dll` | `DllGetClassObject`, `DllCanUnloadNow`, `DllRegisterServer`, `DllUnregisterServer` are present and unmangled |
| INF validation | `infverif /v /w packaging\*.inf` | Componentized APO INF correctness (WDK extension) |
| Module dependencies | `dumpbin /dependents` | Only `audioenginebaseapo.lib`, `propsys.lib`, `combase.dll` family — no unexpected user-mode deps |
| ABI sizes | Compile-time `static_assertions` | Bridged struct sizes (`WAVEFORMATEXTENSIBLE`, `APO_CONNECTION_PROPERTY`) match the C definitions |
| Ad-hoc signing smoke test | `signtool sign /fd SHA256 /a /n "Test Cert"` then `signtool verify /pa` | The signing path is wired correctly (does not validate a real cert) |

The ABI size check uses `static_assertions::assert_eq_size!` against
the `windows` crate's generated types, catching layout drift before
runtime.

Runs on every pull request once a buildable `cdylib` example exists.

### Tier 3: In-process COM activation

Drive the full APO lifecycle from a Rust integration test process via
standard COM activation, without involving `audiodg.exe` or any audio
endpoint.

This tier is available on Windows because the
[official Microsoft documentation][impl-apo] confirms the audio engine
itself activates APOs via `CoCreateInstance`, and the four lifecycle
methods (`CoCreateInstance`, `IsInputFormatSupported`,
`IsOutputFormatSupported`, `LockForProcess`) are the same methods the
audio engine drives. A test harness using `windows` crate or
`libloading` can reproduce that drive sequence.

[impl-apo]: https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/implementing-audio-processing-objects

Sequence per test:

1. `regsvr32 /s target\release\example_apo.dll` (per-user hive; no
   admin required)
2. Test process calls `CoInitializeEx(COINIT_MULTITHREADED)`
3. `CoCreateInstance(CLSID_EXAMPLE_APO, ..., IID_IAudioProcessingObject)`
4. Drive: `Initialize` → `IsInputFormatSupported` →
   `LockForProcess` → `APOProcess` (loop with synthetic buffers) →
   `UnlockForProcess`
5. `APOProcess` invocations run under an `assert_no_alloc` global
   allocator guard. Any allocation fails the test, mechanically
   enforcing `CLAUDE.md` prohibition #1.
6. Output buffer assertions: no `NaN`, no `±Inf`, and per-plugin
   analytic bounds (e.g. for the `gain` example, output RMS = input
   RMS × gain factor).
7. `regsvr32 /u /s` cleanup in a teardown step.

Additional Tier 3 jobs:

- **AEC APO variant**: same sequence but using
  `IApoAcousticEchoCancellation` and a synthetic auxiliary input
  stream. Gated behind the `aec` cargo feature.
- **Failure-counter behaviour**: deliberately return failure HRESULT
  from `IsInputFormatSupported` 10 times and verify the framework
  emits a warning. This is the threshold at which the real audio
  engine sets `PKEY_Endpoint_Disable_SysFx`.
- **AddressSanitizer**: nightly Rust with
  `RUSTFLAGS="-Zsanitizer=address"`, same fixtures, parallel to the
  main job. Catches FFI-boundary UB.

Runs on every merge to `main` and on a daily schedule.

### Tier 4: Real audio engine integration

Actual loading of the APO into `audiodg.exe` via the real Windows
audio service. Out of scope for standard GitHub-hosted runners
because:

- Windows Server 2025 (the basis for hosted runners) has the Windows
  Audio Service (`AudioSrv`) disabled by default.
- Even when the service is started, hosted runners have no physical
  or virtual audio endpoint to which an APO can be bound via
  `FxProperties`.
- There is no equivalent of macOS's HAL plugin loading model (where
  ad-hoc-signed plugins load under `coreaudiod` on SIP-enabled
  runners) — the audio engine requires an actual `MMDevice` endpoint
  for the FxProperties path.

Performed via:

- Developer's local Windows machine during PR review
- A self-hosted runner on a Windows workstation registered with
  GitHub Actions
- Windows-in-cloud services (Azure Windows 11 desktop, AWS EC2
  Windows) with a virtual audio device installed (VB-CABLE, Scream,
  or similar)

Scope:

- Binding the APO to an endpoint's `FxProperties` registry key
- `Restart-Service AudioSrv` and verifying `audiodg.exe` loads the
  APO without falling back to `PKEY_Endpoint_Disable_SysFx`
- ETW capture: `wpr -start AudioGlitches.wprp` for a representative
  workload, inspecting glitch counts and APO latency
- DAW / Communications-mode application listening tests
- WHQL / EV signing validation against the production audio engine

## GitHub-hosted Windows runners

Current runners (as of May 2026):

| Label | OS | Architecture | Specs | Public-repo cost |
|---|---|---|---|---|
| `windows-2025` / `windows-latest` | Windows Server 2025 (Build 26100) | x86_64 | 4 vCPU, 16 GB RAM, 14 GB disk | Free |
| `windows-2022` | Windows Server 2022 | x86_64 | Same | Free |
| `windows-11-arm` | Windows 11 ARM | arm64 | 4 vCPU, 16 GB RAM | Free |
| `windows-latest-l` (large) | Windows Server 2025 | x86_64 | 8+ vCPU | Paid only |

Public repositories receive unlimited free minutes on standard
runners across all GitHub plans.

`tympan-apo` is a public repository, so standard runner usage is
unconstrained by cost. ARM64 runners enable building the
`aarch64-pc-windows-msvc` cdylib for ARM Windows 11 devices.

### Runner image inventory

GitHub publishes per-image software inventories. The components
relevant to APO development are:

- Visual Studio Enterprise 2022 (17.14+) — full MSVC toolchain
- Windows SDK 10.1.26100.x (Windows 11 24H2) — satisfies the
  Windows 11 AEC APO API requirement (23H2+)
- Windows Driver Kit Visual Studio Extension 10.0.26100.x — provides
  `infverif.exe`, WDK headers for `audioenginebaseapo.h`
- `dumpbin.exe`, `signtool.exe`, `regsvr32.exe`, `reg.exe` in PATH
  via the Visual Studio Developer Command Prompt environment
- Rust stable + components (rustup pre-installed)

No additional Microsoft Partner Center enrolment is required for
ad-hoc signing or unsigned APO loading in a test harness.

## Windows Audio Service considerations

Behaviour observed on Windows Server SKUs used by GitHub-hosted
runners:

| Operation | Available | Notes |
|---|---|---|
| `regsvr32` of an APO DLL | Yes | Per-user hive (`HKCU\Software\Classes`) avoids admin requirement |
| `CoCreateInstance` of an APO CLSID | Yes | Works regardless of Audio Service state |
| Driving `IAudioProcessingObject*` methods | Yes | The interfaces are plain COM; no audio engine involvement needed |
| Starting `AudioSrv` | Possible | `Set-Service -Name AudioSrv -StartupType Automatic; Start-Service AudioSrv` (Server SKUs disable by default) |
| `MMDeviceEnumerator::EnumAudioEndpoints` | Returns 0 endpoints | No physical or virtual sound card present |
| Binding APO to an endpoint via FxProperties | No | Requires an existing `MMDevice` endpoint |
| `audiodg.exe` loading the APO | No | No endpoint, no audiodg pipeline graph |
| WHQL test signing | No | Requires Microsoft HLK server submission |

Key observation: **APO COM activation and lifecycle exercising are
fully functional on hosted runners despite the absence of an audio
endpoint**. This is the technical foundation that makes Tier 3
automation possible — a property the LADSPA and macOS HAL ports of
the framework family do not share to the same degree.

## What cannot be verified on GitHub-hosted runners

Hard limits of the standard runner environment:

- **Audio output to physical speakers** — runners have no audio
  output hardware exposed to applications
- **Microphone capture** — runners have no input devices, including
  for AEC reference-stream testing
- **`audiodg.exe`-level integration** — the audio engine pipeline
  cannot be assembled without an endpoint
- **Long-running stability** — jobs time out at 6 hours; realistic
  stability tests run for days under sustained audio load
- **WHQL-signed driver flow** — WHQL requires Microsoft HLK server
  submission with a paid certificate; CI cannot perform this on
  every commit
- **Windows Update re-registration scenarios** — driver
  re-installation overwrite testing requires a Windows Update event,
  which is not reproducible in CI
- **Communications-mode application behaviour** — Teams, Discord,
  WhatsApp, etc. require a logged-in UI session, which Server runners
  do not provide reliably

These gaps motivate the Tier 4 manual / self-hosted verification
step.

## Self-hosted alternatives

When automated Tier 4 verification is required, options include:

### Self-hosted GitHub Actions runner

A developer's Windows machine registered as a GitHub Actions runner.
Cost-effective for solo development; requires the machine to be
powered on and networked when CI runs.

Public repositories incur no platform fee for self-hosted runners.
Private repositories pay a $0.002/min platform fee starting
March 2026.

Steps to register:

1. Settings > Actions > Runners > New self-hosted runner
2. Run the printed installation script on the target Windows machine
3. Optionally install the runner as a Windows service for auto-start

For Tier 4, the registered machine must additionally have a real or
virtual audio endpoint (VB-CABLE, Scream, or a physical sound
device).

### Windows-in-cloud services

| Service | Model | Approximate cost | When useful |
|---|---|---|---|
| Azure Virtual Desktop / Windows 11 Cloud PC | Hourly | $0.10-0.40/hr | Persistent state, full UI session |
| AWS EC2 Windows (`m5.large` etc.) | Hourly | $0.10-0.20/hr | Ad-hoc verification, virtual audio addable |
| GitHub Actions large Windows runners | Per-minute | $0.016/min | Pipeline-integrated, but no virtual audio |
| Microsoft Dev Box | Monthly | ~$30-100/mo | Persistent dev environment |

These services are appropriate when Tier 4 verification must run
automatically as part of pipelines, and a local developer machine is
insufficient (e.g., for release validation).

## Recommended workflow files

The intended `.github/workflows/` layout once implementation begins:

```
.github/workflows/
├── tier1.yml           # cargo build/test/clippy/fmt/doc on every PR
├── tier2.yml           # DLL exports, INF, ABI sizes on every PR
├── tier3.yml           # In-process COM activation on merge + nightly
└── release.yml         # Tagged release publishing (cargo publish dry-run)
```

Tier 4 is intentionally omitted from the workflow set; it is performed
manually or on a self-hosted runner outside the standard pipeline.

## Realtime safety enforcement in CI

In addition to the lints provided by Clippy, the framework defines a
set of project-specific lints that fail CI when realtime-unsafe
patterns appear in the realtime code paths.

Cross-reference of `CLAUDE.md` prohibitions to enforcement:

| Prohibition | Enforcement | Tier |
|---|---|---|
| 1. No allocation in `APOProcess` | `assert_no_alloc` guard in Tier 3 integration tests | 3 |
| 2. No `std::sync::Mutex::lock()` in realtime | `clippy.toml` `disallowed-methods` for `realtime` module | 1 |
| 3. No async runtimes | `cargo deny bans` (`tokio`, `async-std`) | 1 |
| 4. No external C libs beyond Windows audio | `cargo deny` allow-list of native-dep crates | 1 |
| 5. No `unsafe fn` in public API without docs | `clippy::missing_safety_doc` set to deny | 1 |
| 6. No blocking syscalls in realtime | Tier 4 ETW capture (CI cannot mechanically verify) | 4 |

These lints are enforced via:

- A custom `cargo clippy` configuration (`clippy.toml`) restricting
  the realtime module's allowed dependencies and methods
- `cargo-deny` rules preventing accidental introduction of
  realtime-unsafe transitive dependencies
- Compile-time `#[deny(...)]` directives in module-level attributes

Implementation details will be added once the first realtime module
lands.

## Comparison to sibling tympan crates

| Aspect | tympan-ladspa | tympan-aspl | tympan-apo |
|---|---|---|---|
| Host OS | Linux | macOS | Windows |
| Build/test/lint | Tier 1 | Tier 1 | Tier 1 |
| ABI / bundle validation | Tier 1 (`nm`) | Tier 2 (`plutil`, `lipo`) | Tier 2 (`dumpbin`, `infverif`) |
| Plugin lifecycle on CI | Tier 2 (`applyplugin`) | Tier 3 (HAL load under `coreaudiod`) | Tier 3 (in-process `CoCreateInstance`) |
| Sanitizers on CI | Tier 2 ASan, Tier 3 TSan | (not specified) | Tier 3 ASan |
| Real audio I/O | Out of scope (Tier 4 manual) | Out of scope (Tier 4 manual/self-hosted) | Out of scope (Tier 4 manual/self-hosted) |

The APO port enjoys an unusually clean Tier 3 path because the COM
activation model is decoupled from the audio engine pipeline. The
LADSPA port requires `applyplugin` (an external SDK tool), and the
ASPL port requires a real `coreaudiod` restart with HAL plugin
placement.

## Implementation status

CI is not yet configured. Implementation is planned at the same time
as the first source code is committed. The initial CI configuration
will cover Tiers 1 and 2; Tier 3 will be added once the first
example APO is buildable; Tier 4 remains manual indefinitely or
until a self-hosted runner with audio hardware joins the project.
