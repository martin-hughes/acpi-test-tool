# ACPI Test Tool

This is a tool similar to `acpiexec`, that can load an AML file and presents a TUI for users to enter commands that
interrogate the ACPI namespace generated from the AML.

See `INTENT.md` for the plan, MVP definition and design decisions.

## Status

The MVP is implemented: `acpi-test [--acpi | --uacpi | --acpica] <file>` loads a table file and offers a REPL with
`dump`, `find <name>`, `execute <name> [arguments]` (integer and string arguments), `help` and `exit` (each with a
short form). Three AML interpreters are supported: the `acpi` crate (default), uACPI and ACPICA. Future work (other
argument types, `set`) is listed in `INTENT.md`.

## Tech Stack

* The tool is written in Rust, on the nightly toolchain (required by `acpi`; see `rust-toolchain.toml`).
* It uses the `acpi` crate, but not the `crates.io` published version - it uses the current `main` branch from GitHub.
* `aml-test-tools` (from the same GitHub repo, not on `crates.io`) provides `NullHandler` and `new_interpreter`.
* `rustyline` provides the REPL line editing.
* uACPI and ACPICA are C libraries, checked out as git submodules in `ext-libs/uACPI` and `ext-libs/acpica`. The
  `uacpi-sys` and `acpica-sys` workspace crates build them with the `cc` crate and generate Rust bindings with `bindgen`
  (which needs libclang). Each is behind a Cargo feature (`uacpi`, `acpica`), on by default; `--no-default-features`
  builds without a C toolchain or libclang.

## Layout

* `src/main.rs` - file loading, choosing the backend, starts the REPL.
* `src/cli.rs` - command-line parsing (`--acpi`, `--uacpi`, `--acpica`) and the prompt for each.
* `src/arguments.rs` - the `Argument` type (integer or string), the parser for the arguments typed after a method
  name, and the shared "wrong number of arguments" message. Interpreter-independent.
* `src/tables.rs` - splits a binary image into tables (36-byte SDT headers). Interpreter-independent.
* `src/backend.rs` - the `AmlBackend` trait and the backend-neutral `Entry`/`Value` types.
* `src/backend/acpi_crate.rs` - `AcpiCrateBackend`, built on the `acpi` crate's interpreter.
* `src/backend/uacpi.rs` - `UacpiBackend`, built on uACPI (feature `uacpi`), with `uacpi/host.rs` holding the
  `uacpi_kernel_*` functions uACPI calls back into.
* `src/backend/acpica.rs` - `AcpicaBackend`, built on ACPICA (feature `acpica`), with `acpica/host.rs` holding the
  `AcpiOs*` functions ACPICA calls back into, and a filter for its console output.
* `src/firmware.rs` - builds the RSDP, XSDT, FADT and FACS that the C interpreters start from, around the input's AML
  tables. Shared by `uacpi` and `acpica`; it uses its own struct layouts, which the tests check against each library's.
* `src/hostsim.rs` - simulated hardware and OS services (heap, mutexes, semaphores, port I/O, physical memory, timing)
  shared by the two host layers. Nothing touches the real machine.
* `uacpi-sys/`, `acpica-sys/` - workspace crates: `build.rs` compiles the C library and runs bindgen; `wrapper.h` lists
  the headers to bind. `acpica-sys/shim.c` formats ACPICA's printf-style output (Rust can't define C varargs).
* `src/render.rs` - tree rendering and `find` matching over `Entry` lists.
* `src/repl.rs` - command parsing/execution and the `rustyline` loop.
* `tests/common/mod.rs` - shared test helpers, including `compile_asl` (copies to a temp dir and runs `iasl`) and
  `backends()`.
* `tests/execute.rs` - end-to-end tests of `execute`, using `tests/aml/methods.asl` and `returns.asl`.
* `tests/cli.rs` - end-to-end tests using hand-assembled tables, driving the binary over stdin.
* `tests/aml/` - ASL sources used by tests. `iasl` is assumed to be on the path. Tests must copy an ASL file to a
  temporary directory and compile it there, so no compiler output lands in the crate's directories.

uACPI and ACPICA both keep global state, so only one instance of each can exist per process: test them through the
binary (the end-to-end tests), not in-process.

Most end-to-end tests run once per interpreter (`backends()`) and expect identical output from each. Where the
interpreters legitimately differ (uACPI and ACPICA can't show a mutex's sync level, or field and region details), the
tests avoid depending on that detail.

## Commands

* `cargo build`, `cargo test`, `cargo clippy --workspace --all-targets`, `cargo fmt`
* `cargo run -- [--acpi | --uacpi | --acpica] <file.aml>`
* `cargo build --no-default-features` - without the C interpreters (no C compiler or libclang needed). Try
  `--features uacpi` and `--features acpica` alone too, when changing feature-gated code.

## Windows notes

* The Visual Studio C++ build tools' `link.exe` must come before Git's `/usr/bin/link` on `PATH`. In Git Bash it
  usually doesn't, and Rust then fails to link build scripts with "extra operand" errors. PowerShell, or a Developer
  prompt, is fine.
* ACPICA's `acwin64.h` defines `snprintf` as a macro, which the current C runtime's `<stdio.h>` rejects. `acpica-sys`
  works around it by including `<stdio.h>` first (`/FIstdio.h`), and leaves out ACPICA's own copies of the C library's
  printf functions (`utprint.c`), which clash with the runtime's inline ones.
* A running `acpi-test.exe` stops cargo from replacing `target\debug\acpi-test.exe` ("failed to remove file").
