use flate2::write::GzEncoder;
use flate2::Compression;
use rapidgzip::Reader;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    path.push(format!("rapidgzip-example-{name}-{nanos}.gz"));
    path
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let archive_path = temp_path("simple");
    let payload = b"Hello, rapidgzip example!";

    {
        let file = std::fs::File::create(&archive_path)?;
        let mut encoder = GzEncoder::new(file, Compression::default());
        encoder.write_all(payload)?;
        encoder.finish()?;
    }

    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut reader = Reader::open(&archive_path)?;

        let mut prefix = [0u8; 5];
        reader.read_exact(&mut prefix)?;
        println!("Read: {}", String::from_utf8_lossy(&prefix));

        reader.seek(SeekFrom::Start(7))?;

        let mut rest = [0u8; 11];
        reader.read_exact(&mut rest)?;
        println!("Read at offset 7: {}", String::from_utf8_lossy(&rest));

        println!("\nSuccess! rapidgzip-rs works correctly.");
        Ok(())
    })();

    let _ = fs::remove_file(&archive_path);
    result
}
