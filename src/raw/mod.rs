//! Low-level COM bindings.
//!
//! This module is the sole consumer of the `windows` crate's APO
//! interface types and the sole owner of any `IUnknown` / vtable
//! bookkeeping required to expose Rust types as COM objects.
//!
//! Users of `tympan-apo` are not expected to touch this module; the
//! public API in the crate root wraps it. It is `pub` only for
//! advanced users who need to bypass the higher-level abstractions
//! and for the framework's own test harness.
//!
//! The four standard COM in-process server entry points
//! (`DllGetClassObject`, `DllCanUnloadNow`, `DllRegisterServer`,
//! `DllUnregisterServer`) live in [`exports`]; they are stubs at
//! this stage of the implementation but exported unmangled so that
//! Tier 2 `dumpbin /exports` verification can already run.

pub mod class_factory;
pub mod exports;
pub mod instance_com;
