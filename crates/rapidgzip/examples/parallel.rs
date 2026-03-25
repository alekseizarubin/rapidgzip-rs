use rapidgzip::ReaderBuilder;
use std::io::{Cursor, Read};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Prepare data
    let compressed_data = {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        for i in 0..10000 {
            writeln!(
                encoder,
                "Line {} of some repeating data to make it worth parallelizing...",
                i
            )?;
        }
        encoder.finish()?
    };

    // 2. Use Builder to configure parallelism
    println!("Opening with 4 threads using ReaderBuilder...");
    let cursor = Cursor::new(compressed_data);

    let mut reader = ReaderBuilder::new()
        .parallelism(4)
        .chunk_size(64 * 1024)
        .open_cloneable_reader(cursor)?;

    // 3. Read some data
    let mut output = String::new();
    reader.read_to_string(&mut output)?;
    println!("Read {} bytes successfully with 4 threads!", output.len());

    println!("\nMulti-threaded callback-based reading is working!");
    Ok(())
}
