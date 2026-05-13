//! Tier 3 lifecycle harness — AEC variant.
//!
//! AEC counterpart of `tests/tier3_lifecycle.rs`. Loads the built
//! `aec_scaffold.dll` via `LoadLibraryW`, drives the AEC-specific
//! interface family end-to-end:
//!
//! 1. Resolve `DllGetClassObject` via `GetProcAddress`.
//! 2. Mint an `IClassFactory` for the AEC scaffold's CLSID.
//! 3. `CreateInstance(IID_IApoAcousticEchoCancellation)` — must
//!    succeed; the marker interface confirms the engine sees this
//!    APO as AEC-capable.
//! 4. `CreateInstance(IID_IApoAuxiliaryInputConfiguration)` and
//!    drive `AddAuxiliaryInput` + `RemoveAuxiliaryInput`.
//! 5. Drive the SISO lifecycle (`Initialize` →
//!    `IsInputFormatSupported` → `LockForProcess` → `APOProcess` →
//!    `UnlockForProcess`) on the primary input through the same
//!    object to verify the SISO interfaces still resolve via the
//!    AEC carrier.
//!
//! `#[ignore]`-gated; the Tier 3 workflow opts in via
//! `cargo test ... -- --ignored`.

#![cfg(all(windows, feature = "aec"))]

use core::ffi::c_void;
use core::mem::ManuallyDrop;
use core::ptr;
use std::os::windows::ffi::OsStrExt;

use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::Media::Audio::Apo::{
    IApoAcousticEchoCancellation, IApoAuxiliaryInputConfiguration, IAudioProcessingObject,
    IAudioProcessingObjectConfiguration, IAudioProcessingObjectRT,
    APO_CONNECTION_BUFFER_TYPE_ALLOCATED, APO_CONNECTION_DESCRIPTOR, APO_CONNECTION_PROPERTY,
    BUFFER_INVALID, BUFFER_VALID,
};
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, IClassFactory, COINIT_MULTITHREADED,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_core::{Interface, GUID, HRESULT, PCSTR, PCWSTR};

use tympan_apo::format::Format;
use tympan_apo::raw::media_type::media_type_from_format;
use tympan_apo::{Clsid, CONNECTION_PROPERTY_SIGNATURE};

/// CLSID matches `AecScaffold::CLSID` in `examples/aec_scaffold.rs`.
const AEC_SCAFFOLD_CLSID: Clsid = Clsid::from_u128(0x3D5C9E2D_3D2C_4E89_9C1A_6A6F40C92E13);

type DllGetClassObjectFn =
    unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;

struct LoadedModule(HMODULE);

impl Drop for LoadedModule {
    fn drop(&mut self) {
        // Safety: handle is the result of a successful LoadLibraryW
        // earlier in the test.
        unsafe {
            let _ = FreeLibrary(self.0);
        }
    }
}

fn dll_path() -> std::path::PathBuf {
    std::env::var_os("TYMPAN_AEC_SCAFFOLD_DLL")
        .map(std::path::PathBuf::from)
        .expect(
            "TYMPAN_AEC_SCAFFOLD_DLL must point to the built aec_scaffold cdylib; \
             the Tier 3 workflow sets this after `cargo build --features aec --example aec_scaffold`",
        )
}

fn load_aec_scaffold() -> (LoadedModule, DllGetClassObjectFn) {
    let path = dll_path();
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // Safety: wide is a valid null-terminated UTF-16 string.
    let module = unsafe { LoadLibraryW(PCWSTR(wide.as_ptr())) }
        .unwrap_or_else(|e| panic!("LoadLibraryW({}) failed: {e:?}", path.display()));

    // Safety: module is live; "DllGetClassObject" is a stable
    // narrow string literal.
    let raw = unsafe { GetProcAddress(module, PCSTR(c"DllGetClassObject".as_ptr().cast())) };
    let raw = raw.expect("DllGetClassObject not exported by the aec_scaffold DLL");
    // Safety: GetProcAddress returns a function pointer with the
    // canonical Win32 signature.
    let f: DllGetClassObjectFn = unsafe { core::mem::transmute(raw) };
    (LoadedModule(module), f)
}

fn co_initialize() {
    let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    assert!(
        init.is_ok() || init.0 == 1, // S_FALSE
        "CoInitializeEx returned {init:?}"
    );
}

fn create_factory(dll_get_class_object: DllGetClassObjectFn) -> IClassFactory {
    let clsid: GUID = AEC_SCAFFOLD_CLSID.into();
    let iid: GUID = IClassFactory::IID;
    let mut out: *mut c_void = ptr::null_mut();
    let hr = unsafe { dll_get_class_object(&clsid, &iid, &mut out) };
    assert!(hr.is_ok(), "DllGetClassObject returned {hr:?}");
    assert!(!out.is_null());
    unsafe { IClassFactory::from_raw(out) }
}

#[test]
#[ignore = "requires TYMPAN_AEC_SCAFFOLD_DLL pointing at the built aec_scaffold cdylib; \
            opt in via `cargo test --features aec --test tier3_aec_lifecycle -- --ignored`"]
fn aec_scaffold_dll_advertises_aec_interfaces() {
    let (_module_guard, dll_get_class_object) = load_aec_scaffold();
    co_initialize();

    let factory = create_factory(dll_get_class_object);

    // The marker interface — its presence is what tells the audio
    // engine this APO is AEC-capable.
    // Safety: live IClassFactory.
    let _aec: IApoAcousticEchoCancellation = unsafe { factory.CreateInstance(None) }
        .expect("CreateInstance(IApoAcousticEchoCancellation) failed");

    // The auxiliary-input configuration interface — drive
    // AddAuxiliaryInput + RemoveAuxiliaryInput through the COM
    // vtable to verify the user APO's add/remove hooks fire.
    // Safety: live IClassFactory.
    let cfg: IApoAuxiliaryInputConfiguration = unsafe { factory.CreateInstance(None) }
        .expect("CreateInstance(IApoAuxiliaryInputConfiguration) failed");

    let format = Format::pcm_float32(48_000, 1);
    let media_owned = media_type_from_format(&format);
    let desc = APO_CONNECTION_DESCRIPTOR {
        Type: APO_CONNECTION_BUFFER_TYPE_ALLOCATED,
        pBuffer: 0,
        u32MaxFrameCount: 256,
        // Safety: ManuallyDrop carries the cloned interface
        // reference into the descriptor; the original
        // `media_owned` stays alive for the duration of the call
        // and is released when this test scope ends.
        pFormat: ManuallyDrop::new(Some(media_owned.clone())),
        u32Signature: CONNECTION_PROPERTY_SIGNATURE,
    };
    let init_data = [0u8; 0];
    // Safety: live IApoAuxiliaryInputConfiguration; the descriptor
    // is a stack local that lives through the call.
    unsafe { cfg.AddAuxiliaryInput(7, &init_data, &desc) }.expect("AddAuxiliaryInput failed");
    // Safety: live IApoAuxiliaryInputConfiguration.
    unsafe { cfg.RemoveAuxiliaryInput(7) }.expect("RemoveAuxiliaryInput failed");

    drop(cfg);
    drop(factory);

    // Safety: CoInitializeEx was paired above.
    unsafe { CoUninitialize() };
}

#[test]
#[ignore = "requires TYMPAN_AEC_SCAFFOLD_DLL; opts in via \
            `cargo test --features aec --test tier3_aec_lifecycle -- --ignored`"]
fn aec_scaffold_dll_drives_siso_lifecycle_through_aec_carrier() {
    let (_module_guard, dll_get_class_object) = load_aec_scaffold();
    co_initialize();

    let factory = create_factory(dll_get_class_object);
    // Even though the carrier is the AEC variant, it implements
    // IAudioProcessingObject too — drive the SISO lifecycle to
    // confirm that path resolves through the AEC carrier.
    // Safety: live IClassFactory.
    let apo: IAudioProcessingObject = unsafe { factory.CreateInstance(None) }
        .expect("CreateInstance(IAudioProcessingObject) failed");

    // Safety: live interface; AEC Initialize delegates to the
    // shared lifecycle state machine.
    unsafe { apo.Initialize(&[]) }.expect("Initialize failed");

    let format = Format::pcm_float32(48_000, 1);
    let requested_media = media_type_from_format(&format);
    // Safety: live interface.
    let _accepted = unsafe { apo.IsInputFormatSupported(None, &requested_media) }
        .expect("IsInputFormatSupported rejected float32 mono 48 kHz");

    const FRAMES: u32 = 256;
    let input: Vec<f32> = (0..FRAMES)
        .map(|i| (i as f32 / FRAMES as f32) * 2.0 - 1.0)
        .collect();
    let mut output: Vec<f32> = vec![0.0; FRAMES as usize];

    let input_media = media_type_from_format(&format);
    let output_media = media_type_from_format(&format);
    let input_desc = APO_CONNECTION_DESCRIPTOR {
        Type: APO_CONNECTION_BUFFER_TYPE_ALLOCATED,
        pBuffer: input.as_ptr() as usize,
        u32MaxFrameCount: FRAMES,
        pFormat: ManuallyDrop::new(Some(input_media.clone())),
        u32Signature: CONNECTION_PROPERTY_SIGNATURE,
    };
    let output_desc = APO_CONNECTION_DESCRIPTOR {
        Type: APO_CONNECTION_BUFFER_TYPE_ALLOCATED,
        pBuffer: output.as_mut_ptr() as usize,
        u32MaxFrameCount: FRAMES,
        pFormat: ManuallyDrop::new(Some(output_media.clone())),
        u32Signature: CONNECTION_PROPERTY_SIGNATURE,
    };

    let config: IAudioProcessingObjectConfiguration = apo
        .cast()
        .expect("APO carrier missing Configuration interface");
    let in_descs: [*const APO_CONNECTION_DESCRIPTOR; 1] = [&input_desc];
    let out_descs: [*const APO_CONNECTION_DESCRIPTOR; 1] = [&output_desc];
    // Safety: descriptors are live stack locals for the call.
    unsafe { config.LockForProcess(&in_descs, &out_descs) }.expect("LockForProcess failed");

    let rt: IAudioProcessingObjectRT = apo.cast().expect("APO carrier missing RT interface");

    const ITERATIONS: usize = 8;
    for iter in 0..ITERATIONS {
        output.fill(0.0);
        let in_prop = APO_CONNECTION_PROPERTY {
            pBuffer: input.as_ptr() as usize,
            u32ValidFrameCount: FRAMES,
            u32BufferFlags: BUFFER_VALID,
            u32Signature: CONNECTION_PROPERTY_SIGNATURE,
        };
        let mut out_prop = APO_CONNECTION_PROPERTY {
            pBuffer: output.as_mut_ptr() as usize,
            u32ValidFrameCount: 0,
            u32BufferFlags: BUFFER_INVALID,
            u32Signature: CONNECTION_PROPERTY_SIGNATURE,
        };
        let in_pp = &in_prop as *const _;
        let mut out_pp = &mut out_prop as *mut _;
        // Safety: APOProcess returns no HRESULT; pointers refer to
        // live properties.
        unsafe { rt.APOProcess(1, &in_pp, 1, &mut out_pp) };
        assert_eq!(in_prop.u32ValidFrameCount, FRAMES);
        assert_eq!(out_prop.u32ValidFrameCount, FRAMES);
        assert_eq!(out_prop.u32BufferFlags, BUFFER_VALID);
        for (i, (&s_in, &s_out)) in input.iter().zip(output.iter()).enumerate() {
            assert_eq!(
                s_out.to_bits(),
                s_in.to_bits(),
                "iter {iter} frame {i}: output != input"
            );
        }
    }

    // Safety: live Configuration.
    unsafe { config.UnlockForProcess() }.expect("UnlockForProcess failed");

    drop(rt);
    drop(config);
    drop(apo);
    drop(factory);

    // Safety: CoInitializeEx was paired above.
    unsafe { CoUninitialize() };
}
