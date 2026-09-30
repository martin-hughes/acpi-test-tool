//! The host side of ACPICA: the `AcpiOs*` functions in `acpiosxf.h` (its "OS Services Layer").
//!
//! ACPICA is written to be embedded in an OS kernel, and calls back into its host for everything
//! from memory allocation to port I/O. The simulated hardware and OS services are in
//! [`crate::hostsim`]; this layer adapts them to ACPICA's interface.
//!
//! ACPICA keeps its state in C globals, so there can only be one live instance per process, and
//! the state kept here is global too.

use crate::hostsim::{
    self, BlockingMutex, Counter, IoPorts, PhysicalMemory, TIMEOUT_INFINITE, handle_free,
    handle_new, handle_ref,
};
use acpica_sys as sys;
use std::ffi::{CStr, c_char, c_void};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

// ACPICA status codes. They are macros with casts in the C headers, so bindgen skips them. Each
// is a category (ENVIRONMENTAL 0x0000, PROGRAMMER 0x1000, and so on) plus a number.
pub const AE_OK: sys::ACPI_STATUS = 0;
pub const AE_NULL_OBJECT: sys::ACPI_STATUS = 0x0009;
pub const AE_TIME: sys::ACPI_STATUS = 0x0011;
pub const AE_BAD_PARAMETER: sys::ACPI_STATUS = 0x1001;

// Types that the C headers define with macros, which bindgen skips as well.
type AcpiSpinlock = *mut c_void;
type AcpiSemaphore = *mut c_void;
type AcpiCpuFlags = u64;
type AcpiThreadId = u64;

/// The address of the RSDP, as reported to ACPICA.
static RSDP: Mutex<u64> = Mutex::new(0);
static MEMORY: PhysicalMemory = PhysicalMemory::new();
static PORTS: IoPorts = IoPorts::new();

/// Tell the host where the firmware tables live. Called before ACPICA is initialised.
pub fn install_firmware(rsdp: u64, identity_ranges: Vec<(usize, usize)>) {
    *hostsim::lock(&RSDP) = rsdp;
    MEMORY.set_identity_ranges(identity_ranges);
}

/// Forget all per-instance state. Called after `AcpiTerminate`.
pub fn reset() {
    *hostsim::lock(&RSDP) = 0;
    MEMORY.clear();
    PORTS.clear();
}

// ---------------------------------------------------------------------------------------------
// Initialisation and tables
// ---------------------------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsInitialize() -> sys::ACPI_STATUS {
    AE_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsTerminate() -> sys::ACPI_STATUS {
    AE_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsGetRootPointer() -> sys::ACPI_PHYSICAL_ADDRESS {
    *hostsim::lock(&RSDP)
}

/// Nothing overrides ACPICA's predefined objects.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsPredefinedOverride(
    _init_val: *const sys::ACPI_PREDEFINED_NAMES,
    new_val: *mut sys::ACPI_STRING,
) -> sys::ACPI_STATUS {
    unsafe { *new_val = std::ptr::null_mut() };
    AE_OK
}

/// Nothing replaces the tables either.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsTableOverride(
    _existing_table: *mut sys::ACPI_TABLE_HEADER,
    new_table: *mut *mut sys::ACPI_TABLE_HEADER,
) -> sys::ACPI_STATUS {
    unsafe { *new_table = std::ptr::null_mut() };
    AE_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsPhysicalTableOverride(
    _existing_table: *mut sys::ACPI_TABLE_HEADER,
    new_address: *mut sys::ACPI_PHYSICAL_ADDRESS,
    new_table_length: *mut u32,
) -> sys::ACPI_STATUS {
    unsafe {
        *new_address = 0;
        *new_table_length = 0;
    }
    AE_OK
}

// ---------------------------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------------------------

// ACPICA supplies `AcpiOsAllocateZeroed` itself, on top of this.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsAllocate(size: sys::ACPI_SIZE) -> *mut c_void {
    unsafe { hostsim::heap_alloc(size as usize) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsFree(memory: *mut c_void) {
    unsafe { hostsim::heap_free(memory) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsMapMemory(
    where_: sys::ACPI_PHYSICAL_ADDRESS,
    length: sys::ACPI_SIZE,
) -> *mut c_void {
    MEMORY.map(where_, length as usize)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsUnmapMemory(logical_address: *mut c_void, _size: sys::ACPI_SIZE) {
    MEMORY.unmap(logical_address);
}

/// Mapped addresses are their own physical addresses, as far as ACPICA is concerned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsGetPhysicalAddress(
    logical_address: *mut c_void,
    physical_address: *mut sys::ACPI_PHYSICAL_ADDRESS,
) -> sys::ACPI_STATUS {
    unsafe { *physical_address = logical_address as usize as u64 };
    AE_OK
}

/// `AcpiOsReadMemory` and `AcpiOsWriteMemory` are for accesses to physical memory that aren't
/// through a mapping. That memory is not simulated: it reads as zero, and writes are discarded.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsReadMemory(
    _address: sys::ACPI_PHYSICAL_ADDRESS,
    value: *mut u64,
    _width: u32,
) -> sys::ACPI_STATUS {
    unsafe { *value = 0 };
    AE_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsWriteMemory(
    _address: sys::ACPI_PHYSICAL_ADDRESS,
    _value: u64,
    _width: u32,
) -> sys::ACPI_STATUS {
    AE_OK
}

/// Whether the memory at a pointer can be read or written. All of it is, as far as the simulated
/// host is concerned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsReadable(_pointer: *mut c_void, _length: sys::ACPI_SIZE) -> u8 {
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsWritable(_pointer: *mut c_void, _length: sys::ACPI_SIZE) -> u8 {
    1
}

// ---------------------------------------------------------------------------------------------
// Port I/O and PCI configuration space
// ---------------------------------------------------------------------------------------------

/// `width` is in bits.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsReadPort(
    address: sys::ACPI_IO_ADDRESS,
    value: *mut u32,
    width: u32,
) -> sys::ACPI_STATUS {
    unsafe { *value = PORTS.read(address as usize, (width / 8) as usize) as u32 };
    AE_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsWritePort(
    address: sys::ACPI_IO_ADDRESS,
    value: u32,
    width: u32,
) -> sys::ACPI_STATUS {
    PORTS.write(address as usize, (width / 8) as usize, value.into());
    AE_OK
}

// PCI configuration space reads return zero, and writes are discarded.

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsReadPciConfiguration(
    _pci_id: *mut sys::ACPI_PCI_ID,
    _register: u32,
    value: *mut u64,
    _width: u32,
) -> sys::ACPI_STATUS {
    unsafe { *value = 0 };
    AE_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsWritePciConfiguration(
    _pci_id: *mut sys::ACPI_PCI_ID,
    _register: u32,
    _value: u64,
    _width: u32,
) -> sys::ACPI_STATUS {
    AE_OK
}

// ---------------------------------------------------------------------------------------------
// Locks, semaphores and threads
// ---------------------------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsCreateLock(out_handle: *mut AcpiSpinlock) -> sys::ACPI_STATUS {
    unsafe { *out_handle = handle_new(BlockingMutex::new()) };
    AE_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsDeleteLock(handle: AcpiSpinlock) {
    unsafe { handle_free::<BlockingMutex>(handle) }
}

/// Spinlocks are only needed to protect against interrupts and other threads; a mutex does that.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsAcquireLock(handle: AcpiSpinlock) -> AcpiCpuFlags {
    unsafe { handle_ref::<BlockingMutex>(handle) }.acquire(TIMEOUT_INFINITE);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsReleaseLock(handle: AcpiSpinlock, _flags: AcpiCpuFlags) {
    unsafe { handle_ref::<BlockingMutex>(handle) }.release();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsCreateSemaphore(
    _max_units: u32,
    initial_units: u32,
    out_handle: *mut AcpiSemaphore,
) -> sys::ACPI_STATUS {
    unsafe { *out_handle = handle_new(Counter::new(initial_units.into())) };
    AE_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsDeleteSemaphore(handle: AcpiSemaphore) -> sys::ACPI_STATUS {
    unsafe { handle_free::<Counter>(handle) };
    AE_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsWaitSemaphore(
    handle: AcpiSemaphore,
    units: u32,
    timeout: u16,
) -> sys::ACPI_STATUS {
    if unsafe { handle_ref::<Counter>(handle) }.wait(units.into(), timeout) {
        AE_OK
    } else {
        AE_TIME
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsSignalSemaphore(
    handle: AcpiSemaphore,
    units: u32,
) -> sys::ACPI_STATUS {
    unsafe { handle_ref::<Counter>(handle) }.signal(units.into());
    AE_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsGetThreadId() -> AcpiThreadId {
    hostsim::thread_token() as AcpiThreadId
}

/// Deferred work (GPE methods, notifications) is simply run straight away.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsExecute(
    _type: sys::ACPI_EXECUTE_TYPE,
    function: sys::ACPI_OSD_EXEC_CALLBACK,
    context: *mut c_void,
) -> sys::ACPI_STATUS {
    match function {
        Some(function) => {
            unsafe { function(context) };
            AE_OK
        }
        None => AE_BAD_PARAMETER,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsWaitEventsComplete() {}

// There is no interrupt controller, so handlers are accepted but never invoked.

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsInstallInterruptHandler(
    _interrupt_number: u32,
    _service_routine: sys::ACPI_OSD_HANDLER,
    _context: *mut c_void,
) -> sys::ACPI_STATUS {
    AE_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsRemoveInterruptHandler(
    _interrupt_number: u32,
    _service_routine: sys::ACPI_OSD_HANDLER,
) -> sys::ACPI_STATUS {
    AE_OK
}

// ---------------------------------------------------------------------------------------------
// Time, and signals from the firmware
// ---------------------------------------------------------------------------------------------

/// The time in units of 100 nanoseconds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsGetTimer() -> u64 {
    hostsim::nanos_since_start() / 100
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsStall(microseconds: u32) {
    hostsim::stall_micros(microseconds.into());
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsSleep(milliseconds: u64) {
    hostsim::sleep_millis(milliseconds);
}

/// A Fatal or Breakpoint operator in the AML. There is nothing to stop or debug, so a fatal error
/// is reported and both are otherwise ignored.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsSignal(function: u32, info: *mut c_void) -> sys::ACPI_STATUS {
    const ACPI_SIGNAL_FATAL: u32 = 0;
    if function == ACPI_SIGNAL_FATAL && !info.is_null() {
        let fatal = unsafe { &*(info as *const sys::ACPI_SIGNAL_FATAL_INFO) };
        eprintln!(
            "firmware fatal error: type {:#x}, code {:#x}, argument {:#x}",
            fatal.Type, fatal.Code, fatal.Argument
        );
    }
    AE_OK
}

/// Called before entering a sleep state, which never happens here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AcpiOsEnterSleep(
    _sleep_state: u8,
    _rega_value: u32,
    _regb_value: u32,
) -> sys::ACPI_STATUS {
    AE_OK
}

// ---------------------------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------------------------

/// Decides which of ACPICA's output to show.
///
/// ACPICA prints an informational line ("ACPI: RSDP 0x... 000024 (v02 ACPI )", and so on) for every
/// table it installs, which is noise here. Warnings and errors ("ACPI Warning: ...",
/// "ACPI Error: ...") are wanted. Messages arrive in pieces (a prefix, the body, then a suffix), so
/// the decision made for the start of a line applies to the rest of it.
#[derive(Default)]
struct LogFilter {
    /// The last piece didn't end the line.
    mid_line: bool,
    /// The line in progress is being hidden.
    hiding: bool,
}

/// What to do with a piece of output.
#[derive(Debug, PartialEq)]
enum Verdict {
    Hide,
    /// Print it. `starts_error` says whether it begins an error message.
    Show {
        starts_error: bool,
    },
}

impl LogFilter {
    fn feed(&mut self, piece: &str) -> Verdict {
        if piece.is_empty() {
            return Verdict::Hide;
        }
        if !self.mid_line {
            self.hiding = piece.starts_with("ACPI: ");
        }
        let starts_error = !self.mid_line
            && (piece.starts_with("ACPI Error") || piece.starts_with("ACPI Exception"));
        self.mid_line = !piece.ends_with('\n');
        if self.hiding {
            Verdict::Hide
        } else {
            Verdict::Show { starts_error }
        }
    }
}

static LOG_FILTER: Mutex<LogFilter> = Mutex::new(LogFilter {
    mid_line: false,
    hiding: false,
});

/// How many errors ACPICA has printed. ACPICA reports a table that fails to load (bad AML, say)
/// mostly through its messages, so this is a way to find out that it happened.
static ERRORS_LOGGED: AtomicUsize = AtomicUsize::new(0);

/// The number of errors printed since the last call.
pub fn take_logged_errors() -> usize {
    ERRORS_LOGGED.swap(0, Ordering::Relaxed)
}

/// Receives everything ACPICA prints. `shim.c` does the printf formatting and calls this.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn acpica_host_log(message: *const c_char) {
    let message = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    match hostsim::lock(&LOG_FILTER).feed(&message) {
        Verdict::Hide => {}
        Verdict::Show { starts_error } => {
            if starts_error {
                ERRORS_LOGGED.fetch_add(1, Ordering::Relaxed);
            }
            // ACPICA's messages already say what they are ("ACPI Error: ..."), so nothing is
            // added to them.
            eprint!("{message}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(starts_error: bool) -> Verdict {
        Verdict::Show { starts_error }
    }

    #[test]
    fn informational_lines_are_hidden_however_they_arrive() {
        let mut filter = LogFilter::default();
        // All in one piece...
        assert_eq!(filter.feed("ACPI: RSDP 0x1234 000024\n"), Verdict::Hide);
        // ...or in three.
        assert_eq!(filter.feed("ACPI: "), Verdict::Hide);
        assert_eq!(filter.feed("XSDT 0x5678 00002C"), Verdict::Hide);
        assert_eq!(filter.feed("\n"), Verdict::Hide);
        // The next line is judged afresh.
        assert_eq!(filter.feed("ACPI Warning: careful\n"), show(false));
    }

    #[test]
    fn errors_and_warnings_are_shown_and_errors_are_recognised() {
        let mut filter = LogFilter::default();
        assert_eq!(filter.feed("ACPI Error: "), show(true));
        // The body of an error isn't itself the start of one, even if it looks like it.
        assert_eq!(filter.feed("ACPI Error: inside"), show(false));
        assert_eq!(filter.feed(" (20260408/psargs-330)\n"), show(false));
        assert_eq!(filter.feed("ACPI Exception: AE_NOT_FOUND\n"), show(true));
        assert_eq!(filter.feed("ACPI Warning: hmm\n"), show(false));
    }

    #[test]
    fn empty_pieces_change_nothing() {
        let mut filter = LogFilter::default();
        assert_eq!(filter.feed("ACPI: "), Verdict::Hide);
        assert_eq!(filter.feed(""), Verdict::Hide);
        assert_eq!(filter.feed("body\n"), Verdict::Hide);
    }
}
