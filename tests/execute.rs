//! End-to-end tests of the `execute` command, using `tests/aml/methods.asl` and `returns.asl`.
//!
//! Every test runs against each interpreter this build supports, and expects the same output
//! from all of them.

mod common;

use common::{backends, compile_asl, fixture, next_id, run_with};

/// Run `commands` (followed by `exit`) against the compiled ASL file `asl` using the interpreter
/// selected by `backend`, and return what was printed to stdout.
fn execute_with(backend: &str, asl: &str, commands: &str) -> String {
    let file = fixture(&format!("{asl}-{}.aml", next_id()), &compile_asl(asl));
    let (stdout, stderr, ok) = run_with(&[backend], &file, &format!("{commands}exit\n"));
    assert!(ok, "{backend}: stderr: {stderr}");
    assert!(stderr.is_empty(), "{backend}: unexpected stderr: {stderr}");
    stdout
}

/// Check that `commands`, run on `asl` under every interpreter, print a line containing each
/// of `expected`.
fn assert_output_contains(asl: &str, commands: &str, expected: &[&str]) {
    for backend in backends() {
        let stdout = execute_with(backend, asl, commands);
        for text in expected {
            assert!(
                stdout.contains(text),
                "{backend}: expected {text:?} in:\n{stdout}"
            );
        }
    }
}

#[test]
fn returns_a_constant() {
    assert_output_contains(
        "methods",
        "execute RET3\n",
        &["\\RET3 returned [Integer] 0x3 (3)"],
    );
}

#[test]
fn returns_zero() {
    assert_output_contains(
        "methods",
        "execute MAIN\n",
        &["\\MAIN returned [Integer] 0x0 (0)"],
    );
}

#[test]
fn short_command_forms_work() {
    assert_output_contains(
        "methods",
        "e RET3\nf RET3\nd\nh\n",
        &[
            "\\RET3 returned [Integer] 0x3 (3)",
            "\\RET3 [Method]",
            "RET3 [Method]",
            "Commands:",
        ],
    );
}

#[test]
fn q_exits_the_repl() {
    for backend in backends() {
        let file = fixture(
            &format!("methods-{}.aml", next_id()),
            &compile_asl("methods"),
        );
        // Anything after `q` must not be executed.
        let (stdout, _, ok) = run_with(&[backend], &file, "q\nexecute RET3\n");
        assert!(ok, "{backend}");
        assert!(!stdout.contains("returned"), "{backend}: {stdout}");
    }
}

#[test]
fn accepts_absolute_paths() {
    assert_output_contains(
        "methods",
        "execute \\RET3\n",
        &["\\RET3 returned [Integer] 0x3 (3)"],
    );
}

#[test]
fn method_calling_another_method_with_no_return() {
    // NORT calls MAIN() but has no explicit Return, so the result is an uninitialized object.
    assert_output_contains(
        "methods",
        "execute NORT\n",
        &["\\NORT returned [Uninitialized]"],
    );
}

#[test]
fn can_execute_repeatedly() {
    for backend in backends() {
        let stdout = execute_with(
            backend,
            "methods",
            "execute RET3\nexecute RET3\nexecute MAIN\n",
        );
        assert_eq!(
            stdout.matches("returned [Integer]").count(),
            3,
            "{backend}: {stdout}"
        );
    }
}

#[test]
fn execution_does_not_disturb_the_namespace() {
    let tree = |s: &str| {
        s.lines()
            .skip_while(|l| *l != "\\")
            .collect::<Vec<_>>()
            .join("\n")
    };
    for backend in backends() {
        let before = execute_with(backend, "methods", "dump\n");
        let after = execute_with(backend, "methods", "execute RET3\nexecute NORT\ndump\n");
        assert!(tree(&after).ends_with(&tree(&before)), "{backend}: {after}");
    }
}

#[test]
fn unknown_name_is_reported() {
    assert_output_contains("methods", "execute NOPE\n", &["\"NOPE\" not found"]);
}

#[test]
fn non_method_is_rejected() {
    assert_output_contains("methods", "execute _SB_\n", &["is not a method"]);
}

#[test]
fn missing_argument_is_a_usage_error() {
    assert_output_contains("methods", "execute\n", &["usage: execute <method>"]);
}

#[test]
fn returns_each_kind_of_value() {
    assert_output_contains(
        "returns",
        "e STR1\ne BUF1\ne PKG1\n",
        &[
            "\\STR1 returned [String] \"hello\"",
            "\\BUF1 returned [Buffer] 3 bytes: 01 02 03",
            "\\PKG1 returned [Package] { Integer 0x1 (1), String \"two\", Buffer 1 bytes: 04 }",
        ],
    );
}

#[test]
fn a_method_that_needs_arguments_is_refused() {
    assert_output_contains(
        "returns",
        "execute TAK1\n",
        &["takes arguments, which are not supported yet"],
    );
}

#[test]
fn methods_are_listed_with_their_argument_counts() {
    assert_output_contains(
        "returns",
        "find TAK1\nfind STR1\n",
        &["\\TAK1 [Method] 1 args", "\\STR1 [Method] 0 args"],
    );
}
