# ACPI Test Tool

This is a tool similar to `acpiexec`, that can load an AML file and presents a TUI for users to enter commands that
interrogate the ACPI namespace generated from the AML.

See `INTENT.md` for the plan, MVP definition and design decisions.

## Status

The MVP is implemented: `acpi-test [--acpi | --uacpi] <file>` loads a table file and offers a REPL with `dump`,
`find <name>`, `execute <name>` (methods with no arguments only), `help` and `exit` (each with a short form). Two AML
interpreters are supported: the `acpi` crate (default) and uACPI. Future work (ACPICA, method arguments, `set`) is
listed in `INTENT.md`.

## Tech Stack

* The tool is written in Rust, on the nightly toolchain (required by `acpi`; see `rust-toolchain.toml`).
* It uses the `acpi` crate, but not the `crates.io` published version - it uses the current `main` branch from GitHub.
* `aml-test-tools` (from the same GitHub repo, not on `crates.io`) provides `NullHandler` and `new_interpreter`.
* `rustyline` provides the REPL line editing.
* uACPI is a C library, checked out as a git submodule in `ext-libs/uACPI`. `uacpi-sys` builds it with the `cc` crate
  and generates Rust bindings with `bindgen` (which needs libclang). All of this is behind the `uacpi` Cargo feature,
  which is on by default; `--no-default-features` builds without a C toolchain or libclang.

## Layout

* `src/main.rs` - file loading, choosing the backend, starts the REPL.
* `src/cli.rs` - command-line parsing (`--acpi`, `--uacpi`).
* `src/tables.rs` - splits a binary image into tables (36-byte SDT headers). Interpreter-independent.
* `src/backend.rs` - the `AmlBackend` trait and the backend-neutral `Entry`/`Value` types.
* `src/backend/acpi_crate.rs` - `AcpiCrateBackend`, built on the `acpi` crate's interpreter.
* `src/backend/uacpi.rs` - `UacpiBackend`, built on uACPI (feature `uacpi`). Only one can exist per process, because
  uACPI has global state.
  * `uacpi/firmware.rs` - builds the RSDP, XSDT, FADT and FACS that uACPI starts from, around the input's AML tables.
  * `uacpi/host.rs` - the `uacpi_kernel_*` functions uACPI calls back into. Memory and port I/O are simulated.
* `uacpi-sys/` - workspace crate: `build.rs` compiles uACPI and runs bindgen; `wrapper.h` lists the headers to bind.
* `src/render.rs` - tree rendering and `find` matching over `Entry` lists.
* `src/repl.rs` - command parsing/execution and the `rustyline` loop.
* `tests/common/mod.rs` - shared test helpers, including `compile_asl` (copies to a temp dir and runs `iasl`) and
  `backends()`.
* `tests/execute.rs` - end-to-end tests of `execute`, using `tests/aml/methods.asl` and `returns.asl`.
* `tests/cli.rs` - end-to-end tests using hand-assembled tables, driving the binary over stdin.
* `tests/aml/` - ASL sources used by tests. `iasl` is assumed to be on the path. Tests must copy an ASL file to a
  temporary directory and compile it there, so no compiler output lands in the crate's directories.

Most end-to-end tests run once per interpreter (`backends()`) and expect identical output from each. Where the
interpreters legitimately differ (uACPI can't show a mutex's sync level, or field and region details), the tests avoid
depending on that detail.

## Commands

* `cargo build`, `cargo test`, `cargo clippy --workspace --all-targets`, `cargo fmt`
* `cargo run -- [--acpi | --uacpi] <file.aml>`
* `cargo build --no-default-features` - without uACPI (no C compiler or libclang needed)

## Windows notes

* The Visual Studio C++ build tools' `link.exe` must come before Git's `/usr/bin/link` on `PATH`. In Git Bash it
  usually doesn't, and Rust then fails to link build scripts with "extra operand" errors. PowerShell, or a Developer
  prompt, is fine.
