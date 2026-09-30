//! [`AmlBackend`] implemented on the `acpi` crate's AML interpreter.

use super::{AmlBackend, Entry, Value};
use crate::arguments::{Argument, wrong_argument_count};
use crate::tables::Table;
use acpi::aml::{
    Interpreter,
    namespace::{AmlName, Namespace, NamespaceLevelKind},
    object::{FieldUnitKind, Object},
};
use aml_test_tools::{handlers::null_handler::NullHandler, new_interpreter};
use std::str::FromStr;

pub struct AcpiCrateBackend {
    interpreter: Interpreter<NullHandler>,
}

impl AcpiCrateBackend {
    pub fn new() -> Self {
        Self {
            interpreter: new_interpreter(NullHandler),
        }
    }
}

impl AmlBackend for AcpiCrateBackend {
    fn load_tables(&mut self, tables: &[Table]) -> Vec<String> {
        tables
            .iter()
            .filter_map(|table| {
                self.interpreter
                    .load_table(table.body())
                    .err()
                    .map(|e| format!("error loading {}: {e:?}", table.signature))
            })
            .collect()
    }

    fn entries(&mut self) -> Vec<Entry> {
        let mut entries = Vec::new();
        let mut namespace = self.interpreter.namespace.lock();
        collect(&mut namespace, &mut entries);
        entries
    }

    fn execute(&mut self, path: &str, arguments: &[Argument]) -> Result<Value, String> {
        let name = AmlName::from_str(path).map_err(|e| format!("invalid path: {e:?}"))?;

        // The interpreter would reject a wrong count too, but wouldn't say what was expected.
        let expected = {
            let object = self
                .interpreter
                .namespace
                .lock()
                .get(name.clone())
                .map_err(|e| format!("{e:?}"))?;
            match &*object {
                Object::Method { flags, .. } | Object::NativeMethod { flags, .. } => {
                    Some(flags.arg_count())
                }
                _ => None,
            }
        };
        match expected {
            Some(expected) if expected != arguments.len() => {
                return Err(wrong_argument_count(expected, arguments.len()));
            }
            Some(_) => {}
            None => return Err("not a method".into()),
        }

        let arguments = arguments
            .iter()
            .map(|argument| match argument {
                Argument::Integer(value) => Object::Integer(*value).wrap(),
                Argument::String(text) => Object::String(text.clone()).wrap(),
            })
            .collect();
        let result = self
            .interpreter
            .evaluate(name, arguments)
            .map_err(|e| format!("{e:?}"))?;
        Ok(Value {
            type_name: format!("{:?}", result.typ()),
            summary: describe(&result),
        })
    }
}

fn collect(namespace: &mut Namespace, entries: &mut Vec<Entry>) {
    let result = namespace.traverse(|path, level| {
        if level.kind == NamespaceLevelKind::MethodLocals {
            return Ok(false);
        }
        let parent = path.as_string();
        let join = |seg: &str| {
            if parent == "\\" {
                format!("\\{seg}")
            } else {
                format!("{parent}.{seg}")
            }
        };

        for (seg, (flags, object)) in level.values.iter() {
            let mut summary = summarise(object);
            if flags.is_alias() {
                summary = format!("(alias) {summary}").trim_end().to_string();
            }
            entries.push(Entry {
                path: join(seg.as_str()),
                type_name: format!("{:?}", object.typ()),
                summary,
            });
        }
        // Scopes have a namespace level but no object of their own.
        for (seg, child) in level.children.iter() {
            if child.kind == NamespaceLevelKind::Scope && !level.values.contains_key(seg) {
                entries.push(Entry {
                    path: join(seg.as_str()),
                    type_name: "Scope".to_string(),
                    summary: String::new(),
                });
            }
        }
        Ok(true)
    });
    if let Err(e) = result {
        eprintln!("warning: namespace walk stopped early: {e:?}");
    }
}

fn summarise(object: &Object) -> String {
    match object {
        Object::Integer(v) => format!("{v:#x} ({v})"),
        Object::String(s) => format!("{s:?}"),
        Object::Buffer(bytes) => {
            let shown: Vec<String> = bytes.iter().take(16).map(|b| format!("{b:02x}")).collect();
            let more = if bytes.len() > 16 { " ..." } else { "" };
            format!("{} bytes: {}{more}", bytes.len(), shown.join(" "))
        }
        Object::Package(items) => format!("{} elements", items.len()),
        Object::Method { flags, .. } | Object::NativeMethod { flags, .. } => {
            format!(
                "{} args{}",
                flags.arg_count(),
                if flags.serialize() {
                    ", serialized"
                } else {
                    ""
                }
            )
        }
        Object::OpRegion(r) => format!("{:?} base={:#x} length={:#x}", r.space, r.base, r.length),
        Object::FieldUnit(f) => {
            let kind = match &f.kind {
                FieldUnitKind::Normal { .. } => "normal",
                FieldUnitKind::Bank { .. } => "bank",
                FieldUnitKind::Index { .. } => "index",
            };
            format!(
                "{kind} field, bit offset {} length {}",
                f.bit_index, f.bit_length
            )
        }
        Object::BufferField { offset, length, .. } => {
            format!("bit offset {offset} length {length}")
        }
        Object::Processor {
            proc_id,
            pblk_address,
            pblk_length,
        } => {
            format!("id {proc_id}, pblk {pblk_address:#x} length {pblk_length}")
        }
        Object::PowerResource {
            system_level,
            resource_order,
        } => {
            format!("system level {system_level}, order {resource_order}")
        }
        Object::Mutex { sync_level, .. } => format!("sync level {sync_level}"),
        Object::NamePath { name, .. } => crate::render::display_path(&name.as_string()),
        _ => String::new(),
    }
}

/// Like [`summarise`], but also looks inside references and packages. Used for method results.
fn describe(object: &Object) -> String {
    match object {
        Object::Reference { inner, .. } => describe(inner),
        Object::Package(items) => {
            let parts: Vec<String> = items
                .iter()
                .map(|item| {
                    format!("{:?} {}", item.typ(), describe(item))
                        .trim_end()
                        .to_string()
                })
                .collect();
            format!("{{ {} }}", parts.join(", "))
        }
        other => summarise(other),
    }
}
