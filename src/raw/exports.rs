//! Shared helpers for the COM in-process server entry points.
//!
//! Every APO `.dll` exports the four standard COM functions
//! `DllGetClassObject`, `DllCanUnloadNow`, `DllRegisterServer`,
//! and `DllUnregisterServer`. Those exports are emitted by the
//! [`crate::register_apo!`] macro at the user's crate-root scope —
//! the framework crate itself does not produce them, since the
//! macro's emitted exports would otherwise collide with framework
//! ones at link time.
//!
//! This module supplies the reusable building block the macro's
//! emitted `DllGetClassObject` calls into:
//! [`dll_get_class_object_dispatch`] — a CLSID-to-factory lookup
//! that materialises an [`ApoClassFactory`] and routes the
//! requested IID through `IUnknown::QueryInterface`.

use core::ffi::c_void;

use windows_core::{ComObject, IUnknown, Interface, GUID, HRESULT};

use crate::clsid::Clsid;
use crate::error::HResult;
use crate::raw::class_factory::{ApoClassFactory, ApoVTable};

/// CLSID → factory dispatch shared by every user-emitted
/// `DllGetClassObject`.
///
/// Looks up `rclsid` in `registry`, materialises an
/// [`ApoClassFactory`] for the matching [`ApoVTable`], wraps it in
/// a COM object, and routes the requested `riid` through
/// `IUnknown::QueryInterface`. Returns `CLASS_E_CLASSNOTAVAILABLE`
/// if the CLSID is not registered, `E_POINTER` if `ppv` (or
/// `rclsid` / `riid`) is null.
///
/// # Safety
///
/// Called from COM entry points. The caller must guarantee:
///
/// - `rclsid` points to a valid `GUID` for the lifetime of this
///   call (or is null, in which case the function returns
///   `E_POINTER`).
/// - `riid` points to a valid `GUID` for the lifetime of this
///   call (or is null, in which case the function returns
///   `E_POINTER`).
/// - `ppv` points to a writable `*mut c_void` slot, or is null
///   (in which case the function returns `E_POINTER` without
///   dereferencing it).
pub unsafe fn dll_get_class_object_dispatch(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
    registry: &[&'static ApoVTable],
) -> HRESULT {
    if ppv.is_null() {
        return HResult::E_POINTER.into();
    }
    // Safety: ppv is non-null per the check above.
    unsafe {
        *ppv = core::ptr::null_mut();
    }
    if rclsid.is_null() || riid.is_null() {
        return HResult::E_POINTER.into();
    }
    // Safety: caller guarantees rclsid points to a valid GUID.
    let requested = Clsid::from(unsafe { *rclsid });

    let Some(vtable) = registry.iter().find(|v| v.clsid == requested) else {
        return HResult::CLASS_E_CLASSNOTAVAILABLE.into();
    };

    let factory = ApoClassFactory::new(vtable);
    let com = ComObject::new(factory);
    let unknown: IUnknown = com.into_interface();
    // Safety: unknown is a valid IUnknown pointer; the COM
    // caller guarantees `riid` and `ppv` are valid.
    unsafe { unknown.query(riid, ppv) }
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::sync::Arc;

    use super::*;
    use crate::apo::{ApoCategory, ProcessInput, ProcessingObject};
    use crate::buffer::BufferFlags;
    use crate::instance::{AnyApoInstance, ApoInstance};
    use crate::realtime::RealtimeContext;

    struct Dummy;
    impl ProcessingObject for Dummy {
        const CLSID: Clsid = Clsid::from_u128(0xA0A0A0A0_0000_0000_0000_0000A0A0A0A0);
        const NAME: &'static str = "dummy";
        const COPYRIGHT: &'static str = "test";
        const CATEGORY: ApoCategory = ApoCategory::Sfx;
        fn new() -> Self {
            Self
        }
        fn process(
            &mut self,
            _rt: &RealtimeContext,
            input: ProcessInput<'_>,
            output: &mut [f32],
        ) -> BufferFlags {
            output.copy_from_slice(input.samples());
            input.flags()
        }
    }

    fn dummy_create() -> Arc<dyn AnyApoInstance> {
        Arc::new(ApoInstance::<Dummy>::new())
    }

    static DUMMY_VT: ApoVTable = ApoVTable {
        clsid: Dummy::CLSID,
        name: Dummy::NAME,
        copyright: Dummy::COPYRIGHT,
        category: Dummy::CATEGORY,
        create: dummy_create,
    };

    /// Driver: invokes `dll_get_class_object_dispatch` with COM-side
    /// pointer plumbing so each test can assert on the resulting
    /// HRESULT and the out-pointer state.
    fn dispatch(
        clsid: Clsid,
        riid: GUID,
        registry: &[&'static ApoVTable],
        ppv_null: bool,
    ) -> (HRESULT, *mut c_void) {
        let mut out: *mut c_void = 0xDEAD_BEEF as *mut c_void;
        let ppv_ptr = if ppv_null {
            core::ptr::null_mut()
        } else {
            &mut out as *mut *mut c_void
        };
        let g: GUID = clsid.into();
        let hr = unsafe { dll_get_class_object_dispatch(&g, &riid, ppv_ptr, registry) };
        (hr, out)
    }

    #[test]
    fn dispatch_null_ppv_returns_e_pointer() {
        let (hr, out) = dispatch(Dummy::CLSID, IUnknown::IID, &[], true);
        assert_eq!(hr, HResult::E_POINTER.into());
        // out untouched because ppv was null
        assert_eq!(out, 0xDEAD_BEEF as *mut c_void);
    }

    #[test]
    fn dispatch_unknown_clsid_returns_class_e_classnotavailable() {
        // Empty registry.
        let unknown_clsid = Clsid::from_u128(0xBADBAD00_0000_0000_0000_0000BADBAD00);
        let (hr, out) = dispatch(unknown_clsid, IUnknown::IID, &[], false);
        assert_eq!(hr, HResult::CLASS_E_CLASSNOTAVAILABLE.into());
        // out should have been zeroed before lookup.
        assert!(out.is_null());
    }

    #[test]
    fn dispatch_matching_clsid_returns_class_factory() {
        use windows::Win32::System::Com::IClassFactory;

        let registry: &[&ApoVTable] = &[&DUMMY_VT];
        let (hr, out) = dispatch(Dummy::CLSID, IClassFactory::IID, registry, false);
        assert!(hr.is_ok(), "expected S_OK from query, got {hr:?}");
        assert!(!out.is_null());

        // Drop the returned interface to release the factory.
        // Safety: `out` is a valid IClassFactory pointer the
        // dispatcher just handed us via QueryInterface.
        unsafe {
            let _factory = IClassFactory::from_raw(out);
        }
    }

    #[test]
    fn dispatch_matching_clsid_with_unsupported_riid_returns_no_interface() {
        // GUID not implemented by IUnknown / IClassFactory.
        let unsupported = GUID::from_u128(0xCAFE0001_0000_0000_0000_000000000001);
        let registry: &[&ApoVTable] = &[&DUMMY_VT];
        let (hr, out) = dispatch(Dummy::CLSID, unsupported, registry, false);
        assert_eq!(hr, HResult::E_NOINTERFACE.into());
        // out should be null after a failed QueryInterface.
        assert!(out.is_null());
    }
}
