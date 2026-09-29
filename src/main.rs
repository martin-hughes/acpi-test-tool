mod backend;
mod render;
mod repl;
mod tables;

use backend::{AmlBackend, acpi_crate::AcpiCrateBackend};
use std::process::ExitCode;

fn load(backend: &mut dyn AmlBackend, bytes: &[u8]) -> Result<(), String> {
    let mut failed = false;
    for table in tables::split_tables(bytes)? {
        if !table.is_aml() {
            eprintln!("skipping non-AML table '{}'", table.signature);
            continue;
        }
        if let Err(e) = backend.load_table(&table) {
            eprintln!("error loading {}: {e}", table.signature);
            failed = true;
        }
    }
    if failed {
        Err("some tables failed to load; the namespace may be incomplete".into())
    } else {
        Ok(())
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let [_, path] = args.as_slice() else {
        eprintln!("usage: acpi-test <file.aml>");
        return ExitCode::from(2);
    };
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut backend = AcpiCrateBackend::new();
    if let Err(e) = load(&mut backend, &bytes) {
        eprintln!("{e}");
    }
    if let Err(e) = repl::run(&mut backend) {
        eprintln!("input error: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
