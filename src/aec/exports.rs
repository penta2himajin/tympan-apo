//! AEC counterparts of the shared `Dll*` dispatch helpers in
//! [`crate::raw::exports`].
//!
//! Walk the AEC vtable registry, mint AEC-side class factories
//! and registry-property payloads, and dispatch the four standard
//! `Dll*` entry points the `register_aec_apo!` macro emits.

extern crate alloc;

use core::ffi::c_void;

use windows::Win32::Media::Audio::Apo::{
    IApoAcousticEchoCancellation, IApoAuxiliaryInputConfiguration, IApoAuxiliaryInputRT,
    IAudioProcessingObject, IAudioProcessingObjectConfiguration, IAudioProcessingObjectRT,
    IAudioSystemEffects, IAudioSystemEffects2, IAudioSystemEffects3, APO_REG_PROPERTIES,
};
use windows_core::{ComObject, IUnknown, Interface, GUID, HRESULT};

use crate::aec::class_factory::{AecApoClassFactory, AecApoVTable};
use crate::clsid::Clsid;
use crate::error::HResult;

/// IID list emitted in `APO_REG_PROPERTIES.iidAPOInterfaceList` for
/// AEC APOs. The first six match the SISO list; the trailing three
/// are the AEC-specific interfaces.
fn aec_supported_interfaces() -> [GUID; 9] {
    [
        IAudioProcessingObject::IID,
        IAudioProcessingObjectConfiguration::IID,
        IAudioProcessingObjectRT::IID,
        IAudioSystemEffects::IID,
        IAudioSystemEffects2::IID,
        IAudioSystemEffects3::IID,
        IApoAcousticEchoCancellation::IID,
        IApoAuxiliaryInputConfiguration::IID,
        IApoAuxiliaryInputRT::IID,
    ]
}

/// CLSID → AEC factory dispatch shared by every user-emitted
/// `DllGetClassObject` from `register_aec_apo!`.
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
pub unsafe fn aec_dll_get_class_object_dispatch(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
    registry: &[&'static AecApoVTable],
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

    let factory = AecApoClassFactory::new(vtable);
    let com = ComObject::new(factory);
    let unknown: IUnknown = com.into_interface();
    // Safety: unknown is a valid IUnknown pointer; the COM caller
    // guarantees `riid` and `ppv` are valid.
    unsafe { unknown.query(riid, ppv) }
}

/// `DllRegisterServer` dispatch for AEC vtables. Walks the
/// registry and writes each CLSID subtree via
/// [`crate::raw::register::write_registry`]. The same HKCU layout
/// as SISO; the engine consults
/// `IID_IApoAcousticEchoCancellation` separately to pick the AEC
/// slot.
pub fn aec_dll_register_server_dispatch(registry: &[&'static AecApoVTable]) -> HRESULT {
    let dll_path = match crate::raw::exports::own_module_path() {
        Ok(p) => p,
        Err(e) => return e.code(),
    };
    for vtable in registry {
        if let Err(e) =
            crate::raw::register::write_registry_with(vtable.clsid, vtable.name, &dll_path)
        {
            return e.code();
        }
    }
    HRESULT(0)
}

/// `DllUnregisterServer` dispatch for AEC vtables.
pub fn aec_dll_unregister_server_dispatch(registry: &[&'static AecApoVTable]) -> HRESULT {
    for vtable in registry {
        if let Err(e) = crate::raw::register::clear_registry(&vtable.clsid) {
            return e.code();
        }
    }
    HRESULT(0)
}

/// Build the variable-length `APO_REG_PROPERTIES` payload for an
/// AEC APO, advertising all nine interfaces in
/// `iidAPOInterfaceList`. The audio engine consults this through
/// `IAudioProcessingObject::GetRegistrationProperties` on the
/// AEC carrier.
pub fn build_aec_registration_properties(
    instance: &dyn crate::instance::AnyApoInstance,
) -> windows_core::Result<*mut APO_REG_PROPERTIES> {
    crate::raw::reg_properties::build_registration_properties_with(
        instance,
        &aec_supported_interfaces(),
    )
}
