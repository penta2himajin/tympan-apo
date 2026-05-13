//! COM class factory exposing a `T: ProcessingObject` to the
//! Windows audio engine.
//!
//! The audio engine resolves the factory by CLSID
//! (`DllGetClassObject` → factory), calls
//! `IClassFactory::CreateInstance` to materialise an APO, and
//! optionally uses `IClassFactory::LockServer` to keep the DLL
//! loaded even when no APO instances are outstanding.
//!
//! ## Why no generics on the factory struct
//!
//! The `windows_core::implement` proc-macro does not support
//! generic parameters on the implementing struct. To bind a
//! `T: ProcessingObject` we instead carry it indirectly: each
//! factory stores a `&'static ApoVTable` describing how to mint a
//! `T` instance and which CLSID it answers to. A `register_apo!`
//! macro (follow-up PR) will emit one such `ApoVTable` per
//! user-defined APO and a paired `ApoClassFactory` constructor.
//!
//! ## Current state
//!
//! `CreateInstance` returns `CLASS_E_CLASSNOTAVAILABLE` while the
//! IUnknown wrapper for [`crate::instance::ApoInstance`] is still
//! under construction. `LockServer` is functional and increments
//! the per-factory `server_lock` counter that
//! `DllCanUnloadNow` will consult.

// The `windows_core::implement` proc-macro generates a sibling
// `*_Impl` struct that does not carry doc-comments; the crate-wide
// `#![deny(missing_docs)]` would otherwise reject the expansion.
#![allow(missing_docs)]

use core::ffi::c_void;

use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows_core::{implement, IUnknown, Ref, BOOL, GUID, HRESULT};

use crate::clsid::Clsid;
use crate::error::HResult;
use crate::realtime::Refcount;

/// VTable-style metadata describing one user APO.
///
/// One static instance per user-implementor will be emitted by the
/// future `register_apo!` macro. The factory points to the table
/// rather than carrying a generic parameter, side-stepping
/// `windows_core::implement`'s lack of generics support.
#[derive(Debug)]
pub struct ApoVTable {
    /// The CLSID this factory answers to.
    pub clsid: Clsid,
    /// Human-readable name (`T::NAME`).
    pub name: &'static str,
    /// Copyright string (`T::COPYRIGHT`).
    pub copyright: &'static str,
}

/// COM class factory.
///
/// Owns a [`Refcount`] tracking `LockServer(TRUE/FALSE)` calls and
/// a reference to the static [`ApoVTable`] describing the APO the
/// factory creates.
#[implement(IClassFactory)]
pub struct ApoClassFactory {
    server_lock: Refcount,
    vtable: &'static ApoVTable,
}

impl ApoClassFactory {
    /// Construct a factory bound to the given static [`ApoVTable`].
    /// The factory will only accept CLSIDs matching
    /// `vtable.clsid`.
    #[must_use]
    pub const fn new(vtable: &'static ApoVTable) -> Self {
        Self {
            server_lock: Refcount::new(),
            vtable,
        }
    }

    /// Current server-lock count (the number of outstanding
    /// `IClassFactory::LockServer(TRUE)` calls minus `LockServer(FALSE)`
    /// calls). Consulted by the future `DllCanUnloadNow` wiring.
    #[inline]
    #[must_use]
    pub fn server_lock_count(&self) -> u32 {
        self.server_lock.count()
    }

    /// CLSID this factory answers to.
    #[inline]
    #[must_use]
    pub fn clsid(&self) -> Clsid {
        self.vtable.clsid
    }
}

impl IClassFactory_Impl for ApoClassFactory_Impl {
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn CreateInstance(
        &self,
        _punkouter: Ref<IUnknown>,
        _riid: *const GUID,
        ppvobject: *mut *mut c_void,
    ) -> windows_core::Result<()> {
        // Zero the out-pointer per the COM contract on failure.
        if !ppvobject.is_null() {
            // Safety: the COM caller guarantees `ppvobject` points
            // to a writable pointer slot.
            unsafe {
                *ppvobject = core::ptr::null_mut();
            }
        }
        Err(windows_core::Error::new(
            HRESULT::from(HResult::CLASS_E_CLASSNOTAVAILABLE),
            "ApoInstance IUnknown wrapper not yet implemented",
        ))
    }

    fn LockServer(&self, flock: BOOL) -> windows_core::Result<()> {
        if flock.as_bool() {
            self.server_lock.add_ref();
        } else {
            // LockServer(FALSE) decrements. COM contracts every
            // TRUE to be paired with a matching FALSE.
            self.server_lock.release();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static DUMMY_VT: ApoVTable = ApoVTable {
        clsid: Clsid::from_u128(0xABCDEF01_2345_6789_0123_456789ABCDEF),
        name: "dummy",
        copyright: "test",
    };

    #[test]
    fn new_factory_has_zero_server_lock_and_records_clsid() {
        let f = ApoClassFactory::new(&DUMMY_VT);
        assert_eq!(f.server_lock_count(), 0);
        assert_eq!(f.clsid(), DUMMY_VT.clsid);
    }
}
