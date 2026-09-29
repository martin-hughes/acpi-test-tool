//! The command interpreter and the interactive loop.

use crate::backend::AmlBackend;
use crate::render;
use rustyline::{DefaultEditor, error::ReadlineError};

#[derive(Debug, PartialEq)]
pub enum Command {
    Dump,
    Find(String),
    Help,
    Exit,
    Empty,
    Invalid(String),
}

pub fn parse(line: &str) -> Command {
    let mut words = line.split_whitespace();
    match (words.next(), words.next(), words.next()) {
        (None, ..) => Command::Empty,
        (Some("dump"), None, _) => Command::Dump,
        (Some("find"), Some(name), None) => Command::Find(name.to_string()),
        (Some("find"), ..) => Command::Invalid("usage: find <name>".into()),
        (Some("help"), ..) => Command::Help,
        (Some("exit" | "quit"), ..) => Command::Exit,
        (Some(other), ..) => {
            Command::Invalid(format!("unknown command \"{other}\" (try \"help\")"))
        }
    }
}

const HELP: &str = "\
Commands:
  dump          print a tree of the namespace
  find <name>   find objects by absolute path (\\_SB_.PCI0) or by name (PCI0)
  help          show this text
  exit          quit
";

/// Run one command, returning its output, or `None` to exit.
pub fn execute(backend: &mut dyn AmlBackend, command: Command) -> Option<String> {
    Some(match command {
        Command::Exit => return None,
        Command::Empty => String::new(),
        Command::Help => HELP.to_string(),
        Command::Invalid(msg) => format!("{msg}\n"),
        Command::Dump => render::render_tree(&backend.entries()),
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
}
