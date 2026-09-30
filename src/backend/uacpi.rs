//! [`AmlBackend`] implemented on [uACPI](https://github.com/uACPI/uACPI).
//!
//! uACPI is a C library with global state, so only one [`UacpiBackend`] can exist in a process
//! at a time. `firmware` builds the tables uACPI is started from, and `host` provides the
//! functions uACPI calls back into.

mod firmware;
mod host;

use super::{AmlBackend, Entry, Value};
use crate::tables::Table;
use firmware::Firmware;
use std::ffi::{CStr, CString, c_void};
use std::ptr::{null, null_mut};
use uacpi_sys as sys;

pub struct UacpiBackend {
    /// The tables uACPI reads from. They must stay alive, at the same addresses, until uACPI is
    /// reset.
    firmware: Option<Firmware>,
    /// Whether the namespace was loaded, and so can be walked.
    loaded: bool,
}

impl UacpiBackend {
    pub fn new() -> Self {
        Self {
            firmware: None,
            loaded: false,
        }
    }
}

impl Drop for UacpiBackend {
    fn drop(&mut self) {
        if self.firmware.is_some() {
            // SAFETY: uACPI was initialised by `load_tables`, and nothing uses it after this.
            unsafe { sys::uacpi_state_reset() };
            host::reset();
        }
    }
}

fn status_text(status: sys::uacpi_status) -> String {
    // SAFETY: uACPI returns a pointer to a static string for any status value.
    let text = unsafe { CStr::from_ptr(sys::uacpi_status_to_string(status)) };
    text.to_string_lossy().into_owned()
}

impl AmlBackend for UacpiBackend {
    fn load_tables(&mut self, tables: &[Table]) -> Vec<String> {
        let mut problems = Vec::new();
        if self.firmware.is_some() {
            problems.push("tables have already been loaded".to_string());
            return problems;
        }

        let firmware = Firmware::build(tables, &mut problems);
        host::install_firmware(firmware.rsdp_address(), firmware.address_ranges());
        self.firmware = Some(firmware);

        // SAFETY: the host functions uACPI needs are defined in `host`, and the tables it will
        // read are kept alive by `self.firmware`.
        unsafe {
            // Only warnings and errors are worth showing; the rest is uACPI narrating its setup.
            sys::uacpi_context_set_log_level(sys::uacpi_log_level_UACPI_LOG_WARN);

            // Not entering ACPI mode means uACPI never touches the (simulated) hardware registers.
            let status = sys::uacpi_initialize(sys::UACPI_FLAG_NO_ACPI_MODE as u64);
            if status != sys::uacpi_status_UACPI_STATUS_OK {
                problems.push(format!(
                    "uACPI failed to initialise: {}",
                    status_text(status)
                ));
                return problems;
            }

            // This parses the AML, building the namespace. It deliberately stops short of
            // `uacpi_namespace_initialize`, which would run _STA and _INI methods.
            let status = sys::uacpi_namespace_load();
            if status != sys::uacpi_status_UACPI_STATUS_OK {
                problems.push(format!(
                    "uACPI failed to load the namespace: {}",
                    status_text(status)
                ));
                return problems;
            }
        }
        self.loaded = true;

        // Errors in the AML itself don't fail the calls above; uACPI logs them (already shown on
        // stderr) and moves on to the next table.
        let logged = host::take_logged_errors();
        if logged > 0 {
            problems.push(format!(
                "uACPI logged {logged} error(s) while loading; the namespace may be incomplete"
            ));
        }
        problems
    }

    fn entries(&mut self) -> Vec<Entry> {
        if !self.loaded {
            return Vec::new();
        }
        let mut entries: Vec<Entry> = Vec::new();
        // SAFETY: the callback only uses `user` as the `Vec` passed here, which outlives the call.
        let status = unsafe {
            sys::uacpi_namespace_for_each_child_simple(
                sys::uacpi_namespace_root(),
                Some(visit_node),
                &mut entries as *mut Vec<Entry> as *mut c_void,
            )
        };
        if status != sys::uacpi_status_UACPI_STATUS_OK {
            eprintln!(
                "warning: namespace walk stopped early: {}",
                status_text(status)
            );
        }
        entries
    }

    fn execute(&mut self, path: &str) -> Result<Value, String> {
        if !self.loaded {
            return Err("no namespace is loaded".into());
        }
        let path = CString::new(path).map_err(|_| "invalid path".to_string())?;

        // SAFETY: `path` is a valid C string for the whole block, and the object returned by
        // `uacpi_eval` is released before the block ends.
        unsafe {
            let mut node: *mut sys::uacpi_namespace_node = null_mut();
            let status = sys::uacpi_namespace_node_find(null_mut(), path.as_ptr(), &mut node);
            if status != sys::uacpi_status_UACPI_STATUS_OK {
                return Err(status_text(status));
            }
            if method_arguments(node) != Some(0) {
                return Err("the method takes arguments, which are not supported yet".into());
            }

            let mut result: *mut sys::uacpi_object = null_mut();
            let status = sys::uacpi_eval(null_mut(), path.as_ptr(), null(), &mut result);
            if status != sys::uacpi_status_UACPI_STATUS_OK {
                return Err(status_text(status));
            }
            if result.is_null() {
                // The method finished without returning anything.
                return Ok(Value {
                    type_name: "Uninitialized".into(),
                    summary: String::new(),
                });
            }
            let value = Value {
                type_name: type_name(sys::uacpi_object_get_type(result)).into(),
                summary: describe(result),
            };
            sys::uacpi_object_unref(result);
            Ok(value)
        }
    }
}

/// The number of arguments the method at `node` takes, or `None` if it isn't a method.
///
/// # Safety
/// `node` must be a valid namespace node.
unsafe fn method_arguments(node: *mut sys::uacpi_namespace_node) -> Option<u8> {
    let mut info: *mut sys::uacpi_namespace_node_info = null_mut();
    // This only evaluates other objects (_HID and so on) for devices and processors, never for
    // the methods it is used on here.
    let status = unsafe { sys::uacpi_get_namespace_node_info(node, &mut info) };
    if status != sys::uacpi_status_UACPI_STATUS_OK {
        return None;
    }
    let info_ref = unsafe { &*info };
    let arguments = (info_ref.type_ == sys::uacpi_object_type_UACPI_OBJECT_METHOD)
        .then_some(info_ref.num_params);
    unsafe { sys::uacpi_free_namespace_node_info(info) };
    arguments
}

/// Called by uACPI for every node in the namespace. `user` is a `*mut Vec<Entry>`.
unsafe extern "C" fn visit_node(
    user: *mut c_void,
    node: *mut sys::uacpi_namespace_node,
    _depth: sys::uacpi_u32,
) -> sys::uacpi_iteration_decision {
    let entries = unsafe { &mut *(user as *mut Vec<Entry>) };
    let object = unsafe { sys::uacpi_namespace_node_get_object(node) };
    if object.is_null() {
        return sys::uacpi_iteration_decision_UACPI_ITERATION_DECISION_CONTINUE;
    }

    let path = unsafe {
        let generated = sys::uacpi_namespace_node_generate_absolute_path(node);
        let path = CStr::from_ptr(generated).to_string_lossy().into_owned();
        sys::uacpi_free_absolute_path(generated);
        path
    };

    let object_type = unsafe { sys::uacpi_object_get_type(object) };
    let mut summary = if object_type == sys::uacpi_object_type_UACPI_OBJECT_METHOD {
        match unsafe { method_arguments(node) } {
            Some(1) => "1 args".to_string(),
            Some(n) => format!("{n} args"),
            None => String::new(),
        }
    } else {
        unsafe { describe(object) }
    };
    if unsafe { sys::uacpi_namespace_node_is_alias(node) } {
        summary = format!("(alias) {summary}").trim_end().to_string();
    }

    // Nodes made by Scope() (and the predefined \_SB, \_GPE etc.) have a namespace node but no
    // object of their own, which uACPI reports as an uninitialized object.
    let type_name = if object_type == sys::uacpi_object_type_UACPI_OBJECT_UNINITIALIZED {
        "Scope"
    } else {
        type_name(object_type)
    };
    entries.push(Entry {
        path,
        type_name: type_name.to_string(),
        summary,
    });
    sys::uacpi_iteration_decision_UACPI_ITERATION_DECISION_CONTINUE
}

/// The names used for object types are the same as the `acpi` backend's, so that the two
/// interpreters' output can be compared.
fn type_name(object_type: sys::uacpi_object_type) -> &'static str {
    match object_type {
        sys::uacpi_object_type_UACPI_OBJECT_UNINITIALIZED => "Uninitialized",
        sys::uacpi_object_type_UACPI_OBJECT_INTEGER => "Integer",
        sys::uacpi_object_type_UACPI_OBJECT_STRING => "String",
        sys::uacpi_object_type_UACPI_OBJECT_BUFFER => "Buffer",
        sys::uacpi_object_type_UACPI_OBJECT_PACKAGE => "Package",
        sys::uacpi_object_type_UACPI_OBJECT_FIELD_UNIT => "FieldUnit",
        sys::uacpi_object_type_UACPI_OBJECT_DEVICE => "Device",
        sys::uacpi_object_type_UACPI_OBJECT_EVENT => "Event",
        sys::uacpi_object_type_UACPI_OBJECT_METHOD => "Method",
        sys::uacpi_object_type_UACPI_OBJECT_MUTEX => "Mutex",
        sys::uacpi_object_type_UACPI_OBJECT_OPERATION_REGION => "OpRegion",
        sys::uacpi_object_type_UACPI_OBJECT_POWER_RESOURCE => "PowerResource",
        sys::uacpi_object_type_UACPI_OBJECT_PROCESSOR => "Processor",
        sys::uacpi_object_type_UACPI_OBJECT_THERMAL_ZONE => "ThermalZone",
        sys::uacpi_object_type_UACPI_OBJECT_BUFFER_FIELD => "BufferField",
        sys::uacpi_object_type_UACPI_OBJECT_DEBUG => "Debug",
        sys::uacpi_object_type_UACPI_OBJECT_REFERENCE => "Reference",
        sys::uacpi_object_type_UACPI_OBJECT_BUFFER_INDEX => "BufferIndex",
        _ => "Unknown",
    }
}

/// A short human-readable rendering of an object's value. Packages are expanded to show their
/// elements. uACPI has no public way to look inside operation regions, fields and the like, so
/// those have no summary.
///
/// # Safety
/// `object` must be a valid uACPI object.
unsafe fn describe(object: *mut sys::uacpi_object) -> String {
    unsafe {
        match sys::uacpi_object_get_type(object) {
            sys::uacpi_object_type_UACPI_OBJECT_INTEGER => {
                let mut value = 0u64;
                sys::uacpi_object_get_integer(object, &mut value);
                format!("{value:#x} ({value})")
            }
            sys::uacpi_object_type_UACPI_OBJECT_STRING => {
                let mut view = sys::uacpi_data_view::default();
                sys::uacpi_object_get_string(object, &mut view);
                // The view is the string's characters, without its terminator.
                let bytes =
                    std::slice::from_raw_parts(view.__bindgen_anon_1.const_bytes, view.length);
                format!(
                    "{:?}",
                    String::from_utf8_lossy(bytes.strip_suffix(&[0]).unwrap_or(bytes))
                )
            }
            sys::uacpi_object_type_UACPI_OBJECT_BUFFER => {
                let mut view = sys::uacpi_data_view::default();
                sys::uacpi_object_get_buffer(object, &mut view);
                let bytes =
                    std::slice::from_raw_parts(view.__bindgen_anon_1.const_bytes, view.length);
                let shown: Vec<String> =
                    bytes.iter().take(16).map(|b| format!("{b:02x}")).collect();
                let more = if bytes.len() > 16 { " ..." } else { "" };
                format!("{} bytes: {}{more}", bytes.len(), shown.join(" "))
            }
            sys::uacpi_object_type_UACPI_OBJECT_PACKAGE => {
                let mut array = sys::uacpi_object_array {
                    objects: null_mut(),
                    count: 0,
                };
                sys::uacpi_object_get_package(object, &mut array);
                let items = if array.objects.is_null() {
                    &[]
                } else {
                    std::slice::from_raw_parts(array.objects, array.count)
                };
                let parts: Vec<String> = items
                    .iter()
                    .map(|&item| {
                        let kind = type_name(sys::uacpi_object_get_type(item));
                        format!("{kind} {}", describe(item)).trim_end().to_string()
                    })
                    .collect();
                format!("{{ {} }}", parts.join(", "))
            }
            _ => String::new(),
        }
    }
}
