//! End-to-end tests: run the binary against a hand-assembled DSDT, driving the REPL via stdin.

mod common;

use common::{fixture, run, table};
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

#[test]
fn dump_and_find() {
    let file = fixture("simple.aml", &table(b"DSDT", AML));
    let (stdout, stderr, ok) = run(
        &file,
        "dump\nfind FOO_\nfind \\_SB_.DEV0\nfind BAR_\nfind NOPE\nfind \\_SB.DEV0\nexit\n",
    );
    assert!(ok, "stderr: {stderr}");
    assert!(
        stdout.contains("└── _SB [Scope]") || stdout.contains("├── _SB [Scope]"),
        "{stdout}"
    );
    assert!(stdout.contains("DEV0 [Device]"), "{stdout}");
    assert!(stdout.contains("FOO [Integer] 0x2a (42)"), "{stdout}");
    assert!(!stdout.contains("FOO_"), "{stdout}");
    assert!(!stdout.contains("_SB_"), "{stdout}");
    assert!(!stdout.contains("BAR_"), "{stdout}");
    assert!(
        stdout.contains("\\_SB.DEV0.FOO [Integer] 0x2a (42)"),
        "{stdout}"
    );
    // Queried once with the padded name and once with the displayed name.
    assert_eq!(stdout.matches("\\_SB.DEV0 [Device]").count(), 2, "{stdout}");
    assert!(stdout.contains("\\BAR [String] \"hi\""), "{stdout}");
    assert!(stdout.contains("\"NOPE\" not found"), "{stdout}");
}

#[test]
fn multiple_tables_in_one_file() {
    let mut image = table(b"DSDT", AML);
    image.extend(table(b"SSDT", &[0x08, b'S', b'S', b'D', b'T', 0x0A, 0x07]));
    let file = fixture("multi.aml", &image);
    let (stdout, stderr, ok) = run(&file, "find SSDT\nfind FOO_\nexit\n");
    assert!(ok, "stderr: {stderr}");
    assert!(stdout.contains("\\SSDT [Integer] 0x7 (7)"), "{stdout}");
    assert!(stdout.contains("\\_SB.DEV0.FOO"), "{stdout}");
}

#[test]
fn truncated_file_reports_error_and_still_exits_cleanly() {
    let mut bytes = table(b"DSDT", AML);
    bytes.truncate(bytes.len() - 5);
    let file = fixture("truncated.aml", &bytes);
    let (_, stderr, ok) = run(&file, "exit\n");
    assert!(ok);
    assert!(stderr.contains("claims"), "{stderr}");
}

#[test]
fn missing_file_fails() {
    let (_, stderr, ok) = run(&PathBuf::from("does-not-exist.aml"), "");
    assert!(!ok);
    assert!(stderr.contains("cannot read"), "{stderr}");
}
