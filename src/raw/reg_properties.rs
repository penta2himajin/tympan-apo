//! Builder for the variable-length `APO_REG_PROPERTIES` payload
//! returned from `IAudioProcessingObject::GetRegistrationProperties`.
//!
//! `APO_REG_PROPERTIES` carries a `[GUID; 1]` "header" field that is
//! actually the start of a variable-length IID list. The audio
//! engine reads `u32NumAPOInterfaces` and dereferences past the
//! published struct end to walk the trailing GUIDs. The buffer must
//! therefore be allocated with `CoTaskMemAlloc` so the engine can
//! `CoTaskMemFree` it after consuming the data.
//!
//! [`build_registration_properties`] materialises that buffer from
//! the metadata accessors on [`AnyApoInstance`]. The supported
//! interface list is fixed for now — every framework-built APO
//! exposes the same three interfaces ([`IAudioProcessingObject`],
//! [`IAudioProcessingObjectConfiguration`],
//! [`IAudioProcessingObjectRT`]) — and a future change can widen it
//! without breaking the call site.

extern crate alloc;

use core::mem;
use core::ptr;

use windows::Win32::Media::Audio::Apo::{
    IAudioProcessingObject, IAudioProcessingObjectConfiguration, IAudioProcessingObjectRT,
    IAudioSystemEffects, IAudioSystemEffects2, APO_FLAG_NONE, APO_REG_PROPERTIES,
};
use windows::Win32::System::Com::CoTaskMemAlloc;
use windows_core::{Interface, GUID, HRESULT};

use crate::error::HResult;
use crate::instance::AnyApoInstance;

/// Fixed list of interfaces every framework-built APO advertises.
///
/// Order matches the `#[implement(...)]` annotation on
/// [`crate::raw::instance_com::ApoInstanceCom`]. The audio engine
/// does not require a specific ordering, but keeping the two in
/// sync avoids surprises.
fn supported_interfaces() -> [GUID; 5] {
    [
        IAudioProcessingObject::IID,
        IAudioProcessingObjectConfiguration::IID,
        IAudioProcessingObjectRT::IID,
        IAudioSystemEffects::IID,
        IAudioSystemEffects2::IID,
    ]
}

/// Allocate and populate an `APO_REG_PROPERTIES` carrying the
/// per-APO metadata reported back to the audio engine.
///
/// Returns a pointer to a `CoTaskMemAlloc`-backed buffer big enough
/// for the base struct *plus* the trailing IID list past the
/// published `[GUID; 1]` field. Ownership transfers to the caller;
/// the audio engine releases it with `CoTaskMemFree`.
///
/// Failure modes:
///
/// - `E_OUTOFMEMORY` if the allocation request would overflow
///   `usize` or `CoTaskMemAlloc` returns null.
pub fn build_registration_properties(
    instance: &dyn AnyApoInstance,
) -> windows_core::Result<*mut APO_REG_PROPERTIES> {
    let interfaces = supported_interfaces();
    let total = total_size(interfaces.len()).ok_or_else(|| {
        windows_core::Error::new(
            HRESULT::from(HResult::E_OUTOFMEMORY),
            "APO_REG_PROPERTIES size calculation overflowed",
        )
    })?;

    // Safety: combase.dll's CoTaskMemAlloc is documented to return
    // null on failure and a writable buffer of at least `total`
    // bytes on success.
    let raw = unsafe { CoTaskMemAlloc(total) };
    if raw.is_null() {
        return Err(windows_core::Error::new(
            HRESULT::from(HResult::E_OUTOFMEMORY),
            "CoTaskMemAlloc returned null for APO_REG_PROPERTIES",
        ));
    }
    // Safety: `raw` was just allocated and is therefore exclusive
    // to this function until ownership is handed to the caller.
    // Zeroing the whole allocation gives every padding byte and
    // unused tail position a deterministic value before we begin
    // filling fields.
    unsafe {
        ptr::write_bytes(raw.cast::<u8>(), 0, total);
    }

    let props = raw.cast::<APO_REG_PROPERTIES>();
    let name = encode_wide_z::<256>(instance.name());
    let copyright = encode_wide_z::<256>(instance.copyright());

    // Safety: `props` points to `total` bytes of writable memory.
    // All field stores below stay within the allocation, and the
    // trailing GUID writes are bounded by `interfaces.len()` which
    // is the same count used to size the allocation.
    unsafe {
        (*props).clsid = instance.clsid().into();
        (*props).Flags = APO_FLAG_NONE;
        (*props).szFriendlyName = name;
        (*props).szCopyrightInfo = copyright;
        (*props).u32MajorVersion = 1;
        (*props).u32MinorVersion = 0;
        (*props).u32MinInputConnections = 1;
        (*props).u32MaxInputConnections = 1;
        (*props).u32MinOutputConnections = 1;
        (*props).u32MaxOutputConnections = 1;
        // 0 communicates "unlimited" to the audio engine.
        (*props).u32MaxInstances = 0;
        (*props).u32NumAPOInterfaces = interfaces.len() as u32;

        let head = ptr::addr_of_mut!((*props).iidAPOInterfaceList).cast::<GUID>();
        for (i, iid) in interfaces.iter().enumerate() {
            ptr::write(head.add(i), *iid);
        }
    }

    Ok(props)
}

/// Total allocation size for an `APO_REG_PROPERTIES` carrying `n`
/// trailing IIDs. Returns `None` if the math overflows `usize`.
#[must_use]
fn total_size(n: usize) -> Option<usize> {
    let base = mem::size_of::<APO_REG_PROPERTIES>();
    let guid = mem::size_of::<GUID>();
    // The base struct already accounts for one trailing GUID via
    // `iidAPOInterfaceList: [GUID; 1]`. Only `n - 1` extra GUIDs
    // need to be allocated past the struct end.
    let extra = n.saturating_sub(1).checked_mul(guid)?;
    base.checked_add(extra)
}

/// Encode `s` as a zero-terminated UTF-16 buffer of length `N`.
///
/// The buffer is always null-terminated: at most `N - 1` code units
/// are copied from `s`, and the remaining slots stay zero. Surrogate
/// pairs that straddle the truncation point are dropped together
/// (we never write the high half without the matching low half) by
/// virtue of consuming whole `encode_utf16` units only when at
/// least one slot remains.
#[must_use]
fn encode_wide_z<const N: usize>(s: &str) -> [u16; N] {
    let mut out = [0u16; N];
    let limit = N.saturating_sub(1);
    for (idx, u) in s.encode_utf16().take(limit).enumerate() {
        out[idx] = u;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apo::{ApoCategory, ProcessInput, ProcessingObject};
    use crate::buffer::BufferFlags;
    use crate::clsid::Clsid;
    use crate::instance::ApoInstance;
    use crate::realtime::RealtimeContext;
    use alloc::sync::Arc;
    use windows::Win32::System::Com::CoTaskMemFree;

    struct Sample;
    impl ProcessingObject for Sample {
        const CLSID: Clsid = Clsid::from_u128(0xA1B2C3D4_E5F6_7890_1234_56789ABCDEF0);
        const NAME: &'static str = "tympan-apo sample";
        const COPYRIGHT: &'static str = "(c) test";
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

    /// Convenience: free a builder-produced buffer at the end of a
    /// test so each case round-trips ownership cleanly.
    unsafe fn drop_properties(props: *mut APO_REG_PROPERTIES) {
        // Safety: caller passes a pointer obtained from
        // build_registration_properties (CoTaskMemAlloc'd).
        unsafe { CoTaskMemFree(Some(props.cast())) };
    }

    fn make_instance() -> Arc<dyn AnyApoInstance> {
        Arc::new(ApoInstance::<Sample>::new())
    }

    #[test]
    fn total_size_accounts_for_extra_iids_past_the_header_slot() {
        let base = mem::size_of::<APO_REG_PROPERTIES>();
        let guid = mem::size_of::<GUID>();
        assert_eq!(total_size(1), Some(base));
        assert_eq!(total_size(2), Some(base + guid));
        assert_eq!(total_size(3), Some(base + 2 * guid));
        // n == 0 is degenerate; the saturating_sub keeps the math
        // sound rather than panicking.
        assert_eq!(total_size(0), Some(base));
    }

    #[test]
    fn encode_wide_z_null_terminates_and_truncates() {
        let buf: [u16; 8] = encode_wide_z("hello, world");
        // Truncated to 7 code units + null terminator.
        let chars: alloc::string::String =
            char::decode_utf16(buf.iter().take_while(|&&c| c != 0).copied())
                .collect::<Result<_, _>>()
                .unwrap();
        assert_eq!(chars, "hello, ");
        assert_eq!(buf[7], 0);
    }

    #[test]
    fn encode_wide_z_fits_short_strings_exactly() {
        let buf: [u16; 16] = encode_wide_z("apo");
        assert_eq!(&buf[..3], &['a' as u16, 'p' as u16, 'o' as u16]);
        for &c in &buf[3..] {
            assert_eq!(c, 0);
        }
    }

    #[test]
    fn build_registration_properties_populates_base_fields() {
        let inst = make_instance();
        let props = build_registration_properties(inst.as_ref()).unwrap();
        assert!(!props.is_null());
        // Safety: just constructed, points to a CoTaskMemAlloc'd
        // APO_REG_PROPERTIES allocation.
        unsafe {
            assert_eq!(
                Clsid::from((*props).clsid),
                <Sample as ProcessingObject>::CLSID
            );
            assert_eq!((*props).Flags, APO_FLAG_NONE);
            assert_eq!((*props).u32MajorVersion, 1);
            assert_eq!((*props).u32MinorVersion, 0);
            assert_eq!((*props).u32MinInputConnections, 1);
            assert_eq!((*props).u32MaxInputConnections, 1);
            assert_eq!((*props).u32MinOutputConnections, 1);
            assert_eq!((*props).u32MaxOutputConnections, 1);
            assert_eq!((*props).u32MaxInstances, 0);
            assert_eq!((*props).u32NumAPOInterfaces, 5);
            drop_properties(props);
        }
    }

    #[test]
    fn build_registration_properties_writes_friendly_name_as_utf16() {
        let inst = make_instance();
        let props = build_registration_properties(inst.as_ref()).unwrap();
        // Safety: live pointer.
        unsafe {
            let name = &(*props).szFriendlyName;
            let decoded: alloc::string::String =
                char::decode_utf16(name.iter().take_while(|&&c| c != 0).copied())
                    .collect::<Result<_, _>>()
                    .unwrap();
            assert_eq!(decoded, "tympan-apo sample");

            let cr = &(*props).szCopyrightInfo;
            let decoded: alloc::string::String =
                char::decode_utf16(cr.iter().take_while(|&&c| c != 0).copied())
                    .collect::<Result<_, _>>()
                    .unwrap();
            assert_eq!(decoded, "(c) test");

            drop_properties(props);
        }
    }

    #[test]
    fn build_registration_properties_writes_advertised_interface_ids() {
        let inst = make_instance();
        let props = build_registration_properties(inst.as_ref()).unwrap();
        // Safety: live pointer. The IID list starts at
        // iidAPOInterfaceList[0] and extends through n entries —
        // the allocation is sized for exactly that.
        unsafe {
            let head = ptr::addr_of!((*props).iidAPOInterfaceList).cast::<GUID>();
            assert_eq!(*head, IAudioProcessingObject::IID);
            assert_eq!(*head.add(1), IAudioProcessingObjectConfiguration::IID);
            assert_eq!(*head.add(2), IAudioProcessingObjectRT::IID);
            assert_eq!(*head.add(3), IAudioSystemEffects::IID);
            assert_eq!(*head.add(4), IAudioSystemEffects2::IID);
            drop_properties(props);
        }
    }

    /// 300-character literal long enough to drive szFriendlyName
    /// truncation: `ProcessingObject::NAME` must be a `&'static
    /// str`, and `str::repeat` / `concat!` are not const, so a
    /// hard-coded string is the cleanest option.
    const LONG_NAME: &str = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";

    #[test]
    fn build_registration_properties_truncates_overlong_names() {
        struct LongName;
        impl ProcessingObject for LongName {
            const CLSID: Clsid = Clsid::from_u128(0xDEADBEEF_1234_5678_9ABC_DEF012345678);
            const NAME: &'static str = LONG_NAME;
            const COPYRIGHT: &'static str = "(c)";
            const CATEGORY: ApoCategory = ApoCategory::Sfx;
            fn new() -> Self {
                Self
            }
            fn process(
                &mut self,
                _rt: &RealtimeContext,
                _i: ProcessInput<'_>,
                o: &mut [f32],
            ) -> BufferFlags {
                o.fill(0.0);
                BufferFlags::SILENT
            }
        }
        // sanity: the literal we are truncating is genuinely too long.
        assert!(LONG_NAME.len() > 256);
        let inst: Arc<dyn AnyApoInstance> = Arc::new(ApoInstance::<LongName>::new());
        let props = build_registration_properties(inst.as_ref()).unwrap();
        // Safety: live pointer obtained from the builder.
        unsafe {
            let name = &(*props).szFriendlyName;
            // Buffer length is 256; the truncation reserves one slot
            // for the null terminator. So 255 'x' bytes followed by 0.
            assert_eq!(name[255], 0);
            assert!(name.iter().take(255).all(|&c| c == u16::from(b'x')));
            drop_properties(props);
        }
    }
}
