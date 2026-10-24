//! Adapter identity is execution evidence, never permission to silently select
//! another GPU when resuming a job. The system driver remains user managed.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Adapter {
    pub name: String,
    pub uuid: String,
}
impl Adapter {
    pub(crate) fn validate(&self) -> bool {
        !self.name.is_empty()
            && self.name.len() <= 256
            && self.uuid.len() == 32
            && self
                .uuid
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }
    pub(crate) fn available(&self) -> bool {
        // Both bundled engines explicitly select Vulkan0. Device reordering
        // must require a new job rather than execute on another adapter.
        self.validate() && adapters().first() == Some(self)
    }
}
pub(crate) fn adapters() -> Vec<Adapter> {
    #[cfg(windows)]
    {
        windows::adapters().unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}
#[cfg(windows)]
mod windows {
    use super::*;
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryExW(path: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        fn FreeLibrary(module: *mut c_void) -> i32;
    }
    #[repr(C)]
    struct CreateInfo {
        kind: u32,
        next: *const c_void,
        flags: u32,
        application: *const c_void,
        layer_count: u32,
        layers: *const *const u8,
        extension_count: u32,
        extensions: *const *const u8,
    }
    #[repr(C)]
    struct Application {
        kind: u32,
        next: *const c_void,
        name: *const u8,
        version: u32,
        engine: *const u8,
        engine_version: u32,
        api_version: u32,
    }
    #[repr(C)]
    struct DeviceId {
        kind: u32,
        next: *mut c_void,
        uuid: [u8; 16],
        driver: [u8; 16],
        luid: [u8; 8],
        node: u32,
        luid_valid: u32,
    }
    #[repr(C)]
    struct Properties2 {
        kind: u32,
        next: *mut c_void,
        data: [u64; 128],
    }
    type Create =
        unsafe extern "system" fn(*const CreateInfo, *const c_void, *mut *mut c_void) -> i32;
    type Enumerate = unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut c_void) -> i32;
    type Properties = unsafe extern "system" fn(*mut c_void, *mut Properties2);
    type Destroy = unsafe extern "system" fn(*mut c_void, *const c_void);
    pub(super) fn adapters() -> Option<Vec<Adapter>> {
        unsafe {
            let path: Vec<u16> = "vulkan-1.dll".encode_utf16().chain(Some(0)).collect();
            let library = LoadLibraryExW(path.as_ptr(), std::ptr::null_mut(), 0x800);
            if library.is_null() {
                return None;
            }
            struct Library(*mut c_void);
            impl Drop for Library {
                fn drop(&mut self) {
                    unsafe {
                        FreeLibrary(self.0);
                    }
                }
            }
            let _library = Library(library);
            let create = GetProcAddress(library, c"vkCreateInstance".as_ptr().cast());
            let enumerate = GetProcAddress(library, c"vkEnumeratePhysicalDevices".as_ptr().cast());
            let properties =
                GetProcAddress(library, c"vkGetPhysicalDeviceProperties2".as_ptr().cast());
            let destroy = GetProcAddress(library, c"vkDestroyInstance".as_ptr().cast());
            if [create, enumerate, properties, destroy]
                .iter()
                .any(|p| p.is_null())
            {
                return None;
            }
            let create: Create = std::mem::transmute(create);
            let enumerate: Enumerate = std::mem::transmute(enumerate);
            let properties: Properties = std::mem::transmute(properties);
            let destroy: Destroy = std::mem::transmute(destroy);
            let application = Application {
                kind: 0,
                next: std::ptr::null(),
                name: c"MON GPU admission".as_ptr().cast(),
                version: 1,
                engine: std::ptr::null(),
                engine_version: 0,
                api_version: (1 << 22) | (1 << 12),
            };
            let info = CreateInfo {
                kind: 1,
                next: std::ptr::null(),
                flags: 0,
                application: (&application as *const Application).cast(),
                layer_count: 0,
                layers: std::ptr::null(),
                extension_count: 0,
                extensions: std::ptr::null(),
            };
            let mut instance = std::ptr::null_mut();
            if create(&info, std::ptr::null(), &mut instance) != 0 {
                return None;
            }
            struct Instance(*mut c_void, Destroy);
            impl Drop for Instance {
                fn drop(&mut self) {
                    unsafe {
                        (self.1)(self.0, std::ptr::null());
                    }
                }
            }
            let _instance = Instance(instance, destroy);
            let mut count = 0;
            if enumerate(instance, &mut count, std::ptr::null_mut()) != 0 || count > 64 {
                return None;
            }
            let mut devices = vec![std::ptr::null_mut(); count as usize];
            if enumerate(instance, &mut count, devices.as_mut_ptr()) != 0
                || count as usize > devices.len()
            {
                return None;
            }
            let mut result = Vec::new();
            for device in devices.into_iter().take(count as usize) {
                let mut id = DeviceId {
                    kind: 1000071004,
                    next: std::ptr::null_mut(),
                    uuid: [0; 16],
                    driver: [0; 16],
                    luid: [0; 8],
                    node: 0,
                    luid_valid: 0,
                };
                let mut data = Properties2 {
                    kind: 1000059001,
                    next: (&mut id as *mut DeviceId).cast(),
                    data: [0; 128],
                };
                properties(device, &mut data);
                let bytes = std::slice::from_raw_parts(data.data.as_ptr().cast::<u8>(), 1024);
                let kind = u32::from_ne_bytes(bytes[16..20].try_into().ok()?);
                if !(1..=3).contains(&kind) {
                    continue;
                }
                let end = bytes[20..276].iter().position(|v| *v == 0)?;
                let name = std::str::from_utf8(&bytes[20..20 + end]).ok()?.to_owned();
                if id.uuid.iter().all(|b| *b == 0) {
                    continue;
                }
                let uuid = id.uuid.iter().map(|b| format!("{b:02x}")).collect();
                let adapter = Adapter { name, uuid };
                if adapter.validate() {
                    result.push(adapter);
                }
            }
            Some(result)
        }
    }
}
