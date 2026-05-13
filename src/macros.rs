//! User-facing macros.
//!
//! The only macro for now is [`crate::register_apo`], which emits the
//! four standard COM in-process server entry points
//! (`DllGetClassObject`, `DllCanUnloadNow`, `DllRegisterServer`,
//! `DllUnregisterServer`) wired to a `T: ProcessingObject`.

/// Register a `T: ProcessingObject` as the APO this DLL exposes.
///
/// Emits, in the calling scope:
///
/// - A `static __TYMPAN_APO_VTABLE` of type
///   [`crate::raw::class_factory::ApoVTable`] populated from
///   `T`'s associated constants and a creator function that
///   yields `Arc<dyn AnyApoInstance>` over [`crate::instance::ApoInstance<T>`].
/// - A `static __TYMPAN_APO_REGISTRY` array of length 1
///   pointing at the vtable.
/// - `#[no_mangle] pub unsafe extern "system" fn DllGetClassObject`
///   that forwards to
///   [`crate::raw::exports::dll_get_class_object_dispatch`] with
///   the registry above.
/// - `#[no_mangle] pub unsafe extern "system" fn DllCanUnloadNow`,
///   `DllRegisterServer`, and `DllUnregisterServer` as stubs
///   (return `S_FALSE` / `S_OK` / `S_OK` respectively; real
///   registry-write bodies land in a later PR).
///
/// ## Usage
///
/// ```ignore
/// use tympan_apo::{ProcessingObject, ProcessInput, BufferFlags,
///                  RealtimeContext, ApoCategory, Clsid};
///
/// struct MyApo;
/// impl ProcessingObject for MyApo {
///     const CLSID: Clsid = Clsid::from_u128(
///         0x12345678_1234_5678_1234_567812345678);
///     const NAME: &'static str = "My APO";
///     const COPYRIGHT: &'static str = "© 2026";
///     const CATEGORY: ApoCategory = ApoCategory::Sfx;
///     fn new() -> Self { Self }
///     fn process(
///         &mut self,
///         _rt: &RealtimeContext,
///         input: ProcessInput<'_>,
///         output: &mut [f32],
///     ) -> BufferFlags {
///         output.copy_from_slice(input.samples());
///         input.flags()
///     }
/// }
///
/// tympan_apo::register_apo!(MyApo);
/// ```
///
/// ## Single call per crate
///
/// The macro emits items with fixed `__TYMPAN_APO_*` symbol
/// names, including `#[no_mangle]` entry points which must be
/// unique in the link tree. Each cdylib must therefore invoke
/// `register_apo!` exactly once at the crate root.
#[macro_export]
macro_rules! register_apo {
    ($t:ty) => {
        /// Creator routed into the `ApoVTable::create` function
        /// pointer slot.
        #[doc(hidden)]
        fn __tympan_apo_create() -> ::std::sync::Arc<dyn $crate::instance::AnyApoInstance> {
            ::std::sync::Arc::new($crate::instance::ApoInstance::<$t>::new())
        }

        /// Per-APO vtable, consumed by `dll_get_class_object_dispatch`.
        #[doc(hidden)]
        pub static __TYMPAN_APO_VTABLE: $crate::raw::class_factory::ApoVTable =
            $crate::raw::class_factory::ApoVTable {
                clsid: <$t as $crate::ProcessingObject>::CLSID,
                name: <$t as $crate::ProcessingObject>::NAME,
                copyright: <$t as $crate::ProcessingObject>::COPYRIGHT,
                category: <$t as $crate::ProcessingObject>::CATEGORY,
                create: __tympan_apo_create,
            };

        /// Registry handed to `dll_get_class_object_dispatch`.
        #[doc(hidden)]
        static __TYMPAN_APO_REGISTRY: [&'static $crate::raw::class_factory::ApoVTable; 1] =
            [&__TYMPAN_APO_VTABLE];

        /// COM class-object factory entry point.
        ///
        /// # Safety
        ///
        /// Called by COM; arguments follow the
        /// `DllGetClassObject` ABI documented at
        /// <https://learn.microsoft.com/en-us/windows/win32/api/combaseapi/nf-combaseapi-dllgetclassobject>.
        #[no_mangle]
        pub unsafe extern "system" fn DllGetClassObject(
            rclsid: *const $crate::GUID,
            riid: *const $crate::GUID,
            ppv: *mut *mut ::core::ffi::c_void,
        ) -> $crate::HRESULT {
            // Safety: forwarded directly from the COM caller.
            unsafe {
                $crate::raw::exports::dll_get_class_object_dispatch(
                    rclsid,
                    riid,
                    ppv,
                    &__TYMPAN_APO_REGISTRY,
                )
            }
        }

        /// COM unload-readiness query.
        ///
        /// Returns `S_FALSE` (1) to keep the DLL loaded — the
        /// outstanding-instance counter is wired in a follow-up
        /// PR; until then, the DLL never reports itself as
        /// unloadable.
        ///
        /// # Safety
        ///
        /// Called by COM; takes no parameters.
        #[no_mangle]
        pub unsafe extern "system" fn DllCanUnloadNow() -> $crate::HRESULT {
            $crate::HRESULT(1)
        }

        /// COM self-registration entry point.
        ///
        /// Returns `S_OK` without touching the registry; real
        /// CLSID-key writes land alongside the registration
        /// helper in a later PR.
        ///
        /// # Safety
        ///
        /// Called by `regsvr32`; takes no parameters.
        #[no_mangle]
        pub unsafe extern "system" fn DllRegisterServer() -> $crate::HRESULT {
            $crate::HRESULT(0)
        }

        /// Inverse of `DllRegisterServer`. See its documentation.
        ///
        /// # Safety
        ///
        /// Called by `regsvr32 /u`; takes no parameters.
        #[no_mangle]
        pub unsafe extern "system" fn DllUnregisterServer() -> $crate::HRESULT {
            $crate::HRESULT(0)
        }
    };
}
