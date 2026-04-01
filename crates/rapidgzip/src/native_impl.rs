use std::ffi::{CStr, CString};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::raw::{c_char, c_int, c_void};
use std::path::Path;

#[cfg(unix)]
use std::os::fd::RawFd;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

use rapidgzip_sys as ffi;

/// Error returned by the safe `rapidgzip` API.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(String),
    #[error("Gzip data error: {0}")]
    Data(String),
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
    #[error("Operation not supported: {0}")]
    Unsupported(String),
    #[error("Seek error: {0}")]
    Seek(String),
    #[error("Index error: {0}")]
    Index(String),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("EOF reached")]
    Eof,
    #[error("Unknown error with status {0:?}")]
    Unknown(ffi::rgz_status_t),
}

fn default_status_message(status: ffi::rgz_status_t) -> &'static str {
    match status {
        ffi::rgz_status_t::RGZ_STATUS_OK => "Success status inappropriately converted to error",
        ffi::rgz_status_t::RGZ_STATUS_EOF => "EOF reached",
        ffi::rgz_status_t::RGZ_STATUS_INVALID_ARGUMENT => "Invalid argument",
        ffi::rgz_status_t::RGZ_STATUS_IO_ERROR => "I/O error",
        ffi::rgz_status_t::RGZ_STATUS_DATA_ERROR => "Gzip data error",
        ffi::rgz_status_t::RGZ_STATUS_UNSUPPORTED => "Unsupported operation",
        ffi::rgz_status_t::RGZ_STATUS_OUT_OF_MEMORY => "Out of memory",
        ffi::rgz_status_t::RGZ_STATUS_INTERNAL_ERROR => "Internal error",
        ffi::rgz_status_t::RGZ_STATUS_SEEK_ERROR => "Seek error",
        ffi::rgz_status_t::RGZ_STATUS_INDEX_ERROR => "Index error",
        ffi::rgz_status_t::RGZ_STATUS_STATE_ERROR => "Invalid reader state",
        ffi::rgz_status_t::RGZ_STATUS_NOT_IMPLEMENTED => "Not implemented",
    }
}

impl Error {
    fn from_status_message(status: ffi::rgz_status_t, message: Option<String>) -> Self {
        let message = message
            .filter(|message| !message.trim().is_empty())
            .unwrap_or_else(|| default_status_message(status).to_string());

        match status {
            ffi::rgz_status_t::RGZ_STATUS_OK => Error::Internal(message),
            ffi::rgz_status_t::RGZ_STATUS_EOF => Error::Eof,
            ffi::rgz_status_t::RGZ_STATUS_INVALID_ARGUMENT => Error::InvalidArgument(message),
            ffi::rgz_status_t::RGZ_STATUS_IO_ERROR => Error::Io(message),
            ffi::rgz_status_t::RGZ_STATUS_DATA_ERROR => Error::Data(message),
            ffi::rgz_status_t::RGZ_STATUS_UNSUPPORTED => Error::Unsupported(message),
            ffi::rgz_status_t::RGZ_STATUS_OUT_OF_MEMORY => Error::Internal(message),
            ffi::rgz_status_t::RGZ_STATUS_INTERNAL_ERROR => Error::Internal(message),
            ffi::rgz_status_t::RGZ_STATUS_SEEK_ERROR => Error::Seek(message),
            ffi::rgz_status_t::RGZ_STATUS_INDEX_ERROR => Error::Index(message),
            ffi::rgz_status_t::RGZ_STATUS_STATE_ERROR => Error::Internal(message),
            ffi::rgz_status_t::RGZ_STATUS_NOT_IMPLEMENTED => Error::Unsupported(message),
        }
    }
}

impl From<ffi::rgz_status_t> for Error {
    fn from(status: ffi::rgz_status_t) -> Self {
        Error::from_status_message(status, None)
    }
}

/// Trait object bound for callback-backed readers that implement `Read` and `Seek`.
///
/// Only `Send` is required (not `Sync`) because the C++ backend calls callbacks
/// sequentially — never from multiple threads simultaneously on the same handle.
/// This means types like `BufReader<File>` (which is `Send` but not `Sync`) can
/// be passed directly without wrapping.
pub trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

/// Callback-backed reader that can be cloned into independent handles for parallel decoding.
///
/// `clone_box` is called by the C++ backend to create per-worker reader handles.
/// Each clone is used independently; the C++ layer never shares a single clone
/// between threads.
///
/// **Position semantics:** clones may start at the same position as the original
/// or at position 0 — the exact behaviour depends on the implementation. The
/// C++ backend always seeks each worker to its target offset before reading, so
/// the starting position of a fresh clone does not affect correctness.
pub trait CloneableReadSeek: ReadSeek + Send {
    fn clone_box(&self) -> Box<dyn CloneableReadSeek>;
}

impl<T: Read + Seek + Clone + Send + 'static> CloneableReadSeek for T {
    fn clone_box(&self) -> Box<dyn CloneableReadSeek> {
        Box::new(self.clone())
    }
}

/// Selects how the native backend should access the compressed input.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum IoReadMode {
    /// Keep the native default behavior for the current input type.
    Auto,
    /// Force sequential buffered reads. Useful when avoiding random disk seeks matters more than peak latency.
    Sequential,
    /// Force positioned reads (`pread`) on seekable files. Useful for SSD-friendly random access and parallel workers.
    Pread,
    /// Force a shared reader that serializes read/seek operations without `pread`. Useful as a conservative fallback.
    LockedReadAndSeek,
}

impl IoReadMode {
    fn to_ffi_flags(self) -> u32 {
        match self {
            IoReadMode::Auto => ffi::RGZ_IO_READ_MODE_AUTO,
            IoReadMode::Sequential => ffi::RGZ_IO_READ_MODE_SEQUENTIAL,
            IoReadMode::Pread => ffi::RGZ_IO_READ_MODE_PREAD,
            IoReadMode::LockedReadAndSeek => ffi::RGZ_IO_READ_MODE_LOCKED_READ_AND_SEEK,
        }
    }
}

/// Builder for configuring a native `Reader` before opening an input.
pub struct ReaderBuilder {
    parallelism: u32,
    chunk_size: u64,
    keep_index: Option<bool>,
    io_read_mode: IoReadMode,
}

impl Default for ReaderBuilder {
    fn default() -> Self {
        Self {
            parallelism: 0, // 0 means auto in rapidgzip
            chunk_size: 4 * 1024 * 1024,
            keep_index: None,
            io_read_mode: IoReadMode::Auto,
        }
    }
}

#[cfg(any(test, not(unix)))]
#[derive(Debug, Clone)]
struct CloneFailureReader {
    message: std::sync::Arc<str>,
}

#[cfg(any(test, not(unix)))]
impl CloneFailureReader {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: std::sync::Arc::from(message.into()),
        }
    }

    fn io_error(&self) -> io::Error {
        io::Error::other(self.message.to_string())
    }
}

#[cfg(any(test, not(unix)))]
impl Read for CloneFailureReader {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(self.io_error())
    }
}

#[cfg(any(test, not(unix)))]
impl Seek for CloneFailureReader {
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Err(self.io_error())
    }
}

#[cfg(any(test, not(unix)))]
#[derive(Debug)]
struct FileWrapper {
    file: std::fs::File,
    reopen_path: std::path::PathBuf,
}

#[cfg(any(test, not(unix)))]
impl Read for FileWrapper {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

#[cfg(any(test, not(unix)))]
impl Seek for FileWrapper {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

#[cfg(any(test, not(unix)))]
impl CloneableReadSeek for FileWrapper {
    fn clone_box(&self) -> Box<dyn CloneableReadSeek> {
        match std::fs::File::open(&self.reopen_path) {
            Ok(reopened_file) => Box::new(FileWrapper {
                file: reopened_file,
                reopen_path: self.reopen_path.clone(),
            }),
            Err(error) => Box::new(CloneFailureReader::new(format!(
                "Failed to reopen file handle for cloneable reader '{}': {}",
                self.reopen_path.display(),
                error,
            ))),
        }
    }
}

fn last_global_error() -> Option<String> {
    unsafe {
        let c_str = ffi::rgz_last_global_error();
        if c_str.is_null() {
            return None;
        }
        let message = CStr::from_ptr(c_str).to_string_lossy().into_owned();
        if message.trim().is_empty() {
            None
        } else {
            Some(message)
        }
    }
}

impl ReaderBuilder {
    /// Creates a builder with rapidgzip defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the requested parallelism. `0` keeps the native auto policy.
    pub fn parallelism(mut self, n: u32) -> Self {
        self.parallelism = n;
        self
    }

    /// Sets the native chunk size used for parallel decoding.
    pub fn chunk_size(mut self, size: u64) -> Self {
        self.chunk_size = size;
        self
    }

    /// Controls whether the native reader retains its in-memory index after use.
    pub fn keep_index(mut self, keep_index: bool) -> Self {
        self.keep_index = Some(keep_index);
        self
    }

    /// Selects the native I/O path for the compressed input.
    ///
    /// `Sequential` is the most relevant mode for spinning disks or pipe-like
    /// sources because it favors linear reads and buffering over random access.
    pub fn io_read_mode(mut self, mode: IoReadMode) -> Self {
        self.io_read_mode = mode;
        self
    }

    fn to_ffi_config(&self) -> ffi::rgz_config_t {
        ffi::rgz_config_t {
            struct_size: std::mem::size_of::<ffi::rgz_config_t>() as u32,
            flags: self.io_read_mode.to_ffi_flags() & ffi::RGZ_IO_READ_MODE_MASK,
            parallelism: self.parallelism,
            reserved0: 0,
            chunk_size: self.chunk_size,
            reserved1: 0,
        }
    }

    fn apply_reader_config(&self, reader: &mut Reader) -> Result<(), Error> {
        if let Some(keep_index) = self.keep_index {
            let status = unsafe { ffi::rgz_set_keep_index(reader.inner, keep_index) };
            if status != ffi::rgz_status_t::RGZ_STATUS_OK {
                return Err(Error::from_status_message(
                    status,
                    Some(reader.get_last_error()),
                ));
            }
        }

        Ok(())
    }

    /// Opens a gzip or BGZF file from a filesystem path.
    pub fn open<P: AsRef<Path>>(&self, path: P) -> Result<Reader, Error> {
        let path = path.as_ref();

        #[cfg(unix)]
        {
            self.open_path_direct(path)
        }

        #[cfg(not(unix))]
        {
            let file = std::fs::File::open(path).map_err(|e| Error::Io(e.to_string()))?;
            self.open_cloneable_reader(FileWrapper {
                file,
                reopen_path: path.to_path_buf(),
            })
        }
    }

    #[cfg(unix)]
    fn open_path_direct(&self, path: &Path) -> Result<Reader, Error> {
        let path_cstring = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            Error::InvalidArgument(format!(
                "Path contains an interior NUL byte: {}",
                path.display()
            ))
        })?;

        let config = self.to_ffi_config();
        let mut status = ffi::rgz_status_t::RGZ_STATUS_OK;
        let reader_ptr =
            unsafe { ffi::rgz_open_path_ex(path_cstring.as_ptr(), &config, &mut status) };

        if reader_ptr.is_null() {
            return Err(Error::from_status_message(status, last_global_error()));
        }

        let mut reader = Reader { inner: reader_ptr };
        self.apply_reader_config(&mut reader)?;
        Ok(reader)
    }

    /// Opens a generic `Read + Seek` source.
    ///
    /// Because generic readers cannot be cloned into independent handles, the
    /// native backend forces parallelism to `1` for this path.
    pub fn open_reader<R: Read + Seek + Send + 'static>(&self, reader: R) -> Result<Reader, Error> {
        let boxed_reader: Box<dyn ReadSeek> = Box::new(reader);
        let user_data = Box::into_raw(Box::new(boxed_reader)) as *mut c_void;

        let callbacks = ffi::rgz_callbacks_t {
            read: Some(cb_read),
            seek: Some(cb_seek),
            get_size: Some(cb_get_size),
            clone: None, // Standard readers are not cloneable
            free_user_data: Some(cb_free_user_data),
        };

        self.open_with_callbacks(callbacks, user_data)
    }

    /// Opens a cloneable `Read + Seek` source.
    ///
    /// Use this path when callback-backed readers should support parallel
    /// decompression through independent clones.
    pub fn open_cloneable_reader<R: CloneableReadSeek + Send + 'static>(
        &self,
        reader: R,
    ) -> Result<Reader, Error> {
        let boxed_reader: Box<dyn CloneableReadSeek> = Box::new(reader);
        let user_data = Box::into_raw(Box::new(boxed_reader)) as *mut c_void;

        let callbacks = ffi::rgz_callbacks_t {
            read: Some(cb_read_cloneable),
            seek: Some(cb_seek_cloneable),
            get_size: Some(cb_get_size_cloneable),
            clone: Some(cb_clone_cloneable),
            free_user_data: Some(cb_free_user_data_cloneable),
        };

        self.open_with_callbacks(callbacks, user_data)
    }

    fn open_with_callbacks(
        &self,
        callbacks: ffi::rgz_callbacks_t,
        user_data: *mut c_void,
    ) -> Result<Reader, Error> {
        let config = self.to_ffi_config();
        let mut status = ffi::rgz_status_t::RGZ_STATUS_OK;
        let reader_ptr =
            unsafe { ffi::rgz_open_callbacks(callbacks, user_data, &config, &mut status) };

        if reader_ptr.is_null() {
            return Err(Error::from_status_message(status, last_global_error()));
        }

        let mut reader = Reader { inner: reader_ptr };
        self.apply_reader_config(&mut reader)?;
        Ok(reader)
    }
}

/// Opaque decompression reader backed by the native `rapidgzip` engine.
pub struct Reader {
    inner: *mut ffi::rgz_reader_t,
}

// Safety: `Reader` has unique ownership over the native handle. Moving that
// handle to another thread is safe, but shared concurrent access is still
// prevented because `Reader` does not implement `Sync`.
unsafe impl Send for Reader {}

impl Reader {
    /// Opens a gzip or BGZF file from a filesystem path using default settings.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        ReaderBuilder::new().open(path)
    }

    /// Opens a generic `Read + Seek` source with default settings.
    ///
    /// Because generic readers cannot be cloned into independent handles, the
    /// native backend forces parallelism to `1` for this path.
    pub fn open_reader<R: Read + Seek + Send + 'static>(reader: R) -> Result<Self, Error> {
        ReaderBuilder::new().open_reader(reader)
    }

    /// Exports the current random-access index to a file.
    ///
    /// Writes into a temporary file in the same directory first, then atomically
    /// renames it into place via `rename(2)`. This ensures the output file is
    /// never left in a partially written state if an error occurs mid-export.
    ///
    /// If `path` already exists, its permissions are preserved on Unix. If `path`
    /// is a symlink, the resolved target is updated (not the symlink itself).
    /// Broken symlinks, symlink loops, and other resolution failures are returned
    /// as errors rather than silently falling back to overwriting the symlink entry.
    ///
    /// **Hard-link note:** because this function uses `rename(2)` internally, the
    /// exported file receives a new inode. Any other hard links that pointed at the
    /// previous inode will keep the old index content. This is a known trade-off of
    /// atomic rename; in-place truncation is not used because it can leave a
    /// partially written file on error.
    pub fn export_index<P: AsRef<Path>>(&mut self, path: P) -> Result<(), Error> {
        use std::io::Write;

        // Resolve symlinks so we write to the real target and preserve its metadata.
        //
        // We use symlink_metadata() (which does NOT follow symlinks) to distinguish
        // between "path does not exist at all" (new file) and "path exists but
        // canonicalize fails" (broken symlink, loop, bad permissions, etc.).
        // In the latter case we propagate the error rather than silently writing
        // to the symlink entry itself.
        let path = path.as_ref();
        let real_path = if path.symlink_metadata().is_ok() {
            // Path exists (may be a symlink) — must resolve fully.
            path.canonicalize().map_err(|e| Error::Io(e.to_string()))?
        } else {
            // Path does not exist yet — write to this location directly.
            path.to_path_buf()
        };
        let parent = real_path.parent().unwrap_or_else(|| Path::new("."));

        // Snapshot existing permissions before we replace the file.
        #[cfg(unix)]
        let existing_mode: Option<u32> = {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(&real_path)
                .ok()
                .map(|m| m.permissions().mode())
        };

        let mut temp_file = tempfile::Builder::new()
            .prefix(".rapidgzip-index.")
            .suffix(".tmp")
            .tempfile_in(parent)
            .map_err(|e| Error::Io(e.to_string()))?;

        unsafe extern "C" fn write_cb(
            user_data: *mut c_void,
            buffer: *const c_void,
            size: usize,
        ) -> usize {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if size == 0 {
                    return 0;
                }
                if buffer.is_null() {
                    return 0;
                }
                let file = &mut *(user_data as *mut std::fs::File);
                let slice = std::slice::from_raw_parts(buffer as *const u8, size);
                if file.write_all(slice).is_ok() {
                    size
                } else {
                    0
                }
            }))
            .unwrap_or(0)
        }

        let user_data = temp_file.as_file_mut() as *mut std::fs::File as *mut c_void;
        let status = unsafe { ffi::rgz_export_index_write(self.inner, write_cb, user_data) };

        if status != ffi::rgz_status_t::RGZ_STATUS_OK {
            return Err(Error::from_status_message(
                status,
                Some(self.get_last_error()),
            ));
        }

        // Restore permissions from the existing file before renaming into place.
        #[cfg(unix)]
        if let Some(mode) = existing_mode {
            use std::os::unix::fs::PermissionsExt;
            let _ =
                std::fs::set_permissions(temp_file.path(), std::fs::Permissions::from_mode(mode));
        }

        temp_file
            .persist(&real_path)
            .map_err(|e| Error::Io(e.to_string()))?;

        Ok(())
    }

    /// Imports a previously exported random-access index from an arbitrary reader.
    ///
    /// The reader must implement [`ReadSeek`] so the C++ library can seek
    /// within the index during import. Use this when the index comes from a
    /// non-file source (e.g. an HTTP range reader).
    pub fn import_index_reader(&mut self, reader: Box<dyn ReadSeek>) -> Result<(), Error> {
        let user_data = Box::into_raw(Box::new(reader)) as *mut c_void;

        let callbacks = ffi::rgz_callbacks_t {
            read: Some(cb_read),
            seek: Some(cb_seek),
            get_size: Some(cb_get_size),
            clone: None,
            free_user_data: Some(cb_free_user_data),
        };

        let status = unsafe { ffi::rgz_import_index_callbacks(self.inner, callbacks, user_data) };
        if status != ffi::rgz_status_t::RGZ_STATUS_OK {
            return Err(Error::from_status_message(
                status,
                Some(self.get_last_error()),
            ));
        }
        Ok(())
    }

    /// Imports a previously exported random-access index from a file.
    pub fn import_index<P: AsRef<Path>>(&mut self, path: P) -> Result<(), Error> {
        let file = std::fs::File::open(path).map_err(|e| Error::Io(e.to_string()))?;
        self.import_index_reader(Box::new(file))
    }

    fn get_last_error(&self) -> String {
        unsafe {
            let c_str = ffi::rgz_last_error(self.inner);
            if c_str.is_null() {
                return "Unknown error".into();
            }
            CStr::from_ptr(c_str).to_string_lossy().into_owned()
        }
    }
}

// --- Callback implementations for Box<dyn ReadSeek> ---

unsafe extern "C" fn cb_read(user_data: *mut c_void, dst: *mut c_char, len: usize) -> usize {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if len == 0 {
            return 0;
        }
        if dst.is_null() {
            return usize::MAX;
        }
        let reader = &mut *(user_data as *mut Box<dyn ReadSeek>);
        let buf = std::slice::from_raw_parts_mut(dst as *mut u8, len);
        reader.read(buf).unwrap_or(usize::MAX)
    }))
    .unwrap_or(usize::MAX)
}

unsafe extern "C" fn cb_seek(user_data: *mut c_void, offset: i64, origin: c_int) -> u64 {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let reader = &mut *(user_data as *mut Box<dyn ReadSeek>);
        let pos = match origin {
            0 => {
                if offset < 0 {
                    return u64::MAX;
                }
                SeekFrom::Start(offset as u64)
            }
            1 => SeekFrom::Current(offset),
            2 => SeekFrom::End(offset),
            _ => return u64::MAX,
        };
        reader.seek(pos).unwrap_or(u64::MAX)
    }))
    .unwrap_or(u64::MAX)
}

unsafe extern "C" fn cb_get_size(user_data: *mut c_void) -> u64 {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let reader = &mut *(user_data as *mut Box<dyn ReadSeek>);
        let current = match reader.stream_position() {
            Ok(current) => current,
            Err(_) => return u64::MAX,
        };
        let size = match reader.seek(SeekFrom::End(0)) {
            Ok(size) => size,
            Err(_) => {
                let _ = reader.seek(SeekFrom::Start(current));
                return u64::MAX;
            }
        };
        if reader.seek(SeekFrom::Start(current)).is_err() {
            return u64::MAX;
        }
        size
    }))
    .unwrap_or(u64::MAX)
}

unsafe extern "C" fn cb_free_user_data(user_data: *mut c_void) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if !user_data.is_null() {
            let _ = Box::from_raw(user_data as *mut Box<dyn ReadSeek>);
        }
    }));
}

// --- Callback implementations for Box<dyn CloneableReadSeek> ---

unsafe extern "C" fn cb_read_cloneable(
    user_data: *mut c_void,
    dst: *mut c_char,
    len: usize,
) -> usize {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if len == 0 {
            return 0;
        }
        if dst.is_null() {
            return usize::MAX;
        }
        let reader = &mut *(user_data as *mut Box<dyn CloneableReadSeek>);
        let buf = std::slice::from_raw_parts_mut(dst as *mut u8, len);
        reader.read(buf).unwrap_or(usize::MAX)
    }))
    .unwrap_or(usize::MAX)
}

unsafe extern "C" fn cb_seek_cloneable(user_data: *mut c_void, offset: i64, origin: c_int) -> u64 {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let reader = &mut *(user_data as *mut Box<dyn CloneableReadSeek>);
        let pos = match origin {
            0 => {
                if offset < 0 {
                    return u64::MAX;
                }
                SeekFrom::Start(offset as u64)
            }
            1 => SeekFrom::Current(offset),
            2 => SeekFrom::End(offset),
            _ => return u64::MAX,
        };
        reader.seek(pos).unwrap_or(u64::MAX)
    }))
    .unwrap_or(u64::MAX)
}

unsafe extern "C" fn cb_get_size_cloneable(user_data: *mut c_void) -> u64 {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let reader = &mut *(user_data as *mut Box<dyn CloneableReadSeek>);
        let current = match reader.stream_position() {
            Ok(current) => current,
            Err(_) => return u64::MAX,
        };
        let size = match reader.seek(SeekFrom::End(0)) {
            Ok(size) => size,
            Err(_) => {
                let _ = reader.seek(SeekFrom::Start(current));
                return u64::MAX;
            }
        };
        if reader.seek(SeekFrom::Start(current)).is_err() {
            return u64::MAX;
        }
        size
    }))
    .unwrap_or(u64::MAX)
}

unsafe extern "C" fn cb_clone_cloneable(user_data: *mut c_void) -> *mut c_void {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let reader = &*(user_data as *mut Box<dyn CloneableReadSeek>);
        let cloned = reader.clone_box();
        Box::into_raw(Box::new(cloned)) as *mut c_void
    }))
    .unwrap_or(std::ptr::null_mut())
}

unsafe extern "C" fn cb_free_user_data_cloneable(user_data: *mut c_void) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if !user_data.is_null() {
            let _ = Box::from_raw(user_data as *mut Box<dyn CloneableReadSeek>);
        }
    }));
}

impl Drop for Reader {
    fn drop(&mut self) {
        unsafe {
            ffi::rgz_close(self.inner);
        }
    }
}

impl Reader {
    fn map_read_status(&self, status: ffi::rgz_status_t, read_bytes: usize) -> io::Result<usize> {
        match status {
            ffi::rgz_status_t::RGZ_STATUS_OK | ffi::rgz_status_t::RGZ_STATUS_EOF => Ok(read_bytes),
            _ => Err(io::Error::other(self.get_last_error())),
        }
    }

    /// Reads and discards up to `len` decompressed bytes without copying them into Rust memory.
    pub fn read_discard(&mut self, len: usize) -> io::Result<usize> {
        if len == 0 {
            return Ok(0);
        }

        let mut read_bytes: usize = 0;
        let status = unsafe { ffi::rgz_read_discard(self.inner, len, &mut read_bytes) };
        self.map_read_status(status, read_bytes)
    }

    /// Streams decompressed bytes directly into a Unix file descriptor.
    #[cfg(unix)]
    pub fn read_to_fd(&mut self, output_fd: RawFd, len: usize) -> io::Result<usize> {
        if len == 0 {
            return Ok(0);
        }

        let mut read_bytes: usize = 0;
        let status = unsafe { ffi::rgz_read_to_fd(self.inner, output_fd, len, &mut read_bytes) };
        self.map_read_status(status, read_bytes)
    }
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let mut read_bytes: usize = 0;
        let status = unsafe {
            ffi::rgz_read(
                self.inner,
                buf.as_mut_ptr() as *mut _,
                buf.len(),
                &mut read_bytes,
            )
        };

        self.map_read_status(status, read_bytes)
    }
}

fn checked_relative_seek(base: u64, offset: i64, label: &'static str) -> io::Result<u64> {
    if offset >= 0 {
        base.checked_add(offset as u64).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Seek overflow for {}", label),
            )
        })
    } else {
        base.checked_sub(offset.unsigned_abs()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Seek before start of {}", label),
            )
        })
    }
}

impl Seek for Reader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target_offset = match pos {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(offset) => {
                let mut current: u64 = 0;
                let status = unsafe { ffi::rgz_tell(self.inner, &mut current) };
                if status != ffi::rgz_status_t::RGZ_STATUS_OK {
                    return Err(io::Error::other(self.get_last_error()));
                }
                checked_relative_seek(current, offset, "stream")?
            }
            SeekFrom::End(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Seek from end is not yet supported by the rapidgzip native ABI v0",
                ));
            }
        };

        let status = unsafe { ffi::rgz_seek_to(self.inner, target_offset) };
        if status != ffi::rgz_status_t::RGZ_STATUS_OK {
            return Err(io::Error::other(self.get_last_error()));
        }

        Ok(target_offset)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        checked_relative_seek, ffi, CloneableReadSeek, FileWrapper, IoReadMode, ReaderBuilder,
    };
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};

    fn create_test_gz(payload: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(payload).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn checked_relative_seek_allows_valid_positive_offsets() {
        assert_eq!(checked_relative_seek(10, 5, "stream").unwrap(), 15);
    }

    #[test]
    fn checked_relative_seek_rejects_underflow() {
        let error = checked_relative_seek(3, -4, "stream").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn checked_relative_seek_rejects_overflow() {
        let error = checked_relative_seek(u64::MAX - 1, 5, "stream").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn file_wrapper_clone_box_returns_error_reader_when_reopen_fails() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let live_path = std::env::temp_dir().join(format!(
            "rapidgzip-file-wrapper-live-{}-{}",
            std::process::id(),
            unique,
        ));
        let missing_path = std::env::temp_dir().join(format!(
            "rapidgzip-file-wrapper-missing-{}-{}",
            std::process::id(),
            unique,
        ));

        {
            let mut file = std::fs::File::create(&live_path).unwrap();
            file.write_all(b"test").unwrap();
        }

        let file = std::fs::File::open(&live_path).unwrap();
        let mut cloned = FileWrapper {
            file,
            reopen_path: missing_path,
        }
        .clone_box();

        let mut buffer = [0_u8; 1];
        let read_error = cloned.read(&mut buffer).unwrap_err();
        assert_eq!(read_error.kind(), ErrorKind::Other);
        assert!(read_error
            .to_string()
            .contains("Failed to reopen file handle for cloneable reader"));

        let seek_error = cloned.seek(SeekFrom::Start(0)).unwrap_err();
        assert_eq!(seek_error.kind(), ErrorKind::Other);

        let _ = std::fs::remove_file(&live_path);
    }
    #[test]
    fn reader_can_be_moved_to_another_thread() {
        let payload = b"reader send should allow ownership transfer across threads".repeat(2048);
        let gzip = create_test_gz(&payload);
        let path = std::env::temp_dir().join(format!(
            "rapidgzip-send-reader-{}-{}.gz",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, gzip).unwrap();

        let reader = ReaderBuilder::new().open(&path).unwrap();
        let payload_from_thread = std::thread::spawn(move || {
            let mut reader = reader;
            let mut output = Vec::new();
            reader.read_to_end(&mut output).unwrap();
            output
        })
        .join()
        .unwrap();

        assert_eq!(payload_from_thread, payload);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn builder_encodes_io_read_mode_into_ffi_flags() {
        let config = ReaderBuilder::new()
            .io_read_mode(IoReadMode::Sequential)
            .to_ffi_config();
        assert_eq!(
            config.flags & ffi::RGZ_IO_READ_MODE_MASK,
            ffi::RGZ_IO_READ_MODE_SEQUENTIAL
        );

        let config = ReaderBuilder::new()
            .io_read_mode(IoReadMode::Pread)
            .to_ffi_config();
        assert_eq!(
            config.flags & ffi::RGZ_IO_READ_MODE_MASK,
            ffi::RGZ_IO_READ_MODE_PREAD
        );
    }

    #[test]
    fn read_discard_reports_decompressed_byte_count() {
        let payload = b"discard fast path should count bytes without copying".repeat(4096);
        let gzip = create_test_gz(&payload);
        let path = std::env::temp_dir().join(format!(
            "rapidgzip-read-discard-{}-{}.gz",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, gzip).unwrap();

        let mut reader = ReaderBuilder::new().open(&path).unwrap();
        let read_bytes = reader.read_discard(usize::MAX).unwrap();
        assert_eq!(read_bytes, payload.len());
        assert_eq!(reader.read_discard(usize::MAX).unwrap(), 0);

        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn read_to_fd_writes_decompressed_output() {
        use std::os::fd::AsRawFd;

        let payload = b"fd fast path should bypass the Rust copy loop".repeat(4096);
        let gzip = create_test_gz(&payload);
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let input_path = std::env::temp_dir().join(format!(
            "rapidgzip-read-to-fd-input-{}-{}.gz",
            std::process::id(),
            unique,
        ));
        let output_path = std::env::temp_dir().join(format!(
            "rapidgzip-read-to-fd-output-{}-{}",
            std::process::id(),
            unique,
        ));
        std::fs::write(&input_path, gzip).unwrap();

        let mut reader = ReaderBuilder::new().open(&input_path).unwrap();
        let output_file = std::fs::File::create(&output_path).unwrap();
        let read_bytes = reader
            .read_to_fd(output_file.as_raw_fd(), usize::MAX)
            .unwrap();
        drop(output_file);

        assert_eq!(read_bytes, payload.len());
        assert_eq!(std::fs::read(&output_path).unwrap(), payload);

        let _ = std::fs::remove_file(&input_path);
        let _ = std::fs::remove_file(&output_path);
    }
}
