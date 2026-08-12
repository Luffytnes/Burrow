use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Serialize, Clone)]
pub struct DuplicateGroup {
    pub hash: String,
    pub size_bytes: u64,
    pub paths: Vec<String>,
    pub wasted_bytes: u64,
}

fn hash_file(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(_) => return None,
        }
    }
    Some(format!("{:x}", hasher.finalize()))
}

fn hash_file_partial(path: &Path) -> Option<u64> {
    use std::io::Read;
    let mut file = fs::File::open(path).ok()?;
    let mut buf = [0u8; 65536];
    let n = file.read(&mut buf).ok()?;
    // FNV-1a 64-bit
    let mut h: u64 = 14695981039346656037;
    for &b in &buf[..n] {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    Some(h)
}

fn walk_files(dir: &Path, out: &mut Vec<(u64, String)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_symlink() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        if path.is_dir() {
            if matches!(
                name.as_str(),
                ".git"
                    | "node_modules"
                    | "target"
                    | ".build"
                    | ".Trash"
                    | "Library"
                    | "Applications"
            ) {
                continue;
            }
            walk_files(&path, out);
        } else if path.is_file() {
            if let Ok(meta) = fs::metadata(&path) {
                let size = meta.len();
                if size >= 4096 {
                    out.push((size, path.to_string_lossy().to_string()));
                }
            }
        }
    }
}

#[tauri::command]
pub fn find_duplicates(paths: Vec<String>) -> Vec<DuplicateGroup> {
    let scan_paths: Vec<String> = if paths.is_empty() {
        let home = crate::home_dir();
        [
            "Documents",
            "Downloads",
            "Desktop",
            "Movies",
            "Music",
            "Pictures",
            "Developer",
            "Code",
            "Projects",
        ]
        .iter()
        .map(|d| home.join(d))
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().to_string())
        .collect()
    } else {
        paths
    };

    use std::os::unix::fs::MetadataExt;

    let mut by_size: HashMap<u64, Vec<String>> = HashMap::new();
    for path_str in &scan_paths {
        let mut files = Vec::new();
        walk_files(Path::new(path_str), &mut files);
        for (size, path) in files {
            by_size.entry(size).or_default().push(path);
        }
    }

    // Phase 1: keep only size groups with >1 file
    let size_candidates: Vec<(u64, Vec<String>)> =
        by_size.into_iter().filter(|(_, v)| v.len() > 1).collect();

    // Phase 2: inode dedup — skip hard links (same inode = same file)
    let size_candidates: Vec<(u64, Vec<String>)> = size_candidates
        .into_iter()
        .map(|(sz, files)| {
            let mut seen_inodes: std::collections::HashSet<u64> = std::collections::HashSet::new();
            let unique: Vec<String> = files
                .into_iter()
                .filter(|f| {
                    let inode = fs::metadata(f).map(|m| m.ino()).unwrap_or(0);
                    seen_inodes.insert(inode)
                })
                .collect();
            (sz, unique)
        })
        .filter(|(_, v)| v.len() > 1)
        .collect();

    // Phase 3: partial hash (first 64 KB) to avoid hashing all same-size files
    let partial_map: Arc<Mutex<HashMap<String, Vec<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let handles: Vec<_> = size_candidates
        .into_iter()
        .flat_map(|(size, files)| {
            let arc = Arc::clone(&partial_map);
            files.into_iter().map(move |file| {
                let arc = Arc::clone(&arc);
                let sz = size;
                std::thread::spawn(move || {
                    if let Some(ph) = hash_file_partial(Path::new(&file)) {
                        let key = format!("{}:{}", sz, ph);
                        arc.lock().unwrap().entry(key).or_default().push(file);
                    }
                })
            })
        })
        .collect();
    for h in handles {
        let _ = h.join();
    }

    let partial = Arc::try_unwrap(partial_map)
        .map(|m| m.into_inner().unwrap_or_else(|p| p.into_inner()))
        .unwrap_or_default();

    // Phase 4: full SHA256 only on partial-hash groups with >1 file
    let candidates: Vec<(u64, Vec<String>)> = partial
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(key, files)| {
            let size = key
                .split(':')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0u64);
            (size, files)
        })
        .collect();

    type HashGroup = Arc<Mutex<HashMap<String, (u64, Vec<String>)>>>;
    let by_hash: HashGroup = Arc::new(Mutex::new(HashMap::new()));
    let handles: Vec<_> = candidates
        .into_iter()
        .flat_map(|(size, files)| {
            let outer_arc = Arc::clone(&by_hash);
            files.into_iter().map(move |file| {
                let by_hash = Arc::clone(&outer_arc);
                let size_val = size;
                std::thread::spawn(move || {
                    if let Some(hash) = hash_file(Path::new(&file)) {
                        let key = format!("{}:{}", size_val, &hash[..16]);
                        let mut map = by_hash.lock().unwrap();
                        let entry = map.entry(key).or_insert((size_val, Vec::new()));
                        entry.1.push(file);
                    }
                })
            })
        })
        .collect();
    for h in handles {
        let _ = h.join();
    }

    let map = match Arc::try_unwrap(by_hash) {
        Ok(m) => m
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        Err(_) => return Vec::new(),
    };
    let mut groups: Vec<DuplicateGroup> = map
        .into_iter()
        .filter(|(_, (_, paths))| paths.len() > 1)
        .map(|(key, (size_bytes, mut paths))| {
            paths.sort();
            let hash = key.split_once(':').map(|x| x.1).unwrap_or("").to_string();
            let wasted_bytes = size_bytes.saturating_mul(paths.len() as u64 - 1);
            DuplicateGroup {
                hash,
                size_bytes,
                paths,
                wasted_bytes,
            }
        })
        .collect();
    groups.sort_by_key(|k| std::cmp::Reverse(k.wasted_bytes));
    groups.truncate(500);
    for group in &groups {
        for path in &group.paths {
            crate::grant_path(Path::new(path), crate::PathGrantPurpose::Trash);
        }
    }
    groups
}
