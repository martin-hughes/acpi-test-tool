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

/// Join command lines into what is typed at the prompt.
fn typed(lines: &[&str]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

#[test]
fn integer_arguments_are_passed_to_the_method() {
    assert_output_contains(
        "returns",
        &typed(&[
            "execute TAK1 42",
            "execute TAK1 0x2a",
            "execute TAK1 0",
            "execute ADD2 40 2",
            "e ADD2 0x10 16",
        ]),
        &[
            "\\TAK1 returned [Integer] 0x2a (42)",
            "\\TAK1 returned [Integer] 0x0 (0)",
            "\\ADD2 returned [Integer] 0x2a (42)",
            "\\ADD2 returned [Integer] 0x20 (32)",
        ],
    );
}

#[test]
fn hexadecimal_and_decimal_mean_what_they_say() {
    for backend in backends() {
        let stdout = execute_with(backend, "returns", &typed(&["e TAK1 10", "e TAK1 0x10"]));
        assert!(stdout.contains("[Integer] 0xa (10)"), "{backend}: {stdout}");
        assert!(
            stdout.contains("[Integer] 0x10 (16)"),
            "{backend}: {stdout}"
        );
    }
}

#[test]
fn the_largest_integer_can_be_passed() {
    assert_output_contains(
        "returns",
        &typed(&["e TAK1 18446744073709551615", "e TAK1 0xffffffffffffffff"]),
        &["\\TAK1 returned [Integer] 0xffffffffffffffff (18446744073709551615)"],
    );
}

#[test]
fn string_arguments_are_passed_to_the_method() {
    assert_output_contains(
        "returns",
        &typed(&[
            r#"execute TAK1 "hello""#,
            r#"e TAK1 "two words""#,
            r#"e TAK1 """#,
            r#"e CAT2 "foo" "bar""#,
        ]),
        &[
            "\\TAK1 returned [String] \"hello\"",
            "\\TAK1 returned [String] \"two words\"",
            "\\TAK1 returned [String] \"\"",
            "\\CAT2 returned [String] \"foobar\"",
        ],
    );
}

#[test]
fn string_arguments_can_use_escapes() {
    assert_output_contains(
        "returns",
        &typed(&[
            r#"e TAK1 "say \"hi\"\tnow""#,
            r#"e TAK1 "\x41\u{42}C""#,
            r#"e TAK1 "back\\slash""#,
        ]),
        &[
            // The result is shown with the same escapes (it is printed with Rust's `{:?}`).
            r#"[String] "say \"hi\"\tnow""#,
            r#"[String] "ABC""#,
            r#"[String] "back\\slash""#,
        ],
    );
}

#[test]
fn a_method_called_with_the_wrong_number_of_arguments_is_refused() {
    assert_output_contains(
        "returns",
        &typed(&[
            "execute TAK1",
            "execute TAK1 1 2",
            "execute ADD2 1",
            "execute ADD2 1 2 3",
            "execute STR1 1",
        ]),
        &[
            "error executing \\TAK1: the method takes 1 argument, but 0 were given",
            "error executing \\TAK1: the method takes 1 argument, but 2 were given",
            "error executing \\ADD2: the method takes 2 arguments, but 1 was given",
            "error executing \\ADD2: the method takes 2 arguments, but 3 were given",
            "error executing \\STR1: the method takes no arguments, but 1 was given",
        ],
    );
}

#[test]
fn an_argument_is_refused_by_a_method_that_takes_none() {
    assert_output_contains(
        "methods",
        &typed(&["execute MAIN 1"]),
        &["error executing \\MAIN: the method takes no arguments, but 1 was given"],
    );
}

#[test]
fn invalid_arguments_are_reported_and_nothing_is_executed() {
    for backend in backends() {
        for (typed_line, complaint) in [
            ("e TAK1 ff", "argument 1: \"ff\" is not a valid argument"),
            (
                "e TAK1 hello",
                "argument 1: \"hello\" is not a valid argument",
            ),
            ("e TAK1 -1", "argument 1: \"-1\" is not a valid argument"),
            ("e TAK1 0x", "argument 1: \"0x\" is not a valid argument"),
            (
                "e TAK1 0xzz",
                "argument 1: \"0xzz\" is not a valid argument",
            ),
            ("e TAK1 1.5", "argument 1: \"1.5\" is not a valid argument"),
            (
                "e TAK1 18446744073709551616",
                "argument 1: \"18446744073709551616\" is too large",
            ),
            (
                r#"e TAK1 "abc"#,
                "argument 1: the string is missing its closing quote",
            ),
            (
                r#"e ADD2 1 "a"b"#,
                "argument 2: a string must be followed by a space",
            ),
            (r#"e TAK1 "\q""#, "argument 1: unknown escape \\q"),
            (
                r#"e TAK1 "\x80""#,
                "argument 1: \\x escapes must be at most \\x7F",
            ),
            (
                r#"e TAK1 "caf\u{e9}""#,
                "argument 1: strings must be ASCII and cannot contain NUL",
            ),
        ] {
            let stdout = execute_with(backend, "returns", &typed(&[typed_line]));
            assert!(
                stdout.contains(complaint),
                "{backend}: {typed_line:?}: expected {complaint:?} in:\n{stdout}"
            );
            assert!(
                !stdout.contains("returned"),
                "{backend}: {typed_line:?} should not have run:\n{stdout}"
            );
        }
    }
}

#[test]
fn a_bad_argument_is_reported_even_for_an_unknown_method() {
    // Arguments are checked before the method is looked up.
    assert_output_contains(
        "returns",
        &typed(&["e NOPE ff"]),
        &["argument 1: \"ff\" is not a valid argument"],
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
