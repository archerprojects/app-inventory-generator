// scanner.rs — System package enumeration
// Developed for Lean Linux by archerprojects (archer.projects@proton.me)
// https://github.com/archerprojects/app-inventory-generator
//
// Four package categories:
//   menu     — apps with .desktop files (non-Flatpak), blacklisted entries excluded
//   terminal — explicitly user-installed CLI packages, no .desktop entry
//   flatpak  — all Flatpak apps installed on the system
//   snap     — all Snap apps installed on the system (excludes base/platform snaps)
//
// All packages default to unselected — user builds list deliberately.
// Distro detected via /etc/os-release. Missing tools skipped cleanly.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Source {
    Apt,
    Dnf,
    Pacman,
    Zypper,
    Flatpak,
    Snap,
    Local,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Package {
    /// Actual package name used for resolution and script generation.
    pub name: String,
    /// Human-readable display name shown in the UI.
    pub display_name: String,
    pub version: String,
    pub source: Source,
    pub flatpak_id: Option<String>,
    /// Detected config path for this app — shown in UI to help user locate settings.
    pub config_path: Option<String>,
    /// Resolution result for the currently selected target distro.
    /// Populated during scan after target distro is chosen. None until resolved.
    #[serde(skip)]
    pub resolution: Option<crate::resolver::Resolution>,
    pub selected: bool,
}

#[derive(Debug, Default, Clone)]
pub struct PackageList {
    /// Apps with .desktop entries (non-Flatpak).
    pub menu: Vec<Package>,
    /// User-installed CLI/terminal packages with no menu entry.
    pub terminal: Vec<Package>,
    /// All Flatpak apps installed on the system.
    pub flatpak: Vec<Package>,
    /// All Snap apps installed on the system.
    pub snap: Vec<Package>,
}

impl PackageList {
    pub fn all_packages(&self) -> impl Iterator<Item = &Package> {
        self.menu.iter()
            .chain(self.terminal.iter())
            .chain(self.flatpak.iter())
            .chain(self.snap.iter())
    }
}

// ---------------------------------------------------------------------------
// Distro detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Distro {
    Debian,
    Fedora,
    Arch,
    OpenSuse,
    Unknown,
}

pub fn detect_distro() -> Distro {
    let content = fs::read_to_string("/etc/os-release").unwrap_or_default();

    let id = content.lines()
        .find(|l| l.starts_with("ID="))
        .map(|l| l["ID=".len()..].trim().trim_matches('"').to_lowercase())
        .unwrap_or_default();

    let distro = match id.as_str() {
        "ubuntu" | "debian" | "linuxmint" | "pop" | "elementary" | "zorin" => Distro::Debian,
        "fedora" | "rhel" | "rocky" | "almalinux" | "centos" => Distro::Fedora,
        "arch" | "manjaro" | "endeavouros" | "garuda" => Distro::Arch,
        s if s.starts_with("opensuse") => Distro::OpenSuse,
        _ => Distro::Unknown,
    };

    if distro != Distro::Unknown {
        return distro;
    }

    // Fallback to ID_LIKE
    content.lines()
        .find(|l| l.starts_with("ID_LIKE="))
        .map(|l| l["ID_LIKE=".len()..].trim().trim_matches('"').to_lowercase())
        .map(|like| {
            if like.contains("debian") || like.contains("ubuntu") { Distro::Debian }
            else if like.contains("fedora") || like.contains("rhel") { Distro::Fedora }
            else if like.contains("arch") { Distro::Arch }
            else if like.contains("suse") { Distro::OpenSuse }
            else { Distro::Unknown }
        })
        .unwrap_or(Distro::Unknown)
}

// ---------------------------------------------------------------------------
// Blacklists
// ---------------------------------------------------------------------------

/// Package names that should never appear in the menu tab.
/// Distro-specific tools, hardware drivers, and system utilities
/// that don't make sense to migrate to another system.
fn is_blacklisted_package(name: &str) -> bool {
    let blacklist = [
        // Nvidia — hardware specific
        "nvidia", "nvidia-settings", "nvidia-prime-applet",
        // Mint-specific
        "mint-", "mintupdate", "mintinstall", "mintdrivers",
        "mintwelcome", "mintreport", "mintsources", "mintlocale",
        "mintbackup", "mintstick", "mintsystem", "mintmenu",
        "mint-info-", "mint-meta-", "mint-artwork", "mint-common",
        // Display manager / greeter — system specific
        "lightdm", "slick-greeter", "lightdm-settings",
        // Hardware / system drivers
        "driver-manager", "ubuntu-drivers",
        // Distro welcome / setup screens
        "gnome-initial-setup", "ubiquity", "calamares",
    ];
    blacklist.iter().any(|b| name.starts_with(b) || name == *b)
}

/// Desktop file names (stems) that should be excluded from menu scan.
/// These are system panels, settings applets, and DE-specific tools
/// that don't belong in a migration script.
fn is_blacklisted_desktop(stem: &str) -> bool {
    let blacklist = [
        "cinnamon", "cinnamon2d", "cinnamon-killer-daemon",
        "cinnamon-settings", "cinnamon-color-panel",
        "cinnamon-display-panel", "cinnamon-menu-editor",
        "cinnamon-network-panel", "cinnamon-onscreen-keyboard",
        "cinnamon-screensaver-command",
        "gnome-control-center", "gnome-session-properties",
        "software-properties-gtk", "software-properties-kde",
        "update-manager", "mintupdate", "mintwelcome",
        "mintinstall", "mintdrivers", "mintreport",
        "mintsources", "mintlocale", "mintbackup",
        "mintstick", "webapp-manager",
        "nvidia-settings", "nvidia-prime-applet",
        "lightdm-settings", "slick-greeter",
        "im-config", "ibus", "ibus-setup",
        "nm-connection-editor",
    ];
    blacklist.iter().any(|b| stem.starts_with(b) || stem == *b)
}

// ---------------------------------------------------------------------------
// Main scan
// ---------------------------------------------------------------------------

pub fn scan_system() -> PackageList {
    let distro = detect_distro();
    let home = home_dir();

    // Flatpak — independent of distro
    let flatpak = scan_flatpak();
    let flatpak_ids: HashSet<String> = flatpak
        .iter()
        .filter_map(|p| p.flatpak_id.clone())
        .collect();

    // Menu apps from .desktop files (non-Flatpak)
    let menu = scan_desktop_files(&home, &flatpak_ids);
    let menu_names: HashSet<String> = menu.iter().map(|p| p.name.clone()).collect();
    let flatpak_names: HashSet<String> = flatpak.iter().map(|p| p.name.clone()).collect();

    // Terminal — user-installed CLI packages, no .desktop, no Flatpak
    let terminal = scan_terminal(&distro, &menu_names, &flatpak_names);

    // Snap — independent of distro
    let snap = scan_snap();

    PackageList { menu, terminal, flatpak, snap }
}

// ---------------------------------------------------------------------------
// Desktop file scan — menu apps (non-Flatpak)
// ---------------------------------------------------------------------------

fn scan_desktop_files(
    home: &Option<PathBuf>,
    flatpak_ids: &HashSet<String>,
) -> Vec<Package> {
    let mut locations: Vec<(PathBuf, bool)> = vec![
        (PathBuf::from("/usr/share/applications"), false),
    ];
    if let Some(h) = home {
        locations.push((h.join(".local/share/applications"), true));
    }

    let mut packages: Vec<Package> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for (dir, is_local) in &locations {
        if !dir.exists() { continue; }
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }

            // Skip blacklisted desktop file stems
            let stem = path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if is_blacklisted_desktop(stem) { continue; }

            if let Some(pkg) = parse_desktop_file(&path, *is_local) {
                // Skip if it's actually a Flatpak app
                if flatpak_ids.iter().any(|id| id.contains(&pkg.name)) { continue; }
                // Skip blacklisted packages
                if is_blacklisted_package(&pkg.name) { continue; }
                if seen.insert(pkg.name.clone()) {
                    packages.push(pkg);
                }
            }
        }
    }

    packages.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    packages
}

/// Parse a .desktop file.
/// Resolution order for package name:
///   1. X-Flatpak= present → mark as Flatpak, return None (handled by Flatpak tab)
///   2. TryExec= binary → dpkg -S (most reliable)
///   3. Exec= binary → dpkg -S
///   4. Icon= name → use directly as package name if no dots
///   5. .desktop filename stem → final fallback
/// Rejects any dpkg result containing 2+ dots (D-Bus/Flatpak ID leaked in).
fn parse_desktop_file(path: &Path, is_local: bool) -> Option<Package> {
    let content = fs::read_to_string(path).ok()?;
    let mut display_name = None;
    let mut exec_line = None;
    let mut try_exec = None;
    let mut icon = None;
    let mut flatpak_id = None;
    let mut no_display = false;
    let mut only_show_in = false;
    let mut in_entry = false;

    for line in content.lines() {
        if line.trim() == "[Desktop Entry]" { in_entry = true; continue; }
        if line.starts_with('[') { in_entry = false; continue; }
        if !in_entry { continue; }

        if line.starts_with("Name=") && !line.starts_with("Name[") {
            display_name = Some(line["Name=".len()..].trim().to_string());
        }
        if line.starts_with("Exec=") {
            exec_line = Some(line["Exec=".len()..].trim().to_string());
        }
        if line.starts_with("TryExec=") {
            try_exec = Some(line["TryExec=".len()..].trim().to_string());
        }
        if line.starts_with("Icon=") {
            icon = Some(line["Icon=".len()..].trim().to_string());
        }
        if line.starts_with("X-Flatpak=") {
            flatpak_id = Some(line["X-Flatpak=".len()..].trim().to_string());
        }
        if line == "NoDisplay=true" { no_display = true; }
        if line.starts_with("OnlyShowIn=") { only_show_in = true; }
    }

    if no_display || only_show_in { return None; }
    let display_name = display_name?;

    // If X-Flatpak= is present, this belongs in the Flatpak tab — skip here
    if flatpak_id.is_some() { return None; }

    let stem = path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    let pkg_name = if is_local {
        stem.clone()
    } else {
        // Try TryExec first — cleanest binary reference
        let from_try_exec = try_exec.as_ref().and_then(|te| {
            let te = te.trim().to_string();
            // Preserve absolute path so local install detection works
            dpkg_owner_validated(&te)
        });

        if let Some(name) = from_try_exec {
            name
        } else {
            // Try Exec= binary
            let from_exec = exec_line.as_ref().and_then(|exec| {
                let first = exec.split_whitespace().next().unwrap_or("").trim().to_string();
                if first.is_empty() { return None; }
                // Preserve absolute path so local install detection works
                dpkg_owner_validated(&first)
            });

            if let Some(name) = from_exec {
                name
            } else {
                // Try Icon= name if it looks like a package name (no dots)
                let from_icon = icon.as_ref().and_then(|ic| {
                    if !ic.contains('.') && !ic.is_empty() {
                        Some(ic.clone())
                    } else {
                        None
                    }
                });
                from_icon.unwrap_or_else(|| stem.clone())
            }
        }
    };

    // Detect config path
    let config_path = detect_config_path(&pkg_name, None);

    let source = if is_local { Source::Local } else { Source::Apt };

    Some(Package {
        name: pkg_name,
        display_name,
        version: String::new(),
        source,
        flatpak_id: None,
        config_path,
        resolution: None,
        selected: false,
    })
}

/// dpkg -S lookup with exact path matching.
/// Searches /usr/bin/<binary> first, then /usr/libexec/<binary>.
/// Rejects results containing 2+ dots (D-Bus/Flatpak ID leaks).
/// Returns None if binary path is outside system dirs (user-local install).
fn dpkg_owner_validated(binary: &str) -> Option<String> {
    // If binary is an absolute path outside system dirs, it's a local install
    if binary.starts_with('/') {
        let system_prefixes = ["/usr/", "/bin/", "/sbin/", "/opt/"];
        if !system_prefixes.iter().any(|p| binary.starts_with(p)) {
            return None; // local install — skip dpkg lookup
        }
    }

    // Strip path if absolute — get just the binary name
    let bin_name = binary.split('/').last().unwrap_or(binary).trim();
    if bin_name.is_empty() { return None; }

    // Search exact paths to avoid substring matches
    let search_paths = [
        format!("/usr/bin/{}", bin_name),
        format!("/usr/libexec/{}", bin_name),
        format!("/usr/local/bin/{}", bin_name),
        format!("/bin/{}", bin_name),
    ];

    for search_path in &search_paths {
        let output = match Command::new("dpkg")
            .args(["-S", search_path])
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => continue,
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let pkg = match stdout.lines().next() {
            Some(line) => line.split(':').next().unwrap_or("").trim().to_string(),
            None => continue,
        };

        if pkg.is_empty() { continue; }

        // Reject D-Bus names and Flatpak IDs (2+ dots)
        if pkg.chars().filter(|&c| c == '.').count() >= 2 { continue; }

        return Some(pkg);
    }

    None
}

/// Detect where an app's config files live.
/// Checks multiple name variations since config dir rarely matches package name exactly.
fn detect_config_path(pkg_name: &str, flatpak_id: Option<&str>) -> Option<String> {
    let home = std::env::var("HOME").ok()?;

    if let Some(fid) = flatpak_id {
        // Flatpak — always in ~/.var/app/<id>/config, show even if not yet created
        let path = format!("{}/.var/app/{}/config", home, fid);
        if std::path::Path::new(&path).exists() {
            return Some(format!("~/.var/app/{}/config", fid));
        }
        return Some(format!("~/.var/app/{}/config", fid));
    }

    // Build list of candidate names to try
    // Package names often differ from config dir names:
    //   transmission-gtk → transmission
    //   gnome-calculator → gnome-calculator or Calculator
    //   libreoffice-common → libreoffice
    let candidates: Vec<String> = {
        let mut v = vec![pkg_name.to_string()];

        // Strip common suffixes
        for suffix in &["-gtk", "-qt", "-gnome", "-kde", "-common", "-bin", "-utils"] {
            if let Some(stripped) = pkg_name.strip_suffix(suffix) {
                v.push(stripped.to_string());
            }
        }

        // Strip version suffixes like -7.2
        if let Some(pos) = pkg_name.rfind('-') {
            let after = &pkg_name[pos + 1..];
            if after.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                v.push(pkg_name[..pos].to_string());
            }
        }

        v
    };

    // Check ~/.config/<candidate> and ~/.local/share/<candidate>
    for candidate in &candidates {
        let xdg = format!("{}/.config/{}", home, candidate);
        if std::path::Path::new(&xdg).exists() {
            return Some(format!("~/.config/{}", candidate));
        }

        let local = format!("{}/.local/share/{}", home, candidate);
        if std::path::Path::new(&local).exists() {
            return Some(format!("~/.local/share/{}", candidate));
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Flatpak — own tab
// ---------------------------------------------------------------------------

fn scan_flatpak() -> Vec<Package> {
    if !cmd_exists("flatpak") { return vec![]; }

    let out = run("flatpak", &["list", "--app", "--columns=application,name,version"]);
    let mut packages = Vec::new();

    for line in out.lines() {
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() < 2 { continue; }
        let flatpak_id = parts[0].trim().to_string();
        let display_name = parts[1].trim().to_string();
        let version = parts.get(2).unwrap_or(&"").trim().to_string();
        let config_path = detect_config_path(&flatpak_id, Some(&flatpak_id));
        packages.push(Package {
            name: flatpak_id.clone(),
            display_name,
            version,
            source: Source::Flatpak,
            flatpak_id: Some(flatpak_id),
            config_path,
            resolution: None,
            selected: false,
        });
    }

    packages.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    packages
}

// ---------------------------------------------------------------------------
// Snap — own tab
// ---------------------------------------------------------------------------

fn scan_snap() -> Vec<Package> {
    if !cmd_exists("snap") { return vec![]; }

    // snap list columns: Name  Version  Rev  Tracking  Publisher  Notes
    let out = run("snap", &["list"]);
    let mut packages = Vec::new();
    let home = std::env::var("HOME").unwrap_or_default();

    for line in out.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.is_empty() { continue; }

        let name = cols[0].trim().to_string();
        if is_snap_base(&name) { continue; }

        let version = cols.get(1).unwrap_or(&"").to_string();
        // Notes is the last column; classic-confined snaps carry "classic".
        let classic = cols.last().map(|n| n.contains("classic")).unwrap_or(false);

        // Snap configs live under ~/snap/<name>/current/
        let cfg = format!("{}/snap/{}/current", home, name);
        let config_path = if !home.is_empty() && Path::new(&cfg).exists() {
            Some(format!("~/snap/{}/current", name))
        } else {
            None
        };

        packages.push(Package {
            name: name.clone(),
            display_name: name.clone(),
            version,
            source: Source::Snap,
            flatpak_id: None,
            config_path,
            // Snaps are universal — resolved at scan, no Repology lookup needed.
            resolution: Some(crate::resolver::Resolution::Snap { name, classic }),
            selected: false,
        });
    }

    packages.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    packages
}

/// Base/platform snaps that aren't user-facing apps — excluded from the tab.
fn is_snap_base(name: &str) -> bool {
    matches!(name, "snapd" | "core" | "bare")
        || name.starts_with("core1")            // core18
        || name.starts_with("core2")            // core20 / core22 / core24
        || name.starts_with("gtk-common-themes")
        || name.starts_with("kde-frameworks")
        || name.starts_with("mesa-")
        // gnome-<digits>-<digits> platform snaps (e.g. gnome-46-2404)
        || (name.starts_with("gnome-")
            && name.chars().nth(6).map(|c| c.is_ascii_digit()).unwrap_or(false))
}

// ---------------------------------------------------------------------------
// Terminal apps — user-installed CLI packages
// ---------------------------------------------------------------------------

fn scan_terminal(
    distro: &Distro,
    menu_names: &HashSet<String>,
    flatpak_names: &HashSet<String>,
) -> Vec<Package> {
    let raw = match distro {
        Distro::Debian   => apt_mark_showmanual(),
        Distro::Fedora   => dnf_userinstalled(),
        Distro::Arch     => pacman_explicit(),
        Distro::OpenSuse => zypper_installed(),
        Distro::Unknown  => vec![],
    };

    raw.into_iter()
        .filter(|p| !menu_names.contains(&p.name))
        .filter(|p| !flatpak_names.contains(&p.name))
        .filter(|p| !is_system_noise(&p.name))
        .filter(|p| !is_blacklisted_package(&p.name))
        .collect()
}

/// Filter obvious system/library packages from terminal tab.
fn is_system_noise(name: &str) -> bool {
    let prefixes = [
        "lib", "gir1.2-", "python3-", "python-", "linux-",
        "fonts-", "xserver-", "x11-", "xfonts-", "perl",
        "ruby", "r-", "golang-", "ghc-", "haskell-", "ocaml-",
        "texlive-", "libreoffice-l10n-", "libreoffice-help-",
        "kde-", "kdelibs-", "plasma-",
    ];
    prefixes.iter().any(|p| name.starts_with(p))
}

// ---------------------------------------------------------------------------
// Debian/Ubuntu/Mint — apt
// ---------------------------------------------------------------------------

fn apt_mark_showmanual() -> Vec<Package> {
    if !cmd_exists("apt-mark") { return vec![]; }
    let versions = dpkg_version_map();
    run("apt-mark", &["showmanual"])
        .lines()
        .filter(|l| !l.is_empty())
        .map(|name| {
            let name = name.trim().to_string();
            let version = versions.get(&name).cloned().unwrap_or_default();
            Package {
                display_name: name.clone(),
                name,
                version,
                source: Source::Apt,
                flatpak_id: None,
            config_path: None,
                resolution: None,
                selected: false,
            }
        })
        .collect()
}

fn dpkg_version_map() -> std::collections::HashMap<String, String> {
    let out = run("dpkg-query", &[
        "--show",
        "--showformat=${Package}\t${Version}\t${db:Status-Status}\n",
    ]);
    let mut map = std::collections::HashMap::new();
    for line in out.lines() {
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() == 3 && parts[2].trim() == "installed" {
            map.insert(parts[0].trim().to_string(), parts[1].trim().to_string());
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Fedora/RHEL — dnf
// ---------------------------------------------------------------------------

fn dnf_userinstalled() -> Vec<Package> {
    if !cmd_exists("dnf") { return vec![]; }
    run("dnf", &["repoquery", "--userinstalled", "--qf", "%{name}\t%{version}"])
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with("Last metadata"))
        .map(|line| {
            let parts: Vec<&str> = line.splitn(2, '\t').collect();
            let name = parts[0].trim().to_string();
            Package {
                display_name: name.clone(),
                name,
                version: parts.get(1).unwrap_or(&"").trim().to_string(),
                source: Source::Dnf,
                flatpak_id: None,
            config_path: None,
                resolution: None,
                selected: false,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Arch/Manjaro — pacman
// ---------------------------------------------------------------------------

fn pacman_explicit() -> Vec<Package> {
    if !cmd_exists("pacman") { return vec![]; }
    let versions = pacman_version_map();
    run("pacman", &["-Qeq"])
        .lines()
        .filter(|l| !l.is_empty())
        .map(|name| {
            let name = name.trim().to_string();
            let version = versions.get(&name).cloned().unwrap_or_default();
            Package {
                display_name: name.clone(),
                name,
                version,
                source: Source::Pacman,
                flatpak_id: None,
            config_path: None,
                resolution: None,
                selected: false,
            }
        })
        .collect()
}

fn pacman_version_map() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for line in run("pacman", &["-Q"]).lines() {
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() == 2 {
            map.insert(parts[0].trim().to_string(), parts[1].trim().to_string());
        }
    }
    map
}

// ---------------------------------------------------------------------------
// openSUSE — zypper
// ---------------------------------------------------------------------------

fn zypper_installed() -> Vec<Package> {
    if !cmd_exists("zypper") { return vec![]; }
    let out = run("zypper", &["--no-refresh", "se", "-i", "--type", "package"]);
    let mut packages = Vec::new();
    for line in out.lines().skip(4) {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() < 5 { continue; }
        let status = parts[0].trim();
        if status != "i" && status != "i+" { continue; }
        let name = parts[1].trim().to_string();
        packages.push(Package {
            display_name: name.clone(),
            name,
            version: parts[3].trim().to_string(),
            source: Source::Zypper,
            flatpak_id: None,
            config_path: None,
            resolution: None,
            selected: false,
        });
    }
    packages
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn cmd_exists(cmd: &str) -> bool {
    Command::new("which").arg(cmd).output()
        .map(|o| o.status.success()).unwrap_or(false)
}

fn run(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd).args(args).output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}
