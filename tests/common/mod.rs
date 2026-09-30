//! Helpers shared by the integration tests.
#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

/// A number that is unique within this test process, for naming scratch files.
pub fn next_id() -> usize {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// Build an ACPI table (36-byte header, checksum not computed) around `body`.
pub fn table(signature: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; 36];
    out[0..4].copy_from_slice(signature);
    out[4..8].copy_from_slice(&((36 + body.len()) as u32).to_le_bytes());
    out[8] = 2;
    out.extend_from_slice(body);
    out
}

/// Write `bytes` to a file called `name` in cargo's per-target scratch directory.
pub fn fixture(name: &str, bytes: &[u8]) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// A uniquely named temporary directory that is removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "acpi-test-{}-{}-{label}",
            std::process::id(),
            next_id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Compile `tests/aml/<name>.asl` with `iasl` and return the resulting table bytes.
///
/// The source is copied to a temporary directory and compiled there, so no compiler output is
/// left in the crate's directories.
pub fn compile_asl(name: &str) -> Vec<u8> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("aml")
        .join(format!("{name}.asl"));
    let dir = TempDir::new(name);
    let copy = dir.path().join(format!("{name}.asl"));
    std::fs::copy(&source, &copy).unwrap_or_else(|e| panic!("cannot copy {source:?}: {e}"));

    let output = Command::new("iasl")
        .arg(format!("{name}.asl"))
        .current_dir(dir.path())
        .output()
        .expect("failed to run iasl; it must be on the path");
    assert!(
        output.status.success(),
        "iasl failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::read(dir.path().join(format!("{name}.aml"))).unwrap()
}

/// The command-line flags that select each interpreter this build supports. Tests that should
/// hold for every interpreter loop over these.
pub fn backends() -> Vec<&'static str> {
    let mut flags = vec!["--acpi"];
    if cfg!(feature = "uacpi") {
        flags.push("--uacpi");
    }
    flags
}

/// Run the tool on `file` with the default interpreter, feeding `input` to the REPL. Returns
/// (stdout, stderr, success).
pub fn run(file: &Path, input: &str) -> (String, String, bool) {
    run_with(&[], file, input)
}

/// Like [`run`], with extra command-line arguments (such as an interpreter flag) before the file.
pub fn run_with(args: &[&str], file: &Path, input: &str) -> (String, String, bool) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_acpi-test"))
        .args(args)
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
