// SPDX-License-Identifier: LGPL-2.1-or-later
//! The NX-style shell: the eframe application that ties the pieces together.
//!
//! ```text
//! ┌ ribbon: [File ▾] tabs of the active application → groups → buttons (from the registry) ───────┐
//! ├ top border bar: selection filter ▾ | Command Finder [______] | undo / redo | view ▾ | theme ────┤
//! ├ resource bar (egui_dock) ┬ graphics window (dock tab, pxp-render) ───────────────────────────────┤
//! │ Part Navigator           │   model, view triad, WCS, datum axes; MB2 rotate, wheel zoom …       │
//! │ Assembly Navigator       │                                                                      │
//! │ Roles                    │   [feature dialog: Block / Cylinder / Cone / Sphere, floating]        │
//! ├ cue line: what to do next ───────────────────────────────────────────────────────────────────────┤
//! └ status line: what just happened, warnings (software rasteriser, defaults errors) ───────────────┘
//! ```
//!
//! Domain work goes through `pxp_app_modeling::session::PartSession`
//! (transactions, recompute, journal, autosave, display meshes); the shell
//! only gathers input and shows results.
//!
//! ## Keyboard (remappable command shortcuts come from customer defaults)
//!
//! | Key | Action |
//! | --- | --- |
//! | Home / End | trimetric / isometric view |
//! | F8 | snap to the nearest standard view |
//! | Ctrl+F | fit |
//! | Ctrl+Z / Ctrl+Y | undo / redo |
//! | Ctrl+N / Ctrl+O / Ctrl+S | new / open / save |
//! | Esc | cancel the dialog, else clear the selection |
//! | Ctrl+M, Ctrl+Shift+D, Ctrl+Alt+M | switch application (from the registry) |

use crate::defaults::{Defaults, Level};
use crate::dialog::{Dialog, Outcome};
use crate::keys::{Chord, ShortcutMap};
use crate::ribbon::{self, Role};
use crate::viewport::{Overlay, Pick, ViewPane};
use eframe::egui;
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use pxp_app_modeling::session::{OpenNotes, PartSession, Primitive};
use pxp_appkit::{AppId, AppRegistry, UiContext};
use pxp_doc::{ObjectId, ObjectState};
use pxp_kernel::KernelHost;
use pxp_render::{Camera, DisplaySettings, Projection, StandardView};
use pxp_units::UnitSystem;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Things to do right after start-up, for scripted screenshots and the
/// verification packet (`pxp --help` lists the flags).
#[derive(Debug, Clone, Default)]
pub struct Autostart {
    /// Create a new millimetre part here.
    pub new_part: Option<PathBuf>,
    /// Blocks to add: origin x, y, z and lengths x, y, z.
    pub blocks: Vec<[String; 6]>,
    /// Save the part after the blocks are added.
    pub save: bool,
    /// Open this feature dialog.
    pub dialog: Option<Primitive>,
    /// Click the centre of the graphics window (selects what is there).
    pub select_center: bool,
    /// Show this standard view.
    pub view: Option<StandardView>,
    /// Section along X at this coordinate (mm).
    pub section_x: Option<f64>,
    /// Save a PNG of the whole window here, then exit.
    pub screenshot: Option<PathBuf>,
}

/// Everything the shell needs from `pxp-app`.
pub struct ShellConfig {
    /// Registered applications (Modeling connected to the kernel).
    pub registry: AppRegistry,
    /// The geometry kernel (display meshes, mass properties).
    pub kernel: Arc<KernelHost>,
    /// Kernel name and version for Help > About.
    pub kernel_label: String,
    /// Customer defaults.
    pub defaults: Defaults,
    /// Problems found reading the defaults files.
    pub warnings: Vec<String>,
    /// A part to open at start-up.
    pub open: Option<PathBuf>,
    /// Scripted start-up actions.
    pub autostart: Autostart,
}

/// Dockable panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pane {
    /// Model History of the work part.
    PartNavigator,
    /// Assemblies arrive in Stage 1C.
    AssemblyNavigator,
    /// Role selection.
    Roles,
    /// The graphics window.
    Graphics,
}

/// The selection filter on the top border bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    /// No selection filter: the most specific entity (a face).
    None,
    /// Solid bodies.
    Body,
    /// Faces.
    Face,
    /// Features (the body a feature made, and its Part Navigator row).
    Feature,
}

impl Filter {
    const ALL: [Self; 4] = [Self::None, Self::Body, Self::Face, Self::Feature];
    const fn label(self) -> &'static str {
        match self {
            Self::None => "No Selection Filter",
            Self::Body => "Solid Body",
            Self::Face => "Face",
            Self::Feature => "Feature",
        }
    }
    /// Whether a pick selects one face (else the whole body).
    const fn picks_face(self) -> bool {
        matches!(self, Self::None | Self::Face)
    }
}

/// Status line severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    Info,
    Warning,
    Error,
}

/// The New / Open prompt (an in-app file prompt: no native dialog
/// dependency in Stage 0).
#[derive(Debug, Clone)]
struct FilePrompt {
    new: bool,
    path: String,
    inch: bool,
}

/// Section view state (View > Section).
#[derive(Debug, Clone, Copy)]
struct Section {
    on: bool,
    axis: usize,
    offset: f64,
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "independent window and mode flags, not a state machine"
)]
struct State {
    registry: AppRegistry,
    kernel: Arc<KernelHost>,
    kernel_label: String,
    defaults: Defaults,
    ui_ctx: UiContext,
    ribbon_tab: String,
    role: Role,
    dark: bool,
    shortcuts: ShortcutMap,
    session: Option<PartSession>,
    view: Option<ViewPane>,
    overlay: Overlay,
    selection: Option<(ObjectId, Option<u32>)>,
    filter: Filter,
    snaps: pxp_render::snap::SnapSet,
    dialog: Option<Dialog>,
    cue: String,
    status: String,
    severity: Severity,
    finder: String,
    info: Option<(String, String)>,
    prompt: Option<FilePrompt>,
    recovery: Option<OpenNotes>,
    defaults_open: bool,
    about_open: bool,
    named_views: Vec<(String, Camera)>,
    view_name: String,
    section: Section,
    context_menu: Option<egui::Pos2>,
    bodies_dirty: bool,
    frame: u64,
    autostart: Autostart,
    autostart_done: bool,
    shot_done: Arc<AtomicBool>,
    startup_warnings: Vec<String>,
    /// Shown permanently at the right of the status line (software
    /// rasteriser, no renderer).
    gpu_warning: Option<String>,
    last_tick: Instant,
}

/// The desktop shell (an `eframe::App`).
pub struct Shell {
    dock: DockState<Pane>,
    st: State,
}

impl Shell {
    /// Builds the shell. `cc` gives eframe's wgpu device; without one (a
    /// backend other than wgpu) the graphics window says so instead of
    /// drawing.
    #[must_use]
    #[allow(
        clippy::too_many_lines,
        reason = "one-time wiring of every shell part, read top to bottom"
    )]
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: ShellConfig) -> Self {
        let ShellConfig {
            registry,
            kernel,
            kernel_label,
            defaults,
            warnings,
            open,
            autostart,
        } = cfg;
        let dark = defaults.text("theme", "light").eq_ignore_ascii_case("dark");
        cc.egui_ctx.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        let mut settings = if dark {
            DisplaySettings::DARK
        } else {
            DisplaySettings::LIGHT
        };
        settings.edges = defaults.flag("edges", true);
        settings.silhouettes = defaults.flag("silhouettes", true);
        let mut view = cc
            .wgpu_render_state
            .as_ref()
            .map(|rs| ViewPane::new(rs, settings));
        if let Some(v) = view.as_mut()
            && defaults
                .text("projection", "orthographic")
                .eq_ignore_ascii_case("perspective")
        {
            v.camera.projection = Projection::Perspective;
        }
        let shortcuts = build_shortcuts(&registry, &defaults);
        let mut dock = DockState::new(vec![Pane::Graphics]);
        let _ = dock.main_surface_mut().split_left(
            NodeIndex::root(),
            0.2,
            vec![Pane::PartNavigator, Pane::AssemblyNavigator, Pane::Roles],
        );
        let mut startup_warnings = warnings;
        for (chord, kept, lost) in &shortcuts.conflicts {
            startup_warnings.push(format!(
                "shortcut {chord}: kept {kept}, removed from {lost}"
            ));
        }
        for (id, text) in &shortcuts.invalid {
            startup_warnings.push(format!("shortcut for {id}: '{text}' is not a key chord"));
        }
        let gpu_warning = match view.as_ref() {
            Some(v) => v.software_warning(),
            None => {
                Some("The graphics window needs the wgpu renderer, which did not start.".into())
            }
        };
        let mut st = State {
            role: Role::parse(defaults.text("role", "advanced")),
            registry,
            kernel,
            kernel_label,
            defaults,
            ui_ctx: UiContext::default(),
            ribbon_tab: String::new(),
            dark,
            shortcuts,
            session: None,
            view,
            overlay: Overlay::default(),
            selection: None,
            filter: Filter::None,
            snaps: pxp_render::snap::SnapSet::default(),
            dialog: None,
            cue: "Open a part (Ctrl+O) or create one (Ctrl+N)".into(),
            status: String::new(),
            severity: Severity::Info,
            finder: String::new(),
            info: None,
            prompt: None,
            recovery: None,
            defaults_open: false,
            about_open: false,
            named_views: Vec::new(),
            view_name: String::new(),
            section: Section {
                on: false,
                axis: 0,
                offset: 0.0,
            },
            context_menu: None,
            bodies_dirty: false,
            frame: 0,
            autostart,
            autostart_done: false,
            shot_done: Arc::new(AtomicBool::new(false)),
            startup_warnings,
            gpu_warning,
            last_tick: Instant::now(),
        };
        st.switch_app("pxp.modeling");
        if let Some(w) = st.startup_warnings.first().cloned() {
            st.say(Severity::Warning, w);
        }
        // Opening last, so its result is what the status line shows.
        if let Some(path) = open {
            st.open(&path);
        }
        Self { dock, st }
    }
}

fn build_shortcuts(registry: &AppRegistry, defaults: &Defaults) -> ShortcutMap {
    let from_commands: Vec<(String, String)> = registry
        .commands()
        .iter()
        .filter_map(|c| {
            c.shortcut
                .as_ref()
                .map(|k| (c.id.to_string(), k.to_string()))
        })
        .collect();
    ShortcutMap::build(&from_commands, &defaults.shortcuts())
}

impl State {
    fn say(&mut self, severity: Severity, text: impl Into<String>) {
        self.severity = severity;
        self.status = text.into();
    }

    fn switch_app(&mut self, id: &str) {
        let Ok(target) = AppId::new(id) else { return };
        match self.registry.switch_to(&target, &mut self.ui_ctx) {
            Ok(()) => {
                self.ribbon_tab = self
                    .ui_ctx
                    .ribbon_tabs
                    .first()
                    .map(|t| (*t).to_owned())
                    .unwrap_or_default();
                let name = self
                    .registry
                    .apps()
                    .find(|a| a.id() == target)
                    .map_or(id, |a| a.name());
                self.say(Severity::Info, format!("Application: {name}"));
            }
            Err(e) => self.say(Severity::Error, e.to_string()),
        }
    }

    fn inch(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| s.meta().units == UnitSystem::Inch)
    }

    // ---- files -----------------------------------------------------------

    fn install(&mut self, mut s: PartSession) {
        let minutes = self.defaults.number("autosave_minutes", 5);
        if minutes > 0 {
            s.set_autosave_interval(Duration::from_secs(minutes * 60));
        }
        self.session = Some(s);
        self.selection = None;
        self.dialog = None;
        self.bodies_dirty = true;
        self.cue = "Select a command or an object".into();
    }

    fn close_part(&mut self) {
        if let Some(s) = self.session.take()
            && let Err(e) = s.close()
        {
            self.say(Severity::Error, format!("closing: {e}"));
        }
        if let Some(v) = self.view.as_mut() {
            v.set_bodies(&[]);
        }
        self.overlay = Overlay::default();
    }

    fn new_part(&mut self, path: &std::path::Path, units: UnitSystem) {
        self.close_part();
        match PartSession::create(path, units, &self.registry) {
            Ok(s) => {
                self.install(s);
                self.say(Severity::Info, format!("Created {}", path.display()));
            }
            Err(e) => self.say(Severity::Error, format!("New part: {e}")),
        }
    }

    fn open(&mut self, path: &std::path::Path) {
        self.close_part();
        match PartSession::open(path) {
            Ok((s, notes)) => {
                let ro = notes.read_only.clone();
                if notes.recovery.is_some() || notes.autosave.is_some() {
                    self.recovery = Some(notes);
                }
                self.install(s);
                match ro {
                    Some(why) => self.say(
                        Severity::Warning,
                        format!("Opened {} read-only: {why}", path.display()),
                    ),
                    None => self.say(Severity::Info, format!("Opened {}", path.display())),
                }
            }
            Err(e) => self.say(Severity::Error, format!("Open: {e}")),
        }
    }

    fn save(&mut self) {
        let Some(s) = self.session.as_mut() else {
            return self.say(Severity::Warning, "No part is open");
        };
        match s.save() {
            Ok(()) => {
                let p = s.path().display().to_string();
                self.say(Severity::Info, format!("Saved {p}"));
            }
            Err(e) => self.say(Severity::Error, format!("Save: {e}")),
        }
    }

    // ---- commands --------------------------------------------------------

    fn run_command(&mut self, id: &str) {
        let Some(spec) = self.registry.command(id).cloned() else {
            return self.say(Severity::Error, format!("unknown command {id}"));
        };
        if !spec.is_available() {
            return self.say(
                Severity::Info,
                format!("{}: {}", spec.label, spec.tooltip()),
            );
        }
        if let Some(app) = id.strip_prefix("app.switch.") {
            return self.switch_app(app);
        }
        if self.session.is_none() {
            return self.say(
                Severity::Warning,
                format!(
                    "{}: open or create a part first (Ctrl+O / Ctrl+N)",
                    spec.label
                ),
            );
        }
        let primitive = match id {
            "modeling.block" => Some(Primitive::Block),
            "modeling.cylinder" => Some(Primitive::Cylinder),
            "modeling.cone" => Some(Primitive::Cone),
            "modeling.sphere" => Some(Primitive::Sphere),
            _ => None,
        };
        if let Some(p) = primitive {
            let d = Dialog::new(p, self.inch());
            self.cue = d.cue();
            self.dialog = Some(d);
            return;
        }
        if id == "modeling.datum_csys" {
            let origin = self.overlay.wcs.map_or([0.0; 3], |(o, _)| o);
            let result = self
                .session
                .as_mut()
                .map(|s| s.add_datum_csys(&self.registry, origin));
            match result {
                Some(Ok(_)) => {
                    self.bodies_dirty = true;
                    self.say(Severity::Info, "Datum CSYS created at the WCS origin");
                }
                Some(Err(e)) => self.say(Severity::Error, format!("Datum CSYS: {e}")),
                None => {}
            }
            return;
        }
        // An available command with no shell binding is a wiring bug: say so
        // rather than do nothing.
        self.say(
            Severity::Error,
            format!(
                "{} is available but the desktop shell has no dialog for it yet",
                spec.label
            ),
        );
    }

    fn finish_dialog(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::None => {}
            Outcome::Cancel => {
                self.dialog = None;
                self.cue = "Select a command or an object".into();
            }
            Outcome::Ok | Outcome::Apply => {
                let Some(d) = self.dialog.as_mut() else {
                    return;
                };
                if let Some(field) = d.missing() {
                    d.error = Some(format!("{field} is empty"));
                    return;
                }
                let values = d.values();
                let name = d.name.trim().to_owned();
                let kind = d.kind;
                let Some(session) = self.session.as_mut() else {
                    return;
                };
                let name = (!name.is_empty()).then_some(name);
                match session.add_primitive(&self.registry, kind, name.as_deref(), &values) {
                    Ok((id, failed)) => {
                        self.bodies_dirty = true;
                        self.selection = Some((id, None));
                        match failed {
                            Some(why) => self.say(
                                Severity::Error,
                                format!("{} failed to update: {why}", kind.label()),
                            ),
                            None => self.say(Severity::Info, format!("{} created", kind.label())),
                        }
                        if outcome == Outcome::Ok {
                            self.dialog = None;
                            self.cue = "Select a command or an object".into();
                        } else if let Some(d) = self.dialog.as_mut() {
                            d.error = None;
                        }
                    }
                    Err(e) => {
                        if let Some(d) = self.dialog.as_mut() {
                            d.error = Some(e.to_string());
                        }
                    }
                }
            }
        }
    }

    fn undo(&mut self, redo: bool) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        let r = if redo {
            s.redo(&self.registry)
        } else {
            s.undo(&self.registry)
        };
        match r {
            Ok(label) => {
                self.bodies_dirty = true;
                self.selection = None;
                self.say(
                    Severity::Info,
                    format!("{} {label}", if redo { "Redo" } else { "Undo" }),
                );
            }
            Err(e) => self.say(Severity::Info, e.to_string()),
        }
    }

    fn information(&mut self, feature: ObjectId) {
        let Some(s) = self.session.as_ref() else {
            return;
        };
        let row = s.history().into_iter().find(|r| r.id == feature);
        let Some(row) = row else { return };
        let mut text = format!(
            "Name: {}\nType: {}\nState: {:?}\n",
            row.name, row.kind, row.state
        );
        if let Some(e) = &row.error {
            let _ = writeln!(text, "Reason: {e}");
        }
        let inch = s.meta().units == UnitSystem::Inch;
        match s.mass_properties(feature, &self.kernel) {
            Ok(m) => {
                let (lu, k) = if inch { ("in", 25.4) } else { ("mm", 1.0) };
                let c = m.centroid;
                let _ = write!(
                    text,
                    "\nMass properties (unit density, {}):\nVolume: {:.6} {lu}³\nArea: {:.6} {lu}²\nCentroid: ({:.6}, {:.6}, {:.6}) {lu}\n",
                    self.kernel_label,
                    m.volume.mm3() / (k * k * k),
                    m.area.mm2() / (k * k),
                    c.x / k,
                    c.y / k,
                    c.z / k
                );
            }
            Err(e) if row.kind != "Datum Coordinate System" => {
                let _ = writeln!(text, "\nMass properties: {e}");
            }
            Err(_) => {}
        }
        self.info = Some((format!("Information: {}", row.name), text));
    }

    // ---- per-frame upkeep -----------------------------------------------

    fn refresh_bodies(&mut self) {
        if !self.bodies_dirty {
            return;
        }
        self.bodies_dirty = false;
        let Some(s) = self.session.as_ref() else {
            return;
        };
        let (bodies, problems) = s.display_bodies(&self.kernel);
        self.overlay.datums = s.datum_frames();
        self.overlay.wcs = Some((
            [0.0; 3],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        ));
        if let Some(v) = self.view.as_mut() {
            v.set_bodies(&bodies);
        }
        if let Some(p) = problems.first() {
            self.say(Severity::Warning, format!("Not displayed: {p}"));
        }
        self.apply_highlight();
    }

    fn apply_highlight(&mut self) {
        let Some(v) = self.view.as_mut() else { return };
        v.settings.highlight = self
            .selection
            .and_then(|(f, face)| v.body_of(f).map(|b| (b, face)));
        v.settings.section = self.section.on.then(|| {
            let mut n = [0.0; 3];
            if let Some(x) = n.get_mut(self.section.axis) {
                *x = 1.0;
            }
            (n, -self.section.offset)
        });
        v.invalidate();
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_tick) < Duration::from_secs(5) {
            return;
        }
        self.last_tick = now;
        if let Some(s) = self.session.as_mut() {
            match s.tick(now) {
                Ok(Some(p)) => self.say(Severity::Info, format!("Autosaved to {}", p.display())),
                Ok(None) => {}
                Err(e) => self.say(Severity::Warning, format!("Autosave failed: {e}")),
            }
        }
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            // A text field has focus: Esc still cancels a dialog.
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && self.dialog.is_some() {
                self.finish_dialog(Outcome::Cancel);
            }
            return;
        }
        let presses: Vec<(egui::Key, egui::Modifiers)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } => Some((*key, *modifiers)),
                    _ => None,
                })
                .collect()
        });
        for (key, m) in presses {
            let chord = Chord {
                ctrl: m.command,
                alt: m.alt,
                shift: m.shift,
                key: key.name().to_owned(),
            };
            let text = chord.to_string();
            match text.as_str() {
                "Home" => self.with_view(|v| v.standard_view(StandardView::Trimetric)),
                "End" => self.with_view(|v| v.standard_view(StandardView::Isometric)),
                "F8" => self.with_view(ViewPane::snap),
                "Ctrl+F" => self.with_view(ViewPane::fit),
                "Ctrl+Z" => self.undo(false),
                "Ctrl+Y" => self.undo(true),
                "Ctrl+S" => self.save(),
                "Ctrl+N" => {
                    self.prompt = Some(FilePrompt {
                        new: true,
                        path: "model1.pxp".into(),
                        inch: self.defaults.text("units", "mm") == "in",
                    });
                }
                "Ctrl+O" => {
                    self.prompt = Some(FilePrompt {
                        new: false,
                        path: String::new(),
                        inch: false,
                    });
                }
                "Escape" => {
                    if self.dialog.is_some() {
                        self.finish_dialog(Outcome::Cancel);
                    } else {
                        self.selection = None;
                        self.apply_highlight();
                    }
                }
                "Enter" if self.dialog.is_some() => self.finish_dialog(Outcome::Ok),
                _ => {
                    if let Some(id) = self.shortcuts.command(&chord).map(str::to_owned) {
                        self.run_command(&id);
                    }
                }
            }
        }
    }

    /// The graphics window has drawn at least once (it has a size).
    fn dirty_view_ready(&self) -> bool {
        self.view.is_some()
    }

    fn with_view(&mut self, f: impl FnOnce(&mut ViewPane)) {
        if let Some(v) = self.view.as_mut() {
            f(v);
        }
    }

    // ---- scripted start-up ------------------------------------------------

    fn autostart(&mut self, ctx: &egui::Context) {
        if self.autostart_done {
            if let Some(path) = self.autostart.screenshot.clone()
                && self.frame == 25
            {
                let done = Arc::clone(&self.shot_done);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                    egui::ScreenshotCallback::new(move |img| {
                        let [w, h] = img.size;
                        let bytes: Vec<u8> = img
                            .pixels
                            .iter()
                            .flat_map(egui::Color32::to_array)
                            .collect();
                        if let Some(buf) = image::RgbaImage::from_raw(
                            u32::try_from(w).unwrap_or(0),
                            u32::try_from(h).unwrap_or(0),
                            bytes,
                        ) && let Err(e) = buf.save(&path)
                        {
                            eprintln!("pxp: screenshot {}: {e}", path.display());
                        }
                        done.store(true, Ordering::SeqCst);
                    }),
                ));
            }
            if self.autostart.screenshot.is_some()
                && (self.shot_done.load(Ordering::SeqCst) || self.frame > 900)
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            if self.autostart.select_center && self.frame == 12 {
                self.autostart.select_center = false;
                if self.dirty_view_ready()
                    && let Some(p) = self.view.as_mut().and_then(ViewPane::click_center)
                {
                    let selecting = self
                        .dialog
                        .as_ref()
                        .is_none_or(|d| d.step.is_none_or(|s| !s.active));
                    self.on_pick(p);
                    if selecting && let Some((f, _)) = self.selection {
                        self.information(f);
                    }
                }
            }
            return;
        }
        if self.frame < 3 {
            return;
        }
        self.autostart_done = true;
        let a = self.autostart.clone();
        if let Some(p) = a.new_part {
            self.new_part(&p, UnitSystem::Millimetre);
        }
        for b in &a.blocks {
            if let Some(s) = self.session.as_mut() {
                let r = s.add_primitive(&self.registry, Primitive::Block, None, b);
                match r {
                    Ok(_) => self.bodies_dirty = true,
                    Err(e) => self.say(Severity::Error, format!("--block: {e}")),
                }
            }
        }
        if a.save {
            self.save();
        }
        self.refresh_bodies();
        if let Some(v) = a.view {
            self.with_view(|pane| pane.standard_view(v));
        }
        if let Some(x) = a.section_x {
            self.section = Section {
                on: true,
                axis: 0,
                offset: x,
            };
            self.apply_highlight();
        }
        if let Some(p) = a.dialog {
            let d = Dialog::new(p, self.inch());
            self.cue = d.cue();
            self.dialog = Some(d);
        }
    }
}

// ---- drawing -------------------------------------------------------------

impl State {
    fn ribbon(&mut self, ui: &mut egui::Ui) {
        let tabs = ribbon::build(&self.registry, &self.ui_ctx.ribbon_tabs, self.role);
        ui.horizontal(|ui| {
            ui.menu_button("File", |ui| self.file_menu(ui));
            ui.separator();
            for t in &tabs {
                if ui
                    .selectable_label(self.ribbon_tab == t.name, &t.name)
                    .clicked()
                {
                    self.ribbon_tab.clone_from(&t.name);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Help").clicked() {
                    self.about_open = true;
                }
                if ui.button("Customer Defaults").clicked() {
                    self.defaults_open = true;
                }
            });
        });
        let Some(tab) = tabs
            .iter()
            .find(|t| t.name == self.ribbon_tab)
            .or_else(|| tabs.first())
        else {
            return;
        };
        let mut clicked = None;
        egui::ScrollArea::horizontal()
            .id_salt("ribbon-scroll")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for g in &tab.groups {
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                for c in &g.commands {
                                    let mut tip = c.tooltip();
                                    if let Some(k) = self.shortcuts.chord_of(&c.id) {
                                        let _ = write!(tip, "  ({k})");
                                    }
                                    let b = egui::Button::new(c.label.as_ref())
                                        .min_size(egui::vec2(58.0, 40.0));
                                    let r = ui
                                        .add_enabled(c.is_available(), b)
                                        .on_hover_text(&tip)
                                        .on_disabled_hover_text(&tip);
                                    if r.clicked() {
                                        clicked = Some(c.id.to_string());
                                    }
                                }
                            });
                            ui.label(egui::RichText::new(&g.name).small().weak());
                        });
                        ui.separator();
                    }
                });
            });
        if let Some(id) = clicked {
            self.run_command(&id);
        }
    }

    fn file_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("New…  (Ctrl+N)").clicked() {
            self.prompt = Some(FilePrompt {
                new: true,
                path: "model1.pxp".into(),
                inch: self.defaults.text("units", "mm") == "in",
            });
            ui.close();
        }
        if ui.button("Open…  (Ctrl+O)").clicked() {
            self.prompt = Some(FilePrompt {
                new: false,
                path: String::new(),
                inch: false,
            });
            ui.close();
        }
        if ui
            .add_enabled(self.session.is_some(), egui::Button::new("Save  (Ctrl+S)"))
            .clicked()
        {
            self.save();
            ui.close();
        }
        if ui
            .add_enabled(self.session.is_some(), egui::Button::new("Close"))
            .clicked()
        {
            self.close_part();
            self.cue = "Open a part (Ctrl+O) or create one (Ctrl+N)".into();
            ui.close();
        }
        ui.separator();
        if ui.button("Exit").clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn top_border_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("filter")
                .selected_text(self.filter.label())
                .show_ui(ui, |ui| {
                    for f in Filter::ALL {
                        ui.selectable_value(&mut self.filter, f, f.label());
                    }
                });
            ui.separator();
            self.snap_toolbar(ui);
            ui.separator();
            let undo = self
                .session
                .as_ref()
                .is_some_and(|s| s.document().can_undo());
            let redo = self
                .session
                .as_ref()
                .is_some_and(|s| s.document().can_redo());
            if ui.add_enabled(undo, egui::Button::new("Undo")).clicked() {
                self.undo(false);
            }
            if ui.add_enabled(redo, egui::Button::new("Redo")).clicked() {
                self.undo(true);
            }
            ui.separator();
            ui.menu_button("View", |ui| self.view_menu(ui));
            if ui.selectable_label(self.dark, "Dark").clicked() {
                self.dark = !self.dark;
                ui.ctx().set_visuals(if self.dark {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                });
                let dark = self.dark;
                self.with_view(|v| {
                    let keep = v.settings;
                    let base = if dark {
                        DisplaySettings::DARK
                    } else {
                        DisplaySettings::LIGHT
                    };
                    v.settings = DisplaySettings {
                        edges: keep.edges,
                        silhouettes: keep.silhouettes,
                        section: keep.section,
                        highlight: keep.highlight,
                        ..base
                    };
                    v.invalidate();
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.finder)
                        .hint_text("Command Finder")
                        .desired_width(220.0),
                );
                if !self.finder.trim().is_empty() && (r.has_focus() || r.lost_focus()) {
                    let found: Vec<(String, String, String)> = self
                        .registry
                        .find(self.finder.trim())
                        .into_iter()
                        .take(12)
                        .map(|c| {
                            (
                                c.id.to_string(),
                                format!(
                                    "{}  ({} › {})",
                                    c.label, c.placement.tab, c.placement.group
                                ),
                                c.tooltip(),
                            )
                        })
                        .collect();
                    let mut run = None;
                    egui::Popup::from_response(&r).open(true).show(|ui| {
                        if found.is_empty() {
                            ui.label("No command matches");
                        }
                        for (id, label, tip) in &found {
                            if ui.button(label).on_hover_text(tip).clicked() {
                                run = Some(id.clone());
                            }
                        }
                    });
                    if let Some(id) = run {
                        self.finder.clear();
                        self.run_command(&id);
                    }
                }
            });
        });
    }

    /// The snap toolbar: what a "Specify Point" click snaps to.
    fn snap_toolbar(&mut self, ui: &mut egui::Ui) {
        let set = &mut self.snaps;
        for (on, label, tip) in [
            (
                &mut set.existing,
                "Existing",
                "Existing Point: datum origins",
            ),
            (&mut set.end, "End", "End Point: edge ends"),
            (&mut set.mid, "Mid", "Mid Point: edge middles"),
            (
                &mut set.on_face,
                "On Face",
                "Point on Face: the point under the cursor",
            ),
        ] {
            if ui.selectable_label(*on, label).on_hover_text(tip).clicked() {
                *on = !*on;
            }
        }
        for label in ["Control", "Intersection", "Arc Center", "Quadrant"] {
            ui.add_enabled(false, egui::Button::new(label).small())
                .on_disabled_hover_text("Planned for Stage 1.");
        }
    }

    fn view_menu(&mut self, ui: &mut egui::Ui) {
        for v in StandardView::ALL {
            if ui.button(v.label()).clicked() {
                self.with_view(|p| p.standard_view(v));
                ui.close();
            }
        }
        ui.separator();
        if ui.button("Fit  (Ctrl+F)").clicked() {
            self.with_view(ViewPane::fit);
            ui.close();
        }
        if ui.button("Snap View  (F8)").clicked() {
            self.with_view(ViewPane::snap);
            ui.close();
        }
        ui.separator();
        let mut changed = false;
        if let Some(v) = self.view.as_mut() {
            changed |= ui.checkbox(&mut v.settings.edges, "Edges").changed();
            changed |= ui
                .checkbox(&mut v.settings.silhouettes, "Silhouettes")
                .changed();
            let mut persp = v.camera.projection == Projection::Perspective;
            if ui.checkbox(&mut persp, "Perspective").changed() {
                v.camera.projection = if persp {
                    Projection::Perspective
                } else {
                    Projection::Orthographic
                };
                changed = true;
            }
        }
        ui.separator();
        ui.label("Section");
        changed |= ui.checkbox(&mut self.section.on, "Clip").changed();
        ui.horizontal(|ui| {
            for (i, a) in ["X", "Y", "Z"].iter().enumerate() {
                changed |= ui.radio_value(&mut self.section.axis, i, *a).changed();
            }
        });
        changed |= ui
            .add(
                egui::DragValue::new(&mut self.section.offset)
                    .speed(1.0)
                    .suffix(" mm"),
            )
            .changed();
        if changed {
            self.apply_highlight();
        }
        ui.separator();
        ui.label("Named views");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.view_name).desired_width(100.0));
            if ui.button("Save").clicked()
                && !self.view_name.trim().is_empty()
                && let Some(v) = self.view.as_ref()
            {
                let name = self.view_name.trim().to_owned();
                self.named_views.retain(|(n, _)| *n != name);
                self.named_views.push((name, v.camera));
                self.view_name.clear();
            }
        });
        let mut restore = None;
        for (n, c) in &self.named_views {
            if ui.button(n).clicked() {
                restore = Some(*c);
            }
        }
        if let Some(c) = restore {
            self.with_view(|v| v.set_camera(c));
            ui.close();
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context, hover: Option<egui::Pos2>) {
        let mut outcome = Outcome::None;
        if let Some(d) = self.dialog.as_mut() {
            egui::Window::new(d.title())
                .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 120.0))
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new("Type: Origin and Edge Lengths").weak());
                    if let Some(st) = d.step.as_mut() {
                        let text = if st.active {
                            format!("> {} (click in the graphics window)", st.label)
                        } else {
                            format!("{} (done)", st.label)
                        };
                        if ui
                            .selectable_label(st.active, text)
                            .on_hover_text("Click to pick the point again")
                            .clicked()
                        {
                            st.active = true;
                        }
                    }
                    for (caption, idx) in d.groups.clone() {
                        ui.separator();
                        ui.strong(caption);
                        egui::Grid::new(caption).num_columns(2).show(ui, |ui| {
                            for i in idx {
                                if let Some(f) = d.fields.get_mut(i) {
                                    ui.label(f.label);
                                    let r = ui.add(
                                        egui::TextEdit::singleline(&mut f.value)
                                            .desired_width(110.0),
                                    );
                                    if r.changed() || r.gained_focus() {
                                        d.active = i;
                                    }
                                    if r.lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                    {
                                        outcome = Outcome::Ok;
                                    }
                                    ui.end_row();
                                }
                            }
                        });
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        ui.add(
                            egui::TextEdit::singleline(&mut d.name)
                                .hint_text("automatic")
                                .desired_width(120.0),
                        );
                    });
                    if let Some(e) = &d.error {
                        ui.colored_label(egui::Color32::from_rgb(200, 40, 40), e);
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            outcome = Outcome::Ok;
                        }
                        if ui.button("Apply").clicked() {
                            outcome = Outcome::Apply;
                        }
                        if ui.button("Cancel").clicked() {
                            outcome = Outcome::Cancel;
                        }
                    });
                });
            // The on-screen input box: the active field, beside the cursor.
            if let Some(p) = hover
                && let Some(f) = d.fields.get_mut(d.active)
            {
                egui::Area::new(egui::Id::unique("onscreen-input"))
                    .fixed_pos(p + egui::vec2(18.0, 18.0))
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(f.label);
                                ui.add(
                                    egui::TextEdit::singleline(&mut f.value).desired_width(70.0),
                                );
                            });
                        });
                    });
            }
        }
        self.finish_dialog(outcome);
        self.prompt_window(ctx);
        self.recovery_window(ctx);
        self.info_window(ctx);
        self.defaults_window(ctx);
        self.about_window(ctx);
    }

    fn prompt_window(&mut self, ctx: &egui::Context) {
        let Some(mut p) = self.prompt.take() else {
            return;
        };
        let mut keep = true;
        let mut go = false;
        egui::Window::new(if p.new { "New Part" } else { "Open Part" })
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("File");
                    let r = ui.add(egui::TextEdit::singleline(&mut p.path).desired_width(360.0));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        go = true;
                    }
                });
                if p.new {
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut p.inch, false, "Millimetres");
                        ui.radio_value(&mut p.inch, true, "Inches");
                    });
                } else {
                    // Parts in the folder of the typed path (or the current one).
                    let dir = std::path::Path::new(&p.path)
                        .parent()
                        .filter(|d| !d.as_os_str().is_empty())
                        .map_or_else(|| PathBuf::from("."), PathBuf::from);
                    let mut parts: Vec<PathBuf> = std::fs::read_dir(&dir)
                        .map(|rd| {
                            rd.filter_map(Result::ok)
                                .map(|e| e.path())
                                .filter(|q| q.extension().is_some_and(|x| x == "pxp"))
                                .collect()
                        })
                        .unwrap_or_default();
                    parts.sort();
                    egui::ScrollArea::vertical()
                        .max_height(160.0)
                        .show(ui, |ui| {
                            for q in parts {
                                if ui
                                    .selectable_label(false, q.display().to_string())
                                    .clicked()
                                {
                                    p.path = q.display().to_string();
                                }
                            }
                        });
                }
                ui.horizontal(|ui| {
                    if ui.button(if p.new { "Create" } else { "Open" }).clicked() {
                        go = true;
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            });
        if go && !p.path.trim().is_empty() {
            let path = PathBuf::from(p.path.trim());
            if p.new {
                self.new_part(
                    &path,
                    if p.inch {
                        UnitSystem::Inch
                    } else {
                        UnitSystem::Millimetre
                    },
                );
            } else {
                self.open(&path);
            }
        } else if keep {
            self.prompt = Some(p);
        }
    }

    fn recovery_window(&mut self, ctx: &egui::Context) {
        let Some(notes) = self.recovery.clone() else {
            return;
        };
        let mut choice = 0;
        egui::Window::new("Unsaved work found")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                if let Some(r) = &notes.recovery {
                    ui.label(format!(
                        "{} change(s) were not saved; the last was \"{}\".",
                        r.transactions, r.last_label
                    ));
                    if ui.button("Restore unsaved work").clicked() {
                        choice = 1;
                    }
                } else if notes.autosave.is_some() {
                    ui.label("An autosaved copy newer than the saved part exists.");
                    if ui.button("Restore the autosave").clicked() {
                        choice = 2;
                    }
                }
                if ui.button("Discard it and open the saved part").clicked() {
                    choice = 3;
                }
            });
        let Some(s) = self.session.as_mut() else {
            self.recovery = None;
            return;
        };
        let r = match choice {
            1 => s.restore_recovery(),
            2 => s.restore_autosave(),
            3 => s.discard_unsaved(),
            _ => return,
        };
        self.recovery = None;
        self.bodies_dirty = true;
        match r {
            Ok(()) => self.say(
                Severity::Info,
                if choice == 3 {
                    "Unsaved work discarded"
                } else {
                    "Unsaved work restored (not yet saved)"
                },
            ),
            Err(e) => self.say(Severity::Error, format!("Recovery: {e}")),
        }
    }

    fn info_window(&mut self, ctx: &egui::Context) {
        let Some((title, text)) = self.info.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new("Information")
            .id(egui::Id::unique("information"))
            .open(&mut open)
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -60.0))
            .show(ctx, |ui| {
                ui.strong(&title);
                ui.add(egui::Label::new(egui::RichText::new(&text).monospace()));
            });
        if !open {
            self.info = None;
        }
    }

    fn defaults_window(&mut self, ctx: &egui::Context) {
        if !self.defaults_open {
            return;
        }
        let mut open = true;
        let mut changes: Vec<(String, Option<String>)> = Vec::new();
        let mut save = false;
        egui::Window::new("Customer Defaults").open(&mut open).show(ctx, |ui| {
            ui.label("Shipped < Site (PXP_SITE_DEFAULTS) < User. Locked site values cannot be changed here.");
            egui::Grid::new("defaults").striped(true).num_columns(4).show(ui, |ui| {
                ui.strong("Key");
                ui.strong("Value");
                ui.strong("Level");
                ui.strong("");
                ui.end_row();
                for key in self.defaults.keys() {
                    let (value, level) = self.defaults.get(&key).map_or((String::new(), Level::Shipped), |(v, l)| (v.to_owned(), l));
                    let locked = self.defaults.is_locked(&key);
                    ui.label(&key);
                    let mut edit = value.clone();
                    let r = ui.add_enabled(!locked, egui::TextEdit::singleline(&mut edit).desired_width(140.0));
                    if r.changed() {
                        changes.push((key.clone(), Some(edit)));
                    }
                    ui.label(if locked { "Site (locked)" } else { level.label() });
                    if level == Level::User && ui.small_button("Reset").clicked() {
                        changes.push((key.clone(), None));
                    }
                    ui.end_row();
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Save user defaults").clicked() {
                    save = true;
                }
                ui.label(self.defaults.user_path().map_or_else(|| "(no user folder)".to_owned(), |p| p.display().to_string()));
            });
            ui.label(egui::RichText::new("Theme, role and shortcut changes apply now; the rest at the next start.").weak());
        });
        for (k, v) in changes {
            if let Err(e) = self.defaults.set_user(&k, v.as_deref()) {
                self.say(Severity::Warning, e);
            }
        }
        self.role = Role::parse(self.defaults.text("role", "advanced"));
        self.shortcuts = build_shortcuts(&self.registry, &self.defaults);
        if save {
            match self.defaults.save_user() {
                Ok(p) => self.say(Severity::Info, format!("Saved {}", p.display())),
                Err(e) => self.say(Severity::Error, e),
            }
        }
        self.defaults_open = open;
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        let mut open = true;
        let adapter = self
            .view
            .as_ref()
            .map_or_else(|| "none".to_owned(), ViewPane::adapter);
        egui::Window::new("About PXP Engineering Suite")
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Version {} (pre-alpha, Stage 0), LGPL-2.1-or-later",
                    env!("CARGO_PKG_VERSION")
                ));
                ui.label(format!("Geometry kernel: {}", self.kernel_label));
                ui.label(format!("Graphics: {adapter}"));
                ui.label(format!(
                    "Crash reports (local only): {}",
                    pxp_diag::crash::default_dir().display()
                ));
                for w in self.gpu_warning.iter().chain(&self.startup_warnings) {
                    ui.colored_label(egui::Color32::from_rgb(190, 120, 0), w);
                }
            });
        self.about_open = open;
    }
}

impl State {
    fn cue_and_status(&self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new(&self.cue).strong());
        let color = match self.severity {
            Severity::Info => ui.visuals().text_color(),
            Severity::Warning => egui::Color32::from_rgb(190, 120, 0),
            Severity::Error => egui::Color32::from_rgb(200, 40, 40),
        };
        ui.horizontal(|ui| {
            ui.colored_label(color, &self.status);
            if let Some(w) = &self.gpu_warning {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.colored_label(egui::Color32::from_rgb(190, 120, 0), w);
                });
            }
        });
    }

    fn on_pick(&mut self, p: Pick) {
        // A dialog waiting for a point takes the click (snapped).
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| d.step.is_some_and(|s| s.active))
        {
            let clicked = match p {
                Pick::Face(_, _, c) | Pick::Nothing(c) => c,
            };
            let points: Vec<pxp_render::camera::V3> =
                self.overlay.datums.iter().map(|(o, _)| *o).collect();
            let snapped = self
                .view
                .as_ref()
                .and_then(|v| v.snap_point(&clicked, &points, self.snaps));
            let inch = self.inch();
            match snapped {
                Some((pt, kind)) => {
                    if let Some(d) = self.dialog.as_mut() {
                        d.set_point(pt, inch);
                        self.cue = d.cue();
                    }
                    self.say(
                        Severity::Info,
                        format!(
                            "Point: {} ({:.4}, {:.4}, {:.4}) mm",
                            kind.label(),
                            pt[0],
                            pt[1],
                            pt[2]
                        ),
                    );
                }
                None => self.say(
                    Severity::Info,
                    "No point there: click a face, an edge, or a datum origin",
                ),
            }
            return;
        }
        self.selection = match p {
            Pick::Face(f, face, _) => Some((f, self.filter.picks_face().then_some(face))),
            Pick::Nothing(_) => None,
        };
        if let Some((f, face)) = self.selection {
            let name = self
                .session
                .as_ref()
                .and_then(|s| s.history().into_iter().find(|r| r.id == f))
                .map_or_else(String::new, |r| r.name);
            self.say(
                Severity::Info,
                match face {
                    Some(n) => format!("Selected face {n} of {name}"),
                    None => format!("Selected {name}"),
                },
            );
        }
        self.apply_highlight();
    }

    /// The MB3 menu in the graphics window.
    fn graphics_menu(&mut self, ctx: &egui::Context) {
        let Some(p) = self.context_menu else { return };
        let mut close = false;
        egui::Area::new(egui::Id::unique("graphics-context"))
            .fixed_pos(p)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    if let Some((f, _)) = self.selection
                        && ui.button("Information").clicked()
                    {
                        self.information(f);
                        close = true;
                    }
                    if ui.button("Fit").clicked() {
                        self.with_view(ViewPane::fit);
                        close = true;
                    }
                    if ui.button("Snap View").clicked() {
                        self.with_view(ViewPane::snap);
                        close = true;
                    }
                    if ui.button("Close menu").clicked() {
                        close = true;
                    }
                });
            });
        if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.context_menu = None;
        }
    }
}

/// The dock's tab viewer: it borrows the shell state while panes draw.
struct Tabs<'a> {
    st: &'a mut State,
    hover: Option<egui::Pos2>,
    picked: Option<Pick>,
    ok: bool,
    context: Option<egui::Pos2>,
}

impl TabViewer for Tabs<'_> {
    type Tab = Pane;

    fn id(&mut self, tab: &mut Pane) -> egui::Id {
        egui::Id::unique(("pane", *tab as u8))
    }

    fn title(&mut self, tab: &mut Pane) -> egui::WidgetText {
        match tab {
            Pane::PartNavigator => "Part Navigator".into(),
            Pane::AssemblyNavigator => "Assembly Navigator".into(),
            Pane::Roles => "Roles".into(),
            Pane::Graphics => self
                .st
                .session
                .as_ref()
                .map_or_else(
                    || "Graphics".to_owned(),
                    |s| {
                        let name = s.path().file_name().map_or_else(
                            || "part".to_owned(),
                            |n| n.to_string_lossy().into_owned(),
                        );
                        if s.is_modified() {
                            format!("{name} (Modified)")
                        } else {
                            name
                        }
                    },
                )
                .into(),
        }
    }

    fn closeable(&mut self, _tab: &mut Pane) -> bool {
        false
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Pane) {
        match tab {
            Pane::PartNavigator => self.part_navigator(ui),
            Pane::AssemblyNavigator => {
                ui.label("Assemblies arrive in Stage 1C (Planned for Stage 1).");
            }
            Pane::Roles => {
                for r in [Role::Essentials, Role::Advanced] {
                    if ui.radio(self.st.role == r, r.label()).clicked() {
                        self.st.role = r;
                    }
                }
                ui.label(
                    egui::RichText::new("Essentials shows the Home tab; Advanced shows every tab.")
                        .weak(),
                );
            }
            Pane::Graphics => {
                let text = ui.visuals().text_color();
                if let Some(v) = self.st.view.as_mut() {
                    let e = v.ui(ui, &self.st.overlay, text);
                    self.hover = e.hover;
                    self.picked = e.picked;
                    self.ok = e.ok;
                    self.context = e.context;
                } else {
                    ui.label("No graphics device: the graphics window cannot draw.");
                }
            }
        }
    }
}

impl Tabs<'_> {
    fn part_navigator(&mut self, ui: &mut egui::Ui) {
        let Some(s) = self.st.session.as_ref() else {
            ui.label("No part open.");
            return;
        };
        let rows = s.history();
        let selected = self.st.selection.map(|(f, _)| f);
        let mut click = None;
        let mut info = None;
        egui::CollapsingHeader::new("Model History")
            .default_open(true)
            .show(ui, |ui| {
                for row in &rows {
                    ui.horizontal(|ui| {
                        // Status mark: a painted dot, so no glyph font is needed.
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        let color = match row.state {
                            ObjectState::UpToDate => egui::Color32::from_rgb(40, 160, 60),
                            ObjectState::Failed => egui::Color32::from_rgb(210, 40, 40),
                            ObjectState::OutOfDate => egui::Color32::from_rgb(220, 160, 0),
                            ObjectState::Suppressed => egui::Color32::GRAY,
                        };
                        ui.painter().circle_filled(rect.center(), 4.0, color);
                        let r = ui.selectable_label(selected == Some(row.id), &row.name);
                        let r = match &row.error {
                            Some(e) => r.on_hover_text(e),
                            None => r,
                        };
                        if r.clicked() {
                            click = Some(row.id);
                        }
                        r.context_menu(|ui| {
                            if ui.button("Information").clicked() {
                                info = Some(row.id);
                                ui.close();
                            }
                        });
                    });
                }
            });
        if let Some(id) = click {
            self.st.selection = Some((id, None));
            self.st.apply_highlight();
        }
        if let Some(id) = info {
            self.st.information(id);
        }
    }
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let st = &mut self.st;
        st.frame += 1;
        st.autostart(&ctx);
        st.refresh_bodies();
        st.keys(&ctx);
        st.tick();
        egui::Panel::top("ribbon").show(ui, |ui| st.ribbon(ui));
        egui::Panel::top("top-border-bar").show(ui, |ui| st.top_border_bar(ui));
        egui::Panel::bottom("cue-status").show(ui, |ui| st.cue_and_status(ui));
        let mut tabs = Tabs {
            st,
            hover: None,
            picked: None,
            ok: false,
            context: None,
        };
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                DockArea::new(&mut self.dock)
                    .show_leaf_close_all_buttons(false)
                    .show_leaf_collapse_buttons(false)
                    .show_inside(ui, &mut tabs);
            });
        let Tabs {
            hover,
            picked,
            ok,
            context,
            ..
        } = tabs;
        let st = &mut self.st;
        if let Some(p) = picked {
            st.on_pick(p);
        }
        if ok && st.dialog.is_some() {
            st.finish_dialog(Outcome::Ok);
        }
        if let Some(p) = context {
            st.context_menu = Some(p);
        }
        st.graphics_menu(&ctx);
        st.dialogs(&ctx, hover);
        // Autosave needs a tick even when nothing moves.
        ctx.request_repaint_after(Duration::from_secs(5));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn application_switching_shortcuts_come_from_the_registry() {
        let mut r = AppRegistry::new();
        r.register(Box::new(pxp_app_modeling::Modeling::new()))
            .unwrap();
        r.register(Box::new(pxp_app_drafting::Drafting)).unwrap();
        let map = build_shortcuts(&r, &Defaults::default());
        let cmd = |k: &str| map.command(&Chord::parse(k).unwrap()).map(str::to_owned);
        assert_eq!(cmd("Ctrl+M").as_deref(), Some("app.switch.pxp.modeling"));
        assert_eq!(
            cmd("Ctrl+Shift+D").as_deref(),
            Some("app.switch.pxp.drafting")
        );
        assert!(map.conflicts.is_empty(), "{:?}", map.conflicts);
    }
}
