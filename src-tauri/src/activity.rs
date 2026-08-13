use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
const MAX_RETAINED_ENTRIES: usize = 1_000;
const MAX_RETURNED_ENTRIES: usize = 500;

fn activity_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub id: String,
    pub timestamp: u64,
    pub category: String,
    pub action: String,
    pub status: String,
    pub summary: String,
    pub bytes: Option<u64>,
    pub reversible: bool,
}

fn activity_dir() -> PathBuf {
    super::home_dir()
        .join("Library")
        .join("Application Support")
        .join("Burrow")
        .join("activity")
}

fn activity_path() -> PathBuf {
    activity_dir().join("activity.jsonl")
}

fn sanitize(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() || *c == '\t')
        .take(max_chars)
        .collect()
}

fn ensure_private_parent(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let Some(parent) = path.parent() else {
        return Err(std::io::Error::other("activity log parent missing"));
    };
    fs::create_dir_all(parent)?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
}

fn read_entries_from(path: &Path) -> Vec<ActivityEntry> {
    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<ActivityEntry>(&line).ok())
        .collect()
}

fn compact_if_needed(path: &Path) -> std::io::Result<()> {
    if fs::metadata(path).map(|m| m.len()).unwrap_or(0) <= MAX_LOG_BYTES {
        return Ok(());
    }

    let entries = read_entries_from(path);
    let start = entries.len().saturating_sub(MAX_RETAINED_ENTRIES);
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("activity log parent missing"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    for entry in &entries[start..] {
        serde_json::to_writer(&mut temporary, entry)?;
        temporary.write_all(b"\n")?;
    }
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|error| error.error)
}

pub fn record(
    category: &str,
    action: &str,
    status: &str,
    summary: &str,
    bytes: Option<u64>,
    reversible: bool,
) {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let Ok(_guard) = activity_lock().lock() else {
        return;
    };
    let path = activity_path();
    if ensure_private_parent(&path).is_err() {
        return;
    }
    let entry = ActivityEntry {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0),
        category: sanitize(category, 40),
        action: sanitize(action, 80),
        status: sanitize(status, 20),
        summary: sanitize(summary, 500),
        bytes,
        reversible,
    };

    let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&path)
    else {
        return;
    };
    let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
    if serde_json::to_writer(&mut file, &entry).is_ok() {
        let _ = file.write_all(b"\n");
        let _ = file.flush();
        let _ = compact_if_needed(&path);
    }
}

#[tauri::command]
pub fn list_activity(limit: Option<usize>) -> Vec<ActivityEntry> {
    let Ok(_guard) = activity_lock().lock() else {
        return Vec::new();
    };
    let limit = limit.unwrap_or(200).clamp(1, MAX_RETURNED_ENTRIES);
    let mut entries = read_entries_from(&activity_path());
    entries.reverse();
    entries.truncate(limit);
    entries
}

#[tauri::command]
pub fn clear_activity() -> Result<(), String> {
    let path = activity_path();
    {
        let _guard = activity_lock()
            .lock()
            .map_err(|_| "Journal d’activité indisponible".to_string())?;
        if !path.exists() {
            return Ok(());
        }
        super::move_path_to_trash(&path)?;
    }
    record(
        "journal",
        "Journal précédent déplacé dans la Corbeille",
        "success",
        "L'historique peut être restauré depuis Finder",
        None,
        true,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_control_characters_and_limits_length() {
        let input = format!("hello\nworld{}", "x".repeat(600));
        let output = sanitize(&input, 20);
        assert!(!output.contains('\n'));
        assert_eq!(output.chars().count(), 20);
    }

    #[test]
    fn reads_json_lines_and_ignores_corrupt_records() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("activity.jsonl");
        let entry = ActivityEntry {
            id: "one".into(),
            timestamp: 1,
            category: "clean".into(),
            action: "trash".into(),
            status: "success".into(),
            summary: "done".into(),
            bytes: Some(42),
            reversible: true,
        };
        let valid = serde_json::to_string(&entry).expect("json");
        fs::write(&path, format!("{valid}\nnot-json\n")).expect("write");
        let entries = read_entries_from(&path);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "one");
    }
}
