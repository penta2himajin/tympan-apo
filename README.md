# tympan-apo

*Read this in other languages: [日本語](README.ja.md).*

A Rust framework for writing Windows Audio Processing Objects (APOs).

`tympan-apo` provides Rust abstractions over the Windows Audio Processing
Object COM interfaces, enabling Rust applications to implement custom
system-effect audio processors (SFX, MFX) that run inside the Windows
Audio Engine without writing C++.

Special support is included for the Windows 11 AEC APO API, allowing
Rust code to participate in the official acoustic-echo-cancellation
processing slot in the microphone capture pipeline.

## Status

**Intended functionality complete.** Every "In scope" item from
[`docs/overview.md`](docs/overview.md) is implemented:

- COM object infrastructure — IUnknown, IClassFactory, registration,
  and the four `Dll*` exports ([`src/raw/`](src/raw/)).
- The required interface set — `IAudioProcessingObject` family plus
  `IAudioSystemEffects` v1/v2/v3 — and the SFX, MFX, and EFX
  categories.
- AEC APO support ([`src/aec/`](src/aec/)): the `AecProcessingObject`
  foundation, the COM bridge (`AecApoInstanceCom` + `register_aec_apo!`),
  and the `aec_scaffold` example, gated behind the `aec` feature.
- Format negotiation for both `WAVEFORMATEX` and
  `WAVEFORMATEXTENSIBLE` ([`src/format.rs`](src/format.rs)).
- Realtime-safe primitives ([`src/realtime/`](src/realtime/)): a
  lock-free ring buffer, atomic state helpers, and an atomic refcount.
- Registration helpers ([`src/clsid.rs`](src/clsid.rs),
  [`src/raw/register.rs`](src/raw/register.rs),
  [`src/inf.rs`](src/inf.rs),
  [`src/fx_properties.rs`](src/fx_properties.rs)): CLSID assignment,
  registry writes, an INF generator, and FxProperties endpoint binding.
- Example APOs under [`examples/`](examples/): `passthrough`, `gain`,
  and `aec_scaffold`.

CI runs Tier 1 (fmt, clippy, build/test), Tier 2 (multi-DLL export,
dependency, and signing verification), and Tier 3 (the COM lifecycle
harness, including the AEC variant, plus an AddressSanitizer nightly).
See [`docs/architecture.md`](docs/architecture.md) for the API design
and [`docs/testing.md`](docs/testing.md) for the CI strategy.

## Naming

*Tympan* — the tympanal organ of moths, a membrane-based ultrasound sensor
on the abdomen of pyralid and noctuid moths. Evolved to detect the
echolocation calls of bats. The name reflects the library's role: a thin
membrane between the OS audio engine and user-space Rust code.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.

## Documentation

| Doc | Content |
|---|---|
| [`docs/overview.md`](docs/overview.md) | Project purpose, scope, comparison to existing implementations |
| [`docs/architecture.md`](docs/architecture.md) | API design and module layout |
| [`docs/references.md`](docs/references.md) | Microsoft documentation, prior art, related crates |
| [`docs/testing.md`](docs/testing.md) | Testing and CI strategy across GitHub-hosted Windows runners |
| [`docs/decisions/`](docs/decisions/) | Architecture Decision Records (ADRs) |
| [`docs/handoff-protocol.md`](docs/handoff-protocol.md) | Session handoff protocol for long-running work |
