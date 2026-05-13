//! Tier 3 lifecycle harness — drives the full APO COM lifecycle
//! against the **built `passthrough.dll`** loaded via `LoadLibrary`.
//!
//! Per `docs/decisions/0001-ci-verification-strategy.md` § Tier 3,
//! this integration test:
//!
//! 1. Loads the cdylib artefact produced by `examples/passthrough.rs`
//!    via `LoadLibraryW`.
//! 2. Resolves `DllGetClassObject` via `GetProcAddress` and uses it
//!    to mint an `IClassFactory` for the passthrough CLSID.
//! 3. Creates an `IAudioProcessingObject` via
//!    `IClassFactory::CreateInstance`.
//! 4. Drives `Initialize → IsInputFormatSupported → LockForProcess
//!    → APOProcess × N → UnlockForProcess`.
//! 5. Asserts the output buffer is bitwise-equal to the input
//!    (passthrough's analytic bound), no `NaN`, no `±Inf`.
//! 6. Releases the interface and unloads the library.
//!
//! The DLL path is selected by the `TYMPAN_PASSTHROUGH_DLL`
//! environment variable; the Tier 3 workflow sets it after building
//! the example. Outside CI, run as:
//!
//! ```bash
//! cargo build --release --target x86_64-pc-windows-msvc --example passthrough
//! TYMPAN_PASSTHROUGH_DLL=target\\x86_64-pc-windows-msvc\\release\\examples\\passthrough.dll \
//!   cargo test --target x86_64-pc-windows-msvc --test tier3_lifecycle -- --nocapture
//! ```
//!
//! ## Why not `assert_no_alloc` here
//!
//! `#[global_allocator]` is per-link-unit. The test binary and the
//! loaded `passthrough.dll` are separate link units with their own
//! `__rust_alloc` symbols, so a guard installed in the test crate
//! cannot intercept allocations inside the DLL. The
//! [`realtime_safety`](realtime_safety) integration test runs the
//! framework's `process` path through the rlib in the same link unit
//! as the test binary, which is where the alloc-free invariant can
//! be mechanically enforced.

#![cfg(windows)]

use core::ffi::c_void;
use core::mem::ManuallyDrop;
use core::ptr;

use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::Media::Audio::Apo::{
    IAudioMediaType, IAudioProcessingObject, IAudioProcessingObjectConfiguration,
    IAudioProcessingObjectRT, APO_CONNECTION_BUFFER_TYPE_ALLOCATED, APO_CONNECTION_DESCRIPTOR,
    APO_CONNECTION_PROPERTY, BUFFER_INVALID, BUFFER_VALID,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IClassFactory, CLSCTX_INPROC_SERVER,
    COINIT_MULTITHREADED,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_core::{Interface, GUID, HRESULT, PCSTR, PCWSTR};

use tympan_apo::format::Format;
use tympan_apo::raw::media_type::media_type_from_format;
use tympan_apo::{Clsid, CONNECTION_PROPERTY_SIGNATURE};

/// CLSID matches `Passthrough::CLSID` in `examples/passthrough.rs`.
const PASSTHROUGH_CLSID: Clsid = Clsid::from_u128(0x1B7E5A4F_3D2C_4E89_9C1A_6A6F40C92E11);

type DllGetClassObjectFn =
    unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;

/// RAII wrapper so the loaded module is unloaded even on test
/// failure / panic.
struct LoadedModule(HMODULE);

impl Drop for LoadedModule {
    fn drop(&mut self) {
        // Safety: handle is the result of a successful LoadLibraryW
        // earlier in the test; FreeLibrary may legitimately return
        // FALSE if the DLL has already been unloaded by COM
        // reference-counting, so we don't escalate to a panic.
        unsafe {
            let _ = FreeLibrary(self.0);
        }
    }
}

fn dll_path() -> std::path::PathBuf {
    std::env::var_os("TYMPAN_PASSTHROUGH_DLL")
        .map(std::path::PathBuf::from)
        .expect(
            "TYMPAN_PASSTHROUGH_DLL must point to the built passthrough cdylib; \
             the Tier 3 workflow sets this after `cargo build --example passthrough`",
        )
}

fn load_passthrough() -> (LoadedModule, DllGetClassObjectFn) {
    let path = dll_path();
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // Safety: path is a valid null-terminated UTF-16 string.
    let module = unsafe { LoadLibraryW(PCWSTR(wide.as_ptr())) }
        .unwrap_or_else(|e| panic!("LoadLibraryW({}) failed: {e:?}", path.display()));

    // Safety: module is live; "DllGetClassObject" is a stable narrow string literal.
    let raw = unsafe { GetProcAddress(module, PCSTR(c"DllGetClassObject".as_ptr().cast())) };
    let raw = raw.expect("DllGetClassObject not exported by the passthrough DLL");
    // Safety: GetProcAddress returns a function pointer with the
    // canonical Win32 signature; our `DllGetClassObjectFn` typedef
    // describes the actual COM ABI.
    let f: DllGetClassObjectFn = unsafe { core::mem::transmute(raw) };
    (LoadedModule(module), f)
}

fn create_factory(dll_get_class_object: DllGetClassObjectFn) -> IClassFactory {
    let clsid: GUID = PASSTHROUGH_CLSID.into();
    let iid: GUID = IClassFactory::IID;
    let mut out: *mut c_void = ptr::null_mut();
    // Safety: arguments adhere to the DllGetClassObject ABI.
    let hr = unsafe { dll_get_class_object(&clsid, &iid, &mut out) };
    assert!(hr.is_ok(), "DllGetClassObject returned {hr:?}");
    assert!(!out.is_null());
    // Safety: out is a valid IClassFactory pointer with a refcount
    // of 1; IClassFactory::from_raw assumes ownership.
    unsafe { IClassFactory::from_raw(out) }
}

fn descriptor(
    buffer_addr: usize,
    frames: u32,
    format: &IAudioMediaType,
) -> APO_CONNECTION_DESCRIPTOR {
    APO_CONNECTION_DESCRIPTOR {
        Type: APO_CONNECTION_BUFFER_TYPE_ALLOCATED,
        pBuffer: buffer_addr,
        u32MaxFrameCount: frames,
        // Safety: ManuallyDrop is the field type; we clone the
        // IAudioMediaType into it. The descriptor lives only for
        // the LockForProcess call; the original IAudioMediaType
        // outside ManuallyDrop is dropped normally.
        pFormat: ManuallyDrop::new(Some(format.clone())),
        u32Signature: CONNECTION_PROPERTY_SIGNATURE,
    }
}

/// Drive `Initialize → IsInputFormatSupported → LockForProcess →
/// APOProcess × N → UnlockForProcess` against `apo` and assert
/// passthrough's analytic bounds (output bitwise-equal to input,
/// every sample finite). Shared between the LoadLibrary and
/// regsvr32 + CoCreateInstance activation paths.
fn drive_passthrough_lifecycle(apo: &IAudioProcessingObject) {
    // Safety: live IAudioProcessingObject. The framework's
    // `Initialize` does not yet consume init data; pass an empty
    // slice.
    unsafe { apo.Initialize(&[]) }.expect("Initialize failed");

    let format = Format::pcm_float32(48_000, 1);
    let requested_media = media_type_from_format(&format);
    // Safety: live IAudioProcessingObject; opposite-format
    // pointer is allowed to be null per the engine contract.
    let _accepted = unsafe { apo.IsInputFormatSupported(None, &requested_media) }
        .expect("IsInputFormatSupported rejected float32 mono 48 kHz");

    // 256-frame buffer. Single-channel float32 ⇒ 256 samples.
    const FRAMES: u32 = 256;
    let input: Vec<f32> = (0..FRAMES)
        .map(|i| (i as f32 / FRAMES as f32) * 2.0 - 1.0)
        .collect();
    let mut output: Vec<f32> = vec![0.0; FRAMES as usize];

    let input_media = media_type_from_format(&format);
    let output_media = media_type_from_format(&format);
    let input_desc = descriptor(input.as_ptr() as usize, FRAMES, &input_media);
    let output_desc = descriptor(output.as_mut_ptr() as usize, FRAMES, &output_media);

    let config: IAudioProcessingObjectConfiguration = apo
        .cast()
        .expect("APO does not implement IAudioProcessingObjectConfiguration");

    let input_descs: [*const APO_CONNECTION_DESCRIPTOR; 1] = [&input_desc];
    let output_descs: [*const APO_CONNECTION_DESCRIPTOR; 1] = [&output_desc];
    // Safety: pointers refer to the live descriptors above; the
    // descriptors carry valid IAudioMediaType handles.
    unsafe { config.LockForProcess(&input_descs, &output_descs) }.expect("LockForProcess failed");

    let rt: IAudioProcessingObjectRT = apo
        .cast()
        .expect("APO does not implement IAudioProcessingObjectRT");

    const ITERATIONS: usize = 32;
    for iter in 0..ITERATIONS {
        // Zero the output between iterations so any non-write would
        // show up as a mismatch on assertion below.
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
        // Safety: APOProcess returns no HRESULT and never fails;
        // it stamps u32BufferFlags / u32ValidFrameCount on the
        // output property.
        unsafe { rt.APOProcess(1, &in_pp, 1, &mut out_pp) };
        // Touch in_prop after the call so the borrow checker keeps
        // it live for the entire APOProcess invocation.
        assert_eq!(in_prop.u32ValidFrameCount, FRAMES);
        assert_eq!(out_prop.u32ValidFrameCount, FRAMES);
        assert_eq!(out_prop.u32BufferFlags, BUFFER_VALID);
        for (i, (&s_in, &s_out)) in input.iter().zip(output.iter()).enumerate() {
            assert!(
                s_out.is_finite(),
                "iter {iter} frame {i}: output sample is not finite ({s_out})"
            );
            assert_eq!(
                s_out.to_bits(),
                s_in.to_bits(),
                "iter {iter} frame {i}: output != input"
            );
        }
    }

    // Safety: live IAudioProcessingObjectConfiguration.
    unsafe { config.UnlockForProcess() }.expect("UnlockForProcess failed");
}

/// Initialise COM on this thread; accept both `S_OK` and `S_FALSE`
/// (the latter is returned when COM is already initialised by a
/// previous test in the same process).
fn co_initialize() {
    let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    assert!(
        init.is_ok() || init.0 == 1, /* S_FALSE */
        "CoInitializeEx returned {init:?}"
    );
}

/// RAII cleanup: invoke `regsvr32 /s /u <dll>` on drop so the user
/// hive is restored to its starting state even if a test panics
/// after registration.
struct Regsvr32Cleanup(std::path::PathBuf);

impl Drop for Regsvr32Cleanup {
    fn drop(&mut self) {
        let _ = std::process::Command::new("regsvr32")
            .args(["/s", "/u"])
            .arg(&self.0)
            .status();
    }
}

fn regsvr32_register(dll: &std::path::Path) {
    let status = std::process::Command::new("regsvr32")
        .args(["/s"])
        .arg(dll)
        .status()
        .expect("failed to spawn regsvr32");
    assert!(
        status.success(),
        "regsvr32 /s {} exited with {status:?}",
        dll.display()
    );
}

#[test]
#[ignore = "requires TYMPAN_PASSTHROUGH_DLL pointing at the built passthrough cdylib; \
            opt in via `cargo test --test tier3_lifecycle -- --ignored` (Tier 3 CI does this)"]
fn passthrough_dll_drives_full_lifecycle() {
    let (_module_guard, dll_get_class_object) = load_passthrough();

    co_initialize();

    let factory = create_factory(dll_get_class_object);
    // Safety: live IClassFactory; aggregation is unsupported so
    // `pUnkOuter = None` is the only correct argument.
    let apo: IAudioProcessingObject = unsafe { factory.CreateInstance(None) }
        .expect("CreateInstance(IAudioProcessingObject) failed");

    drive_passthrough_lifecycle(&apo);

    drop(apo);
    drop(factory);

    // Safety: every prior CoInitializeEx that returned S_OK or
    // S_FALSE must be paired with a CoUninitialize.
    unsafe { CoUninitialize() };
}

#[test]
#[ignore = "requires TYMPAN_PASSTHROUGH_DLL pointing at the built passthrough cdylib and `regsvr32` \
            on PATH; opt in via `cargo test --test tier3_lifecycle -- --ignored` (Tier 3 CI does this)"]
fn passthrough_via_regsvr32_and_cocreateinstance() {
    let path = dll_path();

    // Initialise COM before any registry write, so the activator's
    // first lookup happens against a COM-initialised thread.
    co_initialize();

    // regsvr32 invokes DllRegisterServer in its own process which
    // calls into the framework's HKCU-writing dispatch helper. The
    // user hive is the same across processes for this user, so the
    // test process's CoCreateInstance below resolves the CLSID
    // through that fresh registration.
    regsvr32_register(&path);
    let _cleanup = Regsvr32Cleanup(path.clone());

    let clsid: GUID = PASSTHROUGH_CLSID.into();
    // Safety: clsid points to a live GUID; CoCreateInstance with
    // CLSCTX_INPROC_SERVER does the registry lookup, LoadLibrary's
    // the resolved DLL, calls DllGetClassObject, and routes
    // CreateInstance to the IID_IAudioProcessingObject vtable.
    let apo: IAudioProcessingObject =
        unsafe { CoCreateInstance(&clsid, None, CLSCTX_INPROC_SERVER) }
            .expect("CoCreateInstance(CLSID_PASSTHROUGH, IAudioProcessingObject) failed");

    drive_passthrough_lifecycle(&apo);

    drop(apo);

    // Safety: every prior CoInitializeEx that returned S_OK or
    // S_FALSE must be paired with a CoUninitialize.
    unsafe { CoUninitialize() };
}

// `std::os::windows::ffi::OsStrExt::encode_wide` is what we use
// above; re-export the trait so it resolves on Windows targets.
use std::os::windows::ffi::OsStrExt;
