//! End-to-end tests: run the binary against hand-assembled tables, driving the REPL via stdin.
//!
//! Tests that don't concern a particular interpreter run against each one this build supports.

mod common;

use common::{backends, fixture, run, run_with, table};
use std::path::PathBuf;

/// AML for:
///
/// ```asl
/// Scope (\_SB_) { Device (DEV0) { Name (FOO_, 0x2A) } }
/// Name (BAR_, "hi")
/// ```
const AML: &[u8] = &[
    0x10, 0x14, 0x5C, 0x5F, 0x53, 0x42, 0x5F, // Scope (\_SB_)
    0x5B, 0x82, 0x0C, b'D', b'E', b'V', b'0', // Device (DEV0)
    0x08, b'F', b'O', b'O', b'_', 0x0A, 0x2A, // Name (FOO_, 0x2A)
    0x08, b'B', b'A', b'R', b'_', 0x0D, b'h', b'i', 0x00, // Name (BAR_, "hi")
];

/// AML for `Name (SSDT, 7)`.
const SSDT_AML: &[u8] = &[0x08, b'S', b'S', b'D', b'T', 0x0A, 0x07];

#[test]
fn dump_and_find() {
    for backend in backends() {
        let file = fixture("simple.aml", &table(b"DSDT", AML));
        let (stdout, stderr, ok) = run_with(
            &[backend],
            &file,
            "dump\nfind FOO_\nfind \\_SB_.DEV0\nfind BAR_\nfind NOPE\nfind \\_SB.DEV0\nexit\n",
        );
        assert!(ok, "{backend}: stderr: {stderr}");
        assert!(
            stdout.contains("└── _SB [Scope]") || stdout.contains("├── _SB [Scope]"),
            "{backend}: {stdout}"
        );
        assert!(stdout.contains("DEV0 [Device]"), "{backend}: {stdout}");
        assert!(
            stdout.contains("FOO [Integer] 0x2a (42)"),
            "{backend}: {stdout}"
        );
        assert!(!stdout.contains("FOO_"), "{backend}: {stdout}");
        assert!(!stdout.contains("_SB_"), "{backend}: {stdout}");
        assert!(!stdout.contains("BAR_"), "{backend}: {stdout}");
        assert!(
            stdout.contains("\\_SB.DEV0.FOO [Integer] 0x2a (42)"),
            "{backend}: {stdout}"
        );
        // Queried once with the padded name and once with the displayed name.
        assert_eq!(
            stdout.matches("\\_SB.DEV0 [Device]").count(),
            2,
            "{backend}: {stdout}"
        );
        assert!(
            stdout.contains("\\BAR [String] \"hi\""),
            "{backend}: {stdout}"
        );
        assert!(stdout.contains("\"NOPE\" not found"), "{backend}: {stdout}");
    }
}

#[test]
fn multiple_tables_in_one_file() {
    for backend in backends() {
        let mut image = table(b"DSDT", AML);
        image.extend(table(b"SSDT", SSDT_AML));
        let file = fixture("multi.aml", &image);
        let (stdout, stderr, ok) = run_with(&[backend], &file, "find SSDT\nfind FOO_\nexit\n");
        assert!(ok, "{backend}: stderr: {stderr}");
        assert!(
            stdout.contains("\\SSDT [Integer] 0x7 (7)"),
            "{backend}: {stdout}"
        );
        assert!(stdout.contains("\\_SB.DEV0.FOO"), "{backend}: {stdout}");
    }
}

#[test]
fn a_file_with_only_an_ssdt_loads() {
    for backend in backends() {
        let file = fixture("ssdt-only.aml", &table(b"SSDT", SSDT_AML));
        let (stdout, stderr, ok) = run_with(&[backend], &file, "find SSDT\nexit\n");
        assert!(ok, "{backend}: stderr: {stderr}");
        assert!(stderr.is_empty(), "{backend}: {stderr}");
        assert!(
            stdout.contains("\\SSDT [Integer] 0x7 (7)"),
            "{backend}: {stdout}"
        );
    }
}

#[test]
fn a_second_dsdt_is_ignored_by_uacpi_and_loaded_by_acpi() {
    // The `acpi` crate simply runs every table it is given. uACPI needs a single DSDT, so it
    // reports the extra one.
    let mut image = table(b"DSDT", AML);
    image.extend(table(b"DSDT", SSDT_AML));
    let file = fixture("two-dsdts.aml", &image);

    let (stdout, stderr, ok) = run_with(&["--acpi"], &file, "find SSDT\nexit\n");
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.contains("\\SSDT [Integer]"), "{stdout}");

    if cfg!(feature = "uacpi") {
        let (stdout, stderr, ok) = run_with(&["--uacpi"], &file, "find SSDT\nexit\n");
        assert!(ok, "stderr: {stderr}");
        assert!(stderr.contains("extra DSDT"), "{stderr}");
        assert!(stdout.contains("\"SSDT\" not found"), "{stdout}");
    }
}

#[test]
fn bad_aml_is_reported_but_the_namespace_is_still_usable() {
    for backend in backends() {
        // A NameOp followed by garbage where its name should be.
        let file = fixture(
            "bad-aml.aml",
            &table(b"DSDT", &[0xFF, 0xFF, 0xFF, 0xFF, 0x08]),
        );
        let (stdout, stderr, ok) = run_with(&[backend], &file, "find _OSI\nexit\n");
        assert!(ok, "{backend}: stderr: {stderr}");
        assert!(
            stderr.contains("the namespace may be incomplete"),
            "{backend}: {stderr}"
        );
        // The predefined objects are there regardless.
        assert!(stdout.contains("\\_OSI [Method]"), "{backend}: {stdout}");
    }
}

#[test]
fn truncated_file_reports_error_and_still_exits_cleanly() {
    for backend in backends() {
        let mut bytes = table(b"DSDT", AML);
        bytes.truncate(bytes.len() - 5);
        let file = fixture("truncated.aml", &bytes);
        let (_, stderr, ok) = run_with(&[backend], &file, "exit\n");
        assert!(ok, "{backend}");
        assert!(stderr.contains("claims"), "{backend}: {stderr}");
    }
}

#[test]
fn missing_file_fails() {
    for backend in backends() {
        let (_, stderr, ok) = run_with(&[backend], &PathBuf::from("does-not-exist.aml"), "");
        assert!(!ok, "{backend}");
        assert!(stderr.contains("cannot read"), "{backend}: {stderr}");
    }
}

// ---- Choosing the interpreter ----

#[test]
fn acpi_is_the_default_interpreter() {
    // `_GL` shows a sync level under the `acpi` crate and nothing under uACPI, which tells the
    // two apart.
    let file = fixture("default.aml", &table(b"DSDT", AML));
    let (default, _, ok) = run(&file, "find _GL\nexit\n");
    assert!(ok);
    let (acpi, _, _) = run_with(&["--acpi"], &file, "find _GL\nexit\n");
    assert_eq!(default, acpi);
    assert!(default.contains("sync level"), "{default}");
}

#[test]
#[cfg(feature = "uacpi")]
fn uacpi_can_be_selected() {
    let file = fixture("choose-uacpi.aml", &table(b"DSDT", AML));
    let (stdout, stderr, ok) = run_with(&["--uacpi"], &file, "find _GL\nexit\n");
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.contains("\\_GL [Mutex]"), "{stdout}");
    assert!(!stdout.contains("sync level"), "{stdout}");
}

#[test]
#[cfg(not(feature = "uacpi"))]
fn uacpi_is_refused_when_not_compiled_in() {
    let file = fixture("no-uacpi.aml", &table(b"DSDT", AML));
    let (_, stderr, ok) = run_with(&["--uacpi"], &file, "exit\n");
    assert!(!ok);
    assert!(stderr.contains("not compiled in"), "{stderr}");
}

#[test]
fn conflicting_interpreter_flags_are_an_error() {
    let file = fixture("conflict.aml", &table(b"DSDT", AML));
    let (stdout, stderr, ok) = run_with(&["--acpi", "--uacpi"], &file, "dump\n");
    assert!(!ok);
    assert!(stderr.contains("cannot be used together"), "{stderr}");
    assert!(stderr.contains("usage:"), "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
}

#[test]
fn unknown_option_is_an_error() {
    let file = fixture("unknown-option.aml", &table(b"DSDT", AML));
    let (_, stderr, ok) = run_with(&["--bogus"], &file, "");
    assert!(!ok);
    assert!(stderr.contains("unknown option"), "{stderr}");
}

#[test]
fn help_shows_usage() {
    let (stdout, _, ok) = run_with(&["--help"], &PathBuf::from("unused"), "");
    assert!(ok);
    assert!(stdout.contains("--uacpi"), "{stdout}");
}
