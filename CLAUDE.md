# ACPI Test Tool

This is a tool similar to `acpiexec`, that can load an AML file and presents a TUI for users to enter commands that
interrogate the ACPI namespace generated from the AML.

See `INTENT.md` for the plan, MVP definition and design decisions.

## Status

The MVP is implemented: `acpi-test <file>` loads a table file and offers a REPL with `dump`, `find <name>`,
`execute <name>` (methods with no arguments only), `help` and `exit`. Future work (other interpreters, method arguments,
`set`) is listed in `INTENT.md`.

## Tech Stack

* The tool is written in Rust, on the nightly toolchain (required by `acpi`; see `rust-toolchain.toml`).
* It uses the `acpi` crate, but not the `crates.io` published version - it uses the current `main` branch from GitHub.
* `aml-test-tools` (from the same GitHub repo, not on `crates.io`) provides `NullHandler` and `new_interpreter`.
* `rustyline` provides the REPL line editing.

## Layout

* `src/main.rs` - argument handling, file loading, starts the REPL.
* `src/tables.rs` - splits a binary image into tables (36-byte SDT headers). Interpreter-independent.
* `src/backend.rs` - the `AmlBackend` trait and the backend-neutral `Entry` type.
* `src/backend/acpi_crate.rs` - `AcpiCrateBackend`, built on the `acpi` crate's interpreter.
* `src/render.rs` - tree rendering and `find` matching over `Entry` lists.
* `src/repl.rs` - command parsing/execution and the `rustyline` loop.
* `tests/common/mod.rs` - shared test helpers, including `compile_asl` (copies to a temp dir and runs `iasl`).
* `tests/execute.rs` - end-to-end tests of `execute`, using `tests/aml/methods.asl`.
* `tests/cli.rs` - end-to-end tests using a hand-assembled DSDT, driving the binary over stdin.
* `tests/aml/` - ASL sources used by tests (e.g. `methods.asl`). `iasl` is assumed to be on the path. Tests must copy
  an ASL file to a temporary directory and compile it there, so no compiler output lands in the crate's directories.

## Commands

* `cargo build`, `cargo test`, `cargo clippy --all-targets`
* `cargo run -- <file.aml>`
