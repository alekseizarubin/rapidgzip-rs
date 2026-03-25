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
}
