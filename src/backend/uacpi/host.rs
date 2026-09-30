//! The host side of uACPI: the `uacpi_kernel_*` functions declared in `uacpi/kernel_api.h`.
//!
//! uACPI is written to be embedded in an OS kernel, and calls back into its host for everything
//! from memory allocation to port I/O. Here, "hardware" is simulated the same way the `acpi`
//! backend's `NullHandler` does it: reads return zero and writes are discarded.
//!
//! uACPI keeps its state in C globals, so there can only be one live instance per process, and
//! the state kept here is global too.

use std::alloc::{Layout, alloc, dealloc};
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_void};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use uacpi_sys as sys;

const STATUS_OK: sys::uacpi_status = sys::uacpi_status_UACPI_STATUS_OK;
const STATUS_TIMEOUT: sys::uacpi_status = sys::uacpi_status_UACPI_STATUS_TIMEOUT;
const STATUS_INVALID_ARGUMENT: sys::uacpi_status = sys::uacpi_status_UACPI_STATUS_INVALID_ARGUMENT;

/// uACPI's "wait forever" timeout value.
const TIMEOUT_INFINITE: u16 = 0xFFFF;

/// Lock a std mutex, ignoring poisoning: the guarded data is plain state and stays consistent.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------------------------
// Physical memory
// ---------------------------------------------------------------------------------------------

/// A block of zeroed memory standing in for a physical address that uACPI has mapped.
struct FakeMapping {
    memory: Box<[u8]>,
    references: usize,
}

#[derive(Default)]
struct MemoryState {
    /// The address of the RSDP, as reported to uACPI.
    rsdp: u64,
    /// Address ranges (start, length) holding the tables built for uACPI. The "physical" addresses
    /// in those tables are really the addresses of host memory, so mapping them is the identity.
    identity: Vec<(usize, usize)>,
    /// Fake memory handed out for every other physical address (operation regions and so on),
    /// keyed by (physical address, length).
    fake: HashMap<(u64, usize), FakeMapping>,
    /// Maps the pointer returned for a fake mapping back to its key.
    by_pointer: HashMap<usize, (u64, usize)>,
}

static MEMORY: Mutex<Option<MemoryState>> = Mutex::new(None);

fn with_memory<R>(f: impl FnOnce(&mut MemoryState) -> R) -> R {
    let mut guard = lock(&MEMORY);
    f(guard.get_or_insert_with(MemoryState::default))
}

/// Tell the host where the firmware tables live. Called before uACPI is initialised.
pub fn install_firmware(rsdp: u64, identity_ranges: Vec<(usize, usize)>) {
    with_memory(|m| {
        m.rsdp = rsdp;
        m.identity = identity_ranges;
    });
}

/// Forget all per-instance state. Called after `uacpi_state_reset`.
pub fn reset() {
    *lock(&MEMORY) = None;
    lock(&IO_SPACE).clear();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_get_rsdp(
    out_rsdp_address: *mut sys::uacpi_phys_addr,
) -> sys::uacpi_status {
    let rsdp = with_memory(|m| m.rsdp);
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
    with_memory(|m| {
        let start = addr as usize;
        if m.identity
            .iter()
            .any(|&(base, size)| start >= base && start.saturating_add(len) <= base + size)
        {
            return start as *mut c_void;
        }

        let key = (addr, len);
        let mapping = m.fake.entry(key).or_insert_with(|| FakeMapping {
            memory: vec![0u8; len.max(1)].into_boxed_slice(),
            references: 0,
        });
        mapping.references += 1;
        let pointer = mapping.memory.as_mut_ptr() as usize;
        m.by_pointer.insert(pointer, key);
        pointer as *mut c_void
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_unmap(addr: *mut c_void, _len: sys::uacpi_size) {
    with_memory(|m| {
        // Identity mappings were never allocated, so there is nothing to release for them.
        let Some(&key) = m.by_pointer.get(&(addr as usize)) else {
            return;
        };
        if let Some(mapping) = m.fake.get_mut(&key) {
            mapping.references -= 1;
            if mapping.references == 0 {
                m.fake.remove(&key);
                m.by_pointer.remove(&(addr as usize));
            }
        }
    });
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
// Port I/O: a block of memory, so that what is written can be read back
// ---------------------------------------------------------------------------------------------

/// The 64 KiB of x86 port space, all zero to begin with. uACPI expects to read back what it
/// wrote to registers such as the fixed event enables, and complains if it doesn't.
static IO_SPACE: Mutex<Vec<u8>> = Mutex::new(Vec::new());

const IO_SPACE_SIZE: usize = 0x1_0000;

fn with_io_space<R>(f: impl FnOnce(&mut [u8]) -> R) -> R {
    let mut space = lock(&IO_SPACE);
    if space.is_empty() {
        space.resize(IO_SPACE_SIZE, 0);
    }
    f(&mut space)
}

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
            let value = with_io_space(|space| {
                let mut bytes = [0u8; size_of::<$ty>()];
                if let Some(stored) = space.get(port..port + bytes.len()) {
                    bytes.copy_from_slice(stored);
                }
                <$ty>::from_le_bytes(bytes)
            });
            unsafe { *out = value };
            STATUS_OK
        }

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $write(
            handle: sys::uacpi_handle,
            offset: sys::uacpi_size,
            value: $ty,
        ) -> sys::uacpi_status {
            let port = handle as usize + offset;
            with_io_space(|space| {
                if let Some(stored) = space.get_mut(port..port + size_of::<$ty>()) {
                    stored.copy_from_slice(&value.to_le_bytes());
                }
            });
            STATUS_OK
        }
    };
}

port_io!(uacpi_kernel_io_read8, uacpi_kernel_io_write8, u8);
port_io!(uacpi_kernel_io_read16, uacpi_kernel_io_write16, u16);
port_io!(uacpi_kernel_io_read32, uacpi_kernel_io_write32, u32);

// ---------------------------------------------------------------------------------------------
// PCI configuration space: reads return zero, writes are discarded
// ---------------------------------------------------------------------------------------------

macro_rules! zero_io {
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

zero_io!(uacpi_kernel_pci_read8, uacpi_kernel_pci_write8, u8);
zero_io!(uacpi_kernel_pci_read16, uacpi_kernel_pci_write16, u16);
zero_io!(uacpi_kernel_pci_read32, uacpi_kernel_pci_write32, u32);

// ---------------------------------------------------------------------------------------------
// Heap
// ---------------------------------------------------------------------------------------------

/// Allocations carry a header holding their size, so `free` (which is not told the size) can
/// hand the right layout back to the allocator.
const HEADER: usize = 16;

fn layout(size: usize) -> Option<Layout> {
    Layout::from_size_align(size.checked_add(HEADER)?, HEADER).ok()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_alloc(size: sys::uacpi_size) -> *mut c_void {
    let Some(layout) = layout(size) else {
        return std::ptr::null_mut();
    };
    unsafe {
        let base = alloc(layout);
        if base.is_null() {
            return base as *mut c_void;
        }
        (base as *mut usize).write(size);
        base.add(HEADER) as *mut c_void
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_free(mem: *mut c_void) {
    if mem.is_null() {
        return;
    }
    unsafe {
        let base = (mem as *mut u8).sub(HEADER);
        let size = (base as *const usize).read();
        dealloc(base, layout(size).expect("layout was valid when allocated"));
    }
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

fn start_time() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_get_nanoseconds_since_boot() -> u64 {
    start_time().elapsed().as_nanos() as u64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_stall(microseconds: u8) {
    let end = Instant::now() + Duration::from_micros(microseconds.into());
    while Instant::now() < end {
        std::hint::spin_loop();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_sleep(milliseconds: u64) {
    std::thread::sleep(Duration::from_millis(milliseconds));
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
    // The address of a thread-local is unique to the thread, and is neither null nor -1 (which
    // uACPI reserves as "no thread").
    thread_local!(static ID: u8 = const { 0 });
    ID.with(|id| id as *const u8 as *mut c_void)
}

/// uACPI's interrupts don't exist here, so disabling and restoring them do nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_disable_interrupts() -> sys::uacpi_interrupt_state {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_restore_interrupts(_state: sys::uacpi_interrupt_state) {}

/// A non-recursive mutex that, unlike `std::sync::Mutex`, is locked and unlocked by separate calls.
struct HostMutex {
    locked: Mutex<bool>,
    changed: Condvar,
}

impl HostMutex {
    fn create() -> sys::uacpi_handle {
        Box::into_raw(Box::new(HostMutex {
            locked: Mutex::new(false),
            changed: Condvar::new(),
        })) as sys::uacpi_handle
    }

    fn acquire(&self, timeout_ms: u16) -> sys::uacpi_status {
        let mut locked = lock(&self.locked);
        match timeout_ms {
            0 => {}
            TIMEOUT_INFINITE => {
                while *locked {
                    locked = self.changed.wait(locked).unwrap_or_else(|e| e.into_inner());
                }
            }
            ms => {
                let (guard, _) = self
                    .changed
                    .wait_timeout_while(locked, Duration::from_millis(ms.into()), |held| *held)
                    .unwrap_or_else(|e| e.into_inner());
                locked = guard;
            }
        }
        if *locked {
            return STATUS_TIMEOUT;
        }
        *locked = true;
        STATUS_OK
    }

    fn release(&self) {
        *lock(&self.locked) = false;
        self.changed.notify_one();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_create_mutex() -> sys::uacpi_handle {
    HostMutex::create()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_free_mutex(handle: sys::uacpi_handle) {
    drop(unsafe { Box::from_raw(handle as *mut HostMutex) });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_acquire_mutex(
    handle: sys::uacpi_handle,
    timeout: u16,
) -> sys::uacpi_status {
    unsafe { &*(handle as *const HostMutex) }.acquire(timeout)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_release_mutex(handle: sys::uacpi_handle) {
    unsafe { &*(handle as *const HostMutex) }.release();
}

/// A semaphore-like counter that threads can wait on.
struct HostEvent {
    count: Mutex<u64>,
    changed: Condvar,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_create_event() -> sys::uacpi_handle {
    Box::into_raw(Box::new(HostEvent {
        count: Mutex::new(0),
        changed: Condvar::new(),
    })) as sys::uacpi_handle
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_free_event(handle: sys::uacpi_handle) {
    drop(unsafe { Box::from_raw(handle as *mut HostEvent) });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_wait_for_event(
    handle: sys::uacpi_handle,
    timeout: u16,
) -> sys::uacpi_bool {
    let event = unsafe { &*(handle as *const HostEvent) };
    let mut count = lock(&event.count);
    match timeout {
        0 => {}
        TIMEOUT_INFINITE => {
            while *count == 0 {
                count = event.changed.wait(count).unwrap_or_else(|e| e.into_inner());
            }
        }
        ms => {
            let (guard, _) = event
                .changed
                .wait_timeout_while(count, Duration::from_millis(ms.into()), |c| *c == 0)
                .unwrap_or_else(|e| e.into_inner());
            count = guard;
        }
    }
    if *count == 0 {
        return false;
    }
    *count -= 1;
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_signal_event(handle: sys::uacpi_handle) {
    let event = unsafe { &*(handle as *const HostEvent) };
    *lock(&event.count) += 1;
    event.changed.notify_one();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_reset_event(handle: sys::uacpi_handle) {
    let event = unsafe { &*(handle as *const HostEvent) };
    *lock(&event.count) = 0;
}

// Spinlocks are only needed to protect against interrupts and other threads; a mutex does that.

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_create_spinlock() -> sys::uacpi_handle {
    HostMutex::create()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_free_spinlock(handle: sys::uacpi_handle) {
    unsafe { uacpi_kernel_free_mutex(handle) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_lock_spinlock(
    handle: sys::uacpi_handle,
) -> sys::uacpi_cpu_flags {
    unsafe { &*(handle as *const HostMutex) }.acquire(TIMEOUT_INFINITE);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uacpi_kernel_unlock_spinlock(
    handle: sys::uacpi_handle,
    _flags: sys::uacpi_cpu_flags,
) {
    unsafe { &*(handle as *const HostMutex) }.release();
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
