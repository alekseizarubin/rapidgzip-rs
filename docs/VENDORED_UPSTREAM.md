# Vendored Upstream Provenance

This repository vendors the native decoder sources as a plain source snapshot
under `crates/rapidgzip-sys/vendor/librapidarchive`.

The snapshot was extracted from the stable integration baseline:

- source workspace commit: `302b273c833273a78a15a5f1691c3fc82b2b1c99`
- vendored `rapidgzip` commit: `88e1683ca2df73540f9efcd6664c46555c3bc221`
- vendored `librapidarchive` commit: `b7839405bff62377c5a3ae45942354761261eae2`

## Upstream Remotes

- `rapidgzip`: `https://github.com/mxmlnkn/rapidgzip.git`
- `librapidarchive` / `indexed_bzip2`: `https://github.com/mxmlnkn/indexed_bzip2.git`

## Nested Dependency Snapshot

- `src/external/cxxopts`: `44380e5a44706ab7347f400698c703eb2a196202` from `https://github.com/jarro2783/cxxopts.git`
- `src/external/isa-l`: `6a7c87e34293f427600e37f702d8a4d73391e48d` from `https://github.com/mxmlnkn/isa-l`
- `src/external/rpmalloc`: `66fd705a811764035ec80f54928748d2b31a3827` from `https://github.com/mjansson/rpmalloc.git`
- `src/external/zlib`: `51b7f2abdade71cd9bb0e7a373ef2610ec6f9daf` from `https://github.com/madler/zlib.git`
- `src/external/zlib-ng`: `860e4cff7917d93f54f5d7f0bc1d0e8b1a3cb988` from `https://github.com/zlib-ng/zlib-ng.git`

## Local Patch Carried Into The Snapshot

The vendored snapshot includes one local performance patch that was validated
before the repository split:

- `crates/rapidgzip-sys/vendor/librapidarchive/src/filereader/BitReader.hpp`
  - `DEFAULT_BUFFER_REFILL_SIZE` changed from `128_Ki` to `1_Mi`
  - reason: improved BGZF throughput on the current test machine

## Lean Snapshot Policy

The public repository carries a reduced upstream snapshot centered on the files
required to build the native library for `rapidgzip-sys`. Most unrelated
upstream repository material was omitted during the split, but some upstream
third-party test and documentation directories still remain inside nested
vendor trees such as `zlib`, `zlib-ng`, `isa-l`, and `rpmalloc`.
