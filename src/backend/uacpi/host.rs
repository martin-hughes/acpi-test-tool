//! The host side of uACPI: the `uacpi_kernel_*` functions declared in `uacpi/kernel_api.h`.
//!
//! uACPI is written to be embedded in an OS kernel, and calls back into its host for everything
//! from memory allocation to port I/O. The simulated hardware and OS services are in
//! [`crate::hostsim`]; this layer adapts them to uACPI's interface.
//!
//! uACPI keeps its state in C globals, so there can only be one live instance per process, and
//! the state kept here is global too.

use crate::hostsim::{
    self, BlockingMutex, Counter, IoPorts, PhysicalMemory, TIMEOUT_INFINITE, handle_free,
    handle_new, handle_ref,
};
use std::ffi::{CStr, c_char, c_void};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use uacpi_sys as sys;

const STATUS_OK: sys::uacpi_status = sys::uacpi_status_UACPI_STATUS_OK;
const STATUS_TIMEOUT: sys::uacpi_status = sys::uacpi_status_UACPI_STATUS_TIMEOUT;
const STATUS_INVALID_ARGUMENT: sys::uacpi_status = sys::uacpi_status_UACPI_STATUS_INVALID_ARGUMENT;

// ---------------------------------------------------------------------------------------------
// Physical memory
// ---------------------------------------------------------------------------------------------

/// The address of the RSDP, as reported to uACPI.
static RSDP: Mutex<u64> = Mutex::new(0);
static MEMORY: PhysicalMemory = PhysicalMemory::new();
static PORTS: IoPorts = IoPorts::new();

/// Tell the host where the firmware tables live. Called before uACPI is initialised.
pub fn install_firmware(rsdp: u64, identity_ranges: Vec<(usize, usize)>) {
    *hostsim::lock(&RSDP) = rsdp;
    MEMORY.set_identity_ranges(identity_ranges);
}

/// Forget all per-instance state. Called after `uacpi_state_reset`.
pub fn reset() {
    *hostsim::lock(&RSDP) = 0;
    MEMORY.clear();
    PORTS.clear();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_get_rsdp(
    out_rsdp_address: *mut sys::uacpi_phys_addr,
) -> sys::uacpi_status {
    let rsdp = *hostsim::lock(&RSDP);
    if rsdp == 0 {
        return sys::uacpi_status_UACPI_STATUS_NOT_FOUND;
    }
    unsafe { *out_rsdp_address = rsdp };
    STATUS_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_map(
    addr: sys::uacpi_phys_addr,
    len: sys::uacpi_size,
) -> *mut c_void {
    MEMORY.map(addr, len)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_unmap(addr: *mut c_void, _len: sys::uacpi_size) {
    MEMORY.unmap(addr);
}

macro_rules! mmio {
    ($read:ident, $write:ident, $ty:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $read(address: *mut c_void) -> $ty {
            unsafe { (address as *const $ty).read_volatile() }
        }

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $write(address: *mut c_void, value: $ty) {
            unsafe { (address as *mut $ty).write_volatile(value) }
        }
    };
}

// The addresses passed to these come from `uacpi_kernel_map`, so they are always host memory.
mmio!(uacpi_kernel_mmio_read8, uacpi_kernel_mmio_write8, u8);
mmio!(uacpi_kernel_mmio_read16, uacpi_kernel_mmio_write16, u16);
mmio!(uacpi_kernel_mmio_read32, uacpi_kernel_mmio_write32, u32);
mmio!(uacpi_kernel_mmio_read64, uacpi_kernel_mmio_write64, u64);

// ---------------------------------------------------------------------------------------------
// Port I/O and PCI configuration space
// ---------------------------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_io_map(
    base: sys::uacpi_io_addr,
    _len: sys::uacpi_size,
    out_handle: *mut sys::uacpi_handle,
) -> sys::uacpi_status {
    unsafe { *out_handle = base as usize as *mut c_void };
    STATUS_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_io_unmap(_handle: sys::uacpi_handle) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_pci_device_open(
    _address: sys::uacpi_pci_address,
    out_handle: *mut sys::uacpi_handle,
) -> sys::uacpi_status {
    unsafe { *out_handle = std::ptr::null_mut() };
    STATUS_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_pci_device_close(_handle: sys::uacpi_handle) {}

macro_rules! port_io {
    ($read:ident, $write:ident, $ty:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $read(
            handle: sys::uacpi_handle,
            offset: sys::uacpi_size,
            out: *mut $ty,
        ) -> sys::uacpi_status {
            let port = handle as usize + offset;
            unsafe { *out = PORTS.read(port, size_of::<$ty>()) as $ty };
            STATUS_OK
        }

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $write(
            handle: sys::uacpi_handle,
            offset: sys::uacpi_size,
            value: $ty,
        ) -> sys::uacpi_status {
            PORTS.write(handle as usize + offset, size_of::<$ty>(), value as u64);
            STATUS_OK
        }
    };
}

port_io!(uacpi_kernel_io_read8, uacpi_kernel_io_write8, u8);
port_io!(uacpi_kernel_io_read16, uacpi_kernel_io_write16, u16);
port_io!(uacpi_kernel_io_read32, uacpi_kernel_io_write32, u32);

// PCI configuration space reads return zero, and writes are discarded.
macro_rules! zero_pci {
    ($read:ident, $write:ident, $ty:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $read(
            _handle: sys::uacpi_handle,
            _offset: sys::uacpi_size,
            out: *mut $ty,
        ) -> sys::uacpi_status {
            unsafe { *out = 0 };
            STATUS_OK
        }

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $write(
            _handle: sys::uacpi_handle,
            _offset: sys::uacpi_size,
            _value: $ty,
        ) -> sys::uacpi_status {
            STATUS_OK
        }
    };
}

zero_pci!(uacpi_kernel_pci_read8, uacpi_kernel_pci_write8, u8);
zero_pci!(uacpi_kernel_pci_read16, uacpi_kernel_pci_write16, u16);
zero_pci!(uacpi_kernel_pci_read32, uacpi_kernel_pci_write32, u32);

// ---------------------------------------------------------------------------------------------
// Heap
// ---------------------------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_alloc(size: sys::uacpi_size) -> *mut c_void {
    unsafe { hostsim::heap_alloc(size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_free(mem: *mut c_void) {
    unsafe { hostsim::heap_free(mem) }
}

// ---------------------------------------------------------------------------------------------
// Logging, time, and firmware requests
// ---------------------------------------------------------------------------------------------

/// How many errors uACPI has logged. uACPI reports a table that fails to load (bad AML, say)
/// only through the log, and carries on, so this is the way to find out that it happened.
static ERRORS_LOGGED: AtomicUsize = AtomicUsize::new(0);

/// The number of errors logged since the last call.
pub fn take_logged_errors() -> usize {
    ERRORS_LOGGED.swap(0, Ordering::Relaxed)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_log(level: sys::uacpi_log_level, message: *const c_char) {
    let label = match level {
        sys::uacpi_log_level_UACPI_LOG_ERROR => "ERROR",
        sys::uacpi_log_level_UACPI_LOG_WARN => "WARN",
        sys::uacpi_log_level_UACPI_LOG_INFO => "INFO",
        sys::uacpi_log_level_UACPI_LOG_TRACE => "TRACE",
        _ => "DEBUG",
    };
    if level == sys::uacpi_log_level_UACPI_LOG_ERROR {
        ERRORS_LOGGED.fetch_add(1, Ordering::Relaxed);
    }
    let message = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    // uACPI's messages already end with a newline.
    eprint!("[uACPI][{label}] {message}");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_get_nanoseconds_since_boot() -> u64 {
    hostsim::nanos_since_start()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_stall(microseconds: u8) {
    hostsim::stall_micros(microseconds.into());
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_sleep(milliseconds: u64) {
    hostsim::sleep_millis(milliseconds);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_handle_firmware_request(
    request: *mut sys::uacpi_firmware_request,
) -> sys::uacpi_status {
    let request = unsafe { &*request };
    if request.type_ as sys::uacpi_firmware_request_type
        == sys::uacpi_firmware_request_type_UACPI_FIRMWARE_REQUEST_TYPE_FATAL
    {
        let fatal = unsafe { request.__bindgen_anon_1.fatal };
        eprintln!(
            "firmware fatal error: type {:#x}, code {:#x}, argument {:#x}",
            fatal.type_, fatal.code, fatal.arg
        );
    }
    // A Breakpoint has nothing to break into here, so both kinds are simply acknowledged.
    STATUS_OK
}

// ---------------------------------------------------------------------------------------------
// Threads, mutexes, events, spinlocks and interrupts
// ---------------------------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_get_thread_id() -> sys::uacpi_thread_id {
    hostsim::thread_token() as sys::uacpi_thread_id
}

/// uACPI's interrupts don't exist here, so disabling and restoring them do nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_disable_interrupts() -> sys::uacpi_interrupt_state {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_restore_interrupts(_state: sys::uacpi_interrupt_state) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_create_mutex() -> sys::uacpi_handle {
    handle_new(BlockingMutex::new())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_free_mutex(handle: sys::uacpi_handle) {
    unsafe { handle_free::<BlockingMutex>(handle) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_acquire_mutex(
    handle: sys::uacpi_handle,
    timeout: u16,
) -> sys::uacpi_status {
    if unsafe { handle_ref::<BlockingMutex>(handle) }.acquire(timeout) {
        STATUS_OK
    } else {
        STATUS_TIMEOUT
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_release_mutex(handle: sys::uacpi_handle) {
    unsafe { handle_ref::<BlockingMutex>(handle) }.release();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_create_event() -> sys::uacpi_handle {
    handle_new(Counter::new(0))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_free_event(handle: sys::uacpi_handle) {
    unsafe { handle_free::<Counter>(handle) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_wait_for_event(
    handle: sys::uacpi_handle,
    timeout: u16,
) -> sys::uacpi_bool {
    unsafe { handle_ref::<Counter>(handle) }.wait(1, timeout)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_signal_event(handle: sys::uacpi_handle) {
    unsafe { handle_ref::<Counter>(handle) }.signal(1);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_reset_event(handle: sys::uacpi_handle) {
    unsafe { handle_ref::<Counter>(handle) }.reset();
}

// Spinlocks are only needed to protect against interrupts and other threads; a mutex does that.

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_create_spinlock() -> sys::uacpi_handle {
    handle_new(BlockingMutex::new())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_free_spinlock(handle: sys::uacpi_handle) {
    unsafe { handle_free::<BlockingMutex>(handle) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_lock_spinlock(
    handle: sys::uacpi_handle,
) -> sys::uacpi_cpu_flags {
    unsafe { handle_ref::<BlockingMutex>(handle) }.acquire(TIMEOUT_INFINITE);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_unlock_spinlock(
    handle: sys::uacpi_handle,
    _flags: sys::uacpi_cpu_flags,
) {
    unsafe { handle_ref::<BlockingMutex>(handle) }.release();
}

// There is no interrupt controller, so handlers are accepted but never invoked.

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_install_interrupt_handler(
    _irq: u32,
    _handler: sys::uacpi_interrupt_handler,
    _context: sys::uacpi_handle,
    out_irq_handle: *mut sys::uacpi_handle,
) -> sys::uacpi_status {
    unsafe { *out_irq_handle = std::ptr::null_mut() };
    STATUS_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_uninstall_interrupt_handler(
    _handler: sys::uacpi_interrupt_handler,
    _irq_handle: sys::uacpi_handle,
) -> sys::uacpi_status {
    STATUS_OK
}

/// Deferred work (GPE methods, notifications) is simply run straight away.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_schedule_work(
    _work_type: sys::uacpi_work_type,
    handler: sys::uacpi_work_handler,
    context: sys::uacpi_handle,
) -> sys::uacpi_status {
    match handler {
        Some(handler) => {
            unsafe { handler(context) };
            STATUS_OK
        }
        None => STATUS_INVALID_ARGUMENT,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_wait_for_work_completion() -> sys::uacpi_status {
    STATUS_OK
}
