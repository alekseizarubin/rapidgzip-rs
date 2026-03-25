# rapidgzip-rs

`rapidgzip-rs` provides Rust bindings for the native `rapidgzip` decoder with
support for gzip, BGZF, parallel decompression, and index import/export.

## Crates

- `rapidgzip`: safe high-level API for opening files and custom readers
- `rapidgzip-sys`: raw FFI bindings and native build glue

Companion repositories:

- CLI: `https://github.com/alekseizarubin/rapidgzip-cli`
- Benchmarks: `https://github.com/alekseizarubin/rapidgzip-benchmarks`

## Features

- gzip and BGZF decoding
- path-based readers with native fast paths
- callback-based readers for custom `Read + Seek` sources
- cloneable callback readers for parallel decode paths
- index import/export for random access workflows
- vendored accelerated native backend based on `librapidarchive`

## Installation

High-level API:

```toml
[dependencies]
rapidgzip = "0.1.0"
```

Low-level FFI:

```toml
[dependencies]
rapidgzip-sys = "0.1.0"
```

## Build Requirements

Building the native backend requires:

- Rust stable
- CMake 3.17 or newer
- a C/C++ toolchain with C++17 support
- `nasm` on targets where ISA-L uses x86 assembly

The vendored native sources live in
[`crates/rapidgzip-sys/vendor/librapidarchive`](crates/rapidgzip-sys/vendor/librapidarchive).
Upstream provenance is documented in
[`docs/VENDORED_UPSTREAM.md`](docs/VENDORED_UPSTREAM.md).

## Supported Platforms

The repository is intended to support these target families:

- Linux `x86_64` and `aarch64`
- macOS `x86_64` and `aarch64`
- Windows `x86_64` and `aarch64`

## Example

```rust
use rapidgzip::ReaderBuilder;
use std::io::Read;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut reader = ReaderBuilder::new()
        .parallelism(0)
        .open("reads.fastq.gz")?;

    let mut output = Vec::new();
    reader.read_to_end(&mut output)?;
    Ok(())
}
```

More examples are available in
[`crates/rapidgzip/examples`](crates/rapidgzip/examples).

## Limitations

- the published API currently exposes only the native backend
- `Reader::seek(SeekFrom::End(_))` is not supported by the current ABI
- `ReaderBuilder::open_reader` forces parallelism to `1` because generic readers
  cannot be cloned into independent file handles
- use `ReaderBuilder::open_cloneable_reader` when custom inputs should support
  parallel decode paths

## Repository Layout

- [`crates/rapidgzip`](crates/rapidgzip)
- [`crates/rapidgzip-sys`](crates/rapidgzip-sys)
- [`crates/rapidgzip-tests`](crates/rapidgzip-tests)
- [`docs/VENDORED_UPSTREAM.md`](docs/VENDORED_UPSTREAM.md)

The main CI job continuously validates Linux. Release and prebuilt workflows cover the broader target matrix.

## Development

Typical validation commands:

```bash
cargo test -p rapidgzip
cargo test -p rapidgzip-tests
cargo test --workspace
```
