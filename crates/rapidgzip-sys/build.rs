use std::env;
use std::path::{Path, PathBuf};

fn build_from_source(capi_dir: &Path, target: &str) -> PathBuf {
    let mut config = cmake::Config::new(capi_dir);

    if target.contains("msvc") {
        config.cxxflag("/EHsc");
    }

    if target.contains("windows-msvc") {
        // Keep the native dependency on the Windows path in Release mode.
        // The upstream ISA-L/zlib-ng stack is validated in Release in our CI,
        // while Cargo's default debug profile would otherwise drive a Debug
        // multi-config CMake build here.
        config.profile("Release");
    }

    config.build()
}

fn emit_link_search(path: &Path) {
    if path.exists() {
        println!("cargo:rustc-link-search=native={}", path.display());
    }
}

fn static_lib_filename(name: &str, target: &str) -> String {
    if target.contains("windows") {
        format!("{name}.lib")
    } else {
        format!("lib{name}.a")
    }
}

fn has_static_lib(dir: &Path, name: &str, target: &str) -> bool {
    dir.join(static_lib_filename(name, target)).exists()
}

fn emit_common_system_links(target: &str, bundled_zlib: bool) {
    if target.contains("apple") {
        println!("cargo:rustc-link-lib=dylib=c++");
    } else if target.contains("linux") || target.contains("windows-gnu") {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }

    if !target.contains("windows-msvc") && !bundled_zlib {
        println!("cargo:rustc-link-lib=z");
    }
}

fn emit_feature_cfgs(has_rpmalloc: bool, has_isal: bool, bundled_zlib: bool) {
    println!("cargo:rustc-check-cfg=cfg(rapidgzip_has_rpmalloc)");
    println!("cargo:rustc-check-cfg=cfg(rapidgzip_has_isal)");
    println!("cargo:rustc-check-cfg=cfg(rapidgzip_has_bundled_zlib)");

    if has_rpmalloc {
        println!("cargo:rustc-cfg=rapidgzip_has_rpmalloc");
    }
    if has_isal {
        println!("cargo:rustc-cfg=rapidgzip_has_isal");
    }
    if bundled_zlib {
        println!("cargo:rustc-cfg=rapidgzip_has_bundled_zlib");
    }
}

fn emit_source_build_links(dst: &Path, target: &str) {
    let librapidarchive_src_dir = dst.join("build/librapidarchive/src");
    let librapidarchive_release_dir = librapidarchive_src_dir.join("Release");
    let isal_build_dir = dst.join("build/_deps/isal_project-build");
    let isal_release_dir = isal_build_dir.join("Release");

    emit_link_search(&dst.join("lib"));
    emit_link_search(&dst.join("build"));
    emit_link_search(&librapidarchive_src_dir);
    emit_link_search(&librapidarchive_release_dir);
    emit_link_search(&isal_build_dir);
    emit_link_search(&isal_release_dir);

    let bundled_zlib = librapidarchive_src_dir
        .join(static_lib_filename("zlibstatic", target))
        .exists()
        || librapidarchive_release_dir
            .join(static_lib_filename("zlibstatic", target))
            .exists();
    let has_rpmalloc = librapidarchive_src_dir
        .join(static_lib_filename("rpmalloc", target))
        .exists()
        || librapidarchive_release_dir
            .join(static_lib_filename("rpmalloc", target))
            .exists();
    let has_isal = isal_build_dir
        .join(static_lib_filename("isal", target))
        .exists()
        || isal_release_dir
            .join(static_lib_filename("isal", target))
            .exists();

    emit_feature_cfgs(has_rpmalloc, has_isal, bundled_zlib);

    println!("cargo:rustc-link-lib=static=rapidgzip-capi");
    if has_rpmalloc {
        println!("cargo:rustc-link-lib=static=rpmalloc");
    }
    if has_isal {
        println!("cargo:rustc-link-lib=static=isal");
    }
    if bundled_zlib {
        println!("cargo:rustc-link-lib=static=zlibstatic");
    }

    emit_common_system_links(target, bundled_zlib);
}

fn emit_prebuilt_links(prebuilt_dir: &Path, target: &str) {
    emit_link_search(prebuilt_dir);

    let bundled_zlib = has_static_lib(prebuilt_dir, "zlibstatic", target);
    let has_rpmalloc = has_static_lib(prebuilt_dir, "rpmalloc", target);
    let has_isal = has_static_lib(prebuilt_dir, "isal", target);

    emit_feature_cfgs(has_rpmalloc, has_isal, bundled_zlib);

    println!("cargo:rustc-link-lib=static=rapidgzip-capi");
    if has_rpmalloc {
        println!("cargo:rustc-link-lib=static=rpmalloc");
    }
    if has_isal {
        println!("cargo:rustc-link-lib=static=isal");
    }
    if bundled_zlib {
        println!("cargo:rustc-link-lib=static=zlibstatic");
    }

    emit_common_system_links(target, bundled_zlib);
}

fn main() {
    if env::var_os("DOCS_RS").is_some() {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let target = env::var("TARGET").unwrap();
    let _host = env::var("HOST").unwrap();

    let lib_name = static_lib_filename("rapidgzip-capi", &target);

    let prebuilt_dir = manifest_dir.join("prebuilt").join(&target);
    let prebuilt_lib = prebuilt_dir.join(lib_name);
    let capi_dir = manifest_dir.join("native/rapidgzip-capi");

    let prefer_prebuilt = env::var_os("RAPIDGZIP_USE_PREBUILT").is_some();

    if capi_dir.exists() && !prefer_prebuilt {
        let dst = build_from_source(&capi_dir, &target);
        emit_source_build_links(&dst, &target);
    } else if prebuilt_lib.exists() {
        emit_prebuilt_links(&prebuilt_dir, &target);
    } else if capi_dir.exists() {
        let dst = build_from_source(&capi_dir, &target);
        emit_source_build_links(&dst, &target);
    } else {
        panic!("Neither source directory nor prebuilt binary found for rapidgzip-capi");
    }

    let upstream_dir = manifest_dir.join("vendor/librapidarchive");
    let prebuilt_root = manifest_dir.join("prebuilt");

    println!("cargo:rerun-if-changed=build.rs");
    println!(
        "cargo:rerun-if-changed={}",
        capi_dir.join("CMakeLists.txt").display()
    );
    println!("cargo:rerun-if-changed={}", capi_dir.join("src").display());
    println!("cargo:rerun-if-changed={}", upstream_dir.display());
    if prebuilt_root.exists() {
        println!("cargo:rerun-if-changed={}", prebuilt_root.display());
    }
}
