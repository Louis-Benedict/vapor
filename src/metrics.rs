use std::os::raw::{c_char, c_void};
use std::sync::Mutex;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {}

unsafe extern "C" {
    fn mach_host_self() -> u32;
    fn host_statistics(host: u32, flavor: i32, info: *mut u32, count: *mut u32) -> i32;
    fn host_statistics64(host: u32, flavor: i32, info: *mut u32, count: *mut u32) -> i32;
    fn sysctlbyname(
        name: *const u8,
        oldp: *mut c_void,
        oldlenp: *mut usize,
        newp: *const c_void,
        newlen: usize,
    ) -> i32;

    // IOKit — GPU utilization via IOAccelerator PerformanceStatistics
    fn IOServiceMatching(name: *const u8) -> *mut c_void;
    fn IOServiceGetMatchingServices(master: u32, matching: *mut c_void, iter: *mut u32) -> i32;
    fn IOIteratorNext(iter: u32) -> u32;
    fn IOObjectRelease(obj: u32) -> i32;
    fn IORegistryEntryCreateCFProperties(
        entry: u32,
        props: *mut *mut c_void,
        alloc: *const c_void,
        opts: u32,
    ) -> i32;

    // CoreFoundation
    fn CFStringCreateWithCString(
        alloc: *const c_void,
        cstr: *const c_char,
        enc: u32,
    ) -> *const c_void;
    fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
    fn CFNumberGetValue(num: *const c_void, typ: i32, val: *mut c_void) -> bool;
    fn CFRelease(cf: *const c_void);
}

const HOST_CPU_LOAD_INFO: i32 = 3;
const HOST_VM_INFO64: i32 = 4;
const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const CF_NUMBER_SINT32: i32 = 3;

// cpu_ticks[user, system, idle, nice]
#[repr(C)]
struct CpuLoadInfo {
    cpu_ticks: [u32; 4],
}

// vm_statistics64_data_t — layout verified for arm64 macOS
#[repr(C)]
struct VmStats64 {
    free_count: u32,
    active_count: u32,
    inactive_count: u32,
    wire_count: u32,
    zero_fill_count: u64,
    reactivations: u64,
    pageins: u64,
    pageouts: u64,
    faults: u64,
    cow_faults: u64,
    lookups: u64,
    hits: u64,
    purges: u64,
    purgeable_count: u32,
    speculative_count: u32,
    decompressions: u64,
    compressions: u64,
    swapins: u64,
    swapouts: u64,
    compressor_page_count: u32,
    throttled_count: u32,
    external_page_count: u32,
    internal_page_count: u32,
    total_uncompressed_pages_in_compressor: u64,
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn get_cpu_ticks() -> Option<[u32; 4]> {
    let mut info = CpuLoadInfo { cpu_ticks: [0; 4] };
    let mut count = (size_of::<CpuLoadInfo>() / 4) as u32;
    let r = unsafe {
        host_statistics(
            mach_host_self(),
            HOST_CPU_LOAD_INFO,
            info.cpu_ticks.as_mut_ptr(),
            &mut count,
        )
    };
    if r == 0 { Some(info.cpu_ticks) } else { None }
}

fn get_vm_stats() -> Option<VmStats64> {
    let mut stats: VmStats64 = unsafe { std::mem::zeroed() };
    let mut count = (size_of::<VmStats64>() / 4) as u32;
    let r = unsafe {
        host_statistics64(
            mach_host_self(),
            HOST_VM_INFO64,
            &mut stats as *mut VmStats64 as *mut u32,
            &mut count,
        )
    };
    if r == 0 { Some(stats) } else { None }
}

fn sysctl_u64(name: &[u8]) -> u64 {
    let mut val: u64 = 0;
    let mut len = size_of::<u64>();
    unsafe {
        sysctlbyname(name.as_ptr(), &mut val as *mut u64 as _, &mut len, std::ptr::null(), 0)
    };
    val
}

fn sysctl_u32(name: &[u8]) -> u32 {
    let mut val: u32 = 0;
    let mut len = size_of::<u32>();
    unsafe {
        sysctlbyname(name.as_ptr(), &mut val as *mut u32 as _, &mut len, std::ptr::null(), 0)
    };
    val
}

// Create a temporary CFString from a null-terminated byte slice.
// Caller must CFRelease the result.
fn cf_str(s: &[u8]) -> *const c_void {
    unsafe {
        CFStringCreateWithCString(
            std::ptr::null(),
            s.as_ptr() as *const c_char,
            CF_STRING_ENCODING_UTF8,
        )
    }
}

// Read a CFNumber as i32 from a CFDictionary entry identified by a C string key.
fn dict_i32(dict: *const c_void, key: &[u8]) -> Option<i32> {
    let k = cf_str(key);
    let v = unsafe { CFDictionaryGetValue(dict, k) };
    unsafe { CFRelease(k) };
    if v.is_null() {
        return None;
    }
    let mut n: i32 = 0;
    if unsafe { CFNumberGetValue(v, CF_NUMBER_SINT32, &mut n as *mut i32 as *mut c_void) } {
        Some(n)
    } else {
        None
    }
}

// ── GPU usage via IOAccelerator ───────────────────────────────────────────────

fn gpu_usage_pct() -> Option<f32> {
    let matching = unsafe { IOServiceMatching(b"IOAccelerator\0".as_ptr()) };
    if matching.is_null() {
        return None;
    }
    let mut iter: u32 = 0;
    // IOServiceGetMatchingServices consumes `matching` — no CFRelease needed.
    if unsafe { IOServiceGetMatchingServices(0, matching, &mut iter) } != 0 || iter == 0 {
        return None;
    }

    let mut result: Option<f32> = None;

    loop {
        let service = unsafe { IOIteratorNext(iter) };
        if service == 0 {
            break;
        }
        let mut props: *mut c_void = std::ptr::null_mut();
        if unsafe {
            IORegistryEntryCreateCFProperties(service, &mut props, std::ptr::null(), 0)
        } == 0
            && !props.is_null()
        {
            let perf_key = cf_str(b"PerformanceStatistics\0");
            let perf = unsafe { CFDictionaryGetValue(props, perf_key) };
            unsafe { CFRelease(perf_key) };

            if !perf.is_null() {
                // Apple Silicon reports "Device Utilization %".
                // Some AMD/Intel GPUs report "GPU Activity(GFX)".
                let pct = dict_i32(perf, b"Device Utilization %\0")
                    .or_else(|| dict_i32(perf, b"GPU Activity(GFX)\0"));

                if let Some(p) = pct {
                    result = Some(p.clamp(0, 100) as f32);
                }
            }
            unsafe { CFRelease(props) };
        }
        unsafe { IOObjectRelease(service) };
    }
    unsafe { IOObjectRelease(iter) };

    result
}

// ── public API ────────────────────────────────────────────────────────────────

static PREV_TICKS: Mutex<Option<[u32; 4]>> = Mutex::new(None);

pub struct MetricsFlags {
    pub cpu_usage: bool,
    pub gpu_usage: bool,
    pub ram: bool,
}

pub struct Metrics {
    pub cpu_pct: Option<f32>,
    pub gpu_pct: Option<f32>,
    pub ram_used_gb: f32,
    pub ram_total_gb: f32,
}

pub fn read_metrics(flags: MetricsFlags) -> Metrics {
    // ── CPU usage ─────────────────────────────────────────────────────────────
    let cpu_pct = if flags.cpu_usage {
        let ticks = get_cpu_ticks();
        let mut prev = PREV_TICKS.lock().unwrap();
        let pct = if let (Some(curr), Some(p)) = (ticks, *prev) {
            let d: [u32; 4] = std::array::from_fn(|i| curr[i].wrapping_sub(p[i]));
            let total: u32 = d.iter().sum();
            // 0=user 1=system 2=idle 3=nice
            if total == 0 {
                None
            } else {
                Some((d[0] + d[1] + d[3]) as f32 / total as f32 * 100.0)
            }
        } else {
            None
        };
        *prev = ticks;
        pct
    } else {
        // Reset stored ticks so the next enabled poll gets a fresh delta.
        *PREV_TICKS.lock().unwrap() = None;
        None
    };

    // ── GPU usage ─────────────────────────────────────────────────────────────
    let gpu_pct = if flags.gpu_usage { gpu_usage_pct() } else { None };

    // ── RAM usage ─────────────────────────────────────────────────────────────
    const GB: f32 = 1_073_741_824.0;
    let (ram_used_gb, ram_total_gb) = if flags.ram {
        let page_bytes = sysctl_u32(b"hw.pagesize\0") as u64;
        let total_bytes = sysctl_u64(b"hw.memsize\0");
        if let Some(vm) = get_vm_stats() {
            let used_pages =
                vm.active_count as u64 + vm.wire_count as u64 + vm.compressor_page_count as u64;
            ((used_pages * page_bytes) as f32 / GB, total_bytes as f32 / GB)
        } else {
            (0.0, total_bytes as f32 / GB)
        }
    } else {
        (0.0, 0.0)
    };

    Metrics { cpu_pct, gpu_pct, ram_used_gb, ram_total_gb }
}
