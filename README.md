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

**Design phase.** No implementation yet. See [`docs/overview.md`](docs/overview.md)
for planned scope and [`docs/architecture.md`](docs/architecture.md) for the
planned API design.

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
| [`docs/architecture.md`](docs/architecture.md) | Planned API design and module layout |
| [`docs/references.md`](docs/references.md) | Microsoft documentation, prior art, related crates |
| [`docs/handoff-protocol.md`](docs/handoff-protocol.md) | Session handoff protocol for long-running work |
