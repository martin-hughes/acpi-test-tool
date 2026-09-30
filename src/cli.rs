//! Command-line argument handling.

/// Which AML interpreter to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpreter {
    /// The `acpi` crate's interpreter (the default).
    Acpi,
    /// uACPI.
    Uacpi,
    /// ACPICA.
    Acpica,
}

impl Interpreter {
    /// The REPL prompt, which shows which interpreter is in use.
    pub fn prompt(self) -> &'static str {
        match self {
            Interpreter::Acpi => "acpi> ",
            Interpreter::Uacpi => "uacpi> ",
            Interpreter::Acpica => "acpica> ",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub interpreter: Interpreter,
    pub path: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Run(Options),
    Help,
}

pub const USAGE: &str = "\
usage: acpi-test [--acpi | --uacpi | --acpica] <file.aml>

  --acpi    use the `acpi` crate's AML interpreter (default)
  --uacpi   use the uACPI AML interpreter
  --acpica  use the ACPICA AML interpreter
  -h, --help  show this text
";

/// Parse the arguments that follow the program name.
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Parsed, String> {
    let mut interpreter = None;
    let mut path = None;
    for arg in args {
        let choice = match arg.as_str() {
            "-h" | "--help" => return Ok(Parsed::Help),
            "--acpi" => Some(Interpreter::Acpi),
            "--uacpi" => Some(Interpreter::Uacpi),
            "--acpica" => Some(Interpreter::Acpica),
            flag if flag.starts_with("--") => return Err(format!("unknown option \"{flag}\"")),
            _ => None,
        };
        match choice {
            Some(chosen) => match interpreter {
                Some(previous) if previous != chosen => {
                    return Err("only one of --acpi, --uacpi and --acpica may be given".into());
                }
                _ => interpreter = Some(chosen),
            },
            None => {
                if path.replace(arg).is_some() {
                    return Err("only one file may be given".into());
                }
            }
        }
    }
    let path = path.ok_or_else(|| "no file given".to_string())?;
    Ok(Parsed::Run(Options {
        interpreter: interpreter.unwrap_or(Interpreter::Acpi),
        path,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strs(args: &[&str]) -> Result<Parsed, String> {
        parse(args.iter().map(|s| s.to_string()))
    }

    fn run(interpreter: Interpreter, path: &str) -> Result<Parsed, String> {
        Ok(Parsed::Run(Options {
            interpreter,
            path: path.into(),
        }))
    }

    #[test]
    fn acpi_is_the_default() {
        assert_eq!(parse_strs(&["a.aml"]), run(Interpreter::Acpi, "a.aml"));
    }

    #[test]
    fn flags_choose_the_interpreter_in_either_position() {
        assert_eq!(
            parse_strs(&["--acpi", "a.aml"]),
            run(Interpreter::Acpi, "a.aml")
        );
        assert_eq!(
            parse_strs(&["--uacpi", "a.aml"]),
            run(Interpreter::Uacpi, "a.aml")
        );
        assert_eq!(
            parse_strs(&["a.aml", "--uacpi"]),
            run(Interpreter::Uacpi, "a.aml")
        );
        assert_eq!(
            parse_strs(&["--acpica", "a.aml"]),
            run(Interpreter::Acpica, "a.aml")
        );
    }

    #[test]
    fn repeating_the_same_flag_is_fine_but_conflicts_are_not() {
        assert_eq!(
            parse_strs(&["--uacpi", "--uacpi", "a.aml"]),
            run(Interpreter::Uacpi, "a.aml")
        );
        assert!(parse_strs(&["--acpi", "--uacpi", "a.aml"]).is_err());
        assert!(parse_strs(&["--uacpi", "--acpica", "a.aml"]).is_err());
        assert!(parse_strs(&["--acpica", "--acpi", "a.aml"]).is_err());
    }

    #[test]
    fn rejects_bad_arguments() {
        assert!(parse_strs(&[]).is_err());
        assert!(parse_strs(&["--uacpi"]).is_err());
        assert!(parse_strs(&["a.aml", "b.aml"]).is_err());
        assert!(parse_strs(&["--bogus", "a.aml"]).is_err());
    }

    #[test]
    fn the_prompt_names_the_interpreter() {
        assert_eq!(Interpreter::Acpi.prompt(), "acpi> ");
        assert_eq!(Interpreter::Uacpi.prompt(), "uacpi> ");
        assert_eq!(Interpreter::Acpica.prompt(), "acpica> ");
    }

    #[test]
    fn help() {
        assert_eq!(parse_strs(&["--help"]), Ok(Parsed::Help));
        assert_eq!(parse_strs(&["a.aml", "-h"]), Ok(Parsed::Help));
    }
}
