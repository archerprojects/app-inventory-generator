// resolver.rs — Cross-distro package name resolution via Repology API
// Developed for Lean Linux by archerprojects (archer.projects@proton.me)
// https://github.com/archerprojects/app-inventory-generator
//
// Also handles add-package search against the target distro.
// All network calls are async and run during the scan/resolution pass.

use serde::Deserialize;

// ---------------------------------------------------------------------------
// Repology repository identifiers — THE single edit point for distro pinning
// ---------------------------------------------------------------------------
// Repology tracks each distro release as a separate repository, so these
// slugs carry a version. When a target distro ships a new stable release,
// update the slug HERE and nowhere else — repology_repo() is the only reader.
// Slug format: <distro>_<version>  (verify at https://repology.org/repositories)
const REPO_DEBIAN: &str   = "debian_stable";
const REPO_UBUNTU: &str   = "ubuntu_24_04";       // Mint and Pop!_OS track this LTS
const REPO_FEDORA: &str   = "fedora_44";
const REPO_ARCH: &str     = "arch";               // rolling — no version
const REPO_MANJARO: &str  = "manjaro_stable";
const REPO_OPENSUSE: &str = "opensuse_tumbleweed"; // rolling

/// Target distro families and their Repology repository identifiers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TargetDistro {
    Debian,   // apt
    Ubuntu,   // apt
    Mint,     // apt
    PopOs,    // apt
    Fedora,   // dnf
    Arch,     // pacman
    Manjaro,  // pacman
    OpenSuse, // zypper
}

impl TargetDistro {
    /// Human-readable display name.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Debian => "Debian",
            Self::Ubuntu => "Ubuntu",
            Self::Mint => "Linux Mint",
            Self::PopOs => "Pop!_OS",
            Self::Fedora => "Fedora",
            Self::Arch => "Arch Linux",
            Self::Manjaro => "Manjaro",
            Self::OpenSuse => "openSUSE",
        }
    }

    /// Repology repository filter string for this distro.
    /// Reads the REPO_* constants above — the single distro-version edit point.
    pub fn repology_repo(&self) -> &'static str {
        match self {
            Self::Debian => REPO_DEBIAN,
            Self::Ubuntu => REPO_UBUNTU,
            Self::Mint => REPO_UBUNTU,  // Mint tracks Ubuntu
            Self::PopOs => REPO_UBUNTU, // Pop!_OS tracks Ubuntu LTS
            Self::Fedora => REPO_FEDORA,
            Self::Arch => REPO_ARCH,
            Self::Manjaro => REPO_MANJARO,
            Self::OpenSuse => REPO_OPENSUSE,
        }
    }

    /// All distros in display order.
    pub fn all() -> Vec<Self> {
        vec![
            Self::Ubuntu,
            Self::Debian,
            Self::Mint,
            Self::PopOs,
            Self::Fedora,
            Self::Arch,
            Self::Manjaro,
            Self::OpenSuse,
        ]
    }

    /// Lowercase slug used in generated script filenames.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::Debian => "debian",
            Self::Ubuntu => "ubuntu",
            Self::Mint => "mint",
            Self::PopOs => "popos",
            Self::Fedora => "fedora",
            Self::Arch => "arch",
            Self::Manjaro => "manjaro",
            Self::OpenSuse => "opensuse",
        }
    }
}

/// Resolution result for a single package.
#[derive(Debug, Clone)]
pub enum Resolution {
    /// Found as a native package under this name on the target distro.
    Native(String),
    /// No native package found; use Flatpak ID instead.
    Flatpak(String),
    /// Snap package — universal name across distros. `classic` carries the
    /// confinement flag captured from `snap list` so the generator can emit
    /// `--classic` where required. Installed on targets that ship snapd;
    /// listed for manual install on targets that do not.
    Snap { name: String, classic: bool },
    /// Neither native nor Flatpak found — user must resolve manually.
    Unresolved,
}

/// Repology API project record (partial — only fields we use).
#[derive(Debug, Deserialize)]
struct RepologyPackage {
    repo: String,
    name: Option<String>,
    binname: Option<String>,
}

/// Resolve a package name to the target distro's native name.
/// For same-family apt targets (Debian/Ubuntu/Mint/Pop), the source
/// package name is already correct — skip Repology entirely.
/// Falls back to Flatpak ID if provided and native resolution fails.
pub async fn resolve_package(
    source_name: &str,
    flatpak_id: Option<&str>,
    target: &TargetDistro,
) -> Resolution {
    // Flatpak packages — always use the Flatpak ID directly
    if let Some(fid) = flatpak_id {
        return Resolution::Flatpak(fid.to_string());
    }

    // Same-family apt targets — package name is already valid, no lookup needed
    match target {
        TargetDistro::Ubuntu | TargetDistro::Debian |
        TargetDistro::Mint | TargetDistro::PopOs => {
            return Resolution::Native(source_name.to_string());
        }
        _ => {}
    }

    // Cross-distro — query Repology
    match query_repology(source_name, target).await {
        Some(name) => Resolution::Native(name),
        None => Resolution::Unresolved,
    }
}

/// Search Repology for packages matching a query string on the target distro.
/// Used by the add-package lookup feature.
pub async fn search_packages(
    query: &str,
    target: &TargetDistro,
) -> Vec<String> {
    // Repology search endpoint: /api/v1/projects/?search=<query>&inrepo=<repo>
    let url = format!(
        "https://repology.org/api/v1/projects/?search={}&inrepo={}&limit=20",
        urlencoded(query),
        target.repology_repo()
    );

    let client = match reqwest::Client::builder()
        .user_agent("app-inventory-generator/1.0")
        .build()
    {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    // Response is a JSON object keyed by project name.
    // { "vlc": [ { "repo": "arch", "name": "vlc", ... }, ... ], ... }
    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(_) => return vec![],
    };

    let repo = target.repology_repo();
    let mut results = Vec::new();

    if let Some(obj) = json.as_object() {
        for (_project, packages) in obj {
            if let Some(pkgs) = packages.as_array() {
                for pkg in pkgs {
                    let pkg_repo = pkg.get("repo").and_then(|v| v.as_str()).unwrap_or("");
                    if pkg_repo == repo {
                        let name = pkg
                            .get("binname")
                            .or_else(|| pkg.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if !name.is_empty() {
                            results.push(name);
                            break; // one name per project
                        }
                    }
                }
            }
        }
    }

    results.sort();
    results.dedup();
    results
}

/// Query Repology for the target-distro package name of a given source package.
async fn query_repology(name: &str, target: &TargetDistro) -> Option<String> {
    // Repology project lookup: /api/v1/project/<name>
    let url = format!("https://repology.org/api/v1/project/{}", urlencoded(name));

    let client = reqwest::Client::builder()
        .user_agent("app-inventory-generator/1.0")
        .build()
        .ok()?;

    let resp = client.get(&url).send().await.ok()?;
    let packages: Vec<RepologyPackage> = resp.json().await.ok()?;

    let repo = target.repology_repo();
    for pkg in &packages {
        if pkg.repo == repo {
            return pkg
                .binname
                .clone()
                .or_else(|| pkg.name.clone());
        }
    }
    None
}

/// Minimal URL encoding for query strings (replaces space and special chars).
fn urlencoded(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ' ' => "%20".to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}
