use rapidgzip::Reader;
use std::io::{Cursor, Read, Seek, SeekFrom};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Create compressed data in memory
    println!("Preparing compressed data in memory...");
    let compressed_data = {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(b"This data comes from a Rust Cursor in memory!")?;
        encoder.finish()?
    };

    // 2. Open using our open_reader API
    println!("Opening Reader from memory Cursor...");
    let cursor = Cursor::new(compressed_data);
    let mut reader = Reader::open_reader(cursor)?;

    // 3. Test Read and Seek
    println!("Seeking to position 18...");
    reader.seek(SeekFrom::Start(18))?;

    let mut buf = [0u8; 11];
    reader.read_exact(&mut buf)?;
    println!("Read at offset 18: '{}'", String::from_utf8_lossy(&buf));

    println!("\nCallback-based reading works correctly!");
    Ok(())
}
