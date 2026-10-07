//! # StateFS Universal C-ABI Bridge
//!
//! Provides an `extern "C"` FFI layer enabling C, C++, C#, Python, Go, and game engines
//! to interact with StateFS in-memory trees with zero Rust runtime dependencies.
//!
//! # Safety Guarantees
//!
//! - **Panic Barrier**: All entry points use `std::panic::catch_unwind` to prevent unwinding across the C ABI.
//! - **Null-Pointer Safety**: All pointer arguments are guarded against `NULL`.
//! - **Thread Safety**: Independent stores are fully isolated; operations on a single store follow standard C pointer semantics.

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::catch_unwind;
use std::ptr;
use std::slice;

use statefs_adapter_bridge_env::EnvSource;
use statefs_codec_bin::{export_snapshot_bytes, restore_snapshot_bytes};
use statefs_codec_json::ingest_json;
use statefs_codec_toml::ingest_toml;
use statefs_core::{MemStore, Path, Store, Value};

/// Opaque handle representing a StateFS memory store.
pub struct StatefsStore {
    inner: MemStore,
}

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

fn set_last_error(err_msg: &str) {
    let clean = err_msg.replace('\0', " ");
    LAST_ERROR.with(|cell| {
        *cell.borrow_mut() = CString::new(clean).ok();
    });
}

fn clear_last_error() {
    LAST_ERROR.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

/// Retrieves the most recent error message on the calling thread.
///
/// Returns the number of bytes written, or 0 if no error is recorded or buffer is invalid.
///
/// # Safety
///
/// Caller must pass a valid, non-null pointer `buf` pointing to at least `max_len` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn statefs_last_error(buf: *mut c_char, max_len: usize) -> usize {
    if buf.is_null() || max_len == 0 {
        return 0;
    }

    LAST_ERROR.with(|cell| {
        if let Some(ref err) = *cell.borrow() {
            let bytes = err.as_bytes_with_nul();
            let to_copy = bytes.len().min(max_len);
            // SAFETY: Caller guarantees buf is valid for max_len bytes.
            unsafe {
                ptr::copy_nonoverlapping(bytes.as_ptr() as *const c_char, buf, to_copy);
                if to_copy < bytes.len() {
                    *buf.add(max_len - 1) = 0;
                }
            }
            to_copy
        } else {
            // SAFETY: Caller guarantees buf has at least 1 byte capacity.
            unsafe {
                *buf = 0;
            }
            0
        }
    })
}

/// Allocates and initializes a new empty StateFS store.
///
/// Must be freed using [`statefs_store_free`].
#[unsafe(no_mangle)]
pub extern "C" fn statefs_store_new() -> *mut StatefsStore {
    let result = catch_unwind(|| {
        clear_last_error();
        Box::into_raw(Box::new(StatefsStore {
            inner: MemStore::new(),
        }))
    });

    match result {
        Ok(ptr) => ptr,
        Err(_) => {
            set_last_error("Panic occurred while allocating StatefsStore");
            ptr::null_mut()
        }
    }
}

/// Deallocates a previously created StateFS store. Safe no-op if `store` is NULL.
///
/// # Safety
///
/// `store` must either be `NULL` or a valid pointer returned by [`statefs_store_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn statefs_store_free(store: *mut StatefsStore) {
    if store.is_null() {
        return;
    }
    let _ = catch_unwind(|| {
        // SAFETY: Pointer was verified non-null and was allocated by Box::into_raw in statefs_store_new.
        unsafe { drop(Box::from_raw(store)) };
    });
}

/// Inserts a string value at `path`. Returns 0 on success, -1 on error.
///
/// # Safety
///
/// `store` must be a valid pointer to a `StatefsStore`. `path_cstr` and `val_cstr` must be valid,
/// null-terminated UTF-8 strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn statefs_store_insert_str(
    store: *mut StatefsStore,
    path_cstr: *const c_char,
    val_cstr: *const c_char,
) -> i32 {
    let result = catch_unwind(|| {
        clear_last_error();
        if store.is_null() || path_cstr.is_null() || val_cstr.is_null() {
            set_last_error("Null pointer argument passed to statefs_store_insert_str");
            return -1;
        }

        // SAFETY: Pointers checked non-null; valid null-terminated strings required.
        let path_str = match unsafe { CStr::from_ptr(path_cstr) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(&format!("Invalid UTF-8 in path: {e}"));
                return -1;
            }
        };

        let val_str = match unsafe { CStr::from_ptr(val_cstr) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(&format!("Invalid UTF-8 in value: {e}"));
                return -1;
            }
        };

        // SAFETY: store is verified non-null.
        let store_ref = unsafe { &mut (*store).inner };
        let path = Path::parse(path_str);
        if let Err(e) = store_ref.insert(&path, Value::String(val_str.to_string())) {
            set_last_error(&format!("Insert error: {e}"));
            return -1;
        }

        0
    });

    result.unwrap_or_else(|_| {
        set_last_error("Panic occurred in statefs_store_insert_str");
        -1
    })
}

/// Inserts a 64-bit signed integer value at `path`. Returns 0 on success, -1 on error.
///
/// # Safety
///
/// `store` must be a valid pointer to a `StatefsStore`. `path_cstr` must be a valid null-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn statefs_store_insert_int(
    store: *mut StatefsStore,
    path_cstr: *const c_char,
    val: i64,
) -> i32 {
    let result = catch_unwind(|| {
        clear_last_error();
        if store.is_null() || path_cstr.is_null() {
            set_last_error("Null pointer argument passed to statefs_store_insert_int");
            return -1;
        }

        let path_str = match unsafe { CStr::from_ptr(path_cstr) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(&format!("Invalid UTF-8 in path: {e}"));
                return -1;
            }
        };

        let store_ref = unsafe { &mut (*store).inner };
        let path = Path::parse(path_str);
        if let Err(e) = store_ref.insert(&path, Value::Int(val)) {
            set_last_error(&format!("Insert error: {e}"));
            return -1;
        }

        0
    });

    result.unwrap_or_else(|_| {
        set_last_error("Panic occurred in statefs_store_insert_int");
        -1
    })
}

/// Inserts a boolean value (0 = false, non-zero = true) at `path`. Returns 0 on success, -1 on error.
///
/// # Safety
///
/// `store` must be a valid pointer to a `StatefsStore`. `path_cstr` must be a valid null-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn statefs_store_insert_bool(
    store: *mut StatefsStore,
    path_cstr: *const c_char,
    val: i32,
) -> i32 {
    let result = catch_unwind(|| {
        clear_last_error();
        if store.is_null() || path_cstr.is_null() {
            set_last_error("Null pointer argument passed to statefs_store_insert_bool");
            return -1;
        }

        let path_str = match unsafe { CStr::from_ptr(path_cstr) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(&format!("Invalid UTF-8 in path: {e}"));
                return -1;
            }
        };

        let store_ref = unsafe { &mut (*store).inner };
        let path = Path::parse(path_str);
        if let Err(e) = store_ref.insert(&path, Value::Bool(val != 0)) {
            set_last_error(&format!("Insert error: {e}"));
            return -1;
        }

        0
    });

    result.unwrap_or_else(|_| {
        set_last_error("Panic occurred in statefs_store_insert_bool");
        -1
    })
}

/// Inserts a 64-bit float value at `path`. Returns 0 on success, -1 on error.
///
/// # Safety
///
/// `store` must be a valid pointer to a `StatefsStore`. `path_cstr` must be a valid null-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn statefs_store_insert_float(
    store: *mut StatefsStore,
    path_cstr: *const c_char,
    val: f64,
) -> i32 {
    let result = catch_unwind(|| {
        clear_last_error();
        if store.is_null() || path_cstr.is_null() {
            set_last_error("Null pointer argument passed to statefs_store_insert_float");
            return -1;
        }

        let path_str = match unsafe { CStr::from_ptr(path_cstr) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(&format!("Invalid UTF-8 in path: {e}"));
                return -1;
            }
        };

        let store_ref = unsafe { &mut (*store).inner };
        let path = Path::parse(path_str);
        if let Err(e) = store_ref.insert(&path, Value::Float(val)) {
            set_last_error(&format!("Insert error: {e}"));
            return -1;
        }

        0
    });

    result.unwrap_or_else(|_| {
        set_last_error("Panic occurred in statefs_store_insert_float");
        -1
    })
}

/// Reads a string value from `path` into `buf`.
///
/// Returns 0 on success, 1 if path not found, -1 on error/type mismatch.
///
/// # Safety
///
/// `store` and `path_cstr` must be valid. `buf` must point to at least `buf_len` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn statefs_store_get_str(
    store: *const StatefsStore,
    path_cstr: *const c_char,
    buf: *mut c_char,
    buf_len: usize,
    written: *mut usize,
) -> i32 {
    let result = catch_unwind(|| {
        clear_last_error();
        if store.is_null() || path_cstr.is_null() || buf.is_null() || buf_len == 0 {
            set_last_error("Null or invalid buffer argument passed to statefs_store_get_str");
            return -1;
        }

        let path_str = match unsafe { CStr::from_ptr(path_cstr) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(&format!("Invalid UTF-8 in path: {e}"));
                return -1;
            }
        };

        let store_ref = unsafe { &(*store).inner };
        match store_ref.get_str(path_str) {
            Some(node) => match node.value.as_str() {
                Some(s) => {
                    let bytes = s.as_bytes();
                    let to_copy = bytes.len().min(buf_len - 1);
                    unsafe {
                        ptr::copy_nonoverlapping(bytes.as_ptr() as *const c_char, buf, to_copy);
                        *buf.add(to_copy) = 0;
                        if !written.is_null() {
                            *written = to_copy;
                        }
                    }
                    0
                }
                None => {
                    set_last_error("Node exists but value is not a string");
                    -1
                }
            },
            None => 1, // Not found
        }
    });

    result.unwrap_or_else(|_| {
        set_last_error("Panic occurred in statefs_store_get_str");
        -1
    })
}

/// Reads a 64-bit integer from `path` into `val_out`.
///
/// Returns 0 on success, 1 if not found, -1 on type mismatch or null args.
///
/// # Safety
///
/// `store` and `path_cstr` must be valid. `val_out` must be a valid pointer to writable `i64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn statefs_store_get_int(
    store: *const StatefsStore,
    path_cstr: *const c_char,
    val_out: *mut i64,
) -> i32 {
    let result = catch_unwind(|| {
        clear_last_error();
        if store.is_null() || path_cstr.is_null() || val_out.is_null() {
            set_last_error("Null argument passed to statefs_store_get_int");
            return -1;
        }

        let path_str = match unsafe { CStr::from_ptr(path_cstr) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(&format!("Invalid UTF-8 in path: {e}"));
                return -1;
            }
        };

        let store_ref = unsafe { &(*store).inner };
        match store_ref.get_str(path_str) {
            Some(node) => match node.value.as_int() {
                Some(i) => {
                    unsafe { *val_out = i };
                    0
                }
                None => {
                    set_last_error("Node value is not an integer");
                    -1
                }
            },
            None => 1,
        }
    });

    result.unwrap_or_else(|_| {
        set_last_error("Panic occurred in statefs_store_get_int");
        -1
    })
}

/// Reads a boolean from `path` into `val_out` (0 = false, 1 = true).
///
/// Returns 0 on success, 1 if not found, -1 on error.
///
/// # Safety
///
/// `store` and `path_cstr` must be valid. `val_out` must point to a writable `i32`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn statefs_store_get_bool(
    store: *const StatefsStore,
    path_cstr: *const c_char,
    val_out: *mut i32,
) -> i32 {
    let result = catch_unwind(|| {
        clear_last_error();
        if store.is_null() || path_cstr.is_null() || val_out.is_null() {
            set_last_error("Null argument passed to statefs_store_get_bool");
            return -1;
        }

        let path_str = match unsafe { CStr::from_ptr(path_cstr) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(&format!("Invalid UTF-8 in path: {e}"));
                return -1;
            }
        };

        let store_ref = unsafe { &(*store).inner };
        match store_ref.get_str(path_str) {
            Some(node) => match node.value.as_bool() {
                Some(b) => {
                    unsafe { *val_out = if b { 1 } else { 0 } };
                    0
                }
                None => {
                    set_last_error("Node value is not a boolean");
                    -1
                }
            },
            None => 1,
        }
    });

    result.unwrap_or_else(|_| {
        set_last_error("Panic occurred in statefs_store_get_bool");
        -1
    })
}

/// Reads a float from `path` into `val_out`.
///
/// Returns 0 on success, 1 if not found, -1 on error.
///
/// # Safety
///
/// `store` and `path_cstr` must be valid. `val_out` must point to a writable `f64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn statefs_store_get_float(
    store: *const StatefsStore,
    path_cstr: *const c_char,
    val_out: *mut f64,
) -> i32 {
    let result = catch_unwind(|| {
        clear_last_error();
        if store.is_null() || path_cstr.is_null() || val_out.is_null() {
            set_last_error("Null argument passed to statefs_store_get_float");
            return -1;
        }

        let path_str = match unsafe { CStr::from_ptr(path_cstr) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(&format!("Invalid UTF-8 in path: {e}"));
                return -1;
            }
        };

        let store_ref = unsafe { &(*store).inner };
        match store_ref.get_str(path_str) {
            Some(node) => match node.value.as_float() {
                Some(f) => {
                    unsafe { *val_out = f };
                    0
                }
                None => {
                    set_last_error("Node value is not a float");
                    -1
                }
            },
            None => 1,
        }
    });

    result.unwrap_or_else(|_| {
        set_last_error("Panic occurred in statefs_store_get_float");
        -1
    })
}

/// Checks if a node exists at `path`. Returns 1 if present, 0 if not, -1 on error.
///
/// # Safety
///
/// `store` and `path_cstr` must be valid non-null pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn statefs_store_contains(
    store: *const StatefsStore,
    path_cstr: *const c_char,
) -> i32 {
    let result = catch_unwind(|| {
        clear_last_error();
        if store.is_null() || path_cstr.is_null() {
            set_last_error("Null argument passed to statefs_store_contains");
            return -1;
        }

        let path_str = match unsafe { CStr::from_ptr(path_cstr) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(&format!("Invalid UTF-8 in path: {e}"));
                return -1;
            }
        };

        let store_ref = unsafe { &(*store).inner };
        if store_ref.get_str(path_str).is_some() {
            1
        } else {
            0
        }
    });

    result.unwrap_or_else(|_| {
        set_last_error("Panic occurred in statefs_store_contains");
        -1
    })
}

/// Removes the node at `path`. Returns 0 on success, 1 if not found, -1 on error.
///
/// # Safety
///
/// `store` and `path_cstr` must be valid non-null pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn statefs_store_remove(
    store: *mut StatefsStore,
    path_cstr: *const c_char,
) -> i32 {
    let result = catch_unwind(|| {
        clear_last_error();
        if store.is_null() || path_cstr.is_null() {
            set_last_error("Null argument passed to statefs_store_remove");
            return -1;
        }

        let path_str = match unsafe { CStr::from_ptr(path_cstr) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(&format!("Invalid UTF-8 in path: {e}"));
                return -1;
            }
        };

        let store_ref = unsafe { &mut (*store).inner };
        let path = Path::parse(path_str);
        match store_ref.remove(&path) {
            Ok(Some(_)) => 0,
            Ok(None) => 1,
            Err(e) => {
                set_last_error(&format!("Remove error: {e}"));
                -1
            }
        }
    });

    result.unwrap_or_else(|_| {
        set_last_error("Panic occurred in statefs_store_remove");
        -1
    })
}

/// Ingests a JSON string into `store`. Returns 0 on success, -1 on error.
///
/// # Safety
///
/// `store` must be a valid pointer to a `StatefsStore`. `json_cstr` must be a valid null-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn statefs_store_load_json_str(
    store: *mut StatefsStore,
    json_cstr: *const c_char,
) -> i32 {
    let result = catch_unwind(|| {
        clear_last_error();
        if store.is_null() || json_cstr.is_null() {
            set_last_error("Null argument passed to statefs_store_load_json_str");
            return -1;
        }

        let json_str = match unsafe { CStr::from_ptr(json_cstr) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(&format!("Invalid UTF-8 in json: {e}"));
                return -1;
            }
        };

        let store_ref = unsafe { &mut (*store).inner };
        if let Err(e) = ingest_json(store_ref, json_str) {
            set_last_error(&format!("JSON parse error: {e}"));
            return -1;
        }

        0
    });

    result.unwrap_or_else(|_| {
        set_last_error("Panic occurred in statefs_store_load_json_str");
        -1
    })
}

/// Ingests a TOML string into `store`. Returns 0 on success, -1 on error.
///
/// # Safety
///
/// `store` must be a valid pointer to a `StatefsStore`. `toml_cstr` must be a valid null-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn statefs_store_load_toml_str(
    store: *mut StatefsStore,
    toml_cstr: *const c_char,
) -> i32 {
    let result = catch_unwind(|| {
        clear_last_error();
        if store.is_null() || toml_cstr.is_null() {
            set_last_error("Null argument passed to statefs_store_load_toml_str");
            return -1;
        }

        let toml_str = match unsafe { CStr::from_ptr(toml_cstr) }.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(&format!("Invalid UTF-8 in toml: {e}"));
                return -1;
            }
        };

        let store_ref = unsafe { &mut (*store).inner };
        if let Err(e) = ingest_toml(store_ref, toml_str) {
            set_last_error(&format!("TOML parse error: {e}"));
            return -1;
        }

        0
    });

    result.unwrap_or_else(|_| {
        set_last_error("Panic occurred in statefs_store_load_toml_str");
        -1
    })
}

/// Ingests system environment variables with given prefix and separator. Returns count on success, -1 on error.
///
/// # Safety
///
/// `store` must be a valid pointer to a `StatefsStore`. `prefix_cstr` and `sep_cstr` may be NULL or valid C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn statefs_store_load_env(
    store: *mut StatefsStore,
    prefix_cstr: *const c_char,
    sep_cstr: *const c_char,
) -> i32 {
    let result = catch_unwind(|| {
        clear_last_error();
        if store.is_null() {
            set_last_error("Null store pointer in statefs_store_load_env");
            return -1;
        }

        let prefix_opt = if !prefix_cstr.is_null() {
            match unsafe { CStr::from_ptr(prefix_cstr) }.to_str() {
                Ok(s) => Some(s),
                Err(e) => {
                    set_last_error(&format!("Invalid prefix UTF-8: {e}"));
                    return -1;
                }
            }
        } else {
            None
        };

        let sep = if !sep_cstr.is_null() {
            match unsafe { CStr::from_ptr(sep_cstr) }.to_str() {
                Ok(s) => s,
                Err(e) => {
                    set_last_error(&format!("Invalid separator UTF-8: {e}"));
                    return -1;
                }
            }
        } else {
            "__"
        };

        let store_ref = unsafe { &mut (*store).inner };
        let mut source = EnvSource::new().with_separator(sep);
        if let Some(p) = prefix_opt {
            source = source.with_prefix(p);
        }

        match source.ingest(store_ref) {
            Ok(count) => count as i32,
            Err(e) => {
                set_last_error(&format!("Env ingest error: {e}"));
                -1
            }
        }
    });

    result.unwrap_or_else(|_| {
        set_last_error("Panic occurred in statefs_store_load_env");
        -1
    })
}

/// Exports the store into a binary snapshot byte buffer.
///
/// Caller must free buffer with [`statefs_free_bytes`]. Returns 0 on success, -1 on error.
///
/// # Safety
///
/// `store` must be valid. `out_ptr` and `out_len` must be non-null writable pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn statefs_store_export_snapshot(
    store: *const StatefsStore,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    let result = catch_unwind(|| {
        clear_last_error();
        if store.is_null() || out_ptr.is_null() || out_len.is_null() {
            set_last_error("Null argument passed to statefs_store_export_snapshot");
            return -1;
        }

        let store_ref = unsafe { &(*store).inner };
        let bytes = export_snapshot_bytes(store_ref);
        let len = bytes.len();
        let mut boxed_slice = bytes.into_boxed_slice();
        let ptr = boxed_slice.as_mut_ptr();
        std::mem::forget(boxed_slice);

        unsafe {
            *out_ptr = ptr;
            *out_len = len;
        }

        0
    });

    result.unwrap_or_else(|_| {
        set_last_error("Panic occurred in statefs_store_export_snapshot");
        -1
    })
}

/// Restores a new StateFS store from a binary snapshot byte buffer.
///
/// Returns valid store pointer or NULL on error.
///
/// # Safety
///
/// `data` must point to at least `len` valid readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn statefs_store_restore_snapshot(
    data: *const u8,
    len: usize,
) -> *mut StatefsStore {
    let result = catch_unwind(|| {
        clear_last_error();
        if data.is_null() || len == 0 {
            set_last_error("Null or empty data buffer in statefs_store_restore_snapshot");
            return ptr::null_mut();
        }

        let slice = unsafe { slice::from_raw_parts(data, len) };
        match restore_snapshot_bytes(slice) {
            Ok(mem_store) => Box::into_raw(Box::new(StatefsStore { inner: mem_store })),
            Err(e) => {
                set_last_error(&format!("Snapshot restore error: {e}"));
                ptr::null_mut()
            }
        }
    });

    result.unwrap_or_else(|_| {
        set_last_error("Panic occurred in statefs_store_restore_snapshot");
        ptr::null_mut()
    })
}

/// Deallocates bytes returned by [`statefs_store_export_snapshot`]. Safe no-op on NULL.
///
/// # Safety
///
/// `ptr` must either be NULL or an allocation produced by [`statefs_store_export_snapshot`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn statefs_free_bytes(ptr: *mut u8, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    let _ = catch_unwind(|| {
        // SAFETY: Pointer and len originated from into_boxed_slice in export_snapshot.
        let slice = unsafe { slice::from_raw_parts_mut(ptr, len) };
        let boxed = unsafe { Box::from_raw(slice) };
        drop(boxed);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn test_c_abi_crud_roundtrip() {
        unsafe {
            let store = statefs_store_new();
            assert!(!store.is_null());

            // 1. Insert & Get string
            let path = CString::new("/server/motd").unwrap();
            let val = CString::new("Welcome to GoldSrc").unwrap();
            assert_eq!(
                statefs_store_insert_str(store, path.as_ptr(), val.as_ptr()),
                0
            );

            let mut buf = [0 as c_char; 64];
            let mut written = 0;
            assert_eq!(
                statefs_store_get_str(
                    store,
                    path.as_ptr(),
                    buf.as_mut_ptr(),
                    buf.len(),
                    &mut written
                ),
                0
            );
            let read_str = CStr::from_ptr(buf.as_ptr()).to_str().unwrap();
            assert_eq!(read_str, "Welcome to GoldSrc");

            // 2. Insert & Get int
            let int_path = CString::new("/server/max_players").unwrap();
            assert_eq!(statefs_store_insert_int(store, int_path.as_ptr(), 32), 0);
            let mut read_int = 0;
            assert_eq!(
                statefs_store_get_int(store, int_path.as_ptr(), &mut read_int),
                0
            );
            assert_eq!(read_int, 32);

            // 3. Insert & Get bool
            let bool_path = CString::new("/server/secure").unwrap();
            assert_eq!(statefs_store_insert_bool(store, bool_path.as_ptr(), 1), 0);
            let mut read_bool = 0;
            assert_eq!(
                statefs_store_get_bool(store, bool_path.as_ptr(), &mut read_bool),
                0
            );
            assert_eq!(read_bool, 1);

            // 4. Contains & Remove
            assert_eq!(statefs_store_contains(store, int_path.as_ptr()), 1);
            assert_eq!(statefs_store_remove(store, int_path.as_ptr()), 0);
            assert_eq!(statefs_store_contains(store, int_path.as_ptr()), 0);

            statefs_store_free(store);
        }
    }

    #[test]
    fn test_c_abi_json_toml_snapshot() {
        unsafe {
            let store = statefs_store_new();

            // Load JSON
            let json = CString::new(r#"{"game": {"name": "cstrike", "fps_max": 100}}"#).unwrap();
            assert_eq!(statefs_store_load_json_str(store, json.as_ptr()), 0);

            let mut fps = 0;
            let fps_path = CString::new("/game/fps_max").unwrap();
            assert_eq!(statefs_store_get_int(store, fps_path.as_ptr(), &mut fps), 0);
            assert_eq!(fps, 100);

            // Export & Restore snapshot
            let mut snap_ptr = ptr::null_mut();
            let mut snap_len = 0;
            assert_eq!(
                statefs_store_export_snapshot(store, &mut snap_ptr, &mut snap_len),
                0
            );
            assert!(!snap_ptr.is_null());
            assert!(snap_len > 0);

            let restored = statefs_store_restore_snapshot(snap_ptr, snap_len);
            assert!(!restored.is_null());

            let mut restored_fps = 0;
            assert_eq!(
                statefs_store_get_int(restored, fps_path.as_ptr(), &mut restored_fps),
                0
            );
            assert_eq!(restored_fps, 100);

            statefs_free_bytes(snap_ptr, snap_len);
            statefs_store_free(restored);
            statefs_store_free(store);
        }
    }

    #[test]
    fn test_c_abi_null_safety_and_errors() {
        unsafe {
            // Null store
            let path = CString::new("/test").unwrap();
            assert_eq!(
                statefs_store_insert_int(ptr::null_mut(), path.as_ptr(), 1),
                -1
            );

            let mut err_buf = [0 as c_char; 128];
            let err_len = statefs_last_error(err_buf.as_mut_ptr(), err_buf.len());
            assert!(err_len > 0);
            let err_msg = CStr::from_ptr(err_buf.as_ptr()).to_str().unwrap();
            assert!(err_msg.contains("Null pointer"));

            // Null free is safe no-op
            statefs_store_free(ptr::null_mut());
            statefs_free_bytes(ptr::null_mut(), 0);
        }
    }
}
