//! The command interpreter and the interactive loop.

use crate::backend::AmlBackend;
use crate::render;
use rustyline::{DefaultEditor, error::ReadlineError};

#[derive(Debug, PartialEq)]
pub enum Command {
    Dump,
    Find(String),
    Execute(String),
    Help,
    Exit,
    Empty,
    Invalid(String),
}

pub fn parse(line: &str) -> Command {
    let mut words = line.split_whitespace();
    match (words.next(), words.next(), words.next()) {
        (None, ..) => Command::Empty,
        (Some("dump" | "d"), None, _) => Command::Dump,
        (Some("dump" | "d"), ..) => Command::Invalid("usage: dump".into()),
        (Some("find" | "f"), Some(name), None) => Command::Find(name.to_string()),
        (Some("find" | "f"), ..) => Command::Invalid("usage: find <name>".into()),
        (Some("execute" | "e"), Some(name), None) => Command::Execute(name.to_string()),
        (Some("execute" | "e"), ..) => Command::Invalid("usage: execute <method>".into()),
        (Some("help" | "h"), ..) => Command::Help,
        (Some("exit" | "quit" | "q"), ..) => Command::Exit,
        (Some(other), ..) => {
            Command::Invalid(format!("unknown command \"{other}\" (try \"help\")"))
        }
    }
}

const HELP: &str = "\
Commands:
  dump, d              print a tree of the namespace
  find, f <name>       find objects by absolute path (\\_SB_.PCI0) or by name (PCI0)
  execute, e <name>    run a method (with no arguments) and show its result
  help, h              show this text
  exit, quit, q        quit
";

/// Run one command, returning its output, or `None` to exit.
pub fn execute(backend: &mut dyn AmlBackend, command: Command) -> Option<String> {
    Some(match command {
        Command::Exit => return None,
        Command::Empty => String::new(),
        Command::Help => HELP.to_string(),
        Command::Invalid(msg) => format!("{msg}\n"),
        Command::Dump => render::render_tree(&backend.entries()),
        Command::Execute(name) => execute_method(backend, &name),
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
fn execute_method(backend: &mut dyn AmlBackend, name: &str) -> String {
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
        [method] => match backend.execute(&method.path) {
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

pub fn run(backend: &mut dyn AmlBackend) -> Result<(), ReadlineError> {
    let mut editor = DefaultEditor::new()?;
    loop {
        match editor.readline("acpi> ") {
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
        assert_eq!(parse("execute RET3"), Command::Execute("RET3".into()));
        assert_eq!(parse("quit"), Command::Exit);
        // Short forms follow the same argument rules.
        assert!(matches!(parse("f"), Command::Invalid(_)));
        assert!(matches!(parse("e a b"), Command::Invalid(_)));
        assert_eq!(parse("dump foo"), Command::Invalid("usage: dump".into()));
        assert_eq!(parse("d foo"), Command::Invalid("usage: dump".into()));
    }
}
