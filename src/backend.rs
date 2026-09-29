//! The interface between the REPL and an AML interpreter. Future interpreters (uACPI, ACPICA)
//! implement [`AmlBackend`]; the rest of the tool only sees backend-neutral [`Entry`] values.

pub mod acpi_crate;

use crate::tables::Table;

/// One named object in the namespace.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Absolute path, e.g. `\_SB_.PCI0`. The root is `\`.
    pub path: String,
    /// Type name, e.g. `Device`, `Integer`, `Scope`.
    pub type_name: String,
    /// A short human-readable summary of the value; may be empty.
    pub summary: String,
}

pub trait AmlBackend {
    /// Load one AML table (DSDT/SSDT) into the namespace.
    fn load_table(&mut self, table: &Table) -> Result<(), String>;

    /// Every named object currently in the namespace, in no particular order.
    fn entries(&mut self) -> Vec<Entry>;
}
