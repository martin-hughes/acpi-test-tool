//! End-to-end tests: run the binary against a hand-assembled DSDT, driving the REPL via stdin.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

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

fn table(signature: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; 36];
    out[0..4].copy_from_slice(signature);
    out[4..8].copy_from_slice(&((36 + body.len()) as u32).to_le_bytes());
    out[8] = 2;
    out.extend_from_slice(body);
    out
}

fn fixture(name: &str, bytes: &[u8]) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// Returns (stdout, stderr, success).
fn run(file: &PathBuf, input: &str) -> (String, String, bool) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_acpi-test"))
        .arg(file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

#[test]
fn dump_and_find() {
    let file = fixture("simple.aml", &table(b"DSDT", AML));
    let (stdout, stderr, ok) = run(
        &file,
        "dump\nfind FOO_\nfind \\_SB_.DEV0\nfind BAR_\nfind NOPE\nexit\n",
    );
    assert!(ok, "stderr: {stderr}");
    assert!(
        stdout.contains("└── _SB_ [Scope]") || stdout.contains("├── _SB_ [Scope]"),
        "{stdout}"
    );
    assert!(stdout.contains("DEV0 [Device]"), "{stdout}");
    assert!(stdout.contains("FOO_ [Integer] 0x2a (42)"), "{stdout}");
    assert!(
        stdout.contains("\\_SB_.DEV0.FOO_ [Integer] 0x2a (42)"),
        "{stdout}"
    );
    assert!(stdout.contains("\\_SB_.DEV0 [Device]"), "{stdout}");
    assert!(stdout.contains("\\BAR_ [String] \"hi\""), "{stdout}");
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
    assert!(stdout.contains("\\_SB_.DEV0.FOO_"), "{stdout}");
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
