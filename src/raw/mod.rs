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
//! The DLL entry points (`DllGetClassObject`, `DllCanUnloadNow`,
//! `DllRegisterServer`, `DllUnregisterServer`) will live here once
//! they are implemented; for now this module is a placeholder so
//! the cdylib layout is in place.
