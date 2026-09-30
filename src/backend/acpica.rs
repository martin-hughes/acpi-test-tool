//! [`AmlBackend`] implemented on [ACPICA](https://github.com/acpica/acpica).
//!
//! ACPICA is a C library with global state, so only one [`AcpicaBackend`] can exist in a process
//! at a time. [`crate::firmware`] builds the tables ACPICA is started from, and `host` provides
//! the functions ACPICA calls back into.

mod host;

use super::{AmlBackend, Entry, Value};
use crate::firmware::Firmware;
use crate::tables::Table;
use acpica_sys as sys;
use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr::null_mut;

/// `ACPI_ROOT_OBJECT`, the handle of the root of the namespace: all bits set.
const ROOT_OBJECT: sys::ACPI_HANDLE = usize::MAX as sys::ACPI_HANDLE;
/// `ACPI_ALLOCATE_BUFFER`: tells ACPICA to allocate the buffer for a result itself.
const ALLOCATE_BUFFER: sys::ACPI_SIZE = sys::ACPI_SIZE::MAX;

// Object types (`ACPI_TYPE_*`).
const TYPE_ANY: u32 = 0x00;
const TYPE_INTEGER: u32 = 0x01;
const TYPE_STRING: u32 = 0x02;
const TYPE_BUFFER: u32 = 0x03;
const TYPE_PACKAGE: u32 = 0x04;
const TYPE_FIELD_UNIT: u32 = 0x05;
const TYPE_DEVICE: u32 = 0x06;
const TYPE_EVENT: u32 = 0x07;
const TYPE_METHOD: u32 = 0x08;
const TYPE_MUTEX: u32 = 0x09;
const TYPE_REGION: u32 = 0x0A;
const TYPE_POWER: u32 = 0x0B;
const TYPE_PROCESSOR: u32 = 0x0C;
const TYPE_THERMAL: u32 = 0x0D;
const TYPE_BUFFER_FIELD: u32 = 0x0E;
const TYPE_DDB_HANDLE: u32 = 0x0F;
const TYPE_DEBUG_OBJECT: u32 = 0x10;
const TYPE_LOCAL_REGION_FIELD: u32 = 0x11;
const TYPE_LOCAL_BANK_FIELD: u32 = 0x12;
const TYPE_LOCAL_INDEX_FIELD: u32 = 0x13;
const TYPE_LOCAL_REFERENCE: u32 = 0x14;
const TYPE_LOCAL_ALIAS: u32 = 0x15;
const TYPE_LOCAL_METHOD_ALIAS: u32 = 0x16;
const TYPE_LOCAL_SCOPE: u32 = 0x1B;

pub struct AcpicaBackend {
    /// The tables ACPICA reads from. They must stay alive, at the same addresses, until ACPICA is
    /// terminated.
    firmware: Option<Firmware>,
    /// Whether the namespace was loaded, and so can be walked.
    loaded: bool,
}

impl AcpicaBackend {
    pub fn new() -> Self {
        Self {
            firmware: None,
            loaded: false,
        }
    }
}

impl Drop for AcpicaBackend {
    fn drop(&mut self) {
        if self.firmware.is_some() {
            // SAFETY: ACPICA was initialised by `load_tables`, and nothing uses it after this.
            unsafe { sys::AcpiTerminate() };
            host::reset();
        }
    }
}

fn status_text(status: sys::ACPI_STATUS) -> String {
    // SAFETY: ACPICA returns a pointer to a static string for any status value.
    let text = unsafe { CStr::from_ptr(sys::AcpiFormatException(status)) };
    text.to_string_lossy().into_owned()
}

fn succeeded(status: sys::ACPI_STATUS) -> bool {
    status == host::AE_OK
}

impl AmlBackend for AcpicaBackend {
    fn load_tables(&mut self, tables: &[Table]) -> Vec<String> {
        let mut problems = Vec::new();
        if self.firmware.is_some() {
            problems.push("tables have already been loaded".to_string());
            return problems;
        }

        let firmware = Firmware::build(tables, &mut problems);
        host::install_firmware(firmware.rsdp_address(), firmware.address_ranges());
        self.firmware = Some(firmware);

        // SAFETY: the OS layer functions ACPICA needs are defined in `host`, and the tables it
        // will read are kept alive by `self.firmware`.
        unsafe {
            let status = sys::AcpiInitializeSubsystem();
            if !succeeded(status) {
                problems.push(format!(
                    "ACPICA failed to initialise: {}",
                    status_text(status)
                ));
                return problems;
            }

            // This finds the tables by way of the RSDP (see `AcpiOsGetRootPointer`).
            let status = sys::AcpiInitializeTables(null_mut(), 0, 0);
            if !succeeded(status) {
                problems.push(format!(
                    "ACPICA failed to find the tables: {}",
                    status_text(status)
                ));
                return problems;
            }

            // This parses the AML, building the namespace. A failure here (bad AML) can still
            // leave a partly populated namespace, so carry on to use it.
            let status = sys::AcpiLoadTables();
            if !succeeded(status) {
                problems.push(format!(
                    "ACPICA failed to load the tables: {}",
                    status_text(status)
                ));
            }

            // This finishes ACPICA's start-up. None of the hardware, event or interrupt handler
            // set-up is wanted, and the namespace is deliberately not initialised (that would run
            // _STA and _INI methods, with `AcpiInitializeObjects`).
            let status = sys::AcpiEnableSubsystem(
                sys::ACPI_NO_ACPI_ENABLE | sys::ACPI_NO_EVENT_INIT | sys::ACPI_NO_HANDLER_INIT,
            );
            if !succeeded(status) {
                problems.push(format!("ACPICA failed to start: {}", status_text(status)));
                return problems;
            }
        }
        self.loaded = true;

        // ACPICA prints its errors as it meets them (they are already on stderr), and doesn't
        // always turn them into a failing status.
        let logged = host::take_logged_errors();
        if logged > 0 && problems.is_empty() {
            problems.push(format!(
                "ACPICA reported {logged} error(s) while loading; the namespace may be incomplete"
            ));
        }
        problems
    }

    fn entries(&mut self) -> Vec<Entry> {
        if !self.loaded {
            return Vec::new();
        }
        let mut entries: Vec<Entry> = Vec::new();
        // SAFETY: the callback only uses its context as the `Vec` passed here, which outlives the
        // call.
        let status = unsafe {
            sys::AcpiWalkNamespace(
                TYPE_ANY,
                ROOT_OBJECT,
                u32::MAX,
                Some(visit_node),
                None,
                &mut entries as *mut Vec<Entry> as *mut c_void,
                null_mut(),
            )
        };
        if !succeeded(status) {
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

        // SAFETY: `path` is a valid C string for the whole block, and everything ACPICA allocates
        // is freed before the block ends.
        unsafe {
            let mut handle: sys::ACPI_HANDLE = null_mut();
            let status = sys::AcpiGetHandle(null_mut(), path.as_ptr(), &mut handle);
            if !succeeded(status) {
                return Err(status_text(status));
            }
            if method_arguments(handle).is_some_and(|arguments| arguments > 0) {
                return Err("the method takes arguments, which are not supported yet".into());
            }

            let mut result = sys::ACPI_BUFFER {
                Length: ALLOCATE_BUFFER,
                Pointer: null_mut(),
            };
            let status = sys::AcpiEvaluateObject(
                null_mut(),
                path.as_ptr() as sys::ACPI_STRING,
                null_mut(),
                &mut result,
            );
            // A method that finishes without returning anything is reported as an error, or
            // succeeds with no result.
            if status == host::AE_NULL_OBJECT || (succeeded(status) && result.Pointer.is_null()) {
                return Ok(Value {
                    type_name: "Uninitialized".into(),
                    summary: String::new(),
                });
            }
            if !succeeded(status) {
                return Err(status_text(status));
            }

            let object = result.Pointer as *const sys::ACPI_OBJECT;
            let value = Value {
                type_name: type_name((*object).Type).into(),
                summary: describe(object),
            };
            sys::AcpiOsFree(result.Pointer);
            Ok(value)
        }
    }
}

/// The number of arguments the method at `handle` takes, or `None` if it isn't a method.
///
/// # Safety
/// `handle` must be a valid namespace handle.
unsafe fn method_arguments(handle: sys::ACPI_HANDLE) -> Option<u8> {
    let mut info: *mut sys::ACPI_DEVICE_INFO = null_mut();
    // This only evaluates other objects (_HID and so on) for devices and processors, never for
    // the methods it is used on here.
    let status = unsafe { sys::AcpiGetObjectInfo(handle, &mut info) };
    if !succeeded(status) || info.is_null() {
        return None;
    }
    let info_ref = unsafe { &*info };
    let arguments = (info_ref.Type == TYPE_METHOD).then_some(info_ref.ParamCount);
    unsafe { sys::AcpiOsFree(info as *mut c_void) };
    arguments
}

/// Called by ACPICA for every node in the namespace. `context` is a `*mut Vec<Entry>`.
unsafe extern "C" fn visit_node(
    handle: sys::ACPI_HANDLE,
    _level: u32,
    context: *mut c_void,
    _return_value: *mut *mut c_void,
) -> sys::ACPI_STATUS {
    let entries = unsafe { &mut *(context as *mut Vec<Entry>) };

    let mut buffer = sys::ACPI_BUFFER {
        Length: ALLOCATE_BUFFER,
        Pointer: null_mut(),
    };
    let status = unsafe { sys::AcpiGetName(handle, sys::ACPI_FULL_PATHNAME, &mut buffer) };
    if !succeeded(status) || buffer.Pointer.is_null() {
        return host::AE_OK;
    }
    let path = unsafe { CStr::from_ptr(buffer.Pointer as *const c_char) }
        .to_string_lossy()
        .into_owned();
    unsafe { sys::AcpiOsFree(buffer.Pointer) };

    let mut object_type = TYPE_ANY;
    unsafe { sys::AcpiGetType(handle, &mut object_type) };

    let summary = match object_type {
        TYPE_METHOD => match unsafe { method_arguments(handle) } {
            Some(n) => format!("{n} args"),
            None => String::new(),
        },
        // These hold plain values, which are read without running any AML.
        TYPE_INTEGER | TYPE_STRING | TYPE_BUFFER | TYPE_PACKAGE => unsafe { value_summary(handle) },
        _ => String::new(),
    };

    // ACPICA makes the predefined \_SB_ a Device and \_TZ_ a ThermalZone. The other interpreters
    // treat them, like \_GPE, \_PR_ and \_SI_, as scopes, and this matches that.
    let name = if path == "\\_SB_" || path == "\\_TZ_" {
        "Scope"
    } else {
        type_name(object_type)
    };
    entries.push(Entry {
        path,
        type_name: name.to_string(),
        summary,
    });
    host::AE_OK
}

/// The value of the (non-method) object at `handle`, as text.
///
/// # Safety
/// `handle` must be a valid namespace handle, for an object that can be read without running code.
unsafe fn value_summary(handle: sys::ACPI_HANDLE) -> String {
    let mut result = sys::ACPI_BUFFER {
        Length: ALLOCATE_BUFFER,
        Pointer: null_mut(),
    };
    let status = unsafe { sys::AcpiEvaluateObject(handle, null_mut(), null_mut(), &mut result) };
    if !succeeded(status) || result.Pointer.is_null() {
        return String::new();
    }
    let summary = unsafe { describe(result.Pointer as *const sys::ACPI_OBJECT) };
    unsafe { sys::AcpiOsFree(result.Pointer) };
    summary
}

/// The names used for object types are the same as the other backends', so that the
/// interpreters' output can be compared.
fn type_name(object_type: u32) -> &'static str {
    match object_type {
        TYPE_ANY | TYPE_LOCAL_SCOPE => "Scope",
        TYPE_INTEGER => "Integer",
        TYPE_STRING => "String",
        TYPE_BUFFER => "Buffer",
        TYPE_PACKAGE => "Package",
        TYPE_FIELD_UNIT
        | TYPE_LOCAL_REGION_FIELD
        | TYPE_LOCAL_BANK_FIELD
        | TYPE_LOCAL_INDEX_FIELD => "FieldUnit",
        TYPE_DEVICE => "Device",
        TYPE_EVENT => "Event",
        TYPE_METHOD => "Method",
        TYPE_MUTEX => "Mutex",
        TYPE_REGION => "OpRegion",
        TYPE_POWER => "PowerResource",
        TYPE_PROCESSOR => "Processor",
        TYPE_THERMAL => "ThermalZone",
        TYPE_BUFFER_FIELD => "BufferField",
        TYPE_DDB_HANDLE => "DdbHandle",
        TYPE_DEBUG_OBJECT => "Debug",
        TYPE_LOCAL_REFERENCE => "Reference",
        TYPE_LOCAL_ALIAS | TYPE_LOCAL_METHOD_ALIAS => "Alias",
        _ => "Unknown",
    }
}

/// A short human-readable rendering of an object's value. Packages are expanded to show their
/// elements.
///
/// # Safety
/// `object` must point to a valid `ACPI_OBJECT`, as returned by `AcpiEvaluateObject`.
unsafe fn describe(object: *const sys::ACPI_OBJECT) -> String {
    unsafe {
        match (*object).Type {
            TYPE_INTEGER => {
                let value = (*object).Integer.Value;
                format!("{value:#x} ({value})")
            }
            TYPE_STRING => {
                let string = &(*object).String;
                let bytes =
                    std::slice::from_raw_parts(string.Pointer as *const u8, string.Length as usize);
                format!("{:?}", String::from_utf8_lossy(bytes))
            }
            TYPE_BUFFER => {
                let buffer = &(*object).Buffer;
                let bytes = if buffer.Pointer.is_null() {
                    &[]
                } else {
                    std::slice::from_raw_parts(buffer.Pointer, buffer.Length as usize)
                };
                let shown: Vec<String> =
                    bytes.iter().take(16).map(|b| format!("{b:02x}")).collect();
                let more = if bytes.len() > 16 { " ..." } else { "" };
                format!("{} bytes: {}{more}", bytes.len(), shown.join(" "))
            }
            TYPE_PACKAGE => {
                let package = &(*object).Package;
                let elements = if package.Elements.is_null() {
                    &[]
                } else {
                    std::slice::from_raw_parts(package.Elements, package.Count as usize)
                };
                let parts: Vec<String> = elements
                    .iter()
                    .map(|element| {
                        let kind = type_name(element.Type);
                        format!("{kind} {}", describe(element))
                            .trim_end()
                            .to_string()
                    })
                    .collect();
                format!("{{ {} }}", parts.join(", "))
            }
            _ => String::new(),
        }
    }
}
