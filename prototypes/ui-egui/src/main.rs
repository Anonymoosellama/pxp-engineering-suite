// SPDX-License-Identifier: LGPL-2.1-or-later
//! ADR-0002 prototype: an NX-style shell in egui + `egui_dock`.
//!
//! ```text
//! ┌ title / application name ───────────────────────────────────────────┐
//! │ ribbon: tabs of the active application → groups → command buttons    │  built from AppRegistry::commands()
//! ├ resource bar (dock) ┬ graphics window (dock tab) ────────────────────┤
//! │ Part Navigator      │  placeholder viewport (painter wireframe)      │
//! │ Roles               │                                                │
//! ├ cue line ───────────────────────────────────────────────────────────┤
//! └ status line ────────────────────────────────────────────────────────┘
//! ```
//!
//! Everything shown comes from the registry, so the prototype measures the
//! toolkit, not hand-placed widgets. With `PXP_PROTO_SCREENSHOT=FILE` it
//! switches to Modeling, hovers a planned command, saves a screenshot and
//! exits (used by ADR-0002 for an image of each toolkit).

use eframe::egui;
use egui_dock::{DockArea, DockState, NodeIndex, Style, TabViewer};
use pxp_appkit::{AppRegistry, Application, CommandSpec, UiContext};

/// The dockable panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    PartNavigator,
    Roles,
    Graphics,
}

struct Shell {
    registry: AppRegistry,
    ui_ctx: UiContext,
    dock: DockState<Pane>,
    ribbon_tab: String,
    cue: String,
    status: String,
    screenshot: Option<std::path::PathBuf>,
    shot_done: std::sync::Arc<std::sync::atomic::AtomicBool>,
    frame: u32,
}

/// Saves an egui screenshot as PNG (best effort: the prototype exits anyway).
fn save_png(path: &std::path::Path, img: &egui::ColorImage) {
    let [w, h] = img.size;
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
    if let Some(buf) = image::RgbaImage::from_raw(u32::try_from(w).unwrap_or(0), u32::try_from(h).unwrap_or(0), bytes) {
        let _ = buf.save(path);
    }
}

fn registry() -> AppRegistry {
    let mut registry = AppRegistry::new();
    let apps: [Box<dyn Application>; 6] = [
        Box::new(pxp_app_modeling::Modeling::new()),
        Box::new(pxp_app_drafting::Drafting),
        Box::new(pxp_app_manufacturing::Manufacturing),
        Box::new(pxp_app_postgen::PostGeneration),
        Box::new(pxp_app_machine_kit::MachineKitBuilder),
        Box::new(pxp_app_cae::Cae),
    ];
    for app in apps {
        // The prototype only displays; a duplicate registration cannot occur
        // with this fixed list.
        let _ = registry.register(app);
    }
    registry
}

impl Shell {
    fn new() -> Self {
        let mut dock = DockState::new(vec![Pane::Graphics]);
        let surface = dock.main_surface_mut();
        let [_, left] = surface.split_left(NodeIndex::root(), 0.22, vec![Pane::PartNavigator, Pane::Roles]);
        let _ = left;
        let mut shell = Self {
            registry: registry(),
            ui_ctx: UiContext::default(),
            dock,
            ribbon_tab: String::new(),
            cue: "Select a command or an object".into(),
            status: String::new(),
            screenshot: std::env::var_os("PXP_PROTO_SCREENSHOT").map(Into::into),
            shot_done: std::sync::Arc::default(),
            frame: 0,
        };
        shell.switch("pxp.modeling");
        shell
    }

    /// Switches application through the registry, exactly as the shell will.
    fn switch(&mut self, app: &str) {
        let Ok(target) = pxp_appkit::AppId::new(app) else {
            return;
        };
        match self.registry.switch_to(&target, &mut self.ui_ctx) {
            Ok(()) => {
                self.ribbon_tab = self.ui_ctx.ribbon_tabs.first().map(|t| (*t).to_owned()).unwrap_or_default();
                self.status = format!("Application: {app}");
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    fn active_commands(&self) -> Vec<&CommandSpec> {
        let active = self.registry.active_app();
        self.registry
            .commands()
            .iter()
            .filter(|c| Some(&c.app) == active.as_ref())
            .collect()
    }

    fn ribbon(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for tab in self.ui_ctx.ribbon_tabs.clone() {
                ui.selectable_value(&mut self.ribbon_tab, tab.to_owned(), tab);
            }
        });
        ui.separator();
        let commands: Vec<CommandSpec> = self
            .active_commands()
            .into_iter()
            .filter(|c| c.placement.tab.as_ref() == self.ribbon_tab)
            .cloned()
            .collect();
        let mut groups: Vec<&str> = commands.iter().map(|c| c.placement.group.as_ref()).collect();
        groups.dedup();
        ui.horizontal(|ui| {
            for group in groups {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        for c in commands.iter().filter(|c| c.placement.group.as_ref() == group) {
                            let button = egui::Button::new(c.label.as_ref()).min_size(egui::vec2(64.0, 40.0));
                            let r = ui.add_enabled(c.is_available(), button).on_hover_text(c.tooltip()).on_disabled_hover_text(c.tooltip());
                            if r.clicked() {
                                self.cue = format!("{}: select objects", c.label);
                            }
                        }
                    });
                    ui.label(egui::RichText::new(group).small().weak());
                });
                ui.separator();
            }
        });
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let keys = ctx.input(|i| {
            let m = i.modifiers;
            [
                (m.ctrl && !m.shift && !m.alt && i.key_pressed(egui::Key::M), "Ctrl+M"),
                (m.ctrl && m.shift && i.key_pressed(egui::Key::D), "Ctrl+Shift+D"),
                (m.ctrl && m.alt && i.key_pressed(egui::Key::M), "Ctrl+Alt+M"),
            ]
            .into_iter()
            .find(|(hit, _)| *hit)
            .map(|(_, k)| k)
        });
        if let Some(k) = keys
            && let Some(c) = self.registry.by_shortcut(k)
        {
            let app = c.app.as_str().to_owned();
            self.switch(&app);
        }
    }
}

struct Viewer<'a> {
    registry: &'a AppRegistry,
    frame: u32,
}

impl TabViewer for Viewer<'_> {
    type Tab = Pane;
    fn id(&mut self, tab: &mut Pane) -> egui::Id {
        egui::Id::new(format!("{tab:?}"))
    }
    fn title(&mut self, tab: &mut Pane) -> egui::WidgetText {
        match tab {
            Pane::PartNavigator => "Part Navigator".into(),
            Pane::Roles => "Roles".into(),
            Pane::Graphics => "model1.pxp".into(),
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Pane) {
        match tab {
            Pane::PartNavigator => {
                egui::CollapsingHeader::new("Model History").default_open(true).show(ui, |ui| {
                    for (name, ok) in [("Datum Coordinate System (0)", true), ("Block (1)", true), ("Cylinder (2)", false)] {
                        ui.horizontal(|ui| {
                            ui.label(if ok { "✔" } else { "⚠" });
                            ui.label(name);
                        });
                    }
                });
                egui::CollapsingHeader::new("Applications").show(ui, |ui| {
                    for app in self.registry.apps() {
                        ui.label(format!("{} (Stage {})", app.name(), app.planned_stage()));
                    }
                });
            }
            Pane::Roles => {
                ui.label("Essentials");
                ui.label("Advanced");
            }
            Pane::Graphics => {
                let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::drag());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(58, 66, 82));
                // A wireframe cube in trimetric view: a stand-in for the wgpu viewport.
                let a = f32::from(u16::try_from(self.frame % 360).unwrap_or(0)).to_radians() * 0.0 + 0.6;
                let c = rect.center();
                let s = rect.height().min(rect.width()) * 0.18;
                let p = |x: f32, y: f32, z: f32| {
                    let (sa, ca) = a.sin_cos();
                    let (xr, zr) = (x * ca - z * sa, x * sa + z * ca);
                    egui::pos2(c.x + s * xr, c.y - s * (y * 0.9 - zr * 0.35))
                };
                let v: Vec<egui::Pos2> = (0..8).map(|i| p(if i & 1 == 0 { -1.0 } else { 1.0 }, if i & 2 == 0 { -1.0 } else { 1.0 }, if i & 4 == 0 { -1.0 } else { 1.0 })).collect();
                for (i, j) in [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)] {
                    if let (Some(&a), Some(&b)) = (v.get(i), v.get(j)) {
                        painter.line_segment([a, b], egui::Stroke::new(1.5, egui::Color32::from_rgb(230, 230, 235)));
                    }
                }
            }
        }
    }
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        self.frame += 1;
        self.shortcuts(ctx);
        egui::Panel::top("ribbon").show(ui, |ui| self.ribbon(ui));
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.label(egui::RichText::new(&self.cue).strong());
            ui.label(&self.status);
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let mut viewer = Viewer { registry: &self.registry, frame: self.frame };
            DockArea::new(&mut self.dock).style(Style::from_egui(ui.style().as_ref())).show_inside(ui, &mut viewer);
        });
        if let Some(path) = self.screenshot.clone() {
            if self.frame == 5 {
                let done = std::sync::Arc::clone(&self.shot_done);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                    save_png(&path, &img);
                    done.store(true, std::sync::atomic::Ordering::SeqCst);
                })));
            }
            if self.shot_done.load(std::sync::atomic::Ordering::SeqCst) || self.frame > 600 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        ctx.request_repaint();
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1600.0, 900.0]).with_title("PXP Engineering Suite — egui prototype"),
        ..Default::default()
    };
    eframe::run_native("pxp-proto-egui", options, Box::new(|_| Ok(Box::new(Shell::new()))))
}
