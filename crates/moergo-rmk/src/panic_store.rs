//! Persist the last panic across resets so a crash-looping half can be
//! interrogated over Rynk after it comes back up.
//!
//! The store lives at a fixed address above the app's RAM (see `memory.x`),
//! untouched by init, so it survives both `SCB::sys_reset` and a watchdog
//! reset — and its location is identical across builds. The panic handler formats location and message into it
//! and reboots immediately (no halt-until-watchdog); the next boot that
//! reaches [`capture_boot`] moves the report into ordinary memory and
//! clears the marker, so one report describes exactly one crash.
//!
//! Including this module replaces `panic-probe`: it defines the binary's
//! `#[panic_handler]` and its `HardFault` exception handler.

use core::cell::RefCell;
use core::fmt::Write as _;
use core::panic::PanicInfo;

use embassy_sync::blocking_mutex::Mutex as BlockingMutex;

const MAGIC: u32 = 0x50414e49;
pub const REPORT_CAP: usize = 64;

#[repr(C)]
struct Store {
    magic: u32,
    loc: [u8; REPORT_CAP],
    loc_len: u32,
    msg: [u8; REPORT_CAP],
    msg_len: u32,
}

/// Fixed address carved out of the top of app RAM by `memory.x`, so every
/// build — crashing or reading — agrees on where the report lives.
const STORE_ADDR: usize = 0x2003_FB08;

fn store_ptr() -> *mut Store {
    STORE_ADDR as *mut Store
}

struct Buf<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl core::fmt::Write for Buf<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let take = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}

fn record(loc: core::fmt::Arguments<'_>, msg: core::fmt::Arguments<'_>) -> ! {
    cortex_m::interrupt::disable();
    let store = unsafe {
        core::ptr::write_volatile(store_ptr(), core::mem::zeroed());
        &mut *store_ptr()
    };
    let mut b = Buf {
        buf: &mut store.loc,
        len: 0,
    };
    let _ = b.write_fmt(loc);
    store.loc_len = b.len as u32;
    let mut b = Buf {
        buf: &mut store.msg,
        len: 0,
    };
    let _ = b.write_fmt(msg);
    store.msg_len = b.len as u32;
    store.magic = MAGIC;
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    cortex_m::peripheral::SCB::sys_reset();
}

#[panic_handler]
fn on_panic(info: &PanicInfo) -> ! {
    match info.location() {
        // Keep the informative tail of a long path plus the line number.
        Some(l) => {
            let file = l.file();
            let tail = &file[file.len().saturating_sub(48)..];
            record(
                format_args!("{}:{}", tail, l.line()),
                format_args!("{}", info.message()),
            )
        }
        None => record(format_args!("unknown"), format_args!("{}", info.message())),
    }
}

#[cortex_m_rt::exception]
unsafe fn HardFault(ef: &cortex_m_rt::ExceptionFrame) -> ! {
    record(
        format_args!("HARDFAULT pc={:#010x}", ef.pc()),
        format_args!("lr={:#010x} xpsr={:#010x}", ef.lr(), ef.xpsr()),
    )
}

#[derive(Clone)]
pub struct LastPanic {
    pub loc: heapless::String<REPORT_CAP>,
    pub msg: heapless::String<REPORT_CAP>,
}

static LAST: BlockingMutex<rmk::RawMutex, RefCell<Option<LastPanic>>> =
    BlockingMutex::new(RefCell::new(None));

fn read_slice(bytes: &[u8; REPORT_CAP], len: u32) -> heapless::String<REPORT_CAP> {
    let len = (len as usize).min(REPORT_CAP);
    let mut out = heapless::String::new();
    if let Ok(s) = core::str::from_utf8(&bytes[..len]) {
        let _ = out.push_str(s);
    }
    out
}

/// Move a persisted crash report out of the uninit store. Call once early
/// in boot, before anything can panic concurrently.
pub fn capture_boot() {
    let p = store_ptr();
    let magic = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*p).magic)) };
    if magic != MAGIC {
        return;
    }
    let report = unsafe {
        LastPanic {
            loc: read_slice(&(*p).loc, (*p).loc_len),
            msg: read_slice(&(*p).msg, (*p).msg_len),
        }
    };
    // Deliberately left uncleared: in a crash loop the final iteration exits
    // via the bootloader rather than a panic, so clearing here would discard
    // the only surviving report before a stable image can read it.
    LAST.lock(|slot| slot.borrow_mut().replace(report));
}

/// The report captured at this boot, if the previous run crashed.
pub fn last_panic() -> Option<LastPanic> {
    LAST.lock(|slot| slot.borrow().clone())
}

// --- boot trace: stage breadcrumbs + reset causes, next to the store ---

const STAMP_MAGIC: u32 = 0x53544d50;

#[repr(C)]
struct Stamps {
    magic: u32,
    stage: u32,
    boots: u32,
    resetreas: [u32; 2],
}

fn stamps_ptr() -> *mut Stamps {
    (STORE_ADDR + 0x90) as *mut Stamps
}

/// Record the highest boot milestone reached. Survives a watchdog reset, so
/// after a silent hang the persisted stage brackets where execution stopped.
pub fn stamp(stage: u32) {
    unsafe {
        let p = stamps_ptr();
        if core::ptr::read_volatile(core::ptr::addr_of!((*p).magic)) != STAMP_MAGIC {
            core::ptr::write_volatile(p, core::mem::zeroed());
            core::ptr::write_volatile(core::ptr::addr_of_mut!((*p).magic), STAMP_MAGIC);
        }
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*p).stage), stage);
    }
}

/// Roll this boot into the trace: bump the boot counter and shift in the
/// hardware reset reason (bit 1 = watchdog), clearing it for the next run.
pub fn boot_mark() {
    const RESETREAS: *mut u32 = 0x4000_0400 as *mut u32;
    unsafe {
        let reas = core::ptr::read_volatile(RESETREAS);
        core::ptr::write_volatile(RESETREAS, 0xFFFF_FFFF);
        let p = stamps_ptr();
        if core::ptr::read_volatile(core::ptr::addr_of!((*p).magic)) != STAMP_MAGIC {
            core::ptr::write_volatile(p, core::mem::zeroed());
            core::ptr::write_volatile(core::ptr::addr_of_mut!((*p).magic), STAMP_MAGIC);
            // The cause ring lives outside this struct; scrub its power-on garbage.
            core::ptr::write_volatile(0x2003_FBB0usize as *mut u32, 0);
            core::ptr::write_volatile(0x2003_FBB4usize as *mut u32, 0);
        }
        let prev = core::ptr::read_volatile(core::ptr::addr_of!((*p).resetreas[0]));
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*p).resetreas[1]), prev);
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*p).resetreas[0]), reas);
        let boots = core::ptr::read_volatile(core::ptr::addr_of!((*p).boots));
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*p).boots), boots.wrapping_add(1));
    }
}

/// The numeric trace fields, for the split-link debug relay.
pub fn trace_parts() -> (u32, u32, [u32; 2], [u32; 2]) {
    unsafe {
        let p = stamps_ptr();
        if core::ptr::read_volatile(core::ptr::addr_of!((*p).magic)) != STAMP_MAGIC {
            return (0, 0, [0; 2], [0; 2]);
        }
        (
            core::ptr::read_volatile(core::ptr::addr_of!((*p).stage)),
            core::ptr::read_volatile(core::ptr::addr_of!((*p).boots)),
            [
                core::ptr::read_volatile(core::ptr::addr_of!((*p).resetreas[0])),
                core::ptr::read_volatile(core::ptr::addr_of!((*p).resetreas[1])),
            ],
            [
                core::ptr::read_volatile(0x2003_FBB0usize as *const u32),
                core::ptr::read_volatile(0x2003_FBB4usize as *const u32),
            ],
        )
    }
}

/// The persisted report's location line, read straight from the store (the
/// peripheral never calls [`capture_boot`], so the store is the source).
pub fn raw_report_loc() -> Option<heapless::String<REPORT_CAP>> {
    let p = store_ptr();
    unsafe {
        if core::ptr::read_volatile(core::ptr::addr_of!((*p).magic)) != MAGIC {
            return None;
        }
        Some(read_slice(&(*p).loc, (*p).loc_len))
    }
}

/// Render the persisted trace: `stage=N boots=N rr=this,prev`.
pub fn boot_trace() -> heapless::String<REPORT_CAP> {
    let mut out = heapless::String::new();
    unsafe {
        let p = stamps_ptr();
        if core::ptr::read_volatile(core::ptr::addr_of!((*p).magic)) == STAMP_MAGIC {
            let _ = write!(
                out,
                "stage={} boots={} rr={:#x},{:#x} cause={},{}",
                core::ptr::read_volatile(core::ptr::addr_of!((*p).stage)),
                core::ptr::read_volatile(core::ptr::addr_of!((*p).boots)),
                core::ptr::read_volatile(core::ptr::addr_of!((*p).resetreas[0])),
                core::ptr::read_volatile(core::ptr::addr_of!((*p).resetreas[1])),
                core::ptr::read_volatile(0x2003_FBB0usize as *const u32),
                core::ptr::read_volatile(0x2003_FBB4usize as *const u32),
            );
        } else {
            let _ = out.push_str("no trace");
        }
    }
    out
}
