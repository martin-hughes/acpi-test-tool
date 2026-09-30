//! Simulated hardware and operating system services, shared by the host layers of the C
//! interpreters (uACPI and ACPICA).
//!
//! Both are written to be embedded in an OS kernel, and call back into their host for everything
//! from memory allocation to port I/O. Nothing here touches the real machine: memory that isn't
//! one of the interpreter's own tables is a block of zeroes, port I/O is a block of memory that
//! reads back what was written, and PCI reads return zero.

// Only the C interpreters use this, and they are optional.
#![cfg_attr(not(any(feature = "uacpi", feature = "acpica")), allow(dead_code))]

use std::alloc::{Layout, alloc, dealloc};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// The "wait forever" timeout used by both interpreters (in milliseconds).
pub const TIMEOUT_INFINITE: u16 = 0xFFFF;

/// Lock a std mutex, ignoring poisoning: the guarded data is plain state and stays consistent.
pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------------------------
// Heap
// ---------------------------------------------------------------------------------------------

/// Allocations carry a header holding their size, so `free` (which is not told the size) can
/// hand the right layout back to the allocator.
const HEADER: usize = 16;

fn layout(size: usize) -> Option<Layout> {
    Layout::from_size_align(size.checked_add(HEADER)?, HEADER).ok()
}

/// Allocate `size` bytes, or return null. Free the result with [`heap_free`].
///
/// # Safety
/// The memory is uninitialised.
pub unsafe fn heap_alloc(size: usize) -> *mut c_void {
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

/// Free memory from [`heap_alloc`]. Null is allowed.
///
/// # Safety
/// `mem` must be null or a pointer returned by `heap_alloc` that hasn't been freed.
pub unsafe fn heap_free(mem: *mut c_void) {
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
// Synchronisation
// ---------------------------------------------------------------------------------------------

/// Wait on `condvar` while `blocked(&*guard)` holds, for `timeout_ms` (or forever, or not at all
/// for 0). Returns the guard, and whether it is still blocked (that is, the wait timed out).
fn wait_while<'a, T>(
    guard: MutexGuard<'a, T>,
    condvar: &Condvar,
    timeout_ms: u16,
    blocked: impl Fn(&T) -> bool,
) -> (MutexGuard<'a, T>, bool) {
    let mut guard = guard;
    match timeout_ms {
        0 => {}
        TIMEOUT_INFINITE => {
            while blocked(&guard) {
                guard = condvar.wait(guard).unwrap_or_else(|e| e.into_inner());
            }
        }
        ms => {
            guard = condvar
                .wait_timeout_while(guard, Duration::from_millis(ms.into()), |t| blocked(t))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
    let still_blocked = blocked(&guard);
    (guard, still_blocked)
}

/// A non-recursive mutex that, unlike `std::sync::Mutex`, is locked and unlocked by separate
/// calls rather than by a guard's lifetime.
pub struct BlockingMutex {
    locked: Mutex<bool>,
    changed: Condvar,
}

impl BlockingMutex {
    pub fn new() -> Self {
        Self {
            locked: Mutex::new(false),
            changed: Condvar::new(),
        }
    }

    /// Try to take the mutex, waiting up to `timeout_ms` milliseconds (0 means don't wait, and
    /// [`TIMEOUT_INFINITE`] means wait as long as it takes). Returns whether it was taken.
    pub fn acquire(&self, timeout_ms: u16) -> bool {
        let (mut locked, still_locked) =
            wait_while(lock(&self.locked), &self.changed, timeout_ms, |held| *held);
        if still_locked {
            return false;
        }
        *locked = true;
        true
    }

    pub fn release(&self) {
        *lock(&self.locked) = false;
        self.changed.notify_one();
    }
}

/// A counter that threads can wait on: both uACPI's events and ACPICA's semaphores.
pub struct Counter {
    count: Mutex<u64>,
    changed: Condvar,
}

impl Counter {
    pub fn new(initial: u64) -> Self {
        Self {
            count: Mutex::new(initial),
            changed: Condvar::new(),
        }
    }

    /// Take `units` from the counter, waiting up to `timeout_ms` milliseconds for that many to be
    /// available. Returns whether they were taken.
    pub fn wait(&self, units: u64, timeout_ms: u16) -> bool {
        let (mut count, short) =
            wait_while(lock(&self.count), &self.changed, timeout_ms, |c| *c < units);
        if short {
            return false;
        }
        *count -= units;
        true
    }

    pub fn signal(&self, units: u64) {
        *lock(&self.count) += units;
        self.changed.notify_all();
    }

    // Only uACPI's events can be reset.
    #[cfg_attr(not(feature = "uacpi"), allow(dead_code))]
    pub fn reset(&self) {
        *lock(&self.count) = 0;
    }
}

/// Turn a value into an opaque handle to give the interpreter, to be got back with [`handle_ref`]
/// and released with [`handle_free`].
pub fn handle_new<T>(value: T) -> *mut c_void {
    Box::into_raw(Box::new(value)) as *mut c_void
}

/// # Safety
/// `handle` must come from [`handle_new`] for the same `T`, and not have been freed.
pub unsafe fn handle_ref<'a, T>(handle: *mut c_void) -> &'a T {
    unsafe { &*(handle as *const T) }
}

/// # Safety
/// `handle` must come from [`handle_new`] for the same `T`, and must not be used again.
pub unsafe fn handle_free<T>(handle: *mut c_void) {
    drop(unsafe { Box::from_raw(handle as *mut T) });
}

/// A number that identifies the calling thread: unique to it, and never 0 or -1 (which the
/// interpreters reserve).
pub fn thread_token() -> usize {
    // The address of a thread-local is unique to the thread.
    thread_local!(static ID: u8 = const { 0 });
    ID.with(|id| id as *const u8 as usize)
}

// ---------------------------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------------------------

/// Nanoseconds since the first time this was called.
pub fn nanos_since_start() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

pub fn stall_micros(microseconds: u64) {
    let end = Instant::now() + Duration::from_micros(microseconds);
    while Instant::now() < end {
        std::hint::spin_loop();
    }
}

pub fn sleep_millis(milliseconds: u64) {
    std::thread::sleep(Duration::from_millis(milliseconds));
}

// ---------------------------------------------------------------------------------------------
// Port I/O
// ---------------------------------------------------------------------------------------------

/// The 64 KiB of x86 port space, all zero to begin with. The interpreters expect to read back what
/// they wrote to registers such as the fixed event enables, and complain if they don't.
#[derive(Default)]
pub struct IoPorts {
    space: Mutex<Vec<u8>>,
}

const IO_SPACE_SIZE: usize = 0x1_0000;

impl IoPorts {
    pub const fn new() -> Self {
        Self {
            space: Mutex::new(Vec::new()),
        }
    }

    fn with_space<R>(&self, f: impl FnOnce(&mut [u8]) -> R) -> R {
        let mut space = lock(&self.space);
        if space.is_empty() {
            space.resize(IO_SPACE_SIZE, 0);
        }
        f(&mut space)
    }

    /// Read `bytes` (1 to 8) bytes, little-endian, starting at `port`. Ports outside the space
    /// read as zero.
    pub fn read(&self, port: usize, bytes: usize) -> u64 {
        self.with_space(|space| {
            let mut value = [0u8; 8];
            if let Some(stored) = port.checked_add(bytes).and_then(|end| space.get(port..end)) {
                value[..bytes].copy_from_slice(stored);
            }
            u64::from_le_bytes(value)
        })
    }

    /// Write the low `bytes` (1 to 8) bytes of `value`, little-endian, starting at `port`. Writes
    /// outside the space are discarded.
    pub fn write(&self, port: usize, bytes: usize, value: u64) {
        self.with_space(|space| {
            if let Some(stored) = port
                .checked_add(bytes)
                .and_then(|end| space.get_mut(port..end))
            {
                stored.copy_from_slice(&value.to_le_bytes()[..bytes]);
            }
        });
    }

    /// Forget everything that was written.
    pub fn clear(&self) {
        lock(&self.space).clear();
    }
}

// ---------------------------------------------------------------------------------------------
// Physical memory
// ---------------------------------------------------------------------------------------------

/// A block of zeroed memory standing in for a physical address that has been mapped.
struct FakeMapping {
    memory: Box<[u8]>,
    references: usize,
}

#[derive(Default)]
struct MemoryState {
    /// Address ranges (start, length) holding the tables built for the interpreter. The
    /// "physical" addresses in those tables are really the addresses of host memory, so mapping
    /// them is the identity.
    identity: Vec<(usize, usize)>,
    /// Fake memory handed out for every other physical address (operation regions and so on),
    /// keyed by (physical address, length).
    fake: HashMap<(u64, usize), FakeMapping>,
    /// Maps the pointer returned for a fake mapping back to its key.
    by_pointer: HashMap<usize, (u64, usize)>,
}

/// Maps "physical" addresses for an interpreter.
#[derive(Default)]
pub struct PhysicalMemory {
    state: Mutex<Option<MemoryState>>,
}

impl PhysicalMemory {
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(None),
        }
    }

    fn with_state<R>(&self, f: impl FnOnce(&mut MemoryState) -> R) -> R {
        let mut state = lock(&self.state);
        f(state.get_or_insert_with(MemoryState::default))
    }

    /// Say which ranges of host memory hold firmware tables (see [`crate::firmware::Firmware`]).
    pub fn set_identity_ranges(&self, ranges: Vec<(usize, usize)>) {
        self.with_state(|state| state.identity = ranges);
    }

    /// Map `len` bytes at physical address `addr`. The tables map to themselves; anything else is
    /// zeroed memory, shared by mappings of the same address and length.
    pub fn map(&self, addr: u64, len: usize) -> *mut c_void {
        self.with_state(|state| {
            let start = addr as usize;
            if state
                .identity
                .iter()
                .any(|&(base, size)| start >= base && start.saturating_add(len) <= base + size)
            {
                return start as *mut c_void;
            }

            let key = (addr, len);
            let mapping = state.fake.entry(key).or_insert_with(|| FakeMapping {
                memory: vec![0u8; len.max(1)].into_boxed_slice(),
                references: 0,
            });
            mapping.references += 1;
            let pointer = mapping.memory.as_mut_ptr() as usize;
            state.by_pointer.insert(pointer, key);
            pointer as *mut c_void
        })
    }

    /// Release a mapping from [`map`](Self::map). Identity mappings need no release.
    pub fn unmap(&self, pointer: *mut c_void) {
        self.with_state(|state| {
            let Some(&key) = state.by_pointer.get(&(pointer as usize)) else {
                return;
            };
            if let Some(mapping) = state.fake.get_mut(&key) {
                mapping.references -= 1;
                if mapping.references == 0 {
                    state.fake.remove(&key);
                    state.by_pointer.remove(&(pointer as usize));
                }
            }
        });
    }

    /// Forget all mappings.
    pub fn clear(&self) {
        *lock(&self.state) = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heap_round_trips() {
        unsafe {
            let p = heap_alloc(100) as *mut u8;
            assert!(!p.is_null());
            p.write_bytes(0xAB, 100);
            heap_free(p as *mut c_void);
            heap_free(std::ptr::null_mut());
        }
    }

    #[test]
    fn mutex_times_out_and_then_succeeds() {
        let mutex = BlockingMutex::new();
        assert!(mutex.acquire(0));
        assert!(!mutex.acquire(0));
        assert!(!mutex.acquire(10));
        mutex.release();
        assert!(mutex.acquire(TIMEOUT_INFINITE));
    }

    #[test]
    fn counter_hands_out_units() {
        let counter = Counter::new(2);
        assert!(counter.wait(2, 0));
        assert!(!counter.wait(1, 0));
        counter.signal(3);
        assert!(!counter.wait(4, 5));
        assert!(counter.wait(3, 0));
        counter.signal(1);
        counter.reset();
        assert!(!counter.wait(1, 0));
    }

    #[test]
    fn a_waiter_is_woken_by_a_signal() {
        let counter = std::sync::Arc::new(Counter::new(0));
        let waiter = {
            let counter = counter.clone();
            std::thread::spawn(move || counter.wait(1, TIMEOUT_INFINITE))
        };
        std::thread::sleep(Duration::from_millis(20));
        counter.signal(1);
        assert!(waiter.join().unwrap());
    }

    #[test]
    fn io_ports_read_back_what_was_written() {
        let ports = IoPorts::new();
        assert_eq!(ports.read(0x400, 4), 0);
        ports.write(0x400, 4, 0xDEADBEEF);
        assert_eq!(ports.read(0x400, 4), 0xDEADBEEF);
        assert_eq!(ports.read(0x401, 1), 0xBE);
        assert_eq!(ports.read(0x402, 2), 0xDEAD);
        // Out of range accesses are harmless.
        ports.write(0xFFFF, 4, 1);
        assert_eq!(ports.read(0xFFFF, 4), 0);
        assert_eq!(ports.read(usize::MAX, 4), 0);
        ports.clear();
        assert_eq!(ports.read(0x400, 4), 0);
    }

    #[test]
    fn firmware_maps_to_itself_and_everything_else_is_zeroed_memory() {
        let table = [1u8, 2, 3, 4];
        let memory = PhysicalMemory::new();
        memory.set_identity_ranges(vec![(table.as_ptr() as usize, table.len())]);

        let same = memory.map(table.as_ptr() as u64, 4);
        assert_eq!(same as usize, table.as_ptr() as usize);
        memory.unmap(same);

        let fake = memory.map(0xFED0_0000, 16) as *mut u8;
        assert_ne!(fake as usize, 0xFED0_0000);
        unsafe {
            assert_eq!(fake.read(), 0);
            fake.write(7);
        }
        // The same address and length share memory, until every mapping is released.
        let again = memory.map(0xFED0_0000, 16) as *mut u8;
        assert_eq!(again, fake);
        assert_eq!(unsafe { again.read() }, 7);
        memory.unmap(again as *mut c_void);
        memory.unmap(fake as *mut c_void);
        let fresh = memory.map(0xFED0_0000, 16) as *mut u8;
        assert_eq!(unsafe { fresh.read() }, 0);
    }

    #[test]
    fn thread_tokens_are_distinct_and_usable() {
        let here = thread_token();
        assert_ne!(here, 0);
        assert_ne!(here, usize::MAX);
        let there = std::thread::spawn(thread_token).join().unwrap();
        assert_ne!(here, there);
    }
}
