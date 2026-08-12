pub mod duplicates;
pub mod gpu;
mod guard;
mod ior;

extern crate libc;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

// ── ClamAV static PID store ───────────────────────────────────────────────────

static SCAN_PID: OnceLock<Mutex<Option<u32>>> = OnceLock::new();
fn scan_pid_store() -> &'static Mutex<Option<u32>> {
    SCAN_PID.get_or_init(|| Mutex::new(None))
}

static QUICK_SYS: OnceLock<Mutex<sysinfo::System>> = OnceLock::new();
// All IOReport/SMC values cached × 10 (one decimal place stored as integer)
static GPU_USAGE_X10: AtomicU32 = AtomicU32::new(0);
static FAN_RPM: AtomicU32 = AtomicU32::new(0);
static CPU_TEMP_X10: AtomicU32 = AtomicU32::new(0);
static GPU_TEMP_X10: AtomicU32 = AtomicU32::new(0);
static SOC_TEMP_X10: AtomicU32 = AtomicU32::new(0);
static NAND_TEMP_X10: AtomicU32 = AtomicU32::new(0);
static ANE_TEMP_X10: AtomicU32 = AtomicU32::new(0);
static CPU_POWER_X10: AtomicU32 = AtomicU32::new(0);
static GPU_POWER_X10: AtomicU32 = AtomicU32::new(0);
static RAM_POWER_X10: AtomicU32 = AtomicU32::new(0);
static ANE_POWER_X10: AtomicU32 = AtomicU32::new(0);
static LAST_NOTIF_PCT: AtomicU8 = AtomicU8::new(0);

// ── App info ──────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
pub struct AppInfo {
    pub name: String,
    pub path: String,
    pub size_mb: u64,
}

// ── System metrics (from mo status --json) ────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct NetInterface {
    pub name: String,
    pub ip: String,
    pub rx_rate: f64,
    pub tx_rate: f64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ProcessInfo {
    pub name: String,
    pub command: String,
    pub cpu: f64,
    pub memory: f64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct BluetoothDevice {
    pub name: String,
    pub connected: bool,
    pub battery: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SystemMetrics {
    // General
    pub host: String,
    pub platform: String,
    pub uptime: String,
    pub procs: i64,
    // Hardware
    pub model: String,
    pub cpu_model: String,
    pub os_version: String,
    // Health
    pub health_score: i64,
    pub health_score_msg: String,
    // CPU
    pub cpu_usage: f64,
    pub cpu_per_core: Vec<f64>,
    pub cpu_load1: f64,
    pub cpu_load5: f64,
    pub cpu_load15: f64,
    pub cpu_core_count: i64,
    // Memory
    pub mem_used: u64,
    pub mem_total: u64,
    pub mem_available: u64,
    pub mem_used_percent: f64,
    pub mem_swap_used: u64,
    pub mem_swap_total: u64,
    // Disk (mount = "/")
    pub disk_used: u64,
    pub disk_total: u64,
    pub disk_used_percent: f64,
    pub disk_io_read: f64,
    pub disk_io_write: f64,
    pub trash_size: u64,
    // Network
    pub net_interfaces: Vec<NetInterface>,
    pub proxy_enabled: bool,
    pub proxy_type: String,
    pub proxy_host: String,
    // Battery (first battery)
    pub battery_percent: i64,
    pub battery_status: String,
    pub battery_time_left: String,
    pub battery_health: String,
    pub battery_cycles: i64,
    pub battery_capacity: i64,
    // Thermal
    pub thermal_cpu_temp: f64,
    pub thermal_battery_temp: f64,
    pub thermal_system_power: f64,
    pub thermal_adapter_power: f64,
    pub thermal_battery_power: f64,
    pub thermal_fan_speed: i64,
    // Top processes
    pub top_processes: Vec<ProcessInfo>,
    // Bluetooth
    pub bluetooth_devices: Vec<BluetoothDevice>,
}

// ── Installer files ───────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
pub struct InstallerFile {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub source: String,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub(crate) fn home_dir() -> std::path::PathBuf {
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
}

/// Crée un répertoire temporaire privé (mode 0700) via `tempfile::TempDir`.
/// Le dossier est nommé aléatoirement — aucun PID ni préfixe frontend dans le nom.
/// Le caller doit conserver le `TempDir` en vie pour éviter la suppression RAII.
fn burrow_tempdir() -> std::io::Result<tempfile::TempDir> {
    tempfile::Builder::new().prefix("burrow_").tempdir_in(
        std::env::var("TMPDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir()),
    )
}

/// Crée un fichier temporaire nommé aléatoirement dans `$TMPDIR`.
/// Renvoie un `NamedTempFile` dont le fichier est supprimé à la destruction.
/// Le `suffix` doit être une extension fixe (".mobileconfig", ".m", etc.) — pas de valeur frontend.
fn burrow_tempfile(suffix: &str) -> std::io::Result<tempfile::NamedTempFile> {
    tempfile::Builder::new()
        .prefix("burrow_")
        .suffix(suffix)
        .tempfile_in(
            std::env::var("TMPDIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::env::temp_dir()),
        )
}

/// Calcule la taille approximative d'un répertoire (profondeur limitée à 8 niveaux
/// pour éviter les boucles infinies sur les symlinks).
fn folder_size_approx(path: &std::path::Path) -> u64 {
    fn inner(path: &std::path::Path, depth: u8) -> u64 {
        if depth == 0 {
            return 0;
        }
        let Ok(entries) = fs::read_dir(path) else {
            return 0;
        };
        entries
            .flatten()
            .map(|e| {
                let p = e.path();
                // Ne pas suivre les symlinks (évite les boucles)
                if p.is_symlink() {
                    fs::symlink_metadata(&p).map(|m| m.len()).unwrap_or(0)
                } else if p.is_dir() {
                    inner(&p, depth - 1)
                } else {
                    fs::metadata(&p).map(|m| m.len()).unwrap_or(0)
                }
            })
            .sum()
    }
    inner(path, 8)
}

#[tauri::command]
fn get_home_dir() -> String {
    home_dir().to_string_lossy().to_string()
}

fn get_mo_path_internal(app: &tauri::AppHandle) -> Result<String, String> {
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    let mo_path = resource_dir.join("mole").join("bin").join("mo");
    if mo_path.exists() {
        Ok(mo_path.to_string_lossy().to_string())
    } else {
        which_mo()
    }
}

fn which_mo() -> Result<String, String> {
    let out = Command::new("which")
        .arg("mo")
        .output()
        .map_err(|e| e.to_string())?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() {
        Err("mo not found".to_string())
    } else {
        Ok(path)
    }
}

fn du_mb(path: &Path) -> u64 {
    Command::new("du")
        .args(["-sk", &path.to_string_lossy()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| {
            s.split_whitespace()
                .next()
                .map(|n| n.parse::<u64>().unwrap_or(0))
        })
        .map(|kb| kb / 1024)
        .unwrap_or(0)
}

// ── Metrics cache (stale-while-revalidate) ────────────────────────────────────

struct CachedMetrics {
    data: Option<SystemMetrics>,
    is_refreshing: bool,
}

static METRICS_CACHE: OnceLock<Mutex<CachedMetrics>> = OnceLock::new();

fn metrics_cache() -> &'static Mutex<CachedMetrics> {
    METRICS_CACHE.get_or_init(|| {
        Mutex::new(CachedMetrics {
            data: None,
            is_refreshing: false,
        })
    })
}

// ── Native metrics (no Mole dependency) ──────────────────────────────────────

fn sysctl_string(key: &str) -> String {
    Command::new("sysctl")
        .args(["-n", key])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn format_uptime(secs: u64) -> String {
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    match (d, h, m) {
        (0, 0, m) => format!("{} min", m),
        (0, h, m) => format!("{}h {}min", h, m),
        (d, h, _) => format!("{}j {}h", d, h),
    }
}

fn compute_health_score(cpu: f64, mem_pct: f64, disk_pct: f64, battery_pct: i64) -> (i64, String) {
    let mut score = 100i64;
    let mut issues: Vec<&str> = Vec::new();
    if cpu > 85.0 {
        score -= 25;
        issues.push("High CPU");
    } else if cpu > 70.0 {
        score -= 10;
    }
    if mem_pct > 90.0 {
        score -= 25;
        issues.push("High Memory");
    } else if mem_pct > 75.0 {
        score -= 10;
    }
    if disk_pct > 90.0 {
        score -= 20;
        issues.push("High Disk Usage");
    } else if disk_pct > 80.0 {
        score -= 8;
    }
    if battery_pct > 0 && battery_pct < 15 {
        score -= 10;
        issues.push("Low Battery");
    }
    let score = score.max(0);
    let msg = match issues.len() {
        0 => if score >= 90 { "Excellent" } else { "Good" }.to_string(),
        1 => issues[0].to_string(),
        _ => "Multiple Issues Detected".to_string(),
    };
    (score, msg)
}

fn parse_battery_native() -> (i64, String, String, String, i64, i64) {
    // percent, status, time_left, health, cycles, capacity
    let out = Command::new("ioreg")
        .args(["-n", "AppleSmartBattery", "-r", "-a"])
        .output()
        .unwrap_or_else(|_| std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: vec![],
            stderr: vec![],
        });
    let raw = String::from_utf8_lossy(&out.stdout);

    fn ioreg_int(s: &str, key: &str) -> i64 {
        let needle = format!("<key>{}</key>", key);
        s.find(&needle)
            .and_then(|i| {
                let rest = &s[i + needle.len()..];
                let val_start = rest.find("<integer>")? + 9;
                let val_end = rest[val_start..].find("</integer>")?;
                rest[val_start..val_start + val_end]
                    .trim()
                    .parse::<i64>()
                    .ok()
            })
            .unwrap_or(0)
    }

    let current = ioreg_int(&raw, "CurrentCapacity");
    let max_cap = ioreg_int(&raw, "MaxCapacity");
    let design = ioreg_int(&raw, "DesignCapacity");
    let cycles = ioreg_int(&raw, "CycleCount");
    let is_charging = raw.contains("<key>IsCharging</key>\n\t<true/>");
    let is_full = raw.contains("<key>FullyCharged</key>\n\t<true/>");

    let pct = if max_cap > 0 {
        current * 100 / max_cap
    } else {
        0
    };
    let status = if is_full {
        "Chargée"
    } else if is_charging {
        "En charge"
    } else {
        "Déchargement"
    }
    .to_string();
    let capacity = if design > 0 {
        max_cap * 100 / design
    } else {
        0
    };
    (pct, status, String::new(), String::new(), cycles, capacity)
}

fn parse_proxy_native() -> (bool, String, String) {
    let out = Command::new("scutil").arg("--proxy").output().ok();
    let raw = out
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    let enabled = raw.contains("HTTPEnable : 1") || raw.contains("SOCKSEnable : 1");
    let proxy_type = if raw.contains("SOCKSEnable : 1") {
        "SOCKS"
    } else if raw.contains("HTTPSEnable : 1") {
        "HTTPS"
    } else if raw.contains("HTTPEnable : 1") {
        "HTTP"
    } else {
        ""
    }
    .to_string();
    let host = raw
        .lines()
        .find(|l| l.contains("HTTPProxy :") || l.contains("SOCKSProxy :"))
        .and_then(|l| l.split(':').nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    (enabled, proxy_type, host)
}

fn parse_net_interfaces_native() -> Vec<NetInterface> {
    let out = Command::new("ifconfig").output().ok();
    let raw = out
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    let mut result = Vec::new();
    let mut cur_name = String::new();
    let mut cur_ip = String::new();
    for line in raw.lines() {
        if !line.starts_with('\t') && !line.starts_with(' ') {
            if !cur_name.is_empty() && !cur_name.starts_with("lo") && !cur_ip.is_empty() {
                result.push(NetInterface {
                    name: cur_name.clone(),
                    ip: cur_ip.clone(),
                    rx_rate: 0.0,
                    tx_rate: 0.0,
                });
            }
            cur_name = line.split(':').next().unwrap_or("").trim().to_string();
            cur_ip = String::new();
        } else if line.trim_start().starts_with("inet ") && !line.contains("inet6") {
            cur_ip = line.split_whitespace().nth(1).unwrap_or("").to_string();
        }
    }
    if !cur_name.is_empty() && !cur_name.starts_with("lo") && !cur_ip.is_empty() {
        result.push(NetInterface {
            name: cur_name,
            ip: cur_ip,
            rx_rate: 0.0,
            tx_rate: 0.0,
        });
    }
    result
}

fn do_fetch_metrics(_app: &tauri::AppHandle) -> Result<SystemMetrics, String> {
    use sysinfo::{Disks, System};

    let mut sys = System::new_all();
    sys.refresh_all();

    let host = System::host_name().unwrap_or_default();
    let os_ver_raw = System::os_version().unwrap_or_default();
    let os_version = format!("macOS {}", os_ver_raw);
    let platform = System::long_os_version().unwrap_or(os_version.clone());
    let uptime_secs = System::uptime();
    let uptime = format_uptime(uptime_secs);
    let procs = sys.processes().len() as i64;
    let model = sysctl_string("hw.model");
    let cpu_model = sys
        .cpus()
        .first()
        .map(|c| c.brand().to_string())
        .unwrap_or_default();
    let cpu_usage = sys.global_cpu_usage() as f64;
    let cpu_per_core: Vec<f64> = sys.cpus().iter().map(|c| c.cpu_usage() as f64).collect();
    let cpu_core_count = sys.cpus().len() as i64;
    let load = System::load_average();

    let mem_total = sys.total_memory();
    let mem_used = sys.used_memory();
    let mem_available = sys.available_memory();
    let mem_swap_used = sys.used_swap();
    let mem_swap_total = sys.total_swap();
    let mem_used_percent = if mem_total > 0 {
        mem_used as f64 / mem_total as f64 * 100.0
    } else {
        0.0
    };

    let disks = Disks::new_with_refreshed_list();
    let root_disk = disks.iter().find(|d| d.mount_point() == Path::new("/"));
    let disk_total = root_disk.map(|d| d.total_space()).unwrap_or(0);
    let disk_avail = root_disk.map(|d| d.available_space()).unwrap_or(0);
    let disk_used = disk_total.saturating_sub(disk_avail);
    let disk_used_percent = if disk_total > 0 {
        disk_used as f64 / disk_total as f64 * 100.0
    } else {
        0.0
    };

    let trash_size = du_bytes(&home_dir().join(".Trash"));

    // Top processes (by CPU)
    let mut procs_vec: Vec<_> = sys.processes().values().collect();
    procs_vec.sort_by(|a, b| {
        b.cpu_usage()
            .partial_cmp(&a.cpu_usage())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let top_processes: Vec<ProcessInfo> = procs_vec
        .iter()
        .take(10)
        .map(|p| ProcessInfo {
            name: p.name().to_string_lossy().to_string(),
            command: p
                .exe()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_default(),
            cpu: p.cpu_usage() as f64,
            memory: p.memory() as f64,
        })
        .collect();

    let (
        battery_percent,
        battery_status,
        battery_time_left,
        battery_health,
        battery_cycles,
        battery_capacity,
    ) = parse_battery_native();
    let (proxy_enabled, proxy_type, proxy_host) = parse_proxy_native();
    let net_interfaces = parse_net_interfaces_native();

    let lx = |a: &AtomicU32| a.load(Ordering::Relaxed) as f64 / 10.0;
    let (health_score, health_score_msg) = compute_health_score(
        cpu_usage,
        mem_used_percent,
        disk_used_percent,
        battery_percent,
    );

    Ok(SystemMetrics {
        host,
        platform,
        uptime,
        procs,
        model,
        cpu_model,
        os_version,
        health_score,
        health_score_msg,
        cpu_usage,
        cpu_per_core,
        cpu_load1: load.one,
        cpu_load5: load.five,
        cpu_load15: load.fifteen,
        cpu_core_count,
        mem_used,
        mem_total,
        mem_available,
        mem_used_percent,
        mem_swap_used,
        mem_swap_total,
        disk_used,
        disk_total,
        disk_used_percent,
        disk_io_read: 0.0,
        disk_io_write: 0.0,
        trash_size,
        net_interfaces,
        proxy_enabled,
        proxy_type,
        proxy_host,
        battery_percent,
        battery_status,
        battery_time_left,
        battery_health,
        battery_cycles,
        battery_capacity,
        thermal_cpu_temp: lx(&CPU_TEMP_X10),
        thermal_battery_temp: 0.0,
        thermal_system_power: lx(&CPU_POWER_X10)
            + lx(&GPU_POWER_X10)
            + lx(&RAM_POWER_X10)
            + lx(&ANE_POWER_X10),
        thermal_adapter_power: 0.0,
        thermal_battery_power: 0.0,
        thermal_fan_speed: FAN_RPM.load(Ordering::Relaxed) as i64,
        top_processes,
        bluetooth_devices: Vec::new(),
    })
}

// Legacy parse helpers kept for compatibility (unused now)
fn _do_fetch_metrics_mole(app: &tauri::AppHandle) -> Result<SystemMetrics, String> {
    let mo = get_mo_path_internal(app)?;

    let output = Command::new(&mo)
        .args(["status", "--json"])
        .output()
        .map_err(|e| e.to_string())?;

    let json_str = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    let v: serde_json::Value =
        serde_json::from_str(&json_str).map_err(|e| format!("JSON parse error: {}", e))?;

    let s = |val: &serde_json::Value| val.as_str().unwrap_or("").to_string();
    let f = |val: &serde_json::Value| val.as_f64().unwrap_or(0.0);
    let i = |val: &serde_json::Value| val.as_i64().unwrap_or(0);
    let u = |val: &serde_json::Value| val.as_u64().unwrap_or(0);

    let cpu_per_core: Vec<f64> = v["cpu"]["per_core"]
        .as_array()
        .map(|arr| arr.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect())
        .unwrap_or_default();

    let root_disk = v["disks"]
        .as_array()
        .and_then(|arr| arr.iter().find(|d| d["mount"].as_str() == Some("/")))
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let net_interfaces: Vec<NetInterface> = v["network"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|iface| NetInterface {
                    name: s(&iface["name"]),
                    ip: s(&iface["ip"]),
                    rx_rate: f(&iface["rx_rate_mbs"]),
                    tx_rate: f(&iface["tx_rate_mbs"]),
                })
                .collect()
        })
        .unwrap_or_default();

    let bat = v["batteries"]
        .as_array()
        .and_then(|arr| arr.first())
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let top_processes: Vec<ProcessInfo> = v["top_processes"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|p| ProcessInfo {
                    name: s(&p["name"]),
                    command: s(&p["command"]),
                    cpu: f(&p["cpu"]),
                    memory: f(&p["memory"]),
                })
                .collect()
        })
        .unwrap_or_default();

    let bluetooth_devices: Vec<BluetoothDevice> = v["bluetooth"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|d| BluetoothDevice {
                    name: s(&d["name"]),
                    connected: d["connected"].as_bool().unwrap_or(false),
                    battery: s(&d["battery"]),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(SystemMetrics {
        host: s(&v["host"]),
        platform: s(&v["platform"]),
        uptime: s(&v["uptime"]),
        procs: i(&v["procs"]),
        model: s(&v["hardware"]["model"]),
        cpu_model: s(&v["hardware"]["cpu_model"]),
        os_version: s(&v["hardware"]["os_version"]),
        health_score: i(&v["health_score"]),
        health_score_msg: s(&v["health_score_msg"]),
        cpu_usage: f(&v["cpu"]["usage"]),
        cpu_per_core,
        cpu_load1: f(&v["cpu"]["load1"]),
        cpu_load5: f(&v["cpu"]["load5"]),
        cpu_load15: f(&v["cpu"]["load15"]),
        cpu_core_count: i(&v["cpu"]["core_count"]),
        mem_used: u(&v["memory"]["used"]),
        mem_total: u(&v["memory"]["total"]),
        mem_available: u(&v["memory"]["available"]),
        mem_used_percent: f(&v["memory"]["used_percent"]),
        mem_swap_used: u(&v["memory"]["swap_used"]),
        mem_swap_total: u(&v["memory"]["swap_total"]),
        disk_used: u(&root_disk["used"]),
        disk_total: u(&root_disk["total"]),
        disk_used_percent: f(&root_disk["used_percent"]),
        disk_io_read: f(&v["disk_io"]["read_rate"]),
        disk_io_write: f(&v["disk_io"]["write_rate"]),
        trash_size: u(&v["trash_size"]),
        net_interfaces,
        proxy_enabled: v["proxy"]["enabled"].as_bool().unwrap_or(false),
        proxy_type: s(&v["proxy"]["type"]),
        proxy_host: s(&v["proxy"]["host"]),
        battery_percent: i(&bat["percent"]),
        battery_status: s(&bat["status"]),
        battery_time_left: s(&bat["time_left"]),
        battery_health: s(&bat["health"]),
        battery_cycles: i(&bat["cycle_count"]),
        battery_capacity: i(&bat["capacity"]),
        thermal_cpu_temp: f(&v["thermal"]["cpu_temp"]),
        thermal_battery_temp: f(&v["thermal"]["battery_temp"]),
        thermal_system_power: f(&v["thermal"]["system_power"]),
        thermal_adapter_power: f(&v["thermal"]["adapter_power"]),
        thermal_battery_power: f(&v["thermal"]["battery_power"]),
        thermal_fan_speed: i(&v["thermal"]["fan_speed"]),
        top_processes,
        bluetooth_devices,
    })
}

// ── Commands ──────────────────────────────────────────────────────────────────

#[tauri::command]
fn get_mo_path(app: tauri::AppHandle) -> Result<String, String> {
    get_mo_path_internal(&app)
}

#[tauri::command]
fn get_system_metrics(app: tauri::AppHandle) -> Result<SystemMetrics, String> {
    let cache = metrics_cache();

    // Snapshot cached data without holding the lock
    let cached = cache.lock().unwrap().data.clone();

    // Spawn a background refresh if one isn't already running
    {
        let mut guard = cache.lock().unwrap();
        if !guard.is_refreshing {
            guard.is_refreshing = true;
            let app_clone = app.clone();
            std::thread::spawn(move || {
                let result = do_fetch_metrics(&app_clone);
                let mut g = metrics_cache().lock().unwrap();
                if let Ok(m) = result {
                    g.data = Some(m);
                }
                g.is_refreshing = false;
            });
        }
    }

    // Return cached data instantly, or signal that we're still loading
    cached.ok_or_else(|| "loading".to_string())
}

/// Désinstalle une application .app de façon typée.
/// Accepte uniquement le nom d'affichage et le chemin validé — aucun argument libre.
#[tauri::command]
fn uninstall_app(app: tauri::AppHandle, name: String, app_path: String) -> Result<(), String> {
    if app_path.is_empty() {
        return Err("Chemin d'application manquant".to_string());
    }
    // Validation stricte avant de lancer le thread
    guard::validate_app_uninstall_path(&app_path)?;

    // Canonicaliser et revérifier après résolution des symlinks
    let canonical = std::fs::canonicalize(&app_path)
        .map_err(|e| format!("Impossible de résoudre le chemin : {}", e))?;
    guard::validate_app_uninstall_path(&canonical.to_string_lossy())?;

    std::thread::spawn(move || {
        let _ = app.emit("mo-output", format!("→ Suppression de {}…", name));

        // Tentative 1 : suppression directe (apps dans ~/Applications)
        match fs::remove_dir_all(&canonical) {
            Ok(_) => {
                let _ = app.emit("mo-output", format!("✓ {} désinstallé avec succès", name));
                let _ = app.emit("mo-done", 0i32);
                return;
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::PermissionDenied
                    || e.raw_os_error() == Some(13) =>
            {
                let _ = app.emit("mo-output", "→ Autorisations requises, demande en cours…");
            }
            Err(e) => {
                let _ = app.emit("mo-output", format!("✗ Erreur : {}", e));
                let _ = app.emit("mo-done", 1i32);
                return;
            }
        }

        // Tentative 2 : privilèges admin — quoting POSIX correct
        let q = guard::posix_shell_quote(&canonical.to_string_lossy());
        match run_admin_sh(&format!("rm -rf {}", q)) {
            Ok(()) => {
                let _ = app.emit("mo-output", format!("✓ {} désinstallé avec succès", name));
                let _ = app.emit("mo-done", 0i32);
            }
            Err(e) => {
                let _ = app.emit("mo-output", format!("✗ Échec : {}", e));
                let _ = app.emit("mo-done", 1i32);
            }
        }
    });
    Ok(())
}

#[tauri::command]
fn list_apps() -> Vec<AppInfo> {
    let home = home_dir();
    let mut apps = Vec::new();

    let dirs = vec![
        std::path::PathBuf::from("/Applications"),
        home.join("Applications"),
    ];

    for dir in dirs {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("app") {
                continue;
            }
            // Filter out pre-installed Apple apps by bundle ID
            let plist = path.join("Contents/Info.plist");
            let bundle_id = plist_str(&plist, "CFBundleIdentifier").unwrap_or_default();
            if bundle_id.starts_with("com.apple.") {
                continue;
            }
            let name = path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                continue;
            }
            apps.push(AppInfo {
                name,
                path: path.to_string_lossy().to_string(),
                size_mb: 0,
            });
        }
    }

    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

#[tauri::command]
fn get_app_size(app_path: String) -> u64 {
    du_mb(Path::new(&app_path))
}

// ── Native ICNS → PNG extraction (no sips subprocess) ────────────────────────

fn extract_icon_png(icns_path: &Path) -> Option<Vec<u8>> {
    let data = fs::read(icns_path).ok()?;
    if data.len() < 8 || &data[0..4] != b"icns" {
        return None;
    }

    const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
    // Prefer compact icons for fast base64 transfer
    const PREFERRED: &[[u8; 4]] = &[*b"icp4", *b"ic07", *b"icp5", *b"ic11", *b"ic12"];

    let mut first_png: Option<Vec<u8>> = None;
    let mut pos = 8usize;

    while pos + 8 <= data.len() {
        let tag: [u8; 4] = data[pos..pos + 4].try_into().ok()?;
        let sz = u32::from_be_bytes(data[pos + 4..pos + 8].try_into().ok()?) as usize;
        if sz < 8 || pos + sz > data.len() {
            break;
        }
        let payload = &data[pos + 8..pos + sz];
        if payload.starts_with(PNG_MAGIC) {
            if PREFERRED.contains(&tag) {
                return Some(payload.to_vec());
            }
            if first_png.is_none() {
                first_png = Some(payload.to_vec());
            }
        }
        pos += sz;
    }
    first_png
}

fn find_icns_for_app(app_path: &Path) -> Option<std::path::PathBuf> {
    let name = app_path.file_stem()?.to_str()?.to_string();
    let resources = app_path.join("Contents/Resources");

    // Try Info.plist first (accurate), then common fallbacks
    let plist = app_path.join("Contents/Info.plist");
    if plist.exists() {
        if let Some(icon_name) = Command::new("/usr/libexec/PlistBuddy")
            .args(["-c", "Print :CFBundleIconFile", &plist.to_string_lossy()])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            let icns = if icon_name.ends_with(".icns") {
                resources.join(&icon_name)
            } else {
                resources.join(format!("{}.icns", icon_name))
            };
            if icns.exists() {
                return Some(icns);
            }
        }
    }

    // Fallbacks by name then first .icns found
    for candidate in &[format!("{}.icns", name), "AppIcon.icns".to_string()] {
        let p = resources.join(candidate);
        if p.exists() {
            return Some(p);
        }
    }
    fs::read_dir(&resources)
        .ok()?
        .flatten()
        .find(|e| e.path().extension().and_then(|x| x.to_str()) == Some("icns"))
        .map(|e| e.path())
}

fn b64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

#[tauri::command]
fn get_app_icon(app_path: String) -> Option<String> {
    let icns = find_icns_for_app(Path::new(&app_path))?;
    let png = extract_icon_png(&icns)?;
    Some(format!("data:image/png;base64,{}", b64_encode(&png)))
}

// ── Batch icon loading with permanent cache ───────────────────────────────────

static APP_ICONS_CACHE: OnceLock<HashMap<String, String>> = OnceLock::new();

fn load_single_app_icon(app_path: std::path::PathBuf) -> Option<(String, String)> {
    let name = app_path.file_stem()?.to_str()?.to_string();
    let icns = find_icns_for_app(&app_path)?;
    let png = extract_icon_png(&icns)?;
    Some((name, format!("data:image/png;base64,{}", b64_encode(&png))))
}

#[tauri::command]
fn get_all_app_icons() -> HashMap<String, String> {
    APP_ICONS_CACHE
        .get_or_init(|| {
            let apps = collect_all_apps();

            let handles: Vec<_> = apps
                .into_iter()
                .map(|path| std::thread::spawn(move || load_single_app_icon(path)))
                .collect();

            handles
                .into_iter()
                .filter_map(|h| h.join().ok().flatten())
                .collect()
        })
        .clone()
}

#[tauri::command]
fn list_installer_files() -> Vec<InstallerFile> {
    let home = home_dir();
    let dirs_owned: Vec<(std::path::PathBuf, &str)> = vec![
        (home.join("Downloads"), "Downloads"),
        (home.join("Desktop"), "Desktop"),
        (home.join("Documents"), "Documents"),
        (
            home.join("Library/Caches/Homebrew/downloads"),
            "Homebrew Cache",
        ),
    ];

    let exts = ["dmg", "pkg", "iso", "xip"];
    let mut files: Vec<InstallerFile> = Vec::new();

    for (dir, source) in &dirs_owned {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if exts.contains(&ext.as_str()) {
                    let size_bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();
                    files.push(InstallerFile {
                        name,
                        path: path.to_string_lossy().to_string(),
                        size_bytes,
                        source: source.to_string(),
                    });
                }
            }
        }
    }

    files.sort_by_key(|k| std::cmp::Reverse(k.size_bytes));
    files
}

#[tauri::command]
fn check_full_disk_access() -> bool {
    let home = home_dir();
    // Must use File::open (not metadata/stat) — macOS allows stat() without FDA
    // but blocks open() via TCC for protected files.

    // Safari History — most reliable FDA indicator
    let safari = home.join("Library/Safari/History.db");
    match std::fs::File::open(&safari) {
        Ok(_) => return true,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return false,
        Err(_) => {} // file absent, try next
    }

    // Messages DB — always present on macOS
    let messages = home.join("Library/Messages/chat.db");
    match std::fs::File::open(&messages) {
        Ok(_) => return true,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return false,
        Err(_) => {} // absent, try next
    }

    // TCC database — should always exist
    match std::fs::File::open("/Library/Application Support/com.apple.TCC/TCC.db") {
        Ok(_) => true,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => false,
        Err(_) => true, // can't determine → assume OK
    }
}

#[tauri::command]
fn open_full_disk_access_settings() {
    Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
        .spawn()
        .ok();
}

#[tauri::command]
fn move_to_trash(path: String) -> Result<(), String> {
    guard::validate_trash_path(&path)?;
    // Use osascript to move file to Trash safely (preserves filename collisions)
    let script = format!(
        r#"tell application "Finder" to delete POSIX file "{}""#,
        path.replace('"', "\\\"")
    );
    let out = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).to_string();
        Err(if err.is_empty() {
            "Failed to move to trash".to_string()
        } else {
            err
        })
    }
}

// ── 1. Clean category sizes ───────────────────────────────────────────────────

#[derive(Serialize)]
pub struct CleanCategorySize {
    pub id: String,
    pub size_mb: u64,
}

#[tauri::command]
fn get_clean_sizes() -> Vec<CleanCategorySize> {
    let home = home_dir();

    let pairs: Vec<(&'static str, std::path::PathBuf)> = vec![
        ("user_cache", home.join("Library/Caches")),
        ("system_logs", home.join("Library/Logs")),
        ("crash_reports", home.join("Library/Logs/DiagnosticReports")),
        ("npm_cache", home.join(".npm")),
        ("yarn_cache", home.join(".yarn/cache")),
        ("trash", home.join(".Trash")),
        ("xcode", home.join("Library/Developer/Xcode/DerivedData")),
        (
            "ios_backups",
            home.join("Library/Application Support/MobileSync/Backup"),
        ),
        ("brew_cache", home.join("Library/Caches/Homebrew")),
        (
            "simulator",
            home.join("Library/Developer/CoreSimulator/Caches"),
        ),
    ];

    let browser_paths: Vec<std::path::PathBuf> = vec![
        home.join("Library/Caches/com.apple.Safari"),
        home.join("Library/Application Support/Google/Chrome/Default/Cache"),
        home.join("Library/Caches/Firefox"),
        home.join("Library/Application Support/BraveSoftware/Brave-Browser/Default/Cache"),
    ];

    // Run all du calls in parallel
    let mut handles: Vec<std::thread::JoinHandle<CleanCategorySize>> = pairs
        .into_iter()
        .map(|(id, path)| {
            std::thread::spawn(move || CleanCategorySize {
                id: id.to_string(),
                size_mb: du_mb(&path),
            })
        })
        .collect();

    // Browser cache in parallel too
    let browser_handles: Vec<_> = browser_paths
        .into_iter()
        .map(|p| std::thread::spawn(move || du_mb(&p)))
        .collect();

    let mut result: Vec<CleanCategorySize> =
        handles.drain(..).filter_map(|h| h.join().ok()).collect();

    let browser_mb: u64 = browser_handles
        .into_iter()
        .filter_map(|h| h.join().ok())
        .sum();

    result.push(CleanCategorySize {
        id: "browser_cache".to_string(),
        size_mb: browser_mb,
    });
    result
}

// ── 2. Run clean selection ────────────────────────────────────────────────────

/// Supprime tous les enfants directs d'un répertoire.
/// Si le répertoire n'existe pas, c'est un succès (rien à faire).
fn delete_dir_children(dir: &std::path::Path) -> std::io::Result<()> {
    match fs::read_dir(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(entries) => {
            let mut first_err: Option<std::io::Error> = None;
            for entry in entries.flatten() {
                let p = entry.path();
                let r = if p.is_dir() {
                    fs::remove_dir_all(&p)
                } else {
                    fs::remove_file(&p)
                };
                if let Err(e) = r {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                }
            }
            match first_err {
                None => Ok(()),
                Some(e) => Err(e),
            }
        }
    }
}

/// Supprime un chemin (fichier ou répertoire). Retourne Ok si n'existe pas.
fn remove_if_exists(p: &std::path::Path) -> std::io::Result<()> {
    if !p.exists() {
        return Ok(());
    }
    if p.is_dir() {
        fs::remove_dir_all(p)
    } else {
        fs::remove_file(p)
    }
}

#[tauri::command]
fn run_clean_selection(
    app: tauri::AppHandle,
    categories: Vec<String>,
    installer_paths: Vec<String>,
) {
    std::thread::spawn(move || {
        let home = home_dir();
        let mut any_error = false;

        for cat_id in &categories {
            let (label, result): (&str, std::io::Result<()>) = match cat_id.as_str() {
                "user_cache" => (
                    "Cache utilisateur",
                    delete_dir_children(&home.join("Library/Caches")),
                ),
                "system_logs" => (
                    "Logs système",
                    delete_dir_children(&home.join("Library/Logs")),
                ),
                "crash_reports" => (
                    "Rapports de crash",
                    delete_dir_children(&home.join("Library/Logs/DiagnosticReports")),
                ),
                "npm_cache" => {
                    let r1 = remove_if_exists(&home.join(".npm/cache"));
                    let r2 = remove_if_exists(&home.join(".npm/_logs"));
                    ("Cache npm", r1.and(r2))
                }
                "yarn_cache" => ("Cache yarn", remove_if_exists(&home.join(".yarn/cache"))),
                "browser_cache" => {
                    let paths = [
                        home.join("Library/Caches/com.apple.Safari"),
                        home.join("Library/Application Support/Google/Chrome/Default/Cache"),
                        home.join("Library/Caches/Firefox"),
                        home.join(
                            "Library/Application Support/BraveSoftware/Brave-Browser/Default/Cache",
                        ),
                    ];
                    let r = paths.iter().try_fold((), |(), p| remove_if_exists(p));
                    ("Caches navigateurs", r)
                }
                "trash" => {
                    let out = Command::new("osascript")
                        .args(["-e", "tell application \"Finder\" to empty trash"])
                        .output();
                    let r = match out {
                        Ok(o) if o.status.success() => Ok(()),
                        Ok(o) => Err(std::io::Error::other(
                            String::from_utf8_lossy(&o.stderr).trim().to_string(),
                        )),
                        Err(e) => Err(e),
                    };
                    ("Corbeille", r)
                }
                "xcode" => (
                    "Xcode DerivedData",
                    delete_dir_children(&home.join("Library/Developer/Xcode/DerivedData")),
                ),
                "ios_backups" => (
                    "Sauvegardes iOS",
                    delete_dir_children(
                        &home.join("Library/Application Support/MobileSync/Backup"),
                    ),
                ),
                "brew_cache" => (
                    "Cache Homebrew",
                    delete_dir_children(&home.join("Library/Caches/Homebrew")),
                ),
                "simulator" => (
                    "Cache Simulateur iOS",
                    delete_dir_children(&home.join("Library/Developer/CoreSimulator/Caches")),
                ),
                _ => continue,
            };

            let _ = app.emit("mo-output", format!("→ {}", label));
            match result {
                Ok(_) => {
                    let _ = app.emit("mo-output", format!("  ✓ {}", label));
                }
                Err(e) => {
                    any_error = true;
                    let _ = app.emit("mo-output", format!("  ✗ {} : {}", label, e));
                }
            }
        }

        for path in &installer_paths {
            let p = std::path::Path::new(path);
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or(path);
            let _ = app.emit("mo-output", format!("→ {}", name));
            if let Err(e) = guard::validate_installer_path(path) {
                any_error = true;
                let _ = app.emit("mo-output", format!("  ✗ {} : chemin refusé ({})", name, e));
                continue;
            }
            let result = if p.is_dir() {
                fs::remove_dir_all(p)
            } else {
                fs::remove_file(p)
            };
            match result {
                Ok(_) => {
                    let _ = app.emit("mo-output", format!("  ✓ {}", name));
                }
                Err(e) => {
                    any_error = true;
                    let _ = app.emit("mo-output", format!("  ✗ {} : {}", name, e));
                }
            }
        }

        let _ = app.emit("mo-done", if any_error { 1i32 } else { 0i32 });
    });
}

// ── 3. Run optimize selection ─────────────────────────────────────────────────

#[tauri::command]
fn run_optimize_selection(app: tauri::AppHandle, tasks: Vec<String>) {
    std::thread::spawn(move || {
        let task_info: &[(&str, &str)] = &[
            ("dns", "Cache DNS"),
            ("spotlight", "Spotlight"),
            ("finder", "Finder"),
            ("dock", "Dock"),
            ("swap", "Mémoire swap"),
            ("launchpad", "Launchpad"),
            ("tmutil_thin", "Time Machine (thin)"),
            ("periodic", "Scripts périodiques"),
            ("diskutil_verify", "Vérification disque"),
            ("launch_services", "Base de données apps"),
            ("docker_prune", "Docker (nettoyage)"),
            ("mail_speed", "Mail (accélération)"),
        ];

        let mut any_error = false;

        for task_id in &tasks {
            if let Some(&(_, label)) = task_info.iter().find(|(id, _)| id == task_id) {
                let _ = app.emit("mo-output", format!("→ {}", label));

                let success = match task_id.as_str() {
                    "dns" => {
                        run_admin_sh("dscacheutil -flushcache; killall -HUP mDNSResponder").is_ok()
                    }
                    "spotlight" => run_admin_sh("mdutil -E /").is_ok(),
                    "finder" => Command::new("killall")
                        .arg("Finder")
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false),
                    "dock" => Command::new("killall")
                        .arg("Dock")
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false),
                    "swap" => run_admin_sh("purge").is_ok(),
                    "launchpad" => {
                        let r1 = Command::new("defaults")
                            .args(["write", "com.apple.dock", "ResetLaunchPad", "-bool", "true"])
                            .output()
                            .map(|o| o.status.success())
                            .unwrap_or(false);
                        let r2 = Command::new("killall")
                            .arg("Dock")
                            .output()
                            .map(|o| o.status.success())
                            .unwrap_or(false);
                        r1 && r2
                    }
                    "tmutil_thin" => {
                        run_admin_sh("tmutil thinlocalsnapshots / 999999999999 4").is_ok()
                    }
                    "periodic" => run_admin_sh("/usr/sbin/periodic daily weekly monthly").is_ok(),
                    "diskutil_verify" => Command::new("/usr/sbin/diskutil")
                        .args(["verifyVolume", "/"])
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false),
                    "launch_services" => {
                        let lsreg = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
                        Command::new(lsreg)
                            .args([
                                "-kill", "-r", "-domain", "local", "-domain", "system", "-domain",
                                "user",
                            ])
                            .output()
                            .map(|o| o.status.success())
                            .unwrap_or(false)
                    }
                    "docker_prune" => Command::new("docker")
                        .args(["system", "prune", "-f"])
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false),
                    "mail_speed" => {
                        let home = home_dir();
                        let envelope = home.join("Library/Mail/V10/MailData/Envelope Index");
                        let envelope_alt = home.join("Library/Mail/V9/MailData/Envelope Index");
                        let target = if envelope.exists() {
                            Some(envelope)
                        } else if envelope_alt.exists() {
                            Some(envelope_alt)
                        } else {
                            None
                        };
                        target.map(|p| fs::remove_file(p).is_ok()).unwrap_or(false)
                    }
                    _ => false,
                };

                if success {
                    let _ = app.emit("mo-output", format!("  ✓ {}", label));
                } else {
                    any_error = true;
                    let _ = app.emit("mo-output", format!("  ✗ {}", label));
                }
            }
        }

        let _ = app.emit("mo-done", if any_error { 1i32 } else { 0i32 });
    });
}

// ── 4. Network rates ──────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct NetRateItem {
    pub name: String,
    pub rx_bps: f64,
    pub tx_bps: f64,
}

fn parse_netstat_bytes() -> HashMap<String, (u64, u64)> {
    let mut map: HashMap<String, (u64, u64)> = HashMap::new();
    let output = match Command::new("netstat").args(["-ib"]).output() {
        Ok(o) => o,
        Err(_) => return map,
    };
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        // Link layer lines have column 2 starting with '<'
        if cols.len() < 10 {
            continue;
        }
        if !cols.get(2).map(|c| c.starts_with('<')).unwrap_or(false) {
            continue;
        }
        let name = cols[0].to_string();
        let rx: u64 = cols.get(6).and_then(|v| v.parse().ok()).unwrap_or(0);
        let tx: u64 = cols.get(9).and_then(|v| v.parse().ok()).unwrap_or(0);
        // Accumulate in case of multiple link entries per interface
        let entry = map.entry(name).or_insert((0, 0));
        entry.0 = entry.0.saturating_add(rx);
        entry.1 = entry.1.saturating_add(tx);
    }
    map
}

type NetSnapshot = (Instant, HashMap<String, (u64, u64)>);
static NET_PREV: OnceLock<Mutex<NetSnapshot>> = OnceLock::new();

#[tauri::command]
fn get_net_rates() -> Vec<NetRateItem> {
    let lock = NET_PREV.get_or_init(|| Mutex::new((Instant::now(), parse_netstat_bytes())));

    let now_bytes = parse_netstat_bytes();
    let now = Instant::now();

    let mut guard = lock.lock().unwrap();
    let elapsed = now.duration_since(guard.0).as_secs_f64().max(0.1);
    let prev_bytes = guard.1.clone();

    let mut rates: Vec<NetRateItem> = now_bytes
        .iter()
        .filter(|(name, _)| !name.starts_with("lo"))
        .map(|(name, &(rx_now, tx_now))| {
            let (rx_prev, tx_prev) = prev_bytes.get(name).copied().unwrap_or((rx_now, tx_now));
            let rx_bps = rx_now.saturating_sub(rx_prev) as f64 / elapsed;
            let tx_bps = tx_now.saturating_sub(tx_prev) as f64 / elapsed;
            NetRateItem {
                name: name.clone(),
                rx_bps,
                tx_bps,
            }
        })
        .collect();

    *guard = (now, now_bytes);
    drop(guard);

    rates.sort_by(|a, b| {
        b.rx_bps
            .partial_cmp(&a.rx_bps)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rates
}

// ── 5. Free memory ────────────────────────────────────────────────────────────

#[tauri::command]
fn free_memory(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let _ = app.emit("mo-output", "Libération mémoire inactive…");
        match run_admin_sh("purge") {
            Ok(()) => {
                let _ = app.emit("mo-output", "  ✓ Mémoire inactive libérée");
            }
            Err(e) => {
                let _ = app.emit("mo-output", format!("  ✗ Erreur : {}", e));
            }
        }
        let _ = app.emit("mo-done", 0i32);
    });
}

// ── 6. Low power mode ─────────────────────────────────────────────────────────

#[tauri::command]
fn get_low_power_mode() -> bool {
    Command::new("/usr/bin/pmset")
        .arg("-g")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| {
            s.lines().any(|l| {
                let mut parts = l.split_whitespace();
                parts.next() == Some("lowpowermode") && parts.next() == Some("1")
            })
        })
        .unwrap_or(false)
}

#[tauri::command]
fn set_low_power_mode(enable: bool) -> Result<(), String> {
    let val = if enable { "1" } else { "0" };
    pmset_run(&["-a", "lowpowermode", val])
}

// ── 7. Mole version and update ────────────────────────────────────────────────

#[tauri::command]
fn get_mo_version(app: tauri::AppHandle) -> Result<String, String> {
    let mo = get_mo_path_internal(&app)?;
    let out = Command::new(&mo)
        .arg("--version")
        .output()
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[tauri::command]
fn update_mo_cli(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let _ = app.emit("mo-output", "→ Mise à jour de mole via Homebrew…");
        let child = Command::new("brew")
            .args(["upgrade", "tw93/tap/mole"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        match child {
            Err(e) => {
                let _ = app.emit("mo-output", format!("  ✗ Erreur : {}", e));
                let _ = app.emit("mo-done", 1i32);
            }
            Ok(mut child) => {
                let stdout = child.stdout.take().unwrap();
                let stderr_pipe = child.stderr.take().unwrap();
                let app_out = app.clone();
                let app_err = app.clone();
                let t1 = std::thread::spawn(move || {
                    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                        let _ = app_out.emit("mo-output", line);
                    }
                });
                let t2 = std::thread::spawn(move || {
                    for line in BufReader::new(stderr_pipe).lines().map_while(Result::ok) {
                        let _ = app_err.emit("mo-output", line);
                    }
                });
                t1.join().ok();
                t2.join().ok();
                let code = child.wait().map(|s| s.code().unwrap_or(0)).unwrap_or(1);
                let _ = app.emit("mo-done", code);
            }
        }
    });
}

// ── 8. Brew outdated casks ────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct BrewOutdated {
    pub name: String,
    pub installed_version: String,
    pub current_version: String,
    pub download_url: String,
    pub brew_managed: bool,
}

#[derive(Serialize, Clone)]
pub struct UpToDateApp {
    pub name: String,
    pub current_version: String,
}

#[derive(Serialize, Clone)]
pub struct BrewResult {
    pub updates: Vec<BrewOutdated>,
    pub up_to_date: Vec<UpToDateApp>, // apps gérées par brew, à jour
    pub up_to_date_cask: Vec<UpToDateApp>, // apps détectées via API (non brew), à jour
    pub checked: usize,
}

struct BrewResultCache {
    result: BrewResult,
    at: Instant,
}

fn brew_result_cache() -> &'static Mutex<Option<BrewResultCache>> {
    static CACHE: OnceLock<Mutex<Option<BrewResultCache>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn find_brew() -> Option<String> {
    for path in &["/opt/homebrew/bin/brew", "/usr/local/bin/brew"] {
        if std::path::Path::new(path).exists() {
            return Some(path.to_string());
        }
    }
    Command::new("which")
        .arg("brew")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

struct CaskApiCache {
    casks: Vec<serde_json::Value>,
    by_token: HashMap<String, usize>,
    by_bundle_id: HashMap<String, usize>,
}

// OnceLock garantit une seule initialisation, pas de race condition possible.
fn cask_api() -> &'static CaskApiCache {
    static CACHE: OnceLock<CaskApiCache> = OnceLock::new();
    CACHE.get_or_init(|| {
        let out = Command::new("curl")
            .args([
                "-s",
                "-L",
                "--compressed",
                "--max-time",
                "15",
                "https://formulae.brew.sh/api/cask.json",
            ])
            .output()
            .ok();
        let casks: Vec<serde_json::Value> = out
            .and_then(|o| {
                if o.status.success() {
                    Some(o.stdout)
                } else {
                    None
                }
            })
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut by_token: HashMap<String, usize> = HashMap::new();
        let mut by_bundle_id: HashMap<String, usize> = HashMap::new();
        for (i, c) in casks.iter().enumerate() {
            if let Some(tok) = c["token"].as_str() {
                by_token.insert(tok.to_string(), i);
            }
            if let Some(bid) = c["bundle_id"].as_str() {
                if !bid.is_empty() {
                    by_bundle_id.insert(bid.to_string(), i);
                }
            }
        }
        CaskApiCache {
            casks,
            by_token,
            by_bundle_id,
        }
    })
}

// Extrait la version et le download_url d'une entrée cask API.
fn cask_api_ver_url(cask: &serde_json::Value) -> (String, String) {
    let raw_ver = cask["version"].as_str().unwrap_or("");
    let ver = raw_ver.split(',').next().unwrap_or(raw_ver).to_string();
    let url = cask["url"].as_str().unwrap_or("").to_string();
    (ver, url)
}

// Cherche un cask dans l'index pour un app donnée (par token ou bundle_id).
fn lookup_cask<'a>(
    app_name: &str,
    bundle_id: Option<&str>,
    casks: &'a [serde_json::Value],
    by_token: &HashMap<String, usize>,
    by_bundle_id: &HashMap<String, usize>,
) -> Option<&'a serde_json::Value> {
    // 1. Essaie par bundle_id (le plus fiable)
    if let Some(bid) = bundle_id {
        if let Some(&i) = by_bundle_id.get(bid) {
            return Some(&casks[i]);
        }
    }
    // 2. Essaie par token (nom app → variantes)
    let candidates = [
        app_name.to_lowercase(),
        app_name.to_lowercase().replace(' ', "-"),
        app_name.to_lowercase().replace(' ', ""),
    ];
    for tok in &candidates {
        if let Some(&i) = by_token.get(tok.as_str()) {
            return Some(&casks[i]);
        }
    }
    None
}

#[tauri::command]
fn get_brew_outdated() -> BrewResult {
    // Retourner le cache si < 5 minutes
    {
        let g = brew_result_cache().lock().unwrap();
        if let Some(ref c) = *g {
            if c.at.elapsed() < Duration::from_secs(300) {
                return c.result.clone();
            }
        }
    }

    let api = cask_api();

    let brew = find_brew();

    let mut updates: Vec<BrewOutdated> = Vec::new();
    let mut up_to_date: Vec<UpToDateApp> = Vec::new(); // brew-managed, à jour
    let mut up_to_date_cask: Vec<UpToDateApp> = Vec::new(); // cask API (non brew), à jour

    if let Some(ref brew_path) = brew {
        // ── Phase 1 : casks gérés par Homebrew ──────────────────────────────

        let installed_with_ver: Vec<(String, String)> = Command::new(brew_path)
            .args(["list", "--versions", "--cask"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| {
                s.lines()
                    .filter(|l| !l.trim().is_empty())
                    .filter_map(|l| {
                        let mut it = l.splitn(2, ' ');
                        let name = it.next()?.trim().to_string();
                        let ver = it.next().unwrap_or("").trim().to_string();
                        Some((name, ver))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let total = installed_with_ver.len();

        let out = Command::new(brew_path)
            .args(["outdated", "--cask", "--json=v2"])
            .output();
        if let Ok(out) = out {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                let brew_outdated: Vec<BrewOutdated> = v["casks"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .map(|cask| {
                                let name = cask["name"].as_str().unwrap_or("").to_string();
                                let installed_version = cask["installed_versions"]
                                    .as_array()
                                    .and_then(|a| a.first())
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let current_version =
                                    cask["current_version"].as_str().unwrap_or("").to_string();
                                let download_url = api
                                    .by_token
                                    .get(&name)
                                    .and_then(|&i| api.casks[i]["url"].as_str())
                                    .unwrap_or("")
                                    .to_string();
                                BrewOutdated {
                                    name,
                                    installed_version,
                                    current_version,
                                    download_url,
                                    brew_managed: true,
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let outdated_names: std::collections::HashSet<String> =
                    brew_outdated.iter().map(|u| u.name.clone()).collect();
                updates.extend(brew_outdated);

                let brew_utd: Vec<UpToDateApp> = installed_with_ver
                    .iter()
                    .filter(|(n, _)| !outdated_names.contains(n.as_str()))
                    .map(|(n, v)| UpToDateApp {
                        name: n.clone(),
                        current_version: v.clone(),
                    })
                    .collect();
                up_to_date.extend(brew_utd);
            }
        }

        // ── Phase 1b : casks avec auto_updates=true (ex: Claude) ─────────────
        // brew outdated sans --greedy les ignore, on compare nous-mêmes via l'API
        {
            let outdated_names: std::collections::HashSet<&str> =
                updates.iter().map(|u| u.name.as_str()).collect();
            let phase1b: Vec<BrewOutdated> = installed_with_ver
                .iter()
                .filter(|(n, _)| !outdated_names.contains(n.as_str()))
                .filter_map(|(name, installed_ver)| {
                    let ver = installed_ver
                        .split_whitespace()
                        .next_back()
                        .unwrap_or(installed_ver.as_str());
                    let cask =
                        lookup_cask(name, None, &api.casks, &api.by_token, &api.by_bundle_id)?;
                    let (latest, url) = cask_api_ver_url(cask);
                    if version_gt(&latest, ver) {
                        let token = cask["token"].as_str().unwrap_or(name).to_string();
                        Some(BrewOutdated {
                            name: token,
                            installed_version: ver.to_string(),
                            current_version: latest,
                            download_url: url,
                            brew_managed: true,
                        })
                    } else {
                        None
                    }
                })
                .collect();
            if !phase1b.is_empty() {
                let phase1b_names: std::collections::HashSet<&str> =
                    phase1b.iter().map(|u| u.name.as_str()).collect();
                up_to_date.retain(|u| !phase1b_names.contains(u.name.as_str()));
                updates.extend(phase1b);
            }
        }

        // ── Phase 2 : apps Squirrel/Sparkle-sans-URL non gérées par brew ───

        let brew_known: std::collections::HashSet<String> = installed_with_ver
            .iter()
            .map(|(n, _)| n.to_lowercase())
            .collect();

        let phase2_apps: Vec<_> = collect_all_apps()
            .into_iter()
            .filter(|p| {
                let plist = p.join("Contents/Info.plist");
                if !plist.exists() {
                    return false;
                }
                let name_lc = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_lowercase())
                    .unwrap_or_default();
                if brew_known.contains(&name_lc) {
                    return false;
                }
                if p.join("Contents/_MASReceipt/receipt").exists() {
                    return false;
                }
                if plist_str(&plist, "SUFeedURL")
                    .map(|u| u.starts_with("http"))
                    .unwrap_or(false)
                {
                    return false;
                }
                if p.join("Contents/Resources/app-update.yml").exists() {
                    return false;
                }
                if p.join("Contents/Frameworks/DevMateKit.framework").exists() {
                    return false;
                }
                let has_squirrel = p.join("Contents/Frameworks/Squirrel.framework").exists();
                let has_sparkle_no_url = plist_str(&plist, "SUAllowsAutomaticUpdates").is_some()
                    || plist_str(&plist, "SUAutomaticallyUpdate").is_some();
                has_squirrel || has_sparkle_no_url
            })
            .collect();

        for p in phase2_apps {
            let plist = p.join("Contents/Info.plist");
            let name = match p.file_stem().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let current = match plist_str(&plist, "CFBundleShortVersionString")
                .or_else(|| plist_str(&plist, "CFBundleVersion"))
            {
                Some(v) if !v.is_empty() => v,
                _ => continue,
            };
            let bundle_id = plist_str(&plist, "CFBundleIdentifier");

            if let Some(cask) = lookup_cask(
                &name,
                bundle_id.as_deref(),
                &api.casks,
                &api.by_token,
                &api.by_bundle_id,
            ) {
                let (latest, url) = cask_api_ver_url(cask);
                let token = cask["token"].as_str().unwrap_or(&name).to_string();
                if version_gt(&latest, &current) {
                    updates.push(BrewOutdated {
                        name: token,
                        installed_version: current,
                        current_version: latest,
                        download_url: url,
                        brew_managed: false,
                    });
                } else {
                    up_to_date_cask.push(UpToDateApp {
                        name,
                        current_version: current,
                    });
                }
            }
        }

        updates.sort_by(|a, b| a.name.cmp(&b.name));
        let result = BrewResult {
            updates,
            up_to_date,
            up_to_date_cask,
            checked: total,
        };
        *brew_result_cache().lock().unwrap() = Some(BrewResultCache {
            result: result.clone(),
            at: Instant::now(),
        });
        result
    } else {
        // ── Pas de Homebrew : détection via API uniquement ───────────────────
        // Seulement les apps Squirrel/Sparkle-sans-URL (idem Phase 2)

        let phase_apps: Vec<_> = collect_all_apps()
            .into_iter()
            .filter(|p| {
                let plist = p.join("Contents/Info.plist");
                if !plist.exists() {
                    return false;
                }
                if p.join("Contents/_MASReceipt/receipt").exists() {
                    return false;
                }
                if plist_str(&plist, "SUFeedURL")
                    .map(|u| u.starts_with("http"))
                    .unwrap_or(false)
                {
                    return false;
                }
                if p.join("Contents/Resources/app-update.yml").exists() {
                    return false;
                }
                if p.join("Contents/Frameworks/DevMateKit.framework").exists() {
                    return false;
                }
                let has_squirrel = p.join("Contents/Frameworks/Squirrel.framework").exists();
                let has_sparkle_no_url = plist_str(&plist, "SUAllowsAutomaticUpdates").is_some()
                    || plist_str(&plist, "SUAutomaticallyUpdate").is_some();
                has_squirrel || has_sparkle_no_url
            })
            .collect();

        let total = phase_apps.len();

        for p in phase_apps {
            let plist = p.join("Contents/Info.plist");
            let name = match p.file_stem().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let current = match plist_str(&plist, "CFBundleShortVersionString")
                .or_else(|| plist_str(&plist, "CFBundleVersion"))
            {
                Some(v) if !v.is_empty() => v,
                _ => continue,
            };
            let bundle_id = plist_str(&plist, "CFBundleIdentifier");

            if let Some(cask) = lookup_cask(
                &name,
                bundle_id.as_deref(),
                &api.casks,
                &api.by_token,
                &api.by_bundle_id,
            ) {
                let (latest, url) = cask_api_ver_url(cask);
                let token = cask["token"].as_str().unwrap_or(&name).to_string();
                if version_gt(&latest, &current) {
                    updates.push(BrewOutdated {
                        name: token,
                        installed_version: current,
                        current_version: latest,
                        download_url: url,
                        brew_managed: false,
                    });
                } else {
                    up_to_date_cask.push(UpToDateApp {
                        name,
                        current_version: current,
                    });
                }
            }
        }

        updates.sort_by(|a, b| a.name.cmp(&b.name));
        let result = BrewResult {
            updates,
            up_to_date,
            up_to_date_cask,
            checked: total,
        };
        *brew_result_cache().lock().unwrap() = Some(BrewResultCache {
            result: result.clone(),
            at: Instant::now(),
        });
        result
    }
}

#[tauri::command]
fn update_brew_app(app: tauri::AppHandle, name: String, download_url: String, brew_managed: bool) {
    std::thread::spawn(move || {
        macro_rules! out {
            ($m:expr) => {
                let _ = app.emit("mo-output", $m.to_string());
            };
        }
        macro_rules! done {
            ($c:expr) => {
                let _ = app.emit("mo-done", $c as i32);
                return;
            };
        }

        out!(format!("→ Mise à jour de {}…", name));

        if let Some(brew_path) = find_brew() {
            if brew_managed {
                // Brew refuse de tourner en root → lancer directement comme utilisateur courant
                let mut child = match Command::new(&brew_path)
                    .args(["upgrade", "--cask", &name])
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                {
                    Ok(c) => c,
                    Err(e) => {
                        out!(format!("✗ Impossible de lancer brew : {}", e));
                        done!(1);
                    }
                };
                let stdout = child.stdout.take().unwrap();
                let stderr_pipe = child.stderr.take().unwrap();
                let app_o = app.clone();
                let t1 = std::thread::spawn(move || {
                    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                        let _ = app_o.emit("mo-output", line);
                    }
                });
                let app_e = app.clone();
                let stderr_collected =
                    std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
                let stderr_collected2 = stderr_collected.clone();
                let t2 = std::thread::spawn(move || {
                    for line in BufReader::new(stderr_pipe).lines().map_while(Result::ok) {
                        let _ = app_e.emit("mo-output", line.clone());
                        stderr_collected2.lock().unwrap().push(line);
                    }
                });
                t1.join().ok();
                t2.join().ok();
                let code = child.wait().map(|s| s.code().unwrap_or(0)).unwrap_or(1);
                if code == 0 {
                    out!(format!("✓ {} mis à jour avec succès", name));
                    done!(0);
                }
                // Conflit staging Caskroom (ancienne version encore présente) → retry --force
                let stderr_str = stderr_collected.lock().unwrap().join("\n");
                if stderr_str.contains("there is already an App")
                    || stderr_str.contains("already exists")
                {
                    out!("→ Conflit détecté, nouvelle tentative avec --force…");
                    let mut child2 = match Command::new(&brew_path)
                        .args(["upgrade", "--cask", &name, "--force"])
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .spawn()
                    {
                        Ok(c) => c,
                        Err(e) => {
                            out!(format!("✗ {}", e));
                            done!(1);
                        }
                    };
                    let stdout2 = child2.stdout.take().unwrap();
                    let stderr2 = child2.stderr.take().unwrap();
                    let app_o2 = app.clone();
                    let app_e2 = app.clone();
                    let t3 = std::thread::spawn(move || {
                        for line in BufReader::new(stdout2).lines().map_while(Result::ok) {
                            let _ = app_o2.emit("mo-output", line);
                        }
                    });
                    let t4 = std::thread::spawn(move || {
                        for line in BufReader::new(stderr2).lines().map_while(Result::ok) {
                            let _ = app_e2.emit("mo-output", line);
                        }
                    });
                    t3.join().ok();
                    t4.join().ok();
                    let code2 = child2.wait().map(|s| s.code().unwrap_or(0)).unwrap_or(1);
                    if code2 == 0 {
                        out!(format!("✓ {} mis à jour avec succès", name));
                    }
                    done!(code2);
                }
                done!(code);
            }
            // brew_managed=false → l'app n'est pas gérée par brew, téléchargement direct
        }

        // ── Téléchargement + installation directe du DMG/ZIP ─────────────────
        // (brew non installé OU app non gérée par brew)
        if download_url.is_empty() {
            out!("✗ Application non gérée par Homebrew et aucune URL de téléchargement disponible");
            done!(1);
        }

        // Répertoire temporaire aléatoire RAII — nettoyé automatiquement
        let work_dir = match burrow_tempdir() {
            Ok(d) => d,
            Err(e) => {
                out!(format!("✗ Erreur tmpdir : {}", e));
                done!(1);
            }
        };
        let tmp = work_dir.path().join("download");
        let tmp = tmp.to_string_lossy().into_owned();

        out!(format!("Téléchargement depuis {}…", download_url));
        let ok_dl = Command::new("curl")
            .args([
                "-s",
                "-L",
                "--fail",
                "--max-time",
                "120",
                "-A",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X)",
                "-o",
                &tmp,
                &download_url,
            ])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok_dl {
            out!("✗ Échec du téléchargement");
            done!(1);
        }

        let file_size = fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
        out!(format!("Taille : {:.1} Mo", file_size as f64 / 1_048_576.0));
        if file_size < 500_000 {
            out!("✗ Fichier trop petit — URL invalide ou page d'erreur");
            done!(1);
        }

        let mime = Command::new("file")
            .args(["-b", "--mime-type", &tmp])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();
        let mime = mime.trim().to_string();
        let ext = if mime.contains("bzip2") {
            let is_dmg = Command::new("hdiutil")
                .args(["imageinfo", &tmp])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if is_dmg {
                "dmg"
            } else {
                "tar.bz2"
            }
        } else if mime.contains("gzip") {
            "tar.gz"
        } else if mime.contains("x-xz") || mime.contains("xz") {
            "tar.xz"
        } else if mime.contains("zip") {
            "zip"
        } else if mime.contains("xar") || mime.contains("x-newton") {
            "pkg"
        } else {
            "dmg"
        };
        out!(format!("Format : {}", ext));

        let tmp_ext = work_dir.path().join(format!("download.{}", ext));
        let tmp_ext = tmp_ext.to_string_lossy().into_owned();
        if let Err(e) = fs::rename(&tmp, &tmp_ext) {
            out!(format!("✗ Renommage échoué : {}", e));
            done!(1);
        }
        let tmp = tmp_ext;

        // Quitter l'app si elle tourne
        if Command::new("pgrep")
            .args(["-x", &name])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            out!(format!("Fermeture de {}…", name));
            let _ = Command::new("osascript")
                .args(["-e", &format!("quit app \"{}\"", name)])
                .output();
            std::thread::sleep(Duration::from_secs(1));
        }

        // Trouver le dossier d'installation de l'app existante
        let install_dir = collect_all_apps()
            .into_iter()
            .find(|p| {
                p.file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_lowercase())
                    .as_deref()
                    == Some(&name.to_lowercase())
            })
            .and_then(|p| p.parent().map(|d| d.to_string_lossy().to_string()))
            .unwrap_or_else(|| "/Applications".to_string());

        let ok = match ext {
            "pkg" => {
                out!("Installation du package…");
                {
                    let q = guard::posix_shell_quote(&tmp);
                    run_admin_sh(&format!("installer -pkg {} -target /", q)).is_ok()
                }
            }
            "zip" | "tar.gz" | "tar.bz2" | "tar.xz" => {
                let tmp_dir = work_dir
                    .path()
                    .join("extracted")
                    .to_string_lossy()
                    .into_owned();
                fs::create_dir_all(&tmp_dir).ok();
                out!("Extraction…");
                let ok_x = if ext == "zip" {
                    Command::new("unzip")
                        .args(["-q", "-o", &tmp, "-d", &tmp_dir])
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false)
                } else {
                    Command::new("tar")
                        .args(["-xf", &tmp, "-C", &tmp_dir])
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false)
                };
                if !ok_x {
                    out!("✗ Extraction échouée");
                    let _ = fs::remove_file(&tmp);
                    done!(1);
                }
                let app_src = find_app_bundle(&tmp_dir);
                if app_src.is_none() {
                    out!("✗ Aucune .app dans l'archive");
                    let _ = fs::remove_dir_all(&tmp_dir);
                    let _ = fs::remove_file(&tmp);
                    done!(1);
                }
                let result = match copy_app(app_src.as_deref().unwrap(), &install_dir) {
                    Ok(()) => true,
                    Err(e) => {
                        out!(format!("✗ Copie échouée : {}", e));
                        false
                    }
                };
                let _ = fs::remove_dir_all(&tmp_dir);
                result
            }
            _ => {
                // dmg
                let before_vols: std::collections::HashSet<String> = fs::read_dir("/Volumes")
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.path().to_string_lossy().to_string())
                    .collect();
                out!("Montage du DMG…");
                let mount_out = Command::new("hdiutil")
                    .args(["attach", &tmp, "-nobrowse"])
                    .output();
                let Ok(mo) = mount_out else {
                    out!("✗ Impossible de lancer hdiutil");
                    let _ = fs::remove_file(&tmp);
                    done!(1);
                };
                let stdout_text = String::from_utf8_lossy(&mo.stdout);
                let mount = stdout_text
                    .lines()
                    .filter_map(|line| {
                        line.split('\t')
                            .next_back()
                            .map(|p| p.trim())
                            .filter(|p| p.starts_with("/Volumes/"))
                            .map(|p| p.to_string())
                    })
                    .next()
                    .or_else(|| {
                        let after_vols: std::collections::HashSet<String> =
                            fs::read_dir("/Volumes")
                                .into_iter()
                                .flatten()
                                .flatten()
                                .filter(|e| e.path().is_dir())
                                .map(|e| e.path().to_string_lossy().to_string())
                                .collect();
                        after_vols.difference(&before_vols).next().cloned()
                    });
                let Some(ref mp) = mount else {
                    let err = String::from_utf8_lossy(&mo.stderr).trim().to_string();
                    out!(format!(
                        "✗ Montage échoué : {}",
                        if err.is_empty() {
                            "volume introuvable".into()
                        } else {
                            err
                        }
                    ));
                    let _ = fs::remove_file(&tmp);
                    done!(1);
                };
                out!(format!("Volume monté : {}", mp));
                let app_src = find_app_bundle(mp);
                let Some(ref src) = app_src else {
                    out!("✗ Aucune .app dans le volume");
                    let _ = Command::new("hdiutil")
                        .args(["detach", mp, "-quiet"])
                        .status();
                    let _ = fs::remove_file(&tmp);
                    done!(1);
                };
                out!(format!("Copie → {}…", install_dir));
                let result = match copy_app(src, &install_dir) {
                    Ok(()) => true,
                    Err(e) => {
                        out!(format!("✗ Copie échouée : {}", e));
                        false
                    }
                };
                out!("Démontage…");
                let _ = Command::new("hdiutil")
                    .args(["detach", mp, "-quiet"])
                    .status();
                result
            }
        };

        let _ = fs::remove_file(&tmp);
        if ok {
            out!(format!("✓ {} mis à jour avec succès", name));
            done!(0);
        } else {
            out!(format!("✗ Échec de la mise à jour de {}", name));
            done!(1);
        }
    });
}

// ── 9a-2. Homebrew formulae ───────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct BrewFormulaResult {
    pub updates: Vec<BrewOutdated>,
    pub up_to_date: Vec<UpToDateApp>,
    pub checked: usize,
}

#[tauri::command]
fn get_brew_formula_outdated() -> BrewFormulaResult {
    let Some(brew_path) = find_brew() else {
        return BrewFormulaResult {
            updates: vec![],
            up_to_date: vec![],
            checked: 0,
        };
    };

    let installed: Vec<(String, String)> = Command::new(&brew_path)
        .args(["list", "--versions", "--formula"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| {
            s.lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| {
                    let mut it = l.splitn(2, ' ');
                    let name = it.next()?.trim().to_string();
                    let ver = it.next().unwrap_or("").trim().to_string();
                    let ver = ver.split_whitespace().last().unwrap_or(&ver).to_string();
                    Some((name, ver))
                })
                .collect()
        })
        .unwrap_or_default();

    let total = installed.len();

    let outdated: Vec<BrewOutdated> = Command::new(&brew_path)
        .args(["outdated", "--formula", "--json=v2"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["formulae"].as_array().cloned())
        .map(|arr| {
            arr.iter()
                .filter_map(|f| {
                    let name = f["name"].as_str()?.to_string();
                    let installed_version = f["installed_versions"]
                        .as_array()
                        .and_then(|a| a.last())
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let current_version = f["current_version"].as_str().unwrap_or("").to_string();
                    Some(BrewOutdated {
                        name,
                        installed_version,
                        current_version,
                        download_url: String::new(),
                        brew_managed: true,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let outdated_names: std::collections::HashSet<String> =
        outdated.iter().map(|u| u.name.clone()).collect();
    let up_to_date: Vec<UpToDateApp> = installed
        .iter()
        .filter(|(n, _)| !outdated_names.contains(n))
        .map(|(n, v)| UpToDateApp {
            name: n.clone(),
            current_version: v.clone(),
        })
        .collect();

    BrewFormulaResult {
        updates: outdated,
        up_to_date,
        checked: total,
    }
}

#[tauri::command]
fn update_brew_formula(app: tauri::AppHandle, name: String) {
    std::thread::spawn(move || {
        macro_rules! out {
            ($m:expr) => {
                let _ = app.emit("brew-formula-output", $m.to_string());
            };
        }
        macro_rules! done {
            ($c:expr) => {
                let _ = app.emit("brew-formula-done", $c as i32);
                return;
            };
        }

        out!(format!("→ Mise à jour de {}…", name));

        let Some(brew_path) = find_brew() else {
            out!("✗ Homebrew non trouvé");
            done!(1);
        };

        let child = Command::new(&brew_path)
            .args(["upgrade", &name])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        match child {
            Err(e) => {
                out!(format!("✗ Erreur : {}", e));
                done!(1);
            }
            Ok(mut child) => {
                let stdout = child.stdout.take().unwrap();
                let stderr_pipe = child.stderr.take().unwrap();
                let app_out = app.clone();
                let app_err = app.clone();
                let t1 = std::thread::spawn(move || {
                    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                        let _ = app_out.emit("brew-formula-output", line);
                    }
                });
                let t2 = std::thread::spawn(move || {
                    for line in BufReader::new(stderr_pipe).lines().map_while(Result::ok) {
                        let _ = app_err.emit("brew-formula-output", line);
                    }
                });
                t1.join().ok();
                t2.join().ok();
                let code = child.wait().map(|s| s.code().unwrap_or(0)).unwrap_or(1);
                done!(code);
            }
        }
    });
}

// ── 9b. ClamAV ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ClamavInfo {
    pub installed: bool,
    pub version: String,
    pub freshclam_path: String,
    pub has_database: bool,
    pub db_path: String,
    pub db_version: String,
}

#[derive(Serialize, Clone)]
pub struct QuickMetrics {
    pub cpu_usage: f64,
    pub cpu_per_core: Vec<f64>,
    pub cpu_core_count: usize,
    pub cpu_load1: f64,
    pub cpu_load5: f64,
    pub cpu_load15: f64,
    pub mem_used: u64,
    pub mem_total: u64,
    pub mem_used_percent: f64,
    pub mem_swap_used: u64,
    pub mem_swap_total: u64,
    pub disk_used: u64,
    pub disk_total: u64,
    pub disk_used_percent: f64,
    pub uptime_secs: u64,
    pub gpu_busy_percent: f64,
    pub fan_speed_rpm: f64,
    // Temperatures (°C) — from IOHID, no root
    pub cpu_temp: f64,
    pub gpu_temp: f64,
    pub soc_temp: f64,
    pub nand_temp: f64,
    pub ane_temp: f64,
    // Power (Watts) — from IOReport Energy Model, no root
    pub cpu_power: f64,
    pub gpu_power: f64,
    pub ram_power: f64,
    pub ane_power: f64,
}

fn read_db_version(db_path: &str) -> String {
    use std::io::Read;
    for name in &["daily.cvd", "daily.cld", "main.cvd", "main.cld"] {
        let p = Path::new(db_path).join(name);
        if let Ok(mut f) = fs::File::open(&p) {
            let mut buf = [0u8; 200];
            if f.read(&mut buf).is_ok() {
                let s = String::from_utf8_lossy(&buf);
                // Header: ClamAV-VDB:daily:27425:...
                let parts: Vec<&str> = s.split(':').collect();
                if parts.len() > 2 {
                    let v = parts[2].trim().to_string();
                    if !v.is_empty() {
                        return v;
                    }
                }
            }
        }
    }
    String::new()
}

#[derive(Serialize, Deserialize, Clone)]
pub struct QuarantineEntry {
    pub name: String,
    pub quarantine_path: String,
    pub original_path: String,
    pub size_bytes: u64,
    pub quarantined_at: String,
}

fn find_clamscan(app: &tauri::AppHandle) -> Option<String> {
    if let Ok(res) = app.path().resource_dir() {
        let bundled = res.join("clamav").join("bin").join("clamscan");
        if bundled.exists() {
            return Some(bundled.to_string_lossy().to_string());
        }
    }
    for path in &[
        "/opt/homebrew/bin/clamscan",
        "/usr/local/bin/clamscan",
        "/usr/bin/clamscan",
    ] {
        if std::path::Path::new(path).exists() {
            return Some(path.to_string());
        }
    }
    Command::new("which")
        .arg("clamscan")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn find_freshclam(app: &tauri::AppHandle) -> Option<String> {
    if let Ok(res) = app.path().resource_dir() {
        let bundled = res.join("clamav").join("bin").join("freshclam");
        if bundled.exists() {
            return Some(bundled.to_string_lossy().to_string());
        }
    }
    for path in &[
        "/opt/homebrew/bin/freshclam",
        "/usr/local/bin/freshclam",
        "/usr/bin/freshclam",
    ] {
        if std::path::Path::new(path).exists() {
            return Some(path.to_string());
        }
    }
    Command::new("which")
        .arg("freshclam")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn clamav_db_dir(app: &tauri::AppHandle) -> std::path::PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_default()
        .join("clamav")
        .join("db")
}

fn has_clamav_database(db_dir: &Path) -> bool {
    db_dir.exists()
        && fs::read_dir(db_dir)
            .map(|d| {
                d.flatten().any(|e| {
                    let n = e.file_name();
                    let s = n.to_string_lossy();
                    s.ends_with(".cvd") || s.ends_with(".cld")
                })
            })
            .unwrap_or(false)
}

fn find_clamav_database(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    let burrow = clamav_db_dir(app);
    if has_clamav_database(&burrow) {
        return Some(burrow);
    }

    for sys in &[
        "/opt/homebrew/share/clamav",
        "/usr/local/share/clamav",
        "/var/lib/clamav",
        "/opt/homebrew/var/lib/clamav",
    ] {
        let p = std::path::Path::new(sys);
        if has_clamav_database(p) {
            return Some(p.to_path_buf());
        }
    }
    None
}

fn freshclam_conf_path(app: &tauri::AppHandle) -> std::path::PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_default()
        .join("freshclam.conf")
}

fn write_freshclam_conf(app: &tauri::AppHandle) -> Result<String, String> {
    let db_dir = clamav_db_dir(app);
    let _ = fs::create_dir_all(&db_dir);
    let conf_path = freshclam_conf_path(app);
    let content = format!(
        "DatabaseDirectory {}\nDatabaseMirror database.clamav.net\nMaxAttempts 3\nConnectTimeout 30\nReceiveTimeout 30\n",
        db_dir.to_string_lossy()
    );
    fs::write(&conf_path, content).map_err(|e| e.to_string())?;
    Ok(conf_path.to_string_lossy().to_string())
}

fn quarantine_dir() -> std::path::PathBuf {
    home_dir().join(".burrow-quarantine")
}

fn quarantine_meta_path() -> std::path::PathBuf {
    quarantine_dir().join(".metadata.json")
}

fn read_quarantine_meta() -> Vec<serde_json::Value> {
    fs::read_to_string(quarantine_meta_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_quarantine_meta(entries: &[serde_json::Value]) {
    let _ = fs::create_dir_all(quarantine_dir());
    let _ = fs::write(
        quarantine_meta_path(),
        serde_json::to_string_pretty(entries).unwrap_or_default(),
    );
}

#[tauri::command]
fn check_clamav(app: tauri::AppHandle) -> ClamavInfo {
    let found_db = find_clamav_database(&app);
    let has_db = found_db.is_some();
    let db_path = found_db
        .unwrap_or_else(|| clamav_db_dir(&app))
        .to_string_lossy()
        .to_string();
    let db_version = if has_db {
        read_db_version(&db_path)
    } else {
        String::new()
    };
    match find_clamscan(&app) {
        Some(ref path) => {
            let version = Command::new(path)
                .arg("--version")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.lines().next().unwrap_or("").trim().to_string())
                .unwrap_or_default();
            ClamavInfo {
                installed: true,
                version,
                freshclam_path: find_freshclam(&app).unwrap_or_default(),
                has_database: has_db,
                db_path,
                db_version,
            }
        }
        None => ClamavInfo {
            installed: false,
            version: String::new(),
            freshclam_path: String::new(),
            has_database: false,
            db_path,
            db_version: String::new(),
        },
    }
}

#[tauri::command]
fn start_clamav_scan(app: tauri::AppHandle, paths: Vec<String>) {
    let Some(clamscan) = find_clamscan(&app) else {
        let _ = app.emit("scan-done", 2i32);
        return;
    };
    let db_path = find_clamav_database(&app).map(|p| p.to_string_lossy().to_string());

    std::thread::spawn(move || {
        let home = home_dir();
        let expanded: Vec<String> = paths
            .iter()
            .map(|p| {
                if let Some(stripped) = p.strip_prefix("~/") {
                    format!("{}/{}", home.to_string_lossy(), stripped)
                } else if p == "~" {
                    home.to_string_lossy().to_string()
                } else {
                    p.clone()
                }
            })
            .collect();

        let mut cmd = Command::new(&clamscan);
        cmd.arg("-r").arg("--no-summary");
        if let Some(ref db) = db_path {
            cmd.arg(format!("--database={}", db));
        }
        cmd.args(&expanded)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = app.emit("scan-line", format!("✗ {}", e));
                let _ = app.emit("scan-done", 2i32);
                return;
            }
        };

        *scan_pid_store().lock().unwrap() = Some(child.id());

        let stdout = child.stdout.take().unwrap();
        let stderr_pipe = child.stderr.take().unwrap();
        let app_out = app.clone();
        let app_err = app.clone();

        let t1 = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = app_out.emit("scan-line", line);
            }
        });
        let t2 = std::thread::spawn(move || {
            for line in BufReader::new(stderr_pipe).lines().map_while(Result::ok) {
                let _ = app_err.emit("scan-line", line);
            }
        });
        t1.join().ok();
        t2.join().ok();

        let code = child.wait().map(|s| s.code().unwrap_or(0)).unwrap_or(1);
        *scan_pid_store().lock().unwrap() = None;
        let _ = app.emit("scan-done", code);
    });
}

#[tauri::command]
fn cancel_clamav_scan() {
    if let Some(pid) = *scan_pid_store().lock().unwrap() {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
    }
}

#[tauri::command]
fn update_clamav_defs(app: tauri::AppHandle) {
    let Some(freshclam) = find_freshclam(&app) else {
        let _ = app.emit(
            "clamav-update-line",
            "freshclam non trouvé — installez ClamAV : brew install clamav",
        );
        let _ = app.emit("clamav-update-done", 1i32);
        return;
    };

    let conf_path = match write_freshclam_conf(&app) {
        Ok(p) => p,
        Err(e) => {
            let _ = app.emit("clamav-update-line", format!("✗ conf error: {}", e));
            let _ = app.emit("clamav-update-done", 1i32);
            return;
        }
    };

    let db_dir = clamav_db_dir(&app);
    let _ = app.emit(
        "clamav-update-line",
        format!("→ Dossier base de données : {}", db_dir.to_string_lossy()),
    );

    std::thread::spawn(move || {
        let child = Command::new(&freshclam)
            .arg(format!("--config-file={}", conf_path))
            .arg("--stdout")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        match child {
            Err(e) => {
                let _ = app.emit("clamav-update-line", format!("✗ {}", e));
                let _ = app.emit("clamav-update-done", 1i32);
            }
            Ok(mut child) => {
                let stdout = child.stdout.take().unwrap();
                let stderr_pipe = child.stderr.take().unwrap();
                let app_out = app.clone();
                let app_err = app.clone();
                let t1 = std::thread::spawn(move || {
                    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                        let _ = app_out.emit("clamav-update-line", line);
                    }
                });
                let t2 = std::thread::spawn(move || {
                    for line in BufReader::new(stderr_pipe).lines().map_while(Result::ok) {
                        let _ = app_err.emit("clamav-update-line", line);
                    }
                });
                t1.join().ok();
                t2.join().ok();
                let code = child.wait().map(|s| s.code().unwrap_or(0)).unwrap_or(1);
                let _ = app.emit("clamav-update-done", code);
            }
        }
    });
}

#[tauri::command]
fn list_quarantine() -> Vec<QuarantineEntry> {
    read_quarantine_meta()
        .iter()
        .filter_map(|m| {
            let name = m["name"].as_str()?.to_string();
            let original_path = m["original_path"].as_str()?.to_string();
            let quarantine_path = quarantine_dir().join(&name).to_string_lossy().to_string();
            let quarantined_at = m["quarantined_at"].as_str().unwrap_or("").to_string();
            let size_bytes = fs::metadata(&quarantine_path).map(|m| m.len()).unwrap_or(0);
            Some(QuarantineEntry {
                name,
                quarantine_path,
                original_path,
                size_bytes,
                quarantined_at,
            })
        })
        .collect()
}

#[tauri::command]
fn quarantine_file(original_path: String) -> Result<(), String> {
    guard::validate_trash_path(&original_path)?;
    let src = std::path::Path::new(&original_path);
    let fname = src
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Chemin invalide")?
        .to_string();

    let dir = quarantine_dir();
    let _ = fs::create_dir_all(&dir);

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let unique_name = format!("{}_{}", ts, fname);
    let dest = dir.join(&unique_name);

    if fs::rename(src, &dest).is_err() {
        fs::copy(src, &dest).map_err(|e| e.to_string())?;
        fs::remove_file(src).ok();
    }

    let mut meta = read_quarantine_meta();
    meta.push(serde_json::json!({
        "name": unique_name,
        "original_path": original_path,
        "quarantined_at": ts.to_string()
    }));
    write_quarantine_meta(&meta);
    Ok(())
}

#[tauri::command]
fn restore_from_quarantine(name: String) -> Result<(), String> {
    guard::validate_quarantine_name(&name)?;
    let meta = read_quarantine_meta();
    let entry = meta
        .iter()
        .find(|m| m["name"].as_str() == Some(&name))
        .ok_or("Entrée introuvable")?;
    let original_path = entry["original_path"]
        .as_str()
        .ok_or("Chemin original manquant")?;
    let qpath = quarantine_dir().join(&name);

    if fs::rename(&qpath, original_path).is_err() {
        fs::copy(&qpath, original_path).map_err(|e| e.to_string())?;
        fs::remove_file(&qpath).ok();
    }

    let new_meta: Vec<_> = meta
        .into_iter()
        .filter(|m| m["name"].as_str() != Some(&name))
        .collect();
    write_quarantine_meta(&new_meta);
    Ok(())
}

#[tauri::command]
fn delete_from_quarantine(name: String) -> Result<(), String> {
    guard::validate_quarantine_name(&name)?;
    let qpath = quarantine_dir().join(&name);
    if qpath.is_dir() {
        fs::remove_dir_all(&qpath)
    } else {
        fs::remove_file(&qpath)
    }
    .map_err(|e| e.to_string())?;

    let meta = read_quarantine_meta();
    let new_meta: Vec<_> = meta
        .into_iter()
        .filter(|m| m["name"].as_str() != Some(&name))
        .collect();
    write_quarantine_meta(&new_meta);
    Ok(())
}

fn mo_update_cache() -> &'static Mutex<Option<(bool, Instant)>> {
    static C: OnceLock<Mutex<Option<(bool, Instant)>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

#[tauri::command]
fn check_mo_update_available() -> bool {
    // Cache 10 minutes : brew outdated contacte le serveur, c'est lent
    {
        let g = mo_update_cache().lock().unwrap();
        if let Some((v, t)) = *g {
            if t.elapsed() < Duration::from_secs(600) {
                return v;
            }
        }
    }
    let result = {
        let output = Command::new("brew")
            .args(["outdated", "--formula", "tw93/tap/mole"])
            .output();
        match output {
            Ok(o) if o.status.success() => !String::from_utf8_lossy(&o.stdout).trim().is_empty(),
            _ => false,
        }
    };
    *mo_update_cache().lock().unwrap() = Some((result, Instant::now()));
    result
}

#[tauri::command]
fn check_clamav_defs_outdated(app: tauri::AppHandle) -> bool {
    let db_dir = find_clamav_database(&app).unwrap_or_else(|| clamav_db_dir(&app));
    for name in &["daily.cvd", "daily.cld"] {
        let path = db_dir.join(name);
        if path.exists() {
            return fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.elapsed().ok())
                .map(|e| e.as_secs() > 24 * 3600)
                .unwrap_or(false);
        }
    }
    false
}

#[tauri::command]
fn pick_folder() -> Option<String> {
    let out = Command::new("osascript")
        .args([
            "-e",
            "POSIX path of (choose folder with prompt \"Select folder to scan\")",
        ])
        .output()
        .ok()?;
    if out.status.success() {
        let path = String::from_utf8_lossy(&out.stdout)
            .trim()
            .trim_end_matches('/')
            .to_string();
        if path.is_empty() {
            None
        } else {
            Some(path)
        }
    } else {
        None
    }
}

// ── 9. Folder top files ───────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub is_dir: bool,
}

#[tauri::command]
fn get_folder_top_files(folder_path: String, limit: u32) -> Vec<FileEntry> {
    // Expansion de ~ sans shell
    let home = home_dir();
    let expanded = if let Some(stripped) = folder_path.strip_prefix("~/") {
        home.join(stripped)
    } else if folder_path == "~" {
        home
    } else {
        std::path::PathBuf::from(&folder_path)
    };

    // Validation du chemin (pas de traversée, pas de zones système)
    let validated = match guard::validate_trash_path(&expanded.to_string_lossy()) {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    if !validated.is_dir() {
        return vec![];
    }

    // Implémentation Rust pure — aucune interpolation shell
    let Ok(entries) = fs::read_dir(&validated) else {
        return vec![];
    };
    let mut items: Vec<FileEntry> = entries
        .flatten()
        .filter_map(|entry| {
            let p = entry.path();
            let name = p.file_name()?.to_str()?.to_string();
            let is_dir = p.is_dir();
            let size_bytes = if is_dir {
                folder_size_approx(&p)
            } else {
                fs::metadata(&p).map(|m| m.len()).unwrap_or(0)
            };
            Some(FileEntry {
                name,
                path: p.to_string_lossy().to_string(),
                size_bytes,
                is_dir,
            })
        })
        .collect();

    items.sort_by_key(|k| std::cmp::Reverse(k.size_bytes));
    items.truncate(limit as usize);
    items
}

// ── iCloud / FileProvider safety guard ───────────────────────────────────────

fn is_icloud_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    if s.contains("com~apple~") {
        return true;
    }
    let icloud_roots = [
        "Library/Mobile Documents",
        "Library/CloudStorage",
        "Library/Application Support/FileProvider",
        "Library/Application Support/CloudDocs",
        "Library/Daemon Containers",
        "Library/Caches/CloudKit",
        "Library/Caches/com.apple.bird",
        "Library/Caches/com.apple.cloudkit",
        "Library/Caches/com.apple.cloudd",
        "Library/Caches/com.apple.FileProvider",
    ];
    icloud_roots.iter().any(|root| s.contains(root))
}

// ── SIP / immutability detection (lstat BSD flags) ────────────────────────────

fn is_sip_protected(path: &Path) -> bool {
    use std::ffi::CString;
    let Ok(cpath) = CString::new(path.to_string_lossy().as_bytes()) else {
        return false;
    };
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        if libc::lstat(cpath.as_ptr(), &mut st) != 0 {
            return false;
        }
        const SF_RESTRICTED: u32 = 0x00080000;
        const SF_IMMUTABLE: u32 = 0x00020000;
        const UF_IMMUTABLE: u32 = 0x00000002;
        (st.st_flags & (SF_RESTRICTED | SF_IMMUTABLE | UF_IMMUTABLE)) != 0
    }
}

// ── Container UUID discovery ──────────────────────────────────────────────────

fn container_bundle_id(container_path: &Path) -> Option<String> {
    let meta = container_path.join(".com.apple.containermanagerd.metadata.plist");
    if !meta.exists() {
        return None;
    }
    let out = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print MCMMetadataIdentifier", &meta.to_string_lossy()])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() || s.starts_with("Print:") {
        None
    } else {
        Some(s)
    }
}

// ── Per-app match rules (26 entries, ported from PureMac Conditions.swift) ───

fn app_condition_matches(fname: &str, bundle_id: &str, app_name_lower: &str) -> Option<bool> {
    let f = fname.to_lowercase();
    let b = bundle_id.to_lowercase();
    let app = app_name_lower;

    // Xcode vs Xcodes disambiguation
    if app == "xcode" && (f.contains("xcodes") || b.contains("xcodes")) {
        return Some(false);
    }
    if app == "xcodes" && (f.contains("/xcode/") || b == "com.apple.dt.xcode") {
        return Some(false);
    }

    // Chrome / Chromium disambiguation
    if app == "google chrome" && f.contains("chromium") {
        return Some(false);
    }
    if app == "chromium" && (f.contains("googlechrome") || f.contains("google chrome")) {
        return Some(false);
    }

    // VS Code / VS Code Insiders
    if app == "visual studio code" && f.contains("insiders") {
        return Some(false);
    }
    if app == "visual studio code - insiders" && !f.contains("insiders") && f.contains("code") {
        return Some(false);
    }

    // Microsoft Teams vs generic "teams"
    if app == "microsoft teams" && !f.contains("microsoft") && !b.contains("microsoft") {
        return Some(false);
    }

    // Firefox / Firefox Developer Edition / Firefox Nightly
    if app == "firefox"
        && (f.contains("developer") || f.contains("nightly") || b.contains("nightly"))
    {
        return Some(false);
    }

    // Brave
    if app == "brave browser" && !f.contains("brave") && !b.contains("brave") {
        return Some(false);
    }

    // Arc — only match arc-specific paths
    if app == "arc" && !f.contains("arc") && !b.contains("thebrowser") {
        return Some(false);
    }

    // 1Password
    if app == "1password 7" || app == "1password" {
        if f.contains("1password") || b.contains("1password") || b.contains("agilebits") {
            return Some(true);
        }
        return Some(false);
    }

    // Zoom
    if app == "zoom" && (f.contains("zoomus") || b.contains("zoomus") || f.contains("zoom.us")) {
        return Some(true);
    }

    // Stats (menu bar)
    if app == "stats"
        && (f.contains("stats") && !f.contains("istatistica") && !f.contains("system stats"))
    {
        return Some(true);
    }

    None // no override — use default matching
}

// ── APFS purgeable space ──────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct PurgeableInfo {
    pub purgeable_bytes: u64,
    pub free_bytes: u64,
    pub total_bytes: u64,
}

#[tauri::command]
fn get_purgeable_space() -> PurgeableInfo {
    let out = Command::new("/usr/sbin/diskutil")
        .args(["info", "-plist", "/"])
        .output()
        .unwrap_or_else(|_| std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: vec![],
            stderr: vec![],
        });
    let raw = String::from_utf8_lossy(&out.stdout);

    fn plist_u64(s: &str, key: &str) -> u64 {
        let needle = format!("<key>{}</key>", key);
        s.find(&needle)
            .and_then(|i| {
                let rest = &s[i + needle.len()..];
                let start = rest.find("<integer>")? + 9;
                let end = rest[start..].find("</integer>")?;
                rest[start..start + end].trim().parse::<u64>().ok()
            })
            .unwrap_or(0)
    }

    let total_bytes = plist_u64(&raw, "TotalSize");
    let free_bytes = plist_u64(&raw, "VolumeAvailableSpace");
    let purgeable_bytes = plist_u64(&raw, "APFSContainerFree").saturating_sub(free_bytes);

    PurgeableInfo {
        purgeable_bytes,
        free_bytes,
        total_bytes,
    }
}

#[tauri::command]
fn purge_purgeable_space() -> Result<u64, String> {
    let before = get_purgeable_space().purgeable_bytes;
    let out = Command::new("/usr/sbin/diskutil")
        .args(["apfs", "purgePurgeable", "/"])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(before)
}

// ── Time Machine local snapshots ──────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct TmSnapshot {
    pub date: String, // e.g. "2024-12-01-153022"
    pub size_bytes: u64,
}

#[tauri::command]
fn scan_tm_snapshots() -> Vec<TmSnapshot> {
    let list_out = Command::new("/usr/bin/tmutil")
        .args(["listlocalsnapshots", "/"])
        .output();
    let Ok(list_out) = list_out else {
        return vec![];
    };
    let list_str = String::from_utf8_lossy(&list_out.stdout);

    // Each line: "com.apple.TimeMachine.2024-12-01-153022.local"
    let dates: Vec<String> = list_str
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() {
                return None;
            }
            // Extract date part after "TimeMachine."
            l.find("TimeMachine.").map(|i| {
                let rest = &l[i + 12..];
                rest.trim_end_matches(".local").to_string()
            })
        })
        .collect();

    if dates.is_empty() {
        return vec![];
    }

    // Get sizes from diskutil apfs listSnapshots / -plist
    let size_out = Command::new("/usr/sbin/diskutil")
        .args(["apfs", "listSnapshots", "/", "-plist"])
        .output();

    let mut sizes: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    if let Ok(size_out) = size_out {
        let raw = String::from_utf8_lossy(&size_out.stdout);
        // Parse: <key>SnapshotName</key><string>com.apple.TimeMachine.DATE.local</string>
        //        ... <key>SnapshotSize</key><integer>NNN</integer>
        let mut snap_name = String::new();
        for line in raw.lines() {
            let l = line.trim();
            if l.starts_with("<string>") && l.contains("TimeMachine.") {
                let s = l
                    .trim_start_matches("<string>")
                    .trim_end_matches("</string>");
                if let Some(i) = s.find("TimeMachine.") {
                    let rest = &s[i + 12..];
                    snap_name = rest.trim_end_matches(".local").to_string();
                }
            } else if l.starts_with("<integer>") && !snap_name.is_empty() {
                let n = l
                    .trim_start_matches("<integer>")
                    .trim_end_matches("</integer>")
                    .parse::<u64>()
                    .unwrap_or(0);
                if n > 0 {
                    sizes.insert(snap_name.clone(), n);
                    snap_name.clear();
                }
            }
        }
    }

    dates
        .into_iter()
        .map(|date| {
            let size_bytes = sizes.get(&date).copied().unwrap_or(0);
            TmSnapshot { date, size_bytes }
        })
        .collect()
}

#[tauri::command]
fn delete_tm_snapshot(date: String) -> Result<(), String> {
    let out = Command::new("/usr/bin/tmutil")
        .args(["deletelocalsnapshots", &date])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

// ── Xcode simulator runtimes ──────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct SimRuntime {
    pub identifier: String,
    pub version: String,
    pub build: String,
    pub platform: String,
    pub size_bytes: u64,
    pub deletable: bool,
    pub last_used: String,
}

#[tauri::command]
fn scan_simulator_runtimes() -> Vec<SimRuntime> {
    // Check Xcode CLT available
    let xcode_ok = Command::new("/usr/bin/xcode-select")
        .arg("-p")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !xcode_ok {
        return vec![];
    }

    let out = Command::new("/usr/bin/xcrun")
        .args(["simctl", "runtime", "list", "-j"])
        .output();
    let Ok(out) = out else { return vec![] };
    let Ok(json_str) = String::from_utf8(out.stdout) else {
        return vec![];
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&json_str) else {
        return vec![];
    };

    // Response is an object with identifier keys
    let map = match v.as_object() {
        Some(m) => m,
        None => return vec![],
    };

    map.values()
        .filter_map(|rt| {
            let s = |k: &str| rt[k].as_str().unwrap_or("").to_string();
            let identifier = s("identifier");
            if identifier.is_empty() {
                return None;
            }
            let size_bytes = rt["sizeBytes"].as_u64().unwrap_or(0);
            let deletable = rt["deletable"].as_bool().unwrap_or(false);
            // platform: "com.apple.CoreSimulator.SimRuntime.iOS-17-4" → "iOS 17.4"
            let platform_raw = s("platformIdentifier");
            let platform = platform_raw
                .split('.')
                .next_back()
                .unwrap_or(&platform_raw)
                .replace('-', " ")
                .to_string();
            Some(SimRuntime {
                identifier,
                version: s("version"),
                build: s("build"),
                platform,
                size_bytes,
                deletable,
                last_used: s("lastUsedAt"),
            })
        })
        .collect()
}

#[tauri::command]
fn delete_simulator_runtime(identifier: String) -> Result<(), String> {
    let out = Command::new("/usr/bin/xcrun")
        .args(["simctl", "runtime", "delete", &identifier])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

// ── AI Apps cache paths ───────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct AiCacheItem {
    pub id: String,
    pub label: String,
    pub path: String,
    pub size_bytes: u64,
}

#[tauri::command]
fn scan_ai_caches() -> Vec<AiCacheItem> {
    let home = home_dir();
    let candidates: &[(&str, &str, &str)] = &[
        // Ollama
        ("ollama_logs", "Ollama — Logs", ".ollama/logs"),
        ("ollama_cache", "Ollama — Cache", "Library/Caches/ollama"),
        (
            "ollama_electron_cache",
            "Ollama — Cache Electron",
            "Library/Caches/com.electron.ollama",
        ),
        (
            "ollama_webkit",
            "Ollama — WebKit",
            "Library/WebKit/com.electron.ollama",
        ),
        (
            "ollama_state",
            "Ollama — État sauvegardé",
            "Library/Saved Application State/com.electron.ollama.savedState",
        ),
        // LM Studio
        ("lmstudio_logs", "LM Studio — Logs", ".lmstudio/server-logs"),
        (
            "lmstudio_conv",
            "LM Studio — Conversations",
            ".lmstudio/conversations",
        ),
        // AnythingLLM
        (
            "anythingllm_cache",
            "AnythingLLM — Cache",
            "Library/Caches/com.electron.anythingllm",
        ),
        // Open WebUI
        (
            "openwebui_cache",
            "Open WebUI — Cache",
            "Library/Caches/open-webui",
        ),
        // Cursor (AI editor)
        ("cursor_cache", "Cursor — Cache", "Library/Caches/Cursor"),
        ("cursor_logs", "Cursor — Logs", "Library/Logs/Cursor"),
        // GitHub Copilot cache
        (
            "copilot_cache",
            "GitHub Copilot — Cache",
            "Library/Caches/com.github.GitHubDesktop",
        ),
    ];

    candidates
        .iter()
        .filter_map(|(id, label, rel)| {
            let p = home.join(rel);
            if !p.exists() {
                return None;
            }
            let size_bytes = if p.is_dir() {
                du_bytes(&p)
            } else {
                fs::metadata(&p).map(|m| m.len()).unwrap_or(0)
            };
            if size_bytes == 0 {
                return None;
            }
            Some(AiCacheItem {
                id: id.to_string(),
                label: label.to_string(),
                path: p.to_string_lossy().to_string(),
                size_bytes,
            })
        })
        .collect()
}

#[tauri::command]
fn clean_ai_caches(ids: Vec<String>) -> Result<u64, String> {
    let home = home_dir();
    let all = scan_ai_caches();
    let mut freed = 0u64;
    for item in all.iter().filter(|i| ids.contains(&i.id)) {
        let p = Path::new(&item.path);
        freed += if p.is_dir() {
            du_bytes(p)
        } else {
            fs::metadata(p).map(|m| m.len()).unwrap_or(0)
        };
        if p.is_dir() {
            let _ = fs::remove_dir_all(p);
        } else {
            let _ = fs::remove_file(p);
        }
    }
    let _ = home; // suppress warning
    Ok(freed)
}

// ── Dev caches (npm / yarn / pnpm / brew) — dynamic path detection ────────────

#[derive(Serialize, Clone)]
pub struct DevCacheItem {
    pub id: String,
    pub label: String,
    pub path: String,
    pub size_bytes: u64,
}

fn detect_cli_cache(cli_candidates: &[&str], args: &[&str]) -> Option<String> {
    for cli in cli_candidates {
        let p = Path::new(cli);
        if !p.exists() {
            continue;
        }
        let out = Command::new(p).args(args).output().ok()?;
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() && Path::new(&s).exists() {
            return Some(s);
        }
    }
    None
}

#[tauri::command]
fn scan_dev_caches() -> Vec<DevCacheItem> {
    let home = home_dir();

    let npm_path = detect_cli_cache(
        &[
            "/opt/homebrew/bin/npm",
            "/usr/local/bin/npm",
            &home.join(".local/bin/npm").to_string_lossy(),
        ],
        &["config", "get", "cache"],
    )
    .unwrap_or_else(|| home.join(".npm").to_string_lossy().to_string());

    let yarn_path = detect_cli_cache(
        &["/opt/homebrew/bin/yarn", "/usr/local/bin/yarn"],
        &["cache", "dir"],
    )
    .unwrap_or_else(|| {
        home.join("Library/Caches/Yarn")
            .to_string_lossy()
            .to_string()
    });

    let pnpm_path = detect_cli_cache(
        &["/opt/homebrew/bin/pnpm", "/usr/local/bin/pnpm"],
        &["store", "path"],
    )
    .unwrap_or_else(|| {
        home.join("Library/pnpm/store")
            .to_string_lossy()
            .to_string()
    });

    let brew_path = detect_cli_cache(
        &["/opt/homebrew/bin/brew", "/usr/local/bin/brew"],
        &["--cache"],
    )
    .unwrap_or_else(|| {
        home.join("Library/Caches/Homebrew")
            .to_string_lossy()
            .to_string()
    });

    let bun_path = home
        .join("Library/Caches/bun")
        .to_string_lossy()
        .to_string();
    let pip_path = home
        .join("Library/Caches/pip")
        .to_string_lossy()
        .to_string();
    let cargo_path = home
        .join(".cargo/registry/cache")
        .to_string_lossy()
        .to_string();
    let gradle_path = home.join(".gradle/caches").to_string_lossy().to_string();
    let maven_path = home.join(".m2/repository").to_string_lossy().to_string();
    let cocoapods_path = home
        .join("Library/Caches/CocoaPods")
        .to_string_lossy()
        .to_string();
    let spm_path = home
        .join("Library/Caches/org.swift.swiftpm")
        .to_string_lossy()
        .to_string();

    let candidates: Vec<(&str, &str, String)> = vec![
        ("npm", "npm — Cache", npm_path),
        ("yarn", "Yarn — Cache", yarn_path),
        ("pnpm", "pnpm — Store", pnpm_path),
        ("bun", "Bun — Cache", bun_path),
        ("brew", "Homebrew — Cache", brew_path),
        ("pip", "pip — Cache", pip_path),
        ("cargo", "Cargo — Registry", cargo_path),
        ("gradle", "Gradle — Cache", gradle_path),
        ("maven", "Maven — Repository", maven_path),
        ("cocoapods", "CocoaPods — Cache", cocoapods_path),
        ("spm", "Swift PM — Cache", spm_path),
    ];

    candidates
        .into_iter()
        .filter_map(|(id, label, path)| {
            let p = Path::new(&path);
            if !p.exists() {
                return None;
            }
            let size_bytes = du_bytes(p);
            if size_bytes == 0 {
                return None;
            }
            Some(DevCacheItem {
                id: id.to_string(),
                label: label.to_string(),
                path,
                size_bytes,
            })
        })
        .collect()
}

#[tauri::command]
fn clean_dev_caches(ids: Vec<String>) -> Result<u64, String> {
    let all = scan_dev_caches();
    let mut freed = 0u64;
    for item in all.iter().filter(|i| ids.contains(&i.id)) {
        let p = Path::new(&item.path);
        freed += du_bytes(p);
        if p.is_dir() {
            let _ = fs::remove_dir_all(p);
        } else {
            let _ = fs::remove_file(p);
        }
    }
    Ok(freed)
}

// ── Find residual files left by an app ───────────────────────────────────────

#[tauri::command]
fn find_app_residuals(app_name: String) -> Vec<FileEntry> {
    let home = home_dir();
    let name_lower = app_name.to_lowercase();

    // Build match patterns: name variants + bundle ID components
    let mut patterns: Vec<String> = Vec::new();
    patterns.push(name_lower.clone());
    let normalized: String = name_lower.chars().filter(|c| c.is_alphanumeric()).collect();
    if !normalized.is_empty() && normalized != name_lower {
        patterns.push(normalized.clone());
    }
    let version_stripped: String = name_lower
        .trim_end_matches(|c: char| c.is_numeric() || c == ' ' || c == '.')
        .to_string();
    if !version_stripped.is_empty() && version_stripped != name_lower {
        patterns.push(version_stripped.clone());
    }

    // Bundle ID from /Applications/<name>.app or ~/Applications/<name>.app
    let app_path = [
        PathBuf::from(format!("/Applications/{}.app", app_name)),
        home.join(format!("Applications/{}.app", app_name)),
    ]
    .iter()
    .find(|p| p.exists())
    .cloned()
    .unwrap_or_else(|| PathBuf::from(format!("/Applications/{}.app", app_name)));
    let bundle_id = app_bundle_id(&app_path);
    if !bundle_id.is_empty() {
        patterns.push(bundle_id.to_lowercase());
        let parts: Vec<&str> = bundle_id.split('.').collect();
        if parts.len() >= 3 {
            // "com.microsoft.VSCode" → "microsoft"
            patterns.push(parts[1].to_lowercase());
            // Last 2 components: "company.app"
            patterns.push(parts[parts.len() - 2..].join(".").to_lowercase());
        }
        if let Some(last) = parts.last() {
            let l = last.to_lowercase();
            if l.len() > 3 {
                patterns.push(l);
            }
        }
    }
    patterns.dedup();
    // Drop tokens that are too short or too generic to match safely
    patterns.retain(|p| {
        p.len() >= 4 && p != "app" && p != "data" && p != "com" && p != "net" && p != "org"
    });

    let search_dirs: &[(&str, bool)] = &[
        ("Library/Application Support", true),
        ("Library/Caches", true),
        ("Library/Preferences", false),
        ("Library/Logs", true),
        ("Library/Containers", true),
        ("Library/Group Containers", true),
        ("Library/Saved Application State", true),
        ("Library/WebKit", true),
        ("Library/HTTPStorages", true),
        ("Library/Cookies", false),
    ];

    let mut results: Vec<FileEntry> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (rel, is_dir_search) in search_dirs {
        let dir = home.join(rel);
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            // Skip iCloud paths
            if is_icloud_path(&p) {
                continue;
            }
            // Skip SIP-protected paths
            if is_sip_protected(&p) {
                continue;
            }

            let path_str = p.to_string_lossy().into_owned();
            if seen.contains(&path_str) {
                continue;
            }

            let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let fname_lower = fname.to_lowercase();

            // For UUID-named containers, look up the real bundle ID
            let resolved_name =
                if *rel == "Library/Containers" || *rel == "Library/Group Containers" {
                    container_bundle_id(&p)
                        .unwrap_or_else(|| fname.to_string())
                        .to_lowercase()
                } else {
                    fname_lower.clone()
                };

            // Per-app condition overrides
            let bid = bundle_id.to_lowercase();
            let include = match app_condition_matches(&resolved_name, &bid, &name_lower) {
                Some(v) => v,
                None => patterns
                    .iter()
                    .any(|pat| resolved_name.contains(pat.as_str())),
            };

            if include && !resolved_name.starts_with("com.apple.") {
                seen.insert(path_str.clone());
                let size_bytes = if p.is_dir() {
                    du_bytes(&p)
                } else {
                    fs::metadata(&p).map(|m| m.len()).unwrap_or(0)
                };
                results.push(FileEntry {
                    name: fname.to_string(),
                    path: path_str,
                    size_bytes,
                    is_dir: *is_dir_search && p.is_dir(),
                });
            }
        }
    }

    results.sort_by_key(|k| std::cmp::Reverse(k.size_bytes));
    results
}

#[tauri::command]
fn delete_path(path: String) -> Result<(), String> {
    let p = guard::validate_delete_path(&path)?;
    if p.is_dir() {
        fs::remove_dir_all(&p).map_err(|e| e.to_string())
    } else {
        fs::remove_file(&p).map_err(|e| e.to_string())
    }
}

// ── 10. All processes (sysinfo-based, grouped by name) ────────────────────────

#[derive(Serialize, Clone)]
pub struct ProcessEntry {
    pub pids: Vec<u32>,
    pub name: String,
    pub cpu_usage: f64, // normalized by core count
    pub memory_bytes: u64,
    pub disk_read_bytes: u64,
    pub disk_written_bytes: u64,
}

fn collect_processes_from_sys(sys: &sysinfo::System) -> Vec<ProcessEntry> {
    let core_count = sys.cpus().len().max(1) as f64;
    let mut by_name: std::collections::HashMap<String, ProcessEntry> =
        std::collections::HashMap::new();
    for (pid, proc) in sys.processes() {
        let name = proc.name().to_string_lossy().to_string();
        if name.is_empty() {
            continue;
        }
        let entry = by_name.entry(name.clone()).or_insert(ProcessEntry {
            pids: vec![],
            name: name.clone(),
            cpu_usage: 0.0,
            memory_bytes: 0,
            disk_read_bytes: 0,
            disk_written_bytes: 0,
        });
        entry.pids.push(pid.as_u32());
        entry.cpu_usage += proc.cpu_usage() as f64;
        entry.memory_bytes += proc.memory();
        entry.disk_read_bytes += proc.disk_usage().read_bytes;
        entry.disk_written_bytes += proc.disk_usage().written_bytes;
    }
    let mut procs: Vec<ProcessEntry> = by_name
        .into_values()
        .map(|mut p| {
            p.cpu_usage /= core_count;
            p
        })
        .collect();
    procs.sort_by(|a, b| {
        b.cpu_usage
            .partial_cmp(&a.cpu_usage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    procs
}

#[tauri::command]
fn get_all_processes() -> Vec<ProcessEntry> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_cpu_usage();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        ProcessRefreshKind::new()
            .with_cpu()
            .with_memory()
            .with_disk_usage(),
    );
    std::thread::sleep(Duration::from_millis(200));
    sys.refresh_cpu_usage();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        ProcessRefreshKind::new()
            .with_cpu()
            .with_memory()
            .with_disk_usage(),
    );
    collect_processes_from_sys(&sys)
}

// ── DNS management ────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct NetworkService {
    pub name: String,
    pub dns_servers: Vec<String>,
    pub active: bool,
}

/// Returns the BSD interface name (e.g. "en0") used for the default route.
fn default_route_iface() -> Option<String> {
    let out = Command::new("route")
        .args(["-n", "get", "default"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find(|l| l.trim().starts_with("interface:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .map(|s| s.to_string())
}

/// Maps a BSD interface name to a networksetup service name (e.g. "en0" → "Wi-Fi").
fn iface_to_service(iface: &str) -> Option<String> {
    let out = Command::new("networksetup")
        .arg("-listnetworkserviceorder")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        // Lines look like: "(Hardware Port: Wi-Fi, Device: en0)"
        if line.contains(&format!("Device: {iface})"))
            || line.contains(&format!("Device: {iface},"))
        {
            // Previous line is the service name: "(1) Wi-Fi"
            if let Some(prev) = lines.get(i.wrapping_sub(1)) {
                return prev.split_once(") ").map(|x| x.1.trim().to_string());
            }
        }
    }
    None
}

#[tauri::command]
fn list_network_services() -> Vec<NetworkService> {
    let active_iface = default_route_iface();
    let active_svc_name = active_iface.as_deref().and_then(iface_to_service);

    let out = match Command::new("networksetup")
        .arg("-listallnetworkservices")
        .output()
    {
        Ok(o) => o,
        Err(_) => return vec![],
    };
    let services: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1)
        .filter(|s| !s.is_empty() && !s.starts_with('*'))
        .map(|s| s.trim().to_string())
        .collect();

    services
        .into_iter()
        .filter_map(|svc| {
            let dns_out = Command::new("networksetup")
                .args(["-getdnsservers", &svc])
                .output()
                .ok()?;
            let text = String::from_utf8_lossy(&dns_out.stdout);
            let dns_servers: Vec<String> = if text.contains("aren't any") {
                vec![]
            } else {
                text.lines()
                    .filter(|s| !s.is_empty())
                    .map(|s| s.trim().to_string())
                    .collect()
            };
            let active = active_svc_name.as_deref() == Some(svc.as_str());
            Some(NetworkService {
                name: svc,
                dns_servers,
                active,
            })
        })
        .collect()
}

fn uuid_from_seed(seed: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    seed.hash(&mut h);
    let a = h.finish();
    "seed".to_string().hash(&mut h);
    let b = h.finish();
    format!(
        "{:08X}-{:04X}-4{:03X}-{:04X}-{:012X}",
        a as u32,
        (a >> 32) as u16,
        (a >> 48) & 0xFFF,
        0x8000u16 | ((b & 0x3FFF) as u16),
        b & 0x0000_FFFF_FFFF_FFFF,
    )
}

fn generate_doh_mobileconfig(
    provider_id: &str,
    option_id: &str,
    display_name: &str,
    doh_url: &str,
    servers: &[String],
) -> String {
    let root_uuid = uuid_from_seed(&format!("{}_{}r", provider_id, option_id));
    let payload_uuid = uuid_from_seed(&format!("{}_{}p", provider_id, option_id));
    let root_id = format!("net.burrow.dns.{}.{}", provider_id, option_id);
    let payload_id = format!("com.apple.dnsSettings.managed.{}", payload_uuid);
    let addrs: String = servers
        .iter()
        .map(|s| format!("\t\t\t\t\t<string>{}</string>", s))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>PayloadContent</key>
	<array>
		<dict>
			<key>DNSSettings</key>
			<dict>
				<key>DNSProtocol</key>
				<string>HTTPS</string>
				<key>ServerAddresses</key>
				<array>
{}
				</array>
				<key>ServerURL</key>
				<string>{}</string>
			</dict>
			<key>PayloadDescription</key>
			<string>Configures DNS over HTTPS</string>
			<key>PayloadDisplayName</key>
			<string>{}</string>
			<key>PayloadIdentifier</key>
			<string>{}</string>
			<key>PayloadType</key>
			<string>com.apple.dnsSettings.managed</string>
			<key>PayloadUUID</key>
			<string>{}</string>
			<key>PayloadVersion</key>
			<integer>1</integer>
		</dict>
	</array>
	<key>PayloadDescription</key>
	<string>DNS over HTTPS — {}</string>
	<key>PayloadDisplayName</key>
	<string>{}</string>
	<key>PayloadIdentifier</key>
	<string>{}</string>
	<key>PayloadRemovalDisallowed</key>
	<false/>
	<key>PayloadScope</key>
	<string>System</string>
	<key>PayloadType</key>
	<string>Configuration</string>
	<key>PayloadUUID</key>
	<string>{}</string>
	<key>PayloadVersion</key>
	<integer>1</integer>
</dict>
</plist>"#,
        addrs,
        doh_url,
        display_name,
        payload_id,
        payload_uuid,
        display_name,
        display_name,
        root_id,
        root_uuid,
    )
}

/// Entrée immuable du catalogue DoH — toutes les valeurs sont définies côté backend.
struct DohEntry {
    display_name: &'static str,
    doh_url: &'static str,
    /// IPs statiques (déjà validées — pas de résolution DNS à l'exécution pour ce catalogue)
    servers: &'static [&'static str],
    /// URL Mullvad à télécharger (None → on génère le plist localement)
    mullvad_url: Option<&'static str>,
}

fn doh_catalog() -> std::collections::HashMap<(&'static str, &'static str), DohEntry> {
    use std::collections::HashMap;
    let mut m: HashMap<(&str, &str), DohEntry> = HashMap::new();

    // Mullvad — profils officiels signés, commit épinglé (5a06b0cd)
    m.insert(("mullvad", "std"),      DohEntry { display_name: "Mullvad — Standard",  doh_url: "https://dns.mullvad.net/dns-query",          servers: &["194.242.2.2","193.19.108.2","2a07:e340::2","2001:67c:2208::2"], mullvad_url: Some("https://raw.githubusercontent.com/mullvad/encrypted-dns-profiles/5a06b0cd/vanilla/mullvad-encrypted-dns-https-vanilla.mobileconfig") });
    m.insert(("mullvad", "adblock"),  DohEntry { display_name: "Mullvad — Adblock",   doh_url: "https://adblock.dns.mullvad.net/dns-query",  servers: &["194.242.2.3","193.19.108.3","2a07:e340::3","2001:67c:2208::3"], mullvad_url: Some("https://raw.githubusercontent.com/mullvad/encrypted-dns-profiles/5a06b0cd/adblock/mullvad-encrypted-dns-https-adblock.mobileconfig") });
    m.insert(("mullvad", "base"),     DohEntry { display_name: "Mullvad — Base",       doh_url: "https://base.dns.mullvad.net/dns-query",     servers: &["194.242.2.9","2a07:e340::9"],                                   mullvad_url: Some("https://raw.githubusercontent.com/mullvad/encrypted-dns-profiles/5a06b0cd/base/mullvad-encrypted-dns-https-base.mobileconfig") });
    m.insert(("mullvad", "extended"), DohEntry { display_name: "Mullvad — Extended",   doh_url: "https://extended.dns.mullvad.net/dns-query", servers: &["194.242.2.5","2a07:e340::5"],                                   mullvad_url: Some("https://raw.githubusercontent.com/mullvad/encrypted-dns-profiles/5a06b0cd/extended/mullvad-encrypted-dns-https-extended.mobileconfig") });
    m.insert(("mullvad", "family"),   DohEntry { display_name: "Mullvad — Family",     doh_url: "https://family.dns.mullvad.net/dns-query",   servers: &["194.242.2.6","2a07:e340::6"],                                   mullvad_url: Some("https://raw.githubusercontent.com/mullvad/encrypted-dns-profiles/5a06b0cd/family/mullvad-encrypted-dns-https-family.mobileconfig") });
    m.insert(("mullvad", "all"),      DohEntry { display_name: "Mullvad — All",        doh_url: "https://all.dns.mullvad.net/dns-query",      servers: &["194.242.2.7","2a07:e340::7"],                                   mullvad_url: Some("https://raw.githubusercontent.com/mullvad/encrypted-dns-profiles/5a06b0cd/all/mullvad-encrypted-dns-https-all.mobileconfig") });

    // Quad9
    m.insert(
        ("quad9", "secure"),
        DohEntry {
            display_name: "Quad9 — Secure",
            doh_url: "https://dns.quad9.net/dns-query",
            servers: &["9.9.9.9", "149.112.112.112", "2620:fe::fe", "2620:fe::9"],
            mullvad_url: None,
        },
    );
    m.insert(
        ("quad9", "unsecure"),
        DohEntry {
            display_name: "Quad9 — Unsecure",
            doh_url: "https://dns10.quad9.net/dns-query",
            servers: &["9.9.9.10", "149.112.112.10"],
            mullvad_url: None,
        },
    );
    m.insert(
        ("quad9", "edns"),
        DohEntry {
            display_name: "Quad9 — EDNS",
            doh_url: "https://dns11.quad9.net/dns-query",
            servers: &["9.9.9.11", "149.112.112.11"],
            mullvad_url: None,
        },
    );

    // LibreDNS
    m.insert(
        ("libredns", "std"),
        DohEntry {
            display_name: "LibreDNS — Standard",
            doh_url: "https://doh.libredns.gr/dns-query",
            servers: &["116.202.176.26"],
            mullvad_url: None,
        },
    );

    // AdGuard
    m.insert(
        ("adguard", "std"),
        DohEntry {
            display_name: "AdGuard — Standard",
            doh_url: "https://dns.adguard-dns.com/dns-query",
            servers: &[
                "94.140.14.14",
                "94.140.15.15",
                "2a10:50c0::1:ff",
                "2a10:50c0::2:ff",
            ],
            mullvad_url: None,
        },
    );
    m.insert(
        ("adguard", "family"),
        DohEntry {
            display_name: "AdGuard — Family",
            doh_url: "https://family.adguard-dns.com/dns-query",
            servers: &[
                "94.140.14.15",
                "94.140.15.16",
                "2a10:50c0::bad1:ff",
                "2a10:50c0::bad2:ff",
            ],
            mullvad_url: None,
        },
    );
    m.insert(
        ("adguard", "unfiltered"),
        DohEntry {
            display_name: "AdGuard — Unfiltered",
            doh_url: "https://unfiltered.adguard-dns.com/dns-query",
            servers: &[
                "94.140.14.140",
                "94.140.14.141",
                "2a10:50c0::1:ff",
                "2a10:50c0::2:ff",
            ],
            mullvad_url: None,
        },
    );

    // Cloudflare
    m.insert(
        ("cloudflare", "std"),
        DohEntry {
            display_name: "Cloudflare — Standard",
            doh_url: "https://cloudflare-dns.com/dns-query",
            servers: &[
                "1.1.1.1",
                "1.0.0.1",
                "2606:4700:4700::1111",
                "2606:4700:4700::1001",
            ],
            mullvad_url: None,
        },
    );
    m.insert(
        ("cloudflare", "security"),
        DohEntry {
            display_name: "Cloudflare — Security",
            doh_url: "https://security.cloudflare-dns.com/dns-query",
            servers: &[
                "1.1.1.2",
                "1.0.0.2",
                "2606:4700:4700::1112",
                "2606:4700:4700::1002",
            ],
            mullvad_url: None,
        },
    );
    m.insert(
        ("cloudflare", "family"),
        DohEntry {
            display_name: "Cloudflare — Family",
            doh_url: "https://family.cloudflare-dns.com/dns-query",
            servers: &[
                "1.1.1.3",
                "1.0.0.3",
                "2606:4700:4700::1113",
                "2606:4700:4700::1003",
            ],
            mullvad_url: None,
        },
    );

    m
}

/// Installe un profil DoH à partir du catalogue backend.
/// Le frontend n'envoie que (`provider_id`, `option_id`) — aucun chemin, URL ni serveur libre.
#[tauri::command]
fn install_doh_profile(provider_id: String, option_id: String) -> Result<(), String> {
    use std::io::Write;

    let catalog = doh_catalog();
    let entry = catalog
        .get(&(provider_id.as_str(), option_id.as_str()))
        .ok_or_else(|| format!("Fournisseur DoH inconnu : {}/{}", provider_id, option_id))?;

    // Fichier temporaire aléatoire — aucun nom frontend dans le chemin
    let tmp_file = burrow_tempfile(".mobileconfig").map_err(|e| e.to_string())?;
    let tmp_path = tmp_file.path().to_path_buf();
    // keep tmp_file alive until after `open` so the file isn't deleted prematurely
    let _guard = tmp_file;

    if let Some(url) = entry.mullvad_url {
        // Téléchargement depuis une URL statique du catalogue (commit épinglé)
        let out = Command::new("curl")
            .args([
                "-sSL",
                "--fail",
                "--max-time",
                "30",
                "-o",
                &tmp_path.to_string_lossy(),
                url,
            ])
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!(
                "Téléchargement échoué: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    } else {
        let servers: Vec<String> = entry.servers.iter().map(|s| s.to_string()).collect();
        let xml = generate_doh_mobileconfig(
            &provider_id,
            &option_id,
            entry.display_name,
            entry.doh_url,
            &servers,
        );
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .open(&tmp_path)
            .map_err(|e| e.to_string())?;
        f.write_all(xml.as_bytes()).map_err(|e| e.to_string())?;
    }

    Command::new("open")
        .arg(&tmp_path)
        .output()
        .map_err(|e| e.to_string())?;
    // _guard dropped here → temp file deleted after macOS has opened it
    Ok(())
}

fn resolve_to_ip(entry: &str) -> String {
    if entry.parse::<std::net::IpAddr>().is_ok() {
        return entry.to_string();
    }
    use std::net::ToSocketAddrs;
    if let Ok(mut addrs) = format!("{}:53", entry).to_socket_addrs() {
        if let Some(addr) = addrs.find(|a| a.ip().is_ipv4()) {
            return addr.ip().to_string();
        }
    }
    entry.to_string()
}

#[tauri::command]
fn set_dns_servers(service: String, servers: Vec<String>) -> Result<(), String> {
    guard::validate_service_name(&service)?;
    if servers.is_empty() {
        return Err("Aucun serveur DNS fourni".to_string());
    }
    // Valider chaque IP (les noms de domaine sont résolus côté backend)
    let resolved: Vec<String> = servers.iter().map(|s| resolve_to_ip(s)).collect();
    for ip in &resolved {
        guard::validate_ip_address(ip).map_err(|e| format!("Serveur DNS invalide : {}", e))?;
    }
    // Utiliser Command::arg() — aucune interpolation shell
    let mut cmd = Command::new("networksetup");
    cmd.arg("-setdnsservers").arg(&service);
    for ip in &resolved {
        cmd.arg(ip);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        // Certaines opérations DNS nécessitent des privilèges admin
        if err.contains("Error") || err.contains("permission") || !err.trim().is_empty() {
            // Fallback admin via osascript avec arguments typés (pas d'interpolation)
            let script = format!(
                "do shell script {} with administrator privileges",
                posix_applescript_string(&format!(
                    "networksetup -setdnsservers {} {}",
                    posix_shell_quote_value(&service),
                    resolved
                        .iter()
                        .map(|s| posix_shell_quote_value(s))
                        .collect::<Vec<_>>()
                        .join(" ")
                ))
            );
            let adm = Command::new("osascript")
                .args(["-e", &script])
                .output()
                .map_err(|e| e.to_string())?;
            if !adm.status.success() {
                return Err(String::from_utf8_lossy(&adm.stderr).trim().to_string());
            }
        }
    }
    let _ = Command::new("dscacheutil").arg("-flushcache").output();
    let _ = Command::new("killall")
        .args(["-HUP", "mDNSResponder"])
        .output();
    Ok(())
}

#[tauri::command]
fn reset_dns(service: String) -> Result<(), String> {
    guard::validate_service_name(&service)?;
    let out = Command::new("networksetup")
        .args(["-setdnsservers", &service, "Empty"])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let script = format!(
            "do shell script {} with administrator privileges",
            posix_applescript_string(&format!(
                "networksetup -setdnsservers {} Empty",
                posix_shell_quote_value(&service)
            ))
        );
        let adm = Command::new("osascript")
            .args(["-e", &script])
            .output()
            .map_err(|e| e.to_string())?;
        if !adm.status.success() {
            return Err(String::from_utf8_lossy(&adm.stderr).trim().to_string());
        }
    }
    let _ = Command::new("dscacheutil").arg("-flushcache").output();
    let _ = Command::new("killall")
        .args(["-HUP", "mDNSResponder"])
        .output();
    Ok(())
}

#[tauri::command]
fn get_search_domains(service: String) -> Vec<String> {
    let out = Command::new("networksetup")
        .args(["-getsearchdomains", &service])
        .output()
        .ok();
    let raw = out
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if raw.is_empty()
        || raw.to_lowercase().contains("there aren")
        || raw.to_lowercase().contains("empty")
    {
        return vec![];
    }
    raw.lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[tauri::command]
fn set_search_domains(service: String, domains: Vec<String>) -> Result<(), String> {
    guard::validate_service_name(&service)?;
    for d in &domains {
        guard::validate_domain_name(d)
            .map_err(|e| format!("Domaine de recherche invalide : {}", e))?;
    }
    let mut cmd = Command::new("networksetup");
    cmd.arg("-setsearchdomains").arg(&service);
    if domains.is_empty() {
        cmd.arg("Empty");
    } else {
        for d in &domains {
            cmd.arg(d);
        }
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let domain_args = if domains.is_empty() {
        "Empty".to_string()
    } else {
        domains
            .iter()
            .map(|d| posix_shell_quote_value(d))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let script = format!(
        "do shell script {} with administrator privileges",
        posix_applescript_string(&format!(
            "networksetup -setsearchdomains {} {}",
            posix_shell_quote_value(&service),
            domain_args
        ))
    );
    let adm = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| e.to_string())?;
    if adm.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&adm.stderr).trim().to_string())
    }
}

fn start_process_daemon(app_handle: tauri::AppHandle) {
    std::thread::spawn(move || {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            ProcessRefreshKind::new()
                .with_cpu()
                .with_memory()
                .with_disk_usage(),
        );
        loop {
            std::thread::sleep(Duration::from_secs(1));
            sys.refresh_cpu_usage();
            sys.refresh_processes_specifics(
                ProcessesToUpdate::All,
                ProcessRefreshKind::new()
                    .with_cpu()
                    .with_memory()
                    .with_disk_usage(),
            );
            let procs = collect_processes_from_sys(&sys);
            let _ = app_handle.emit("emit_processes", procs);
        }
    });
}

#[tauri::command]
fn kill_process(pid: u64) -> Result<(), String> {
    guard::validate_kill_pid(pid)?;

    // Vérifier que le processus appartient à l'utilisateur courant (pas de kill admin arbitraire)
    let current_uid = unsafe { libc::getuid() };
    let owner_uid = get_process_uid(pid);
    match owner_uid {
        Some(uid) if uid != current_uid => {
            return Err(format!(
                "Refus de tuer le processus {} appartenant à l'utilisateur uid={}",
                pid, uid
            ));
        }
        None => return Err(format!("Processus {} introuvable", pid)),
        _ => {}
    }

    // SIGTERM d'abord (terminaison propre)
    let _ = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .output();

    // Attendre brièvement et vérifier si le processus s'est terminé
    std::thread::sleep(std::time::Duration::from_millis(400));
    if get_process_uid(pid).is_none() {
        return Ok(());
    }

    // SIGKILL seulement si le processus résiste
    let out = Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .output()
        .map_err(|e| e.to_string())?;

    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Impossible de terminer le processus {} : {}",
            pid,
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Retourne l'UID Unix du propriétaire d'un PID via `ps`.
/// Renvoie `None` si le processus n'existe plus.
fn get_process_uid(pid: u64) -> Option<u32> {
    Command::new("ps")
        .args(["-o", "uid=", "-p", &pid.to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u32>().ok())
}

// ── App discovery (all .app bundles on the system) ───────────────────────────

fn collect_all_apps() -> Vec<std::path::PathBuf> {
    let home = home_dir();
    let search_roots = vec![
        std::path::PathBuf::from("/Applications"),
        std::path::PathBuf::from("/System/Applications"),
        home.join("Applications"),
        std::path::PathBuf::from("/System/Library/CoreServices"),
    ];

    let is_app = |p: &std::path::Path| p.extension().and_then(|x| x.to_str()) == Some("app");

    let mut apps = Vec::new();
    for root in &search_roots {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if is_app(&p) {
                apps.push(p);
            } else if p.is_dir() {
                // Un niveau de sous-dossier (ex : /Applications/Utilities/)
                let Ok(sub_entries) = fs::read_dir(&p) else {
                    continue;
                };
                for sub in sub_entries.flatten() {
                    let sp = sub.path();
                    if is_app(&sp) {
                        apps.push(sp);
                    }
                }
            }
        }
    }
    apps
}

// ── Sparkle update checking ───────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct SparkleUpdate {
    pub name: String,
    pub path: String,
    pub current_version: String,
    pub latest_version: String,
    pub download_url: String,
    pub release_notes: String,
}

#[derive(Serialize)]
pub struct SparkleResult {
    pub updates: Vec<SparkleUpdate>,
    pub up_to_date: Vec<UpToDateApp>,
    pub checked: usize,
}

#[derive(Serialize, Clone)]
pub struct AppStoreUpdate {
    pub name: String,
    pub bundle_id: String,
    pub installed_version: String,
    pub latest_version: String,
    pub release_notes: String,
    pub store_url: String,
    pub track_id: u64,
    pub from_store: bool, // true = installé via App Store (mas peut le mettre à jour)
}

fn plist_str(plist_path: &Path, key: &str) -> Option<String> {
    Command::new("/usr/libexec/PlistBuddy")
        .args([
            "-c",
            &format!("Print :{}", key),
            &plist_path.to_string_lossy(),
        ])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn version_gt(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split(|c: char| !c.is_ascii_digit())
            .filter(|p| !p.is_empty())
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let va = parse(a);
    let vb = parse(b);
    for i in 0..va.len().max(vb.len()) {
        let x = va.get(i).copied().unwrap_or(0);
        let y = vb.get(i).copied().unwrap_or(0);
        if x > y {
            return true;
        }
        if x < y {
            return false;
        }
    }
    false
}

fn xml_attr(tag: &str, attr: &str) -> Option<String> {
    for q in ['"', '\''] {
        let needle = format!("{}={}", attr, q);
        if let Some(pos) = tag.find(&needle) {
            let from = pos + needle.len();
            if let Some(end) = tag[from..].find(q) {
                let val = tag[from..from + end].trim().to_string();
                if !val.is_empty() {
                    return Some(val);
                }
            }
        }
    }
    None
}

fn xml_element(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)?;
    let val = xml[start..start + end].trim().to_string();
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
}

struct FeedResult {
    short_version: String, // sparkle:shortVersionString (version affichée, ex "1.2.3")
    build_version: String, // sparkle:version (numéro de build, ex "12345")
    url: String,
    notes: String,
}

fn parse_feed(xml: &str) -> Option<FeedResult> {
    let pos = xml.find("<enclosure")?;
    let end = xml[pos..].find('>')? + pos;
    let tag = &xml[pos..=end];

    let url = xml_attr(tag, "url")?;
    if !url.starts_with("http") {
        return None;
    }

    let short_version = xml_attr(tag, "sparkle:shortVersionString")
        .or_else(|| xml_element(xml, "sparkle:shortVersionString"))
        .unwrap_or_default();

    let build_version = xml_attr(tag, "sparkle:version")
        .or_else(|| xml_element(xml, "sparkle:version"))
        .unwrap_or_default();

    // Il faut au moins l'un des deux
    if short_version.is_empty() && build_version.is_empty() {
        return None;
    }

    let notes = xml_element(xml, "description")
        .map(|d| strip_html(&d))
        .unwrap_or_default();

    Some(FeedResult {
        short_version,
        build_version,
        url,
        notes,
    })
}

fn strip_html(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    // Condenser les lignes vides
    let lines: Vec<&str> = out
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    lines.join("\n")
}

fn app_current_version(plist: &Path) -> Option<String> {
    plist_str(plist, "CFBundleShortVersionString").or_else(|| plist_str(plist, "CFBundleVersion"))
}

fn sparkle_feed_update(
    path: &std::path::Path,
    name: &str,
    feed_url: &str,
) -> Option<SparkleUpdate> {
    let plist = path.join("Contents/Info.plist");
    let installed_short = plist_str(&plist, "CFBundleShortVersionString").unwrap_or_default();
    let installed_build = plist_str(&plist, "CFBundleVersion").unwrap_or_default();

    if installed_short.is_empty() && installed_build.is_empty() {
        return None;
    }

    let out = Command::new("curl")
        .args([
            "-s",
            "-L",
            "--max-time",
            "8",
            "-A",
            "Mozilla/5.0 Sparkle/2.0",
            feed_url,
        ])
        .output()
        .ok()?;
    if out.stdout.is_empty() {
        return None;
    }

    let xml = String::from_utf8_lossy(&out.stdout);
    let feed = parse_feed(&xml)?;

    // Choisir le couple (version_installée, version_feed) le plus pertinent
    // Priorité : shortVersionString si disponibles des deux côtés, sinon build number
    let (current, latest) = if !feed.short_version.is_empty() && !installed_short.is_empty() {
        (installed_short.clone(), feed.short_version.clone())
    } else if !feed.build_version.is_empty() && !installed_build.is_empty() {
        (installed_build.clone(), feed.build_version.clone())
    } else if !feed.short_version.is_empty() {
        (installed_build.clone(), feed.short_version.clone())
    } else {
        (installed_short.clone(), feed.build_version.clone())
    };

    if !version_gt(&latest, &current) {
        return None;
    }

    // Version affichée : de préférence shortVersionString
    let display_current = if !installed_short.is_empty() {
        installed_short
    } else {
        installed_build
    };
    let display_latest = if !feed.short_version.is_empty() {
        feed.short_version
    } else {
        feed.build_version
    };

    Some(SparkleUpdate {
        name: name.to_string(),
        path: path.to_string_lossy().to_string(),
        current_version: display_current,
        latest_version: display_latest,
        download_url: feed.url,
        release_notes: feed.notes,
    })
}

fn check_one_app(path: std::path::PathBuf) -> Option<SparkleUpdate> {
    let name = path.file_stem()?.to_str()?.to_string();
    let plist = path.join("Contents/Info.plist");
    if !plist.exists() {
        return None;
    }

    // Sparkle via SUFeedURL
    if let Some(feed_url) = plist_str(&plist, "SUFeedURL").filter(|u| u.starts_with("http")) {
        return sparkle_feed_update(&path, &name, &feed_url);
    }

    // DevMate : présence de DevMateKit.framework → feed automatique
    let devmate = path.join("Contents/Frameworks/DevMateKit.framework");
    if devmate.exists() {
        if let Some(bundle_id) = plist_str(&plist, "CFBundleIdentifier") {
            let feed_url = format!("https://updates.devmate.com/{}.xml", bundle_id);
            return sparkle_feed_update(&path, &name, &feed_url);
        }
    }

    None
}

// ── Electron / electron-updater (app-update.yml) ──────────────────────────────

fn yml_value(yml: &str, key: &str) -> Option<String> {
    yml.lines()
        .find(|l| l.trim_start().starts_with(&format!("{}:", key)))
        .and_then(|l| l.split_once(':').map(|x| x.1))
        .map(|v| v.trim().trim_matches('"').to_string())
        .filter(|v| !v.is_empty())
}

fn check_electron_update(path: std::path::PathBuf) -> Option<SparkleUpdate> {
    let name = path.file_stem()?.to_str()?.to_string();
    let plist = path.join("Contents/Info.plist");
    let update_yml = path.join("Contents/Resources/app-update.yml");
    if !update_yml.exists() {
        return None;
    }

    let current = app_current_version(&plist)?;
    let yml = fs::read_to_string(&update_yml).ok()?;
    let provider = yml_value(&yml, "provider")?;

    match provider.as_str() {
        "github" => {
            let owner = yml_value(&yml, "owner")?;
            let repo = yml_value(&yml, "repo")?;
            let api = format!(
                "https://api.github.com/repos/{}/{}/releases/latest",
                owner, repo
            );

            let out = Command::new("curl")
                .args([
                    "-s",
                    "-L",
                    "--max-time",
                    "8",
                    "-H",
                    "Accept: application/vnd.github.v3+json",
                    "-H",
                    "User-Agent: Burrow-Updater",
                    &api,
                ])
                .output()
                .ok()?;
            if out.stdout.is_empty() {
                return None;
            }

            let json: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
            let tag = json["tag_name"].as_str()?;
            let latest = tag.trim_start_matches('v').to_string();
            if !version_gt(&latest, &current) {
                return None;
            }

            // Architecture de la machine
            let arch = Command::new("uname")
                .arg("-m")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();
            let is_arm = arch.trim().contains("arm") || arch.trim().contains("aarch");

            let assets = json["assets"].as_array()?;
            let urls: Vec<&str> = assets
                .iter()
                .filter_map(|a| a["browser_download_url"].as_str())
                .collect();

            // Priorité : DMG natif (arm64/x64) > DMG universel > ZIP mac > ZIP générique
            let arch_label = if is_arm { "arm64" } else { "x64" };

            let download_url = urls
                .iter()
                .find(|u| {
                    let l = u.to_lowercase();
                    l.ends_with(".dmg") && l.contains(arch_label)
                })
                .or_else(|| urls.iter().find(|u| u.to_lowercase().ends_with(".dmg")))
                .or_else(|| {
                    urls.iter().find(|u| {
                        u.to_lowercase()
                            .ends_with(&format!("-{}-mac.zip", arch_label))
                    })
                })
                .or_else(|| urls.iter().find(|u| u.to_lowercase().ends_with("-mac.zip")))
                .or_else(|| {
                    urls.iter().find(|u| {
                        let l = u.to_lowercase();
                        l.contains("mac") && l.ends_with(".zip")
                    })
                })
                .map(|s| s.to_string())?;

            let release_notes = json["body"].as_str().unwrap_or("").trim().to_string();

            Some(SparkleUpdate {
                name,
                path: path.to_string_lossy().to_string(),
                current_version: current,
                latest_version: latest,
                download_url,
                release_notes,
            })
        }
        "generic" => {
            let base_url = yml_value(&yml, "url")?;
            let base_url = base_url.trim_end_matches('/').to_string();
            let manifest_url = format!("{}/latest-mac.yml", base_url);

            let out = Command::new("curl")
                .args([
                    "-s",
                    "-L",
                    "--max-time",
                    "8",
                    "-A",
                    "electron-updater",
                    &manifest_url,
                ])
                .output()
                .ok()?;
            if out.stdout.is_empty() {
                return None;
            }

            let manifest = String::from_utf8_lossy(&out.stdout);
            let latest = yml_value(&manifest, "version")?;
            if !version_gt(&latest, &current) {
                return None;
            }

            let arch = Command::new("uname")
                .arg("-m")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();
            let is_arm = arch.trim().contains("arm") || arch.trim().contains("aarch");
            let arch_label = if is_arm { "arm64" } else { "x64" };

            // Collecter les URLs de fichiers depuis le bloc files:
            let file_urls: Vec<String> = manifest
                .lines()
                .filter_map(|l| {
                    let t = l.trim();
                    if t.starts_with("- url:") {
                        let v = t
                            .trim_start_matches("- url:")
                            .trim()
                            .trim_matches('"')
                            .to_string();
                        if !v.is_empty() {
                            Some(v)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect();

            let pick = file_urls
                .iter()
                .find(|u| {
                    let l = u.to_lowercase();
                    l.ends_with(".dmg") && l.contains(arch_label)
                })
                .or_else(|| {
                    file_urls
                        .iter()
                        .find(|u| u.to_lowercase().ends_with(".dmg"))
                })
                .or_else(|| {
                    file_urls.iter().find(|u| {
                        let l = u.to_lowercase();
                        l.contains(arch_label) && l.ends_with(".zip")
                    })
                })
                .or_else(|| {
                    file_urls
                        .iter()
                        .find(|u| u.to_lowercase().ends_with(".zip"))
                })?;

            let download_url = if pick.starts_with("http") {
                pick.clone()
            } else {
                format!("{}/{}", base_url, pick)
            };

            let release_notes = yml_value(&manifest, "releaseNotes").unwrap_or_default();

            Some(SparkleUpdate {
                name,
                path: path.to_string_lossy().to_string(),
                current_version: current,
                latest_version: latest,
                download_url,
                release_notes,
            })
        }
        // Autres providers (s3, gitlab…) non pris en charge
        _ => None,
    }
}

#[tauri::command]
fn check_sparkle_updates() -> SparkleResult {
    let apps: Vec<_> = collect_all_apps();

    // Ne garder que les apps qui ont un mécanisme de mise à jour connu
    let checkable: Vec<_> = apps
        .into_iter()
        .filter(|p| {
            let plist = p.join("Contents/Info.plist");
            if !plist.exists() {
                return false;
            }
            plist_str(&plist, "SUFeedURL")
                .map(|u| u.starts_with("http"))
                .unwrap_or(false)
                || p.join("Contents/Resources/app-update.yml").exists()
                || p.join("Contents/Frameworks/DevMateKit.framework").exists()
        })
        .collect();

    let checked = checkable.len();

    // Chaque thread retourne (name, current_version, Option<update>)
    let handles: Vec<_> = checkable
        .into_iter()
        .map(|p| {
            std::thread::spawn(move || -> Option<(String, String, Option<SparkleUpdate>)> {
                let name = p.file_stem()?.to_str()?.to_string();
                let plist = p.join("Contents/Info.plist");
                let current = app_current_version(&plist).unwrap_or_default();
                let update = check_one_app(p.clone()).or_else(|| check_electron_update(p));
                Some((name, current, update))
            })
        })
        .collect();

    let mut updates: Vec<SparkleUpdate> = Vec::new();
    let mut up_to_date: Vec<UpToDateApp> = Vec::new();

    for h in handles {
        if let Ok(Some((name, current, update_opt))) = h.join() {
            match update_opt {
                Some(u) => updates.push(u),
                None => up_to_date.push(UpToDateApp {
                    name,
                    current_version: current,
                }),
            }
        }
    }

    updates.sort_by(|a, b| a.name.cmp(&b.name));
    up_to_date.sort_by(|a, b| a.name.cmp(&b.name));
    SparkleResult {
        updates,
        up_to_date,
        checked,
    }
}

// ── Mac App Store update checking ─────────────────────────────────────────────

fn check_app_store_app(path: std::path::PathBuf) -> Option<AppStoreUpdate> {
    let plist = path.join("Contents/Info.plist");
    if !plist.exists() {
        return None;
    }

    // Uniquement les apps installées via App Store
    if !path.join("Contents/_MASReceipt/receipt").exists() {
        return None;
    }

    let name = path.file_stem()?.to_str()?.to_string();
    let bundle_id = plist_str(&plist, "CFBundleIdentifier")?;
    let installed = plist_str(&plist, "CFBundleShortVersionString")?;

    if bundle_id.starts_with("com.apple.") {
        return None;
    }
    if name == "Burrow" {
        return None;
    }

    // Détecter le pays depuis les préférences système (comme Latest)
    let country = Command::new("defaults")
        .args(["read", "-g", "AppleLocale"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .and_then(|s| {
            // "fr_FR" → "fr", "en_US" → "us"
            s.split('_').nth(1).map(|c| c.to_lowercase())
        })
        .unwrap_or_else(|| "us".to_string());

    // Essayer d'abord entity=desktopSoftware (plus précis), puis macSoftware
    let results = ["desktopSoftware", "macSoftware"]
        .iter()
        .find_map(|entity| {
            let url = format!(
                "https://itunes.apple.com/lookup?bundleId={}&entity={}&country={}",
                bundle_id, entity, country
            );
            let out = Command::new("curl")
                .args(["-s", "-L", "--max-time", "8", "-A", "Mozilla/5.0", &url])
                .output()
                .ok()?;
            if out.stdout.is_empty() {
                return None;
            }
            let json: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
            let arr = json["results"].as_array()?.clone();
            if arr.is_empty() {
                None
            } else {
                Some(arr)
            }
        });

    let results = results?;

    let r = results.into_iter().next()?;
    let latest = r["version"].as_str()?.to_string();
    if !version_gt(&latest, &installed) {
        return None;
    }

    let release_notes = r["releaseNotes"].as_str().unwrap_or("").trim().to_string();
    let store_url = r["trackViewUrl"].as_str().unwrap_or("").to_string();
    let track_id = r["trackId"].as_u64().unwrap_or(0);
    let from_store = true;

    Some(AppStoreUpdate {
        name,
        bundle_id,
        installed_version: installed,
        latest_version: latest,
        release_notes,
        store_url,
        track_id,
        from_store,
    })
}

#[derive(Serialize)]
pub struct AppStoreResult {
    pub updates: Vec<AppStoreUpdate>,
    pub up_to_date: Vec<UpToDateApp>,
    pub checked: usize,
}

#[tauri::command]
fn check_app_store_updates() -> AppStoreResult {
    let apps: Vec<_> = collect_all_apps();

    // Apps avec receipt MAS (installées depuis l'App Store)
    let mas_apps: Vec<_> = apps
        .into_iter()
        .filter(|p| p.join("Contents/_MASReceipt/receipt").exists())
        .collect();
    let checked = mas_apps.len();

    // Chaque thread retourne (name, current_version, Option<AppStoreUpdate>)
    let handles: Vec<_> = mas_apps
        .into_iter()
        .map(|p| {
            std::thread::spawn(
                move || -> Option<(String, String, Option<AppStoreUpdate>)> {
                    let name = p.file_stem()?.to_str()?.to_string();
                    let plist = p.join("Contents/Info.plist");
                    let current =
                        plist_str(&plist, "CFBundleShortVersionString").unwrap_or_default();
                    let update = check_app_store_app(p);
                    Some((name, current, update))
                },
            )
        })
        .collect();

    let mut updates: Vec<AppStoreUpdate> = Vec::new();
    let mut up_to_date: Vec<UpToDateApp> = Vec::new();

    for h in handles {
        if let Ok(Some((name, current, update_opt))) = h.join() {
            match update_opt {
                Some(u) => updates.push(u),
                None => up_to_date.push(UpToDateApp {
                    name,
                    current_version: current,
                }),
            }
        }
    }

    updates.sort_by(|a, b| a.name.cmp(&b.name));
    up_to_date.sort_by(|a, b| a.name.cmp(&b.name));
    AppStoreResult {
        updates,
        up_to_date,
        checked,
    }
}

#[tauri::command]
fn update_mas_app(app: tauri::AppHandle, track_id: u64, _name: String) -> Result<(), String> {
    let mas = ["/opt/homebrew/bin/mas", "/usr/local/bin/mas"]
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .map(|s| s.to_string())
        .ok_or_else(|| "mas_not_installed".to_string())?;

    // Lancer dans un thread séparé pour ne pas bloquer Tauri
    std::thread::spawn(move || {
        macro_rules! out {
            ($m:expr) => {
                let _ = app.emit("mas-output", $m.to_string());
            };
        }

        out!("Lancement de la mise à jour App Store…");

        let mut child = match Command::new(&mas)
            .args(["upgrade", &track_id.to_string()])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                out!(format!("Erreur : {}", e));
                let _ = app.emit("mas-done", false);
                return;
            }
        };

        let app_out = app.clone();
        let stdout_thread = child.stdout.take().map(|stdout| {
            std::thread::spawn(move || {
                BufReader::new(stdout)
                    .lines()
                    .map_while(Result::ok)
                    .for_each(|l| {
                        let _ = app_out.emit("mas-output", &l);
                    });
            })
        });
        let app_err = app.clone();
        let stderr_thread = child.stderr.take().map(|stderr| {
            std::thread::spawn(move || {
                BufReader::new(stderr)
                    .lines()
                    .map_while(Result::ok)
                    .for_each(|l| {
                        let _ = app_err.emit("mas-output", &l);
                    });
            })
        });

        if let Some(h) = stdout_thread {
            let _ = h.join();
        }
        if let Some(h) = stderr_thread {
            let _ = h.join();
        }

        let ok = child.wait().map(|s| s.success()).unwrap_or(false);
        let _ = app.emit("mas-done", ok);
    });

    Ok(())
}

// ── Sparkle update installation ───────────────────────────────────────────────

fn find_app_bundle(dir: &str) -> Option<String> {
    let is_app = |p: &std::path::Path| p.extension().and_then(|x| x.to_str()) == Some("app");
    // Root level
    if let Some(p) = fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|e| is_app(&e.path()))
        .map(|e| e.path().to_string_lossy().to_string())
    {
        return Some(p);
    }
    // One level deeper (some DMGs nest inside a subfolder)
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let sub = entry.path();
        if sub.is_dir() && !is_app(&sub) {
            if let Some(p) = fs::read_dir(&sub)
                .ok()?
                .flatten()
                .find(|e| is_app(&e.path()))
                .map(|e| e.path().to_string_lossy().to_string())
            {
                return Some(p);
            }
        }
    }
    None
}

fn copy_app(src: &str, dest_dir: &str) -> Result<(), String> {
    let dest = std::path::Path::new(dest_dir)
        .join(std::path::Path::new(src).file_name().unwrap_or_default());
    let dest_s = dest.to_string_lossy();

    // Try without admin first
    let _ = fs::remove_dir_all(&dest);
    let out = Command::new("ditto")
        .args([src, dest_s.as_ref()])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }

    // Retry: rm -rf then ditto, with admin — quoting POSIX correct
    let q_dest = guard::posix_shell_quote(dest_s.as_ref());
    let q_src = guard::posix_shell_quote(src);
    run_admin_sh(&format!("rm -rf {} && ditto {} {}", q_dest, q_src, q_dest))
}

#[tauri::command]
fn install_sparkle_update(
    app: tauri::AppHandle,
    name: String,
    download_url: String,
    app_path: String,
) {
    std::thread::spawn(move || {
        macro_rules! out {
            ($m:expr) => {
                let _ = app.emit("sparkle-output", $m.to_string());
            };
        }
        macro_rules! done {
            ($ok:expr) => {
                let _ = app.emit("sparkle-done", $ok);
                return;
            };
        }

        // Valider les entrées avant toute opération
        if let Err(e) = guard::validate_update_url(&download_url) {
            out!(format!("✗ URL refusée : {}", e));
            done!(false);
        }
        if let Err(e) = guard::validate_update_app_path(&app_path) {
            out!(format!("✗ Chemin d'application refusé : {}", e));
            done!(false);
        }

        let work_dir = match burrow_tempdir() {
            Ok(d) => d,
            Err(e) => {
                out!(format!("✗ Erreur tmpdir : {}", e));
                done!(false);
            }
        };
        let tmp = work_dir
            .path()
            .join("download")
            .to_string_lossy()
            .into_owned();

        out!(format!("Téléchargement de {}…", name));

        // --fail : curl retourne une erreur si le serveur répond 4xx/5xx
        let ok_dl = Command::new("curl")
            .args([
                "-s",
                "-L",
                "--fail",
                "--max-time",
                "120",
                "-A",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X)",
                "-o",
                &tmp,
                &download_url,
            ])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !ok_dl {
            out!("✗ Échec du téléchargement (serveur inaccessible ou URL invalide)");
            done!(false);
        }

        // Vérification de taille — une app Mac fait au moins 500 Ko
        let file_size = fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
        out!(format!("Taille : {:.1} Mo", file_size as f64 / 1_048_576.0));
        if file_size < 500_000 {
            out!("✗ Fichier trop petit — probable page d'erreur ou redirect invalide");
            let _ = fs::remove_file(&tmp);
            done!(false);
        }

        // Détecter le format depuis le contenu réel du fichier (pas l'URL)
        let mime = Command::new("file")
            .args(["-b", "--mime-type", &tmp])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();
        let mime = mime.trim().to_string();
        // bzip2 doit être testé avant zip ("bzip2" contient "zip").
        // Les DMG UDBZ sont compressés en bzip2 et signalés comme application/x-bzip2 :
        // hdiutil imageinfo le détecte correctement.
        let ext = if mime.contains("bzip2") {
            let is_dmg = Command::new("hdiutil")
                .args(["imageinfo", &tmp])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if is_dmg {
                "dmg"
            } else {
                "tar.bz2"
            }
        } else if mime.contains("gzip") {
            "tar.gz"
        } else if mime.contains("x-xz") || mime.contains("xz") {
            "tar.xz"
        } else if mime.contains("zip") {
            "zip"
        } else if mime.contains("xar") || mime.contains("x-newton") {
            "pkg"
        } else {
            "dmg"
        };
        out!(format!("Format : {} ({})", ext, mime));

        // Renommer avec la bonne extension (hdiutil/unzip/tar en ont besoin)
        let tmp_ext = work_dir
            .path()
            .join(format!("download.{}", ext))
            .to_string_lossy()
            .into_owned();
        if let Err(e) = fs::rename(&tmp, &tmp_ext) {
            out!(format!("✗ Renommage échoué : {}", e));
            done!(false);
        }
        let tmp = tmp_ext;

        // Quit app if running
        if Command::new("pgrep")
            .args(["-x", &name])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            out!(format!("Fermeture de {}…", name));
            let _ = Command::new("osascript")
                .args(["-e", &format!("quit app \"{}\"", name)])
                .output();
            std::thread::sleep(Duration::from_secs(1));
        }

        let install_dir = std::path::Path::new(&app_path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/Applications".to_string());

        let ok = match ext {
            "pkg" => {
                out!("Installation du package…");
                {
                    let q = guard::posix_shell_quote(&tmp);
                    run_admin_sh(&format!("installer -pkg {} -target /", q)).is_ok()
                }
            }
            "zip" | "tar.gz" | "tar.bz2" | "tar.xz" => {
                let tmp_dir = work_dir
                    .path()
                    .join("extracted")
                    .to_string_lossy()
                    .into_owned();
                fs::create_dir_all(&tmp_dir).ok();
                out!("Extraction de l'archive…");
                let ok_x = if ext == "zip" {
                    Command::new("unzip")
                        .args(["-q", "-o", &tmp, "-d", &tmp_dir])
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false)
                } else {
                    Command::new("tar")
                        .args(["-xf", &tmp, "-C", &tmp_dir])
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false)
                };
                if !ok_x {
                    out!("✗ Extraction échouée");
                    let _ = fs::remove_file(&tmp);
                    done!(false);
                }
                let app_src = find_app_bundle(&tmp_dir);
                if app_src.is_none() {
                    out!("✗ Aucune .app dans l'archive");
                    let _ = fs::remove_dir_all(&tmp_dir);
                    let _ = fs::remove_file(&tmp);
                    done!(false);
                }
                out!(format!(
                    "Installation de {}…",
                    app_src.as_deref().unwrap_or("")
                ));
                let result = match copy_app(app_src.as_deref().unwrap(), &install_dir) {
                    Ok(()) => true,
                    Err(e) => {
                        out!(format!("✗ Copie échouée : {}", e));
                        false
                    }
                };
                let _ = fs::remove_dir_all(&tmp_dir);
                result
            }
            _ => {
                // dmg
                // Snapshot /Volumes avant le montage (fallback de détection)
                let before_vols: std::collections::HashSet<String> = fs::read_dir("/Volumes")
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.path().to_string_lossy().to_string())
                    .collect();

                out!("Montage du DMG…");
                // Sans -plist/-quiet : la sortie texte contient le point de montage
                // sous la forme "/dev/diskN\ttype\t/Volumes/..."
                let mount_out = Command::new("hdiutil")
                    .args(["attach", &tmp, "-nobrowse"])
                    .output();

                let Ok(mo) = mount_out else {
                    out!("✗ Impossible de lancer hdiutil");
                    let _ = fs::remove_file(&tmp);
                    done!(false);
                };

                // Chercher /Volumes/... dans la sortie texte de hdiutil
                let stdout_text = String::from_utf8_lossy(&mo.stdout);
                let mount = stdout_text
                    .lines()
                    .filter_map(|line| {
                        line.split('\t')
                            .next_back()
                            .map(|p| p.trim())
                            .filter(|p| p.starts_with("/Volumes/"))
                            .map(|p| p.to_string())
                    })
                    .next()
                    // Fallback : comparer /Volumes avant/après
                    .or_else(|| {
                        let after_vols: std::collections::HashSet<String> =
                            fs::read_dir("/Volumes")
                                .into_iter()
                                .flatten()
                                .flatten()
                                .filter(|e| e.path().is_dir())
                                .map(|e| e.path().to_string_lossy().to_string())
                                .collect();
                        after_vols.difference(&before_vols).next().cloned()
                    });

                let Some(ref mp) = mount else {
                    let err = String::from_utf8_lossy(&mo.stderr).trim().to_string();
                    out!(format!(
                        "✗ Montage échoué : {}",
                        if err.is_empty() {
                            "volume introuvable".into()
                        } else {
                            err
                        }
                    ));
                    let _ = fs::remove_file(&tmp);
                    done!(false);
                };
                out!(format!("Volume monté : {}", mp));

                let app_src = find_app_bundle(mp);
                let Some(ref src) = app_src else {
                    out!("✗ Aucune .app trouvée dans le volume");
                    let _ = Command::new("hdiutil")
                        .args(["detach", mp, "-quiet"])
                        .status();
                    let _ = fs::remove_file(&tmp);
                    done!(false);
                };
                out!(format!("Copie de {} → {}…", src, install_dir));

                let result = match copy_app(src, &install_dir) {
                    Ok(()) => true,
                    Err(e) => {
                        out!(format!("✗ Copie échouée : {}", e));
                        false
                    }
                };

                out!("Démontage…");
                let _ = Command::new("hdiutil")
                    .args(["detach", mp, "-quiet"])
                    .status();
                result
            }
        };

        let _ = fs::remove_file(&tmp);

        if ok {
            out!(format!("✓ {} mis à jour avec succès", name));
            done!(true);
        } else {
            out!(format!("✗ Échec de la mise à jour de {}", name));
            done!(false);
        }
    });
}

// ── External volumes ──────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct VolumeInfo {
    pub name: String,
    pub path: String,
    pub total_gb: f64,
    pub free_gb: f64,
}

fn volume_space(path: &str) -> (f64, f64) {
    let out = Command::new("df").args(["-k", path]).output().ok();
    let out = match out {
        Some(o) => o,
        None => return (0.0, 0.0),
    };
    let s = String::from_utf8_lossy(&out.stdout);
    let line = match s.lines().nth(1) {
        Some(l) => l,
        None => return (0.0, 0.0),
    };
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 4 {
        return (0.0, 0.0);
    }
    let total: f64 = parts[1].parse().unwrap_or(0.0) / 1_048_576.0;
    let free: f64 = parts[3].parse().unwrap_or(0.0) / 1_048_576.0;
    (total, free)
}

const SUDOERS_PATH: &str = "/etc/sudoers.d/burrow-pmset";
const MOLE_SMC: &str = "/Applications/Mole.app/Contents/Helpers/mole-smc";
// Fixed install path for burrow-smc — added to sudoers on first setup
const BURROW_SMC_INSTALL: &str = "/usr/local/lib/burrow-smc";
static BURROW_SMC_BUNDLED: OnceLock<String> = OnceLock::new();

fn find_burrow_smc_bundled(app: &tauri::AppHandle) -> Option<String> {
    if let Some(cached) = BURROW_SMC_BUNDLED.get() {
        return Some(cached.clone());
    }
    if let Ok(res) = app.path().resource_dir() {
        let p = res.join("burrow-smc");
        if p.exists() {
            let s = p.to_string_lossy().to_string();
            let _ = BURROW_SMC_BUNDLED.set(s.clone());
            return Some(s);
        }
    }
    None
}

fn burrow_smc_apply(app: &tauri::AppHandle, mode: u8, percent: u8) -> Result<(), String> {
    // Use installed (fixed-path) binary via sudo -n — requires sudoers setup
    let smc_path = if std::path::Path::new(BURROW_SMC_INSTALL).exists() {
        BURROW_SMC_INSTALL.to_string()
    } else if let Some(p) = find_burrow_smc_bundled(app) {
        p
    } else {
        return Err("burrow-smc not found".to_string());
    };

    // Try with sudo -n (passwordless via sudoers)
    if std::path::Path::new(SUDOERS_PATH).exists() {
        let out = Command::new("sudo")
            .args([
                "-n",
                &smc_path,
                "apply",
                &mode.to_string(),
                &percent.to_string(),
            ])
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            return Ok(());
        }
    }
    Err("sudoers not configured — click Configurer in Settings".to_string())
}

fn pmset_run(args: &[&str]) -> Result<(), String> {
    // Passwordless si sudoers est configuré (fan mode setup)
    if std::path::Path::new(SUDOERS_PATH).exists() {
        let mut cmd = Command::new("sudo");
        cmd.arg("-n").arg("/usr/bin/pmset").args(args);
        if cmd.output().map(|o| o.status.success()).unwrap_or(false) {
            return Ok(());
        }
    }
    // Fallback : sudo (Touch ID via pam_tid.so) → osascript
    run_admin_sh(&format!("/usr/bin/pmset {}", args.join(" ")))
}

/// Returns true if the sudoers file is set up (fan control + GPU metrics work without password)
#[tauri::command]
fn check_system_permissions() -> bool {
    std::path::Path::new(SUDOERS_PATH).exists()
}

/// Obtient le nom d'utilisateur courant via l'API POSIX (pas $USER).
/// $USER peut être forgé par le frontend — on utilise getpwuid(getuid()).
fn current_username() -> Result<String, String> {
    // SAFETY: getuid() et getpwuid_r() sont thread-safe sur macOS.
    let uid = unsafe { libc::getuid() };
    let mut pwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut buf = vec![0i8; 1024];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let ret = unsafe {
        libc::getpwuid_r(
            uid,
            pwd.as_mut_ptr(),
            buf.as_mut_ptr(),
            buf.len(),
            &mut result,
        )
    };
    if ret != 0 || result.is_null() {
        return Err(format!("getpwuid_r échoué (errno {})", ret));
    }
    let pw = unsafe { &*result };
    let name = unsafe { std::ffi::CStr::from_ptr(pw.pw_name) }
        .to_str()
        .map_err(|_| "Nom d'utilisateur non-UTF-8".to_string())?
        .to_string();
    if name.is_empty() {
        return Err("Nom d'utilisateur vide".to_string());
    }
    // Validation : uniquement alphanumériques + _ + - (noms macOS standard)
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!("Nom d'utilisateur suspect : {:?}", name));
    }
    Ok(name)
}

/// One-time setup: admin dialog installs burrow-smc to fixed path + writes sudoers for
/// passwordless pmset, powermetrics (GPU), burrow-smc (fan control), and mole-smc (if present).
#[tauri::command]
fn setup_system_permissions(app: tauri::AppHandle) -> Result<(), String> {
    let username = current_username()?;

    // Find bundled burrow-smc binary
    let bundled = find_burrow_smc_bundled(&app).unwrap_or_else(|| BURROW_SMC_INSTALL.to_string());

    let mole_line = if std::path::Path::new(MOLE_SMC).exists() {
        format!(
            "{u} ALL=(root) NOPASSWD: {s}\\n",
            u = username,
            s = MOLE_SMC
        )
    } else {
        String::new()
    };

    // Quoting POSIX correct via guard::posix_shell_quote
    let q_bundled = guard::posix_shell_quote(&bundled);

    // In the admin shell: copy burrow-smc to fixed location, write sudoers, run pmset to test
    let shell_cmd = format!(
        "mkdir -p /usr/local/lib && \
         cp -f {bundled} {smc} && \
         chmod 755 {smc} && \
         printf '{u} ALL=(root) NOPASSWD: /usr/bin/pmset\\n\
{u} ALL=(root) NOPASSWD: /usr/bin/powermetrics\\n\
{u} ALL=(root) NOPASSWD: {smc}\\n\
{mole}' > {p} && \
         chmod 440 {p} && \
         /usr/bin/pmset -a lowpowermode 0",
        bundled = q_bundled,
        smc = BURROW_SMC_INSTALL,
        u = username,
        mole = mole_line,
        p = SUDOERS_PATH,
    );

    run_admin_sh(&shell_cmd).map_err(|e| {
        if e.is_empty() {
            "Setup failed".to_string()
        } else {
            e
        }
    })
}

/// Use mole-smc (bundled with Mole.app) to write SMC fan keys directly via IOKit.
/// mode=0 → release fan control (auto), mode=1 → force targetPercent (0–100).
fn mole_smc_apply(mode: u8, percent: u8) -> Result<(), String> {
    if !std::path::Path::new(MOLE_SMC).exists() {
        return Err("mole-smc not found".to_string());
    }
    let out = Command::new("sudo")
        .args([
            "-n",
            MOLE_SMC,
            "apply",
            &mode.to_string(),
            &percent.to_string(),
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

#[tauri::command]
fn set_fan_mode(app: tauri::AppHandle, mode: String) -> Result<(), String> {
    match mode.as_str() {
        "silent" => {
            let _ = burrow_smc_apply(&app, 0, 0).or_else(|_| mole_smc_apply(0, 0));
            pmset_run(&["-a", "lowpowermode", "1"])?;
        }
        "auto" => {
            let _ = burrow_smc_apply(&app, 0, 0).or_else(|_| mole_smc_apply(0, 0));
            pmset_run(&["-a", "lowpowermode", "0"])?;
        }
        "cool" => {
            pmset_run(&["-a", "lowpowermode", "0", "disksleep", "0", "sleep", "0"])?;
            burrow_smc_apply(&app, 1, 60)
                .or_else(|_| mole_smc_apply(1, 60))
                .map_err(|_| {
                    "SMC fan control unavailable — install burrow-smc via Réglages".to_string()
                })?;
        }
        _ => return Err(format!("Unknown mode: {mode}")),
    }
    Ok(())
}

#[tauri::command]
fn list_volumes() -> Vec<VolumeInfo> {
    use std::os::unix::fs::MetadataExt;
    let root_dev = fs::metadata("/").map(|m| m.dev()).unwrap_or(0);
    let mut volumes = Vec::new();
    let Ok(entries) = fs::read_dir("/Volumes") else {
        return volumes;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path().to_string_lossy().to_string();
        // skip volumes on the same device as root (system volumes)
        let vol_dev = fs::metadata(&path).map(|m| m.dev()).unwrap_or(root_dev);
        if vol_dev == root_dev {
            continue;
        }
        // skip DMG/disk image mounts (Protocol: Disk Image)
        let is_dmg = Command::new("diskutil")
            .args(["info", &path])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("Disk Image"))
            .unwrap_or(false);
        if is_dmg {
            continue;
        }
        let (total_gb, free_gb) = volume_space(&path);
        volumes.push(VolumeInfo {
            name,
            path,
            total_gb,
            free_gb,
        });
    }
    volumes
}

// Read fan speed from powermetrics SMC sampler (requires sudoers), or fall back to mo cache.
fn sample_fan_rpm() -> u32 {
    if std::path::Path::new(SUDOERS_PATH).exists() {
        if let Ok(out) = Command::new("sudo")
            .args([
                "-n",
                "/usr/bin/powermetrics",
                "-n",
                "1",
                "-i",
                "200",
                "--samplers",
                "smc",
            ])
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                let lo = line.to_lowercase();
                if lo.contains("fan") && lo.contains("rpm") {
                    let n: String = line
                        .chars()
                        .skip_while(|c| !c.is_ascii_digit())
                        .take_while(|c| c.is_ascii_digit())
                        .collect();
                    if let Ok(r) = n.parse::<u32>() {
                        if r > 0 {
                            return r;
                        }
                    }
                }
            }
        }
    }
    // Fallback: mo cache
    if let Some(guard) = METRICS_CACHE.get().and_then(|m| m.lock().ok()) {
        if let Some(ref m) = guard.data {
            return m.thermal_fan_speed.max(0) as u32;
        }
    }
    0
}

fn extract_ioreg_num(line: &str, needle: &str) -> f64 {
    if let Some(pos) = line.find(needle) {
        let rest = &line[pos + needle.len()..];
        let num_str: String = rest
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit())
            .collect();
        return num_str.parse::<f64>().unwrap_or(0.0);
    }
    0.0
}

fn sample_gpu_ioreg() -> f64 {
    let Ok(output) = Command::new("/usr/sbin/ioreg")
        .args(["-r", "-c", "AGXAccelerator", "-k", "PerformanceStatistics"])
        .output()
    else {
        return 0.0;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if line.contains("PerformanceStatistics") && line.contains("Device Utilization") {
            let device = extract_ioreg_num(line, "\"Device Utilization %\"");
            let renderer = extract_ioreg_num(line, "\"Renderer Utilization %\"");
            let tiler = extract_ioreg_num(line, "\"Tiler Utilization %\"");
            return device.max(renderer).max(tiler).min(100.0);
        }
    }
    0.0
}

fn start_sysinfo_daemon() {
    // CPU/memory via sysinfo (per-core data for Dashboard bars)
    std::thread::spawn(|| {
        use sysinfo::System;
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        std::thread::sleep(Duration::from_millis(250));
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        let _ = QUICK_SYS.set(Mutex::new(sys));
        loop {
            std::thread::sleep(Duration::from_millis(1500));
            if let Some(lock) = QUICK_SYS.get() {
                if let Ok(mut s) = lock.lock() {
                    s.refresh_cpu_usage();
                    s.refresh_memory();
                }
            }
        }
    });

    // GPU/CPU perf + temps + power via IOReport+SMC — same approach as macmon, no root.
    std::thread::spawn(|| match ior::Sampler::new() {
        Ok(sampler) => loop {
            let m = sampler.get_metrics(500);
            GPU_USAGE_X10.store((m.gpu_pct * 10.0) as u32, Ordering::Relaxed);
            CPU_TEMP_X10.store((m.cpu_temp * 10.0) as u32, Ordering::Relaxed);
            GPU_TEMP_X10.store((m.gpu_temp * 10.0) as u32, Ordering::Relaxed);
            SOC_TEMP_X10.store((m.soc_temp * 10.0) as u32, Ordering::Relaxed);
            NAND_TEMP_X10.store((m.nand_temp * 10.0) as u32, Ordering::Relaxed);
            ANE_TEMP_X10.store((m.ane_temp * 10.0) as u32, Ordering::Relaxed);
            CPU_POWER_X10.store((m.cpu_power * 10.0) as u32, Ordering::Relaxed);
            GPU_POWER_X10.store((m.gpu_power * 10.0) as u32, Ordering::Relaxed);
            RAM_POWER_X10.store((m.ram_power * 10.0) as u32, Ordering::Relaxed);
            ANE_POWER_X10.store((m.ane_power * 10.0) as u32, Ordering::Relaxed);
        },
        Err(_) => loop {
            let v = sample_gpu_ioreg();
            GPU_USAGE_X10.store((v * 10.0) as u32, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(1000));
        },
    });

    // Fan speed via powermetrics SMC (when sudoers configured, every 2s)
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_millis(2000));
        let rpm = sample_fan_rpm();
        if rpm > 0 {
            FAN_RPM.store(rpm, Ordering::Relaxed);
        }
    });
}

#[tauri::command]
fn get_quick_metrics() -> QuickMetrics {
    use sysinfo::{Disks, System};

    let mut cpu_usage = 0f64;
    let mut cpu_per_core: Vec<f64> = vec![];
    let mut cpu_core_count = 0usize;
    let mut mem_used = 0u64;
    let mut mem_total = 0u64;
    let mut mem_swap_used = 0u64;
    let mut mem_swap_total = 0u64;

    if let Some(lock) = QUICK_SYS.get() {
        if let Ok(sys) = lock.lock() {
            cpu_usage = sys.global_cpu_usage() as f64;
            cpu_per_core = sys.cpus().iter().map(|c| c.cpu_usage() as f64).collect();
            cpu_core_count = sys.cpus().len();
            mem_used = sys.used_memory();
            mem_total = sys.total_memory();
            mem_swap_used = sys.used_swap();
            mem_swap_total = sys.total_swap();
        }
    }

    let mem_used_percent = if mem_total > 0 {
        mem_used as f64 / mem_total as f64 * 100.0
    } else {
        0.0
    };

    let disks = Disks::new_with_refreshed_list();
    let (disk_used, disk_total) = disks
        .iter()
        .find(|d| d.mount_point() == Path::new("/"))
        .map(|d| (d.total_space() - d.available_space(), d.total_space()))
        .unwrap_or((0, 0));
    let disk_used_percent = if disk_total > 0 {
        disk_used as f64 / disk_total as f64 * 100.0
    } else {
        0.0
    };

    let load = System::load_average();
    // Read from cache — updated every 2s by background GPU thread (non-blocking)
    let load_x10 = |a: &AtomicU32| a.load(Ordering::Relaxed) as f64 / 10.0;

    QuickMetrics {
        cpu_usage,
        cpu_per_core,
        cpu_core_count,
        cpu_load1: load.one,
        cpu_load5: load.five,
        cpu_load15: load.fifteen,
        mem_used,
        mem_total,
        mem_used_percent,
        mem_swap_used,
        mem_swap_total,
        disk_used,
        disk_total,
        disk_used_percent,
        uptime_secs: System::uptime(),
        gpu_busy_percent: load_x10(&GPU_USAGE_X10),
        fan_speed_rpm: FAN_RPM.load(Ordering::Relaxed) as f64,
        cpu_temp: load_x10(&CPU_TEMP_X10),
        gpu_temp: load_x10(&GPU_TEMP_X10),
        soc_temp: load_x10(&SOC_TEMP_X10),
        nand_temp: load_x10(&NAND_TEMP_X10),
        ane_temp: load_x10(&ANE_TEMP_X10),
        cpu_power: load_x10(&CPU_POWER_X10),
        gpu_power: load_x10(&GPU_POWER_X10),
        ram_power: load_x10(&RAM_POWER_X10),
        ane_power: load_x10(&ANE_POWER_X10),
    }
}

// ── Storage analysis (Stockage) ───────────────────────────────────────────────

fn du_bytes(path: &Path) -> u64 {
    Command::new("du")
        .args(["-sk", &path.to_string_lossy()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| {
            s.split_whitespace()
                .next()
                .and_then(|n| n.parse::<u64>().ok())
        })
        .map(|kb| kb.saturating_mul(1024))
        .unwrap_or(0)
}

fn days_since_modified(path: &Path) -> i64 {
    fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
        .map(|d| (d.as_secs() / 86400) as i64)
        .unwrap_or(-1)
}

#[derive(Serialize, Clone)]
pub struct DiskCategory {
    pub id: String,
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
}

#[tauri::command]
fn get_disk_categories() -> Vec<DiskCategory> {
    let home = home_dir();
    let items: Vec<(String, String, std::path::PathBuf)> = vec![
        (
            "applications".into(),
            "Applications".into(),
            std::path::PathBuf::from("/Applications"),
        ),
        (
            "documents".into(),
            "Documents".into(),
            home.join("Documents"),
        ),
        (
            "downloads".into(),
            "Téléchargements".into(),
            home.join("Downloads"),
        ),
        ("desktop".into(), "Bureau".into(), home.join("Desktop")),
        (
            "developer".into(),
            "Développeur".into(),
            home.join("Developer"),
        ),
        ("movies".into(), "Films".into(), home.join("Movies")),
        ("music".into(), "Musique".into(), home.join("Music")),
        ("pictures".into(), "Photos".into(), home.join("Pictures")),
        ("trash".into(), "Corbeille".into(), home.join(".Trash")),
    ];
    let handles: Vec<_> = items
        .into_iter()
        .map(|(id, name, path)| {
            std::thread::spawn(move || {
                if !path.exists() {
                    return None;
                }
                let size_bytes = du_bytes(&path);
                if size_bytes == 0 {
                    return None;
                }
                Some(DiskCategory {
                    id,
                    name,
                    path: path.to_string_lossy().to_string(),
                    size_bytes,
                })
            })
        })
        .collect();
    let mut cats: Vec<DiskCategory> = handles
        .into_iter()
        .filter_map(|h| h.join().ok().flatten())
        .collect();
    cats.sort_by_key(|k| std::cmp::Reverse(k.size_bytes));
    cats
}

#[derive(Serialize, Clone)]
pub struct DevCache {
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: String,
    pub size_bytes: u64,
    pub risk: u8,
    pub days_since_use: i64,
}

#[tauri::command]
fn get_dev_caches() -> Vec<DevCache> {
    let home = home_dir();
    // (risk, id, name, subpath, description)
    let entries: &[(u8, &str, &str, &str, &str)] = &[
        // ── JavaScript / Node ─────────────────────────────────────────────────
        (0, "npm",          "npm cache",            ".npm",
         "Paquets mis en cache par npm. Se reconstruit automatiquement au prochain install."),
        (0, "yarn_v1",      "Yarn v1",               ".yarn/cache",
         "Cache global Yarn v1. Se reconstruit automatiquement."),
        (0, "pnpm",         "pnpm store",            "Library/pnpm",
         "Store global pnpm partagé entre projets. Les liens durs seront recréés."),
        (0, "bun",          "Bun cache",             "Library/Caches/bun",
         "Cache des paquets Bun. Se reconstruit automatiquement."),
        (0, "deno",         "Deno cache",            "Library/Caches/deno",
         "Modules Deno mis en cache. Retéléchargés à la prochaine exécution."),
        // ── Python ───────────────────────────────────────────────────────────
        (0, "pip",          "pip cache",             "Library/Caches/pip",
         "Paquets Python mis en cache par pip. Retéléchargés au prochain install."),
        (0, "pip_alt",      "pip cache (alt)",       ".cache/pip",
         "Cache pip alternatif (~/.cache/pip). Comportement identique au cache pip principal."),
        (0, "uv",           "UV cache",              "Library/Caches/uv",
         "Cache du gestionnaire de paquets UV. Se reconstruit automatiquement."),
        (0, "poetry",       "Poetry cache",          "Library/Caches/pypoetry",
         "Cache des dépendances Poetry. Retéléchargées au prochain install."),
        (0, "pipenv_venv",  "pipenv virtualenvs",    ".local/share/virtualenvs",
         "Environnements virtuels créés par pipenv. Recréables avec `pipenv install`."),
        (0, "conda",        "Conda pkgs",            ".conda/pkgs",
         "Paquets Conda mis en cache. Retéléchargés au prochain install."),
        // ── JVM ──────────────────────────────────────────────────────────────
        (0, "gradle",       "Gradle caches",         ".gradle/caches",
         "Caches Gradle (dépendances, caches de build). Se reconstruit au prochain build."),
        (0, "gradle_wrap",  "Gradle Wrapper",        ".gradle/wrapper/dists",
         "Binaires Gradle Wrapper téléchargés. Retéléchargés automatiquement si supprimés."),
        (0, "maven",        "Maven local repo",      ".m2/repository",
         "Dépôt Maven local (~/.m2). Les dépendances sont retéléchargées au prochain build."),
        (0, "sbt",          "SBT cache",             ".sbt",
         "Cache SBT (Scala). Plugins et dépendances retéléchargés au prochain build."),
        (0, "ivy",          "Ivy cache",             ".ivy2/cache",
         "Cache Ivy (Java/Scala). Retéléchargé automatiquement."),
        // ── iOS / macOS ──────────────────────────────────────────────────────
        (0, "cocoapods",    "CocoaPods cache",       "Library/Caches/CocoaPods",
         "Pods mis en cache. Retéléchargés au prochain `pod install`."),
        (0, "carthage",     "Carthage cache",        "Library/Caches/org.carthage.CarthageKit",
         "Frameworks Carthage mis en cache. Retéléchargés si supprimés."),
        (0, "swiftpm",      "Swift PM cache",        "Library/Caches/org.swift.swiftpm",
         "Paquets Swift Package Manager téléchargés. Se reconstruisent au prochain build."),
        (0, "xcode_logs",   "Xcode Logs",            "Library/Developer/Xcode/Logs",
         "Logs de build et de simulateur Xcode. Aucun impact fonctionnel."),
        (0, "xcode_prev",   "Xcode Previews",        "Library/Developer/Xcode/Previews",
         "Aperçus SwiftUI générés. Regénérés automatiquement."),
        (1, "xcode_dd",     "Xcode DerivedData",     "Library/Developer/Xcode/DerivedData",
         "Données de build Xcode. Le prochain build repart de zéro (compilation lente)."),
        (1, "xcode_devsup", "Xcode Device Support",  "Library/Developer/Xcode/iOS DeviceSupport",
         "Symboles pour débogage sur appareils physiques. Retéléchargés si nécessaire."),
        (1, "ios_sim_cache","iOS Simulateurs (cache)","Library/Developer/CoreSimulator/Caches",
         "Caches du simulateur iOS. La prochaine session simulateur peut être plus lente."),
        (2, "ios_sim_dev",  "iOS Simulateurs (appareils)","Library/Developer/CoreSimulator/Devices",
         "Données des appareils simulés (apps installées, données). Non récupérables."),
        (2, "xcode_arch",   "Xcode Archives",        "Library/Developer/Xcode/Archives",
         "Archives Xcode (binaires signés, distributions App Store). JAMAIS récupérables sans sauvegarde."),
        // ── Ruby ─────────────────────────────────────────────────────────────
        (0, "rubygems",     "RubyGems",              ".gem",
         "Gems Ruby installées globalement. Réinstallables avec `gem install`."),
        (0, "bundler",      "Bundler cache",         ".bundle/cache",
         "Cache global Bundler. Se reconstruit au prochain `bundle install`."),
        (1, "rbenv",        "rbenv versions",        ".rbenv/versions",
         "Versions Ruby installées via rbenv. Large téléchargement si supprimées."),
        (1, "rvm",          "RVM rubies",            ".rvm/rubies",
         "Versions Ruby installées via RVM. Large téléchargement si supprimées."),
        // ── Go ────────────────────────────────────────────────────────────────
        (0, "go_build",     "Go build cache",        ".cache/go-build",
         "Cache de compilation Go. Se reconstruit automatiquement à la prochaine compilation."),
        (0, "go_mod",       "Go module cache",       "go/pkg/mod",
         "Cache des modules Go (~~/go/pkg/mod). Retéléchargés si supprimés."),
        // ── Rust ─────────────────────────────────────────────────────────────
        (0, "cargo_reg",    "Cargo registry",        ".cargo/registry",
         "Registry crates.io (sources + index). Retéléchargé à la prochaine compilation Rust."),
        (0, "cargo_git",    "Cargo git sources",     ".cargo/git",
         "Sources Git clonées par Cargo. Reclonées automatiquement si supprimées."),
        // ── Autres gestionnaires ──────────────────────────────────────────────
        (0, "composer",     "Composer cache",        ".composer/cache",
         "Cache Composer (PHP). Les paquets sont retéléchargés au prochain install."),
        (0, "terraform",    "Terraform plugins",     ".terraform.d/plugin-cache",
         "Plugins Terraform téléchargés. Retéléchargés au prochain `terraform init`."),
        (0, "bazel",        "Bazel cache",           ".cache/bazel",
         "Cache de build Bazel. Se reconstruit au prochain build."),
        (0, "pub",          "Dart/Flutter pub",      ".pub-cache",
         "Cache pub (Dart/Flutter). Packages retéléchargés au prochain `flutter pub get`."),
        (0, "docker_buildx","Docker buildx cache",   ".docker/buildx",
         "Cache Docker buildx (couches de build). Retéléchargé au prochain build Docker."),
        // ── Testing ───────────────────────────────────────────────────────────
        (0, "playwright",   "Playwright browsers",   "Library/Caches/ms-playwright",
         "Navigateurs Playwright (Chromium, Firefox, WebKit). Réinstallables via `playwright install`."),
        (0, "cypress",      "Cypress cache",         "Library/Caches/Cypress",
         "Binaires Cypress. Retéléchargés automatiquement."),
        (0, "puppeteer",    "Puppeteer cache",       "Library/Caches/puppeteer",
         "Binaires Puppeteer (Chromium). Retéléchargés à la prochaine installation."),
        (0, "prisma",       "Prisma engines",        "Library/Caches/prisma",
         "Binaires Prisma Query Engine. Regénérés au prochain `prisma generate`."),
        // ── AI Tools ──────────────────────────────────────────────────────────
        (1, "ollama",       "Ollama models",         ".ollama/models",
         "Modèles LLM Ollama téléchargés. Grand téléchargement si supprimés."),
        (1, "huggingface",  "HuggingFace cache",     ".cache/huggingface/hub",
         "Modèles HuggingFace téléchargés. Grand téléchargement si supprimés."),
        (1, "claude_desktop", "Claude Desktop",        "Library/Application Support/Claude",
         "Historique de conversations et cache Claude Desktop. Peut devenir très volumineux."),
        (0, "claude_code",  "Claude Code",           ".claude",
         "Historique de sessions et config Claude Code CLI. Regénéré au prochain lancement."),
        (0, "cursor_cache", "Cursor cache",          "Library/Application Support/Cursor/Cache",
         "Cache Cursor IDE (Chromium). Vidé automatiquement au redémarrage de Cursor."),
        (0, "cursor_data",  "Cursor CachedData",     "Library/Application Support/Cursor/CachedData",
         "JS compilé de Cursor. Regénéré au premier lancement."),
        (0, "windsurf",     "Windsurf cache",        "Library/Application Support/Windsurf/Cache",
         "Cache Windsurf IDE. Regénéré automatiquement."),
        (0, "chatgpt",      "ChatGPT cache",         "Library/Application Support/ChatGPT/Cache",
         "Cache ChatGPT Desktop. Peut nécessiter une reconnexion."),
        // ── VS Code ───────────────────────────────────────────────────────────
        (0, "vscode_cache", "VS Code cache",         "Library/Application Support/Code/Cache",
         "Cache VS Code (Electron). Vidé automatiquement au redémarrage de VS Code."),
        (0, "vscode_data",  "VS Code CachedData",    "Library/Application Support/Code/CachedData",
         "JS compilé de VS Code. Regénéré au premier lancement."),
        (0, "vscode_logs",  "VS Code logs",          "Library/Application Support/Code/logs",
         "Logs VS Code. Aucun impact fonctionnel."),
        (0, "vscode_gpu",   "VS Code GPU cache",     "Library/Application Support/Code/GPUCache",
         "Cache GPU Chromium de VS Code. Regénéré automatiquement."),
        (1, "vscode_ext",   "VS Code extensions",    ".vscode/extensions",
         "Extensions VS Code installées. Réinstallables depuis le marketplace."),
        // ── JetBrains ─────────────────────────────────────────────────────────
        (1, "jetbrains",    "JetBrains caches",      "Library/Caches/JetBrains",
         "Caches des IDEs JetBrains (IntelliJ, WebStorm…). Regénérés au prochain lancement."),
        // ── Game Engines ──────────────────────────────────────────────────────
        (0, "unity_cache",  "Unity cache",           "Library/Caches/Unity",
         "Cache Unity Editor (shaders, assets). Regénéré à l'ouverture du projet."),
        (0, "unity_hub",    "Unity Hub cache",       "Library/Application Support/UnityHub/cache",
         "Cache Unity Hub (installeurs). Retéléchargé si nécessaire."),
        (0, "godot",        "Godot cache",           "Library/Application Support/Godot",
         "Cache Godot Editor. Regénéré automatiquement."),
        // ── Version managers ─────────────────────────────────────────────────
        (1, "nvm",          "nvm Node versions",     ".nvm/versions",
         "Versions Node.js installées via nvm. Réinstallables avec `nvm install`."),
        (1, "pyenv",        "pyenv Python versions", ".pyenv/versions",
         "Versions Python installées via pyenv. Réinstallables avec `pyenv install`."),
        (0, "mise_cache",   "mise cache",            ".mise/cache",
         "Cache mise (gestionnaire de versions). Retéléchargé automatiquement."),
        // ── Cloud ─────────────────────────────────────────────────────────────
        (0, "aws_cli",      "AWS CLI cache",         ".aws/cli/cache",
         "Credentials temporaires et métadonnées AWS CLI. Peut forcer une ré-authentification."),
        // ── Homebrew ─────────────────────────────────────────────────────────
        (1, "brew_cache",   "Homebrew téléchargements","Library/Caches/Homebrew",
         "Archives des formules Homebrew téléchargées. Retéléchargées si nécessaire."),
        // ── Android ──────────────────────────────────────────────────────────
        (2, "android_avd",  "Android Emulators",     ".android/avd",
         "Données des émulateurs Android (AVD). Les données des appareils virtuels seront perdues."),
    ];
    let handles: Vec<_> = entries
        .iter()
        .map(|&(risk, id, name, subpath, desc)| {
            let path = home.join(subpath);
            let id = id.to_string();
            let name = name.to_string();
            let description = desc.to_string();
            std::thread::spawn(move || {
                if !path.exists() {
                    return None;
                }
                let size_bytes = du_bytes(&path);
                if size_bytes == 0 {
                    return None;
                }
                let days_since_use = days_since_modified(&path);
                Some(DevCache {
                    id,
                    name,
                    description,
                    path: path.to_string_lossy().to_string(),
                    size_bytes,
                    risk,
                    days_since_use,
                })
            })
        })
        .collect();
    let mut result: Vec<DevCache> = handles
        .into_iter()
        .filter_map(|h| h.join().ok().flatten())
        .collect();
    result.sort_by_key(|k| std::cmp::Reverse(k.size_bytes));
    result
}

#[derive(Serialize, Clone)]
pub struct ProjectArtifact {
    pub project_name: String,
    pub project_path: String,
    pub artifact_type: String,
    pub artifact_path: String,
    pub size_bytes: u64,
}

fn find_artifacts(
    dir: &Path,
    out: &mut Vec<(String, String, String, std::path::PathBuf)>,
    depth: u32,
) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let hidden_ok = matches!(
            name.as_str(),
            ".next" | ".nuxt" | ".build" | ".venv" | ".gradle"
        );
        if name.starts_with('.') && !hidden_ok {
            continue;
        }
        let atype: Option<&'static str> = match name.as_str() {
            "node_modules" => Some("node_modules"),
            ".next" => Some("Next.js build"),
            ".nuxt" => Some("Nuxt build"),
            "__pycache__" => Some("Python cache"),
            ".venv" | "venv" => Some("Python venv"),
            "vendor" if dir.join("composer.json").exists() => Some("Composer vendor/"),
            "vendor" if dir.join("go.mod").exists() => Some("Go vendor/"),
            "vendor" if dir.join("Gemfile").exists() => Some("Ruby vendor/"),
            "build"
                if dir.join("build.gradle").exists() || dir.join("build.gradle.kts").exists() =>
            {
                Some("Gradle build/")
            }
            "build" if dir.join("CMakeLists.txt").exists() => Some("CMake build/"),
            "target" if dir.join("Cargo.toml").exists() => Some("Rust target/"),
            "target" if dir.join("pom.xml").exists() => Some("Maven target/"),
            ".build" if dir.join("Package.swift").exists() => Some("Swift .build/"),
            ".gradle"
                if dir.join("build.gradle").exists() || dir.join("build.gradle.kts").exists() =>
            {
                Some("Gradle cache")
            }
            _ => None,
        };
        if let Some(t) = atype {
            let proj_name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            out.push((
                proj_name,
                dir.to_string_lossy().to_string(),
                t.to_string(),
                path,
            ));
        } else {
            let skip = matches!(
                name.as_str(),
                ".git"
                    | "target"
                    | ".build"
                    | "node_modules"
                    | "__pycache__"
                    | ".venv"
                    | "venv"
                    | "vendor"
                    | "build"
            );
            if !skip {
                find_artifacts(&path, out, depth - 1);
            }
        }
    }
}

#[tauri::command]
fn get_project_artifacts() -> Vec<ProjectArtifact> {
    let home = home_dir();
    let scan_dirs: Vec<std::path::PathBuf> = [
        "Developer",
        "Documents",
        "Desktop",
        "Code",
        "Projects",
        "repos",
        "workspace",
        "src",
        "Source",
        "git",
        "work",
        "code",
    ]
    .iter()
    .map(|d| home.join(d))
    .filter(|p| p.exists())
    .collect();
    let mut raw: Vec<(String, String, String, std::path::PathBuf)> = Vec::new();
    for dir in &scan_dirs {
        find_artifacts(dir, &mut raw, 4);
    }
    let handles: Vec<_> = raw
        .into_iter()
        .map(|(proj, proj_path, atype, path)| {
            std::thread::spawn(move || {
                let size_bytes = du_bytes(&path);
                if size_bytes < 1_048_576 {
                    return None;
                }
                Some(ProjectArtifact {
                    project_name: proj,
                    project_path: proj_path,
                    artifact_type: atype,
                    artifact_path: path.to_string_lossy().to_string(),
                    size_bytes,
                })
            })
        })
        .collect();
    let mut all: Vec<ProjectArtifact> = handles
        .into_iter()
        .filter_map(|h| h.join().ok().flatten())
        .collect();
    all.sort_by_key(|k| std::cmp::Reverse(k.size_bytes));
    all.truncate(150);
    all
}

#[derive(Serialize, Clone)]
pub struct LargeFile {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub days_old: u64,
}

#[tauri::command]
fn get_large_files() -> Vec<LargeFile> {
    let home = home_dir();
    let dirs: Vec<String> = [
        "Downloads",
        "Documents",
        "Desktop",
        "Movies",
        "Music",
        "Pictures",
    ]
    .iter()
    .map(|d| home.join(d))
    .filter(|p| p.exists())
    .map(|p| p.to_string_lossy().to_string())
    .collect();
    if dirs.is_empty() {
        return vec![];
    }
    let mut args: Vec<String> = dirs;
    args.extend(["-type".into(), "f".into(), "-size".into(), "+100M".into()]);
    let Ok(out) = Command::new("find").args(&args).output() else {
        return vec![];
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut files: Vec<LargeFile> = text
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let path = std::path::Path::new(line);
            let meta = fs::metadata(path).ok()?;
            let days_old = meta
                .modified()
                .ok()
                .and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
                .map(|d| d.as_secs() / 86400)
                .unwrap_or(0);
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(line)
                .to_string();
            Some(LargeFile {
                name,
                path: line.to_string(),
                size_bytes: meta.len(),
                days_old,
            })
        })
        .collect();
    files.sort_by_key(|k| std::cmp::Reverse(k.size_bytes));
    files.truncate(100);
    files
}

#[tauri::command]
fn empty_trash() -> Result<(), String> {
    let out = Command::new("osascript")
        .args(["-e", r#"tell application "Finder" to empty trash"#])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let e = String::from_utf8_lossy(&out.stderr).to_string();
        Err(if e.is_empty() {
            "Échec".to_string()
        } else {
            e
        })
    }
}

// ── DerivedData project breakdown ─────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct DerivedDataProject {
    pub name: String,
    pub path: String,
    pub workspace_path: String,
    pub size_bytes: u64,
}

#[tauri::command]
fn get_derived_data_projects() -> Vec<DerivedDataProject> {
    let dd = home_dir().join("Library/Developer/Xcode/DerivedData");
    if !dd.exists() {
        return vec![];
    }
    let Ok(entries) = fs::read_dir(&dd) else {
        return vec![];
    };
    let handles: Vec<_> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let path = e.path();
            std::thread::spawn(move || {
                let info = path.join("info.plist");
                let workspace_path = fs::read_to_string(&info)
                    .ok()
                    .and_then(|s| {
                        let key = s.find("WorkspaceRootPath")?;
                        let after = &s[key..];
                        let start = after.find("<string>")? + 8;
                        let end = after[start..].find("</string>")?;
                        Some(after[start..start + end].to_string())
                    })
                    .unwrap_or_default();
                let dir_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                let display = if let Some(h) = dir_name.rfind('-') {
                    dir_name[..h].replace('-', " ")
                } else {
                    dir_name
                };
                let size_bytes = du_bytes(&path);
                if size_bytes == 0 {
                    return None;
                }
                Some(DerivedDataProject {
                    name: display,
                    path: path.to_string_lossy().to_string(),
                    workspace_path,
                    size_bytes,
                })
            })
        })
        .collect();
    let mut result: Vec<DerivedDataProject> = handles
        .into_iter()
        .filter_map(|h| h.join().ok().flatten())
        .collect();
    result.sort_by_key(|k| std::cmp::Reverse(k.size_bytes));
    result
}

// ── Xcode running check ───────────────────────────────────────────────────────

#[tauri::command]
fn is_xcode_running() -> bool {
    Command::new("pgrep")
        .args(["-x", "Xcode"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ── Storage forecast (linear regression on daily samples) ────────────────────

fn burrow_data_dir() -> std::path::PathBuf {
    home_dir().join(".burrow")
}

#[derive(Serialize, Deserialize, Clone)]
struct DiskSample {
    ts: u64,
    used: u64,
    total: u64,
}

fn load_disk_history() -> Vec<DiskSample> {
    let path = burrow_data_dir().join("disk_history.json");
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_disk_sample_if_needed(used: u64, total: u64) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut samples = load_disk_history();
    if samples
        .last()
        .map(|s| now - s.ts > 23 * 3600)
        .unwrap_or(true)
    {
        samples.push(DiskSample {
            ts: now,
            used,
            total,
        });
        samples.retain(|s| now - s.ts < 90 * 86400);
        let dir = burrow_data_dir();
        let _ = fs::create_dir_all(&dir);
        if let Ok(json) = serde_json::to_string(&samples) {
            let _ = fs::write(dir.join("disk_history.json"), json);
        }
    }
}

#[tauri::command]
fn get_disk_forecast() -> Option<i64> {
    let samples = load_disk_history();
    if samples.len() < 7 {
        return None;
    }
    let total = samples.last()?.total as f64;
    if total == 0.0 {
        return None;
    }
    let first_ts = samples.first()?.ts as f64;
    let n = samples.len() as f64;
    let xs: Vec<f64> = samples
        .iter()
        .map(|s| (s.ts as f64 - first_ts) / 86400.0)
        .collect();
    let ys: Vec<f64> = samples.iter().map(|s| s.used as f64).collect();
    let sx: f64 = xs.iter().sum();
    let sy: f64 = ys.iter().sum();
    let sxy: f64 = xs.iter().zip(ys.iter()).map(|(x, y)| x * y).sum();
    let sxx: f64 = xs.iter().map(|x| x * x).sum();
    let denom = n * sxx - sx * sx;
    if denom == 0.0 {
        return None;
    }
    let slope = (n * sxy - sx * sy) / denom;
    if slope <= 0.0 {
        return None;
    }
    let last_y = *ys.last()?;
    let days = (total - last_y) / slope;
    if days <= 0.0 || days > 3650.0 {
        return None;
    }
    Some(days as i64)
}

// ── Image preview ─────────────────────────────────────────────────────────────

/// Détecte le type MIME à partir des magic bytes réels du fichier (pas de l'extension).
/// Retourne None si le contenu n'est pas une image reconnue.
fn detect_image_mime(header: &[u8]) -> Option<&'static str> {
    if header.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if header.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if header.starts_with(b"RIFF") && header.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else if header.starts_with(b"BM") {
        Some("image/bmp")
    } else if header.starts_with(b"II\x2a\x00") || header.starts_with(b"MM\x00\x2a") {
        Some("image/tiff")
    } else {
        None
    }
}

#[tauri::command]
fn read_image_preview(path: String) -> Option<String> {
    use base64::Engine;
    use std::os::unix::fs::MetadataExt;

    // Validation de chemin : doit être sous $HOME uniquement
    let home = home_dir();
    if home.as_os_str().is_empty() {
        return None;
    }
    let p = std::path::Path::new(&path);
    if !p.is_absolute() {
        return None;
    }
    // Refus des symlinks : on lit la cible réelle pour éviter un TOCTOU
    let sym_meta = fs::symlink_metadata(p).ok()?;
    if sym_meta.file_type().is_symlink() {
        return None;
    }
    // Doit être sous $HOME
    if !p.starts_with(&home) {
        return None;
    }
    // Limite de taille : 4 Mo max
    let meta = fs::metadata(p).ok()?;
    if meta.len() > 4_000_000 || meta.len() < 4 {
        return None;
    }
    // Capturer l'inode + taille pour revalidation TOCTOU
    let inode_before = meta.ino();
    let size_before = meta.len();

    // Lire uniquement les 12 premiers octets pour détecter le type
    let mut header = [0u8; 12];
    {
        use std::io::Read;
        let mut f = fs::File::open(p).ok()?;
        f.read_exact(&mut header).ok()?;
    }
    let mime = detect_image_mime(&header)?;

    // Lire le contenu complet
    let bytes = fs::read(p).ok()?;

    // Revalidation TOCTOU : même inode + même taille ?
    let meta2 = fs::metadata(p).ok()?;
    if meta2.ino() != inode_before || meta2.len() != size_before {
        return None;
    }

    Some(format!(
        "data:{};base64,{}",
        mime,
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}

// ── Disk browser ──────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct DiskEntry {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub is_dir: bool,
}

#[tauri::command]
fn get_disk_breakdown(path: String) -> Vec<DiskEntry> {
    let base = Path::new(&path);
    let Ok(dir_entries) = fs::read_dir(base) else {
        return vec![];
    };

    let children: Vec<std::path::PathBuf> = dir_entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.is_symlink() {
                None
            } else {
                Some(p)
            }
        })
        .collect();
    if children.is_empty() {
        return vec![];
    }

    // Single du call for all directories → much faster than N separate spawns
    let dir_paths: Vec<String> = children
        .iter()
        .filter(|p| p.is_dir())
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    let mut size_map: HashMap<String, u64> = HashMap::new();
    if !dir_paths.is_empty() {
        let mut args = vec!["-s".to_string(), "-k".to_string(), "--".to_string()];
        args.extend(dir_paths);
        if let Ok(out) = Command::new("du").args(&args).output() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                if let Some((kb_str, path_str)) = line.split_once('\t') {
                    if let Ok(kb) = kb_str.trim().parse::<u64>() {
                        size_map.insert(path_str.to_string(), kb.saturating_mul(1024));
                    }
                }
            }
        }
    }

    let mut entries: Vec<DiskEntry> = children
        .iter()
        .filter_map(|p| {
            let name = p.file_name()?.to_str()?.to_string();
            let is_dir = p.is_dir();
            let path_str = p.to_string_lossy().to_string();
            let size_bytes = if is_dir {
                size_map.get(&path_str).copied().unwrap_or(0)
            } else {
                fs::metadata(p).map(|m| m.len()).unwrap_or(0)
            };
            if size_bytes == 0 {
                return None;
            }
            Some(DiskEntry {
                name,
                path: path_str,
                size_bytes,
                is_dir,
            })
        })
        .collect();
    entries.sort_by_key(|k| std::cmp::Reverse(k.size_bytes));
    entries
}

// ── Homebrew Cask Browser ─────────────────────────────────────────────────────

#[tauri::command]
async fn get_installed_casks() -> Vec<String> {
    tauri::async_runtime::spawn_blocking(|| {
        let home = std::env::var("HOME").unwrap_or_default();

        // `brew info --installed --cask --json=v2` returns metadata for every installed cask,
        // including the exact artifact (.app) names. We cross-check with the filesystem so
        // that apps deleted outside of brew (e.g. via UninstallTab) no longer appear.
        let info = Command::new("brew")
            .args(["info", "--installed", "--cask", "--json=v2"])
            .output();

        if let Ok(output) = info {
            if output.status.success() && !output.stdout.is_empty() {
                if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                    if let Some(casks) = json["casks"].as_array() {
                        return casks
                            .iter()
                            .filter_map(|cask| {
                                let token = cask["token"].as_str()?.to_string();
                                let artifacts = cask["artifacts"].as_array()?;

                                // If no .app artifact is declared, trust brew (CLI tools, fonts…)
                                let has_app_artifact =
                                    artifacts.iter().any(|a| a.get("app").is_some());
                                if !has_app_artifact {
                                    return Some(token);
                                }

                                // Otherwise verify that at least one declared .app exists on disk
                                let app_on_disk = artifacts.iter().any(|art| {
                                    art.get("app")
                                        .and_then(|a| a.as_array())
                                        .map(|apps| {
                                            apps.iter().any(|app| {
                                                let name = app.as_str().unwrap_or("");
                                                Path::new(&format!("/Applications/{name}")).exists()
                                                    || Path::new(&format!(
                                                        "{home}/Applications/{name}"
                                                    ))
                                                    .exists()
                                                    || Path::new(&format!(
                                                        "/Applications/Utilities/{name}"
                                                    ))
                                                    .exists()
                                            })
                                        })
                                        .unwrap_or(false)
                                });

                                if app_on_disk {
                                    Some(token)
                                } else {
                                    None
                                }
                            })
                            .collect();
                    }
                }
            }
        }

        // Fallback: plain list (no filesystem check)
        Command::new("brew")
            .args(["list", "--cask"])
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    })
    .await
    .unwrap_or_default()
}

#[tauri::command]
async fn install_brew_cask(token: String) -> Result<(), String> {
    let result = tauri::async_runtime::spawn_blocking(move || {
        Command::new("brew")
            .args(["reinstall", "--cask", &token])
            .output()
            .map_err(|e| format!("Impossible d'exécuter brew: {}", e))
    })
    .await
    .map_err(|e| format!("Erreur tâche: {}", e))??;
    if result.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        Err(stderr.trim().to_string())
    }
}

// ── Universal Binaries ────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct UniversalBinaryEntry {
    pub name: String,
    pub path: String,
    pub total_size_bytes: u64,
    pub reclaimable_bytes: u64,
    // Always true — codesign --deep doesn't descend into Contents/Resources,
    // leaving nested Mach-O files with mismatched Team IDs → crash at spawn.
    pub thinning_unsafe: bool,
    pub thinning_warning: String,
}

fn is_universal_binary(path: &Path) -> bool {
    let Ok(out) = Command::new("file").arg(path).output() else {
        return false;
    };
    let s = String::from_utf8_lossy(&out.stdout);
    s.contains("x86_64") && s.contains("arm64")
}

fn app_bundle_id(app_path: &Path) -> String {
    let info = app_path.join("Contents/Info.plist");
    Command::new("/usr/libexec/PlistBuddy")
        .args([
            "-c",
            "Print CFBundleIdentifier",
            info.to_str().unwrap_or(""),
        ])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn scan_app_fatbinaries(app_path: &Path) -> Vec<UniversalBinaryEntry> {
    let mut results = Vec::new();
    let macos_dir = app_path.join("Contents/MacOS");
    let Ok(entries) = fs::read_dir(&macos_dir) else {
        return results;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() && is_universal_binary(&p) {
            let total = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            results.push(UniversalBinaryEntry {
                name: p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string(),
                path: p.to_string_lossy().to_string(),
                total_size_bytes: total,
                reclaimable_bytes: total / 2,
                thinning_unsafe: true,
                thinning_warning: "L'amincissement peut corrompre la signature de l'app (codesign --deep ne descend pas dans Contents/Resources). Désactivé par sécurité.".to_string(),
            });
        }
    }
    results
}

#[tauri::command]
async fn scan_universal_binaries() -> Vec<UniversalBinaryEntry> {
    tauri::async_runtime::spawn_blocking(|| {
        let mut results = Vec::new();
        let apps_dir = Path::new("/Applications");
        let Ok(entries) = fs::read_dir(apps_dir) else {
            return results;
        };
        for entry in entries.flatten() {
            let app_path = entry.path();
            if app_path.extension().and_then(|e| e.to_str()) != Some("app") {
                continue;
            }
            if app_bundle_id(&app_path).starts_with("com.apple.") {
                continue;
            }
            results.extend(scan_app_fatbinaries(&app_path));
        }
        results.sort_by_key(|k| std::cmp::Reverse(k.reclaimable_bytes));
        results
    })
    .await
    .unwrap_or_default()
}

#[tauri::command]
async fn thin_binary(binary_path: String) -> Result<u64, String> {
    // Validation : uniquement les binaires dans /Applications
    guard::validate_thin_binary_path(&binary_path)?;
    tauri::async_runtime::spawn_blocking(move || {
        let original_size = fs::metadata(&binary_path).map(|m| m.len()).unwrap_or(0);
        let tmp_path = format!("{}.burrow_thin", binary_path);
        // lipo utilise Command::arg() — aucune interpolation shell
        let out = Command::new("lipo")
            .args(["-remove", "x86_64", &binary_path, "-output", &tmp_path])
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            let _ = fs::remove_file(&tmp_path);
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        if fs::rename(&tmp_path, &binary_path).is_ok() {
            let new_size = fs::metadata(&binary_path).map(|m| m.len()).unwrap_or(0);
            return Ok(original_size.saturating_sub(new_size));
        }
        // Fallback admin copy avec quoting POSIX correct
        let q_tmp = guard::posix_shell_quote(&tmp_path);
        let q_bin = guard::posix_shell_quote(&binary_path);
        let script = format!("cp {} {} && chmod 755 {}", q_tmp, q_bin, q_bin);
        run_admin_sh(&script)?;
        let _ = fs::remove_file(&tmp_path);
        let new_size = fs::metadata(&binary_path).map(|m| m.len()).unwrap_or(0);
        Ok(original_size.saturating_sub(new_size))
    })
    .await
    .map_err(|e| e.to_string())?
}

// ── Login Items / LaunchAgents ─────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct LoginItemEntry {
    pub name: String,
    pub plist_path: String,
    pub program: String,
    pub is_broken: bool,
    pub is_suspicious: bool,
    pub suspicious_reason: String,
    pub is_system: bool,  // /Library → needs admin to delete
    pub can_delete: bool, // plist is writable by current user
}

fn plistbuddy_get(plist: &str, key: &str) -> String {
    let out = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", &format!("Print {}", key), plist])
        .output()
        .unwrap_or_else(|_| std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: vec![],
            stderr: vec![],
        });
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    // Filter PlistBuddy error messages (permission denied, file not found, etc.)
    if stdout.starts_with("Print:")
        || stdout.contains("File Doesn't Exist")
        || stdout.contains("Does Not Exist")
        || !stderr.is_empty()
    {
        String::new()
    } else {
        stdout
    }
}

#[tauri::command]
fn scan_login_items() -> Vec<LoginItemEntry> {
    let home = home_dir();
    let search_dirs: Vec<(PathBuf, bool)> = vec![
        (home.join("Library/LaunchAgents"), false),
        (PathBuf::from("/Library/LaunchAgents"), true),
        (PathBuf::from("/Library/LaunchDaemons"), true),
    ];
    let suspicious_patterns = [
        "/tmp/",
        "curl ",
        "python -c",
        "python3 -c",
        "osascript",
        "base64",
        "eval ",
        "bash -c",
        "sh -c",
        "wget ",
    ];

    let mut results = Vec::new();
    for (dir, is_system) in search_dirs {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let plist_path = entry.path();
            if plist_path.extension().and_then(|e| e.to_str()) != Some("plist") {
                continue;
            }
            let plist_str = plist_path.to_string_lossy().to_string();
            let name = plist_path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            // Try Program first, then ProgramArguments:0
            let mut program = plistbuddy_get(&plist_str, "Program");
            if program.is_empty() {
                program = plistbuddy_get(&plist_str, "ProgramArguments:0");
            }

            // is_broken = we got a real path AND it doesn't exist on disk
            // (skip launchctl paths — they are system tools, always valid)
            let is_broken = !program.is_empty()
                && program != "/bin/launchctl"
                && program != "/usr/bin/launchctl"
                && !Path::new(&program).exists();

            // Suspicious heuristics on full ProgramArguments block
            let args_text = plistbuddy_get(&plist_str, "ProgramArguments");
            let full_text = format!("{} {}", program, args_text);
            let mut is_suspicious = false;
            let mut suspicious_reason = String::new();
            for pat in &suspicious_patterns {
                if full_text.contains(pat) {
                    is_suspicious = true;
                    suspicious_reason = format!("Contient «{}»", pat.trim());
                    break;
                }
            }

            // can_delete: user owns the file OR we'll use admin for system plists
            let can_delete = true; // always offer delete; admin fallback handles /Library

            results.push(LoginItemEntry {
                name,
                plist_path: plist_str,
                program,
                is_broken,
                is_suspicious,
                suspicious_reason,
                is_system,
                can_delete,
            });
        }
    }
    results.sort_by(|a, b| {
        b.is_suspicious
            .cmp(&a.is_suspicious)
            .then(b.is_broken.cmp(&a.is_broken))
    });
    results
}

#[tauri::command]
fn bundle_prefix(plist_path: &str) -> String {
    let stem = Path::new(plist_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let parts: Vec<&str> = stem.split('.').collect();
    let n = if parts.len() >= 5 {
        3
    } else {
        parts.len().saturating_sub(1).max(2).min(parts.len())
    };
    parts[..n.min(parts.len())].join(".")
}

#[tauri::command]
fn find_related_launch_items(plist_path: String) -> Vec<String> {
    let prefix = bundle_prefix(&plist_path);
    if prefix.is_empty() {
        return vec![];
    }
    let home = home_dir();
    let dirs = [
        PathBuf::from("/Library/LaunchAgents"),
        PathBuf::from("/Library/LaunchDaemons"),
        home.join("Library/LaunchAgents"),
    ];
    let mut results = Vec::new();
    for dir in &dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("plist") {
                continue;
            }
            let path_str = path.to_string_lossy().to_string();
            if path_str == plist_path {
                continue;
            }
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if stem.starts_with(&prefix) {
                results.push(path_str);
            }
        }
    }
    results
}

#[tauri::command]
fn delete_launch_items(plist_paths: Vec<String>) -> Result<(), String> {
    let home = home_dir();
    let home_str = home.to_string_lossy().to_string();
    let mut sys_paths: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for p in &plist_paths {
        // Valider chaque chemin contre les répertoires LaunchAgents/LaunchDaemons connus
        if let Err(e) = guard::validate_launch_item_path(p) {
            errors.push(format!("{}: {}", p, e));
            continue;
        }
        if p.starts_with(&home_str) {
            // User-level: pas besoin d'admin, Command::arg() — aucune interpolation
            let _ = Command::new("launchctl").args(["unload", "-w", p]).output();
            let _ = fs::remove_file(p);
        } else {
            sys_paths.push(p.clone());
        }
    }

    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    if sys_paths.is_empty() {
        return Ok(());
    }

    // Construction du script admin avec quoting POSIX correct (guard::posix_shell_quote)
    let sys_cmds: Vec<String> = sys_paths
        .iter()
        .map(|p| {
            let q = guard::posix_shell_quote(p);
            format!("launchctl unload -w {} 2>/dev/null; rm -f {}", q, q)
        })
        .collect();
    run_admin_sh(&sys_cmds.join("; "))
}

#[allow(dead_code)]
fn delete_login_item(plist_path: String) -> Result<(), String> {
    delete_launch_items(vec![plist_path])
}

#[tauri::command]
fn toggle_login_item(plist_path: String, enable: bool) -> Result<(), String> {
    guard::validate_launch_item_path(&plist_path)?;
    let action = if enable { "load" } else { "unload" };
    // Command::arg() — aucune interpolation shell
    let out = Command::new("launchctl")
        .args([action, "-w", &plist_path])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    // Fallback admin avec quoting POSIX correct
    let q = guard::posix_shell_quote(&plist_path);
    run_admin_sh(&format!("launchctl {} -w {}", action, q))
}

// ── Deleted Users ──────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct DeletedUserEntry {
    pub username: String,
    pub home_path: String,
    pub size_bytes: u64,
}

#[tauri::command]
fn scan_deleted_users() -> Vec<DeletedUserEntry> {
    let dscl_out = Command::new("dscl")
        .args([".", "list", "/Users"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let active: std::collections::HashSet<String> = dscl_out
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.starts_with('_') && !l.is_empty())
        .collect();

    let Ok(entries) = fs::read_dir("/Users") else {
        return Vec::new();
    };
    let mut results = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let username = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        if username.starts_with('.') || username == "Shared" {
            continue;
        }
        if active.contains(&username) {
            continue;
        }
        let size_bytes = du_mb(&path).saturating_mul(1024 * 1024);
        results.push(DeletedUserEntry {
            username,
            home_path: path.to_string_lossy().to_string(),
            size_bytes,
        });
    }
    results
}

// ── Privacy Cleaner ────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct PrivacyItem {
    pub id: String,
    pub label: String,
    pub path: String,
    pub size_bytes: u64,
}

fn resolve_privacy_items(home: &Path) -> Vec<(String, String, PathBuf)> {
    // (id, label, resolved_path) — only entries where path exists
    let candidates: &[(&str, &str, &[&str])] = &[
        // ── Safari ──────────────────────────────────────────────────────────
        (
            "safari_history",
            "Safari — Historique",
            &["Library/Safari/History.db"],
        ),
        (
            "safari_cache",
            "Safari — Cache",
            &["Library/Caches/com.apple.Safari"],
        ),
        // ── Chrome ──────────────────────────────────────────────────────────
        (
            "chrome_cache",
            "Chrome — Cache",
            &[
                "Library/Caches/Google/Chrome",
                "Library/Caches/com.google.Chrome",
                "Library/Application Support/Google/Chrome/Default/Cache",
            ],
        ),
        (
            "chrome_history",
            "Chrome — Historique",
            &["Library/Application Support/Google/Chrome/Default/History"],
        ),
        // ── Firefox ─────────────────────────────────────────────────────────
        (
            "firefox_cache",
            "Firefox — Cache",
            &[
                "Library/Caches/Firefox",
                "Library/Caches/org.mozilla.firefox",
            ],
        ),
        // ── LibreWolf ───────────────────────────────────────────────────────
        (
            "librewolf_cache",
            "LibreWolf — Cache",
            &[
                "Library/Caches/io.gitlab.librewolf-community",
                "Library/Application Support/librewolf/Profiles",
            ],
        ),
        // ── Waterfox ────────────────────────────────────────────────────────
        (
            "waterfox_cache",
            "Waterfox — Cache",
            &[
                "Library/Caches/net.waterfox.waterfox",
                "Library/Application Support/Waterfox/Profiles",
            ],
        ),
        // ── Floorp ──────────────────────────────────────────────────────────
        (
            "floorp_cache",
            "Floorp — Cache",
            &[
                "Library/Caches/org.mozilla.floorp",
                "Library/Application Support/Floorp/Profiles",
            ],
        ),
        // ── Mullvad Browser ─────────────────────────────────────────────────
        (
            "mullvad_cache",
            "Mullvad Browser — Cache",
            &[
                "Library/Caches/net.mullvad.MullvadBrowser",
                "Library/Application Support/MullvadBrowser/Profiles",
            ],
        ),
        // ── Tor Browser ─────────────────────────────────────────────────────
        (
            "tor_cache",
            "Tor Browser — Données",
            &["Library/Application Support/TorBrowser-Data"],
        ),
        // ── Brave ───────────────────────────────────────────────────────────
        (
            "brave_cache",
            "Brave — Cache",
            &[
                "Library/Caches/BraveSoftware/Brave-Browser",
                "Library/Caches/com.brave.Browser",
                "Library/Application Support/BraveSoftware/Brave-Browser/Default/Cache",
            ],
        ),
        // ── Edge ────────────────────────────────────────────────────────────
        (
            "edge_cache",
            "Edge — Cache",
            &[
                "Library/Caches/com.microsoft.edgemac",
                "Library/Caches/Microsoft Edge",
                "Library/Application Support/Microsoft Edge/Default/Cache",
            ],
        ),
        // ── Arc ─────────────────────────────────────────────────────────────
        (
            "arc_cache",
            "Arc — Cache",
            &["Library/Caches/company.thebrowser.Browser"],
        ),
        // ── Opera ───────────────────────────────────────────────────────────
        (
            "opera_cache",
            "Opera — Cache",
            &[
                "Library/Caches/com.operasoftware.Opera",
                "Library/Application Support/com.operasoftware.Opera/Default/Cache",
            ],
        ),
        // ── Opera GX ────────────────────────────────────────────────────────
        (
            "operagx_cache",
            "Opera GX — Cache",
            &[
                "Library/Caches/com.operasoftware.OperaGX",
                "Library/Application Support/com.operasoftware.OperaGX/Default/Cache",
            ],
        ),
        // ── Vivaldi ─────────────────────────────────────────────────────────
        (
            "vivaldi_cache",
            "Vivaldi — Cache",
            &[
                "Library/Caches/com.vivaldi.Vivaldi",
                "Library/Application Support/Vivaldi/Default/Cache",
            ],
        ),
        // ── Chromium ────────────────────────────────────────────────────────
        (
            "chromium_cache",
            "Chromium — Cache",
            &[
                "Library/Caches/org.chromium.Chromium",
                "Library/Application Support/Chromium/Default/Cache",
            ],
        ),
        // ── Orion (Kagi) ────────────────────────────────────────────────────
        (
            "orion_cache",
            "Orion — Cache",
            &[
                "Library/Caches/com.kagi.kagimacOS",
                "Library/Application Support/Orion",
            ],
        ),
        // ── Zen ─────────────────────────────────────────────────────────────
        (
            "zen_cache",
            "Zen Browser — Cache",
            &[
                "Library/Caches/app.zen-browser.zen",
                "Library/Application Support/Zen/Profiles",
            ],
        ),
        // ── macOS Recent Items ───────────────────────────────────────────────
        (
            "recent_items",
            "macOS — Éléments récents",
            &["Library/Application Support/com.apple.sharedfilelist"],
        ),
    ];

    let mut results = Vec::new();
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (id, label, paths) in candidates {
        if seen_ids.contains(*id) {
            continue;
        }
        for rel in *paths {
            let p = home.join(rel);
            if p.exists() {
                results.push((id.to_string(), label.to_string(), p));
                seen_ids.insert(id.to_string());
                break;
            }
        }
    }
    results
}

#[tauri::command]
fn scan_privacy_items() -> Vec<PrivacyItem> {
    let home = home_dir();
    resolve_privacy_items(&home)
        .into_iter()
        .filter_map(|(id, label, p)| {
            let size_bytes = if p.is_dir() {
                du_mb(&p).saturating_mul(1024 * 1024)
            } else {
                fs::metadata(&p).map(|m| m.len()).unwrap_or(0)
            };
            if size_bytes == 0 {
                return None;
            }
            Some(PrivacyItem {
                id,
                label,
                path: p.to_string_lossy().to_string(),
                size_bytes,
            })
        })
        .collect()
}

#[tauri::command]
fn clean_privacy_items(ids: Vec<String>) -> Result<u64, String> {
    let home = home_dir();
    let mut freed: u64 = 0;
    for (id, _, p) in resolve_privacy_items(&home) {
        if !ids.iter().any(|x| x == &id) {
            continue;
        }
        if !p.exists() {
            continue;
        }
        let size = if p.is_dir() {
            du_mb(&p).saturating_mul(1024 * 1024)
        } else {
            fs::metadata(&p).map(|m| m.len()).unwrap_or(0)
        };
        let ok = if p.is_dir() {
            fs::remove_dir_all(&p).is_ok()
        } else {
            fs::remove_file(&p).is_ok()
        };
        if ok {
            freed += size;
        }
    }
    Ok(freed)
}

// ── Shell quoting helpers ─────────────────────────────────────────────────────

/// Encapsule une valeur dans des guillemets simples POSIX (pour shell).
/// Délègue à guard::posix_shell_quote qui est testé exhaustivement.
fn posix_shell_quote_value(s: &str) -> String {
    guard::posix_shell_quote(s)
}

/// Encapsule une chaîne shell dans une string AppleScript avec guillemets doubles.
/// AppleScript utilise `"..."` et `\"` pour les guillemets doubles.
/// DOIT être utilisé uniquement avec des valeurs déjà shell-quotées (pas d'entrée frontend brute).
fn posix_applescript_string(shell_cmd: &str) -> String {
    // Remplacer " par \" pour AppleScript, et \ par \\ pour AppleScript
    let escaped = shell_cmd.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{}\"", escaped)
}

// ── Admin helper : sudo (Touch ID via pam_tid.so) → fallback osascript ───────

fn run_admin_sh(script: &str) -> Result<(), String> {
    // -n : non-interactif — réussit uniquement si Touch ID / sudo_local est configuré,
    // échoue immédiatement sinon (pas de prompt terminal depuis une app GUI)
    let sudo_ok = Command::new("sudo")
        .args(["-n", "sh", "-c", script])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if sudo_ok {
        return Ok(());
    }

    // Fallback : dialog mot de passe via osascript
    let esc = script.replace('\\', "\\\\").replace('"', "\\\"");
    let applescript = format!("do shell script \"{}\" with administrator privileges", esc);
    let output = Command::new("osascript")
        .args(["-e", &applescript])
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[tauri::command]
fn setup_pam_touchid() -> Result<(), String> {
    // Configure /etc/pam.d/sudo_local pour activer Touch ID avec sudo (macOS Ventura+)
    let pam_local = "/etc/pam.d/sudo_local";
    let pam_tid_line = "auth       sufficient     pam_tid.so";

    // Vérifier si pam_tid.so est déjà présent dans le fichier — no-op si c'est le cas
    if std::path::Path::new(pam_local).exists() {
        let content = std::fs::read_to_string(pam_local).unwrap_or_default();
        if content.lines().any(|l| {
            let l = l.trim();
            !l.starts_with('#') && l.contains("pam_tid.so")
        }) {
            return Ok(());
        }
        // Fichier existant sans pam_tid.so : préfixer pour que la règle soit évaluée en premier
        // (auth sufficient arrête l'évaluation si Touch ID réussit)
        let new_content = format!(
            "# Burrow : Touch ID pour sudo (ajouté automatiquement)\n{}\n{}",
            pam_tid_line, content
        );
        // Écriture atomique via fichier temporaire dans /etc/pam.d
        let tmp = "/etc/pam.d/sudo_local.burrow_tmp";
        let shell_cmd = format!(
            "printf %s {} > {} && mv {} {}",
            guard::posix_shell_quote(&new_content),
            tmp,
            tmp,
            pam_local
        );
        let applescript = format!(
            "do shell script {} with administrator privileges",
            posix_applescript_string(&shell_cmd)
        );
        let out = Command::new("osascript")
            .args(["-e", &applescript])
            .output()
            .map_err(|e| e.to_string())?;
        return if out.status.success() {
            Ok(())
        } else {
            Err("Échec de la mise à jour de /etc/pam.d/sudo_local".to_string())
        };
    }

    // Fichier inexistant : créer avec le contenu minimal recommandé
    let new_content = format!(
        "# sudo_local: local config file which survives system update\n# Burrow : Touch ID pour sudo\n{}\n",
        pam_tid_line
    );
    let shell_cmd = format!(
        "printf %s {} > {}",
        guard::posix_shell_quote(&new_content),
        pam_local
    );
    let applescript = format!(
        "do shell script {} with administrator privileges",
        posix_applescript_string(&shell_cmd)
    );
    let output = Command::new("osascript")
        .args(["-e", &applescript])
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err("Échec de la configuration PAM Touch ID — mot de passe requis".to_string())
    }
}

// ── Touch ID (LocalAuthentication via ObjC helper) ──────────────────────────
//
// TODO(P0.5): Ce helper est compilé depuis une source ObjC à l'exécution (nécessite clang/Xcode CLT).
// La solution propre est de pré-compiler, signer et embarquer le binaire au build.
// En attendant, on utilise un répertoire temporaire aléatoire (RAII via TempDir).
// Le binaire est recréé à chaque lancement (le TempDir est supprimé à la destruction).

static TOUCHID_DIR: OnceLock<Mutex<Option<std::path::PathBuf>>> = OnceLock::new();

fn touchid_dir() -> &'static Mutex<Option<std::path::PathBuf>> {
    TOUCHID_DIR.get_or_init(|| Mutex::new(None))
}

fn get_or_build_touchid_binary() -> Option<std::path::PathBuf> {
    // Double-checked lock : on compile une seule fois par processus.
    {
        let guard = touchid_dir().lock().unwrap();
        if let Some(ref dir) = *guard {
            let bin = dir.join("touchid");
            if bin.exists() {
                return Some(bin);
            }
        }
    }

    // Créer un dossier temporaire privé pour la compilation
    let tmp_dir = burrow_tempdir().ok()?;
    let src_path = tmp_dir.path().join("touchid.m");
    let bin_path = tmp_dir.path().join("touchid");

    let src = r#"#import <Foundation/Foundation.h>
#import <LocalAuthentication/LocalAuthentication.h>

int main(int argc, char *argv[]) {
    @autoreleasepool {
        BOOL check_only = (argc > 1 && strcmp(argv[1], "--check") == 0);
        NSString *reason = (!check_only && argc > 1)
            ? [NSString stringWithUTF8String:argv[1]]
            : @"Burrow demande votre autorisation";
        LAContext *ctx = [[LAContext alloc] init];
        NSError *err = nil;
        if (![ctx canEvaluatePolicy:LAPolicyDeviceOwnerAuthenticationWithBiometrics error:&err]) {
            return 2; /* Touch ID indisponible */
        }
        if (check_only) { return 0; }
        dispatch_semaphore_t sema = dispatch_semaphore_create(0);
        __block BOOL ok = NO;
        [ctx evaluatePolicy:LAPolicyDeviceOwnerAuthenticationWithBiometrics
            localizedReason:reason
                      reply:^(BOOL result, NSError *e) {
            ok = result;
            dispatch_semaphore_signal(sema);
        }];
        dispatch_semaphore_wait(sema, DISPATCH_TIME_FOREVER);
        return ok ? 0 : 1;
    }
}
"#;
    std::fs::write(&src_path, src).ok()?;
    let status = Command::new("clang")
        .args([
            "-framework",
            "Foundation",
            "-framework",
            "LocalAuthentication",
            "-fobjc-arc",
        ])
        .arg(&src_path)
        .arg("-o")
        .arg(&bin_path)
        .status()
        .ok()?;
    let _ = std::fs::remove_file(&src_path);
    if !status.success() {
        return None;
    }

    // Mémoriser le chemin du binaire; conserver le TempDir via son chemin
    // (on convertit en PathBuf pour éviter de dropper tmp_dir)
    let dir_path = tmp_dir.keep(); // keep() → ne supprime plus le dossier à la destruction
    *touchid_dir().lock().unwrap() = Some(dir_path.clone());
    Some(dir_path.join("touchid"))
}

#[tauri::command]
fn check_touch_id_available() -> bool {
    let Some(bin) = get_or_build_touchid_binary() else {
        return false;
    };
    Command::new(bin)
        .arg("--check")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[tauri::command]
async fn authenticate_touch_id(reason: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let bin = get_or_build_touchid_binary().ok_or_else(|| {
            "Impossible de préparer l'outil Touch ID (Xcode CLT requis)".to_string()
        })?;
        let output = Command::new(bin)
            .arg(&reason)
            .output()
            .map_err(|e| format!("Erreur d'exécution : {}", e))?;
        match output.status.code() {
            Some(0) => Ok(()),
            Some(2) => Err("Touch ID non disponible sur cet appareil".to_string()),
            _ => Err("Authentification refusée ou annulée".to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

// ── Lancer au démarrage (LaunchAgent) ───────────────────────────────────────

#[tauri::command]
fn get_launch_at_login() -> bool {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::Path::new(&format!(
        "{}/Library/LaunchAgents/com.burrow.app.plist",
        home
    ))
    .exists()
}

#[tauri::command]
fn set_launch_at_login(enable: bool) -> Result<(), String> {
    let home = std::env::var("HOME").map_err(|_| "HOME non défini".to_string())?;
    let plist_path = format!("{}/Library/LaunchAgents/com.burrow.app.plist", home);
    if enable {
        let exe = std::env::current_exe().map_err(|e| format!("Chemin introuvable : {}", e))?;
        let exe_str = exe.to_string_lossy();
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
             \t<key>Label</key>\n\
             \t<string>com.burrow.app</string>\n\
             \t<key>ProgramArguments</key>\n\
             \t<array>\n\
             \t\t<string>{exe_str}</string>\n\
             \t</array>\n\
             \t<key>RunAtLoad</key>\n\
             \t<true/>\n\
             \t<key>KeepAlive</key>\n\
             \t<false/>\n\
             \t<key>LimitLoadToSessionType</key>\n\
             \t<string>Aqua</string>\n\
             </dict>\n\
             </plist>"
        );
        std::fs::write(&plist_path, plist)
            .map_err(|e| format!("Écriture plist échouée : {}", e))?;
        let _ = Command::new("launchctl")
            .args(["load", "-w", &plist_path])
            .output();
    } else {
        if std::path::Path::new(&plist_path).exists() {
            let _ = Command::new("launchctl")
                .args(["unload", "-w", &plist_path])
                .output();
            std::fs::remove_file(&plist_path)
                .map_err(|e| format!("Suppression plist échouée : {}", e))?;
        }
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            start_sysinfo_daemon();
            start_process_daemon(app.handle().clone());
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut g = metrics_cache().lock().unwrap();
                if !g.is_refreshing {
                    g.is_refreshing = true;
                    drop(g);
                    let result = do_fetch_metrics(&app_handle);
                    let mut g2 = metrics_cache().lock().unwrap();
                    if let Ok(m) = result { g2.data = Some(m); }
                    g2.is_refreshing = false;
                }
            });
            std::thread::spawn(|| { get_all_app_icons(); });
            std::thread::spawn(|| {
                cask_api();           // OnceLock → une seule initialisation
                get_brew_outdated();  // pré-chauffe le cache brew cask
                check_mo_update_available(); // pré-chauffe le cache mole update
            });

            // ── Menu bar (tray icon) ──────────────────────────────────────────
            let mut tray_builder = tauri::tray::TrayIconBuilder::new();
            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }
            let tray = tray_builder
                .title(" 💾")
                .tooltip("Burrow — Cliquer pour ouvrir")
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left, ..
                    } = event {
                        if let Some(w) = tray.app_handle().get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                })
                .build(app)?;

            // ── Background disk monitor: update tray + notifications ──────────
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(300));
                    use sysinfo::Disks;
                    let disks = Disks::new_with_refreshed_list();
                    let (used, total) = disks.list().iter()
                        .find(|d| d.mount_point() == std::path::Path::new("/"))
                        .map(|d| (d.total_space() - d.available_space(), d.total_space()))
                        .unwrap_or((0, 1));
                    if total == 0 { continue; }
                    let pct = ((used * 100) / total) as u8;
                    let label = if pct >= 90 { format!(" 🔴 {}%", pct) }
                        else if pct >= 80 { format!(" ⚠️ {}%", pct) }
                        else { format!(" 💾 {}%", pct) };
                    let _ = tray.set_title(Some(&label));

                    let last = LAST_NOTIF_PCT.load(Ordering::Relaxed);
                    if pct >= 90 && last < 90 {
                        let _ = Command::new("osascript")
                            .args(["-e", "display notification \"Votre disque est rempli à plus de 90 %. Nettoyez maintenant.\" with title \"Burrow\""])
                            .output();
                        LAST_NOTIF_PCT.store(90, Ordering::Relaxed);
                    } else if pct >= 80 && last < 80 {
                        let _ = Command::new("osascript")
                            .args(["-e", "display notification \"Votre disque est rempli à 80 %.\" with title \"Burrow\""])
                            .output();
                        LAST_NOTIF_PCT.store(80, Ordering::Relaxed);
                    }

                    save_disk_sample_if_needed(used, total);
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_apps,
            get_app_size,
            get_app_icon,
            get_all_app_icons,
            get_mo_path,
            uninstall_app,
            get_system_metrics,
            list_installer_files,
            move_to_trash,
            check_full_disk_access,
            open_full_disk_access_settings,
            get_clean_sizes,
            run_clean_selection,
            run_optimize_selection,
            get_net_rates,
            free_memory,
            get_low_power_mode,
            set_low_power_mode,
            get_mo_version,
            update_mo_cli,
            get_brew_outdated,
            update_brew_app,
            get_folder_top_files,
            find_app_residuals,
            delete_path,
            get_all_processes,
            kill_process,
            gpu::get_gpu_info,
            check_sparkle_updates,
            check_app_store_updates,
            update_mas_app,
            install_sparkle_update,
            check_clamav,
            start_clamav_scan,
            cancel_clamav_scan,
            update_clamav_defs,
            list_quarantine,
            quarantine_file,
            restore_from_quarantine,
            delete_from_quarantine,
            pick_folder,
            check_mo_update_available,
            check_clamav_defs_outdated,
            list_volumes,
            get_quick_metrics,
            set_fan_mode,
            check_system_permissions,
            setup_system_permissions,
            get_disk_categories,
            get_dev_caches,
            get_project_artifacts,
            get_large_files,
            empty_trash,
            get_derived_data_projects,
            is_xcode_running,
            get_disk_forecast,
            get_brew_formula_outdated,
            update_brew_formula,
            duplicates::find_duplicates,
            get_disk_breakdown,
            get_home_dir,
            read_image_preview,
            list_network_services,
            set_dns_servers,
            reset_dns,
            get_search_domains,
            set_search_domains,
            install_doh_profile,
            get_installed_casks,
            install_brew_cask,
            setup_pam_touchid,
            check_touch_id_available,
            authenticate_touch_id,
            get_launch_at_login,
            set_launch_at_login,
            scan_universal_binaries,
            thin_binary,
            scan_login_items,
            scan_deleted_users,
            scan_privacy_items,
            clean_privacy_items,
            toggle_login_item,
            find_related_launch_items,
            delete_launch_items,
            get_purgeable_space,
            purge_purgeable_space,
            scan_tm_snapshots,
            delete_tm_snapshot,
            scan_simulator_runtimes,
            delete_simulator_runtime,
            scan_ai_caches,
            clean_ai_caches,
            scan_dev_caches,
            clean_dev_caches,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
