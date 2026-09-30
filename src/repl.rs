//! The command interpreter and the interactive loop.

use crate::arguments::{Argument, parse_arguments};
use crate::backend::AmlBackend;
use crate::render;
use rustyline::{DefaultEditor, error::ReadlineError};

#[derive(Debug, PartialEq)]
pub enum Command {
    Dump,
    Find(String),
    Execute {
        name: String,
        arguments: Vec<Argument>,
    },
    Help,
    Exit,
    Empty,
    Invalid(String),
}

/// Split off the first whitespace-separated word, returning it and the rest of the text (with
/// its leading whitespace removed, but otherwise as it was: string arguments can contain spaces).
fn split_word(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    let (word, rest) = text.split_at(end);
    (word, rest.trim_start())
}

pub fn parse(line: &str) -> Command {
    let (command, rest) = split_word(line);
    match command {
        "" => Command::Empty,
        "dump" | "d" if rest.is_empty() => Command::Dump,
        "dump" | "d" => Command::Invalid("usage: dump".into()),
        "find" | "f" => {
            let mut words = rest.split_whitespace();
            match (words.next(), words.next()) {
                (Some(name), None) => Command::Find(name.to_string()),
                _ => Command::Invalid("usage: find <name>".into()),
            }
        }
        "execute" | "e" => parse_execute(rest),
        "help" | "h" => Command::Help,
        "exit" | "quit" | "q" => Command::Exit,
        other => Command::Invalid(format!("unknown command \"{other}\" (try \"help\")")),
    }
}

/// Parse what follows `execute`: a method name, then its arguments.
fn parse_execute(text: &str) -> Command {
    let (name, arguments) = split_word(text);
    if name.is_empty() {
        return Command::Invalid(EXECUTE_USAGE.into());
    }
    match parse_arguments(arguments) {
        Ok(arguments) => Command::Execute {
            name: name.to_string(),
            arguments,
        },
        Err(message) => Command::Invalid(message),
    }
}

const EXECUTE_USAGE: &str = "usage: execute <method> [arguments]";

const HELP: &str = "\
Commands:
  dump, d                        print a tree of the namespace
  find, f <name>                 find objects by absolute path (\\_SB_.PCI0) or by name (PCI0)
  execute, e <name> [arguments]  run a method and show its result
  help, h                        show this text
  exit, quit, q                  quit

Arguments to execute are separated by spaces. Each is an integer (decimal, or hexadecimal
with a leading 0x) or a string in double quotes, which can use Rust's escapes (\\n, \\\", ...).
";

/// Run one command, returning its output, or `None` to exit.
pub fn execute(backend: &mut dyn AmlBackend, command: Command) -> Option<String> {
    Some(match command {
        Command::Exit => return None,
        Command::Empty => String::new(),
        Command::Help => HELP.to_string(),
        Command::Invalid(msg) => format!("{msg}\n"),
        Command::Dump => render::render_tree(&backend.entries()),
        Command::Execute { name, arguments } => execute_method(backend, &name, &arguments),
        Command::Find(name) => {
            let entries = backend.entries();
            let found = render::find(&entries, &name);
            if found.is_empty() {
                format!("\"{name}\" not found\n")
            } else {
                render::render_found(&found)
            }
        }
    })
}

/// Resolve `name` to a single method, run it, and describe the outcome.
fn execute_method(backend: &mut dyn AmlBackend, name: &str, arguments: &[Argument]) -> String {
    let entries = backend.entries();
    let found = render::find(&entries, name);
    if found.is_empty() {
        return format!("\"{name}\" not found\n");
    }
    let methods: Vec<_> = found.iter().filter(|e| e.type_name == "Method").collect();
    match methods.as_slice() {
        [] => format!(
            "\"{name}\" is not a method:\n{}",
            render::render_found(&found)
        ),
        [method] => match backend.execute(&method.path, arguments) {
            Ok(value) => render::render_result(&method.path, &value),
            Err(e) => format!(
                "error executing {}: {e}\n",
                render::display_path(&method.path)
            ),
        },
        several => {
            let matches: Vec<&crate::backend::Entry> = several.iter().map(|e| **e).collect();
            format!(
                "\"{name}\" is ambiguous; use a full path:\n{}",
                render::render_found(&matches)
            )
        }
    }
}

pub fn run(backend: &mut dyn AmlBackend, prompt: &str) -> Result<(), ReadlineError> {
    let mut editor = DefaultEditor::new()?;
    loop {
        match editor.readline(prompt) {
            Ok(line) => {
                let _ = editor.add_history_entry(line.as_str());
                match execute(backend, parse(&line)) {
                    Some(output) => print!("{output}"),
                    None => return Ok(()),
                }
            }
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => return Ok(()),
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands() {
        assert_eq!(parse("dump"), Command::Dump);
        assert_eq!(parse("  find  _SB_ "), Command::Find("_SB_".into()));
        assert_eq!(parse("exit"), Command::Exit);
        assert_eq!(parse(""), Command::Empty);
        assert!(matches!(parse("find"), Command::Invalid(_)));
        assert!(matches!(parse("find a b"), Command::Invalid(_)));
        assert!(matches!(parse("bogus"), Command::Invalid(_)));
    }

    #[test]
    fn short_forms_are_synonyms_of_the_long_forms() {
        assert_eq!(parse("d"), parse("dump"));
        assert_eq!(parse("f _SB_"), parse("find _SB_"));
        assert_eq!(parse("e RET3"), parse("execute RET3"));
        assert_eq!(parse("h"), parse("help"));
        assert_eq!(parse("q"), parse("exit"));
        assert_eq!(parse("q"), parse("quit"));
        // The long forms still work.
        assert_eq!(parse("execute RET3"), execute_cmd("RET3", vec![]));
        assert_eq!(parse("quit"), Command::Exit);
        // Short forms follow the same argument rules.
        assert!(matches!(parse("f"), Command::Invalid(_)));
        assert!(matches!(parse("e a b"), Command::Invalid(_)));
        assert_eq!(parse("dump foo"), Command::Invalid("usage: dump".into()));
        assert_eq!(parse("d foo"), Command::Invalid("usage: dump".into()));
    }

    fn execute_cmd(name: &str, arguments: Vec<Argument>) -> Command {
        Command::Execute {
            name: name.into(),
            arguments,
        }
    }

    #[test]
    fn execute_takes_arguments_after_the_method_name() {
        assert_eq!(parse("execute RET3"), execute_cmd("RET3", vec![]));
        assert_eq!(
            parse("e ADD2 1 0x10"),
            execute_cmd("ADD2", vec![Argument::Integer(1), Argument::Integer(16)])
        );
        assert_eq!(
            parse("  e   CAT2   \"a b\"   \"c\"  "),
            execute_cmd(
                "CAT2",
                vec![Argument::String("a b".into()), Argument::String("c".into())]
            )
        );
        // The method name isn't an argument, even if it looks like a number.
        assert_eq!(
            parse("e 42 43"),
            execute_cmd("42", vec![Argument::Integer(43)])
        );
    }

    #[test]
    fn execute_needs_a_method_name_and_valid_arguments() {
        assert_eq!(
            parse("execute"),
            Command::Invalid("usage: execute <method> [arguments]".into())
        );
        assert_eq!(
            parse("e   "),
            Command::Invalid("usage: execute <method> [arguments]".into())
        );
        for bad in [
            "e M ff",
            "e M \"abc",
            "e M 1 unquoted",
            "e M -1",
            "e M \"a\"b",
        ] {
            let Command::Invalid(message) = parse(bad) else {
                panic!("{bad:?} should be invalid");
            };
            assert!(message.starts_with("argument "), "{bad:?}: {message}");
        }
    }
}
