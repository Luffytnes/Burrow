use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_DUPLICATE_FILES: usize = 100_000;
const MAX_SCAN_DEPTH: u8 = 32;
static DUPLICATE_SCAN_ACTIVE: AtomicBool = AtomicBool::new(false);

struct DuplicateScanGuard;

impl Drop for DuplicateScanGuard {
    fn drop(&mut self) {
        DUPLICATE_SCAN_ACTIVE.store(false, Ordering::Release);
    }
}

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
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .ok()?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(_) => return None,
        }
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = hasher.finalize();
    let mut hash = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hash.push(HEX[(byte >> 4) as usize] as char);
        hash.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Some(hash)
}

fn hash_file_partial(path: &Path) -> Option<u64> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .ok()?;
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

fn walk_files(dir: &Path, scan_root: &Path, out: &mut Vec<(u64, String)>, depth: u8) {
    if depth == 0 || out.len() >= MAX_DUPLICATE_FILES {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_DUPLICATE_FILES {
            break;
        }
        let lexical = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&lexical) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        let name = lexical
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        if matches!(
            name.as_str(),
            ".git" | "node_modules" | "target" | ".build" | ".Trash" | "Library" | "Applications"
        ) || crate::guard::readonly_exclusion_names().contains(&name.as_str())
        {
            continue;
        }

        let Ok(path) = fs::canonicalize(&lexical) else {
            continue;
        };
        if !path.starts_with(scan_root) || crate::guard::is_forbidden_for_readonly(&path) {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            walk_files(&path, scan_root, out, depth - 1);
        } else if metadata.is_file() && metadata.len() >= 4096 {
            out.push((metadata.len(), path.to_string_lossy().into_owned()));
        }
    }
}

#[tauri::command]
pub async fn find_duplicates() -> Vec<DuplicateGroup> {
    if DUPLICATE_SCAN_ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Vec::new();
    }
    tauri::async_runtime::spawn_blocking(|| {
        let _activity = DuplicateScanGuard;
        find_duplicates_inner()
    })
    .await
    .unwrap_or_default()
}

fn find_duplicates_inner() -> Vec<DuplicateGroup> {
    let home = crate::home_dir();
    let scan_paths: Vec<_> = [
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
    .map(|directory| home.join(directory))
    .filter(|path| path.exists())
    .filter_map(|path| crate::guard::validate_disk_browse_path(&path.to_string_lossy()).ok())
    .collect();

    use std::os::unix::fs::MetadataExt;

    let mut by_size: HashMap<u64, Vec<String>> = HashMap::new();
    let mut files = Vec::new();
    for scan_root in &scan_paths {
        walk_files(scan_root, scan_root, &mut files, MAX_SCAN_DEPTH);
        if files.len() >= MAX_DUPLICATE_FILES {
            break;
        }
    }
    for (size, path) in files {
        by_size.entry(size).or_default().push(path);
    }

    // Phase 1: keep only size groups with >1 file
    let size_candidates: Vec<(u64, Vec<String>)> =
        by_size.into_iter().filter(|(_, v)| v.len() > 1).collect();

    // Phase 2: inode dedup — skip hard links (same inode = same file)
    let size_candidates: Vec<(u64, Vec<String>)> = size_candidates
        .into_iter()
        .map(|(sz, files)| {
            let mut seen_inodes: std::collections::HashSet<(u64, u64)> =
                std::collections::HashSet::new();
            let unique: Vec<String> = files
                .into_iter()
                .filter(|f| {
                    fs::metadata(f)
                        .map(|metadata| seen_inodes.insert((metadata.dev(), metadata.ino())))
                        .unwrap_or(false)
                })
                .collect();
            (sz, unique)
        })
        .filter(|(_, v)| v.len() > 1)
        .collect();

    // Phase 3: partial hash (first 64 KB) to avoid hashing all same-size files
    let mut partial: HashMap<String, Vec<String>> = HashMap::new();
    for (size, files) in size_candidates {
        for file in files {
            if let Some(partial_hash) = hash_file_partial(Path::new(&file)) {
                partial
                    .entry(format!("{size}:{partial_hash}"))
                    .or_default()
                    .push(file);
            }
        }
    }

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

    let mut map: HashMap<String, (u64, Vec<String>)> = HashMap::new();
    for (size, files) in candidates {
        for file in files {
            if let Some(hash) = hash_file(Path::new(&file)) {
                let key = format!("{}:{}", size, &hash[..16]);
                map.entry(key).or_insert((size, Vec::new())).1.push(file);
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::{hash_file, walk_files, MAX_SCAN_DEPTH};
    use std::fs;
    use std::os::unix::fs::symlink;

    #[test]
    fn walker_skips_sensitive_names_and_symbolic_links() {
        let temp = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("tempdir");
        let root = fs::canonicalize(temp.path()).expect("root");
        let visible = root.join("Documents");
        let sensitive = root.join(".ssh");
        fs::create_dir(&visible).expect("visible");
        fs::create_dir(&sensitive).expect("sensitive");
        fs::write(visible.join("copy.bin"), vec![1u8; 4096]).expect("visible file");
        fs::write(sensitive.join("private.bin"), vec![1u8; 4096]).expect("private file");
        symlink(sensitive.join("private.bin"), root.join("alias.bin")).expect("symlink");

        let mut files = Vec::new();
        walk_files(&root, &root, &mut files, MAX_SCAN_DEPTH);

        assert_eq!(files.len(), 1);
        assert!(files[0].1.ends_with("Documents/copy.bin"));
        assert!(hash_file(&root.join("alias.bin")).is_none());
    }
}
