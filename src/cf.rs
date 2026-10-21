//! Small CoreFoundation and IORegistry helpers. Pointers returned as owned are released by the caller.
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};

pub const UTF8: u32 = 0x0800_0100;
const KERN_SUCCESS: i32 = 0;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub fn CFRelease(value: *const c_void);
    pub fn CFRetain(value: *const c_void) -> *const c_void;
    pub fn CFGetTypeID(value: *const c_void) -> usize;
    pub fn CFStringGetTypeID() -> usize;
    pub fn CFNumberGetTypeID() -> usize;
    pub fn CFDataGetTypeID() -> usize;
    pub fn CFStringCreateWithCString(
        allocator: *const c_void,
        cstr: *const c_char,
        encoding: u32,
    ) -> *const c_void;
    pub fn CFStringGetCString(
        string: *const c_void,
        buffer: *mut c_char,
        size: isize,
        encoding: u32,
    ) -> bool;
    pub fn CFNumberCreate(
        allocator: *const c_void,
        kind: i32,
        value: *const c_void,
    ) -> *const c_void;
    pub fn CFNumberGetValue(number: *const c_void, kind: i32, value: *mut c_void) -> bool;
    pub fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
    pub fn CFDataGetLength(data: *const c_void) -> isize;
    pub fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
    pub fn CFDictionaryCreate(
        allocator: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> *const c_void;
    pub fn CFArrayGetCount(array: *const c_void) -> isize;
    pub fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    pub static kCFTypeDictionaryKeyCallBacks: u8;
    pub static kCFTypeDictionaryValueCallBacks: u8;

    pub fn CFURLCreateFromFileSystemRepresentation(
        allocator: *const c_void,
        path: *const u8,
        length: isize,
        is_directory: bool,
    ) -> *const c_void;
    pub fn CFBundleCreate(allocator: *const c_void, url: *const c_void) -> *const c_void;
    pub fn CFBundleGetIdentifier(bundle: *const c_void) -> *const c_void;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    pub fn IOObjectRelease(object: u32) -> i32;
    pub fn IORegistryEntryFromPath(main_port: u32, path: *const c_char) -> u32;
    pub fn IORegistryEntryCreateCFProperties(
        entry: u32,
        properties: *mut *const c_void,
        allocator: *const c_void,
        options: u32,
    ) -> i32;
    pub fn IORegistryEntryCreateCFProperty(
        entry: u32,
        key: *const c_void,
        allocator: *const c_void,
        options: u32,
    ) -> *const c_void;
    pub fn IORegistryEntryGetName(entry: u32, name: *mut c_char) -> i32;
    pub fn IORegistryEntryGetChildIterator(
        entry: u32,
        plane: *const c_char,
        iterator: *mut u32,
    ) -> i32;
    pub fn IOServiceMatching(name: *const c_char) -> *const c_void;
    pub fn IOServiceGetMatchingServices(
        main_port: u32,
        matching: *const c_void,
        iterator: *mut u32,
    ) -> i32;
    pub fn IOIteratorNext(iterator: u32) -> u32;
}

pub fn string(value: *const c_void) -> Option<String> {
    if value.is_null() || unsafe { CFGetTypeID(value) } != unsafe { CFStringGetTypeID() } {
        return None;
    }
    let mut buffer = vec![0u8; 1024];
    let ok = unsafe {
        CFStringGetCString(
            value,
            buffer.as_mut_ptr().cast(),
            buffer.len() as isize,
            UTF8,
        )
    };
    if !ok {
        return None;
    }
    CStr::from_bytes_until_nul(&buffer)
        .ok()
        .map(|text| text.to_string_lossy().into_owned())
}

pub fn data_bytes(value: *const c_void) -> Option<Vec<u8>> {
    if value.is_null() || unsafe { CFGetTypeID(value) } != unsafe { CFDataGetTypeID() } {
        return None;
    }
    let length = unsafe { CFDataGetLength(value) };
    if length <= 0 {
        return Some(Vec::new());
    }
    let pointer = unsafe { CFDataGetBytePtr(value) };
    if pointer.is_null() {
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(pointer, length as usize) }.to_vec())
}

pub fn number_f64(value: *const c_void) -> Option<f64> {
    if value.is_null() || unsafe { CFGetTypeID(value) } != unsafe { CFNumberGetTypeID() } {
        return None;
    }
    let mut integer: i64 = 0;
    if unsafe { CFNumberGetValue(value, 4, (&mut integer as *mut i64).cast()) } {
        return Some(integer as f64);
    }
    let mut real: f64 = 0.0;
    if unsafe { CFNumberGetValue(value, 13, (&mut real as *mut f64).cast()) } {
        return Some(real);
    }
    None
}

pub fn dict_get(dict: *const c_void, key: &str) -> *const c_void {
    if dict.is_null() {
        return std::ptr::null();
    }
    let Ok(c_key) = CString::new(key) else {
        return std::ptr::null();
    };
    let cf_key = unsafe { CFStringCreateWithCString(std::ptr::null(), c_key.as_ptr(), UTF8) };
    if cf_key.is_null() {
        return std::ptr::null();
    }
    let value = unsafe { CFDictionaryGetValue(dict, cf_key) };
    unsafe { CFRelease(cf_key) };
    value
}

pub fn cf_string(text: &str) -> *const c_void {
    let Ok(c_text) = CString::new(text) else {
        return std::ptr::null();
    };
    unsafe { CFStringCreateWithCString(std::ptr::null(), c_text.as_ptr(), UTF8) }
}

pub fn entry_properties(entry: u32) -> *const c_void {
    let mut properties: *const c_void = std::ptr::null();
    let result =
        unsafe { IORegistryEntryCreateCFProperties(entry, &mut properties, std::ptr::null(), 0) };
    if result == KERN_SUCCESS {
        properties
    } else {
        std::ptr::null()
    }
}

pub fn entry_name(entry: u32) -> String {
    let mut buffer = [0u8; 128];
    let result = unsafe { IORegistryEntryGetName(entry, buffer.as_mut_ptr().cast()) };
    if result != KERN_SUCCESS {
        return String::new();
    }
    CStr::from_bytes_until_nul(&buffer)
        .map(|text| text.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn services(class_name: &str) -> Vec<u32> {
    let Ok(name) = CString::new(class_name) else {
        return Vec::new();
    };
    let matching = unsafe { IOServiceMatching(name.as_ptr()) };
    if matching.is_null() {
        return Vec::new();
    }
    let mut iterator = 0u32;
    if unsafe { IOServiceGetMatchingServices(0, matching, &mut iterator) } != KERN_SUCCESS {
        return Vec::new();
    }
    let mut entries = Vec::new();
    loop {
        let service = unsafe { IOIteratorNext(iterator) };
        if service == 0 {
            break;
        }
        entries.push(service);
    }
    unsafe { IOObjectRelease(iterator) };
    entries
}

pub fn bundle_identifier(app_path: &str) -> Option<String> {
    let url = unsafe {
        CFURLCreateFromFileSystemRepresentation(
            std::ptr::null(),
            app_path.as_ptr(),
            app_path.len() as isize,
            true,
        )
    };
    if url.is_null() {
        return None;
    }
    let bundle = unsafe { CFBundleCreate(std::ptr::null(), url) };
    unsafe { CFRelease(url) };
    if bundle.is_null() {
        return None;
    }
    let identifier = string(unsafe { CFBundleGetIdentifier(bundle) });
    unsafe { CFRelease(bundle) };
    identifier
}
