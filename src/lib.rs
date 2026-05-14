//! `tympan-apo` — Rust framework for Windows Audio Processing Objects.
//!
//! See `docs/overview.md` and `docs/architecture.md` for the design
//! that drives this crate.
//!
//! The crate is organised into four conceptual layers, isolated by
//! module boundary:
//!
//! - `raw` — low-level COM bindings via the `windows` crate
//!   (Windows-only).
//! - [`realtime`] — allocation-free, lock-free primitives intended
//!   for use from the `APOProcess` realtime callback. Cross-platform
//!   so that the realtime invariants can be unit-tested on any host.
//! - Public API (this module plus [`apo`], [`instance`], [`format`],
//!   and the other crate-root modules) — safe, idiomatic wrappers
//!   users implement against.
//! - `aec` — Windows 11 Acoustic Echo Cancellation APO support.
//!   Windows-only and gated behind the `aec` Cargo feature.
//!
//! ## Realtime safety
//!
//! Any code reachable from the `APOProcess` callback must be
//! allocation-free, lock-free, and free of blocking syscalls. The
//! [`realtime`] module exposes a `RealtimeContext` marker that acts
//! as a compile-time witness for the realtime context.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod apo;
pub mod buffer;
pub mod clsid;
pub mod error;
pub mod format;
pub mod fx_properties;
pub mod inf;
pub mod instance;
pub mod realtime;

#[cfg(windows)]
pub mod raw;

#[cfg(all(windows, feature = "aec"))]
#[cfg_attr(docsrs, doc(cfg(all(windows, feature = "aec"))))]
pub mod aec;

pub use apo::{ApoCategory, ProcessInput, ProcessingObject, SystemEffect, SystemEffectState};
pub use buffer::{BufferFlags, ConnectionProperty, CONNECTION_PROPERTY_SIGNATURE};
pub use clsid::Clsid;
pub use error::HResult;
pub use format::{Format, FormatNegotiation};

/// Re-export of `windows_core::GUID` so the `register_apo!` macro's
/// emitted entry-point signatures resolve without users having to
/// add `windows-core` to their own `Cargo.toml`.
#[cfg(windows)]
pub use windows_core::GUID;

/// Re-export of `windows_core::HRESULT`. Same rationale as [`GUID`].
#[cfg(windows)]
pub use windows_core::HRESULT;

#[cfg(windows)]
mod macros;
