//! End-to-end tests of the `execute` command, using `tests/aml/methods.asl`.

mod common;

use common::{compile_asl, fixture, next_id, run};

fn execute(commands: &str) -> String {
    let file = fixture(
        &format!("methods-{}.aml", next_id()),
        &compile_asl("methods"),
    );
    let (stdout, stderr, ok) = run(&file, &format!("{commands}exit\n"));
    assert!(ok, "stderr: {stderr}");
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
    stdout
}

#[test]
fn returns_a_constant() {
    let stdout = execute("execute RET3\n");
    assert!(
        stdout.contains("\\RET3 returned [Integer] 0x3 (3)"),
        "{stdout}"
    );
}

#[test]
fn returns_zero() {
    let stdout = execute("execute MAIN\n");
    assert!(
        stdout.contains("\\MAIN returned [Integer] 0x0 (0)"),
        "{stdout}"
    );
}

#[test]
fn short_command_forms_work() {
    let stdout = execute("e RET3\nf RET3\nd\nh\n");
    assert!(
        stdout.contains("\\RET3 returned [Integer] 0x3 (3)"),
        "{stdout}"
    );
    assert!(stdout.contains("\\RET3 [Method]"), "{stdout}");
    assert!(
        stdout.contains("└── RET3") || stdout.contains("├── RET3"),
        "{stdout}"
    );
    assert!(stdout.contains("Commands:"), "{stdout}");
}

#[test]
fn q_exits_the_repl() {
    let file = fixture(
        &format!("methods-{}.aml", next_id()),
        &compile_asl("methods"),
    );
    // Anything after `q` must not be executed.
    let (stdout, _, ok) = run(&file, "q\nexecute RET3\n");
    assert!(ok);
    assert!(!stdout.contains("returned"), "{stdout}");
}

#[test]
fn accepts_absolute_paths() {
    let stdout = execute("execute \\RET3\n");
    assert!(
        stdout.contains("\\RET3 returned [Integer] 0x3 (3)"),
        "{stdout}"
    );
}

#[test]
fn method_calling_another_method_with_no_return() {
    // NORT calls MAIN() but has no explicit Return, so the result is an uninitialized object.
    let stdout = execute("execute NORT\n");
    assert!(
        stdout.contains("\\NORT returned [Uninitialized]"),
        "{stdout}"
    );
}

#[test]
fn can_execute_repeatedly() {
    let stdout = execute("execute RET3\nexecute RET3\nexecute MAIN\n");
    assert_eq!(stdout.matches("returned [Integer]").count(), 3, "{stdout}");
}

#[test]
fn execution_does_not_disturb_the_namespace() {
    let before = execute("dump\n");
    let after = execute("execute RET3\nexecute NORT\ndump\n");
    let tree = |s: &str| {
        s.lines()
            .skip_while(|l| *l != "\\")
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(tree(&after).ends_with(&tree(&before)), "{after}");
}

#[test]
fn unknown_name_is_reported() {
    let stdout = execute("execute NOPE\n");
    assert!(stdout.contains("\"NOPE\" not found"), "{stdout}");
}

#[test]
fn non_method_is_rejected() {
    let stdout = execute("execute _SB_\n");
    assert!(stdout.contains("is not a method"), "{stdout}");
}

#[test]
fn missing_argument_is_a_usage_error() {
    let stdout = execute("execute\n");
    assert!(stdout.contains("usage: execute <method>"), "{stdout}");
}
