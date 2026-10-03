//! A sampling profiler for sim_load on Windows (`KILN_SAMPLE=1`): a thread suspends the
//! process's other threads every `KILN_SAMPLE_US` microseconds (default 200), unwinds their
//! stacks with the x64 unwind tables and counts each function once per sample, then prints
//! the functions with the most samples, by self and by total (inclusive) time, named
//! through the PDB. Threads waiting in the kernel (parked workers) are left out.
//!
//! Nothing allocates while a thread is suspended: stacks go into a fixed buffer first.

#![allow(clippy::upper_case_acronyms, non_snake_case)]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

type HANDLE = isize;

#[repr(C, align(16))]
struct Context([u8; 1232]);

const CONTEXT_FULL: u32 = 0x0010_000B;
const OFF_FLAGS: usize = 0x30;
const OFF_RSP: usize = 0x98;
const OFF_RIP: usize = 0xF8;

#[repr(C)]
struct ThreadEntry32 {
    dwSize: u32,
    cntUsage: u32,
    th32ThreadID: u32,
    th32OwnerProcessID: u32,
    tpBasePri: i32,
    tpDeltaPri: i32,
    dwFlags: u32,
}

#[repr(C)]
struct SymbolInfoW {
    SizeOfStruct: u32,
    TypeIndex: u32,
    Reserved: [u64; 2],
    Index: u32,
    Size: u32,
    ModBase: u64,
    Flags: u32,
    Value: u64,
    Address: u64,
    Register: u32,
    Scope: u32,
    Tag: u32,
    NameLen: u32,
    MaxNameLen: u32,
    Name: [u16; 512],
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> HANDLE;
    fn Thread32First(snap: HANDLE, e: *mut ThreadEntry32) -> i32;
    fn Thread32Next(snap: HANDLE, e: *mut ThreadEntry32) -> i32;
    fn CloseHandle(h: HANDLE) -> i32;
    fn GetCurrentProcessId() -> u32;
    fn GetLastError() -> u32;
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentProcess() -> HANDLE;
    fn OpenThread(access: u32, inherit: i32, tid: u32) -> HANDLE;
    fn SuspendThread(h: HANDLE) -> u32;
    fn ResumeThread(h: HANDLE) -> u32;
    fn GetThreadContext(h: HANDLE, c: *mut Context) -> i32;
    fn RtlLookupFunctionEntry(pc: u64, image_base: *mut u64, history: *mut u8) -> *const u8;
    fn RtlVirtualUnwind(
        handler_type: u32,
        image_base: u64,
        pc: u64,
        entry: *const u8,
        context: *mut Context,
        handler_data: *mut *mut u8,
        establisher: *mut u64,
        pointers: *mut u8,
    ) -> *mut u8;
}

#[link(name = "dbghelp")]
unsafe extern "system" {
    fn SymInitializeW(process: HANDLE, path: *const u16, invade: i32) -> i32;
    fn SymFromAddrW(process: HANDLE, addr: u64, displacement: *mut u64, info: *mut SymbolInfoW) -> i32;
    fn SymSetOptions(options: u32) -> u32;
    fn SymGetLineFromAddrW64(process: HANDLE, addr: u64, displacement: *mut u32, line: *mut LineW64) -> i32;
}

#[repr(C)]
struct LineW64 {
    SizeOfStruct: u32,
    Key: *mut u8,
    LineNumber: u32,
    FileName: *const u16,
    Address: u64,
}

const MAX_DEPTH: usize = 64;

pub struct Sampler {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<HashMap<Vec<u64>, u32>>>,
}

fn rd(c: &Context, off: usize) -> u64 {
    u64::from_le_bytes(c.0[off..off + 8].try_into().unwrap())
}

fn wr(c: &mut Context, off: usize, v: u64) {
    c.0[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

/// The suspended thread's return addresses, innermost first.
unsafe fn unwind(ctx: &mut Context, out: &mut [u64; MAX_DEPTH]) -> usize {
    let mut n = 0;
    while n < MAX_DEPTH {
        let rip = rd(ctx, OFF_RIP);
        if rip == 0 {
            break;
        }
        out[n] = rip;
        n += 1;
        let mut base = 0u64;
        let entry = unsafe { RtlLookupFunctionEntry(rip, &mut base, std::ptr::null_mut()) };
        if entry.is_null() {
            // A leaf: the return address is on top of the stack.
            let rsp = rd(ctx, OFF_RSP);
            if rsp == 0 || rsp % 8 != 0 {
                break;
            }
            let ret = unsafe { *(rsp as *const u64) };
            wr(ctx, OFF_RIP, ret);
            wr(ctx, OFF_RSP, rsp + 8);
        } else {
            let (mut data, mut frame) = (std::ptr::null_mut(), 0u64);
            unsafe { RtlVirtualUnwind(0, base, rip, entry, ctx, &mut data, &mut frame, std::ptr::null_mut()) };
        }
    }
    n
}

impl Sampler {
    pub fn start() -> Option<Sampler> {
        std::env::var_os("KILN_SAMPLE")?;
        let every = Duration::from_micros(std::env::var("KILN_SAMPLE_US").ok().and_then(|v| v.parse().ok()).unwrap_or(200));
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::spawn(move || {
            let me = unsafe { GetCurrentThreadId() };
            let pid = unsafe { GetCurrentProcessId() };
            let mut counts: HashMap<Vec<u64>, u32> = HashMap::new();
            let mut ctx = Box::new(Context([0; 1232]));
            let mut stack = [0u64; MAX_DEPTH];
            let mut handles: HashMap<u32, HANDLE> = HashMap::new();
            let mut tids = Vec::new();
            let mut round = 0u64;
            while !flag.load(Ordering::Relaxed) {
                if round % 64 == 0 {
                    tids.clear();
                    unsafe {
                        let snap = CreateToolhelp32Snapshot(4, 0);
                        let mut e: ThreadEntry32 = std::mem::zeroed();
                        e.dwSize = std::mem::size_of::<ThreadEntry32>() as u32;
                        let mut ok = Thread32First(snap, &mut e);
                        while ok != 0 {
                            if e.th32OwnerProcessID == pid && e.th32ThreadID != me {
                                tids.push(e.th32ThreadID);
                            }
                            ok = Thread32Next(snap, &mut e);
                        }
                        CloseHandle(snap);
                    }
                    for &t in &tids {
                        handles.entry(t).or_insert_with(|| unsafe { OpenThread(0x0002 | 0x0008 | 0x0040, 0, t) });
                    }
                }
                round += 1;
                for &t in &tids {
                    let h = handles[&t];
                    if h == 0 {
                        continue;
                    }
                    let n = unsafe {
                        if SuspendThread(h) == u32::MAX {
                            continue;
                        }
                        wr(&mut ctx, 0, 0);
                        ctx.0[OFF_FLAGS..OFF_FLAGS + 4].copy_from_slice(&CONTEXT_FULL.to_le_bytes());
                        let n = if GetThreadContext(h, &mut *ctx) != 0 { unwind(&mut ctx, &mut stack) } else { 0 };
                        ResumeThread(h);
                        n
                    };
                    if n > 0 {
                        *counts.entry(stack[..n].to_vec()).or_default() += 1;
                    }
                }
                // (`sleep` waits a timer tick on Windows, a millisecond or more.)
                let until = std::time::Instant::now() + every;
                while std::time::Instant::now() < until {
                    std::thread::yield_now();
                }
            }
            counts
        });
        Some(Sampler { stop, thread: Some(thread) })
    }

    pub fn report(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let counts = self.thread.take().unwrap().join().unwrap();
        let process = unsafe { GetCurrentProcess() };
        unsafe {
            SymSetOptions(0x2 | 0x10); // undecorated names, load lines
            let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_string_lossy().into_owned())).unwrap_or_default();
            let path: Vec<u16> = dir.encode_utf16().chain(std::iter::once(0)).collect();
            if SymInitializeW(process, path.as_ptr(), 1) == 0 {
                eprintln!("sampler: SymInitializeW failed");
            }
        }
        let mut names: HashMap<u64, String> = HashMap::new();
        let mut name = |addr: u64| -> String {
            names
                .entry(addr)
                .or_insert_with(|| unsafe {
                    let mut info: SymbolInfoW = std::mem::zeroed();
                    info.SizeOfStruct = 88;
                    info.MaxNameLen = 511;
                    let mut disp = 0u64;
                    if SymFromAddrW(process, addr, &mut disp, &mut info) != 0 {
                        String::from_utf16_lossy(&info.Name[..(info.NameLen as usize).min(511)])
                    } else {
                        format!("{addr:#x}")
                    }
                })
                .clone()
        };
        let idle = |n: &str| {
            n.starts_with("Nt") || n.starts_with("Zw") || n.contains("WaitFor") || n.contains("SleepEx") || n.contains("park") || n.contains("Sleep")
        };
        // `KILN_SAMPLE_FILTER`: only the stacks with a function whose name contains it.
        let filter = std::env::var("KILN_SAMPLE_FILTER").ok();
        let counts: Vec<(Vec<u64>, u32)> = counts
            .into_iter()
            .filter(|(stack, _)| filter.as_ref().is_none_or(|f| stack.iter().any(|&a| name(a).contains(f.as_str()))))
            .collect();
        let (mut selfs, mut totals): (HashMap<String, u64>, HashMap<String, u64>) = Default::default();
        let mut busy = 0u64;
        for (stack, n) in &counts {
            let leaf = name(stack[0]);
            if idle(&leaf) {
                continue;
            }
            busy += *n as u64;
            *selfs.entry(leaf).or_default() += *n as u64;
            let mut seen: Vec<String> = Vec::new();
            for &a in stack {
                let f = name(a);
                if !seen.contains(&f) {
                    *totals.entry(f.clone()).or_default() += *n as u64;
                    seen.push(f);
                }
            }
        }
        let top = |m: HashMap<String, u64>, k: usize| {
            let mut v: Vec<_> = m.into_iter().collect();
            v.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
            v.truncate(k);
            v
        };
        let limit = std::env::var("KILN_SAMPLE_TOP").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
        println!("sampler: {busy} busy samples");
        println!("-- self --");
        for (f, n) in top(selfs, limit) {
            println!("{:6.2}% {f}", n as f64 * 100.0 / busy.max(1) as f64);
        }
        // The hottest lines of the hottest functions (self time).
        let mut lines: HashMap<(String, u32), u64> = HashMap::new();
        for (stack, n) in &counts {
            let leaf = name(stack[0]);
            if idle(&leaf) {
                continue;
            }
            let line = unsafe {
                let mut l: LineW64 = std::mem::zeroed();
                l.SizeOfStruct = std::mem::size_of::<LineW64>() as u32;
                let mut disp = 0u32;
                if SymGetLineFromAddrW64(process, stack[0], &mut disp, &mut l) != 0 {
                    l.LineNumber
                } else {
                    static ONCE: std::sync::Once = std::sync::Once::new();
                    ONCE.call_once(|| eprintln!("sampler: no line for {:#x}: error {}", stack[0], GetLastError()));
                    0
                }
            };
            *lines.entry((leaf, line)).or_default() += *n as u64;
        }
        println!("-- self by line --");
        for (f, n) in top(lines.into_iter().map(|((f, l), n)| (format!("{f}:{l}"), n)).collect::<HashMap<_, _>>(), limit) {
            println!("{:6.2}% {f}", n as f64 * 100.0 / busy.max(1) as f64);
        }
        println!("-- total --");
        for (f, n) in top(totals, limit) {
            println!("{:6.2}% {f}", n as f64 * 100.0 / busy.max(1) as f64);
        }
    }
}
