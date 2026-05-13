//! Realtime-safe primitives.
//!
//! Everything in this module must be allocation-free and lock-free.
//! In particular:
//!
//! - No `std::sync::Mutex`, `std::collections::HashMap`, `Vec::push`,
//!   `Box::new`, or other allocator-touching operations may appear
//!   in code reachable from `crate::raw`'s `APOProcess` callback.
//! - Errors are represented as `HRESULT`-style integers, never as
//!   heap-allocated `String`s.
//!
//! The module is intentionally cross-platform: the realtime
//! invariants do not depend on Windows-specific APIs, and being able
//! to unit-test them on any host is more valuable than gating them
//! behind `#[cfg(windows)]`.

mod context;

pub use context::RealtimeContext;
