use flate2::Compression;
use flate2::write::GzEncoder;
use rapidgzip::Reader;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_path(suffix: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    path.push(format!("rapidgzip-example-{suffix}-{nanos}"));
    path
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let archive_name = temp_path("index.gz");
    let index_name = temp_path("index.idx");

    let mut payload = Vec::new();
    for i in 0..256 {
        payload.extend_from_slice(format!("line-{i:03}\n").as_bytes());
    }

    {
        let file = std::fs::File::create(&archive_name)?;
        let mut encoder = GzEncoder::new(file, Compression::default());
        encoder.write_all(&payload)?;
        encoder.finish()?;
    }

    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        {
            let mut reader = Reader::open(&archive_name)?;
            let mut sink = Vec::new();
            reader.read_to_end(&mut sink)?;
            reader.export_index(&index_name)?;
        }

        {
            let mut reader = Reader::open(&archive_name)?;
            reader.import_index(&index_name)?;
            reader.seek(SeekFrom::Start(50))?;

            let mut buf = [0u8; 10];
            reader.read_exact(&mut buf)?;
            println!("Read at offset 50: {}", String::from_utf8_lossy(&buf));
        }

        println!("\nIndex import/export works correctly!");
        Ok(())
    })();

    let _ = fs::remove_file(&archive_name);
    let _ = fs::remove_file(&index_name);
    result
}
