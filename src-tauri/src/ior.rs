// IOReport-based CPU/GPU metrics for Apple Silicon — same approach as macmon
// https://github.com/vladkens/macmon  (MIT)
// No root required. Uses IOReport private framework (same as Activity Monitor).

#![allow(
    non_snake_case,
    non_camel_case_types,
    dead_code,
    clippy::upper_case_acronyms
)]

use core_foundation::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation::base::{kCFAllocatorDefault, CFAllocatorRef, CFIndex, CFRelease, CFTypeRef};
use core_foundation::dictionary::{
    CFDictionaryCreateMutableCopy, CFDictionaryGetCount, CFDictionaryGetValue, CFDictionaryRef,
    CFMutableDictionaryRef,
};
use core_foundation::number::{kCFNumberSInt32Type, CFNumberGetValue, CFNumberRef};
use core_foundation::string::{kCFStringEncodingUTF8, CFStringGetCStringPtr, CFStringRef};
use std::ffi::{c_void, CStr};
use std::mem::MaybeUninit;
use std::ptr::null;

// ── Opaque IOReport type ───────────────────────────────────────────────────────

// Opaque enum — FFI-safe pointer target with no known layout.
#[allow(dead_code)]
enum IOReportSubscription {}
type IOReportSubscriptionRef = *const IOReportSubscription;
type CVoidRef = *const c_void;

// ── IOKit framework ────────────────────────────────────────────────────────────

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOServiceGetMatchingService(mainPort: u32, matching: CFDictionaryRef) -> u32;
    fn IOServiceMatching(name: *const i8) -> CFMutableDictionaryRef;
    fn IOServiceGetMatchingServices(
        mainPort: u32,
        matching: CFDictionaryRef,
        existing: *mut u32,
    ) -> i32;
    fn IOIteratorNext(iterator: u32) -> u32;
    fn IORegistryEntryCreateCFProperties(
        entry: u32,
        properties: *mut CFMutableDictionaryRef,
        allocator: CFAllocatorRef,
        options: u32,
    ) -> i32;
    fn IORegistryEntryGetName(entry: u32, name: *mut i8) -> i32;
    fn IOObjectRelease(obj: u32) -> u32;
    fn IOServiceOpen(service: u32, owner_task: u32, conn_type: u32, connect: *mut u32) -> i32;
    fn IOServiceClose(connect: u32) -> i32;
    fn IOConnectCallStructMethod(
        connect: u32,
        selector: u32,
        input_struct: *const c_void,
        input_struct_cnt: usize,
        output_struct: *mut c_void,
        output_struct_cnt: *mut usize,
    ) -> i32;
    // IOHID event system (private, but accessible — same as macmon)
    fn IOHIDEventSystemClientCreate(alloc: CFAllocatorRef) -> *mut c_void;
    fn IOHIDEventSystemClientSetMatching(client: *mut c_void, matching: CFDictionaryRef);
    fn IOHIDEventSystemClientCopyServices(client: *mut c_void) -> CFArrayRef;
    fn IOHIDServiceClientCopyProperty(svc: *const c_void, key: CFStringRef) -> CFTypeRef;
    fn IOHIDServiceClientCopyEvent(
        svc: *const c_void,
        etype: i64,
        opt: i32,
        timeout: i64,
    ) -> *const c_void;
    fn IOHIDEventGetFloatValue(event: *const c_void, field: i64) -> f64;
}

extern "C" {
    static mach_task_self_: u32;
    // CoreFoundation globals needed for CFDictionaryCreate
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFNumberCreate(alloc: CFAllocatorRef, the_type: i64, value: *const c_void) -> CFNumberRef;
    fn CFDictionaryCreate(
        alloc: CFAllocatorRef,
        keys: *const *const c_void,
        values: *const *const c_void,
        num_values: CFIndex,
        key_cbs: *const c_void,
        val_cbs: *const c_void,
    ) -> CFDictionaryRef;
}

// ── IOReport dylib (private, but always present on macOS 11+) ─────────────────

#[link(name = "IOReport", kind = "dylib")]
extern "C" {
    fn IOReportCopyAllChannels(a: u64, b: u64) -> CFDictionaryRef;
    fn IOReportCopyChannelsInGroup(
        group: CFStringRef,
        subgroup: CFStringRef,
        c: u64,
        d: u64,
        e: u64,
    ) -> CFDictionaryRef;
    fn IOReportMergeChannels(a: CFDictionaryRef, b: CFDictionaryRef, nil: CFTypeRef);
    fn IOReportCreateSubscription(
        a: CVoidRef,
        desired: CFMutableDictionaryRef,
        sub_chan: *mut CFMutableDictionaryRef,
        d: u64,
        e: CFTypeRef,
    ) -> IOReportSubscriptionRef;
    fn IOReportCreateSamples(
        subs: IOReportSubscriptionRef,
        chan: CFMutableDictionaryRef,
        nil: CFTypeRef,
    ) -> CFDictionaryRef;
    fn IOReportCreateSamplesDelta(
        prev: CFDictionaryRef,
        next: CFDictionaryRef,
        nil: CFTypeRef,
    ) -> CFDictionaryRef;
    fn IOReportChannelGetGroup(item: CFDictionaryRef) -> CFStringRef;
    fn IOReportChannelGetSubGroup(item: CFDictionaryRef) -> CFStringRef;
    fn IOReportChannelGetChannelName(item: CFDictionaryRef) -> CFStringRef;
    fn IOReportChannelGetUnitLabel(item: CFDictionaryRef) -> CFStringRef;
    fn IOReportStateGetCount(item: CFDictionaryRef) -> i32;
    fn IOReportStateGetNameForIndex(item: CFDictionaryRef, idx: i32) -> CFStringRef;
    fn IOReportStateGetResidency(item: CFDictionaryRef, idx: i32) -> i64;
    fn IOReportSimpleGetIntegerValue(item: CFDictionaryRef, b: i32) -> i64;
}

// ── SMC temperature reading (no root — same approach as macmon) ───────────────

const SMC_CMD_READ_KEYINFO: u8 = 9;
const SMC_CMD_READ_BYTES: u8 = 5;
const SMC_CMD_READ_INDEX: u8 = 8;
const KERNEL_INDEX_SMC: u32 = 2;
const SMC_TYPE_FLT: u32 = u32::from_be_bytes(*b"flt ");
const SMC_TYPE_SP78: u32 = u32::from_be_bytes(*b"sp78"); // signed 7.8 fixed-point (most temp keys)
const SMC_TYPE_FPE2: u32 = u32::from_be_bytes(*b"fpe2"); // unsigned 14.2 fixed-point

fn smc_fourcc(s: &str) -> u32 {
    s.bytes().take(4).fold(0u32, |acc, b| (acc << 8) | b as u32)
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct SmcVersion {
    major: u8,
    minor: u8,
    build: u8,
    reserved: u8,
    release: u16,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct SmcPLimit {
    version: u16,
    length: u16,
    cpu: u32,
    gpu: u32,
    mem: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct SmcKeyInfo {
    data_size: u32,
    data_type: u32,
    data_attributes: u8,
}

#[repr(C)]
#[derive(Clone)]
struct SmcKeyData {
    key: u32,
    vers: SmcVersion,
    p_limit: SmcPLimit,
    key_info: SmcKeyInfo,
    result: u8,
    status: u8,
    data8: u8,
    data32: u32,
    bytes: [u8; 32],
}

impl Default for SmcKeyData {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

pub struct SmcSensors {
    conn: u32,
    cpu_keys: Vec<u32>, // discovered Tp*/Te* FourCC keys
    gpu_keys: Vec<u32>, // discovered Tg* FourCC keys
}

impl SmcSensors {
    pub fn new() -> Option<Self> {
        let conn = Self::open_connection()?;
        let mut s = Self {
            conn,
            cpu_keys: vec![],
            gpu_keys: vec![],
        };
        s.discover_keys();
        // Only useful if we found at least CPU keys
        if s.cpu_keys.is_empty() {
            return None;
        }
        Some(s)
    }

    fn open_connection() -> Option<u32> {
        unsafe {
            let c_name = std::ffi::CString::new("AppleSMC").ok()?;
            let matching = IOServiceMatching(c_name.as_ptr());
            let mut iter = 0u32;
            if IOServiceGetMatchingServices(0, matching as _, &mut iter) != 0 {
                return None;
            }

            let mut conn = 0u32;
            loop {
                let svc = IOIteratorNext(iter);
                if svc == 0 {
                    break;
                }

                // macmon targets "AppleSMCKeysEndpoint" specifically
                let mut name_buf = [0i8; 128];
                IORegistryEntryGetName(svc, name_buf.as_mut_ptr());
                let entry_name = CStr::from_ptr(name_buf.as_ptr()).to_string_lossy();
                if entry_name == "AppleSMCKeysEndpoint" {
                    let r = IOServiceOpen(svc, mach_task_self_, 0, &mut conn);
                    IOObjectRelease(svc);
                    if r == 0 && conn != 0 {
                        break;
                    }
                    conn = 0;
                } else {
                    IOObjectRelease(svc);
                }
            }
            IOObjectRelease(iter);
            if conn == 0 {
                None
            } else {
                Some(conn)
            }
        }
    }

    fn smc_call(&self, inp: &SmcKeyData) -> Option<SmcKeyData> {
        let mut out = SmcKeyData::default();
        let sz = std::mem::size_of::<SmcKeyData>();
        let mut out_sz = sz;
        let ok = unsafe {
            IOConnectCallStructMethod(
                self.conn,
                KERNEL_INDEX_SMC,
                inp as *const _ as _,
                sz,
                &mut out as *mut _ as _,
                &mut out_sz,
            )
        } == 0;
        if ok && out.result == 0 {
            Some(out)
        } else {
            None
        }
    }

    fn key_info(&self, key: u32) -> Option<SmcKeyInfo> {
        let inp = SmcKeyData {
            key,
            data8: SMC_CMD_READ_KEYINFO,
            ..Default::default()
        };
        Some(self.smc_call(&inp)?.key_info)
    }

    fn read_bytes(&self, key: u32, info: SmcKeyInfo) -> Option<[u8; 32]> {
        let inp = SmcKeyData {
            key,
            key_info: info,
            data8: SMC_CMD_READ_BYTES,
            ..Default::default()
        };
        Some(self.smc_call(&inp)?.bytes)
    }

    fn key_at_index(&self, idx: u32) -> Option<u32> {
        let inp = SmcKeyData {
            data8: SMC_CMD_READ_INDEX,
            data32: idx,
            ..Default::default()
        };
        Some(self.smc_call(&inp)?.key)
    }

    fn key_count(&self) -> u32 {
        let k = smc_fourcc("#KEY");
        let info = match self.key_info(k) {
            Some(i) => i,
            None => return 0,
        };
        let bytes = match self.read_bytes(k, info) {
            Some(b) => b,
            None => return 0,
        };
        u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }

    fn discover_keys(&mut self) {
        let count = self.key_count().min(1000);
        for i in 0..count {
            let key_fourcc = match self.key_at_index(i) {
                Some(k) => k,
                None => continue,
            };
            let info = match self.key_info(key_fourcc) {
                Some(i) => i,
                None => continue,
            };

            // Accept flt (4B float), sp78 (2B signed 7.8), fpe2 (2B unsigned 14.2)
            let is_temp_type = matches!(info.data_type, t if t == SMC_TYPE_FLT || t == SMC_TYPE_SP78 || t == SMC_TYPE_FPE2);
            if !is_temp_type {
                continue;
            }

            let b = key_fourcc.to_be_bytes();
            let name = match std::str::from_utf8(&b) {
                Ok(s) => s,
                Err(_) => continue,
            };

            if name.starts_with("Tp") || name.starts_with("Te") || name.starts_with("Ts") {
                self.cpu_keys.push(key_fourcc);
            } else if name.starts_with("Tg") || name.starts_with("TG") {
                self.gpu_keys.push(key_fourcc);
            }
        }
    }

    fn read_flt(&self, key: u32) -> Option<f32> {
        let info = self.key_info(key)?;
        let bytes = self.read_bytes(key, info)?;
        let v = match info.data_type {
            SMC_TYPE_FLT => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            SMC_TYPE_SP78 => i16::from_be_bytes([bytes[0], bytes[1]]) as f32 / 256.0,
            SMC_TYPE_FPE2 => u16::from_be_bytes([bytes[0], bytes[1]]) as f32 / 4.0,
            _ => return None,
        };
        if v > 0.0 && v <= 150.0 {
            Some(v)
        } else {
            None
        }
    }

    fn avg_keys(&self, keys: &[u32]) -> f32 {
        let vals: Vec<f32> = keys.iter().filter_map(|&k| self.read_flt(k)).collect();
        if vals.is_empty() {
            0.0
        } else {
            vals.iter().sum::<f32>() / vals.len() as f32
        }
    }

    pub fn read_temps(&self) -> (f32, f32) {
        (self.avg_keys(&self.cpu_keys), self.avg_keys(&self.gpu_keys))
    }
}

impl Drop for SmcSensors {
    fn drop(&mut self) {
        unsafe {
            IOServiceClose(self.conn);
        }
    }
}

// ── IOHID temperature sensors (M1 / fallback when SMC GPU keys absent) ────────
// Sensor name prefixes from macmon:
//   CPU: "pACC MTR Temp Sensor" (P-cores), "eACC MTR Temp Sensor" (E-cores)
//   GPU: "GPU MTR Temp Sensor"

const K_HID_PAGE_APPLE_VENDOR: i32 = 0xff00;
const K_HID_USAGE_APPLE_TEMP: i32 = 0x0005;
const K_IOHID_TYPE_TEMP: i64 = 15;
const K_CF_NUMBER_SINT32: i64 = 3;

pub struct IohidSensors;

pub struct IohidAllTemps {
    pub cpu: f32,  // pACC + eACC MTR average
    pub gpu: f32,  // GPU MTR average
    pub soc: f32,  // SOC MTR Die average
    pub nand: f32, // NAND CH0 temp
    pub ane: f32,  // ANE MTR average
}

impl IohidSensors {
    pub fn get_all_temps() -> IohidAllTemps {
        let zero = IohidAllTemps {
            cpu: 0.0,
            gpu: 0.0,
            soc: 0.0,
            nand: 0.0,
            ane: 0.0,
        };
        unsafe {
            let page_key = cfstr("PrimaryUsagePage");
            let usage_key = cfstr("PrimaryUsage");
            let page_val: i32 = K_HID_PAGE_APPLE_VENDOR;
            let usage_val: i32 = K_HID_USAGE_APPLE_TEMP;
            let pn = CFNumberCreate(
                kCFAllocatorDefault,
                K_CF_NUMBER_SINT32,
                &page_val as *const _ as _,
            );
            let un = CFNumberCreate(
                kCFAllocatorDefault,
                K_CF_NUMBER_SINT32,
                &usage_val as *const _ as _,
            );
            let keys: [*const c_void; 2] = [page_key as _, usage_key as _];
            let values: [*const c_void; 2] = [pn as _, un as _];
            let matching = CFDictionaryCreate(
                kCFAllocatorDefault,
                keys.as_ptr(),
                values.as_ptr(),
                2,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            );
            CFRelease(pn as _);
            CFRelease(un as _);
            CFRelease(page_key as _);
            CFRelease(usage_key as _);

            let client = IOHIDEventSystemClientCreate(kCFAllocatorDefault);
            if client.is_null() {
                CFRelease(matching as _);
                return zero;
            }
            IOHIDEventSystemClientSetMatching(client, matching);
            CFRelease(matching as _);

            let services = IOHIDEventSystemClientCopyServices(client);
            if services.is_null() {
                CFRelease(client as _);
                return zero;
            }

            let count = CFArrayGetCount(services);
            let mut cpu_v: Vec<f32> = vec![];
            let mut gpu_v: Vec<f32> = vec![];
            let mut soc_v: Vec<f32> = vec![];
            let mut nand_v: Vec<f32> = vec![];
            let mut ane_v: Vec<f32> = vec![];

            for i in 0..count {
                let svc = CFArrayGetValueAtIndex(services, i);
                if svc.is_null() {
                    continue;
                }

                let prod_key = cfstr("Product");
                let name_ref = IOHIDServiceClientCopyProperty(svc, prod_key);
                CFRelease(prod_key as _);
                if name_ref.is_null() {
                    continue;
                }
                let name = from_cfstr(name_ref as CFStringRef);
                CFRelease(name_ref as _);

                let event = IOHIDServiceClientCopyEvent(svc, K_IOHID_TYPE_TEMP, 0, 0);
                if event.is_null() {
                    continue;
                }
                let temp = IOHIDEventGetFloatValue(event, K_IOHID_TYPE_TEMP << 16) as f32;
                CFRelease(event as _);

                // Ignore invalid readings (< 5°C or > 120°C, or the -21.4 dead sensors)
                if !(5.0..=120.0).contains(&temp) {
                    continue;
                }

                if name.contains("pACC") || name.contains("eACC") {
                    cpu_v.push(temp);
                } else if name.contains("GPU MTR") {
                    gpu_v.push(temp);
                } else if name.contains("SOC MTR") || name.contains("PMGR SOC") {
                    soc_v.push(temp);
                } else if name.contains("NAND") {
                    nand_v.push(temp);
                } else if name.contains("ANE MTR") {
                    ane_v.push(temp);
                }
            }
            CFRelease(services as _);
            CFRelease(client as _);

            let avg = |v: &[f32]| {
                if v.is_empty() {
                    0.0
                } else {
                    v.iter().sum::<f32>() / v.len() as f32
                }
            };
            IohidAllTemps {
                cpu: avg(&cpu_v),
                gpu: avg(&gpu_v),
                soc: avg(&soc_v),
                nand: avg(&nand_v),
                ane: avg(&ane_v),
            }
        }
    }

    // Compatibility shim used by SmcSensors fallback path
    pub fn get_temps() -> (f32, f32) {
        let t = Self::get_all_temps();
        (t.cpu, t.gpu)
    }
}

// ── CoreFoundation helpers ─────────────────────────────────────────────────────

fn cfstr(s: &str) -> CFStringRef {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;
    let cf = CFString::new(s);
    let raw = cf.as_concrete_TypeRef();
    std::mem::forget(cf); // caller must release
    raw
}

fn from_cfstr(s: CFStringRef) -> String {
    if s.is_null() {
        return String::new();
    }
    unsafe {
        let ptr = CFStringGetCStringPtr(s, kCFStringEncodingUTF8);
        if !ptr.is_null() {
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        } else {
            // Fallback for strings that don't have a direct C-string pointer
            use core_foundation::base::TCFType;
            use core_foundation::string::CFString;
            CFString::wrap_under_get_rule(s).to_string()
        }
    }
}

// ── IOReport channel subscription ─────────────────────────────────────────────

fn cfio_get_chan(
    channels: &[(&str, Option<&str>)],
) -> Result<CFMutableDictionaryRef, &'static str> {
    if channels.is_empty() {
        let c = unsafe { IOReportCopyAllChannels(0, 0) };
        let r = unsafe {
            CFDictionaryCreateMutableCopy(kCFAllocatorDefault, CFDictionaryGetCount(c), c)
        };
        unsafe { CFRelease(c as _) };
        return Ok(r);
    }

    let mut parts: Vec<CFDictionaryRef> = Vec::new();
    for (group, subgroup) in channels {
        let gname = cfstr(group);
        let sname = subgroup.map_or(null(), cfstr);
        let chan = unsafe { IOReportCopyChannelsInGroup(gname, sname, 0, 0, 0) };
        parts.push(chan);
        unsafe { CFRelease(gname as _) };
        if subgroup.is_some() {
            unsafe { CFRelease(sname as _) }
        };
    }

    let first = parts[0];
    for part in parts.iter().skip(1) {
        unsafe { IOReportMergeChannels(first, *part, null()) };
    }

    let size = unsafe { CFDictionaryGetCount(first) };
    let merged = unsafe { CFDictionaryCreateMutableCopy(kCFAllocatorDefault, size, first) };

    for p in &parts {
        unsafe { CFRelease(*p as _) };
    }

    Ok(merged)
}

// Returns (subscription_ref, subscribed_channels_dict).
// The subscribed_channels_dict (sub_chan) is what IOReportCreateSamples must receive,
// NOT the original desired-channels dict — using the wrong one yields no data.
fn cfio_get_subs(
    chan: CFMutableDictionaryRef,
) -> Result<(IOReportSubscriptionRef, CFMutableDictionaryRef), &'static str> {
    let mut sub_chan: CFMutableDictionaryRef = std::ptr::null_mut();
    let subs = unsafe { IOReportCreateSubscription(null(), chan, &mut sub_chan, 0, null()) };
    if subs.is_null() || sub_chan.is_null() {
        return Err("IOReportCreateSubscription failed");
    }
    Ok((subs, sub_chan))
}

// ── Residency extraction ───────────────────────────────────────────────────────

fn cfio_get_residencies(item: CFDictionaryRef) -> Vec<(String, i64)> {
    let count = unsafe { IOReportStateGetCount(item) };
    let mut result = Vec::with_capacity(count as usize);
    for i in 0..count {
        let name = from_cfstr(unsafe { IOReportStateGetNameForIndex(item, i) });
        let val = unsafe { IOReportStateGetResidency(item, i) };
        result.push((name, val));
    }
    result
}

// Calculate frequency usage from residency states.
// Returns (avg_freq_mhz, usage_fraction 0.0–1.0)
fn calc_freq(item: CFDictionaryRef, freqs: &[u32]) -> (u32, f32) {
    let residencies = cfio_get_residencies(item);
    if residencies.is_empty() || freqs.is_empty() {
        return (0, 0.0);
    }

    // Find where active states begin (skip IDLE/DOWN/OFF)
    let offset = residencies
        .iter()
        .position(|(name, _)| name != "IDLE" && name != "DOWN" && name != "OFF")
        .unwrap_or(0);

    let total: f64 = residencies.iter().map(|(_, v)| *v as f64).sum();
    let active: f64 = residencies
        .iter()
        .skip(offset)
        .map(|(_, v)| *v as f64)
        .sum();

    if total == 0.0 {
        return (0, 0.0);
    }

    let n = freqs.len().min(residencies.len().saturating_sub(offset));
    let mut avg_freq = 0f64;
    for i in 0..n {
        let res = residencies[offset + i].1 as f64;
        let pct = if active > 0.0 { res / active } else { 0.0 };
        avg_freq += pct * freqs[i] as f64;
    }

    let usage = active / total;
    let max_freq = *freqs.last().unwrap_or(&1) as f64;
    let min_freq = *freqs.first().unwrap_or(&1) as f64;
    let from_max = (avg_freq.max(min_freq) * usage) / max_freq;

    (avg_freq as u32, from_max.min(1.0) as f32)
}

// ── IOReport item iterator ─────────────────────────────────────────────────────

struct IOReportIterator {
    sample: CFDictionaryRef,
}

impl IOReportIterator {
    fn new(sample: CFDictionaryRef) -> Self {
        Self { sample }
    }

    fn items(&self) -> Vec<IOReportItem> {
        // IOReport sample is a dict with a key "IOReportChannels" → CFArray of channel dicts
        use core_foundation::base::TCFType;
        use core_foundation::string::CFString;
        let key = CFString::new("IOReportChannels");
        let arr = unsafe {
            CFDictionaryGetValue(self.sample, key.as_concrete_TypeRef() as _) as CFArrayRef
        };
        if arr.is_null() {
            return vec![];
        }

        let count = unsafe { CFArrayGetCount(arr) };
        let mut result = Vec::with_capacity(count as usize);
        for i in 0..count {
            let item = unsafe { CFArrayGetValueAtIndex(arr, i) as CFDictionaryRef };
            if item.is_null() {
                continue;
            }
            result.push(IOReportItem {
                group: from_cfstr(unsafe { IOReportChannelGetGroup(item) })
                    .trim()
                    .to_owned(),
                subgroup: from_cfstr(unsafe { IOReportChannelGetSubGroup(item) })
                    .trim()
                    .to_owned(),
                channel: from_cfstr(unsafe { IOReportChannelGetChannelName(item) })
                    .trim()
                    .to_owned(),
                unit: from_cfstr(unsafe { IOReportChannelGetUnitLabel(item) })
                    .trim()
                    .to_owned(),
                item,
            });
        }
        result
    }
}

impl Drop for IOReportIterator {
    fn drop(&mut self) {
        if !self.sample.is_null() {
            unsafe { CFRelease(self.sample as _) };
        }
    }
}

struct IOReportItem {
    pub group: String,
    pub subgroup: String,
    pub channel: String,
    pub unit: String,
    pub item: CFDictionaryRef,
}

// ── SocInfo — chip frequencies from IOKit ─────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SocInfo {
    pub ecpu_freqs: Vec<u32>, // MHz
    pub pcpu_freqs: Vec<u32>, // MHz
    pub gpu_freqs: Vec<u32>,  // MHz
    pub ecpu_cores: u32,
    pub pcpu_cores: u32,
    pub gpu_cores: u32,
}

impl Default for SocInfo {
    fn default() -> Self {
        // Sensible defaults for M1 13" MBP if IOKit reads fail
        Self {
            ecpu_freqs: vec![600, 972, 1332, 1704, 2064],
            pcpu_freqs: vec![600, 828, 1056, 1296, 1524, 1752, 1980, 2064, 2988, 3204],
            gpu_freqs: vec![396, 528, 720, 912, 1080, 1296],
            ecpu_cores: 4,
            pcpu_cores: 4,
            gpu_cores: 8,
        }
    }
}

type CFDataRef = *const c_void;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFDataGetBytePtr(data: CFDataRef) -> *const u8;
    fn CFDataGetLength(data: CFDataRef) -> CFIndex;
}

fn read_freq_table(props: CFDictionaryRef, key: &str) -> Vec<u32> {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;

    let cf_key = CFString::new(key);
    let data = unsafe { CFDictionaryGetValue(props, cf_key.as_concrete_TypeRef() as _) };
    if data.is_null() {
        return vec![];
    }

    let data = data as CFDataRef;
    let len = unsafe { CFDataGetLength(data) } as usize;
    let ptr = unsafe { CFDataGetBytePtr(data) };
    if ptr.is_null() || len < 8 {
        return vec![];
    }

    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    // Each entry: 4-byte frequency in MHz (LE) + 4-byte voltage — pick freq only
    bytes
        .chunks_exact(8)
        .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .filter(|&f| f > 0 && f < 10000)
        .collect()
}

impl SocInfo {
    pub fn new() -> Self {
        Self::from_iokit().unwrap_or_default()
    }

    fn from_iokit() -> Option<Self> {
        unsafe {
            // Find the CPU performance state node in IOKit registry
            // macmon uses: IOServiceGetMatchingServices → "AppleARMIODevice"
            let c_name = std::ffi::CString::new("AppleARMIODevice").ok()?;
            let matching = IOServiceMatching(c_name.as_ptr());
            let mut iter: u32 = 0;
            if IOServiceGetMatchingServices(0, matching as _, &mut iter) != 0 {
                return None;
            }

            let mut result: Option<Self> = None;

            loop {
                let service = IOIteratorNext(iter);
                if service == 0 {
                    break;
                }

                let mut props: MaybeUninit<CFMutableDictionaryRef> = MaybeUninit::uninit();
                if IORegistryEntryCreateCFProperties(
                    service,
                    props.as_mut_ptr(),
                    kCFAllocatorDefault,
                    0,
                ) != 0
                {
                    IOObjectRelease(service);
                    continue;
                }
                let props = props.assume_init();

                // Look for the node that has "voltage-states5-sram" (PCPU frequencies)
                use core_foundation::base::TCFType;
                use core_foundation::string::CFString;
                let check_key = CFString::new("voltage-states5-sram");
                let has_pcpu =
                    !CFDictionaryGetValue(props, check_key.as_concrete_TypeRef() as _).is_null();

                if has_pcpu {
                    let ecpu = read_freq_table(props, "voltage-states1-sram");
                    let pcpu = read_freq_table(props, "voltage-states5-sram");
                    let gpu = read_freq_table(props, "voltage-states9");

                    // Read core counts from the same node
                    let ecpu_cores = read_u32_prop(props, "e-core-count").unwrap_or(4);
                    let pcpu_cores = read_u32_prop(props, "p-core-count").unwrap_or(4);
                    let gpu_cores = read_u32_prop(props, "gpu-core-count").unwrap_or(8);

                    result = Some(SocInfo {
                        ecpu_freqs: if ecpu.is_empty() {
                            SocInfo::default().ecpu_freqs
                        } else {
                            ecpu
                        },
                        pcpu_freqs: if pcpu.is_empty() {
                            SocInfo::default().pcpu_freqs
                        } else {
                            pcpu
                        },
                        gpu_freqs: if gpu.is_empty() {
                            SocInfo::default().gpu_freqs
                        } else {
                            gpu
                        },
                        ecpu_cores,
                        pcpu_cores,
                        gpu_cores,
                    });

                    CFRelease(props as _);
                    IOObjectRelease(service);
                    break;
                }

                CFRelease(props as _);
                IOObjectRelease(service);
            }

            IOObjectRelease(iter);
            result
        }
    }
}

fn read_u32_prop(props: CFDictionaryRef, key: &str) -> Option<u32> {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;
    let cf_key = CFString::new(key);
    let val = unsafe { CFDictionaryGetValue(props, cf_key.as_concrete_TypeRef() as _) };
    if val.is_null() {
        return None;
    }
    let mut out: i32 = 0;
    let ok = unsafe {
        CFNumberGetValue(
            val as CFNumberRef,
            kCFNumberSInt32Type,
            &mut out as *mut _ as _,
        )
    };
    if ok {
        Some(out as u32)
    } else {
        None
    }
}

// ── Sampler ────────────────────────────────────────────────────────────────────

pub struct Sampler {
    subs: IOReportSubscriptionRef,
    sub_chan: CFMutableDictionaryRef,
    soc: SocInfo,
    smc: Option<SmcSensors>,
}

unsafe impl Send for Sampler {}
unsafe impl Send for SmcSensors {}

pub struct Metrics {
    pub gpu_pct: f32,
    pub ecpu_pct: f32,
    pub pcpu_pct: f32,
    pub cpu_pct: f32,
    // Temperatures (°C), 0.0 = unavailable
    pub cpu_temp: f32,
    pub gpu_temp: f32,
    pub soc_temp: f32,
    pub nand_temp: f32,
    pub ane_temp: f32,
    // Power (Watts), 0.0 = unavailable / idle
    pub cpu_power: f32,
    pub gpu_power: f32,
    pub ram_power: f32,
    pub ane_power: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            gpu_pct: 0.0,
            ecpu_pct: 0.0,
            pcpu_pct: 0.0,
            cpu_pct: 0.0,
            cpu_temp: 0.0,
            gpu_temp: 0.0,
            soc_temp: 0.0,
            nand_temp: 0.0,
            ane_temp: 0.0,
            cpu_power: 0.0,
            gpu_power: 0.0,
            ram_power: 0.0,
            ane_power: 0.0,
        }
    }
}

const GPU_FREQ_SUBG: &str = "GPU Performance States";
const CPU_FREQ_SUBG: &str = "CPU Core Performance States";

fn ior_watts(item: CFDictionaryRef, unit: &str, duration_ms: u64) -> f32 {
    let raw = unsafe { IOReportSimpleGetIntegerValue(item, 0) } as f64;
    let secs = duration_ms as f64 / 1000.0;
    let w = match unit {
        "mJ" => raw / 1e3 / secs,
        "uJ" => raw / 1e6 / secs,
        "nJ" => raw / 1e9 / secs,
        _ => return 0.0,
    };
    w.max(0.0) as f32
}

impl Sampler {
    pub fn new() -> Result<Self, &'static str> {
        let channels = [
            ("GPU Stats", Some("GPU Performance States")),
            ("CPU Stats", Some("CPU Core Performance States")),
            ("Energy Model", None),
        ];

        let desired = cfio_get_chan(&channels)?;
        let (subs, sub_chan) = cfio_get_subs(desired)?;
        unsafe { CFRelease(desired as _) };
        let soc = SocInfo::new();
        let smc = SmcSensors::new(); // None on failure (non-fatal)

        Ok(Self {
            subs,
            sub_chan,
            soc,
            smc,
        })
    }

    pub fn get_metrics(&self, interval_ms: u64) -> Metrics {
        // Take 4 sub-samples and average — same strategy as macmon for stability
        let n: u64 = 4;
        let sub_ms = (interval_ms / n).max(100);

        let mut gpu_pct = 0.0f32;
        let mut ecpu_usages: Vec<f32> = Vec::new();
        let mut pcpu_usages: Vec<f32> = Vec::new();
        let mut cpu_power_sum = 0.0f32;
        let mut gpu_power_sum = 0.0f32;
        let mut ram_power_sum = 0.0f32;
        let mut ane_power_sum = 0.0f32;
        let mut power_samples = 0u32;

        for _ in 0..n {
            let t0 = std::time::Instant::now();
            let (s1, s2) = unsafe {
                let s1 = IOReportCreateSamples(self.subs, self.sub_chan, null());
                std::thread::sleep(std::time::Duration::from_millis(sub_ms));
                let s2 = IOReportCreateSamples(self.subs, self.sub_chan, null());
                (s1, s2)
            };
            let elapsed_ms = t0.elapsed().as_millis() as u64;
            let elapsed_ms = elapsed_ms.max(1);

            let delta = unsafe { IOReportCreateSamplesDelta(s1, s2, null()) };
            unsafe {
                CFRelease(s1 as _);
                CFRelease(s2 as _)
            };

            let iter = IOReportIterator::new(delta);
            let items = iter.items();

            let mut found_power = false;
            for x in &items {
                match x.group.as_str() {
                    "GPU Stats" if x.subgroup == GPU_FREQ_SUBG && x.channel == "GPUPH" => {
                        let (_, pct) = calc_freq(x.item, &self.soc.gpu_freqs[1..]);
                        gpu_pct = pct * 100.0;
                    }
                    "CPU Stats" if x.subgroup == CPU_FREQ_SUBG => {
                        if x.channel.contains("ECPU") || x.channel.contains("MCPU") {
                            let (_, pct) = calc_freq(x.item, &self.soc.ecpu_freqs);
                            ecpu_usages.push(pct * 100.0);
                        } else if x.channel.contains("PCPU") {
                            let (_, pct) = calc_freq(x.item, &self.soc.pcpu_freqs);
                            pcpu_usages.push(pct * 100.0);
                        }
                    }
                    "Energy Model" => {
                        let w = ior_watts(x.item, &x.unit, elapsed_ms);
                        if x.channel.ends_with("CPU Energy") {
                            cpu_power_sum += w;
                            found_power = true;
                        } else if x.channel == "GPU Energy" {
                            gpu_power_sum += w;
                            found_power = true;
                        } else if x.channel.starts_with("DRAM") || x.channel.starts_with("DDR") {
                            ram_power_sum += w;
                            found_power = true;
                        } else if x.channel.starts_with("ANE") {
                            ane_power_sum += w;
                        }
                    }
                    _ => {}
                }
            }
            if found_power {
                power_samples += 1;
            }
        }

        let n_pow = power_samples.max(1) as f32;
        let cpu_power = cpu_power_sum / n_pow;
        let gpu_power = gpu_power_sum / n_pow;
        let ram_power = ram_power_sum / n_pow;
        let ane_power = ane_power_sum / n_pow;

        ecpu_usages.retain(|&p| p >= 0.0);
        pcpu_usages.retain(|&p| p >= 0.0);

        let ecpu_pct = avg(&ecpu_usages);
        let pcpu_pct = avg(&pcpu_usages);
        let ec = self.soc.ecpu_cores as f32;
        let pc = self.soc.pcpu_cores as f32;
        let cpu_pct = (ecpu_pct * ec + pcpu_pct * pc) / (ec + pc).max(1.0);

        // Always use IOHID for full sensor coverage (SoC, NAND, ANE, GPU)
        // SMC only used to cross-check CPU if IOHID CPU reads are absent
        let iohid = IohidSensors::get_all_temps();
        let cpu_temp = if iohid.cpu > 0.0 {
            iohid.cpu
        } else if let Some(s) = &self.smc {
            s.avg_keys(&s.cpu_keys)
        } else {
            0.0
        };
        let gpu_temp = iohid.gpu;
        let soc_temp = iohid.soc;
        let nand_temp = iohid.nand;
        let ane_temp = iohid.ane;

        Metrics {
            gpu_pct,
            ecpu_pct,
            pcpu_pct,
            cpu_pct,
            cpu_temp,
            gpu_temp,
            soc_temp,
            nand_temp,
            ane_temp,
            cpu_power,
            gpu_power,
            ram_power,
            ane_power,
        }
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        unsafe { CFRelease(self.sub_chan as _) };
    }
}

fn avg(v: &[f32]) -> f32 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f32>() / v.len() as f32
    }
}
