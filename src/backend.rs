//! The interface between the REPL and an AML interpreter. Each interpreter (the `acpi` crate,
//! uACPI and ACPICA) implements [`AmlBackend`]; the rest of the tool only sees backend-neutral
//! [`Entry`] and [`Value`] values.

pub mod acpi_crate;
#[cfg(feature = "acpica")]
pub mod acpica;
#[cfg(feature = "uacpi")]
pub mod uacpi;

use crate::arguments::Argument;
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

/// The value produced by executing a method.
#[derive(Clone, Debug, PartialEq)]
pub struct Value {
    /// Type name, e.g. `Integer`, `String`, `Package`.
    pub type_name: String,
    /// A human-readable rendering of the value; may be empty.
    pub summary: String,
}

pub trait AmlBackend {
    /// Load AML tables (DSDT/SSDT) into the namespace. All tables are given at once because some
    /// interpreters (uACPI) need the complete set before they can build a namespace.
    ///
    /// Returns a message for each problem found. Loading continues past errors where the
    /// interpreter allows it, so the namespace may still be partly populated.
    fn load_tables(&mut self, tables: &[Table]) -> Vec<String>;

    /// Every named object currently in the namespace, in no particular order.
    fn entries(&mut self) -> Vec<Entry>;

    /// Execute the method at the absolute `path`, with the given arguments, and return its
    /// result. A method called with the wrong number of arguments is an error (see
    /// [`crate::arguments::wrong_argument_count`] for its message).
    fn execute(&mut self, path: &str, arguments: &[Argument]) -> Result<Value, String>;
}
