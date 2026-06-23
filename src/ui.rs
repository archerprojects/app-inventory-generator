// ui.rs — egui application state and interface
// Developed for Lean Linux by archerprojects (archer.projects@proton.me)
// https://github.com/archerprojects/app-inventory-generator
//
// Flow: Select Target → Scan (includes resolution) → Review (four tabs) → Generate.
// Output: ~/app_inventory/<distro>_app_install.sh — one file per target, overwritten on regen.
// Theming: gsettings detection per lean-app-directive-v3.md.
// NOTE: egui implementation section not yet in directive — flagged to build project.

use std::sync::{Arc, Mutex};
use std::process::Command;

use eframe::egui;
use tokio::runtime::Runtime;

use crate::generator::{self, GeneratorInput, ResolvedPackage};
use crate::resolver::{self, Resolution, TargetDistro};
use crate::scanner::{self, Package, PackageList, Source};

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

fn is_dark_theme() -> bool {
    Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "gtk-theme"])
        .output()
        .map(|o| !String::from_utf8_lossy(&o.stdout).to_lowercase().contains("light"))
        .unwrap_or(true)
}

fn lean_visuals(dark: bool) -> egui::Visuals {
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let accent = egui::Color32::from_rgb(0x4b, 0x8b, 0xd4);
    if dark {
        v.panel_fill       = egui::Color32::from_rgb(0x38, 0x38, 0x38);
        v.window_fill      = egui::Color32::from_rgb(0x3e, 0x3e, 0x3e);
        v.faint_bg_color   = egui::Color32::from_rgb(0x40, 0x40, 0x40);
        v.extreme_bg_color = egui::Color32::from_rgb(0x2e, 0x2e, 0x2e);
        v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(0x52, 0x52, 0x52);
        v.widgets.inactive.bg_fill       = egui::Color32::from_rgb(0x52, 0x52, 0x52);
        v.widgets.hovered.bg_fill        = egui::Color32::from_rgb(0x5c, 0x5c, 0x5c);
        v.widgets.active.bg_fill         = accent;
        v.selection.bg_fill = egui::Color32::from_rgba_unmultiplied(0x4b, 0x8b, 0xd4, 0x40);
        v.hyperlink_color   = accent;
        v.widgets.noninteractive.fg_stroke.color = egui::Color32::from_rgb(0xf0, 0xf0, 0xf0);
        v.widgets.inactive.fg_stroke.color       = egui::Color32::from_rgb(0xf0, 0xf0, 0xf0);
    } else {
        v.panel_fill       = egui::Color32::from_rgb(0xf5, 0xf5, 0xf5);
        v.window_fill      = egui::Color32::from_rgb(0xff, 0xff, 0xff);
        v.faint_bg_color   = egui::Color32::from_rgb(0xff, 0xff, 0xff);
        v.extreme_bg_color = egui::Color32::from_rgb(0xeb, 0xeb, 0xeb);
        v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(0xf0, 0xf0, 0xf0);
        v.widgets.inactive.bg_fill       = egui::Color32::from_rgb(0xf0, 0xf0, 0xf0);
        v.widgets.hovered.bg_fill        = egui::Color32::from_rgb(0xf4, 0xf4, 0xf4);
        v.widgets.active.bg_fill         = accent;
        v.selection.bg_fill = egui::Color32::from_rgba_unmultiplied(0x4b, 0x8b, 0xd4, 0x40);
        v.hyperlink_color   = accent;
        v.widgets.noninteractive.fg_stroke.color = egui::Color32::from_rgb(0x1a, 0x1a, 0x1a);
        v.widgets.inactive.fg_stroke.color       = egui::Color32::from_rgb(0x1a, 0x1a, 0x1a);
    }
    v.error_fg_color = egui::Color32::from_rgb(0xe3, 0x5d, 0x4f);
    v.warn_fg_color  = egui::Color32::from_rgb(0xf2, 0x7e, 0x3f);
    v
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Default)]
enum AppStep {
    #[default] SelectTarget,
    Scanning,
    Review,
    Generating,
    Done(String),
    Error(String),
}

#[derive(Default, PartialEq, Eq, Clone, Copy)]
enum ActiveTab {
    #[default] MenuApps,
    TerminalApps,
    FlatpakApps,
    SnapApps,
}

#[derive(Default, Clone)]
struct Progress {
    current: usize,
    total: usize,
    label: String,
}

pub struct AppState {
    step: AppStep,
    packages: PackageList,
    target: TargetDistro,
    active_tab: ActiveTab,
    show_config_info: bool,
    search_query: String,
    search_results: Arc<Mutex<Vec<String>>>,
    searching: bool,
    progress: Arc<Mutex<Progress>>,
    bg_result: Arc<Mutex<Option<BgResult>>>,
    rt: Runtime,
}

enum BgResult {
    ScanDone(PackageList),
    GenerateDone(Result<String, String>),
    SearchDone(Vec<String>),
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            step: AppStep::SelectTarget,
            packages: PackageList::default(),
            target: TargetDistro::Ubuntu,
            active_tab: ActiveTab::default(),
            show_config_info: false,
            search_query: String::new(),
            search_results: Arc::new(Mutex::new(vec![])),
            searching: false,
            progress: Arc::new(Mutex::new(Progress::default())),
            bg_result: Arc::new(Mutex::new(None)),
            rt: Runtime::new().expect("tokio runtime"),
        }
    }
}

// ---------------------------------------------------------------------------
// eframe App
// ---------------------------------------------------------------------------

impl eframe::App for AppState {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        ctx.set_visuals(lean_visuals(is_dark_theme()));
        self.poll_background(&ctx);

        if matches!(self.step, AppStep::Generating | AppStep::Scanning) {
            ctx.request_repaint();
        }

        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.heading("App Inventory Generator");
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(8.0);

                match &self.step {
                    AppStep::SelectTarget => self.draw_select_target(ui, &ctx),
                    AppStep::Scanning     => self.draw_scanning(ui),
                    AppStep::Review       => self.draw_review(ui, &ctx),
                    AppStep::Generating   => self.draw_generating(ui),
                    AppStep::Done(_)      => self.draw_done(ui),
                    AppStep::Error(_)     => self.draw_error(ui),
                }
            });
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

impl AppState {
    fn draw_select_target(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let accent = egui::Color32::from_rgb(0x4b, 0x8b, 0xd4);

        ui.add_space(60.0);

        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("App Inventory Generator")
                    .size(28.0)
                    .strong(),
            );
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(
                    "Scan your system and generate an app install script for your new system.",
                )
                .size(15.0)
                .color(egui::Color32::GRAY),
            );
            ui.add_space(40.0);

            ui.label(
                egui::RichText::new("Select target distro")
                    .size(16.0)
                    .strong(),
            );
            ui.add_space(12.0);

            ui.horizontal(|ui| {
                // Centre the combo
                let combo_width = 260.0;
                let avail = ui.available_width();
                ui.add_space((avail - combo_width) / 2.0);
                egui::ComboBox::from_id_salt("target_distro")
                    .width(combo_width)
                    .selected_text(
                        egui::RichText::new(self.target.label()).size(15.0),
                    )
                    .show_ui(ui, |ui| {
                        for distro in TargetDistro::all() {
                            let label = distro.label();
                            ui.selectable_value(
                                &mut self.target,
                                distro,
                                egui::RichText::new(label).size(14.0),
                            );
                        }
                    });
            });

            ui.add_space(32.0);

            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("  Scan System  ").size(16.0),
                    )
                    .fill(accent)
                    .min_size(egui::vec2(200.0, 44.0)),
                )
                .clicked()
            {
                self.start_scan(ctx);
            }

            ui.add_space(48.0);
            ui.separator();
            ui.add_space(16.0);

            ui.label(
                egui::RichText::new(
                    "The scan checks which of your installed apps are available \
                     on the target distro.\nAll packages default to unselected — \
                     you choose what to carry to the new system.",
                )
                .size(13.0)
                .color(egui::Color32::GRAY),
            );
        });

        // About — developer identity (directive requirement), pinned to the
        // bottom of the opening screen. Shown here only, not on later steps.
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.add_space(14.0);
            ui.hyperlink_to(
                "github.com/archerprojects/app-inventory-generator",
                "https://github.com/archerprojects/app-inventory-generator",
            );
            ui.label(
                egui::RichText::new(
                    "Developed by archerprojects (archer.projects@proton.me)",
                )
                .size(12.0)
                .color(egui::Color32::GRAY),
            );
            ui.add_space(12.0);
        });
    }

    fn draw_scanning(&self, ui: &mut egui::Ui) {
        let progress = self.progress.lock().unwrap().clone();
        let accent = egui::Color32::from_rgb(0x4b, 0x8b, 0xd4);

        ui.add_space(80.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(&progress.label)
                    .size(16.0)
                    .color(egui::Color32::from_rgb(0xf0, 0xf0, 0xf0)),
            );
            ui.add_space(24.0);
            if progress.total > 0 {
                let fraction = progress.current as f32 / progress.total as f32;
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .text(
                            egui::RichText::new(format!(
                                "{} / {}",
                                progress.current, progress.total
                            ))
                            .size(13.0),
                        )
                        .fill(accent)
                        .desired_width(500.0),
                );
            } else {
                ui.add(
                    egui::widgets::Spinner::new()
                        .size(40.0)
                        .color(accent),
                );
            }
        });
    }

    fn draw_review(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        // ── Target reminder + tabs ───────────────────────────────────────
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("Target: {}", self.target.label()))
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("ℹ Config paths").clicked() {
                    self.show_config_info = true;
                }
                if ui.small_button("Change target").clicked() {
                    self.step = AppStep::SelectTarget;
                }
            });
        });

        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.active_tab,
                ActiveTab::MenuApps,
                format!("Menu Apps installed on System ({})",
                    self.packages.menu.len()),
            );
            ui.selectable_value(
                &mut self.active_tab,
                ActiveTab::TerminalApps,
                format!("Terminal Apps installed on System ({})",
                    self.packages.terminal.len()),
            );
            ui.selectable_value(
                &mut self.active_tab,
                ActiveTab::FlatpakApps,
                format!("Flatpak Apps installed on System ({})",
                    self.packages.flatpak.len()),
            );
            ui.selectable_value(
                &mut self.active_tab,
                ActiveTab::SnapApps,
                format!("Snap Apps installed on System ({})",
                    self.packages.snap.len()),
            );
        });

        // ── Config path info dialog ───────────────────────────────────────
        if self.show_config_info {
            egui::Window::new("About config paths")
                .collapsible(false)
                .resizable(false)
                .default_width(480.0)
                .show(ui.ctx(), |ui| {
                    ui.label(egui::RichText::new("Where are app settings stored?").strong());
                    ui.add_space(8.0);
                    ui.label(
                        "Native apps (Menu / Terminal tabs) store settings in:\n\
                         \t~/.config/<appname>/\n\
                         \t~/.local/share/<appname>/\n\n\
                         Flatpak apps store settings in:\n\
                         \t~/.var/app/<flatpak-id>/config/\n\
                         \t~/.var/app/<flatpak-id>/data/\n\n\
                         The path shown next to each app is detected automatically \
                         where possible. Some apps use non-standard locations — \
                         check the app's documentation if the shown path is not \
                         where you expect settings to be.\n\n\
                         This app does not migrate settings. The paths are shown \
                         so you know where to look when moving your settings manually."
                    );
                    ui.add_space(8.0);
                    if ui.button("Close").clicked() {
                        self.show_config_info = false;
                    }
                });
        }

        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);

        // ── Select / deselect for active tab ─────────────────────────────
        ui.horizontal(|ui| {
            if ui.button("Select All").clicked() {
                self.set_selected_for_tab(true);
            }
            if ui.button("Deselect All").clicked() {
                self.set_selected_for_tab(false);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let count = self.selected_in_tab();
                ui.label(
                    egui::RichText::new(format!("{count} selected"))
                        .color(egui::Color32::GRAY),
                );
            });
        });

        ui.add_space(4.0);
        ui.separator();

        // ── Package list ─────────────────────────────────────────────────
        let available_h = ui.available_height() - 180.0;
        egui::ScrollArea::vertical()
            .max_height(available_h)
            .show(ui, |ui| {
                ui.add_space(4.0);
                match self.active_tab {
                    ActiveTab::MenuApps     => draw_package_list(ui, &mut self.packages.menu),
                    ActiveTab::TerminalApps => draw_package_list(ui, &mut self.packages.terminal),
                    ActiveTab::FlatpakApps  => draw_package_list(ui, &mut self.packages.flatpak),
                    ActiveTab::SnapApps     => draw_package_list(ui, &mut self.packages.snap),
                }
            });

        ui.separator();
        ui.add_space(4.0);

        // ── Add package ───────────────────────────────────────────────────
        ui.collapsing("Add a package", |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.search_query);
                if ui.button("Search").clicked() && !self.search_query.is_empty() {
                    self.start_search(ctx);
                }
            });
            if self.searching {
                ui.add(egui::widgets::Spinner::new());
            } else {
                let results = self.search_results.lock().unwrap().clone();
                for result in &results {
                    ui.horizontal(|ui| {
                        ui.label(result);
                        if ui.small_button("+").clicked() {
                            self.packages.terminal.push(Package {
                                name: result.clone(),
                                display_name: result.clone(),
                                version: "latest".to_string(),
                                source: Source::Apt,
                                flatpak_id: None,
                                config_path: None,
                                resolution: Some(Resolution::Native(result.clone())),
                                selected: true,
                            });
                        }
                    });
                }
            }
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        // ── Generate ──────────────────────────────────────────────────────
        ui.horizontal(|ui| {
            let total = self.total_selected();
            if ui
                .add_enabled(total > 0, egui::Button::new("  Generate Script  "))
                .clicked()
            {
                self.start_generate(ctx);
            }
            if total == 0 {
                ui.label(
                    egui::RichText::new("Select packages to include.")
                        .color(egui::Color32::GRAY)
                        .small(),
                );
            } else {
                ui.label(
                    egui::RichText::new(format!(
                        "{total} packages selected — will write to ~/app_inventory/{}_app_install.sh",
                        self.target.slug()
                    ))
                    .color(egui::Color32::GRAY)
                    .small(),
                );
            }
        });
    }

    fn draw_generating(&self, ui: &mut egui::Ui) {
        let accent = egui::Color32::from_rgb(0x4b, 0x8b, 0xd4);
        ui.add_space(80.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("Writing script…")
                    .size(16.0)
                    .color(egui::Color32::from_rgb(0xf0, 0xf0, 0xf0)),
            );
            ui.add_space(24.0);
            ui.add(egui::widgets::Spinner::new().size(40.0).color(accent));
        });
    }

    fn draw_done(&mut self, ui: &mut egui::Ui) {
        if let AppStep::Done(ref path) = self.step {
            let path = path.clone();
            ui.add_space(20.0);
            ui.label(
                egui::RichText::new("✓ Script generated successfully.")
                    .color(egui::Color32::GREEN)
                    .size(16.0),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label("Saved to:");
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(path.clone()).color(egui::Color32::WHITE),
                    )
                    .selectable(true),
                );
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Open folder").clicked() {
                    let folder = std::path::Path::new(&path)
                        .parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.clone());
                    let _ = std::process::Command::new("xdg-open").arg(&folder).spawn();
                }
                if ui.button("Generate for another distro").clicked() {
                    self.step = AppStep::SelectTarget;
                }
            });

            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);

            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.label(egui::RichText::new("What this app and script does").strong());
                ui.add_space(4.0);
                ui.label(
                    "The app scans your system for installed apps (native or Flatpak) \
                     and allows you to select which apps to install on a new system. \
                     The script generated, when run in terminal, will auto-install your \
                     selected packages on the new system. This app can be run on any \
                     Linux system to install the required apps providing that system was \
                     set as target in the original script to confirm the proper install \
                     codes and sequence for the new system requirements were met."
                );
                ui.add_space(10.0);

                ui.label(egui::RichText::new("What this app does not do").strong());
                ui.add_space(4.0);
                ui.label(
                    "It does not migrate application settings or configuration files. \
                     On an existing system that you are reinstalling, these files are \
                     usually already in place and reinstalling the app will configure \
                     to these existing settings. On a fresh install, these files must \
                     be moved manually."
                );
                ui.add_space(10.0);

                ui.label(egui::RichText::new("Moving your settings").strong());
                ui.add_space(4.0);
                ui.label(
                    "Separate home partition — mount it on the new system. Most settings \
                     come back automatically.\n\n\
                     Copying your home directory — on the old system run:\n\
                     \ttar -czf ~/home-backup.tar.gz ~/.config ~/.bashrc ~/.gitconfig\n\
                     Copy to USB, then on new system:\n\
                     \ttar -xzf home-backup.tar.gz -C ~/\n\n\
                     Browsers — Firefox: sign in to Firefox Sync. Chrome: sign in to \
                     Google account. Brave: use brave://settings/braveSync.\n\n\
                     Thunderbird — use Import/Export Tools addon or copy ~/.thunderbird\n\n\
                     VSCode — sign in with Settings Sync.\n\n\
                     Some settings can also be moved through profile configurations on \
                     different browsers and mail clients. Check internet sources for more \
                     detailed information concerning your particular app regarding moving \
                     passwords, bookmarks and mail settings during migration if needed."
                );
            });

            ui.add_space(20.0);
            if ui.button("Start Over").clicked() {
                self.step = AppStep::SelectTarget;
                self.packages = PackageList::default();
            }
        }
    }

    fn draw_error(&mut self, ui: &mut egui::Ui) {
        if let AppStep::Error(ref msg) = self.step {
            ui.add_space(20.0);
            ui.label(
                egui::RichText::new(format!("Error: {msg}"))
                    .color(egui::Color32::RED),
            );
            ui.add_space(20.0);
            if ui.button("Back").clicked() {
                self.step = AppStep::Review;
            }
        }
    }

    fn selected_in_tab(&self) -> usize {
        match self.active_tab {
            ActiveTab::MenuApps     => self.packages.menu.iter().filter(|p| p.selected).count(),
            ActiveTab::TerminalApps => self.packages.terminal.iter().filter(|p| p.selected).count(),
            ActiveTab::FlatpakApps  => self.packages.flatpak.iter().filter(|p| p.selected).count(),
            ActiveTab::SnapApps     => self.packages.snap.iter().filter(|p| p.selected).count(),
        }
    }

    fn total_selected(&self) -> usize {
        self.packages.all_packages().filter(|p| p.selected).count()
    }

    fn set_selected_for_tab(&mut self, selected: bool) {
        let packages = match self.active_tab {
            ActiveTab::MenuApps     => &mut self.packages.menu,
            ActiveTab::TerminalApps => &mut self.packages.terminal,
            ActiveTab::FlatpakApps  => &mut self.packages.flatpak,
            ActiveTab::SnapApps     => &mut self.packages.snap,
        };
        for p in packages.iter_mut() { p.selected = selected; }
    }
}

// ---------------------------------------------------------------------------
// Background tasks
// ---------------------------------------------------------------------------

impl AppState {
    /// Scan the system, then resolve every package against the selected target.
    /// Progress bar covers the resolution pass (the slow part).
    fn start_scan(&mut self, ctx: &egui::Context) {
        self.step = AppStep::Scanning;
        let target = self.target.clone();
        let tx = Arc::clone(&self.bg_result);
        let progress = Arc::clone(&self.progress);
        let ctx = ctx.clone();

        {
            let mut p = progress.lock().unwrap();
            p.total = 0;
            p.current = 0;
            p.label = "Scanning installed packages…".to_string();
        }

        self.rt.spawn(async move {
            // 1. Scan
            let mut list = tokio::task::spawn_blocking(scanner::scan_system)
                .await
                .unwrap_or_default();

            // 2. Resolve every package against target (parallel)
            let total: usize = list.menu.len() + list.terminal.len() + list.flatpak.len();
            {
                let mut p = progress.lock().unwrap();
                p.total = total;
                p.current = 0;
                p.label = format!("Checking package availability for {}…", target.label());
            }
            ctx.request_repaint();

            resolve_all(&mut list.menu, &target, &progress, &ctx).await;
            resolve_all(&mut list.terminal, &target, &progress, &ctx).await;
            resolve_all(&mut list.flatpak, &target, &progress, &ctx).await;

            *tx.lock().unwrap() = Some(BgResult::ScanDone(list));
            ctx.request_repaint();
        });
    }

    fn start_search(&mut self, ctx: &egui::Context) {
        self.searching = true;
        *self.search_results.lock().unwrap() = vec![];
        let query = self.search_query.clone();
        let target = self.target.clone();
        let tx = Arc::clone(&self.bg_result);
        let ctx = ctx.clone();
        self.rt.spawn(async move {
            let results = resolver::search_packages(&query, &target).await;
            *tx.lock().unwrap() = Some(BgResult::SearchDone(results));
            ctx.request_repaint();
        });
    }

    /// Packages arrive pre-resolved from scan — generate just writes the script.
    fn start_generate(&mut self, ctx: &egui::Context) {
        self.step = AppStep::Generating;

        // Collect selected packages — deduplicate by name across tabs
        let mut seen = std::collections::HashSet::new();
        let resolved: Vec<ResolvedPackage> = self.packages.all_packages()
            .filter(|p| p.selected)
            .filter(|p| seen.insert(p.name.clone()))
            .map(|p| ResolvedPackage {
                display_name: p.display_name.clone(),
                resolution: p.resolution.clone().unwrap_or(Resolution::Unresolved),
            })
            .collect();

        let target = self.target.clone();
        let tx = Arc::clone(&self.bg_result);
        let ctx = ctx.clone();

        self.rt.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                generator::generate(&GeneratorInput {
                    packages: &resolved,
                    target: &target,
                })
                .map(|p| p.to_string_lossy().to_string())
                .map_err(|e| e.to_string())
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));

            *tx.lock().unwrap() = Some(BgResult::GenerateDone(result));
            ctx.request_repaint();
        });
    }

    fn poll_background(&mut self, _ctx: &egui::Context) {
        let result = self.bg_result.lock().unwrap().take();
        match result {
            Some(BgResult::ScanDone(list)) => {
                self.packages = list;
                self.step = AppStep::Review;
            }
            Some(BgResult::GenerateDone(Ok(path))) => {
                self.step = AppStep::Done(path);
            }
            Some(BgResult::GenerateDone(Err(e))) => {
                self.step = AppStep::Error(e);
            }
            Some(BgResult::SearchDone(results)) => {
                *self.search_results.lock().unwrap() = results;
                self.searching = false;
            }
            None => {}
        }
    }
}

/// Resolve every package in a list against the target distro, updating progress.
async fn resolve_all(
    packages: &mut [Package],
    target: &TargetDistro,
    progress: &Arc<Mutex<Progress>>,
    ctx: &egui::Context,
) {
    let mut handles = Vec::new();
    for (i, pkg) in packages.iter().enumerate() {
        let name = pkg.name.clone();
        let fid = pkg.flatpak_id.clone();
        let target = target.clone();
        let progress = Arc::clone(progress);
        let ctx = ctx.clone();
        handles.push(tokio::spawn(async move {
            let res = resolver::resolve_package(&name, fid.as_deref(), &target).await;
            {
                let mut p = progress.lock().unwrap();
                p.current += 1;
                p.label = format!(
                    "Checking package availability for {}… ({}/{})",
                    target.label(), p.current, p.total
                );
            }
            ctx.request_repaint();
            (i, res)
        }));
    }

    for handle in handles {
        if let Ok((i, res)) = handle.await {
            packages[i].resolution = Some(res);
        }
    }
}

// ---------------------------------------------------------------------------
// Package list renderer
// ---------------------------------------------------------------------------

fn draw_package_list(ui: &mut egui::Ui, packages: &mut Vec<Package>) {
    if packages.is_empty() {
        ui.label(
            egui::RichText::new("No packages in this category.")
                .color(egui::Color32::GRAY),
        );
        return;
    }
    for pkg in packages.iter_mut() {
        ui.horizontal(|ui| {
            ui.checkbox(&mut pkg.selected, "");
            ui.label(&pkg.display_name);

            // Resolution status badge
            match &pkg.resolution {
                Some(Resolution::Native(name)) => {
                    if name != &pkg.name {
                        ui.label(
                            egui::RichText::new(format!("→ {}", name))
                                .color(egui::Color32::from_rgb(0x7a, 0xc8, 0x7a))
                                .small(),
                        );
                    }
                }
                Some(Resolution::Flatpak(_)) => {
                    ui.label(
                        egui::RichText::new("(Flatpak)")
                            .color(egui::Color32::from_rgb(0x6a, 0x9f, 0xc8))
                            .small(),
                    );
                }
                Some(Resolution::Snap { classic, .. }) => {
                    let label = if *classic { "(Snap · classic)" } else { "(Snap)" };
                    ui.label(
                        egui::RichText::new(label)
                            .color(egui::Color32::from_rgb(0x6a, 0x9f, 0xc8))
                            .small(),
                    );
                }
                Some(Resolution::Unresolved) => {
                    ui.label(
                        egui::RichText::new("⚠ not available on target")
                            .color(egui::Color32::from_rgb(0xf2, 0x7e, 0x3f))
                            .small(),
                    );
                }
                None => {}
            }

            if let Some(ref path) = pkg.config_path {
                ui.label(
                    egui::RichText::new(path)
                        .color(egui::Color32::from_rgb(0x6a, 0x9f, 0xc8))
                        .small(),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(&pkg.version)
                        .color(egui::Color32::GRAY)
                        .small(),
                );
            });
        });
    }
}
