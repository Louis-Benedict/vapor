use std::os::raw::c_void;
use std::sync::{Mutex, OnceLock};

type IoObject = u32;
type IoService = IoObject;
type IoConnect = IoObject;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const u8) -> *mut c_void;
    fn IOServiceGetMatchingService(master_port: u32, matching: *const c_void) -> IoService;
    fn IOObjectRelease(object: IoObject) -> i32;
    fn IOServiceOpen(
        service: IoService,
        owning_task: *mut c_void,
        type_: u32,
        connect: *mut IoConnect,
    ) -> i32;
    fn IOServiceClose(connect: IoConnect) -> i32;
    fn IOConnectCallStructMethod(
        connection: IoConnect,
        selector: u32,
        input: *const c_void,
        input_size: usize,
        output: *mut c_void,
        output_size: *mut usize,
    ) -> i32;
}

unsafe extern "C" {
    fn mach_task_self() -> *mut c_void;
}

const SMC_CMD_GET_KEY_INFO: u8 = 9;
const SMC_CMD_READ_KEY: u8 = 5;
const SMC_CMD_GET_KEY_FROM_INDEX: u8 = 8;

const TYPE_SP78: u32 = u32::from_be_bytes([b's', b'p', b'7', b'8']);
const TYPE_FLT: u32 = u32::from_be_bytes([b'f', b'l', b't', b' ']);

#[repr(C)]
#[derive(Default, Copy, Clone)]
struct SmcVersion {
    major: u8,
    minor: u8,
    build: u8,
    reserved: u8,
    release: u16,
}

// size 16, alignment 4
#[repr(C)]
#[derive(Default, Copy, Clone)]
struct SmcPLimit {
    version: u16,
    length: u16,
    cpu_plimit: u32,
    gpu_plimit: u32,
    mem_plimit: u32,
}

// size 12 (9 data + 3 trailing pad), alignment 4
#[repr(C)]
#[derive(Default, Copy, Clone)]
struct SmcKeyInfo {
    data_size: u32,
    data_type: u32,
    data_attributes: u8,
}

// Layout (total 80 bytes):
//  0: key (u32)
//  4: vers (6)
// 10: _pad1 (2) — aligns p_limit to offset 12
// 12: p_limit (16)
// 28: key_info (12)
// 40: result, status, selector, _pad2
// 44: data32 (u32)
// 48: bytes ([u8; 32])
#[repr(C)]
#[derive(Default, Copy, Clone)]
struct SmcParam {
    key: u32,
    vers: SmcVersion,
    _pad1: [u8; 2],
    p_limit: SmcPLimit,
    key_info: SmcKeyInfo,
    result: u8,
    status: u8,
    selector: u8,
    _pad2: u8,
    data32: u32,
    bytes: [u8; 32],
}

struct Smc {
    conn: IoConnect,
}

// IoConnect is a Mach port (u32) usable from any thread once opened.
unsafe impl Send for Smc {}

impl Smc {
    fn open() -> Option<Self> {
        unsafe {
            let service = IOServiceGetMatchingService(
                0, // kIOMasterPortDefault
                IOServiceMatching(b"AppleSMC\0".as_ptr()),
            );
            if service == 0 {
                return None;
            }
            let mut conn: IoConnect = 0;
            let result = IOServiceOpen(service, mach_task_self(), 0, &mut conn);
            IOObjectRelease(service);
            if result != 0 {
                return None;
            }
            Some(Smc { conn })
        }
    }

    fn call(&self, input: &SmcParam) -> Option<SmcParam> {
        let mut output = SmcParam::default();
        let input_size = size_of::<SmcParam>();
        let mut output_size = size_of::<SmcParam>();
        let result = unsafe {
            IOConnectCallStructMethod(
                self.conn,
                2, // kSMCHandleYPCEvent
                input as *const SmcParam as *const c_void,
                input_size,
                &mut output as *mut SmcParam as *mut c_void,
                &mut output_size,
            )
        };
        if result == 0 && output.result == 0 {
            Some(output)
        } else {
            None
        }
    }

    fn get_key_info(&self, key: u32) -> Option<SmcKeyInfo> {
        let input = SmcParam {
            key,
            selector: SMC_CMD_GET_KEY_INFO,
            ..Default::default()
        };
        self.call(&input).map(|o| o.key_info)
    }

    // One IOKit call per read — uses pre-fetched key info.
    fn read_key_bytes(&self, key: u32, info: SmcKeyInfo) -> Option<[u8; 32]> {
        let input = SmcParam {
            key,
            key_info: info,
            selector: SMC_CMD_READ_KEY,
            ..Default::default()
        };
        self.call(&input).map(|o| o.bytes)
    }

    // Two IOKit calls — used only during cache warm-up.
    fn read_key(&self, key: u32) -> Option<(SmcKeyInfo, [u8; 32])> {
        let info = self.get_key_info(key)?;
        let bytes = self.read_key_bytes(key, info)?;
        Some((info, bytes))
    }

    fn key_count(&self) -> u32 {
        let key = u32::from_be_bytes(*b"#KEY");
        match self.read_key(key) {
            Some((_, bytes)) => u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            None => 0,
        }
    }

    fn key_at_index(&self, index: u32) -> Option<u32> {
        let input = SmcParam {
            selector: SMC_CMD_GET_KEY_FROM_INDEX,
            data32: index,
            ..Default::default()
        };
        self.call(&input).map(|o| o.key)
    }
}

impl Drop for Smc {
    fn drop(&mut self) {
        unsafe { IOServiceClose(self.conn) };
    }
}

fn decode_temp(info: &SmcKeyInfo, bytes: &[u8; 32]) -> Option<f32> {
    let t = match info.data_type {
        TYPE_SP78 if info.data_size >= 2 => {
            i16::from_be_bytes([bytes[0], bytes[1]]) as f32 / 256.0
        }
        TYPE_FLT if info.data_size >= 4 => {
            f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        }
        _ => return None,
    };
    if (1.0..120.0).contains(&t) { Some(t) } else { None }
}

// Cached list of (key_code, key_info) for CPU and GPU sensors.
// Populated once on first call; subsequent polls skip enumeration entirely.
struct SmcCache {
    smc: Smc,
    cpu_keys: Vec<(u32, SmcKeyInfo)>,
    gpu_keys: Vec<(u32, SmcKeyInfo)>,
}

impl SmcCache {
    fn build() -> Option<Self> {
        let smc = Smc::open()?;
        let count = smc.key_count();
        let mut cpu_keys = Vec::new();
        let mut gpu_keys = Vec::new();

        for i in 0..count {
            let Some(key) = smc.key_at_index(i) else { continue };
            let kb = key.to_be_bytes();
            if kb[0] != b'T' {
                continue;
            }
            let is_gpu = kb[1] == b'g';
            let is_cpu = kb[1] == b'p'; // Tp* = CPU package/cluster on Apple Silicon
            if !is_gpu && !is_cpu {
                continue;
            }
            if let Some(info) = smc.get_key_info(key) {
                // Only keep keys whose type we can decode as a temperature.
                let decodable = matches!(info.data_type, TYPE_SP78 | TYPE_FLT);
                if !decodable {
                    continue;
                }
                if is_gpu {
                    gpu_keys.push((key, info));
                } else {
                    cpu_keys.push((key, info));
                }
            }
        }

        Some(SmcCache { smc, cpu_keys, gpu_keys })
    }

    fn read_temps(&self, want_cpu: bool, want_gpu: bool) -> Temps {
        let avg = |keys: &[(u32, SmcKeyInfo)]| -> Option<f32> {
            let mut sum = 0f32;
            let mut n = 0u32;
            for &(key, info) in keys {
                if let Some(bytes) = self.smc.read_key_bytes(key, info) {
                    if let Some(t) = decode_temp(&info, &bytes) {
                        sum += t;
                        n += 1;
                    }
                }
            }
            if n > 0 { Some(sum / n as f32) } else { None }
        };

        Temps {
            cpu: if want_cpu { avg(&self.cpu_keys) } else { None },
            gpu: if want_gpu { avg(&self.gpu_keys) } else { None },
        }
    }
}

static CACHE: OnceLock<Mutex<Option<SmcCache>>> = OnceLock::new();

pub struct Temps {
    pub cpu: Option<f32>,
    pub gpu: Option<f32>,
}

pub fn read_temps(want_cpu: bool, want_gpu: bool) -> Temps {
    if !want_cpu && !want_gpu {
        return Temps { cpu: None, gpu: None };
    }
    let mut guard = CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap();

    if guard.is_none() {
        *guard = SmcCache::build();
    }

    match guard.as_ref() {
        Some(cache) => cache.read_temps(want_cpu, want_gpu),
        None => Temps { cpu: None, gpu: None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn smc_reads_temps() {
        let temps = read_temps(true, true);
        let cpu = temps.cpu.expect("expected CPU temperature from SMC");
        let gpu = temps.gpu.expect("expected GPU temperature from SMC");
        assert!((1.0..120.0).contains(&cpu), "CPU temp {cpu} out of range");
        assert!((1.0..120.0).contains(&gpu), "GPU temp {gpu} out of range");
        println!("CPU {cpu:.1}°C  GPU {gpu:.1}°C");
    }
}
