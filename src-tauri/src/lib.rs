mod activity;
pub mod duplicates;
pub mod gpu;
mod guard;
mod ior;

extern crate libc;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

fn show_main_window(app: &tauri::AppHandle, page: Option<&str>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        if let Some(page) = page {
            let _ = window.emit("navigate", page);
        }
    }
}

// ── ClamAV child-process store ────────────────────────────────────────────────

// Stores the running ClamAV child so that cancel_clamav_scan can kill it by
// handle rather than by PID, eliminating the PID-reuse race.
static SCAN_CHILD: OnceLock<Mutex<Option<std::process::Child>>> = OnceLock::new();
fn scan_child_store() -> &'static Mutex<Option<std::process::Child>> {
    SCAN_CHILD.get_or_init(|| Mutex::new(None))
}

static SCAN_ACTIVE: AtomicBool = AtomicBool::new(false);
static SCAN_CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);

// ── Disk-browse concurrency guard ─────────────────────────────────────────────

// Prevents a compromised frontend from launching unbounded concurrent du(1)
// processes. Stores true while a get_disk_breakdown call is in progress.
static DISK_BROWSE_ACTIVE: AtomicBool = AtomicBool::new(false);

struct ActivityGuard(&'static AtomicBool);

impl ActivityGuard {
    fn try_acquire(flag: &'static AtomicBool) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(flag))
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
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

#[derive(Clone, Copy)]
pub(crate) enum PathGrantPurpose {
    Trash = 1,
    Quarantine = 2,
    LaunchItem = 4,
    Uninstall = 8,
    Thin = 16,
}

struct PathGrant {
    device: u64,
    inode: u64,
    purposes: u8,
    issued_at: Instant,
}

fn path_grants() -> &'static Mutex<HashMap<PathBuf, PathGrant>> {
    static GRANTS: OnceLock<Mutex<HashMap<PathBuf, PathGrant>>> = OnceLock::new();
    GRANTS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn grant_path(path: &Path, purpose: PathGrantPurpose) {
    use std::os::unix::fs::MetadataExt;

    let Ok(canonical) = fs::canonicalize(path) else {
        return;
    };
    let Ok(metadata) = fs::symlink_metadata(&canonical) else {
        return;
    };
    if metadata.file_type().is_symlink() {
        return;
    }
    if let Ok(mut grants) = path_grants().lock() {
        grants.retain(|_, grant| grant.issued_at.elapsed() < Duration::from_secs(30 * 60));
        grants
            .entry(canonical)
            .and_modify(|grant| {
                grant.device = metadata.dev();
                grant.inode = metadata.ino();
                grant.purposes |= purpose as u8;
                grant.issued_at = Instant::now();
            })
            .or_insert(PathGrant {
                device: metadata.dev(),
                inode: metadata.ino(),
                purposes: purpose as u8,
                issued_at: Instant::now(),
            });
    }
}

fn require_path_grant(path: &str, purpose: PathGrantPurpose) -> Result<PathBuf, String> {
    use std::os::unix::fs::MetadataExt;

    let canonical =
        fs::canonicalize(path).map_err(|e| format!("Chemin autorisé devenu inaccessible : {e}"))?;
    let metadata = fs::symlink_metadata(&canonical).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() {
        return Err("Lien symbolique refusé".to_string());
    }
    let grants = path_grants()
        .lock()
        .map_err(|_| "Registre d'autorisations indisponible".to_string())?;
    let grant = grants
        .get(&canonical)
        .filter(|grant| grant.issued_at.elapsed() < Duration::from_secs(30 * 60))
        .filter(|grant| grant.purposes & purpose as u8 != 0)
        .filter(|grant| grant.device == metadata.dev() && grant.inode == metadata.ino())
        .ok_or_else(|| "Chemin non autorisé : relancez l'analyse avant cette action".to_string())?;
    let _ = grant;
    Ok(canonical)
}

struct MolePreviewGrant {
    name: String,
    bundle_id: String,
    paths: Vec<PathBuf>,
    issued_at: Instant,
}

fn mole_preview_grants() -> &'static Mutex<HashMap<PathBuf, MolePreviewGrant>> {
    static GRANTS: OnceLock<Mutex<HashMap<PathBuf, MolePreviewGrant>>> = OnceLock::new();
    GRANTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn grant_mole_preview(path: &Path, name: &str, bundle_id: &str, paths: &[PathBuf]) {
    if let Ok(mut grants) = mole_preview_grants().lock() {
        grants.retain(|_, grant| grant.issued_at.elapsed() < Duration::from_secs(10 * 60));
        grants.insert(
            path.to_path_buf(),
            MolePreviewGrant {
                name: name.to_string(),
                bundle_id: bundle_id.to_string(),
                paths: paths.to_vec(),
                issued_at: Instant::now(),
            },
        );
    }
}

fn consume_mole_preview(path: &Path, name: &str, bundle_id: &str) -> Result<Vec<PathBuf>, String> {
    let mut grants = mole_preview_grants()
        .lock()
        .map_err(|_| "Registre d'aperçus Mole indisponible".to_string())?;
    grants.retain(|_, grant| grant.issued_at.elapsed() < Duration::from_secs(10 * 60));
    let valid = grants
        .get(path)
        .filter(|grant| grant.name == name && grant.bundle_id == bundle_id)
        .is_some();
    if !valid {
        return Err(
            "Aperçu Mole expiré : ouvrez à nouveau l'application avant de la désinstaller"
                .to_string(),
        );
    }
    Ok(grants
        .remove(path)
        .expect("validated Mole preview grant")
        .paths)
}

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

fn current_user_record() -> Result<(String, PathBuf), String> {
    let uid = unsafe { libc::getuid() };
    let mut buffer_size = 4096usize;

    loop {
        let mut pwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut buffer = vec![0i8; buffer_size];
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let error = unsafe {
            libc::getpwuid_r(
                uid,
                pwd.as_mut_ptr(),
                buffer.as_mut_ptr(),
                buffer.len(),
                &mut result,
            )
        };
        if error == libc::ERANGE && buffer_size < 1_048_576 {
            buffer_size *= 2;
            continue;
        }
        if error != 0 || result.is_null() {
            return Err(format!("getpwuid_r failed (errno {error})"));
        }

        let pwd = unsafe { pwd.assume_init() };
        let name = unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) }
            .to_str()
            .map_err(|_| "Nom d'utilisateur non UTF-8".to_string())?
            .to_string();
        let home = unsafe { std::ffi::CStr::from_ptr(pwd.pw_dir) }
            .to_str()
            .map_err(|_| "Répertoire utilisateur non UTF-8".to_string())
            .map(PathBuf::from)?;

        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            || !home.is_absolute()
        {
            return Err("Compte utilisateur POSIX invalide".to_string());
        }
        return Ok((name, home));
    }
}

pub(crate) fn home_dir() -> PathBuf {
    current_user_record()
        .map(|(_, home)| home)
        .unwrap_or_else(|_| PathBuf::from("/nonexistent"))
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
    for path in ["/opt/homebrew/bin/mo", "/usr/local/bin/mo"] {
        if Path::new(path).is_file() {
            return Ok(path.to_string());
        }
    }
    Err("mo not found".to_string())
}

const MOLE_RESTRICTED_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

fn bundled_mo_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|error| error.to_string())?;
    let mole_root = resource_dir.join("mole");
    let mo_path = mole_root.join("bin").join("mo");
    let canonical_root = fs::canonicalize(&mole_root)
        .map_err(|error| format!("Ressources Mole introuvables : {error}"))?;
    let canonical_mo = fs::canonicalize(&mo_path)
        .map_err(|error| format!("CLI Mole embarqué introuvable : {error}"))?;
    if !canonical_mo.starts_with(&canonical_root) || !canonical_mo.is_file() {
        return Err("Chemin du CLI Mole embarqué invalide".to_string());
    }
    Ok(canonical_mo)
}

fn validate_mole_app_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 255
        || name.starts_with('-')
        || name.chars().any(|character| character.is_control())
    {
        return Err("Nom d'application incompatible avec la désinstallation Mole".to_string());
    }
    Ok(())
}

fn mole_uninstall_command(mo_path: &Path, dry_run: bool, name: &str) -> Command {
    let mut command = Command::new(mo_path);
    command
        .env("PATH", MOLE_RESTRICTED_PATH)
        // Burrow garantit une politique récupérable. Masquer Homebrew empêche
        // Mole d'emprunter `brew uninstall --zap`, qui peut supprimer des
        // fichiers au lieu de les déplacer dans la Corbeille.
        .env("MOLE_DELETE_MODE", "trash")
        // Burrow centralise l'historique dans son propre journal d'activité.
        // Mole conserve ses diagnostics ordinaires mais ne duplique pas le
        // journal détaillé des chemins manipulés.
        .env("MO_NO_OPLOG", "1")
        .env_remove("MOLE_DRY_RUN")
        .env_remove("MOLE_TEST_MODE")
        .env_remove("MOLE_TEST_NO_AUTH")
        .arg("uninstall");
    if dry_run {
        command.arg("--dry-run");
    }
    command.arg(name);
    command
}

fn run_mole_uninstall(mo_path: &Path, name: &str, dry_run: bool) -> Result<String, String> {
    validate_mole_app_name(name)?;
    let mut child = mole_uninstall_command(mo_path, dry_run, name)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("Impossible de lancer Mole : {error}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        // Le clic de confirmation Burrow + Touch ID remplacent la première
        // confirmation interactive du CLI. Mole effectue ensuite son propre
        // aperçu et accepte EOF comme validation de ce plan.
        stdin
            .write_all(b"y\n")
            .map_err(|error| format!("Impossible de confirmer Mole : {error}"))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|error| format!("Impossible d'attendre Mole : {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if output.status.success() {
        Ok(format!("{stdout}\n{stderr}"))
    } else {
        let details = [stdout.trim(), stderr.trim()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        Err(if details.is_empty() {
            format!("Mole a échoué avec le code {:?}", output.status.code())
        } else {
            details
        })
    }
}

fn strip_terminal_sequences(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for sequence_character in chars.by_ref() {
                    if ('@'..='~').contains(&sequence_character) {
                        break;
                    }
                }
            }
            continue;
        }
        if character != '\r' {
            result.push(character);
        }
    }
    result
}

fn du_mb(path: &Path) -> u64 {
    Command::new("/usr/bin/du")
        .args(["-s", "-k", "-P", "--"])
        .arg(path)
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
    Command::new("/usr/sbin/sysctl")
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
    let out = Command::new("/usr/sbin/ioreg")
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
    let out = Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .output()
        .ok();
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
    let out = Command::new("/sbin/ifconfig").output().ok();
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

/// Désinstalle une application et ses fichiers associés avec le CLI Mole
/// embarqué. Le nom et le chemin restent typés et liés à un aperçu backend.
#[tauri::command]
fn uninstall_app(app: tauri::AppHandle, name: String, app_path: String) -> Result<(), String> {
    if app_path.is_empty() {
        return Err("Chemin d'application manquant".to_string());
    }
    // Validation stricte avant de lancer le thread
    guard::validate_app_uninstall_path(&app_path)?;
    let canonical = require_path_grant(&app_path, PathGrantPurpose::Uninstall)?;
    guard::validate_app_uninstall_path(&canonical.to_string_lossy())?;
    let expected_name = canonical.file_stem().and_then(|value| value.to_str());
    if expected_name != Some(name.as_str()) {
        return Err("Le nom ne correspond pas à l'application analysée".to_string());
    }
    validate_mole_app_name(&name)?;
    let bundle_id = app_bundle_id(&canonical);
    let expected_paths = consume_mole_preview(&canonical, &name, &bundle_id)?;
    let mo_path = bundled_mo_path(&app)?;
    let home = home_dir();

    // La préférence de protection est détenue et appliquée par le backend.
    // Un frontend compromis ne peut donc pas contourner Touch ID.
    if touchid_enabled(&app) {
        run_touchid(&app, &format!("Désinstaller {name}"))?;
    }

    std::thread::spawn(move || {
        let _ = app.emit(
            "mo-output",
            format!(
                "→ Mole analyse et désinstalle {} de façon récupérable…",
                name
            ),
        );
        let size = du_bytes(&canonical);
        let fresh_preview = run_mole_uninstall(&mo_path, &name, true).and_then(|output| {
            let mut paths = parse_mole_preview_paths(&output, &home);
            paths.sort();
            if paths == expected_paths {
                Ok(())
            } else {
                Err(
                    "Le contenu associé à l'application a changé depuis l'aperçu. Ouvrez-la à nouveau et contrôlez la nouvelle liste avant de confirmer."
                        .to_string(),
                )
            }
        });
        if let Err(error) = fresh_preview {
            let message = strip_terminal_sequences(&error).trim().to_string();
            let _ = app.emit("mo-error", message.clone());
            let _ = app.emit(
                "mo-output",
                format!("✗ Aperçu Mole devenu obsolète : {message}"),
            );
            activity::record(
                "applications",
                "Désinstallation récupérable Mole",
                "error",
                &name,
                Some(size),
                true,
            );
            let _ = app.emit("mo-done", 1i32);
            return;
        }
        match run_mole_uninstall(&mo_path, &name, false) {
            Ok(output) if !canonical.exists() => {
                for line in strip_terminal_sequences(&output)
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                {
                    let _ = app.emit("mo-output", line.to_string());
                }
                let _ = app.emit(
                    "mo-output",
                    format!(
                        "✓ {} et ses fichiers associés ont été placés dans la Corbeille",
                        name
                    ),
                );
                activity::record(
                    "applications",
                    "Désinstallation récupérable",
                    "success",
                    &name,
                    Some(size),
                    true,
                );
                let _ = app.emit("mo-done", 0i32);
            }
            Ok(output) => {
                let clean_output = strip_terminal_sequences(&output);
                let message = if clean_output.trim().is_empty() {
                    "Mole n'a pas retiré l'application sélectionnée".to_string()
                } else {
                    format!(
                        "Mole n'a pas retiré l'application sélectionnée : {}",
                        clean_output.trim()
                    )
                };
                let _ = app.emit("mo-error", message.clone());
                activity::record(
                    "applications",
                    "Désinstallation récupérable Mole",
                    "error",
                    &name,
                    Some(size),
                    true,
                );
                let _ = app.emit("mo-done", 1i32);
            }
            Err(error) => {
                let message = strip_terminal_sequences(&error).trim().to_string();
                let _ = app.emit("mo-error", message.clone());
                let _ = app.emit("mo-output", format!("✗ Échec Mole : {message}"));
                activity::record(
                    "applications",
                    "Désinstallation récupérable Mole",
                    "error",
                    &name,
                    Some(size),
                    true,
                );
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
    for app in &apps {
        grant_path(Path::new(&app.path), PathGrantPurpose::Uninstall);
    }
    apps
}

#[tauri::command]
fn get_app_size(app_path: String) -> u64 {
    let Ok(canonical) = require_path_grant(&app_path, PathGrantPurpose::Uninstall) else {
        return 0;
    };
    if guard::validate_app_uninstall_path(&canonical.to_string_lossy()).is_err() {
        return 0;
    }
    du_mb(&canonical)
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
    for file in &files {
        grant_path(Path::new(&file.path), PathGrantPurpose::Trash);
    }
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
    std::fs::File::open("/Library/Application Support/com.apple.TCC/TCC.db").is_ok()
}

#[tauri::command]
fn open_full_disk_access_settings() {
    Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
        .spawn()
        .ok();
}

#[tauri::command]
fn move_to_trash(path: String) -> Result<(), String> {
    guard::validate_trash_path(&path)?;
    let path = require_path_grant(&path, PathGrantPurpose::Trash)?;
    guard::validate_trash_path(&path.to_string_lossy())?;
    let size = if path.is_dir() {
        du_bytes(&path)
    } else {
        fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0)
    };
    let result = move_path_to_trash(&path);
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("élément");
    activity::record(
        "suppression",
        "Déplacement dans la Corbeille",
        if result.is_ok() { "success" } else { "error" },
        name,
        Some(size),
        true,
    );
    result
}

/// Déplace un chemin dans la Corbeille avec l'API native de macOS. L'appel
/// provient ainsi de Burrow lui-même (TCC/FDA), sans automatiser Finder.
fn move_path_to_trash(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    use trash::macos::{DeleteMethod, TrashContextExtMacos};

    let mut context = trash::TrashContext::new();
    context.set_delete_method(DeleteMethod::NsFileManager);
    context
        .delete(path)
        .map_err(|error| format!("Impossible de déplacer l'élément dans la Corbeille : {error}"))
}

fn move_directory_children_to_trash(directory: &Path) -> Result<u64, String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.to_string()),
    };
    let mut moved = 0u64;
    let mut errors = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let size = if path.is_dir() {
            du_bytes(&path)
        } else {
            fs::metadata(&path)
                .map(|metadata| metadata.len())
                .unwrap_or(0)
        };
        match move_path_to_trash(&path) {
            Ok(()) => moved = moved.saturating_add(size),
            Err(error) => errors.push(error),
        }
    }
    if errors.is_empty() {
        Ok(moved)
    } else {
        Err(errors.join("; "))
    }
}

fn quit_application(name: &str) {
    let script = r#"on run argv
tell application (item 1 of argv) to quit
end run"#;
    let _ = Command::new("/usr/bin/osascript")
        .args(["-e", script, "--", name])
        .output();
}

// ── 1. Clean category sizes ───────────────────────────────────────────────────

#[derive(Serialize)]
pub struct CleanCategorySize {
    pub id: String,
    pub size_mb: u64,
}

#[derive(Serialize)]
pub struct SmartScanResult {
    pub clean_categories: Vec<CleanCategorySize>,
    pub safe_clean_bytes: u64,
    pub clamav_ready: bool,
    pub definitions_outdated: bool,
    pub maintenance_recommendations: Vec<String>,
    pub disk_used_percent: f64,
}

#[tauri::command]
fn run_smart_scan(app: tauri::AppHandle) -> SmartScanResult {
    let clean_categories = get_clean_sizes();
    let safe_clean_bytes = clean_categories
        .iter()
        .filter(|category| category.id != "trash" && category.id != "ios_backups")
        .map(|category| category.size_mb.saturating_mul(1024 * 1024))
        .sum();
    let clamav = check_clamav(app.clone());
    let metrics = get_quick_metrics();
    let mut maintenance_recommendations = vec!["dns".to_string(), "diskutil_verify".to_string()];
    if metrics.mem_used_percent >= 80.0 {
        maintenance_recommendations.push("swap".to_string());
    }
    activity::record(
        "smart scan",
        "Diagnostic combiné",
        "success",
        &format!(
            "{} récupérables · ClamAV {} · {} recommandations",
            safe_clean_bytes,
            if clamav.installed && clamav.has_database {
                "prêt"
            } else {
                "indisponible"
            },
            maintenance_recommendations.len()
        ),
        Some(safe_clean_bytes),
        false,
    );
    SmartScanResult {
        clean_categories,
        safe_clean_bytes,
        clamav_ready: clamav.installed && clamav.has_database,
        definitions_outdated: check_clamav_defs_outdated(app),
        maintenance_recommendations,
        disk_used_percent: metrics.disk_used_percent,
    }
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
            let (label, result): (&str, Result<u64, String>) = match cat_id.as_str() {
                "user_cache" => (
                    "Cache utilisateur",
                    move_directory_children_to_trash(&home.join("Library/Caches")),
                ),
                "system_logs" => (
                    "Logs système",
                    move_directory_children_to_trash(&home.join("Library/Logs")),
                ),
                "crash_reports" => (
                    "Rapports de crash",
                    move_directory_children_to_trash(
                        &home.join("Library/Logs/DiagnosticReports"),
                    ),
                ),
                "npm_cache" => {
                    let mut moved = 0u64;
                    let result = [home.join(".npm/cache"), home.join(".npm/_logs")]
                        .iter()
                        .try_for_each(|path| {
                            let size = du_bytes(path);
                            move_path_to_trash(path)?;
                            moved = moved.saturating_add(size);
                            Ok::<(), String>(())
                        })
                        .map(|_| moved);
                    ("Cache npm", result)
                }
                "yarn_cache" => {
                    let path = home.join(".yarn/cache");
                    let size = du_bytes(&path);
                    ("Cache yarn", move_path_to_trash(&path).map(|_| size))
                }
                "browser_cache" => {
                    let paths = [
                        home.join("Library/Caches/com.apple.Safari"),
                        home.join("Library/Application Support/Google/Chrome/Default/Cache"),
                        home.join("Library/Caches/Firefox"),
                        home.join(
                            "Library/Application Support/BraveSoftware/Brave-Browser/Default/Cache",
                        ),
                    ];
                    let mut moved = 0u64;
                    let result = paths
                        .iter()
                        .try_for_each(|path| {
                            let size = du_bytes(path);
                            move_path_to_trash(path)?;
                            moved = moved.saturating_add(size);
                            Ok::<(), String>(())
                        })
                        .map(|_| moved);
                    ("Caches navigateurs", result)
                }
                "trash" => (
                    "Corbeille",
                    Err("Burrow n'efface jamais la Corbeille : utilisez Finder pour cette action irréversible".to_string()),
                ),
                "xcode" => (
                    "Xcode DerivedData",
                    move_directory_children_to_trash(
                        &home.join("Library/Developer/Xcode/DerivedData"),
                    ),
                ),
                "ios_backups" => (
                    "Sauvegardes iOS",
                    move_directory_children_to_trash(
                        &home.join("Library/Application Support/MobileSync/Backup"),
                    ),
                ),
                "brew_cache" => (
                    "Cache Homebrew",
                    move_directory_children_to_trash(&home.join("Library/Caches/Homebrew")),
                ),
                "simulator" => (
                    "Cache Simulateur iOS",
                    move_directory_children_to_trash(
                        &home.join("Library/Developer/CoreSimulator/Caches"),
                    ),
                ),
                _ => continue,
            };

            let _ = app.emit("mo-output", format!("→ {}", label));
            match result {
                Ok(bytes) => {
                    let _ = app.emit("mo-output", format!("  ✓ {}", label));
                    activity::record(
                        "nettoyage",
                        "Nettoyage récupérable",
                        "success",
                        label,
                        Some(bytes),
                        true,
                    );
                }
                Err(e) => {
                    any_error = true;
                    let _ = app.emit("mo-output", format!("  ✗ {} : {}", label, e));
                    activity::record(
                        "nettoyage",
                        "Nettoyage récupérable",
                        "error",
                        label,
                        None,
                        true,
                    );
                }
            }
        }

        for path in &installer_paths {
            let name = Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path);
            let _ = app.emit("mo-output", format!("→ {}", name));
            if let Err(e) = guard::validate_installer_path(path) {
                any_error = true;
                let _ = app.emit("mo-output", format!("  ✗ {} : chemin refusé ({})", name, e));
                continue;
            }
            let p = match require_path_grant(path, PathGrantPurpose::Trash) {
                Ok(path) => path,
                Err(e) => {
                    any_error = true;
                    let _ = app.emit("mo-output", format!("  ✗ {} : {}", name, e));
                    continue;
                }
            };
            if let Err(e) = guard::validate_installer_path(&p.to_string_lossy()) {
                any_error = true;
                let _ = app.emit("mo-output", format!("  ✗ {} : chemin refusé ({})", name, e));
                continue;
            }
            let size = if p.is_dir() {
                du_bytes(&p)
            } else {
                fs::metadata(&p).map(|metadata| metadata.len()).unwrap_or(0)
            };
            let result = move_path_to_trash(&p);
            match result {
                Ok(_) => {
                    let _ = app.emit("mo-output", format!("  ✓ {}", name));
                    activity::record(
                        "nettoyage",
                        "Installateur déplacé dans la Corbeille",
                        "success",
                        name,
                        Some(size),
                        true,
                    );
                }
                Err(e) => {
                    any_error = true;
                    let _ = app.emit("mo-output", format!("  ✗ {} : {}", name, e));
                    activity::record(
                        "nettoyage",
                        "Installateur déplacé dans la Corbeille",
                        "error",
                        name,
                        Some(size),
                        true,
                    );
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
            ("periodic", "Scripts périodiques"),
            ("diskutil_verify", "Vérification disque"),
            ("launch_services", "Base de données apps"),
        ];

        let mut any_error = false;

        for task_id in &tasks {
            if let Some(&(_, label)) = task_info.iter().find(|(id, _)| id == task_id) {
                let _ = app.emit("mo-output", format!("→ {}", label));

                let success = match task_id.as_str() {
                    "dns" => run_admin_sh(
                        "/usr/bin/dscacheutil -flushcache; /usr/bin/killall -HUP mDNSResponder",
                    )
                    .is_ok(),
                    "spotlight" => run_admin_sh("/usr/bin/mdutil -E /").is_ok(),
                    "finder" => Command::new("/usr/bin/killall")
                        .arg("Finder")
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false),
                    "dock" => Command::new("/usr/bin/killall")
                        .arg("Dock")
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false),
                    "swap" => run_admin_sh("/usr/bin/purge").is_ok(),
                    "launchpad" => {
                        let r1 = Command::new("/usr/bin/defaults")
                            .args(["write", "com.apple.dock", "ResetLaunchPad", "-bool", "true"])
                            .output()
                            .map(|o| o.status.success())
                            .unwrap_or(false);
                        let r2 = Command::new("/usr/bin/killall")
                            .arg("Dock")
                            .output()
                            .map(|o| o.status.success())
                            .unwrap_or(false);
                        r1 && r2
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
                    _ => false,
                };

                if success {
                    let _ = app.emit("mo-output", format!("  ✓ {}", label));
                    activity::record(
                        "optimisation",
                        "Tâche de maintenance",
                        "success",
                        label,
                        None,
                        false,
                    );
                } else {
                    any_error = true;
                    let _ = app.emit("mo-output", format!("  ✗ {}", label));
                    activity::record(
                        "optimisation",
                        "Tâche de maintenance",
                        "error",
                        label,
                        None,
                        false,
                    );
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
    let output = match Command::new("/usr/sbin/netstat").args(["-ib"]).output() {
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
        match run_admin_sh("/usr/bin/purge") {
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
    if !out.status.success() {
        return Err(
            strip_terminal_sequences(&String::from_utf8_lossy(&out.stderr))
                .trim()
                .to_string(),
        );
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Mole n'a pas renvoyé de version".to_string())
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
    None
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
        let out = Command::new("/usr/bin/curl")
            .args([
                "-s",
                "-L",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
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

fn brew_list_contains(brew_path: &str, kind: &str, token: &str) -> bool {
    guard::validate_brew_token(token).is_ok()
        && Command::new(brew_path)
            .args(["list", kind])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|stdout| stdout.lines().any(|line| line.trim() == token))
            .unwrap_or(false)
}

fn validate_cask_update_request(
    brew_path: Option<&str>,
    token: &str,
    download_url: &str,
    brew_managed: bool,
) -> Result<(), String> {
    guard::validate_brew_token(token)?;
    if brew_managed {
        let brew = brew_path.ok_or_else(|| "Homebrew introuvable".to_string())?;
        if !brew_list_contains(brew, "--cask", token) {
            return Err("Cask non installé ou non autorisé".to_string());
        }
        return Ok(());
    }

    let api = cask_api();
    let entry = api
        .by_token
        .get(token)
        .and_then(|index| api.casks.get(*index))
        .ok_or_else(|| "Cask absent du catalogue Homebrew backend".to_string())?;
    let expected_url = entry["url"]
        .as_str()
        .ok_or_else(|| "URL absente du catalogue Homebrew".to_string())?;
    guard::validate_update_url(expected_url)?;
    if download_url != expected_url {
        return Err("URL de mise à jour différente du catalogue backend".to_string());
    }
    Ok(())
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

        let brew_path = find_brew();
        if let Err(error) =
            validate_cask_update_request(brew_path.as_deref(), &name, &download_url, brew_managed)
        {
            out!(format!("✗ {error}"));
            done!(1);
        }

        if let Some(brew_path) = brew_path {
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
        let ok_dl = Command::new("/usr/bin/curl")
            .args([
                "-s",
                "-L",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--fail",
                "--max-time",
                "120",
                "--max-filesize",
                "2147483648",
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
        if !(500_000..=2_147_483_648).contains(&file_size) {
            out!("✗ Fichier trop petit — URL invalide ou page d'erreur");
            done!(1);
        }

        let mime = Command::new("/usr/bin/file")
            .args(["-b", "--mime-type", &tmp])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();
        let mime = mime.trim().to_string();
        let ext = if mime.contains("bzip2") {
            let is_dmg = Command::new("/usr/bin/hdiutil")
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
        if Command::new("/usr/bin/pgrep")
            .args(["-x", &name])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            out!(format!("Fermeture de {}…", name));
            quit_application(&name);
            std::thread::sleep(Duration::from_secs(1));
        }

        // Relier le token du catalogue à l'application réellement installée.
        let expected_bundle_id = cask_api()
            .by_token
            .get(&name)
            .and_then(|index| cask_api().casks.get(*index))
            .and_then(|entry| entry["bundle_id"].as_str())
            .filter(|value| !value.is_empty());
        let installed_app = collect_all_apps().into_iter().find(|p| {
            let bundle = plist_str(&p.join("Contents/Info.plist"), "CFBundleIdentifier");
            expected_bundle_id
                .map(|expected| bundle.as_deref() == Some(expected))
                .unwrap_or_else(|| {
                    p.file_stem()
                        .and_then(|s| s.to_str())
                        .map(|s| s.eq_ignore_ascii_case(&name))
                        .unwrap_or(false)
                })
        });
        let Some(installed_app) = installed_app else {
            out!("✗ Application installée introuvable");
            done!(1);
        };
        let install_dir = installed_app
            .parent()
            .map(|directory| directory.to_string_lossy().to_string())
            .unwrap_or_else(|| "/Applications".to_string());

        let ok = match ext {
            "pkg" => {
                out!("✗ Installation directe des packages PKG désactivée pour préserver l'identité de signature");
                false
            }
            "zip" | "tar.gz" | "tar.bz2" | "tar.xz" => {
                let tmp_dir = work_dir
                    .path()
                    .join("extracted")
                    .to_string_lossy()
                    .into_owned();
                fs::create_dir_all(&tmp_dir).ok();
                if let Err(e) = validate_archive_entries(Path::new(&tmp), ext) {
                    out!(format!("✗ Archive refusée : {e}"));
                    done!(1);
                }
                out!("Extraction…");
                let ok_x = if ext == "zip" {
                    Command::new("/usr/bin/unzip")
                        .args(["-q", "-o", &tmp, "-d", &tmp_dir])
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false)
                } else {
                    Command::new("/usr/bin/tar")
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
                let app_src = app_src.as_deref().unwrap();
                if let Err(e) = validate_update_bundle(&installed_app, Path::new(app_src)) {
                    out!(format!("✗ Mise à jour non authentique : {e}"));
                    done!(1);
                }
                let result = match copy_app(app_src, &installed_app) {
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
                let mount_out = Command::new("/usr/bin/hdiutil")
                    .args(["attach", &tmp, "-readonly", "-nobrowse"])
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
                    let _ = Command::new("/usr/bin/hdiutil")
                        .args(["detach", mp, "-quiet"])
                        .status();
                    let _ = fs::remove_file(&tmp);
                    done!(1);
                };
                if let Err(e) = validate_update_bundle(&installed_app, Path::new(src)) {
                    out!(format!("✗ Mise à jour non authentique : {e}"));
                    let _ = Command::new("/usr/bin/hdiutil")
                        .args(["detach", mp, "-quiet"])
                        .status();
                    done!(1);
                }
                out!(format!("Copie → {}…", install_dir));
                let result = match copy_app(src, &installed_app) {
                    Ok(()) => true,
                    Err(e) => {
                        out!(format!("✗ Copie échouée : {}", e));
                        false
                    }
                };
                out!("Démontage…");
                let _ = Command::new("/usr/bin/hdiutil")
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

        if let Err(error) = guard::validate_brew_token(&name) {
            out!(format!("✗ {error}"));
            done!(1);
        }

        let Some(brew_path) = find_brew() else {
            out!("✗ Homebrew non trouvé");
            done!(1);
        };

        if !brew_list_contains(&brew_path, "--formula", &name) {
            out!("✗ Formule non installée ou non autorisée");
            done!(1);
        }

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
    None
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
    None
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

fn write_quarantine_meta(entries: &[serde_json::Value]) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let dir = quarantine_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    let json = serde_json::to_vec_pretty(entries).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(&dir).map_err(|e| e.to_string())?;
    temp.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut temp, &json).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(quarantine_meta_path())
        .map_err(|e| e.error.to_string())?;
    Ok(())
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

/// Maximum number of scan targets accepted from the frontend.
const MAX_SCAN_TARGETS: usize = 16;
/// Maximum number of output lines forwarded to the frontend per scan.
const MAX_SCAN_OUTPUT_LINES: u64 = 50_000;
/// Maximum UTF-8 bytes forwarded through scan-line events per scan.
const MAX_SCAN_OUTPUT_BYTES: u64 = 8 * 1024 * 1024;

/// Deduplicate canonical scan paths and remove any path that is a descendant
/// of another path already in the list (the parent covers it).
fn dedup_scan_roots(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.sort();
    paths.dedup();
    let mut result: Vec<PathBuf> = Vec::new();
    for path in paths {
        if !result.iter().any(|parent| path.starts_with(parent)) {
            result.push(path);
        }
    }
    result
}

#[cfg(test)]
mod scan_coordination_tests {
    use super::*;

    #[test]
    fn scan_roots_are_deduplicated_and_descendants_removed() {
        let roots = dedup_scan_roots(vec![
            PathBuf::from("/Users/test/Downloads/subdir"),
            PathBuf::from("/Users/test/Desktop"),
            PathBuf::from("/Users/test/Downloads"),
            PathBuf::from("/Users/test/Downloads"),
        ]);
        assert_eq!(
            roots,
            vec![
                PathBuf::from("/Users/test/Desktop"),
                PathBuf::from("/Users/test/Downloads")
            ]
        );
    }

    #[test]
    fn activity_reservation_is_atomic_and_released_on_drop() {
        let flag = Box::leak(Box::new(AtomicBool::new(false)));
        let first = ActivityGuard::try_acquire(flag).expect("first reservation");
        assert!(ActivityGuard::try_acquire(flag).is_none());
        drop(first);
        assert!(ActivityGuard::try_acquire(flag).is_some());
    }

    #[test]
    fn du_timeout_wrapper_collects_normal_output() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("sample"), b"sample").expect("sample");
        let args = vec![
            std::ffi::OsString::from("-s"),
            std::ffi::OsString::from("-k"),
            std::ffi::OsString::from("-P"),
            std::ffi::OsString::from("--"),
            temp.path().as_os_str().to_owned(),
        ];
        let output = run_du_with_timeout(&args, Duration::from_secs(5)).expect("du output");
        assert!(output.status.success());
        assert!(!output.stdout.is_empty());
    }
}

#[tauri::command]
fn start_clamav_scan(app: tauri::AppHandle, paths: Vec<String>) {
    // Reserve the scan slot before spawning the worker. This makes the
    // check-and-set atomic and prevents two concurrent WebView calls from
    // both starting a process before the child handle is stored.
    let Some(scan_activity) = ActivityGuard::try_acquire(&SCAN_ACTIVE) else {
        let _ = app.emit("scan-error", "already-scanning");
        let _ = app.emit("scan-done", 2i32);
        return;
    };
    SCAN_CANCEL_REQUESTED.store(false, Ordering::Release);

    let Some(clamscan) = find_clamscan(&app) else {
        drop(scan_activity);
        let _ = app.emit("scan-error", "scanner-unavailable");
        let _ = app.emit("scan-done", 2i32);
        return;
    };
    let db_path = find_clamav_database(&app).map(|p| p.to_string_lossy().to_string());

    std::thread::spawn(move || {
        let _scan_activity = scan_activity;
        let home = home_dir();

        // Cap the number of targets to prevent frontend abuse.
        if paths.len() > MAX_SCAN_TARGETS {
            let _ = app.emit("scan-error", "too-many-targets");
            let _ = app.emit("scan-done", 2i32);
            return;
        }

        // Expand ~ and validate each path using the ClamAV-specific validator.
        // The validator returns the canonical path (symlinks resolved + forbidden
        // zones checked on both lexical and canonical path).
        let mut scan_roots: Vec<PathBuf> = Vec::new();
        for raw in &paths {
            let expanded = if let Some(stripped) = raw.strip_prefix("~/") {
                home.join(stripped).to_string_lossy().into_owned()
            } else if raw == "~" {
                home.to_string_lossy().into_owned()
            } else {
                raw.clone()
            };
            match guard::validate_clamav_scan_path(&expanded) {
                Ok(canonical) => scan_roots.push(canonical),
                Err(_) => {
                    // Never reflect a rejected path or canonical target to the
                    // untrusted WebView.
                    let _ = app.emit(
                        "scan-line",
                        "⚠ A scan target was rejected by the security policy",
                    );
                }
            }
        }

        // Deduplicate and remove redundant descendants.
        let scan_roots = dedup_scan_roots(scan_roots);

        if scan_roots.is_empty() {
            let _ = app.emit("scan-error", "no-valid-targets");
            let _ = app.emit("scan-done", 2i32);
            return;
        }

        let identities: Vec<_> = match scan_roots
            .iter()
            .map(|path| guard::path_identity(path))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(identities) => identities,
            Err(_) => {
                let _ = app.emit("scan-error", "target-changed");
                let _ = app.emit("scan-done", 2i32);
                return;
            }
        };

        // Build the ClamAV command.
        let mut cmd = Command::new(&clamscan);
        cmd.arg("-r")
            .arg("--no-summary")
            // Do not follow symlinks at any level.
            .arg("--follow-dir-symlinks=0")
            .arg("--follow-file-symlinks=0");

        if let Some(ref db) = db_path {
            cmd.arg(format!("--database={}", db));
        }

        // Add --exclude-dir arguments for every forbidden subtree that lies
        // under a scan root (e.g. ~/.ssh when scanning home).
        // Each argument is passed separately — never interpolated.
        for root in &scan_roots {
            for excl in guard::clamav_exclude_args(root) {
                cmd.arg(excl);
            }
        }

        // Pass canonical paths as positional arguments.
        for root in &scan_roots {
            cmd.arg(root);
        }

        // Close the validation/use window as far as possible before launch.
        if scan_roots
            .iter()
            .zip(&identities)
            .any(|(path, identity)| guard::revalidate_path_identity(path, *identity).is_err())
        {
            let _ = app.emit("scan-error", "target-changed");
            let _ = app.emit("scan-done", 2i32);
            return;
        }

        if SCAN_CANCEL_REQUESTED.load(Ordering::Acquire) {
            let _ = app.emit("scan-done", 130i32);
            return;
        }

        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = app.emit("scan-line", format!("✗ {e}"));
                let _ = app.emit("scan-done", 2i32);
                return;
            }
        };

        let stdout = child.stdout.take().unwrap();
        let stderr_pipe = child.stderr.take().unwrap();

        // Store the child handle so cancel_clamav_scan can kill it by handle
        // rather than by PID (no PID-reuse race).
        if let Ok(mut slot) = scan_child_store().lock() {
            *slot = Some(child);
            if SCAN_CANCEL_REQUESTED.load(Ordering::Acquire) {
                if let Some(ref mut child) = *slot {
                    let _ = child.kill();
                }
            }
        } else {
            let _ = child.kill();
            let _ = child.wait();
            let _ = app.emit("scan-error", "scanner-state-unavailable");
            let _ = app.emit("scan-done", 2i32);
            return;
        }

        let app_out = app.clone();
        let app_err = app.clone();
        let scan_roots_found = scan_roots.clone();
        let output_lines = Arc::new(AtomicU64::new(0));
        let output_bytes = Arc::new(AtomicU64::new(0));
        let threat_count = Arc::new(AtomicU64::new(0));
        let stdout_lines = Arc::clone(&output_lines);
        let stdout_bytes = Arc::clone(&output_bytes);
        let stdout_threats = Arc::clone(&threat_count);
        let stderr_lines = Arc::clone(&output_lines);
        let stderr_bytes = Arc::clone(&output_bytes);

        // Reader thread: stdout (FOUND results + file paths).
        let t1 = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let line_count = stdout_lines.fetch_add(1, Ordering::Relaxed);
                let byte_count = stdout_bytes.fetch_add(line.len() as u64, Ordering::Relaxed);
                if line_count >= MAX_SCAN_OUTPUT_LINES || byte_count >= MAX_SCAN_OUTPUT_BYTES {
                    let _ = app_out.emit(
                        "scan-line",
                        "⚠ Output truncated (limit reached)".to_string(),
                    );
                    break;
                }

                if line.ends_with(" FOUND") {
                    stdout_threats.fetch_add(1, Ordering::Relaxed);
                    if let Some((raw_path, _)) = line.rsplit_once(": ") {
                        // Validate the FOUND path:
                        // - must still exist (canonicalize is the TOCTOU guard)
                        // - must not be in a forbidden zone
                        // - must belong to one of the scan roots
                        match guard::validate_clamav_found_path(raw_path, &scan_roots_found) {
                            Ok(canonical) => {
                                grant_path(&canonical, PathGrantPurpose::Quarantine);
                            }
                            Err(_) => {
                                // Do not expose the rejected raw or canonical path.
                                let _ = app_out.emit(
                                    "scan-line",
                                    "⚠ A scan result was rejected by the security policy",
                                );
                                continue;
                            }
                        }
                    } else {
                        let _ = app_out.emit("scan-line", "⚠ Malformed ClamAV result rejected");
                        continue;
                    }
                }
                let _ = app_out.emit("scan-line", line);
            }
        });

        // Reader thread: stderr (informational messages).
        let t2 = std::thread::spawn(move || {
            for line in BufReader::new(stderr_pipe).lines().map_while(Result::ok) {
                let line_count = stderr_lines.fetch_add(1, Ordering::Relaxed);
                let byte_count = stderr_bytes.fetch_add(line.len() as u64, Ordering::Relaxed);
                if line_count >= MAX_SCAN_OUTPUT_LINES || byte_count >= MAX_SCAN_OUTPUT_BYTES {
                    break;
                }
                let _ = app_err.emit("scan-line", line);
            }
        });

        t1.join().ok();
        t2.join().ok();

        // Wait for the child and collect the exit code.
        let code = {
            match scan_child_store().lock() {
                Ok(mut slot) => slot
                    .as_mut()
                    .map(|child| child.wait().map(|s| s.code().unwrap_or(1)).unwrap_or(1))
                    .unwrap_or(1),
                Err(_) => 1,
            }
        };

        if let Ok(mut slot) = scan_child_store().lock() {
            *slot = None;
        }
        let threats = threat_count.load(Ordering::Relaxed);
        activity::record(
            "sécurité",
            "Analyse ClamAV",
            if code == 0 || code == 1 {
                "success"
            } else {
                "error"
            },
            &format!("{threats} menace(s) détectée(s)"),
            None,
            false,
        );
        let _ = app.emit("scan-done", code);
    });
}

#[tauri::command]
fn start_smart_security_scan(app: tauri::AppHandle) {
    let home = home_dir();
    let targets = ["Downloads", "Desktop", "Documents"]
        .iter()
        .map(|name| home.join(name))
        .filter(|path| path.exists())
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    start_clamav_scan(app, targets);
}

#[tauri::command]
fn cancel_clamav_scan() {
    SCAN_CANCEL_REQUESTED.store(true, Ordering::Release);
    // Kill by the exact Child handle — no PID-reuse race.
    if let Ok(mut slot) = scan_child_store().lock() {
        if let Some(ref mut child) = *slot {
            let _ = child.kill();
        }
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
    let canonical = require_path_grant(&original_path, PathGrantPurpose::Quarantine)?;
    guard::validate_trash_path(&canonical.to_string_lossy())?;
    let src = canonical.as_path();
    let fname = src
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Chemin invalide")?
        .to_string();

    let dir = quarantine_dir();
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let unique_name = format!("{}_{}_{}", ts, uuid::Uuid::new_v4(), fname);
    let dest = dir.join(&unique_name);

    if fs::rename(src, &dest).is_err() {
        fs::copy(src, &dest).map_err(|e| e.to_string())?;
        fs::remove_file(src).ok();
    }

    let mut meta = read_quarantine_meta();
    meta.push(serde_json::json!({
        "name": unique_name,
        "original_path": canonical.to_string_lossy(),
        "quarantined_at": ts.to_string()
    }));
    write_quarantine_meta(&meta)
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
    let original = guard::validate_trash_path(original_path)?;
    if original.exists() {
        return Err(
            "Le chemin original existe déjà ; restauration refusée pour éviter tout écrasement"
                .to_string(),
        );
    }
    let parent = original.parent().ok_or("Dossier d'origine invalide")?;
    let canonical_parent = fs::canonicalize(parent).map_err(|_| {
        "Le dossier d'origine n'existe plus ; recréez-le avant de restaurer".to_string()
    })?;
    if canonical_parent != parent {
        return Err(
            "Le dossier d'origine traverse un lien symbolique ; restauration refusée".to_string(),
        );
    }
    let qpath = quarantine_dir().join(&name);

    if fs::rename(&qpath, &original).is_err() {
        let mut source = fs::File::open(&qpath).map_err(|e| e.to_string())?;
        let mut destination = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&original)
            .map_err(|e| e.to_string())?;
        std::io::copy(&mut source, &mut destination).map_err(|e| e.to_string())?;
        destination.sync_all().map_err(|e| e.to_string())?;
        fs::remove_file(&qpath).ok();
    }

    let new_meta: Vec<_> = meta
        .into_iter()
        .filter(|m| m["name"].as_str() != Some(&name))
        .collect();
    write_quarantine_meta(&new_meta)
}

#[tauri::command]
fn delete_from_quarantine(name: String) -> Result<(), String> {
    guard::validate_quarantine_name(&name)?;
    let qpath = quarantine_dir().join(&name);
    let size = if qpath.is_dir() {
        du_bytes(&qpath)
    } else {
        fs::metadata(&qpath)
            .map(|metadata| metadata.len())
            .unwrap_or(0)
    };
    move_path_to_trash(&qpath)?;

    let meta = read_quarantine_meta();
    let new_meta: Vec<_> = meta
        .into_iter()
        .filter(|m| m["name"].as_str() != Some(&name))
        .collect();
    write_quarantine_meta(&new_meta)?;
    activity::record(
        "sécurité",
        "Quarantaine déplacée dans la Corbeille",
        "success",
        &name,
        Some(size),
        true,
    );
    Ok(())
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
    let out = Command::new("/usr/bin/osascript")
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

#[derive(Serialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub is_dir: bool,
}

// ── iCloud / FileProvider safety guard ───────────────────────────────────────

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

    let results: Vec<AiCacheItem> = candidates
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
        .collect();
    for item in &results {
        grant_path(Path::new(&item.path), PathGrantPurpose::Trash);
    }
    results
}

#[tauri::command]
fn clean_ai_caches(ids: Vec<String>) -> Result<u64, String> {
    let all = scan_ai_caches();
    let mut freed = 0u64;
    for item in all.iter().filter(|i| ids.contains(&i.id)) {
        let p = require_path_grant(&item.path, PathGrantPurpose::Trash)?;
        guard::validate_trash_path(&p.to_string_lossy())?;
        let size = if p.is_dir() {
            du_bytes(&p)
        } else {
            fs::metadata(&p).map(|m| m.len()).unwrap_or(0)
        };
        move_path_to_trash(&p)?;
        freed = freed.saturating_add(size);
        activity::record(
            "nettoyage",
            "Cache IA déplacé dans la Corbeille",
            "success",
            &item.label,
            Some(size),
            true,
        );
    }
    Ok(freed)
}

// ── Dev caches (npm / yarn / pnpm / brew) ────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct DevCacheItem {
    pub id: String,
    pub label: String,
    pub path: String,
    pub size_bytes: u64,
}

#[tauri::command]
fn scan_dev_caches() -> Vec<DevCacheItem> {
    let home = home_dir();

    // Catalogue backend fixe : une configuration locale de npm/yarn ne doit pas
    // pouvoir rediriger une suppression vers un chemin arbitraire.
    let npm_path = home.join(".npm").to_string_lossy().to_string();
    let yarn_path = home
        .join("Library/Caches/Yarn")
        .to_string_lossy()
        .to_string();
    let pnpm_path = home
        .join("Library/pnpm/store")
        .to_string_lossy()
        .to_string();
    let brew_path = home
        .join("Library/Caches/Homebrew")
        .to_string_lossy()
        .to_string();

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

    let results: Vec<DevCacheItem> = candidates
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
        .collect();
    for item in &results {
        grant_path(Path::new(&item.path), PathGrantPurpose::Trash);
    }
    results
}

#[tauri::command]
fn clean_dev_caches(ids: Vec<String>) -> Result<u64, String> {
    let all = scan_dev_caches();
    let mut freed = 0u64;
    for item in all.iter().filter(|i| ids.contains(&i.id)) {
        let p = require_path_grant(&item.path, PathGrantPurpose::Trash)?;
        guard::validate_trash_path(&p.to_string_lossy())?;
        let size = du_bytes(&p);
        move_path_to_trash(&p)?;
        freed = freed.saturating_add(size);
        activity::record(
            "nettoyage",
            "Cache développeur déplacé dans la Corbeille",
            "success",
            &item.label,
            Some(size),
            true,
        );
    }
    Ok(freed)
}

// ── Find residual files left by an app ───────────────────────────────────────

fn parse_mole_preview_paths(output: &str, home: &Path) -> Vec<PathBuf> {
    let clean = strip_terminal_sequences(output);
    let mut in_file_list = false;
    let mut seen = std::collections::HashSet::new();
    let mut paths = Vec::new();

    for line in clean.lines().map(str::trim) {
        if line == "Files to be removed:" {
            in_file_list = true;
            continue;
        }
        if !in_file_list {
            continue;
        }
        if line.starts_with('➤') {
            break;
        }
        if line.contains("Review only:") {
            continue;
        }
        let Some(path_start) = line.find("~/").or_else(|| line.find('/')) else {
            continue;
        };
        let mut displayed_path = &line[path_start..];
        if let Some((path_part, size_part)) = displayed_path.rsplit_once(" , ") {
            if ["B", "KB", "MB", "GB", "TB"]
                .iter()
                .any(|suffix| size_part.ends_with(suffix))
            {
                displayed_path = path_part;
            }
        }
        let lexical = displayed_path
            .strip_prefix("~/")
            .map(|relative| home.join(relative))
            .unwrap_or_else(|| PathBuf::from(displayed_path));
        let Ok(canonical) = fs::canonicalize(&lexical) else {
            continue;
        };
        if !seen.insert(canonical.clone()) {
            continue;
        }
        paths.push(canonical);
    }
    paths
}

fn validate_mole_preview_paths(
    paths: &[PathBuf],
    selected_app: &Path,
    home: &Path,
) -> Result<(), String> {
    const ALLOWED_SYSTEM_RESIDUAL_ROOTS: &[&str] = &[
        "/Library/Application Support",
        "/Library/Caches",
        "/Library/Preferences",
        "/Library/Logs",
        "/Library/LaunchAgents",
        "/Library/LaunchDaemons",
        "/Library/PrivilegedHelperTools",
        "/Library/Extensions",
        "/Library/Audio/Plug-Ins",
        "/Library/Internet Plug-Ins",
        "/Library/Input Methods",
        "/Library/Screen Savers",
    ];

    for path in paths {
        let is_other_application = path.extension().and_then(|extension| extension.to_str())
            == Some("app")
            && path != selected_app;
        let allowed = !is_other_application
            && (path == selected_app
                || (path.starts_with(home) && !guard::is_forbidden_for_readonly(path))
                || ALLOWED_SYSTEM_RESIDUAL_ROOTS
                    .iter()
                    .any(|root| path.starts_with(root)));
        if !allowed {
            return Err(format!(
                "Mole propose un chemin hors des zones de désinstallation autorisées : {}",
                path.display()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod mole_uninstall_tests {
    use super::*;

    #[test]
    fn accepts_application_names_but_rejects_cli_options_and_controls() {
        assert!(validate_mole_app_name("Ollama").is_ok());
        assert!(validate_mole_app_name("Visual Studio Code").is_ok());
        assert!(validate_mole_app_name("--permanent").is_err());
        assert!(validate_mole_app_name("Signal\n--permanent").is_err());
    }

    #[test]
    fn parses_only_existing_paths_from_the_mole_removal_section() {
        let temp = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("temporary home");
        let home = temp.path();
        let app = home.join("Applications/Ollama.app");
        let residual = home.join("Library/Application Support/Ollama");
        fs::create_dir_all(&app).expect("application fixture");
        fs::create_dir_all(&residual).expect("residual fixture");

        let output = format!(
            "\u{1b}[1mFiles to be removed:\u{1b}[0m\n  ✓ {} , 12 MB\n  ✓ ~/Library/Application Support/Ollama , 4 KB\n  ✓ ~/missing , 1 KB\n  Review only: /tmp/example\n➤ Continue?",
            app.display()
        );
        let paths = parse_mole_preview_paths(&output, home);

        assert_eq!(paths, vec![app, residual]);
    }

    #[test]
    fn rejects_another_application_or_an_unrelated_system_path() {
        let temp = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("temporary home");
        let home = temp.path();
        let selected = home.join("Applications/Ollama.app");
        let other = home.join("Applications/Other.app");
        let residual = home.join("Library/Application Support/Ollama");

        assert!(
            validate_mole_preview_paths(&[selected.clone(), residual], &selected, home).is_ok()
        );
        assert!(validate_mole_preview_paths(&[selected.clone(), other], &selected, home).is_err());
        assert!(validate_mole_preview_paths(
            &[selected.clone(), PathBuf::from("/etc/hosts")],
            &selected,
            home
        )
        .is_err());
    }
}

#[tauri::command]
async fn find_app_residuals(
    app: tauri::AppHandle,
    app_name: String,
    app_path: String,
) -> Result<Vec<FileEntry>, String> {
    let canonical = require_path_grant(&app_path, PathGrantPurpose::Uninstall)?;
    guard::validate_app_uninstall_path(&canonical.to_string_lossy())?;
    if canonical.file_stem().and_then(|value| value.to_str()) != Some(app_name.as_str()) {
        return Err("Le nom ne correspond pas à l'application analysée".to_string());
    }
    validate_mole_app_name(&app_name)?;
    let mo_path = bundled_mo_path(&app)?;
    let bundle_id = app_bundle_id(&canonical);
    let home = home_dir();

    tauri::async_runtime::spawn_blocking(move || {
        let output = run_mole_uninstall(&mo_path, &app_name, true)
            .map_err(|error| strip_terminal_sequences(&error).trim().to_string())?;
        let mut paths = parse_mole_preview_paths(&output, &home);
        paths.sort();
        validate_mole_preview_paths(&paths, &canonical, &home)?;
        if !paths.iter().any(|path| path == &canonical) {
            return Err(
                "Mole n'a pas relié l'application sélectionnée à cet aperçu ; aucune désinstallation ne sera autorisée"
                    .to_string(),
            );
        }

        let mut results = paths
            .iter()
            .filter(|path| *path != &canonical)
            .filter_map(|path| {
                let metadata = fs::metadata(path).ok()?;
                let size_bytes = if metadata.is_dir() {
                    du_bytes(path)
                } else {
                    metadata.len()
                };
                Some(FileEntry {
                    name: path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("Fichier associé")
                        .to_string(),
                    path: path.to_string_lossy().into_owned(),
                    size_bytes,
                    is_dir: metadata.is_dir(),
                })
            })
            .collect::<Vec<_>>();
        results.sort_by_key(|entry| std::cmp::Reverse(entry.size_bytes));
        grant_mole_preview(&canonical, &app_name, &bundle_id, &paths);
        Ok(results)
    })
    .await
    .map_err(|error| format!("Erreur pendant l'aperçu Mole : {error}"))?
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
        true,
        ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory()
            .with_disk_usage(),
    );
    std::thread::sleep(Duration::from_millis(200));
    sys.refresh_cpu_usage();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
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
    let out = Command::new("/sbin/route")
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
    let out = Command::new("/usr/sbin/networksetup")
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

    let out = match Command::new("/usr/sbin/networksetup")
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
            let dns_out = Command::new("/usr/sbin/networksetup")
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
    let root_id = xml_escape(&format!("net.burrow.dns.{}.{}", provider_id, option_id));
    let payload_id = xml_escape(&format!("com.apple.dnsSettings.managed.{}", payload_uuid));
    let display_name = xml_escape(display_name);
    let doh_url = xml_escape(doh_url);
    let addrs: String = servers
        .iter()
        .map(|s| format!("\t\t\t\t\t<string>{}</string>", xml_escape(s)))
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
}

fn doh_catalog() -> std::collections::HashMap<(&'static str, &'static str), DohEntry> {
    use std::collections::HashMap;
    let mut m: HashMap<(&str, &str), DohEntry> = HashMap::new();

    // Mullvad — valeurs publiées dans la documentation officielle.
    // Les profils sont générés localement : aucune URL de téléchargement n'est acceptée.
    m.insert(
        ("mullvad", "std"),
        DohEntry {
            display_name: "Mullvad — Standard",
            doh_url: "https://dns.mullvad.net/dns-query",
            servers: &["194.242.2.2", "2a07:e340::2"],
        },
    );
    m.insert(
        ("mullvad", "adblock"),
        DohEntry {
            display_name: "Mullvad — Adblock",
            doh_url: "https://adblock.dns.mullvad.net/dns-query",
            servers: &["194.242.2.3", "2a07:e340::3"],
        },
    );
    m.insert(
        ("mullvad", "base"),
        DohEntry {
            display_name: "Mullvad — Base",
            doh_url: "https://base.dns.mullvad.net/dns-query",
            servers: &["194.242.2.4", "2a07:e340::4"],
        },
    );
    m.insert(
        ("mullvad", "extended"),
        DohEntry {
            display_name: "Mullvad — Extended",
            doh_url: "https://extended.dns.mullvad.net/dns-query",
            servers: &["194.242.2.5", "2a07:e340::5"],
        },
    );
    m.insert(
        ("mullvad", "family"),
        DohEntry {
            display_name: "Mullvad — Family",
            doh_url: "https://family.dns.mullvad.net/dns-query",
            servers: &["194.242.2.6", "2a07:e340::6"],
        },
    );
    m.insert(
        ("mullvad", "all"),
        DohEntry {
            display_name: "Mullvad — All",
            doh_url: "https://all.dns.mullvad.net/dns-query",
            servers: &["194.242.2.9", "2a07:e340::9"],
        },
    );

    // Quad9
    m.insert(
        ("quad9", "sec"),
        DohEntry {
            display_name: "Quad9 — Secure",
            doh_url: "https://dns.quad9.net/dns-query",
            servers: &["9.9.9.9", "149.112.112.112", "2620:fe::fe", "2620:fe::9"],
        },
    );
    m.insert(
        ("quad9", "unf"),
        DohEntry {
            display_name: "Quad9 — Unsecure",
            doh_url: "https://dns10.quad9.net/dns-query",
            servers: &["9.9.9.10", "149.112.112.10"],
        },
    );
    m.insert(
        ("quad9", "edns"),
        DohEntry {
            display_name: "Quad9 — EDNS",
            doh_url: "https://dns11.quad9.net/dns-query",
            servers: &["9.9.9.11", "149.112.112.11"],
        },
    );

    // LibreDNS
    m.insert(
        ("libredns", "std"),
        DohEntry {
            display_name: "LibreDNS — Standard",
            doh_url: "https://doh.libredns.gr/dns-query",
            servers: &["116.202.176.26"],
        },
    );

    // FDN — https://www.fdn.fr/actions/dns/
    m.insert(
        ("fdn", "std"),
        DohEntry {
            display_name: "FDN — Standard",
            doh_url: "https://ns0.fdn.fr/dns-query",
            servers: &[
                "80.67.169.12",
                "80.67.169.40",
                "2001:910:800::12",
                "2001:910:800::40",
            ],
        },
    );

    // DNS4EU Public Service — https://joindns4.eu/for-public
    m.insert(
        ("dns4eu", "protective"),
        DohEntry {
            display_name: "DNS4EU — Protective",
            doh_url: "https://protective.joindns4.eu/dns-query",
            servers: &[
                "86.54.11.1",
                "86.54.11.201",
                "2a13:1001::86:54:11:1",
                "2a13:1001::86:54:11:201",
            ],
        },
    );
    m.insert(
        ("dns4eu", "child"),
        DohEntry {
            display_name: "DNS4EU — Child Protection",
            doh_url: "https://child.joindns4.eu/dns-query",
            servers: &[
                "86.54.11.12",
                "86.54.11.212",
                "2a13:1001::86:54:11:12",
                "2a13:1001::86:54:11:212",
            ],
        },
    );
    m.insert(
        ("dns4eu", "noads"),
        DohEntry {
            display_name: "DNS4EU — Protective + Ad Blocking",
            doh_url: "https://noads.joindns4.eu/dns-query",
            servers: &[
                "86.54.11.13",
                "86.54.11.213",
                "2a13:1001::86:54:11:13",
                "2a13:1001::86:54:11:213",
            ],
        },
    );
    m.insert(
        ("dns4eu", "child-noads"),
        DohEntry {
            display_name: "DNS4EU — Child Protection + Ad Blocking",
            doh_url: "https://child-noads.joindns4.eu/dns-query",
            servers: &[
                "86.54.11.11",
                "86.54.11.211",
                "2a13:1001::86:54:11:11",
                "2a13:1001::86:54:11:211",
            ],
        },
    );
    m.insert(
        ("dns4eu", "unfiltered"),
        DohEntry {
            display_name: "DNS4EU — Unfiltered",
            doh_url: "https://unfiltered.joindns4.eu/dns-query",
            servers: &[
                "86.54.11.100",
                "86.54.11.200",
                "2a13:1001::86:54:11:100",
                "2a13:1001::86:54:11:200",
            ],
        },
    );

    // DNS.SB — https://dns.sb/guide/ and https://dns.sb/doh/
    m.insert(
        ("dnssb", "std"),
        DohEntry {
            display_name: "DNS.SB — Standard",
            doh_url: "https://doh.dns.sb/dns-query",
            servers: &["185.222.222.222", "45.11.45.11", "2a09::", "2a11::"],
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
        },
    );
    m.insert(
        ("adguard", "unf"),
        DohEntry {
            display_name: "AdGuard — Unfiltered",
            doh_url: "https://unfiltered.adguard-dns.com/dns-query",
            servers: &[
                "94.140.14.140",
                "94.140.14.141",
                "2a10:50c0::1:ff",
                "2a10:50c0::2:ff",
            ],
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
        },
    );
    m.insert(
        ("cloudflare", "mal"),
        DohEntry {
            display_name: "Cloudflare — Security",
            doh_url: "https://security.cloudflare-dns.com/dns-query",
            servers: &[
                "1.1.1.2",
                "1.0.0.2",
                "2606:4700:4700::1112",
                "2606:4700:4700::1002",
            ],
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
        },
    );

    m
}

/// Catalogue DNS classique immuable. Le frontend ne choisit que le fournisseur et le profil ;
/// les adresses effectivement appliquées restent définies et validées dans le backend.
fn classic_dns_servers(provider_id: &str, option_id: &str) -> Option<&'static [&'static str]> {
    match (provider_id, option_id) {
        ("mullvad", "std") => Some(&["194.242.2.2"]),
        ("mullvad", "adblock") => Some(&["194.242.2.3"]),
        ("mullvad", "base") => Some(&["194.242.2.4"]),
        ("mullvad", "extended") => Some(&["194.242.2.5"]),
        ("mullvad", "family") => Some(&["194.242.2.6"]),
        ("mullvad", "all") => Some(&["194.242.2.9"]),
        ("quad9", "sec") => Some(&["9.9.9.9", "149.112.112.112"]),
        ("quad9", "unf") => Some(&["9.9.9.10", "149.112.112.10"]),
        ("quad9", "edns") => Some(&["9.9.9.11", "149.112.112.11"]),
        ("libredns", "std") => Some(&["116.202.176.26"]),
        ("dns4eu", "protective") => Some(&["86.54.11.1", "86.54.11.201"]),
        ("dns4eu", "child") => Some(&["86.54.11.12", "86.54.11.212"]),
        ("dns4eu", "noads") => Some(&["86.54.11.13", "86.54.11.213"]),
        ("dns4eu", "child-noads") => Some(&["86.54.11.11", "86.54.11.211"]),
        ("dns4eu", "unfiltered") => Some(&["86.54.11.100", "86.54.11.200"]),
        ("dnssb", "std") => Some(&["185.222.222.222", "45.11.45.11"]),
        ("opennic", "eu") => Some(&["91.190.185.43", "194.36.144.87"]),
        ("adguard", "std") => Some(&["94.140.14.14", "94.140.15.15"]),
        ("adguard", "family") => Some(&["94.140.14.15", "94.140.15.16"]),
        ("adguard", "unf") => Some(&["94.140.14.140", "94.140.14.141"]),
        ("cloudflare", "std") => Some(&["1.1.1.1", "1.0.0.1"]),
        ("cloudflare", "mal") => Some(&["1.1.1.2", "1.0.0.2"]),
        ("cloudflare", "family") => Some(&["1.1.1.3", "1.0.0.3"]),
        ("dnswatch", "std") => Some(&["84.200.69.80", "84.200.70.40"]),
        _ => None,
    }
}

#[cfg(test)]
mod doh_catalog_tests {
    use super::{classic_dns_servers, doh_catalog, generate_doh_mobileconfig, guard};

    #[test]
    fn contains_every_frontend_doh_option() {
        let catalog = doh_catalog();
        let expected = [
            ("mullvad", "std"),
            ("mullvad", "adblock"),
            ("mullvad", "base"),
            ("mullvad", "extended"),
            ("mullvad", "family"),
            ("mullvad", "all"),
            ("quad9", "sec"),
            ("quad9", "unf"),
            ("quad9", "edns"),
            ("libredns", "std"),
            ("fdn", "std"),
            ("dns4eu", "protective"),
            ("dns4eu", "child"),
            ("dns4eu", "noads"),
            ("dns4eu", "child-noads"),
            ("dns4eu", "unfiltered"),
            ("dnssb", "std"),
            ("adguard", "std"),
            ("adguard", "family"),
            ("adguard", "unf"),
            ("cloudflare", "std"),
            ("cloudflare", "mal"),
            ("cloudflare", "family"),
        ];

        assert_eq!(catalog.len(), expected.len());
        for key in expected {
            assert!(catalog.contains_key(&key), "missing DoH entry: {key:?}");
        }
    }

    #[test]
    fn classic_catalog_contains_only_curated_frontend_options() {
        let expected = [
            ("mullvad", "std"),
            ("mullvad", "adblock"),
            ("mullvad", "base"),
            ("mullvad", "extended"),
            ("mullvad", "family"),
            ("mullvad", "all"),
            ("quad9", "sec"),
            ("quad9", "unf"),
            ("quad9", "edns"),
            ("libredns", "std"),
            ("dns4eu", "protective"),
            ("dns4eu", "child"),
            ("dns4eu", "noads"),
            ("dns4eu", "child-noads"),
            ("dns4eu", "unfiltered"),
            ("dnssb", "std"),
            ("opennic", "eu"),
            ("adguard", "std"),
            ("adguard", "family"),
            ("adguard", "unf"),
            ("cloudflare", "std"),
            ("cloudflare", "mal"),
            ("cloudflare", "family"),
            ("dnswatch", "std"),
        ];
        let doh = doh_catalog();

        for key in expected {
            let servers = classic_dns_servers(key.0, key.1)
                .unwrap_or_else(|| panic!("missing classic DNS entry: {key:?}"));
            assert!(!servers.is_empty());
            for server in servers {
                guard::validate_ip_address(server)
                    .expect("catalog address must be public and valid");
            }
            if let Some(encrypted) = doh.get(&key) {
                for server in servers {
                    assert!(
                        encrypted.servers.contains(server),
                        "classic and DoH catalog disagree for {key:?}: {server}"
                    );
                }
            }
        }

        assert!(classic_dns_servers("fdn", "std").is_none());
        assert!(classic_dns_servers("unknown", "std").is_none());
        assert!(classic_dns_servers("cloudflare", "unknown").is_none());
    }

    #[test]
    fn mobileconfig_escapes_every_xml_value() {
        let xml = generate_doh_mobileconfig(
            "provider<&",
            "option>\"",
            "Display & <Name>",
            "https://example.test/dns-query?a=1&b=2",
            &["1.1.1.1".to_string(), "<invalid>".to_string()],
        );

        assert!(xml.contains("net.burrow.dns.provider&lt;&amp;.option&gt;&quot;"));
        assert!(xml.contains("Display &amp; &lt;Name&gt;"));
        assert!(xml.contains("dns-query?a=1&amp;b=2"));
        assert!(xml.contains("<string>&lt;invalid&gt;</string>"));
        assert!(!xml.contains("<string><invalid></string>"));
    }

    #[test]
    fn uses_current_mullvad_addresses() {
        let catalog = doh_catalog();
        assert_eq!(
            catalog.get(&("mullvad", "base")).expect("base").servers,
            &["194.242.2.4", "2a07:e340::4"]
        );
        assert_eq!(
            catalog.get(&("mullvad", "all")).expect("all").servers,
            &["194.242.2.9", "2a07:e340::9"]
        );
    }

    #[test]
    fn uses_official_fdn_dns4eu_and_dnssb_endpoints() {
        let catalog = doh_catalog();
        assert_eq!(
            catalog.get(&("fdn", "std")).expect("fdn").servers,
            &[
                "80.67.169.12",
                "80.67.169.40",
                "2001:910:800::12",
                "2001:910:800::40",
            ]
        );
        assert_eq!(
            catalog
                .get(&("dns4eu", "protective"))
                .expect("dns4eu protective")
                .doh_url,
            "https://protective.joindns4.eu/dns-query"
        );
        assert_eq!(
            catalog.get(&("dnssb", "std")).expect("dns.sb").servers,
            &["185.222.222.222", "45.11.45.11", "2a09::", "2a11::"]
        );
    }
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
        .truncate(true)
        .open(&tmp_path)
        .map_err(|e| e.to_string())?;
    f.write_all(xml.as_bytes()).map_err(|e| e.to_string())?;
    f.flush().map_err(|e| e.to_string())?;

    let opened = Command::new("/usr/bin/open")
        .arg(&tmp_path)
        .output()
        .map_err(|e| e.to_string())?;
    if !opened.status.success() {
        let stderr = String::from_utf8_lossy(&opened.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "macOS n'a pas pu ouvrir le profil DoH".to_string()
        } else {
            stderr
        });
    }
    // _guard dropped here → temp file deleted after macOS has opened it
    Ok(())
}

#[tauri::command]
fn set_dns_servers(service: String, provider_id: String, option_id: String) -> Result<(), String> {
    guard::validate_service_name(&service)?;
    let servers = classic_dns_servers(&provider_id, &option_id).ok_or_else(|| {
        format!(
            "Profil DNS classique inconnu ou indisponible : {}/{}",
            provider_id, option_id
        )
    })?;
    for ip in servers {
        guard::validate_ip_address(ip).map_err(|e| format!("Serveur DNS invalide : {}", e))?;
    }
    // Utiliser Command::arg() — aucune interpolation shell
    let mut cmd = Command::new("/usr/sbin/networksetup");
    cmd.arg("-setdnsservers").arg(&service);
    for ip in servers {
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
                    servers
                        .iter()
                        .map(|s| posix_shell_quote_value(s))
                        .collect::<Vec<_>>()
                        .join(" ")
                ))
            );
            let adm = Command::new("/usr/bin/osascript")
                .args(["-e", &script])
                .output()
                .map_err(|e| e.to_string())?;
            if !adm.status.success() {
                return Err(String::from_utf8_lossy(&adm.stderr).trim().to_string());
            }
        }
    }
    let _ = Command::new("/usr/bin/dscacheutil")
        .arg("-flushcache")
        .output();
    let _ = Command::new("/usr/bin/killall")
        .args(["-HUP", "mDNSResponder"])
        .output();
    Ok(())
}

#[tauri::command]
fn reset_dns(service: String) -> Result<(), String> {
    guard::validate_service_name(&service)?;
    let out = Command::new("/usr/sbin/networksetup")
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
        let adm = Command::new("/usr/bin/osascript")
            .args(["-e", &script])
            .output()
            .map_err(|e| e.to_string())?;
        if !adm.status.success() {
            return Err(String::from_utf8_lossy(&adm.stderr).trim().to_string());
        }
    }
    let _ = Command::new("/usr/bin/dscacheutil")
        .arg("-flushcache")
        .output();
    let _ = Command::new("/usr/bin/killall")
        .args(["-HUP", "mDNSResponder"])
        .output();
    Ok(())
}

#[tauri::command]
fn get_search_domains(service: String) -> Vec<String> {
    let out = Command::new("/usr/sbin/networksetup")
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
    let mut cmd = Command::new("/usr/sbin/networksetup");
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
    let adm = Command::new("/usr/bin/osascript")
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
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_disk_usage(),
        );
        loop {
            std::thread::sleep(Duration::from_secs(1));
            sys.refresh_cpu_usage();
            sys.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing()
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
    let _ = Command::new("/bin/kill")
        .args(["-TERM", &pid.to_string()])
        .output();

    // Attendre brièvement et vérifier si le processus s'est terminé
    std::thread::sleep(std::time::Duration::from_millis(400));
    if get_process_uid(pid).is_none() {
        return Ok(());
    }

    // SIGKILL seulement si le processus résiste
    let out = Command::new("/bin/kill")
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
    Command::new("/bin/ps")
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
    guard::validate_update_url(feed_url).ok()?;
    let plist = path.join("Contents/Info.plist");
    let installed_short = plist_str(&plist, "CFBundleShortVersionString").unwrap_or_default();
    let installed_build = plist_str(&plist, "CFBundleVersion").unwrap_or_default();

    if installed_short.is_empty() && installed_build.is_empty() {
        return None;
    }

    let out = Command::new("/usr/bin/curl")
        .args([
            "-s",
            "-L",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
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

            let out = Command::new("/usr/bin/curl")
                .args([
                    "-s",
                    "-L",
                    "--proto",
                    "=https",
                    "--proto-redir",
                    "=https",
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
            let arch = Command::new("/usr/bin/uname")
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
            guard::validate_update_url(&manifest_url).ok()?;

            let out = Command::new("/usr/bin/curl")
                .args([
                    "-s",
                    "-L",
                    "--proto",
                    "=https",
                    "--proto-redir",
                    "=https",
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

            let arch = Command::new("/usr/bin/uname")
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

fn authorize_sparkle_update(
    name: &str,
    download_url: &str,
    app_path: &str,
) -> Result<PathBuf, String> {
    guard::validate_update_url(download_url)?;
    guard::validate_update_app_path(app_path)?;
    let canonical = fs::canonicalize(app_path)
        .map_err(|e| format!("Application installée introuvable : {e}"))?;
    guard::validate_update_app_path(&canonical.to_string_lossy())?;

    let update = check_one_app(canonical.clone())
        .or_else(|| check_electron_update(canonical.clone()))
        .ok_or_else(|| "Mise à jour non confirmée par la source backend".to_string())?;
    if update.name != name
        || update.download_url != download_url
        || Path::new(&update.path) != canonical
    {
        return Err("La demande de mise à jour ne correspond pas au catalogue backend".to_string());
    }
    Ok(canonical)
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
    let country = Command::new("/usr/bin/defaults")
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
            let out = Command::new("/usr/bin/curl")
                .args([
                    "-s",
                    "-L",
                    "--proto",
                    "=https",
                    "--proto-redir",
                    "=https",
                    "--max-time",
                    "8",
                    "-A",
                    "Mozilla/5.0",
                    &url,
                ])
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

#[tauri::command]
fn open_app_store_url(url: String) -> Result<(), String> {
    let allowed =
        url.starts_with("https://apps.apple.com/") || url.starts_with("https://itunes.apple.com/");
    if !allowed || url.len() > 2048 || url.chars().any(char::is_whitespace) || url.contains('\0') {
        return Err("URL App Store refusée".to_string());
    }
    let status = Command::new("/usr/bin/open")
        .arg(&url)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("Impossible d'ouvrir l'App Store".to_string())
    }
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

fn signing_field(output: &str, field: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix(field))
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "not set")
        .map(str::to_string)
}

fn app_signing_identity(path: &Path) -> Result<(String, String), String> {
    let verified = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict", "--verbose=2"])
        .arg(path)
        .output()
        .map_err(|e| format!("Impossible de vérifier la signature : {e}"))?;
    if !verified.status.success() {
        return Err(format!(
            "Signature de l'application invalide : {}",
            String::from_utf8_lossy(&verified.stderr).trim()
        ));
    }

    let details = Command::new("/usr/bin/codesign")
        .args(["-d", "--verbose=4"])
        .arg(path)
        .output()
        .map_err(|e| format!("Impossible de lire la signature : {e}"))?;
    let stderr = String::from_utf8_lossy(&details.stderr);
    let identifier = signing_field(&stderr, "Identifier=")
        .ok_or_else(|| "Identifiant de signature absent".to_string())?;
    let team = signing_field(&stderr, "TeamIdentifier=")
        .ok_or_else(|| "Team ID absent : application signée ad hoc refusée".to_string())?;
    Ok((identifier, team))
}

fn validate_update_bundle(current: &Path, replacement: &Path) -> Result<(), String> {
    guard::validate_update_app_path(&current.to_string_lossy())?;
    if replacement
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("app")
    {
        return Err("Le téléchargement ne contient pas une application macOS".to_string());
    }

    let current_bundle = plist_str(&current.join("Contents/Info.plist"), "CFBundleIdentifier")
        .ok_or_else(|| "Bundle ID de l'application installée absent".to_string())?;
    let replacement_bundle = plist_str(
        &replacement.join("Contents/Info.plist"),
        "CFBundleIdentifier",
    )
    .ok_or_else(|| "Bundle ID de la mise à jour absent".to_string())?;
    if current_bundle != replacement_bundle {
        return Err(format!(
            "Bundle ID différent : {current_bundle} → {replacement_bundle}"
        ));
    }

    let (current_identifier, current_team) = app_signing_identity(current)?;
    let (replacement_identifier, replacement_team) = app_signing_identity(replacement)?;
    if current_identifier != replacement_identifier || current_team != replacement_team {
        return Err(
            "La signature de la mise à jour ne correspond pas à l'application installée"
                .to_string(),
        );
    }
    Ok(())
}

fn validate_archive_listing(listing: &str) -> Result<(), String> {
    use std::path::Component;

    let mut count = 0usize;
    for entry in listing.lines() {
        count += 1;
        if count > 50_000
            || entry.is_empty()
            || entry.len() > 4096
            || entry.contains('\\')
            || entry.chars().any(char::is_control)
        {
            return Err("Structure d'archive refusée".to_string());
        }
        let path = Path::new(entry);
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir
                        | Component::RootDir
                        | Component::Prefix(_)
                        | Component::CurDir
                )
            })
        {
            return Err(format!("Chemin dangereux dans l'archive : {entry}"));
        }
    }
    if count == 0 {
        return Err("Archive vide".to_string());
    }
    Ok(())
}

fn validate_archive_entries(archive: &Path, format: &str) -> Result<(), String> {
    let output = if format == "zip" {
        Command::new("/usr/bin/unzip")
            .arg("-Z1")
            .arg(archive)
            .output()
    } else {
        Command::new("/usr/bin/tar")
            .arg("-tf")
            .arg(archive)
            .output()
    }
    .map_err(|e| format!("Impossible d'inspecter l'archive : {e}"))?;
    if !output.status.success() {
        return Err("Archive illisible ou corrompue".to_string());
    }
    if output.stdout.len() > 8_000_000 {
        return Err("Archive contenant trop d'entrées".to_string());
    }

    let listing = String::from_utf8(output.stdout)
        .map_err(|_| "Noms de fichiers non UTF-8 dans l'archive".to_string())?;
    validate_archive_listing(&listing)?;

    let metadata = if format == "zip" {
        Command::new("/usr/bin/unzip")
            .arg("-ZTs")
            .arg(archive)
            .output()
    } else {
        Command::new("/usr/bin/tar")
            .arg("-tvf")
            .arg(archive)
            .output()
    }
    .map_err(|e| format!("Impossible d'inspecter les types d'entrées : {e}"))?;
    if !metadata.status.success() || metadata.stdout.len() > 16_000_000 {
        return Err("Métadonnées d'archive invalides".to_string());
    }
    for line in String::from_utf8_lossy(&metadata.stdout).lines() {
        if matches!(
            line.as_bytes().first(),
            Some(b'l' | b'h' | b'b' | b'c' | b'p' | b's')
        ) {
            return Err("Liens et fichiers spéciaux refusés dans les archives".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod update_security_tests {
    use super::{signing_field, validate_archive_listing};

    #[test]
    fn parses_codesign_identity_fields() {
        let output = "Executable=/Applications/App.app\nIdentifier=com.example.app\nTeamIdentifier=ABCDE12345\n";
        assert_eq!(
            signing_field(output, "Identifier=").as_deref(),
            Some("com.example.app")
        );
        assert_eq!(
            signing_field(output, "TeamIdentifier=").as_deref(),
            Some("ABCDE12345")
        );
    }

    #[test]
    fn rejects_archive_traversal_and_backslashes() {
        assert!(validate_archive_listing("App.app/Contents/MacOS/App\n").is_ok());
        assert!(validate_archive_listing("../escape\n").is_err());
        assert!(validate_archive_listing("App.app/../../escape\n").is_err());
        assert!(validate_archive_listing("..\\escape\n").is_err());
        assert!(validate_archive_listing("/absolute/path\n").is_err());
    }
}

fn copy_app(src: &str, destination: &Path) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "Destination de mise à jour invalide".to_string())?;
    let nonce = uuid::Uuid::new_v4();
    let staging = parent.join(format!(".burrow-update-{nonce}.app"));
    let backup = parent.join(format!(".burrow-backup-{nonce}.app"));

    let copy = Command::new("/usr/bin/ditto")
        .arg(src)
        .arg(&staging)
        .output()
        .map_err(|e| e.to_string())?;
    if !copy.status.success() {
        let _ = fs::remove_dir_all(&staging);
        let command = format!(
            "/bin/rm -rf {stage} && /usr/bin/ditto {source} {stage}",
            stage = guard::posix_shell_quote(&staging.to_string_lossy()),
            source = guard::posix_shell_quote(src),
        );
        run_admin_sh(&command)?;
    }

    if let Err(error) = validate_update_bundle(destination, &staging) {
        let cleanup = format!(
            "/bin/rm -rf {}",
            guard::posix_shell_quote(&staging.to_string_lossy())
        );
        if fs::remove_dir_all(&staging).is_err() {
            let _ = run_admin_sh(&cleanup);
        }
        return Err(error);
    }

    if fs::rename(destination, &backup).is_ok() {
        if fs::rename(&staging, destination).is_ok() {
            let _ = fs::remove_dir_all(&backup);
            return Ok(());
        }
        let _ = fs::rename(&backup, destination);
        let _ = fs::remove_dir_all(&staging);
        return Err("Remplacement atomique échoué ; application d'origine restaurée".to_string());
    }

    let destination_q = guard::posix_shell_quote(&destination.to_string_lossy());
    let staging_q = guard::posix_shell_quote(&staging.to_string_lossy());
    let backup_q = guard::posix_shell_quote(&backup.to_string_lossy());
    let command = format!(
        "/bin/mv {destination_q} {backup_q} && \
         if /bin/mv {staging_q} {destination_q}; then \
           /bin/rm -rf {backup_q}; \
         else \
           /bin/mv {backup_q} {destination_q}; exit 1; \
         fi"
    );
    run_admin_sh(&command)
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

        // Recalculer la mise à jour côté backend. Le frontend ne peut pas
        // substituer une URL ou une application arbitraire.
        let app_path = match authorize_sparkle_update(&name, &download_url, &app_path) {
            Ok(path) => path,
            Err(e) => {
                out!(format!("✗ Mise à jour refusée : {e}"));
                done!(false);
            }
        };

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
        let ok_dl = Command::new("/usr/bin/curl")
            .args([
                "-s",
                "-L",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--fail",
                "--max-time",
                "120",
                "--max-filesize",
                "2147483648",
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
        if !(500_000..=2_147_483_648).contains(&file_size) {
            out!("✗ Fichier trop petit — probable page d'erreur ou redirect invalide");
            let _ = fs::remove_file(&tmp);
            done!(false);
        }

        // Détecter le format depuis le contenu réel du fichier (pas l'URL)
        let mime = Command::new("/usr/bin/file")
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
            let is_dmg = Command::new("/usr/bin/hdiutil")
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
        if Command::new("/usr/bin/pgrep")
            .args(["-x", &name])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            out!(format!("Fermeture de {}…", name));
            quit_application(&name);
            std::thread::sleep(Duration::from_secs(1));
        }

        let install_dir = app_path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/Applications".to_string());

        let ok = match ext {
            "pkg" => {
                out!("✗ Les packages PKG ne peuvent pas être reliés de façon sûre à l'application installée");
                false
            }
            "zip" | "tar.gz" | "tar.bz2" | "tar.xz" => {
                let tmp_dir = work_dir
                    .path()
                    .join("extracted")
                    .to_string_lossy()
                    .into_owned();
                fs::create_dir_all(&tmp_dir).ok();
                if let Err(e) = validate_archive_entries(Path::new(&tmp), ext) {
                    out!(format!("✗ Archive refusée : {e}"));
                    done!(false);
                }
                out!("Extraction de l'archive…");
                let ok_x = if ext == "zip" {
                    Command::new("/usr/bin/unzip")
                        .args(["-q", "-o", &tmp, "-d", &tmp_dir])
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false)
                } else {
                    Command::new("/usr/bin/tar")
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
                let app_src = app_src.as_deref().unwrap();
                if let Err(e) = validate_update_bundle(&app_path, Path::new(app_src)) {
                    out!(format!("✗ Mise à jour non authentique : {e}"));
                    done!(false);
                }
                let result = match copy_app(app_src, &app_path) {
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
                let mount_out = Command::new("/usr/bin/hdiutil")
                    .args(["attach", &tmp, "-readonly", "-nobrowse"])
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
                    let _ = Command::new("/usr/bin/hdiutil")
                        .args(["detach", mp, "-quiet"])
                        .status();
                    let _ = fs::remove_file(&tmp);
                    done!(false);
                };
                if let Err(e) = validate_update_bundle(&app_path, Path::new(src)) {
                    out!(format!("✗ Mise à jour non authentique : {e}"));
                    let _ = Command::new("/usr/bin/hdiutil")
                        .args(["detach", mp, "-quiet"])
                        .status();
                    done!(false);
                }
                out!(format!("Copie de {} → {}…", src, install_dir));

                let result = match copy_app(src, &app_path) {
                    Ok(()) => true,
                    Err(e) => {
                        out!(format!("✗ Copie échouée : {}", e));
                        false
                    }
                };

                out!("Démontage…");
                let _ = Command::new("/usr/bin/hdiutil")
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
    let out = Command::new("/bin/df").args(["-k", path]).output().ok();
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
        let out = Command::new("/usr/bin/sudo")
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
        let mut cmd = Command::new("/usr/bin/sudo");
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
    current_user_record().map(|(name, _)| name)
}

/// One-time setup: admin dialog installs burrow-smc to fixed path + writes sudoers for
/// passwordless pmset, powermetrics (GPU), burrow-smc (fan control), and mole-smc (if present).
#[tauri::command]
fn setup_system_permissions(app: tauri::AppHandle) -> Result<(), String> {
    let username = current_username()?;

    let bundled = find_burrow_smc_bundled(&app).unwrap_or_else(|| BURROW_SMC_INSTALL.to_string());
    let mole_line = if std::path::Path::new(MOLE_SMC).exists() {
        format!(
            "{u} ALL=(root) NOPASSWD: {s} apply 0 0\\n\
{u} ALL=(root) NOPASSWD: {s} apply 1 60\\n",
            u = username,
            s = MOLE_SMC
        )
    } else {
        String::new()
    };

    let q_bundled = guard::posix_shell_quote(&bundled);
    let temp_sudoers = "/etc/sudoers.d/.burrow-pmset.tmp";
    let shell_cmd = format!(
        "/bin/mkdir -p /usr/local/lib && \
         /bin/cp -f {bundled} {smc} && \
         /bin/chmod 755 {smc} && \
         /bin/rm -f {tmp} && \
         /usr/bin/printf '{u} ALL=(root) NOPASSWD: /usr/bin/pmset -a lowpowermode 0\\n\
{u} ALL=(root) NOPASSWD: /usr/bin/pmset -a lowpowermode 1\\n\
{u} ALL=(root) NOPASSWD: /usr/bin/pmset -a lowpowermode 0 disksleep 0 sleep 0\\n\
{u} ALL=(root) NOPASSWD: /usr/bin/powermetrics -n 1 -i 200 --samplers smc\\n\
{u} ALL=(root) NOPASSWD: {smc} apply 0 0\\n\
{u} ALL=(root) NOPASSWD: {smc} apply 1 60\\n\
{mole}' > {tmp} && \
         /bin/chmod 440 {tmp} && \
         /usr/sbin/visudo -cf {tmp} && \
         /bin/mv -f {tmp} {p} && \
         /usr/bin/pmset -a lowpowermode 0",
        bundled = q_bundled,
        smc = BURROW_SMC_INSTALL,
        u = username,
        mole = mole_line,
        p = SUDOERS_PATH,
        tmp = temp_sudoers,
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
    let out = Command::new("/usr/bin/sudo")
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
        let is_dmg = Command::new("/usr/sbin/diskutil")
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
        if let Ok(out) = Command::new("/usr/bin/sudo")
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
    Command::new("/usr/bin/du")
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
    for cache in &result {
        grant_path(Path::new(&cache.path), PathGrantPurpose::Trash);
    }
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
    for artifact in &all {
        grant_path(Path::new(&artifact.artifact_path), PathGrantPurpose::Trash);
    }
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
    let Ok(out) = Command::new("/usr/bin/find").args(&args).output() else {
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
    for file in &files {
        grant_path(Path::new(&file.path), PathGrantPurpose::Trash);
    }
    files
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
    Command::new("/usr/bin/pgrep")
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
    let canonical = require_path_grant(&path, PathGrantPurpose::Trash).ok()?;
    read_image_preview_from_home(&canonical, &home_dir())
}

fn read_image_preview_from_home(path: &Path, home: &Path) -> Option<String> {
    use base64::Engine;
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::path::Component;

    const MAX_IMAGE_BYTES: u64 = 4_000_000;

    if home.as_os_str().is_empty() {
        return None;
    }
    if !path.is_absolute() || !path.starts_with(home) {
        return None;
    }

    // Refuser `..` et chaque symlink sous HOME, y compris dans un répertoire parent.
    // Vérifier une seconde fois après la lecture réduit la fenêtre de remplacement.
    let validate_components = || -> Option<()> {
        let relative = path.strip_prefix(home).ok()?;
        if relative.as_os_str().is_empty() {
            return None;
        }
        let mut current = home.to_path_buf();
        for component in relative.components() {
            let Component::Normal(part) = component else {
                return None;
            };
            current.push(part);
            if fs::symlink_metadata(&current)
                .ok()?
                .file_type()
                .is_symlink()
            {
                return None;
            }
        }
        Some(())
    };
    validate_components()?;

    let canonical_home = fs::canonicalize(home).ok()?;
    let canonical_path = fs::canonicalize(path).ok()?;
    if canonical_path == canonical_home || !canonical_path.starts_with(&canonical_home) {
        return None;
    }

    // O_NOFOLLOW protège aussi le dernier composant au moment exact de l'ouverture.
    // Une seule ouverture est utilisée pour la détection et la lecture complète.
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&canonical_path)
        .ok()?;
    let metadata_before = file.metadata().ok()?;
    if !metadata_before.is_file()
        || metadata_before.len() < 4
        || metadata_before.len() > MAX_IMAGE_BYTES
    {
        return None;
    }

    let mut bytes = Vec::with_capacity(metadata_before.len() as usize);
    std::io::Read::by_ref(&mut file)
        .take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 != metadata_before.len() || bytes.len() as u64 > MAX_IMAGE_BYTES {
        return None;
    }
    let mime = detect_image_mime(&bytes)?;

    // Revalidation TOCTOU sur le descripteur puis sur le chemin visible.
    let metadata_after = file.metadata().ok()?;
    if metadata_after.dev() != metadata_before.dev()
        || metadata_after.ino() != metadata_before.ino()
        || metadata_after.len() != metadata_before.len()
    {
        return None;
    }
    validate_components()?;
    let canonical_after = fs::canonicalize(path).ok()?;
    let path_metadata = fs::metadata(&canonical_after).ok()?;
    if canonical_after != canonical_path
        || path_metadata.dev() != metadata_before.dev()
        || path_metadata.ino() != metadata_before.ino()
        || path_metadata.len() != metadata_before.len()
    {
        return None;
    }

    Some(format!(
        "data:{};base64,{}",
        mime,
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}

#[cfg(test)]
mod image_preview_tests {
    use super::read_image_preview_from_home;
    use std::fs;
    use std::os::unix::fs::symlink;

    #[test]
    fn accepts_a_real_image_below_home() {
        let root = tempfile::tempdir().expect("tempdir");
        let home = root.path().join("home");
        fs::create_dir(&home).expect("home");
        let image = home.join("preview.bin");
        fs::write(&image, b"\x89PNG\r\n\x1a\ncontent").expect("image");

        let preview = read_image_preview_from_home(&image, &home).expect("valid preview");
        assert!(preview.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn rejects_a_final_symlink() {
        let root = tempfile::tempdir().expect("tempdir");
        let home = root.path().join("home");
        fs::create_dir(&home).expect("home");
        let real = home.join("real.png");
        let link = home.join("link.png");
        fs::write(&real, b"\x89PNG\r\n\x1a\ncontent").expect("image");
        symlink(&real, &link).expect("symlink");

        assert!(read_image_preview_from_home(&link, &home).is_none());
    }

    #[test]
    fn rejects_a_symlinked_parent_that_escapes_home() {
        let root = tempfile::tempdir().expect("tempdir");
        let home = root.path().join("home");
        let outside = root.path().join("outside");
        fs::create_dir(&home).expect("home");
        fs::create_dir(&outside).expect("outside");
        fs::write(outside.join("secret.png"), b"\x89PNG\r\n\x1a\nsecret").expect("image");
        symlink(&outside, home.join("alias")).expect("symlink");

        assert!(read_image_preview_from_home(&home.join("alias/secret.png"), &home).is_none());
    }

    #[test]
    fn enforces_the_size_limit_during_read() {
        let root = tempfile::tempdir().expect("tempdir");
        let home = root.path().join("home");
        fs::create_dir(&home).expect("home");
        let image = home.join("large.png");
        let file = fs::File::create(&image).expect("image");
        file.set_len(4_000_001).expect("size");

        assert!(read_image_preview_from_home(&image, &home).is_none());
    }
}

// ── Disk browser ──────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct DiskEntry {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub is_dir: bool,
}

#[derive(Serialize)]
pub struct DiskBreakdownResult {
    pub entries: Vec<DiskEntry>,
    pub truncated: bool,
}

/// Maximum number of direct children passed to `du` in a single call.
/// Prevents a large directory from spawning an unbounded argument list.
const MAX_DISK_CHILDREN: usize = 512;
const DISK_BROWSE_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_DU_OUTPUT_BYTES: u64 = 1_048_576;

fn run_du_with_timeout(
    args: &[std::ffi::OsString],
    timeout: Duration,
) -> Result<std::process::Output, String> {
    let mut child = Command::new("/usr/bin/du")
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("system:unable to start disk analysis: {e}"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "system:disk analysis stdout unavailable".to_string())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "system:disk analysis stderr unavailable".to_string())?;
    // Drain both pipes while the process runs. Waiting with unread pipes can
    // otherwise deadlock once a pipe buffer fills.
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout
            .by_ref()
            .take(MAX_DU_OUTPUT_BYTES)
            .read_to_end(&mut bytes);
        bytes
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr
            .by_ref()
            .take(MAX_DU_OUTPUT_BYTES)
            .read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = stdout_reader.join().unwrap_or_default();
                let stderr = stderr_reader.join().unwrap_or_default();
                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err("timeout:disk analysis took too long".to_string());
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(format!("system:unable to monitor disk analysis: {e}"));
            }
        }
    }
}

#[tauri::command]
fn get_disk_breakdown(path: String) -> Result<DiskBreakdownResult, String> {
    // Prevent concurrent invocations: a compromised frontend could otherwise
    // launch unbounded du(1) processes.
    let Some(_activity) = ActivityGuard::try_acquire(&DISK_BROWSE_ACTIVE) else {
        return Err("busy:disk browser is already running".to_string());
    };
    disk_breakdown_inner(path)
}

fn disk_breakdown_inner(path: String) -> Result<DiskBreakdownResult, String> {
    // Validate and canonicalize: uses browse-specific policy (allows home,
    // rejects system paths and sensitive subtrees, resolves all symlinks).
    let base = guard::validate_disk_browse_path(&path).map_err(|e| format!("protected:{e}"))?;

    let dir_entries = fs::read_dir(&base).map_err(|e| format!("inaccessible:{e}"))?;

    // Enumerate children:
    // - skip symlinks (detected on the lexical child path)
    // - canonicalize each entry and re-check forbidden zones
    // - cap at MAX_DISK_CHILDREN to bound the du argument list
    let mut truncated = false;
    let children: Vec<(PathBuf, guard::PathIdentity)> = dir_entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let metadata = fs::symlink_metadata(&p).ok()?;
            if metadata.file_type().is_symlink() {
                return None;
            }
            let canonical = fs::canonicalize(&p).ok()?;
            if guard::is_forbidden_for_readonly(&canonical) {
                return None;
            }
            let identity = guard::path_identity(&canonical).ok()?;
            Some((canonical, identity))
        })
        .take(MAX_DISK_CHILDREN + 1)
        .collect();

    let children = if children.len() > MAX_DISK_CHILDREN {
        truncated = true;
        children.into_iter().take(MAX_DISK_CHILDREN).collect()
    } else {
        children
    };

    if children.is_empty() {
        return Ok(DiskBreakdownResult {
            entries: Vec::new(),
            truncated,
        });
    }

    // Pass canonical paths to du(1).
    // -s : summarize (one line per argument)
    // -k : kilobytes
    // -P : do not follow symbolic links (explicit, macOS default is -P but
    //      we declare it to be safe against future flag changes)
    // -- : end of options, so paths starting with '-' are not misinterpreted
    let dir_paths: Vec<&(PathBuf, guard::PathIdentity)> =
        children.iter().filter(|(path, _)| path.is_dir()).collect();
    let mut size_map: HashMap<String, u64> = HashMap::new();

    if !dir_paths.is_empty() {
        let mut args: Vec<std::ffi::OsString> =
            vec!["-s".into(), "-k".into(), "-P".into(), "--".into()];
        // Avoid recursively counting sensitive descendants of an allowed
        // parent (for example ~/.ssh while calculating the home directory).
        args.truncate(3);
        for mask in guard::readonly_exclusion_names() {
            args.push("-I".into());
            args.push((*mask).into());
        }
        args.push("--".into());
        for (path, identity) in &dir_paths {
            guard::revalidate_path_identity(path, *identity)
                .map_err(|_| "changed:a directory changed during analysis".to_string())?;
            if guard::is_forbidden_for_readonly(path) {
                return Err("protected:a directory became protected".to_string());
            }
            args.push(path.as_os_str().into());
        }
        let out = run_du_with_timeout(&args, DISK_BROWSE_TIMEOUT)?;
        if !out.status.success() {
            return Err("system:disk analysis failed".to_string());
        }
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some((kb_str, path_str)) = line.split_once('\t') {
                if let Ok(kb) = kb_str.trim().parse::<u64>() {
                    size_map.insert(path_str.to_string(), kb.saturating_mul(1024));
                }
            }
        }
    }

    let mut entries: Vec<DiskEntry> = children
        .iter()
        .filter_map(|(p, identity)| {
            if guard::revalidate_path_identity(p, *identity).is_err() {
                return None;
            }
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
    Ok(DiskBreakdownResult { entries, truncated })
}

// ── Homebrew Cask Browser ─────────────────────────────────────────────────────

#[tauri::command]
async fn get_installed_casks() -> Vec<String> {
    tauri::async_runtime::spawn_blocking(|| {
        let home = home_dir().to_string_lossy().into_owned();
        let Some(brew) = find_brew() else {
            return Vec::new();
        };

        // `brew info --installed --cask --json=v2` returns metadata for every installed cask,
        // including the exact artifact (.app) names. We cross-check with the filesystem so
        // that apps deleted outside of brew (e.g. via UninstallTab) no longer appear.
        let info = Command::new(&brew)
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
        Command::new(&brew)
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
    guard::validate_brew_token(&token)?;
    if !cask_api().by_token.contains_key(&token) {
        return Err("Cask absent du catalogue Homebrew backend".to_string());
    }
    let brew = find_brew().ok_or_else(|| "Homebrew introuvable".to_string())?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        Command::new(brew)
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
    pub binary_count: usize,
    pub thinning_unsafe: bool,
    pub thinning_warning: String,
}

#[derive(Serialize)]
pub struct ThinResult {
    pub bytes_saved: u64,
    pub binary_count: usize,
    pub original_in_trash: bool,
    pub locally_resigned: bool,
}

fn is_fat_macho_candidate(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    if file.read_exact(&mut magic).is_err() {
        return false;
    }
    matches!(
        magic,
        [0xca, 0xfe, 0xba, 0xbe]
            | [0xbe, 0xba, 0xfe, 0xca]
            | [0xca, 0xfe, 0xba, 0xbf]
            | [0xbf, 0xba, 0xfe, 0xca]
    )
}

fn is_universal_binary(path: &Path) -> bool {
    if !is_fat_macho_candidate(path) {
        return false;
    }
    let Ok(out) = Command::new("/usr/bin/lipo")
        .args(["-archs"])
        .arg(path)
        .output()
    else {
        return false;
    };
    let arches = String::from_utf8_lossy(&out.stdout);
    arches.split_whitespace().any(|arch| arch == "x86_64")
        && arches.split_whitespace().any(|arch| arch == "arm64")
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

fn is_app_store_or_provisioned(app_path: &Path) -> bool {
    app_path.join("Contents/_MASReceipt/receipt").is_file()
        || app_path
            .join("Contents/embedded.provisionprofile")
            .is_file()
}

fn collect_universal_files(app_path: &Path) -> Vec<PathBuf> {
    const MAX_VISITED_FILES: usize = 100_000;
    let mut stack = vec![app_path.to_path_buf()];
    let mut visited = 0usize;
    let mut results = Vec::new();

    while let Some(directory) = stack.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > MAX_VISITED_FILES {
                return results;
            }
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file() && is_universal_binary(&path) {
                results.push(path);
            }
        }
    }
    results
}

fn scan_app_fatbinaries(app_path: &Path) -> Option<UniversalBinaryEntry> {
    let binaries = collect_universal_files(app_path);
    if binaries.is_empty() {
        return None;
    }
    let total_size_bytes = binaries
        .iter()
        .filter_map(|path| fs::metadata(path).ok())
        .map(|metadata| metadata.len())
        .sum::<u64>();
    let signature_error = verify_signed_code(app_path, true).err();
    let is_app_store_app = is_app_store_or_provisioned(app_path);
    let thinning_unsafe = signature_error.is_some() || is_app_store_app;
    if !thinning_unsafe {
        grant_path(app_path, PathGrantPurpose::Thin);
    }
    let thinning_warning = if is_app_store_app {
        "Application App Store ou provisionnée : l'identité de distribution est nécessaire à son fonctionnement. Amincissement refusé."
            .to_string()
    } else if thinning_unsafe {
        "Signature d’origine invalide ou absente. Réinstallez ou mettez à jour l’application avant de réessayer."
            .to_string()
    } else {
        "Burrow amincit une copie, la resigne localement si son enveloppe a changé, vérifie son intégrité, puis conserve l’original de l’éditeur dans la Corbeille."
            .to_string()
    };
    Some(UniversalBinaryEntry {
        name: app_path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Application")
            .to_string(),
        path: app_path.to_string_lossy().into_owned(),
        total_size_bytes,
        reclaimable_bytes: total_size_bytes / 2,
        binary_count: binaries.len(),
        thinning_unsafe,
        thinning_warning,
    })
}

fn is_scannable_app_bundle(app_path: &Path) -> bool {
    if app_path
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("app")
    {
        return false;
    }
    fs::symlink_metadata(app_path)
        .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

#[tauri::command]
async fn scan_universal_binaries() -> Vec<UniversalBinaryEntry> {
    tauri::async_runtime::spawn_blocking(|| {
        let mut results = Vec::new();
        let application_dirs = [
            PathBuf::from("/Applications"),
            home_dir().join("Applications"),
        ];
        for apps_dir in application_dirs {
            let Ok(entries) = fs::read_dir(apps_dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let app_path = entry.path();
                if !is_scannable_app_bundle(&app_path) {
                    continue;
                }
                let bundle_id = app_bundle_id(&app_path);
                if bundle_id.starts_with("com.apple.") || bundle_id == "com.karimachi.burrow" {
                    continue;
                }
                if let Some(entry) = scan_app_fatbinaries(&app_path) {
                    results.push(entry);
                }
            }
        }
        results.sort_by_key(|k| std::cmp::Reverse(k.reclaimable_bytes));
        results
    })
    .await
    .unwrap_or_default()
}

fn thin_file_to_arm64(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let parent = path
        .parent()
        .ok_or_else(|| "Dossier du binaire introuvable".to_string())?;
    let temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    let permissions = fs::metadata(path)
        .map_err(|error| error.to_string())?
        .permissions();
    let output = Command::new("/usr/bin/lipo")
        .arg(path)
        .args(["-thin", "arm64", "-output"])
        .arg(temporary.path())
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(permissions.mode()))
        .map_err(|error| error.to_string())?;
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|error| error.error.to_string())
}

fn verify_signed_code(path: &Path, deep: bool) -> Result<(), String> {
    let mut command = Command::new("/usr/bin/codesign");
    command.args(["--verify", "--strict"]);
    if deep {
        command.arg("--deep");
    }
    let output = command
        .arg(path)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn resign_thinned_app(path: &Path) -> Result<(), String> {
    let output = Command::new("/usr/bin/codesign")
        .args([
            "--force",
            "--deep",
            "--sign",
            "-",
            "--timestamp=none",
            "--preserve-metadata=identifier,entitlements,flags,runtime",
        ])
        .arg(path)
        .output()
        .map_err(|error| format!("Impossible de resigner la copie allégée : {error}"))?;
    if output.status.success() {
        verify_signed_code(path, true)
    } else {
        Err(format!(
            "La resignature locale a échoué : {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod universal_binary_tests {
    use super::*;

    #[test]
    fn scan_rejects_symlinked_application_roots() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temporary directory");
        let real_app = directory.path().join("Real.app");
        let linked_app = directory.path().join("Linked.app");
        let unrelated = directory.path().join("NotAnApp");
        fs::create_dir(&real_app).expect("create real app");
        fs::create_dir(&unrelated).expect("create unrelated directory");
        symlink(&real_app, &linked_app).expect("create app symlink");

        assert!(is_scannable_app_bundle(&real_app));
        assert!(!is_scannable_app_bundle(&linked_app));
        assert!(!is_scannable_app_bundle(&unrelated));
    }

    #[test]
    fn recognizes_app_store_and_provisioned_bundles() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let app_store = directory.path().join("Store.app");
        let provisioned = directory.path().join("Provisioned.app");
        let direct = directory.path().join("Direct.app");
        fs::create_dir_all(app_store.join("Contents/_MASReceipt"))
            .expect("create receipt directory");
        fs::write(app_store.join("Contents/_MASReceipt/receipt"), b"fixture")
            .expect("write receipt");
        fs::create_dir_all(provisioned.join("Contents")).expect("create provisioned app");
        fs::write(
            provisioned.join("Contents/embedded.provisionprofile"),
            b"fixture",
        )
        .expect("write provisioning profile");
        fs::create_dir_all(direct.join("Contents")).expect("create direct app");

        assert!(is_app_store_or_provisioned(&app_store));
        assert!(is_app_store_or_provisioned(&provisioned));
        assert!(!is_app_store_or_provisioned(&direct));
    }

    fn build_universal(source: &Path, binary: &Path) {
        fs::write(source, "int main(void) { return 0; }\n").expect("write fixture source");
        let build = Command::new("/usr/bin/xcrun")
            .args(["clang", "-arch", "arm64", "-arch", "x86_64"])
            .arg(source)
            .arg("-o")
            .arg(binary)
            .output()
            .expect("invoke clang");
        assert!(
            build.status.success(),
            "clang failed: {}",
            String::from_utf8_lossy(&build.stderr)
        );
    }

    fn sign(path: &Path) {
        let output = Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-"])
            .arg(path)
            .output()
            .expect("invoke codesign");
        assert!(
            output.status.success(),
            "codesign failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn removes_a_valid_intermediate_application_copy() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let prepared = directory.path().join(".burrow-thinned-fixture.app");
        fs::create_dir(&prepared).expect("create prepared app");
        fs::write(prepared.join("fixture"), b"temporary").expect("write prepared app");

        remove_prepared_app(&prepared).expect("remove prepared app");

        assert!(!prepared.exists());
    }

    #[test]
    fn refuses_to_remove_an_unrelated_path() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let unrelated = directory.path().join("Keep.app");
        fs::create_dir(&unrelated).expect("create unrelated app");

        assert!(remove_prepared_app(&unrelated).is_err());
        assert!(unrelated.exists());
    }

    #[test]
    fn detects_and_thins_a_real_universal_macho() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let source = directory.path().join("fixture.c");
        let binary = directory.path().join("fixture");
        build_universal(&source, &binary);
        assert!(is_universal_binary(&binary));

        sign(&binary);

        verify_signed_code(&binary, false).expect("verify universal fixture");
        thin_file_to_arm64(&binary).expect("thin fixture");
        let arches = Command::new("/usr/bin/lipo")
            .args(["-archs"])
            .arg(&binary)
            .output()
            .expect("inspect thinned fixture");
        assert!(arches.status.success());
        let arches = String::from_utf8_lossy(&arches.stdout);
        assert!(arches.split_whitespace().any(|arch| arch == "arm64"));
        assert!(!arches.split_whitespace().any(|arch| arch == "x86_64"));

        verify_signed_code(&binary, false).expect("verify thinned fixture");
    }

    #[test]
    fn keeps_a_nested_application_valid_after_thinning() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let app = directory.path().join("Fixture.app");
        let helper = app.join("Contents/Frameworks/Helper.app");
        let main_binary = app.join("Contents/MacOS/Fixture");
        let helper_binary = helper.join("Contents/MacOS/Helper");
        fs::create_dir_all(main_binary.parent().expect("main parent")).expect("create main dir");
        fs::create_dir_all(helper_binary.parent().expect("helper parent"))
            .expect("create helper dir");

        let plist = |identifier: &str, executable: &str| {
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
                 \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
                 <plist version=\"1.0\"><dict>\n\
                 <key>CFBundleIdentifier</key><string>{identifier}</string>\n\
                 <key>CFBundleExecutable</key><string>{executable}</string>\n\
                 <key>CFBundlePackageType</key><string>APPL</string>\n\
                 </dict></plist>\n"
            )
        };
        fs::write(
            app.join("Contents/Info.plist"),
            plist("com.burrow.fixture", "Fixture"),
        )
        .expect("write app plist");
        fs::write(
            helper.join("Contents/Info.plist"),
            plist("com.burrow.fixture.helper", "Helper"),
        )
        .expect("write helper plist");
        build_universal(&directory.path().join("main.c"), &main_binary);
        build_universal(&directory.path().join("helper.c"), &helper_binary);

        sign(&helper);
        sign(&app);
        verify_signed_code(&app, true).expect("verify original app");

        for binary in collect_universal_files(&app) {
            thin_file_to_arm64(&binary).expect("thin nested fixture");
        }

        if verify_signed_code(&app, true).is_err() {
            resign_thinned_app(&app).expect("resign thinned nested fixture");
        }
        verify_signed_code(&app, true).expect("verify thinned nested fixture");
    }
}

fn install_prepared_app(source: &Path, destination: &Path) -> Result<(), String> {
    let direct = Command::new("/usr/bin/ditto")
        .args(["--noqtn"])
        .arg(source)
        .arg(destination)
        .output()
        .map_err(|error| error.to_string())?;
    if direct.status.success() {
        return Ok(());
    }
    let source = guard::posix_shell_quote(&source.to_string_lossy());
    let destination = guard::posix_shell_quote(&destination.to_string_lossy());
    run_admin_sh(&format!("/usr/bin/ditto --noqtn {source} {destination}"))
}

fn remove_prepared_app(path: &Path) -> Result<(), String> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if !file_name.starts_with(".burrow-thinned-") || !file_name.ends_with(".app") {
        return Err("Chemin de copie intermédiaire invalide".to_string());
    }

    let direct = if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    if direct.is_ok() || !path.exists() {
        return Ok(());
    }

    let quoted = guard::posix_shell_quote(&path.to_string_lossy());
    run_admin_sh(&format!("/bin/rm -rf -- {quoted}"))?;
    if path.exists() {
        Err("La copie intermédiaire n'a pas pu être supprimée".to_string())
    } else {
        Ok(())
    }
}

#[tauri::command]
async fn thin_universal_app(
    app: tauri::AppHandle,
    name: String,
    app_path: String,
) -> Result<ThinResult, String> {
    guard::validate_app_uninstall_path(&app_path)?;
    let canonical = require_path_grant(&app_path, PathGrantPurpose::Thin)?;
    guard::validate_app_uninstall_path(&canonical.to_string_lossy())?;
    if canonical.file_stem().and_then(|value| value.to_str()) != Some(name.as_str()) {
        return Err("Le nom ne correspond pas à l'application analysée".to_string());
    }
    if is_app_store_or_provisioned(&canonical) {
        return Err(format!(
            "{name} provient de l’App Store ou utilise un profil de distribution. L’amincissement est refusé pour préserver son identité et ses autorisations."
        ));
    }
    verify_signed_code(&canonical, true).map_err(|_| {
        format!(
            "{name} n’est pas compatible avec l’amincissement sûr : sa signature d’origine est invalide ou absente. Aucune modification n’a été effectuée."
        )
    })?;
    if touchid_enabled(&app) {
        run_touchid(&app, &format!("Amincir {name}"))?;
    }

    tauri::async_runtime::spawn_blocking(move || {
        let before = du_bytes(&canonical);
        let temporary = tempfile::tempdir().map_err(|error| error.to_string())?;
        let staged = temporary.path().join(
            canonical
                .file_name()
                .ok_or_else(|| "Nom d'application invalide".to_string())?,
        );
        let copy = Command::new("/usr/bin/ditto")
            .args(["--noqtn"])
            .arg(&canonical)
            .arg(&staged)
            .output()
            .map_err(|error| error.to_string())?;
        if !copy.status.success() {
            return Err(String::from_utf8_lossy(&copy.stderr).trim().to_string());
        }
        verify_signed_code(&staged, true).map_err(|_| {
            "La copie de travail n’a pas conservé la signature d’origine. Aucune modification n’a été effectuée."
                .to_string()
        })?;

        let binaries = collect_universal_files(&staged);
        if binaries.is_empty() {
            return Err("Aucun binaire universel n'est encore présent".to_string());
        }
        for binary in &binaries {
            thin_file_to_arm64(binary)?;
        }
        let locally_resigned = if verify_signed_code(&staged, true).is_err() {
            resign_thinned_app(&staged).map_err(|error| {
                format!(
                    "{name} n'a pas pu être resigné localement après l'amincissement : {error}. Aucune modification n'a été effectuée."
                )
            })?;
            true
        } else {
            false
        };

        let parent = canonical
            .parent()
            .ok_or_else(|| "Dossier Applications introuvable".to_string())?;
        let prepared = parent.join(format!(".burrow-thinned-{}.app", uuid::Uuid::new_v4()));
        install_prepared_app(&staged, &prepared)?;
        if verify_signed_code(&prepared, true).is_err() {
            let cleanup = remove_prepared_app(&prepared);
            return Err(match cleanup {
                Ok(()) => "La copie préparée a échoué à la dernière vérification. L’application originale est intacte."
                    .to_string(),
                Err(error) => format!(
                    "La copie préparée a échoué à la dernière vérification. L’application originale est intacte, mais la copie intermédiaire doit être retirée manuellement : {error}"
                ),
            });
        }
        quit_application(&name);
        if let Err(error) = move_path_to_trash(&canonical) {
            let cleanup = remove_prepared_app(&prepared);
            if let Err(cleanup_error) = cleanup {
                return Err(format!(
                    "L'original n'a pas pu être sauvegardé dans la Corbeille : {error}. {cleanup_error}"
                ));
            }
            return Err(format!("L'original n'a pas pu être sauvegardé dans la Corbeille : {error}"));
        }

        let installed = fs::rename(&prepared, &canonical).or_else(|_| {
            let source = guard::posix_shell_quote(&prepared.to_string_lossy());
            let destination = guard::posix_shell_quote(&canonical.to_string_lossy());
            run_admin_sh(&format!("/bin/mv {source} {destination}"))
                .map_err(std::io::Error::other)
        });
        if let Err(error) = installed {
            let cleanup_error = remove_prepared_app(&prepared).err();
            activity::record(
                "applications",
                "Amincissement de binaire universel",
                "error",
                &format!("{name} — original conservé dans la Corbeille"),
                None,
                true,
            );
            let cleanup_status = cleanup_error
                .map(|cleanup| format!(" La copie intermédiaire doit aussi être retirée manuellement : {cleanup}"))
                .unwrap_or_default();
            return Err(format!(
                "La copie allégée n'a pas pu être installée. L'original reste dans la Corbeille : {error}.{cleanup_status}"
            ));
        }

        let after = du_bytes(&canonical);
        let bytes_saved = before.saturating_sub(after);
        activity::record(
            "applications",
            "Amincissement de binaire universel",
            "success",
            &format!("{name} — original conservé dans la Corbeille"),
            Some(bytes_saved),
            true,
        );
        Ok(ThinResult {
            bytes_saved,
            binary_count: binaries.len(),
            original_in_trash: true,
            locally_resigned,
        })
    })
    .await
    .map_err(|error| error.to_string())?
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
    for entry in &results {
        grant_path(Path::new(&entry.plist_path), PathGrantPurpose::LaunchItem);
    }
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
    if require_path_grant(&plist_path, PathGrantPurpose::LaunchItem).is_err() {
        return vec![];
    }
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
                grant_path(&path, PathGrantPurpose::LaunchItem);
                results.push(path_str);
            }
        }
    }
    results
}

#[tauri::command]
fn delete_launch_items(plist_paths: Vec<String>) -> Result<(), String> {
    let mut errors: Vec<String> = Vec::new();

    for p in &plist_paths {
        // Valider chaque chemin contre les répertoires LaunchAgents/LaunchDaemons connus
        let canonical = match require_path_grant(p, PathGrantPurpose::LaunchItem) {
            Ok(path) => path,
            Err(e) => {
                errors.push(format!("{}: {}", p, e));
                continue;
            }
        };
        let canonical_str = canonical.to_string_lossy().into_owned();
        if let Err(e) = guard::validate_launch_item_path(&canonical_str) {
            errors.push(format!("{}: {}", p, e));
            continue;
        }
        let unloaded = Command::new("/bin/launchctl")
            .args(["unload", "-w", &canonical_str])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
        if !unloaded && !canonical_str.starts_with(&home_dir().to_string_lossy().to_string()) {
            let quoted = guard::posix_shell_quote(&canonical_str);
            if let Err(error) = run_admin_sh(&format!("/bin/launchctl unload -w {quoted}")) {
                errors.push(format!("{}: {}", canonical.display(), error));
                continue;
            }
        }
        if let Err(error) = move_path_to_trash(&canonical) {
            errors.push(format!("{}: {}", canonical.display(), error));
        } else {
            let name = canonical
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("Launch Item");
            activity::record(
                "démarrage",
                "Élément déplacé dans la Corbeille",
                "success",
                name,
                None,
                true,
            );
        }
    }

    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    Ok(())
}

#[allow(dead_code)]
fn delete_login_item(plist_path: String) -> Result<(), String> {
    delete_launch_items(vec![plist_path])
}

#[tauri::command]
fn toggle_login_item(plist_path: String, enable: bool) -> Result<(), String> {
    let canonical = require_path_grant(&plist_path, PathGrantPurpose::LaunchItem)?;
    let plist_path = canonical.to_string_lossy().into_owned();
    guard::validate_launch_item_path(&plist_path)?;
    let action = if enable { "load" } else { "unload" };
    // Command::arg() — aucune interpolation shell
    let out = Command::new("/bin/launchctl")
        .args([action, "-w", &plist_path])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    // Fallback admin avec quoting POSIX correct
    let q = guard::posix_shell_quote(&plist_path);
    run_admin_sh(&format!("/bin/launchctl {} -w {}", action, q))
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
    let dscl_out = Command::new("/usr/bin/dscl")
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
    let results: Vec<PrivacyItem> = resolve_privacy_items(&home)
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
        .collect();
    for item in &results {
        grant_path(Path::new(&item.path), PathGrantPurpose::Trash);
    }
    results
}

#[tauri::command]
fn clean_privacy_items(ids: Vec<String>) -> Result<u64, String> {
    let home = home_dir();
    let mut freed: u64 = 0;
    for (id, label, p) in resolve_privacy_items(&home) {
        if !ids.iter().any(|x| x == &id) {
            continue;
        }
        if !p.exists() {
            continue;
        }
        let canonical = require_path_grant(&p.to_string_lossy(), PathGrantPurpose::Trash)?;
        guard::validate_trash_path(&canonical.to_string_lossy())?;
        let size = if p.is_dir() {
            du_mb(&p).saturating_mul(1024 * 1024)
        } else {
            fs::metadata(&p).map(|m| m.len()).unwrap_or(0)
        };
        move_path_to_trash(&canonical)?;
        freed = freed.saturating_add(size);
        activity::record(
            "confidentialité",
            "Données déplacées dans la Corbeille",
            "success",
            &label,
            Some(size),
            true,
        );
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

fn validate_admin_script(script: &str) -> Result<(), String> {
    if script
        .chars()
        .any(|character| matches!(character, '\n' | '\r' | '\0'))
    {
        return Err("Commande administrateur invalide".to_string());
    }
    Ok(())
}

fn run_admin_sh(script: &str) -> Result<(), String> {
    validate_admin_script(script)?;
    // -n : non-interactif — réussit uniquement si Touch ID / sudo_local est configuré,
    // échoue immédiatement sinon (pas de prompt terminal depuis une app GUI)
    let sudo_ok = Command::new("/usr/bin/sudo")
        .args(["-n", "/bin/sh", "-c", script])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if sudo_ok {
        return Ok(());
    }

    // Fallback : dialog mot de passe via osascript
    let esc = script.replace('\\', "\\\\").replace('"', "\\\"");
    let applescript = format!("do shell script \"{}\" with administrator privileges", esc);
    let output = Command::new("/usr/bin/osascript")
        .args(["-e", &applescript])
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[cfg(test)]
mod admin_command_tests {
    use super::*;

    #[test]
    fn rejects_control_characters_before_any_privileged_execution() {
        assert!(validate_admin_script("/bin/echo ok\n/bin/echo injected").is_err());
        assert!(validate_admin_script("/bin/echo ok\r/bin/echo injected").is_err());
        assert!(validate_admin_script("/bin/echo ok\0injected").is_err());
    }

    #[test]
    fn accepts_the_typed_maintenance_command_shape() {
        assert!(validate_admin_script(
            "/usr/bin/dscacheutil -flushcache; /usr/bin/killall -HUP mDNSResponder"
        )
        .is_ok());
    }
}

// ── Touch ID (helper arm64 précompilé et signé avec le bundle) ───────────────

fn touchid_binary(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let resource = app
        .path()
        .resource_dir()
        .map_err(|e| e.to_string())?
        .join("burrow-touchid");
    if resource.is_file() {
        return Ok(resource);
    }

    // Fallback de développement lorsque Tauri n'a pas encore copié les ressources.
    let development = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("burrow-touchid");
    if development.is_file() {
        Ok(development)
    } else {
        Err("Helper Touch ID embarqué introuvable".to_string())
    }
}

fn validate_touchid_reason(reason: &str) -> Result<(), String> {
    if reason.trim().is_empty() || reason.chars().count() > 200 || reason.contains('\0') {
        return Err("Motif Touch ID invalide".to_string());
    }
    Ok(())
}

fn run_touchid(app: &tauri::AppHandle, reason: &str) -> Result<(), String> {
    validate_touchid_reason(reason)?;
    let output = Command::new(touchid_binary(app)?)
        .arg(reason)
        .output()
        .map_err(|e| format!("Erreur d'exécution Touch ID : {e}"))?;
    match output.status.code() {
        Some(0) => Ok(()),
        Some(2) => Err("Touch ID non disponible sur cet appareil".to_string()),
        _ => Err("Authentification refusée ou annulée".to_string()),
    }
}

fn touchid_setting_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("security").join("touchid-enabled"))
        .map_err(|e| e.to_string())
}

fn touchid_enabled(app: &tauri::AppHandle) -> bool {
    touchid_setting_path(app)
        .ok()
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|value| value.trim() == "enabled")
        .unwrap_or(false)
}

fn write_touchid_setting(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let path = touchid_setting_path(app)?;
    let parent = path
        .parent()
        .ok_or_else(|| "Chemin de configuration Touch ID invalide".to_string())?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;

    if !enabled {
        return match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        };
    }

    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|e| e.to_string())?;
    temporary
        .write_all(b"enabled\n")
        .map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|e| e.error.to_string())
}

#[tauri::command]
fn check_touch_id_available(app: tauri::AppHandle) -> bool {
    touchid_binary(&app)
        .ok()
        .and_then(|binary| Command::new(binary).arg("--check").status().ok())
        .map(|status| status.success())
        .unwrap_or(false)
}

#[tauri::command]
fn get_touch_id_enabled(app: tauri::AppHandle) -> bool {
    touchid_enabled(&app)
}

#[tauri::command]
async fn set_touch_id_enabled(app: tauri::AppHandle, enable: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if touchid_enabled(&app) == enable {
            return Ok(());
        }
        let reason = if enable {
            "Activer la protection Touch ID de Burrow"
        } else {
            "Désactiver la protection Touch ID de Burrow"
        };
        run_touchid(&app, reason)?;
        write_touchid_setting(&app, enable)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ── Lancer au démarrage (LaunchAgent) ───────────────────────────────────────

#[tauri::command]
fn get_launch_at_login() -> bool {
    home_dir()
        .join("Library/LaunchAgents/com.burrow.app.plist")
        .exists()
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[tauri::command]
fn set_launch_at_login(enable: bool) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let launch_agents = home_dir().join("Library/LaunchAgents");
    fs::create_dir_all(&launch_agents).map_err(|e| e.to_string())?;
    let plist_path = launch_agents.join("com.burrow.app.plist");
    if enable {
        let exe = std::env::current_exe().map_err(|e| format!("Chemin introuvable : {}", e))?;
        let exe_str = xml_escape(&exe.to_string_lossy());
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
        let mut temp = tempfile::NamedTempFile::new_in(&launch_agents)
            .map_err(|e| format!("Création plist échouée : {e}"))?;
        std::io::Write::write_all(&mut temp, plist.as_bytes())
            .map_err(|e| format!("Écriture plist échouée : {e}"))?;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        temp.persist(&plist_path).map_err(|e| e.error.to_string())?;
        let output = Command::new("/bin/launchctl")
            .arg("load")
            .arg("-w")
            .arg(&plist_path)
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
    } else if plist_path.exists() {
        let _ = Command::new("/bin/launchctl")
            .arg("unload")
            .arg("-w")
            .arg(&plist_path)
            .output();
        std::fs::remove_file(&plist_path)
            .map_err(|e| format!("Suppression plist échouée : {}", e))?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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
            });

            // ── Menu bar widget ───────────────────────────────────────────────
            use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
            let widget_status = MenuItem::with_id(
                app,
                "widget-status",
                "CPU —  · Mémoire —  · Disque —",
                false,
                None::<&str>,
            )?;
            let widget_thermal = MenuItem::with_id(
                app,
                "widget-thermal",
                "Température —  · GPU —",
                false,
                None::<&str>,
            )?;
            let open_smart = MenuItem::with_id(
                app,
                "open-smart-scan",
                "Lancer Smart Scan…",
                true,
                None::<&str>,
            )?;
            let open_activity = MenuItem::with_id(
                app,
                "open-activity",
                "Ouvrir le journal d’activité",
                true,
                None::<&str>,
            )?;
            let open_burrow = MenuItem::with_id(
                app,
                "open-burrow",
                "Ouvrir Burrow",
                true,
                None::<&str>,
            )?;
            let quit_burrow = MenuItem::with_id(
                app,
                "quit-burrow",
                "Quitter Burrow",
                true,
                None::<&str>,
            )?;
            let tray_menu = Menu::with_items(
                app,
                &[
                    &widget_status,
                    &widget_thermal,
                    &PredefinedMenuItem::separator(app)?,
                    &open_smart,
                    &open_activity,
                    &open_burrow,
                    &PredefinedMenuItem::separator(app)?,
                    &quit_burrow,
                ],
            )?;
            let mut tray_builder = tauri::tray::TrayIconBuilder::new();
            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }
            let tray = tray_builder
                .title(" Burrow")
                .tooltip("Burrow — état du Mac")
                .menu(&tray_menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open-smart-scan" => show_main_window(app, Some("smart-scan")),
                    "open-activity" => show_main_window(app, Some("activity")),
                    "open-burrow" => show_main_window(app, None),
                    "quit-burrow" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::DoubleClick {
                        button: tauri::tray::MouseButton::Left,
                        ..
                    } = event {
                        show_main_window(tray.app_handle(), None);
                    }
                })
                .build(app)?;

            // Refresh the widget immediately, then every five seconds. Disk
            // notifications remain throttled and only transition at thresholds.
            std::thread::spawn(move || {
                loop {
                    let metrics = get_quick_metrics();
                    let disk_pct = metrics.disk_used_percent.round() as u8;
                    let menu_title = if disk_pct >= 90 {
                        format!(" 🔴 {disk_pct}%")
                    } else {
                        format!(" CPU {:.0}%", metrics.cpu_usage)
                    };
                    let _ = tray.set_title(Some(&menu_title));
                    let _ = widget_status.set_text(format!(
                        "CPU {:.0}%  · Mémoire {:.0}%  · Disque {disk_pct}%",
                        metrics.cpu_usage, metrics.mem_used_percent
                    ));
                    let temperature = if metrics.soc_temp > 0.0 {
                        metrics.soc_temp
                    } else {
                        metrics.cpu_temp
                    };
                    let _ = widget_thermal.set_text(format!(
                        "Température {:.0}°C  · GPU {:.0}%",
                        temperature, metrics.gpu_busy_percent
                    ));

                    let last = LAST_NOTIF_PCT.load(Ordering::Relaxed);
                    if disk_pct >= 90 && last < 90 {
                        let _ = Command::new("/usr/bin/osascript")
                            .args(["-e", "display notification \"Votre disque est rempli à plus de 90 %. Nettoyez maintenant.\" with title \"Burrow\""])
                            .output();
                        LAST_NOTIF_PCT.store(90, Ordering::Relaxed);
                    } else if disk_pct >= 80 && last < 80 {
                        let _ = Command::new("/usr/bin/osascript")
                            .args(["-e", "display notification \"Votre disque est rempli à 80 %.\" with title \"Burrow\""])
                            .output();
                        LAST_NOTIF_PCT.store(80, Ordering::Relaxed);
                    } else if disk_pct < 80 && last != 0 {
                        LAST_NOTIF_PCT.store(0, Ordering::Relaxed);
                    }

                    save_disk_sample_if_needed(metrics.disk_used, metrics.disk_total);
                    std::thread::sleep(Duration::from_secs(5));
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_apps,
            get_app_size,
            get_all_app_icons,
            get_mo_path,
            uninstall_app,
            get_system_metrics,
            list_installer_files,
            move_to_trash,
            check_full_disk_access,
            open_full_disk_access_settings,
            get_clean_sizes,
            run_smart_scan,
            start_smart_security_scan,
            run_clean_selection,
            run_optimize_selection,
            get_net_rates,
            free_memory,
            get_low_power_mode,
            set_low_power_mode,
            get_mo_version,
            get_brew_outdated,
            update_brew_app,
            find_app_residuals,
            get_all_processes,
            kill_process,
            gpu::get_gpu_info,
            check_sparkle_updates,
            check_app_store_updates,
            update_mas_app,
            open_app_store_url,
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
            check_touch_id_available,
            get_touch_id_enabled,
            set_touch_id_enabled,
            get_launch_at_login,
            set_launch_at_login,
            scan_universal_binaries,
            thin_universal_app,
            scan_login_items,
            scan_deleted_users,
            scan_privacy_items,
            clean_privacy_items,
            toggle_login_item,
            find_related_launch_items,
            delete_launch_items,
            get_purgeable_space,
            scan_tm_snapshots,
            scan_simulator_runtimes,
            scan_ai_caches,
            clean_ai_caches,
            scan_dev_caches,
            clean_dev_caches,
            activity::list_activity,
            activity::clear_activity,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
