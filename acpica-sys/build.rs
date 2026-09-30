//! Compiles ACPICA from the `ext-libs/acpica` submodule and generates Rust bindings for it.
//!
//! Requires a C compiler (found by the `cc` crate) and libclang (for `bindgen`).
//!
//! Only the core interpreter is built: the parser, namespace, executer, dispatcher, events,
//! hardware, resource and table components, plus the utilities they use. The debugger and the
//! disassembler are left out, and so is the ASL compiler (the only part that needs bison and
//! flex).

use std::path::{Path, PathBuf};

/// The `source/components` subdirectories that make up the core library.
const COMPONENTS: &[&str] = &[
    "dispatcher",
    "events",
    "executer",
    "hardware",
    "namespace",
    "parser",
    "resources",
    "tables",
    "utilities",
];

/// Source files that are left out:
///
/// * `rsdump.c` and `rsdumpinfo.c` print resource descriptors for the debugger, and can't be
///   compiled without it: `rsdump.c` refers to tables that its header only declares when the
///   debugger is enabled. Nothing else in the core library uses them.
/// * `utprint.c` only holds ACPICA's own versions of the C library's `printf` family, for
///   builds that have no C library. This build uses the system's (`ACPI_USE_SYSTEM_CLIBRARY`),
///   and the modern Windows C runtime defines those functions inline, which would clash.
const EXCLUDED: &[&str] = &["rsdump.c", "rsdumpinfo.c", "utprint.c"];

fn c_files_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "c"))
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            !EXCLUDED.contains(&name.as_ref())
        })
        .collect();
    files.sort();
    files
}

/// The macros that select ACPICA's platform configuration, as `(name, value)` pairs.
///
/// ACPICA chooses its platform header from these (see `platform/acenv.h`). `ACPI_APPLICATION`
/// makes it build as a user-space library; `ACPI_LIBRARY` would also switch on the debugger and
/// disassembler, which aren't wanted here.
fn platform_defines() -> Vec<(&'static str, Option<&'static str>)> {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let pointer_width = std::env::var("CARGO_CFG_TARGET_POINTER_WIDTH").unwrap();

    let mut defines = vec![("ACPI_APPLICATION", None)];
    if target_os == "windows" {
        // `WIN32` selects a 32-bit build, `WIN64` a 64-bit one.
        defines.push((
            if pointer_width == "64" {
                "WIN64"
            } else {
                "WIN32"
            },
            None,
        ));
        defines.push(("_CRT_SECURE_NO_WARNINGS", None));
    }
    defines
}

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let source = manifest_dir
        .join("..")
        .join("ext-libs")
        .join("acpica")
        .join("source");
    let include = source.join("include");

    if !include.join("acpi.h").exists() {
        panic!(
            "ACPICA sources not found at {}; run `git submodule update --init`",
            source.display()
        );
    }

    let mut sources: Vec<PathBuf> = COMPONENTS
        .iter()
        .flat_map(|name| c_files_in(&source.join("components").join(name)))
        .collect();
    sources.push(manifest_dir.join("shim.c"));

    let defines = platform_defines();
    let mut build = cc::Build::new();
    build.files(&sources).include(&include).warnings(false);
    for (name, value) in &defines {
        build.define(name, *value);
    }
    let msvc = build.get_compiler().is_like_msvc();
    if msvc {
        // ACPICA's `platform/acwin64.h` defines `snprintf` as a macro, for very old versions of
        // MSVC. Current versions of `<stdio.h>` refuse to be included after that. Including it
        // first makes ACPICA's later include a no-op, and the macro then just picks the older
        // `_snprintf`, which still exists.
        build.flag("/FIstdio.h");
    }
    build.compile("acpica");

    let mut bindings = bindgen::Builder::default()
        .header(manifest_dir.join("wrapper.h").to_str().unwrap())
        .clang_arg(format!("-I{}", include.display()))
        .allowlist_function("Acpi.*")
        .allowlist_type("ACPI_.*|acpi_.*")
        .allowlist_var("ACPI_.*|AE_.*")
        .derive_default(true)
        .derive_debug(true)
        .layout_tests(false)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));
    if msvc {
        // The same `snprintf` clash as above, seen by clang.
        bindings = bindings.clang_arg("-include").clang_arg("stdio.h");
    }
    for (name, value) in &defines {
        bindings = bindings.clang_arg(match value {
            Some(value) => format!("-D{name}={value}"),
            None => format!("-D{name}"),
        });
    }
    let bindings = bindings
        .generate()
        .expect("bindgen failed to generate ACPICA bindings");

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("bindings.rs");
    bindings.write_to_file(out).expect("cannot write bindings");

    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed=shim.c");
    println!("cargo:rerun-if-changed=wrapper.h");
}
