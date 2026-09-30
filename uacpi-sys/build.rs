//! Compiles uACPI from the `ext-libs/uACPI` submodule and generates Rust bindings for it.
//!
//! Requires a C compiler (found by the `cc` crate) and libclang (for `bindgen`).

use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let uacpi = manifest_dir.join("..").join("ext-libs").join("uACPI");
    let include = uacpi.join("include");
    let source = uacpi.join("source");

    if !include.join("uacpi").join("uacpi.h").exists() {
        panic!(
            "uACPI sources not found at {}; run `git submodule update --init`",
            uacpi.display()
        );
    }

    // Compile every uACPI source file. The list mirrors `source/files.cmake`; globbing the
    // directory keeps this in step when a submodule update adds a file.
    let mut sources: Vec<PathBuf> = std::fs::read_dir(&source)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "c"))
        .collect();
    sources.sort();

    let mut build = cc::Build::new();
    build.files(&sources).include(&include).warnings(false);
    // uACPI uses `_Static_assert`, which MSVC only accepts in C11 mode. This is a no-op flag
    // for compilers that don't recognise it.
    build.flag_if_supported("/std:c11");
    build.compile("uacpi");

    let bindings = bindgen::Builder::default()
        .header(manifest_dir.join("wrapper.h").to_str().unwrap())
        .clang_arg(format!("-I{}", include.display()))
        .allowlist_function("uacpi_.*")
        .allowlist_type("uacpi_.*|acpi_.*")
        .allowlist_var("UACPI_.*|ACPI_.*")
        .derive_default(true)
        .derive_debug(true)
        .layout_tests(false)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("bindgen failed to generate uACPI bindings");

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("bindings.rs");
    bindings.write_to_file(out).expect("cannot write bindings");

    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed={}", include.display());
}
