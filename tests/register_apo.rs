//! Integration test for the `register_apo!` macro.
//!
//! Lives in `tests/` (a separate binary) so the test crate gets a
//! single, unique invocation of `register_apo!` — the macro emits
//! `#[no_mangle]` symbols (`DllGetClassObject`, etc.) that must
//! be unique in the link tree.
//!
//! On Windows targets the test:
//! 1. Calls `register_apo!(Passthrough)` to emit the vtable and
//!    DLL entry points.
//! 2. Asserts the emitted vtable carries the values from the
//!    `Passthrough` implementor.
//! 3. Invokes `DllGetClassObject` directly to confirm the emitted
//!    entry point dispatches through to
//!    `dll_get_class_object_dispatch`.
//!
//! On non-Windows targets the test is a no-op: the framework's
//! `register_apo!` macro is `#[cfg(windows)]` gated and the COM
//! types it references are unavailable.

#![cfg(windows)]

use tympan_apo::realtime::RealtimeContext;
use tympan_apo::{ApoCategory, BufferFlags, Clsid, ProcessInput, ProcessingObject};

struct Passthrough;

impl ProcessingObject for Passthrough {
    const CLSID: Clsid = Clsid::from_u128(0x11111111_2222_3333_4444_555555555555);
    const NAME: &'static str = "register_apo test passthrough";
    const COPYRIGHT: &'static str = "tympan-apo test";
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

tympan_apo::register_apo!(Passthrough);

#[test]
fn vtable_constants_match_processing_object() {
    assert_eq!(__TYMPAN_APO_VTABLE.clsid, Passthrough::CLSID);
    assert_eq!(__TYMPAN_APO_VTABLE.name, Passthrough::NAME);
    assert_eq!(__TYMPAN_APO_VTABLE.copyright, Passthrough::COPYRIGHT);
    assert_eq!(__TYMPAN_APO_VTABLE.category, Passthrough::CATEGORY);
}

#[test]
fn vtable_create_yields_uninitialized_instance() {
    let inst = (__TYMPAN_APO_VTABLE.create)();
    assert_eq!(inst.refcount(), 0);
    assert_eq!(inst.state(), tympan_apo::realtime::State::Uninitialized);
}

#[test]
fn dll_get_class_object_returns_class_factory_for_registered_clsid() {
    use core::ffi::c_void;
    use tympan_apo::{GUID, HRESULT};
    use windows::Win32::System::Com::IClassFactory;
    use windows_core::Interface;

    let clsid: GUID = Passthrough::CLSID.into();
    let iid: GUID = IClassFactory::IID;
    let mut out: *mut c_void = core::ptr::null_mut();

    // Safety: arguments adhere to the DllGetClassObject ABI;
    // the macro-emitted entry point forwards into
    // dll_get_class_object_dispatch.
    let hr: HRESULT = unsafe { DllGetClassObject(&clsid, &iid, &mut out) };

    assert!(hr.is_ok(), "DllGetClassObject returned {hr:?}");
    assert!(!out.is_null());

    // Drop the returned interface to release the factory.
    // Safety: out is a valid IClassFactory pointer.
    unsafe {
        let _factory = IClassFactory::from_raw(out);
    }
}

#[test]
fn dll_get_class_object_returns_class_e_classnotavailable_for_unknown_clsid() {
    use core::ffi::c_void;
    use tympan_apo::{HResult, GUID, HRESULT};
    use windows_core::Interface;

    let unknown_clsid = Clsid::from_u128(0xDEADBEEF_0000_0000_0000_000000000000);
    let clsid: GUID = unknown_clsid.into();
    let iid: GUID = windows_core::IUnknown::IID;
    let mut out: *mut c_void = core::ptr::null_mut();

    let hr: HRESULT = unsafe { DllGetClassObject(&clsid, &iid, &mut out) };

    assert_eq!(hr, HResult::CLASS_E_CLASSNOTAVAILABLE.into());
    assert!(out.is_null());
}

#[test]
fn dll_can_unload_now_returns_s_false_while_stubbed() {
    // S_FALSE = 1
    let hr = unsafe { DllCanUnloadNow() };
    assert_eq!(hr.0, 1);
}

#[test]
fn dll_register_server_returns_s_ok_while_stubbed() {
    // S_OK = 0
    assert_eq!(unsafe { DllRegisterServer() }.0, 0);
    assert_eq!(unsafe { DllUnregisterServer() }.0, 0);
}
