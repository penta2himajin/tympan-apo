//! AEC counterpart of [`crate::raw::class_factory`].
//!
//! [`AecApoVTable`] describes one AEC APO; [`AecApoClassFactory`]
//! is the `IClassFactory` that `register_aec_apo!` instantiates
//! per `DllGetClassObject` call. Mirrors the SISO surface line for
//! line; the only differences are the carrier type
//! ([`crate::aec::instance_com::AecApoInstanceCom`]) and the
//! instance constructor signature
//! (`fn() -> Arc<dyn AnyAecApoInstance>`).

// `windows_core::implement` proc-macro generates a sibling
// `*_Impl` struct without doc-comments; the crate-wide
// `#![deny(missing_docs)]` would otherwise reject the expansion.
#![allow(missing_docs)]

extern crate alloc;

use alloc::sync::Arc;
use core::ffi::c_void;

use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows_core::{implement, ComObject, IUnknown, Interface, Ref, BOOL, GUID, HRESULT};

use crate::aec::instance_com::AecApoInstanceCom;
use crate::aec::AnyAecApoInstance;
use crate::apo::ApoCategory;
use crate::clsid::Clsid;
use crate::error::HResult;
use crate::realtime::Refcount;

/// VTable-style metadata describing one AEC APO.
///
/// Emitted as a `'static` constant by the `register_aec_apo!`
/// macro. Parallels [`crate::raw::class_factory::ApoVTable`] but
/// the `create` function pointer returns an
/// `Arc<dyn AnyAecApoInstance>` so the AEC class factory can mint
/// the matching [`AecApoInstanceCom`] carrier.
pub struct AecApoVTable {
    /// The CLSID this factory answers to (`T::CLSID`).
    pub clsid: Clsid,
    /// Human-readable name (`T::NAME`).
    pub name: &'static str,
    /// Copyright string (`T::COPYRIGHT`).
    pub copyright: &'static str,
    /// Category (`T::CATEGORY`).
    pub category: ApoCategory,
    /// Type-erased instance creator. Calls `T::new` internally and
    /// returns the resulting `AecApoInstance<T>` wrapped in
    /// `Arc<dyn AnyAecApoInstance>`.
    pub create: fn() -> Arc<dyn AnyAecApoInstance>,
}

impl core::fmt::Debug for AecApoVTable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AecApoVTable")
            .field("clsid", &self.clsid)
            .field("name", &self.name)
            .field("copyright", &self.copyright)
            .field("category", &self.category)
            .finish_non_exhaustive()
    }
}

/// AEC COM class factory.
///
/// Owns a [`Refcount`] tracking `LockServer(TRUE/FALSE)` calls and
/// a reference to the static [`AecApoVTable`] describing the AEC
/// APO the factory creates.
#[implement(IClassFactory)]
pub struct AecApoClassFactory {
    server_lock: Refcount,
    vtable: &'static AecApoVTable,
}

impl AecApoClassFactory {
    /// Construct a factory bound to the given static
    /// [`AecApoVTable`].
    #[must_use]
    pub const fn new(vtable: &'static AecApoVTable) -> Self {
        Self {
            server_lock: Refcount::new(),
            vtable,
        }
    }

    /// Current `LockServer` count.
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

impl IClassFactory_Impl for AecApoClassFactory_Impl {
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn CreateInstance(
        &self,
        punkouter: Ref<IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut c_void,
    ) -> windows_core::Result<()> {
        if ppvobject.is_null() {
            return Err(windows_core::Error::new(
                HRESULT::from(HResult::E_POINTER),
                "ppvobject is null",
            ));
        }
        // Safety: ppvobject is non-null per the check above.
        unsafe {
            *ppvobject = core::ptr::null_mut();
        }
        if !punkouter.is_null() {
            return Err(windows_core::Error::new(
                HRESULT::from(HResult::CLASS_E_NOAGGREGATION),
                "AecApoClassFactory does not support aggregation",
            ));
        }

        let inner = (self.vtable.create)();
        let com_object = ComObject::new(AecApoInstanceCom::new(inner));
        let unknown: IUnknown = com_object.into_interface();
        // Safety: `unknown` is a valid IUnknown pointer; the COM
        // caller guarantees `riid` and `ppvobject` are valid.
        unsafe { unknown.query(riid, ppvobject) }.ok()
    }

    fn LockServer(&self, flock: BOOL) -> windows_core::Result<()> {
        if flock.as_bool() {
            self.server_lock.add_ref();
        } else {
            self.server_lock.release();
        }
        Ok(())
    }
}
