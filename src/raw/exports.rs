//! COM in-process server entry points.
//!
//! Every APO `.dll` exports the four standard COM in-process server
//! functions below. The Windows audio engine resolves these by name
//! at load time (Tier 2 verification checks they are present and
//! unmangled via `dumpbin /exports`).
//!
//! Current behaviour: the entry points are wired up at the ABI
//! boundary but report no-class-available / no-op. The
//! `IClassFactory` machinery, registry writes, and ref-count
//! bookkeeping land in later commits.

use core::ffi::c_void;

use windows_core::{ComObject, IUnknown, Interface, GUID, HRESULT};

use crate::clsid::Clsid;
use crate::error::HResult;
use crate::raw::class_factory::{ApoClassFactory, ApoVTable};

/// `S_OK = 0x00000000`.
const S_OK: HRESULT = HRESULT(0);

/// `S_FALSE = 0x00000001`. Returned by `DllCanUnloadNow` while the
/// DLL still has live objects.
const S_FALSE: HRESULT = HRESULT(1);

/// CLSID → factory dispatch shared by `DllGetClassObject` and the
/// future `register_apo!` macro.
///
/// Looks up `rclsid` in `registry`, materialises an
/// [`ApoClassFactory`] for the matching [`ApoVTable`], wraps it in a
/// COM object, and routes the requested `riid` through
/// `IUnknown::QueryInterface`. Returns `CLASS_E_CLASSNOTAVAILABLE`
/// if the CLSID is not registered, `E_POINTER` if `ppv` is null.
///
/// # Safety
///
/// Called from COM entry points. The caller must guarantee:
///
/// - `rclsid` points to a valid `GUID` for the lifetime of this
///   call.
/// - `riid` points to a valid `GUID` for the lifetime of this
///   call.
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

/// COM class object factory entry point.
///
/// # Safety
///
/// Called by COM. `rclsid` and `riid` must point to valid `GUID`s
/// and `ppv` must point to a writable pointer slot. The function
/// follows the standard COM contract: on `CLASS_E_CLASSNOTAVAILABLE`
/// the slot is zeroed before returning.
///
/// The framework's own export carries an empty registry; the
/// `register_apo!` macro (follow-up PR) will let users override
/// this with their own non-empty list.
#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    // Safety: forwards the same contract we received.
    unsafe { dll_get_class_object_dispatch(rclsid, riid, ppv, &[]) }
}

/// Returns `S_OK` if the DLL has no outstanding object references
/// and may be unloaded, otherwise `S_FALSE`.
///
/// # Safety
///
/// Called by COM. The placeholder implementation never reports the
/// DLL as unloadable so the host keeps it loaded; once the
/// reference counter is wired in, this will consult it.
#[no_mangle]
pub unsafe extern "system" fn DllCanUnloadNow() -> HRESULT {
    // Stub: with no live objects yet there is nothing to count, but
    // returning S_FALSE is the safer placeholder while
    // `DllGetClassObject` cannot actually hand out objects.
    S_FALSE
}

/// Self-registration entry point invoked by `regsvr32.exe`.
///
/// # Safety
///
/// Called by `regsvr32`. The current implementation does not touch
/// the registry; once `registration` lands it will write the CLSID
/// keys under `HKCU\Software\Classes\CLSID\{...}` (per-user) or
/// `HKLM\...` (machine, with admin).
#[no_mangle]
pub unsafe extern "system" fn DllRegisterServer() -> HRESULT {
    S_OK
}

/// Inverse of `DllRegisterServer`.
///
/// # Safety
///
/// Called by `regsvr32 /u`. Mirrors `DllRegisterServer`'s stub
/// behaviour for now.
#[no_mangle]
pub unsafe extern "system" fn DllUnregisterServer() -> HRESULT {
    S_OK
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
