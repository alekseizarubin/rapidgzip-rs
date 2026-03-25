//! Raw FFI bindings for the native `rapidgzip` C ABI.
//!
//! Most users should depend on the safe `rapidgzip` crate instead of calling
//! this layer directly.

use libc::{c_char, c_int, c_void, size_t};

/// Status code returned by the native `rapidgzip` C ABI.
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum rgz_status_t {
    RGZ_STATUS_OK = 0,
    RGZ_STATUS_EOF = 1,
    RGZ_STATUS_INVALID_ARGUMENT = 10,
    RGZ_STATUS_IO_ERROR = 11,
    RGZ_STATUS_DATA_ERROR = 12,
    RGZ_STATUS_UNSUPPORTED = 13,
    RGZ_STATUS_OUT_OF_MEMORY = 14,
    RGZ_STATUS_INTERNAL_ERROR = 15,
    RGZ_STATUS_SEEK_ERROR = 16,
    RGZ_STATUS_INDEX_ERROR = 17,
    RGZ_STATUS_STATE_ERROR = 18,
    RGZ_STATUS_NOT_IMPLEMENTED = 19,
}

/// Native I/O mode flags stored in `rgz_config_t.flags`.
///
/// These values expose the upstream reader strategies without expanding the
/// ABI struct layout. `AUTO` keeps the native default behavior.
pub const RGZ_IO_READ_MODE_MASK: u32 = 0b11;
pub const RGZ_IO_READ_MODE_AUTO: u32 = 0;
pub const RGZ_IO_READ_MODE_SEQUENTIAL: u32 = 1;
pub const RGZ_IO_READ_MODE_PREAD: u32 = 2;
pub const RGZ_IO_READ_MODE_LOCKED_READ_AND_SEEK: u32 = 3;

/// Reader configuration passed across the FFI boundary.
#[repr(C)]
pub struct rgz_config_t {
    pub struct_size: u32,
    pub flags: u32,
    pub parallelism: u32,
    pub reserved0: u32,
    pub chunk_size: u64,
    pub reserved1: u64,
}

/// Callback table used for custom callback-backed inputs.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct rgz_callbacks_t {
    pub read: Option<
        unsafe extern "C" fn(user_data: *mut c_void, dst: *mut c_char, len: size_t) -> size_t,
    >,
    pub seek:
        Option<unsafe extern "C" fn(user_data: *mut c_void, offset: i64, origin: c_int) -> u64>,
    pub get_size: Option<unsafe extern "C" fn(user_data: *mut c_void) -> u64>,
    pub clone: Option<unsafe extern "C" fn(user_data: *mut c_void) -> *mut c_void>,
    pub free_user_data: Option<unsafe extern "C" fn(user_data: *mut c_void)>,
}

/// Opaque native reader handle.
#[repr(C)]
pub struct rgz_reader_t {
    _unused: [u8; 0],
}

extern "C" {
    pub fn rgz_abi_version() -> u32;
    pub fn rgz_config_init(config: *mut rgz_config_t);

    pub fn rgz_open_path_ex(
        path: *const c_char,
        config: *const rgz_config_t,
        status_out: *mut rgz_status_t,
    ) -> *mut rgz_reader_t;

    pub fn rgz_open_callbacks(
        callbacks: rgz_callbacks_t,
        user_data: *mut c_void,
        config: *const rgz_config_t,
        status_out: *mut rgz_status_t,
    ) -> *mut rgz_reader_t;

    pub fn rgz_close(reader: *mut rgz_reader_t);

    pub fn rgz_read(
        reader: *mut rgz_reader_t,
        dst: *mut c_void,
        len: size_t,
        read_out: *mut size_t,
    ) -> rgz_status_t;

    pub fn rgz_read_discard(
        reader: *mut rgz_reader_t,
        len: size_t,
        read_out: *mut size_t,
    ) -> rgz_status_t;

    pub fn rgz_read_to_fd(
        reader: *mut rgz_reader_t,
        output_fd: c_int,
        len: size_t,
        read_out: *mut size_t,
    ) -> rgz_status_t;

    pub fn rgz_set_keep_index(reader: *mut rgz_reader_t, keep_index: bool) -> rgz_status_t;

    pub fn rgz_seek_to(reader: *mut rgz_reader_t, uncompressed_offset: u64) -> rgz_status_t;

    pub fn rgz_tell(reader: *mut rgz_reader_t, offset_out: *mut u64) -> rgz_status_t;

    pub fn rgz_import_index_path(reader: *mut rgz_reader_t, path: *const c_char) -> rgz_status_t;

    pub fn rgz_export_index_path(reader: *mut rgz_reader_t, path: *const c_char) -> rgz_status_t;

    pub fn rgz_import_index_callbacks(
        reader: *mut rgz_reader_t,
        callbacks: rgz_callbacks_t,
        user_data: *mut c_void,
    ) -> rgz_status_t;

    pub fn rgz_export_index_write(
        reader: *mut rgz_reader_t,
        write_cb: unsafe extern "C" fn(
            user_data: *mut c_void,
            buffer: *const c_void,
            size: size_t,
        ) -> size_t,
        user_data: *mut c_void,
    ) -> rgz_status_t;

    pub fn rgz_last_error(reader: *mut rgz_reader_t) -> *const c_char;
    pub fn rgz_last_global_error() -> *const c_char;
    pub fn rgz_last_status(reader: *mut rgz_reader_t) -> rgz_status_t;
}
