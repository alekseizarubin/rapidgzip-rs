//! Safe Rust bindings for the native `rapidgzip` decoder.
//!
//! The public crate currently exposes only the vendored native backend. The
//! fallback backend is intentionally not part of the published API surface yet.

#[cfg(not(feature = "native"))]
compile_error!("rapidgzip currently requires the `native` feature; the fallback backend is not yet integrated into the public crate");

#[cfg(feature = "native")]
mod native_impl;

#[cfg(feature = "native")]
pub use native_impl::*;
