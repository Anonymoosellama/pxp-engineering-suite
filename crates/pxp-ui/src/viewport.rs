// SPDX-License-Identifier: LGPL-2.1-or-later
//! The graphics window: `pxp-render`'s offscreen viewport shown as an egui
//! image, driven by the NX mouse grammar, with overlays painted on top.
//!
//! ```text
//! egui events ─► MouseGrammar ─► ViewAction ─► Camera (orbit/pan/zoom)  ─► dirty ─► Viewport::render ─► texture ─► painter.image
//!                                            └► Select ─► Viewport::pick ─► (feature, face) ─► the shell's selection
//!                                            └► Ok (MB2 click) / Context (MB3 click) ─► the shell
//! overlays (egui painter): view triad (bottom-left), WCS triad, datum CSYS axes
//! ```
//!
//! The viewport shares eframe's wgpu device, so its colour texture is
//! registered with egui's renderer once and updated in place on resize.
//! Frames are rendered only when something changed (camera, bodies,
//! display settings, size), so an idle window costs nothing.

use eframe::egui;
use eframe::egui_wgpu;
use pxp_app_modeling::session::DisplayBody;
use pxp_doc::ObjectId;
use pxp_render::camera::V3;
use pxp_render::{
    Button, Camera, DisplaySettings, GpuContext, MeshInput, Modifiers, MouseGrammar, Scene,
    StandardView, ViewAction, Viewport,
};
use std::sync::Arc;

/// What an MB1 click hit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pick {
    /// A face of a feature's body: feature, face index, and where.
    Face(ObjectId, u32, Clicked),
    /// The background (clears the selection).
    Nothing(Clicked),
}

/// Where a click landed, for snapping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clicked {
    /// Viewport pixel of the click.
    pub px: [f64; 2],
    /// Body index in the scene, if a body was hit.
    pub body: Option<usize>,
    /// The point on the face under the click (mm).
    pub point: Option<V3>,
}

/// What happened in the graphics window this frame.
#[derive(Debug, Clone, Default)]
pub struct PaneEvents {
    /// MB1 click.
    pub picked: Option<Pick>,
    /// MB2 click: OK in the active dialog.
    pub ok: bool,
    /// MB3 click at a screen position: context menu.
    pub context: Option<egui::Pos2>,
    /// The pointer is over the graphics window here.
    pub hover: Option<egui::Pos2>,
}

/// Model-space things painted over the image.
#[derive(Debug, Clone, Default)]
pub struct Overlay {
    /// Datum CSYS origins and axes (origin, x, y, z unit vectors).
    pub datums: Vec<(V3, [V3; 3])>,
    /// Work coordinate system origin and axes.
    pub wcs: Option<(V3, [V3; 3])>,
}

/// The graphics window's state.
pub struct ViewPane {
    gpu: GpuContext,
    renderer: Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>,
    viewport: Viewport,
    scene: Scene,
    /// The camera (the shell drives standard views and named views).
    pub camera: Camera,
    mouse: MouseGrammar,
    texture: Option<egui::TextureId>,
    size: [u32; 2],
    /// Display settings (edges, silhouettes, section, theme, highlight).
    pub settings: DisplaySettings,
    features: Vec<ObjectId>,
    dirty: bool,
    fitted: bool,
}

/// Body colour: NX-like neutral grey-blue.
const BODY_COLOR: [f32; 4] = [0.55, 0.60, 0.66, 1.0];

impl ViewPane {
    /// Builds the pane on eframe's device.
    #[must_use]
    pub fn new(rs: &egui_wgpu::RenderState, settings: DisplaySettings) -> Self {
        let gpu =
            GpuContext::from_parts(rs.device.clone(), rs.queue.clone(), rs.adapter.get_info());
        let viewport = Viewport::new(&gpu);
        Self {
            gpu,
            renderer: Arc::clone(&rs.renderer),
            viewport,
            scene: Scene::new(),
            camera: Camera::default(),
            mouse: MouseGrammar::default(),
            texture: None,
            size: [0, 0],
            settings,
            features: Vec::new(),
            dirty: true,
            fitted: false,
        }
    }

    /// The software-rasteriser warning, if the device is one.
    #[must_use]
    pub fn software_warning(&self) -> Option<String> {
        self.gpu.software_warning()
    }

    /// Adapter description for Help > About and the status line.
    #[must_use]
    pub fn adapter(&self) -> String {
        format!("{} ({:?})", self.gpu.adapter.name, self.gpu.adapter.backend)
    }

    /// Replaces the displayed bodies. The first time anything appears, the
    /// view fits it.
    pub fn set_bodies(&mut self, bodies: &[DisplayBody]) {
        self.scene = Scene::new();
        self.features.clear();
        for b in bodies {
            let m = &b.mesh;
            let input = MeshInput {
                positions: m.positions.clone(),
                normals: m.normals.clone(),
                triangles: m.triangles.clone(),
                tri_face: m.tri_face.clone(),
                edges: m.edges.clone(),
            };
            self.scene.add(&self.gpu, &input, BODY_COLOR);
            self.features.push(b.feature);
        }
        if !self.fitted && !self.scene.is_empty() {
            self.fit();
            self.fitted = true;
        }
        self.dirty = true;
    }

    /// Asks for a new frame (after changing [`ViewPane::settings`]).
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// Index of a feature's body, for highlighting.
    #[must_use]
    pub fn body_of(&self, feature: ObjectId) -> Option<usize> {
        self.features.iter().position(|f| *f == feature)
    }

    /// Fit (Ctrl+F).
    pub fn fit(&mut self) {
        if let Some((lo, hi)) = self.scene.bounds() {
            let aspect = if self.size[1] > 0 {
                f64::from(self.size[0]) / f64::from(self.size[1])
            } else {
                1.6
            };
            self.camera.fit(lo, hi, aspect);
            self.dirty = true;
        }
    }

    /// Shows a standard view and fits.
    pub fn standard_view(&mut self, v: StandardView) {
        self.camera.set_view(v);
        self.fit();
    }

    /// F8: snap to the nearest standard view.
    pub fn snap(&mut self) {
        let v = self.camera.nearest_standard_view();
        self.camera.set_view(v);
        self.dirty = true;
    }

    /// Restores a saved camera (named views).
    pub fn set_camera(&mut self, c: Camera) {
        self.camera = c;
        self.dirty = true;
    }

    /// Snaps a click for a dialog's "Specify Point" step (see
    /// `pxp_render::snap`): the clicked body's edges and `points` (datum
    /// origins) are the candidates.
    #[must_use]
    pub fn snap_point(
        &self,
        c: &Clicked,
        points: &[V3],
        set: pxp_render::snap::SnapSet,
    ) -> Option<(V3, pxp_render::snap::SnapKind)> {
        let size = [f64::from(self.size[0]), f64::from(self.size[1])];
        let edges = c.body.map_or(&[][..], |b| self.scene.edges(b));
        pxp_render::snap::snap(
            &self.camera,
            size,
            self.scene.radius(),
            c.px,
            c.point,
            edges,
            points,
            set,
        )
    }

    /// An MB1 click at the centre of the graphics window, as the mouse
    /// would make it (scripted start-up and the verification packet).
    pub fn click_center(&mut self) -> Option<Pick> {
        if self.size[0] == 0 {
            return None;
        }
        let at = [f64::from(self.size[0] / 2), f64::from(self.size[1] / 2)];
        let rect = egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(
                f32::from(u16::try_from(self.size[0]).unwrap_or(u16::MAX)),
                f32::from(u16::try_from(self.size[1]).unwrap_or(u16::MAX)),
            ),
        );
        let mut events = PaneEvents::default();
        self.apply(
            ViewAction::Select { at, extend: false },
            f64::from(self.size[1]),
            rect,
            1.0,
            &mut events,
        );
        events.picked
    }

    /// Picks at the centre of the last frame (scripted start-up and tests).
    pub fn pick_center(&mut self) -> Option<(ObjectId, u32)> {
        if self.size[0] == 0 {
            return None;
        }
        if self.dirty {
            self.viewport.render(
                &self.gpu,
                &self.scene,
                &self.camera,
                &self.settings,
                self.size,
            );
            self.register_texture();
            self.dirty = false;
        }
        let at = [self.size[0] / 2, self.size[1] / 2];
        let hit = self
            .viewport
            .pick(&self.gpu, &self.scene, &self.camera, &self.settings, at)
            .ok()
            .flatten()?;
        self.features
            .get(hit.picked.body)
            .map(|f| (*f, hit.picked.face))
    }

    /// Draws the pane and handles its input.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        overlay: &Overlay,
        text_color: egui::Color32,
    ) -> PaneEvents {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let ppp = ui.ctx().pixels_per_point();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let size = [
            (rect.width() * ppp).round().max(1.0) as u32,
            (rect.height() * ppp).round().max(1.0) as u32,
        ];
        let mut events = PaneEvents::default();
        self.input(ui, &response, rect, ppp, size, &mut events);
        if size != self.size || self.dirty {
            self.size = size;
            self.viewport
                .render(&self.gpu, &self.scene, &self.camera, &self.settings, size);
            self.register_texture();
            self.dirty = false;
        }
        let painter = ui.painter_at(rect);
        if let Some(tex) = self.texture {
            painter.image(
                tex,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        self.paint_overlay(&painter, rect, ppp, overlay, text_color);
        events.hover = response.hover_pos();
        events
    }

    fn register_texture(&mut self) {
        let Some(view) = self.viewport.color_view() else {
            return;
        };
        let mut r = self.renderer.write();
        match self.texture {
            Some(id) => r.update_egui_texture_from_wgpu_texture(
                &self.gpu.device,
                view,
                pxp_render::wgpu::FilterMode::Linear,
                id,
            ),
            None => {
                self.texture = Some(r.register_native_texture(
                    &self.gpu.device,
                    view,
                    pxp_render::wgpu::FilterMode::Linear,
                ));
            }
        }
    }

    fn input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        ppp: f32,
        size: [u32; 2],
        events: &mut PaneEvents,
    ) {
        let to_px = |p: egui::Pos2| {
            [
                f64::from((p.x - rect.min.x) * ppp),
                f64::from((p.y - rect.min.y) * ppp),
            ]
        };
        let raw: Vec<egui::Event> = ui.input(|i| i.events.clone());
        let hovered = response.hovered();
        let h = f64::from(size[1]);
        for e in raw {
            let action = match e {
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                } => {
                    let b = match button {
                        egui::PointerButton::Primary => Button::Mb1,
                        egui::PointerButton::Middle => Button::Mb2,
                        egui::PointerButton::Secondary => Button::Mb3,
                        _ => continue,
                    };
                    let m = Modifiers {
                        shift: modifiers.shift,
                        ctrl: modifiers.command,
                    };
                    if pressed {
                        // Gestures start only inside the graphics window.
                        if rect.contains(pos) {
                            self.mouse.press(b, to_px(pos));
                        }
                        ViewAction::None
                    } else {
                        self.mouse.release(b, to_px(pos), m)
                    }
                }
                egui::Event::PointerMoved(pos) => {
                    let m = ui.input(|i| Modifiers {
                        shift: i.modifiers.shift,
                        ctrl: i.modifiers.command,
                    });
                    self.mouse.motion(to_px(pos), m)
                }
                _ => ViewAction::None,
            };
            self.apply(action, h, rect, ppp, events);
        }
        if hovered {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let at = ui
                    .input(|i| i.pointer.hover_pos())
                    .map_or([f64::from(size[0]) / 2.0, h / 2.0], to_px);
                let a = self.mouse.wheel(f64::from(scroll) / 40.0, at);
                self.apply(a, h, rect, ppp, events);
            }
        }
    }

    fn apply(
        &mut self,
        action: ViewAction,
        h: f64,
        rect: egui::Rect,
        ppp: f32,
        events: &mut PaneEvents,
    ) {
        let centre = [f64::from(rect.width() * ppp) / 2.0, h / 2.0];
        match action {
            ViewAction::None => {}
            ViewAction::Rotate { dx, dy } => {
                let pivot = self.camera.target;
                self.camera.orbit(dx, dy, h, pivot);
                self.dirty = true;
            }
            ViewAction::Pan { dx, dy } => {
                self.camera.pan(dx, dy, h);
                self.dirty = true;
            }
            ViewAction::Zoom { factor, at } => {
                self.camera
                    .zoom_at(factor, [at[0] - centre[0], at[1] - centre[1]], h);
                self.dirty = true;
            }
            ViewAction::Select { at, .. } => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let px = [at[0].max(0.0) as u32, at[1].max(0.0) as u32];
                let hit = self
                    .viewport
                    .pick(&self.gpu, &self.scene, &self.camera, &self.settings, px)
                    .ok()
                    .flatten();
                let clicked = Clicked {
                    px: at,
                    body: hit.map(|h| h.picked.body),
                    point: hit.map(|h| h.point),
                };
                events.picked = Some(
                    hit.and_then(|h| {
                        self.features
                            .get(h.picked.body)
                            .map(|f| Pick::Face(*f, h.picked.face, clicked))
                    })
                    .unwrap_or(Pick::Nothing(clicked)),
                );
            }
            ViewAction::Ok => events.ok = true,
            ViewAction::Context { at } => {
                #[allow(clippy::cast_possible_truncation)]
                let p = egui::pos2(
                    rect.min.x + at[0] as f32 / ppp,
                    rect.min.y + at[1] as f32 / ppp,
                );
                events.context = Some(p);
            }
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    fn paint_overlay(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        ppp: f32,
        overlay: &Overlay,
        text: egui::Color32,
    ) {
        let axis_colors = [
            egui::Color32::from_rgb(220, 50, 47),
            egui::Color32::from_rgb(40, 160, 60),
            egui::Color32::from_rgb(38, 110, 210),
        ];
        let font = egui::FontId::proportional(12.0);
        // View triad, bottom-left: model X, Y, Z as they point on screen.
        let origin = egui::pos2(rect.min.x + 44.0, rect.max.y - 44.0);
        for ((axis, color), name) in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
            .iter()
            .zip(axis_colors)
            .zip(["X", "Y", "Z"])
        {
            let [sx, sy] = self.camera.screen_direction(*axis);
            let tip = origin + egui::vec2(sx as f32, -sy as f32) * 30.0;
            painter.line_segment([origin, tip], egui::Stroke::new(2.0, color));
            painter.text(
                tip + egui::vec2(sx as f32, -sy as f32) * 8.0,
                egui::Align2::CENTER_CENTER,
                name,
                font.clone(),
                color,
            );
        }
        let size = [
            f64::from(rect.width() * ppp),
            f64::from(rect.height() * ppp),
        ];
        let radius = self.scene.radius();
        let to_screen = |p: V3| -> Option<egui::Pos2> {
            let [x, y] = self.camera.project(p, size, radius)?;
            Some(egui::pos2(
                rect.min.x + x as f32 / ppp,
                rect.min.y + y as f32 / ppp,
            ))
        };
        // Datum CSYS: three coloured axes, a fixed screen length.
        for (o, axes) in &overlay.datums {
            let Some(p0) = to_screen(*o) else { continue };
            for (a, color) in axes.iter().zip(axis_colors) {
                let [sx, sy] = self.camera.screen_direction(*a);
                painter.line_segment(
                    [p0, p0 + egui::vec2(sx as f32, -sy as f32) * 50.0],
                    egui::Stroke::new(1.5, color),
                );
            }
        }
        // WCS: labelled XC / YC / ZC.
        if let Some((o, axes)) = &overlay.wcs
            && let Some(p0) = to_screen(*o)
        {
            for (a, name) in axes.iter().zip(["XC", "YC", "ZC"]) {
                let [sx, sy] = self.camera.screen_direction(*a);
                let tip = p0 + egui::vec2(sx as f32, -sy as f32) * 28.0;
                painter.line_segment([p0, tip], egui::Stroke::new(1.0, text));
                painter.text(
                    tip + egui::vec2(sx as f32, -sy as f32) * 9.0,
                    egui::Align2::CENTER_CENTER,
                    name,
                    font.clone(),
                    text,
                );
            }
        }
    }
}
