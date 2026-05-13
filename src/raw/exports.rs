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
//! This module supplies the reusable building blocks the macro's
//! emitted entry points call into:
//!
//! - [`dll_get_class_object_dispatch`] — CLSID → factory lookup
//!   that materialises an [`ApoClassFactory`] and routes the
//!   requested IID through `IUnknown::QueryInterface`.
//! - [`dll_register_server_dispatch`] /
//!   [`dll_unregister_server_dispatch`] — registry-side
//!   self-registration, delegating to
//!   [`crate::raw::register`].
//! - [`dll_can_unload_now_dispatch`] — process-wide
//!   "is anyone holding an APO instance" probe.
//!
//! ## Outstanding-instance counter
//!
//! The COM contract for `DllCanUnloadNow` is inherently
//! process-global: the COM loader asks "are there any live
//! references from this DLL?" and expects a single answer. The
//! framework tracks this with one `AtomicU32` per cdylib,
//! incremented by `ApoInstanceCom::new` and decremented by its
//! `Drop`. CLAUDE.md's prohibition on global state applies to
//! APO instance data (which is per-object), not to the single
//! boolean fact "does this DLL still have live instances", which
//! the COM contract forces to be process-global.

extern crate alloc;

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};

use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows_core::{ComObject, IUnknown, Interface, GUID, HRESULT, PCWSTR};

use crate::clsid::Clsid;
use crate::error::HResult;
use crate::raw::class_factory::{ApoClassFactory, ApoVTable};
use crate::raw::register;

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

/// Per-cdylib counter of `ApoInstanceCom` objects currently held
/// by COM callers.
///
/// `ApoInstanceCom::new` increments this; its `Drop` decrements it.
/// `dll_can_unload_now_dispatch` reads it to decide whether the
/// DLL is safe to unload.
static OUTSTANDING: AtomicU32 = AtomicU32::new(0);

/// Increment the outstanding-instance counter.
///
/// Intended for use by [`crate::raw::instance_com::ApoInstanceCom::new`].
/// `Relaxed` is sufficient: we only need the total count, not any
/// happens-before relationship between increments.
pub fn outstanding_inc() {
    OUTSTANDING.fetch_add(1, Ordering::Relaxed);
}

/// Decrement the outstanding-instance counter.
///
/// Symmetric counterpart to [`outstanding_inc`]; called from
/// `Drop for ApoInstanceCom`.
pub fn outstanding_dec() {
    OUTSTANDING.fetch_sub(1, Ordering::Relaxed);
}

/// Current value of the outstanding-instance counter. Test
/// hook; production code reads this through
/// [`dll_can_unload_now_dispatch`].
#[must_use]
pub fn outstanding_count() -> u32 {
    OUTSTANDING.load(Ordering::Relaxed)
}

/// `DllCanUnloadNow` dispatch: returns `S_OK` (0) when no
/// `ApoInstanceCom` is outstanding, `S_FALSE` (1) otherwise.
///
/// The COM loader honours `S_OK` by unloading the DLL once no
/// other consumers are holding references; `S_FALSE` keeps the
/// DLL pinned. This is the standard COM in-process server
/// convention.
pub fn dll_can_unload_now_dispatch() -> HRESULT {
    if outstanding_count() == 0 {
        HRESULT(0) // S_OK
    } else {
        HRESULT(1) // S_FALSE
    }
}

/// `DllRegisterServer` dispatch: writes each `ApoVTable` in
/// `registry` to the per-user CLSID registry hive.
///
/// Discovers the calling DLL's absolute path via
/// `GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS)` on
/// this function's own address — since the framework rlib is
/// linked into each cdylib statically, that address resolves to
/// the cdylib calling us.
///
/// On any registry-write failure the routine returns the first
/// failing `HRESULT` *without* rolling back previously-written
/// keys. Pair with [`dll_unregister_server_dispatch`] to clean up.
pub fn dll_register_server_dispatch(registry: &[&'static ApoVTable]) -> HRESULT {
    let dll_path = match own_module_path() {
        Ok(p) => p,
        Err(e) => return e.code(),
    };
    for vtable in registry {
        if let Err(e) = register::write_registry(vtable, &dll_path) {
            return e.code();
        }
    }
    HRESULT(0)
}

/// `DllUnregisterServer` dispatch: removes each `ApoVTable`'s
/// CLSID subtree under `HKEY_CURRENT_USER`.
///
/// Idempotent on a per-CLSID basis (missing keys are not an error)
/// but iterates through `registry` in order; on the first failure
/// other than "key not present", the routine returns that
/// `HRESULT` without continuing.
pub fn dll_unregister_server_dispatch(registry: &[&'static ApoVTable]) -> HRESULT {
    for vtable in registry {
        if let Err(e) = register::clear_registry(&vtable.clsid) {
            return e.code();
        }
    }
    HRESULT(0)
}

/// Look up the absolute path of the DLL this code is linked into.
///
/// Returns a UTF-16 buffer ending in a null terminator, ready for
/// `RegSetValueExW(REG_SZ)` consumption. The pointer used as the
/// address-of-module probe is this function itself: `dll_register_server_dispatch`
/// would also work and is what the public callers exercise — using
/// the helper means there is only one entry point performing the
/// probe, and `GetModuleHandleExW` resolves it to whichever cdylib
/// statically linked the framework rlib.
pub fn own_module_path() -> windows_core::Result<alloc::vec::Vec<u16>> {
    let mut hmodule = HMODULE::default();
    // Reinterpret the function pointer as a UTF-16 string pointer
    // for the windows-rs signature; with FLAG_FROM_ADDRESS set the
    // API treats it as an address rather than dereferencing it as
    // a string.
    let address = own_module_path as *const c_void;
    // Safety: `address` is the address of a static function in the
    // current DLL; the FROM_ADDRESS flag instructs the loader to
    // resolve the address rather than the conceptual UTF-16
    // string. UNCHANGED_REFCOUNT avoids us holding an extra
    // reference on the module.
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(address.cast::<u16>()),
            &mut hmodule,
        )
    }?;

    // GetModuleFileNameW: 1024 wchars covers typical install
    // paths. The Windows limit is `MAX_PATH = 260` for legacy
    // applications and ~32 KB for long-path-aware ones; APOs are
    // typically under `Program Files\<vendor>\` so 1024 has
    // comfortable headroom.
    let mut buf = alloc::vec![0u16; 1024];
    // Safety: buf is writable for buf.len() wchars; hmodule is
    // live for the duration of the call.
    let written = unsafe { GetModuleFileNameW(Some(hmodule), &mut buf) };
    if written == 0 {
        return Err(windows_core::Error::from_thread());
    }
    // GetModuleFileNameW returns the number of characters written
    // *excluding* the null terminator. Truncate to include the
    // terminator.
    buf.truncate(written as usize + 1);
    Ok(buf)
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
