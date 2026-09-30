mod backend;
mod cli;
mod firmware;
mod hostsim;
mod render;
mod repl;
mod tables;

use backend::{AmlBackend, acpi_crate::AcpiCrateBackend};
use cli::{Interpreter, Parsed};
use std::process::ExitCode;

/// Split `bytes` into tables and hand the AML ones to the backend. Problems are printed and
/// loading carries on; the returned error only says that the namespace may be incomplete.
fn load(backend: &mut dyn AmlBackend, bytes: &[u8]) -> Result<(), String> {
    let mut aml = Vec::new();
    for table in tables::split_tables(bytes)? {
        if table.is_aml() {
            aml.push(table);
        } else {
            eprintln!("skipping non-AML table '{}'", table.signature);
        }
    }

    let problems = backend.load_tables(&aml);
    for problem in &problems {
        eprintln!("{problem}");
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err("some tables failed to load; the namespace may be incomplete".into())
    }
}

fn make_backend(interpreter: Interpreter) -> Result<Box<dyn AmlBackend>, String> {
    match interpreter {
        Interpreter::Acpi => Ok(Box::new(AcpiCrateBackend::new())),
        #[cfg(feature = "uacpi")]
        Interpreter::Uacpi => Ok(Box::new(backend::uacpi::UacpiBackend::new())),
        #[cfg(not(feature = "uacpi"))]
        Interpreter::Uacpi => {
            Err("uACPI support was not compiled in; rebuild with the `uacpi` feature".into())
        }
        #[cfg(feature = "acpica")]
        Interpreter::Acpica => Ok(Box::new(backend::acpica::AcpicaBackend::new())),
        #[cfg(not(feature = "acpica"))]
        Interpreter::Acpica => {
            Err("ACPICA support was not compiled in; rebuild with the `acpica` feature".into())
        }
    }
}

fn main() -> ExitCode {
    let options = match cli::parse(std::env::args().skip(1)) {
        Ok(Parsed::Run(options)) => options,
        Ok(Parsed::Help) => {
            print!("{}", cli::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("{e}\n\n{}", cli::USAGE);
            return ExitCode::from(2);
        }
    };
    let bytes = match std::fs::read(&options.path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {}: {e}", options.path);
            return ExitCode::FAILURE;
        }
    };
    let mut backend = match make_backend(options.interpreter) {
        Ok(backend) => backend,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = load(backend.as_mut(), &bytes) {
        eprintln!("{e}");
    }
    if let Err(e) = repl::run(backend.as_mut(), options.interpreter.prompt()) {
        eprintln!("input error: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
