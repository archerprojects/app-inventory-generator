# App Inventory Generator

### Version 1.0.0

Scan your installed applications and generate a bash script that reinstalls
your chosen apps on a new Linux system.

---

## Developed for Lean Linux by

```
Developer:  archerprojects
Contact:    archer.projects@proton.me
Maintainer: archerprojects <archer.projects@proton.me>
archerprojects (archer.projects@proton.me)
https://github.com/archerprojects/app-inventory-generator
```

Developed for Lean Linux. Not exclusive to Lean Linux — it runs on any
supported Linux desktop.

---

## What it does

App Inventory Generator enumerates the apps installed on your system —
native packages, Flatpaks, and Snaps — lets you pick which ones to carry
forward, and writes a single bash script that reinstalls them on a fresh
system. You choose the target distro first, the app checks each package
against that target, and the generated script installs everything it can
and lists anything it can't for you to handle by hand.

Run the script in a terminal on the new system. Every install is guarded by
a presence check, so the script is safe to re-run.

## What it does not do

It does not migrate application settings or configuration files. On a system
you are reinstalling, those files are usually already in place and the
reinstalled app picks them up. On a fresh install they must be moved
manually. The app shows each app's detected config path so you know where to
look, and an in-app info dialog explains the approach. It is an app
inventory tool, not a settings-migration tool.

---

## The review screen

After scanning, packages are sorted into four tabs, everything unselected by
default so you build the list deliberately:

- **Menu Apps** — apps with a desktop entry, resolved to their real package
  name.
- **Terminal Apps** — explicitly user-installed command-line packages.
- **Flatpak Apps** — Flatpaks installed on the system.
- **Snap Apps** — Snaps installed on the system.

Each entry shows its resolution status against the target, and its detected
config path where one is found.

## Flow

1. **Select target distro** — chosen before the scan so resolution targets
   the right system.
2. **Scan** — enumerates installed packages and resolves each one against the
   target.
3. **Review** — pick what to carry forward across the four tabs.
4. **Generate** — writes the script.

The script is written to:

```
~/app_inventory/<distro>_app_install.sh
```

One file per target (for example `mint_app_install.sh`), overwritten when you
regenerate for the same target.

---

## Supported systems

**Scans (host):** Debian / Ubuntu / Mint / Pop!_OS (apt), Fedora / RHEL /
Rocky (dnf), Arch / Manjaro (pacman), openSUSE (zypper). Flatpak and Snap are
scanned independently of the host distro when present.

**Targets (generated script):** Ubuntu, Debian, Mint, Pop!_OS, Fedora, Arch,
Manjaro, openSUSE.

Snaps are installed automatically on targets that ship snapd (apt-family and
Fedora). On Arch, Manjaro, and openSUSE — where snapd is not in the default
repositories — selected Snaps are listed with an install note instead of a
command, since automating that install is not reliable.

---

## Prerequisites / Requirements

**Runtime (the .deb declares shared-library dependencies automatically):**

- A Linux desktop session with OpenGL support (X11 or Wayland). The UI is a
  single self-contained binary built on egui/eframe.

**Tools used at runtime** (only what's present on the machine is used; missing
tools are skipped cleanly):

- The host package manager for scanning installed packages —
  `dpkg` / `apt-mark`, `dnf`, `pacman`, or `zypper`.
- `flatpak` — optional; the Flatpak tab populates only if it is installed.
- `snap` — optional; the Snap tab populates only if it is installed.
- `gsettings` — used for dark/light theme detection.

**Network:**

- Required only for cross-distro package resolution, which queries the public
  Repology API. When the target is the same apt-family as the host, no network
  lookup is performed.

**Compatibility baseline:** Tested on Debian 12 / Ubuntu 24.04.

---

## Build and install

Standard build and package:

```bash
make deb
```

Clean build (after dependency or major changes):

```bash
cargo clean && make deb
```

The package is written to:

```
dist/app-inventory-generator_1.0.0-1_amd64.deb
```

Install:

```bash
sudo dpkg -i dist/app-inventory-generator_1.0.0-1_amd64.deb
```

Run:

```bash
app-inventory-generator
```

---

## License

GPL-3.0-or-later. See [LICENSE](LICENSE) for the full text.

Copyright (C) 2026 archerprojects (archer.projects@proton.me)
