//! `tympan-apo` — Rust framework for Windows Audio Processing Objects.
//!
//! See `docs/overview.md` and `docs/architecture.md` for the design
//! that drives this crate. This is the initial skeleton: only the
//! module layout and a small number of marker types exist so far.
//!
//! The crate is organised into four conceptual layers, isolated by
//! module boundary:
//!
//! - `raw` — low-level COM bindings via the `windows` crate
//!   (Windows-only).
//! - [`realtime`] — allocation-free, lock-free primitives intended
//!   for use from the `APOProcess` realtime callback. Cross-platform
//!   so that the realtime invariants can be unit-tested on any host.
//! - Public API (this module) — safe, idiomatic wrappers users
//!   implement against. Currently empty.
//! - `aec` — Windows 11 Acoustic Echo Cancellation APO support.
//!   Windows-only and gated behind the `aec` Cargo feature.
//!
//! ## Realtime safety
//!
//! Any code reachable from the `APOProcess` callback must be
//! allocation-free, lock-free, and free of blocking syscalls. The
//! [`realtime`] module exposes a `RealtimeContext` marker that acts
//! as a compile-time witness for the realtime context.
//!
//! ## Status
//!
//! Design phase. The COM bindings, lifecycle harness, registration
//! helpers, and reference example APOs are not yet implemented.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod apo;
pub mod error;
pub mod format;
pub mod realtime;

#[cfg(windows)]
pub mod raw;

#[cfg(all(windows, feature = "aec"))]
#[cfg_attr(docsrs, doc(cfg(all(windows, feature = "aec"))))]
pub mod aec;

pub use apo::ApoCategory;
pub use error::HResult;
pub use format::{Format, FormatNegotiation};
