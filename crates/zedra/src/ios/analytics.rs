// iOS Firebase Analytics + Crashlytics bridge
//
// Declares the C-linkage functions defined in ios/Zedra/ZedraFirebase.m and
// exposes safe Rust wrappers.  The symbols are resolved at link time when the
// app target links the Rust static library alongside ZedraFirebase.m.

use std::ffi::{CString, c_char, c_int};

unsafe extern "C" {
    fn zedra_log_event(
        name: *const c_char,
        keys: *const *const c_char,
        values: *const *const c_char,
        count: c_int,
    );
    fn zedra_record_error(message: *const c_char, file: *const c_char, line: c_int);
    fn zedra_record_panic(message: *const c_char, location: *const c_char);
    fn zedra_set_user_id(user_id: *const c_char);
    fn zedra_set_custom_key(key: *const c_char, value: *const c_char);
}

pub fn log_event(name: &str, params: &[(&str, &str)]) {
    let Ok(cname) = CString::new(name) else { return };

    let ckeys: Vec<CString> = params
        .iter()
        .filter_map(|(k, _)| CString::new(*k).ok())
        .collect();
    let cvals: Vec<CString> = params
        .iter()
        .filter_map(|(_, v)| CString::new(*v).ok())
        .collect();
    let key_ptrs: Vec<*const c_char> = ckeys.iter().map(|s| s.as_ptr()).collect();
    let val_ptrs: Vec<*const c_char> = cvals.iter().map(|s| s.as_ptr()).collect();

    unsafe {
        zedra_log_event(
            cname.as_ptr(),
            key_ptrs.as_ptr(),
            val_ptrs.as_ptr(),
            params.len() as c_int,
        );
    }
}

pub fn record_error(message: &str, file: &str, line: u32) {
    let Ok(cmsg) = CString::new(message) else { return };
    let Ok(cfile) = CString::new(file) else { return };
    unsafe { zedra_record_error(cmsg.as_ptr(), cfile.as_ptr(), line as c_int) };
}

pub fn record_panic(message: &str, location: &str) {
    let Ok(cmsg) = CString::new(message) else { return };
    let Ok(cloc) = CString::new(location) else { return };
    unsafe { zedra_record_panic(cmsg.as_ptr(), cloc.as_ptr()) };
}

pub fn set_user_id(id: &str) {
    let Ok(cid) = CString::new(id) else { return };
    unsafe { zedra_set_user_id(cid.as_ptr()) };
}

pub fn set_custom_key(key: &str, value: &str) {
    let Ok(ck) = CString::new(key) else { return };
    let Ok(cv) = CString::new(value) else { return };
    unsafe { zedra_set_custom_key(ck.as_ptr(), cv.as_ptr()) };
}
