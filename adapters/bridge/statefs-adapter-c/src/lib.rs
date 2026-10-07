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
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::catch_unwind;
use std::ptr;
use std::slice;
use std::sync::{LazyLock, Mutex};

use statefs_adapter_bridge_env::EnvSource;
use statefs_codec_bin::{export_snapshot_bytes, restore_snapshot_bytes};
use statefs_codec_json::ingest_json;
use statefs_codec_toml::ingest_toml;
use statefs_core::{MemStore, Path, Store, Value};

/// Active store pointers to prevent double-free and use-after-free corruption across C ABI.
static ACTIVE_STORES: LazyLock<Mutex<HashSet<usize>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Active snapshot byte allocations with their recorded size to ensure sound deallocation.
static ACTIVE_BUFFERS: LazyLock<Mutex<HashMap<usize, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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
        if let Ok(mut borrow) = cell.try_borrow_mut() {
            *borrow = CString::new(clean).ok();
        }
    });
}

fn clear_last_error() {
    LAST_ERROR.with(|cell| {
        if let Ok(mut borrow) = cell.try_borrow_mut() {
            *borrow = None;
        }
    });
}

/// Retrieves the most recent error message on the calling thread.
///
/// Writes at most `max_len - 1` characters plus null terminator into `buf`.
/// Returns the number of characters written into `buf` (excluding null terminator), or 0 if no error.
///
/// # Safety
///
/// Caller must pass a valid, non-null pointer `buf` pointing to at least `max_len` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn statefs_last_error(buf: *mut c_char, max_len: usize) -> usize {
    let result = catch_unwind(|| {
        if buf.is_null() || max_len == 0 {
            return 0;
        }

        LAST_ERROR.with(|cell| {
            if let Ok(borrow) = cell.try_borrow() {
                if let Some(ref err) = *borrow {
                    let bytes = err.as_bytes();
                    let to_copy = bytes.len().min(max_len.saturating_sub(1));
                    // SAFETY: Caller guarantees buf is valid for max_len bytes.
                    unsafe {
                        ptr::copy_nonoverlapping(bytes.as_ptr() as *const c_char, buf, to_copy);
                        *buf.add(to_copy) = 0;
                    }
                    to_copy
                } else {
                    // SAFETY: Caller guarantees buf has at least 1 byte capacity.
                    unsafe {
                        *buf = 0;
                    }
                    0
                }
            } else {
                unsafe {
                    *buf = 0;
                }
                0
            }
        })
    });

    result.unwrap_or(0)
}

/// Allocates and initializes a new empty StateFS store.
///
/// Must be freed using [`statefs_store_free`].
#[unsafe(no_mangle)]
pub extern "C" fn statefs_store_new() -> *mut StatefsStore {
    let result = catch_unwind(|| {
        clear_last_error();
        let ptr = Box::into_raw(Box::new(StatefsStore {
            inner: MemStore::new(),
        }));
        let mut set = match ACTIVE_STORES.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        set.insert(ptr as usize);
        ptr
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
        let was_active = {
            let mut set = match ACTIVE_STORES.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            set.remove(&(store as usize))
        };

        if !was_active {
            set_last_error("Attempted double-free or free of untracked StatefsStore pointer");
            return;
        }

        // SAFETY: Pointer was verified non-null, active in registry, and allocated by Box::into_raw.
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
/// If `buf` is NULL and `buf_len` is 0, writes required buffer capacity (including null terminator)
/// into `written` and returns 0 (size probe).
/// If `buf_len` is smaller than required capacity, writes required capacity into `written`, sets last error, and returns -1.
/// Returns 0 on success, 1 if path not found, -1 on error/type mismatch.
///
/// # Safety
///
/// `store` and `path_cstr` must be valid non-null pointers. If `buf` is non-null, it must point to at least `buf_len` writable bytes.
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
        if store.is_null() || path_cstr.is_null() {
            set_last_error("Null argument passed to statefs_store_get_str");
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
                    let required_len = bytes.len() + 1;

                    // Size query pattern: buf is null and buf_len == 0
                    if buf.is_null() && buf_len == 0 {
                        if !written.is_null() {
                            unsafe { *written = required_len };
                        }
                        return 0;
                    }

                    if buf.is_null() {
                        set_last_error("Null buffer passed to statefs_store_get_str");
                        return -1;
                    }

                    if buf_len < required_len {
                        if !written.is_null() {
                            unsafe { *written = required_len };
                        }
                        set_last_error(&format!(
                            "Buffer too small: buffer length {buf_len} is less than required {required_len}"
                        ));
                        return -1;
                    }

                    unsafe {
                        ptr::copy_nonoverlapping(bytes.as_ptr() as *const c_char, buf, bytes.len());
                        *buf.add(bytes.len()) = 0;
                        if !written.is_null() {
                            *written = bytes.len();
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

        {
            let mut map = match ACTIVE_BUFFERS.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            map.insert(ptr as usize, len);
        }

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
            Ok(mem_store) => {
                let ptr = Box::into_raw(Box::new(StatefsStore { inner: mem_store }));
                let mut set = match ACTIVE_STORES.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                set.insert(ptr as usize);
                ptr
            }
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
/// Guards against double-free and mismatched lengths by consulting active allocation registry.
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
        let tracked_len = {
            let mut map = match ACTIVE_BUFFERS.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            map.remove(&(ptr as usize))
        };

        let Some(actual_len) = tracked_len else {
            set_last_error("Attempted double-free or free of untracked snapshot buffer");
            return;
        };

        // SAFETY: Pointer and actual_len originated from into_boxed_slice in export_snapshot.
        let slice = unsafe { slice::from_raw_parts_mut(ptr, actual_len) };
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

    #[test]
    fn test_c_abi_double_free_protection() {
        unsafe {
            let store = statefs_store_new();
            assert!(!store.is_null());

            // First free must succeed
            statefs_store_free(store);

            // Second free (double free) must be safely intercepted without crash
            statefs_store_free(store);

            let mut err_buf = [0 as c_char; 128];
            let err_len = statefs_last_error(err_buf.as_mut_ptr(), err_buf.len());
            assert!(err_len > 0);
            let err_msg = CStr::from_ptr(err_buf.as_ptr()).to_str().unwrap();
            assert!(err_msg.contains("double-free"));

            // Snapshot buffer double free protection
            let s2 = statefs_store_new();
            let mut snap_ptr = ptr::null_mut();
            let mut snap_len = 0;
            assert_eq!(
                statefs_store_export_snapshot(s2, &mut snap_ptr, &mut snap_len),
                0
            );
            assert!(!snap_ptr.is_null());

            // First free must succeed
            statefs_free_bytes(snap_ptr, snap_len);

            // Second free must be intercepted
            statefs_free_bytes(snap_ptr, snap_len);
            let err_len = statefs_last_error(err_buf.as_mut_ptr(), err_buf.len());
            assert!(err_len > 0);
            let err_msg = CStr::from_ptr(err_buf.as_ptr()).to_str().unwrap();
            assert!(err_msg.contains("double-free"));

            statefs_store_free(s2);
        }
    }

    #[test]
    fn test_c_abi_buffer_overflow_and_size_probe() {
        unsafe {
            let store = statefs_store_new();
            let path = CString::new("/security/token").unwrap();
            let secret = CString::new("SuperSecretToken123456").unwrap(); // 22 bytes
            assert_eq!(
                statefs_store_insert_str(store, path.as_ptr(), secret.as_ptr()),
                0
            );

            // 1. Capacity probe with NULL buffer and 0 len
            let mut required = 0;
            assert_eq!(
                statefs_store_get_str(store, path.as_ptr(), ptr::null_mut(), 0, &mut required),
                0
            );
            assert_eq!(required, 23); // 22 chars + 1 null terminator

            // 2. Buffer too small: 10 bytes provided for 23 required
            let mut tiny_buf = [0x55 as c_char; 10];
            let mut needed = 0;
            assert_eq!(
                statefs_store_get_str(
                    store,
                    path.as_ptr(),
                    tiny_buf.as_mut_ptr(),
                    tiny_buf.len(),
                    &mut needed
                ),
                -1
            );
            assert_eq!(needed, 23);
            // Verify no partial corrupt data written to tiny buffer
            assert_eq!(tiny_buf[0], 0x55);

            // 3. Exact buffer: 23 bytes provided
            let mut exact_buf = [0 as c_char; 23];
            let mut written = 0;
            assert_eq!(
                statefs_store_get_str(
                    store,
                    path.as_ptr(),
                    exact_buf.as_mut_ptr(),
                    exact_buf.len(),
                    &mut written
                ),
                0
            );
            assert_eq!(written, 22);
            let read_val = CStr::from_ptr(exact_buf.as_ptr()).to_str().unwrap();
            assert_eq!(read_val, "SuperSecretToken123456");

            // 4. statefs_last_error truncation safety
            set_last_error("A long error message that exceeds buffer");
            let mut tiny_err = [0x77 as c_char; 5];
            let copied = statefs_last_error(tiny_err.as_mut_ptr(), tiny_err.len());
            assert_eq!(copied, 4);
            assert_eq!(tiny_err[4], 0); // Must be null-terminated
            let err_read = CStr::from_ptr(tiny_err.as_ptr()).to_str().unwrap();
            assert_eq!(err_read, "A lo");

            // 5. statefs_last_error 1-byte buffer (only null terminator)
            let mut one_byte = [0x77 as c_char; 1];
            let copied_one = statefs_last_error(one_byte.as_mut_ptr(), 1);
            assert_eq!(copied_one, 0);
            assert_eq!(one_byte[0], 0);

            statefs_store_free(store);
        }
    }

    #[test]
    fn test_c_abi_non_utf8_and_empty_inputs() {
        unsafe {
            let store = statefs_store_new();

            // Non-UTF8 path (raw bytes 0xFF, 0xFE)
            let bad_path = [0xFFu8 as i8 as c_char, 0xFEu8 as i8 as c_char, 0];
            let valid_val = CString::new("val").unwrap();
            assert_eq!(
                statefs_store_insert_str(store, bad_path.as_ptr(), valid_val.as_ptr()),
                -1
            );

            let mut err_buf = [0 as c_char; 128];
            statefs_last_error(err_buf.as_mut_ptr(), err_buf.len());
            let err_msg = CStr::from_ptr(err_buf.as_ptr()).to_str().unwrap();
            assert!(err_msg.contains("Invalid UTF-8"));

            // Non-UTF8 value
            let valid_path = CString::new("/key").unwrap();
            let bad_val = [0xC0u8 as i8 as c_char, 0xAFu8 as i8 as c_char, 0];
            assert_eq!(
                statefs_store_insert_str(store, valid_path.as_ptr(), bad_val.as_ptr()),
                -1
            );

            // Empty path and empty string values
            let empty_str = CString::new("").unwrap();
            assert_eq!(
                statefs_store_insert_str(store, empty_str.as_ptr(), empty_str.as_ptr()),
                0
            );

            let mut out_buf = [0 as c_char; 16];
            let mut out_len = 0;
            assert_eq!(
                statefs_store_get_str(
                    store,
                    empty_str.as_ptr(),
                    out_buf.as_mut_ptr(),
                    out_buf.len(),
                    &mut out_len
                ),
                0
            );
            assert_eq!(out_len, 0);
            assert_eq!(CStr::from_ptr(out_buf.as_ptr()).to_str().unwrap(), "");

            statefs_store_free(store);
        }
    }
}
