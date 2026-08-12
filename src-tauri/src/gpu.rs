use serde::Serialize;
use std::process::Command;

#[derive(Serialize)]
pub struct GpuInfo {
    pub model: String,
    pub vram_mb: u64,
    pub vendor: String,
}

#[tauri::command]
pub fn get_gpu_info() -> GpuInfo {
    let default = GpuInfo {
        model: String::new(),
        vram_mb: 0,
        vendor: String::new(),
    };

    let out = match Command::new("system_profiler")
        .args(["SPDisplaysDataType", "-json"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return default,
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return default,
    };
    let gpu = match v["SPDisplaysDataType"].as_array().and_then(|a| a.first()) {
        Some(g) => g.clone(),
        None => return default,
    };

    let model_base = gpu["sppci_model"].as_str().unwrap_or("").to_string();

    // On M1/M2/M3: key is "spdisplays_vendor" with value "sppci_vendor_Apple"
    let vendor_raw = gpu["spdisplays_vendor"]
        .as_str()
        .or_else(|| gpu["sppci_vendor"].as_str())
        .unwrap_or("");
    let vendor = vendor_raw
        .strip_prefix("sppci_vendor_")
        .unwrap_or(vendor_raw)
        .to_string();

    // GPU cores (Apple Silicon only)
    let cores = gpu["sppci_cores"].as_str().unwrap_or("");
    let model = if !cores.is_empty() {
        format!("{} · {} cœurs GPU", model_base, cores)
    } else {
        model_base
    };

    // VRAM: discrete GPU has a value; Apple Silicon uses unified memory
    let vram_str = gpu["spdisplays_vram"]
        .as_str()
        .or_else(|| gpu["spdisplays_vram_shared"].as_str())
        .unwrap_or("");
    let vram_mb = {
        let lower = vram_str.to_lowercase();
        if lower.is_empty() || lower.contains("shared") || lower.contains("partagée") {
            0u64
        } else {
            let num: u64 = lower
                .split_whitespace()
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            if lower.contains("gb") {
                num * 1024
            } else {
                num
            }
        }
    };

    GpuInfo {
        model,
        vram_mb,
        vendor,
    }
}
