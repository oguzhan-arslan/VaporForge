pub mod artwork_picker;
pub mod log;
pub mod nonsteam_manager;
pub mod settings;
pub mod theme;

use egui::{Align, Color32, Layout, Margin, Rounding, Stroke, Ui};

use crate::config::AppConfig;
use log::AppLog;

pub trait View {
    fn name(&self) -> &str;
    fn show(&mut self, ui: &mut Ui, config: &mut AppConfig);
}

pub struct VaporForgeApp {
    config: AppConfig,
    views: Vec<Box<dyn View>>,
    active: usize,
    api_key_warning: bool,
    app_log: AppLog,
}

impl VaporForgeApp {
    pub fn new(ctx: &egui::Context, config: AppConfig, app_log: AppLog) -> Self {
        egui_extras::install_image_loaders(ctx);
        theme::setup(ctx);

        let api_key_warning = config.steamgriddb.api_key.is_empty();

        let views: Vec<Box<dyn View>> = vec![
            Box::new(nonsteam_manager::NonSteamManagerView::new()),
            Box::new(artwork_picker::ArtworkPickerView::new()),
            Box::new(settings::SettingsView::new()),
        ];

        Self {
            config,
            views,
            active: 0,
            api_key_warning,
            app_log,
        }
    }
}

impl eframe::App for VaporForgeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ── Top navigation bar ────────────────────────────────────────────
        egui::TopBottomPanel::top("vf_topbar")
            .exact_height(40.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                egui::Frame::none()
                    .fill(theme::SURFACE_0)
                    .inner_margin(Margin::symmetric(16.0, 0.0))
                    .stroke(Stroke::new(1.0, theme::BORDER)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    // App title
                    ui.label(
                        egui::RichText::new("VaporForge")
                            .size(14.0)
                            .strong()
                            .color(theme::ACCENT),
                    );

                    ui.add_space(12.0);
                    ui.add(egui::Separator::default().vertical().spacing(4.0));
                    ui.add_space(8.0);

                    // Nav tabs
                    let mut clicked: Option<usize> = None;
                    for (i, view) in self.views.iter().enumerate() {
                        if nav_tab(ui, view.name(), self.active == i) {
                            clicked = Some(i);
                        }
                    }
                    if let Some(i) = clicked {
                        self.active = i;
                    }

                    // Version string pushed to right
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                                .size(10.5)
                                .color(theme::TEXT_DIM),
                        );
                    });
                });
            });

        // ── API key warning banner ─────────────────────────────────────────
        if self.api_key_warning {
            egui::TopBottomPanel::top("vf_api_warning")
                .resizable(false)
                .show_separator_line(false)
                .frame(
                    egui::Frame::none()
                        .fill(Color32::from_rgba_premultiplied(60, 42, 0, 220))
                        .inner_margin(Margin::symmetric(16.0, 8.0)),
                )
                .show(ctx, |ui| {
                    api_key_banner(
                        ui,
                        &mut self.active,
                        &mut self.api_key_warning,
                        self.views.len(),
                    );
                });
        }

        // ── Global log bar ────────────────────────────────────────────────
        egui::TopBottomPanel::bottom("vf_log_bar")
            .exact_height(22.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                egui::Frame::none()
                    .fill(theme::SURFACE_0)
                    .inner_margin(Margin::symmetric(12.0, 4.0))
                    .stroke(Stroke::new(1.0, theme::BORDER)),
            )
            .show(ctx, |ui| {
                if let Some(entry) = self.app_log.last() {
                    let color = if entry.is_error { theme::ERROR } else { theme::TEXT_DIM };
                    ui.label(egui::RichText::new(&entry.message).size(11.0).color(color));
                }
            });

        // ── Main content area ─────────────────────────────────────────────
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::SURFACE_1))
            .show(ctx, |ui| {
                if let Some(view) = self.views.get_mut(self.active) {
                    view.show(ui, &mut self.config);
                }
            });
    }
}

// ── Top bar nav tab ───────────────────────────────────────────────────────────

fn nav_tab(ui: &mut Ui, label: &str, is_active: bool) -> bool {
    let text_color = if is_active {
        egui::Color32::WHITE
    } else {
        theme::TEXT_2
    };
    let btn = egui::Button::new(egui::RichText::new(label).size(13.0).color(text_color))
        .fill(if is_active {
            theme::ACCENT
        } else {
            egui::Color32::TRANSPARENT
        })
        .stroke(Stroke::new(
            1.0,
            if is_active {
                theme::ACCENT_HOVER
            } else {
                egui::Color32::TRANSPARENT
            },
        ))
        .rounding(Rounding::same(5.0));

    ui.add(btn).clicked()
}

// ── API-key warning banner ────────────────────────────────────────────────

fn api_key_banner(ui: &mut Ui, active: &mut usize, visible: &mut bool, view_count: usize) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("⚠  No SteamGridDB API key set — artwork features are disabled.")
                .color(Color32::from_rgb(255, 195, 70))
                .size(13.0),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("Dismiss").clicked() {
                *visible = false;
            }
            if ui.small_button("Open Settings").clicked() {
                *active = view_count.saturating_sub(1);
                *visible = false;
            }
        });
    });
}

// ── eframe bootstrap ──────────────────────────────────────────────────────

pub fn run(app_log: AppLog, config: AppConfig) -> eyre::Result<()> {
    let icon = load_icon();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("VaporForge")
        .with_inner_size([1920.0, 1080.0])
        .with_min_inner_size([900.0, 560.0]);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "VaporForge",
        options,
        Box::new(|cc| Ok(Box::new(VaporForgeApp::new(&cc.egui_ctx, config, app_log)))),
    )
    .map_err(|e| eyre::eyre!("{e}"))
}

fn load_icon() -> Option<egui::IconData> {
    let bytes = include_bytes!("../../icon.png");
    let img = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    Some(egui::IconData { rgba: img.into_raw(), width: w, height: h })
}
