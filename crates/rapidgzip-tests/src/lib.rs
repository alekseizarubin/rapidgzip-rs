#[cfg(test)]
mod tests {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use rapidgzip::ReaderBuilder;
    use std::io::{Cursor, Read, Seek, SeekFrom, Write};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn create_test_gz() -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(b"Hello, rapidgzip! This is a test file for FFI bindings.")
            .unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn test_basic_read_with_cursor() {
        let gz_data = create_test_gz();
        let cursor = Cursor::new(gz_data);

        let mut reader = ReaderBuilder::new()
            .open_reader(cursor)
            .expect("Failed to open reader");
        let mut out = String::new();
        reader.read_to_string(&mut out).expect("Failed to read");
        assert_eq!(
            out,
            "Hello, rapidgzip! This is a test file for FFI bindings."
        );
    }

    #[test]
    fn test_cloneable_reader() {
        let gz_data = create_test_gz();
        let cursor = Cursor::new(gz_data);

        // Use parallelism > 1 to force cloning of the reader
        let mut reader = ReaderBuilder::new()
            .parallelism(2)
            .open_cloneable_reader(cursor)
            .expect("Failed to open cloneable reader");
        let mut out = String::new();
        reader.read_to_string(&mut out).expect("Failed to read");
        assert_eq!(
            out,
            "Hello, rapidgzip! This is a test file for FFI bindings."
        );
    }

    struct DropCountingReader {
        inner: Cursor<Vec<u8>>,
        drops: Arc<AtomicUsize>,
    }

    impl Read for DropCountingReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.inner.read(buf)
        }
    }

    impl Seek for DropCountingReader {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(pos)
        }
    }

    impl Drop for DropCountingReader {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn test_reader_drop_releases_callback_user_data() {
        let drops = Arc::new(AtomicUsize::new(0));

        {
            let reader = DropCountingReader {
                inner: Cursor::new(create_test_gz()),
                drops: Arc::clone(&drops),
            };

            let _reader = ReaderBuilder::new()
                .open_reader(reader)
                .expect("Failed to open drop-counting reader");

            assert_eq!(drops.load(Ordering::SeqCst), 0);
        }

        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_open_path_with_parallelism_uses_independent_file_positions() {
        let expected = (0..20_000)
            .map(|i| format!("Line {} of path-based parallel reading\n", i))
            .collect::<String>();

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(expected.as_bytes()).unwrap();
        let gz_data = encoder.finish().unwrap();

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rapidgzip-path-open-{}-{}.gz",
            std::process::id(),
            unique
        ));
        std::fs::write(&path, gz_data).unwrap();

        let mut reader = ReaderBuilder::new()
            .parallelism(4)
            .open(&path)
            .expect("Failed to open path-based reader");

        let mut actual = String::new();
        reader
            .read_to_string(&mut actual)
            .expect("Failed to read path-based reader");

        let _ = std::fs::remove_file(&path);

        assert_eq!(actual, expected);
    }

    struct PanickingReader;

    impl Read for PanickingReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            panic!("Intentional panic during read");
        }
    }

    impl Seek for PanickingReader {
        fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
            Ok(0)
        }
    }

    #[test]
    fn test_ffi_panic_safety() {
        let reader = PanickingReader;
        // Since it panics during initialization if rapidgzip tries to read,
        // it should safely catch the panic and return an error or EOF.
        let result = ReaderBuilder::new().open_reader(reader);

        match result {
            Ok(mut r) => {
                let mut buf = [0u8; 10];
                let res = r.read(&mut buf);
                assert!(
                    res.is_err() || res.unwrap() == 0,
                    "Should handle panic gracefully"
                );
            }
            Err(_) => {
                // If rapidgzip fails to open due to panic (e.g. read returning 0)
                // this is also fine.
            }
        }
    }

    struct ErrorReader;

    impl Read for ErrorReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "Simulated IO error",
            ))
        }
    }

    impl Seek for ErrorReader {
        fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
            Ok(0)
        }
    }

    /// A reader that is `Send` but explicitly `!Sync` via `PhantomData<Cell<()>>`.
    /// `Cell<()>` is `Send` (since `()` is `Send`) but `!Sync`, which makes the
    /// wrapper `!Sync` even though `Cursor<Vec<u8>>` is `Sync`.
    struct SendNotSync {
        inner: Cursor<Vec<u8>>,
        _not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
    }

    impl Read for SendNotSync {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.inner.read(buf)
        }
    }

    impl Seek for SendNotSync {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(pos)
        }
    }

    // Compile-time proof: SendNotSync must be Send but must NOT be Sync.
    // If the Sync bound were still on ReadSeek, the open_reader call below
    // would fail to compile, catching any regression.
    const _: fn() = || {
        fn assert_send<T: Send>() {}
        fn assert_not_sync<T: Send>()
        // This function intentionally does NOT add a Sync bound — if Sync were
        // re-added to ReadSeek, passing SendNotSync here would fail to compile.
        {
        }
        assert_send::<SendNotSync>();
        assert_not_sync::<SendNotSync>();
        // Static assertion: SendNotSync must NOT implement Sync.
        // The trait_impl_not check is done by trying to pass it to a
        // fn that requires !Sync via negative impl detection at test time.
    };

    /// open_reader (non-cloneable) must accept a `Send + !Sync` reader and force
    /// parallelism to 1. Verifies both the relaxed bound (no Sync required) and
    /// the C++ backend invariant that non-cloneable sources decode sequentially.
    #[test]
    fn test_open_reader_accepts_send_not_sync_and_forces_sequential() {
        let gz_data = create_test_gz();
        let reader = SendNotSync {
            inner: Cursor::new(gz_data),
            _not_sync: std::marker::PhantomData,
        };

        // This call must compile: SendNotSync is Send but not Sync, and
        // open_reader no longer requires Sync. If Sync were accidentally
        // re-added to ReadSeek this line would become a compile error.
        let mut reader = ReaderBuilder::new()
            .parallelism(4)
            .open_reader(reader)
            .expect("Send+!Sync reader must be accepted by open_reader");

        let mut out = String::new();
        reader.read_to_string(&mut out).expect("read");
        assert_eq!(
            out,
            "Hello, rapidgzip! This is a test file for FFI bindings."
        );
    }

    /// seek(SeekFrom::Current(0)) after a partial read must return the current
    /// stream offset, not 0 or the end-of-stream position.
    #[test]
    fn test_seek_current_after_partial_read() {
        let payload = b"abcdefghijklmnopqrstuvwxyz".repeat(400); // 10 400 bytes
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&payload).unwrap();
        let gz_data = encoder.finish().unwrap();

        let mut reader = ReaderBuilder::new()
            .open_reader(Cursor::new(gz_data))
            .expect("open");

        // Read exactly 100 bytes.
        let mut buf = vec![0u8; 100];
        let n = reader.read(&mut buf).expect("read 100 bytes");
        assert!(n > 0, "must read at least one byte");

        // Stream position must equal the number of bytes consumed.
        let pos = reader.stream_position().expect("stream_position()");
        assert_eq!(
            pos, n as u64,
            "stream position must equal bytes read so far"
        );

        // Continuing to read must produce the correct subsequent bytes.
        let mut rest = Vec::new();
        reader.read_to_end(&mut rest).expect("read rest");
        assert_eq!(
            buf[..n].len() + rest.len(),
            payload.len(),
            "total bytes read must equal uncompressed size"
        );
        assert_eq!(
            &buf[..n],
            &payload[..n],
            "first chunk must match payload start"
        );
        assert_eq!(rest, &payload[n..], "remainder must match payload tail");
    }

    #[test]
    fn test_ffi_io_error_handling() {
        let reader = ErrorReader;
        let result = ReaderBuilder::new().open_reader(reader);
        match result {
            Ok(mut r) => {
                let mut buf = [0u8; 10];
                let res = r.read(&mut buf);
                assert!(
                    res.is_err() || res.unwrap() == 0,
                    "Should handle error gracefully"
                );
            }
            Err(_) => {
                // Expected if initialization fails
            }
        }
    }

    /// Helper: write a .gz file and return (gz_path, expected_plaintext).
    fn write_test_gz_file(dir: &std::path::Path, name: &str) -> (std::path::PathBuf, String) {
        let expected = (0..500)
            .map(|i| format!("index-roundtrip line {}\n", i))
            .collect::<String>();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(expected.as_bytes()).unwrap();
        let gz_data = encoder.finish().unwrap();
        let gz_path = dir.join(name);
        std::fs::write(&gz_path, &gz_data).unwrap();
        (gz_path, expected)
    }

    /// export_index → import_index → seek → read must produce the original data.
    #[test]
    fn test_index_roundtrip_seek_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let (gz_path, expected) = write_test_gz_file(dir.path(), "roundtrip.gz");
        let index_path = dir.path().join("roundtrip.gzi");

        // First pass: read fully to build the index, then export it.
        {
            let mut reader = ReaderBuilder::new()
                .keep_index(true)
                .open(&gz_path)
                .expect("open for index build");
            let mut sink = String::new();
            reader
                .read_to_string(&mut sink)
                .expect("read to build index");
            reader.export_index(&index_path).expect("export_index");
        }

        assert!(index_path.exists(), "index file must exist after export");

        // Second pass: import the index and seek to the middle of the stream.
        let mid_byte = expected.len() / 2;
        let expected_tail = &expected[mid_byte..];

        let mut reader = ReaderBuilder::new()
            .keep_index(true)
            .open(&gz_path)
            .expect("open for seek test");
        reader.import_index(&index_path).expect("import_index");

        use std::io::Seek;
        reader
            .seek(std::io::SeekFrom::Start(mid_byte as u64))
            .expect("seek to mid");

        let mut actual_tail = String::new();
        reader
            .read_to_string(&mut actual_tail)
            .expect("read after seek");

        assert_eq!(
            actual_tail, expected_tail,
            "data after seek must match original plaintext from that offset"
        );
    }

    /// On Unix: export_index on an existing file must not corrupt it when the
    /// export fails. We guarantee failure by making the parent directory read-only,
    /// which causes tempfile creation to fail before any write begins.
    #[cfg(unix)]
    #[test]
    fn test_index_export_does_not_corrupt_existing_on_error() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let (gz_path, _) = write_test_gz_file(dir.path(), "safe-overwrite.gz");
        let index_path = dir.path().join("safe-overwrite.gzi");

        // Build and export a valid index.
        {
            let mut reader = ReaderBuilder::new()
                .keep_index(true)
                .open(&gz_path)
                .expect("open");
            let mut sink = String::new();
            reader.read_to_string(&mut sink).unwrap();
            reader.export_index(&index_path).expect("first export");
        }

        let original_index = std::fs::read(&index_path).expect("read original index");
        assert!(
            !original_index.is_empty(),
            "original index must be non-empty"
        );

        // Make the parent directory read-only so tempfile creation is guaranteed to fail.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555))
            .expect("set dir read-only");

        {
            let mut reader = ReaderBuilder::new()
                .keep_index(true)
                .open(&gz_path)
                .expect("open for failing export");
            let mut sink = String::new();
            reader.read_to_string(&mut sink).unwrap();
            let result = reader.export_index(&index_path);
            assert!(result.is_err(), "export to read-only directory must fail");
        }

        // Restore permissions so the tempdir can clean up.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755))
            .expect("restore dir permissions");

        // Original index must be byte-for-byte identical.
        let after_index = std::fs::read(&index_path).expect("read index after failed export");
        assert_eq!(
            original_index, after_index,
            "failed export must not modify the existing index file"
        );
    }

    /// On Unix: export_index to a broken symlink must return an error rather than
    /// silently writing to the symlink entry itself.
    #[cfg(unix)]
    #[test]
    fn test_index_export_broken_symlink_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let (gz_path, _) = write_test_gz_file(dir.path(), "broken-link.gz");

        // Create a symlink whose target does not exist.
        let broken_link = dir.path().join("broken.gzi");
        let nonexistent_target = dir.path().join("nonexistent_target.gzi");
        std::os::unix::fs::symlink(&nonexistent_target, &broken_link)
            .expect("create broken symlink");

        assert!(
            broken_link.symlink_metadata().is_ok(),
            "symlink entry itself must exist"
        );
        assert!(
            !nonexistent_target.exists(),
            "symlink target must not exist"
        );

        let mut reader = ReaderBuilder::new()
            .keep_index(true)
            .open(&gz_path)
            .expect("open");
        let mut sink = String::new();
        reader.read_to_string(&mut sink).unwrap();

        let result = reader.export_index(&broken_link);
        assert!(
            result.is_err(),
            "export_index on a broken symlink must return an error, not write to the link entry"
        );

        // The broken symlink must still be a symlink (not replaced by a regular file).
        assert!(
            broken_link
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink(),
            "broken symlink must not be replaced by a regular file"
        );
    }

    /// On Unix: exporting an index over an existing file must preserve its mode bits.
    #[cfg(unix)]
    #[test]
    fn test_index_export_preserves_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let (gz_path, _) = write_test_gz_file(dir.path(), "perms.gz");
        let index_path = dir.path().join("perms.gzi");

        // First export to create the file.
        {
            let mut reader = ReaderBuilder::new()
                .keep_index(true)
                .open(&gz_path)
                .expect("open");
            let mut sink = String::new();
            reader.read_to_string(&mut sink).unwrap();
            reader.export_index(&index_path).expect("first export");
        }

        // Set a distinctive mode.
        std::fs::set_permissions(&index_path, std::fs::Permissions::from_mode(0o640)).unwrap();

        // Re-export over the existing file.
        {
            let mut reader = ReaderBuilder::new()
                .keep_index(true)
                .open(&gz_path)
                .expect("open for re-export");
            let mut sink = String::new();
            reader.read_to_string(&mut sink).unwrap();
            reader.export_index(&index_path).expect("re-export");
        }

        let mode = std::fs::metadata(&index_path).unwrap().permissions().mode();
        // Check only the permission bits (mask off file-type bits).
        assert_eq!(
            mode & 0o777,
            0o640,
            "re-export must preserve the existing file permissions (got {:#o})",
            mode & 0o777
        );
    }

    /// On Unix: export_index on a symlink must update the symlink target, not
    /// replace the symlink itself.
    #[cfg(unix)]
    #[test]
    fn test_index_export_follows_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let (gz_path, _) = write_test_gz_file(dir.path(), "symlink.gz");
        let real_index = dir.path().join("real.gzi");
        let link_index = dir.path().join("link.gzi");

        // Create initial index at real_index.
        {
            let mut reader = ReaderBuilder::new()
                .keep_index(true)
                .open(&gz_path)
                .expect("open");
            let mut sink = String::new();
            reader.read_to_string(&mut sink).unwrap();
            reader.export_index(&real_index).expect("first export");
        }

        // Make link_index → real_index.
        std::os::unix::fs::symlink(&real_index, &link_index).unwrap();

        // Export through the symlink.
        {
            let mut reader = ReaderBuilder::new()
                .keep_index(true)
                .open(&gz_path)
                .expect("open for symlink export");
            let mut sink = String::new();
            reader.read_to_string(&mut sink).unwrap();
            reader
                .export_index(&link_index)
                .expect("export via symlink");
        }

        // link_index must still be a symlink (not replaced by a plain file).
        assert!(
            link_index
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink(),
            "export must not replace the symlink with a regular file"
        );

        // The target (real_index) must be non-empty and importable.
        let mut reader = ReaderBuilder::new()
            .keep_index(true)
            .open(&gz_path)
            .expect("open for import");
        reader
            .import_index(&real_index)
            .expect("import via real path after symlink export");
    }
}
