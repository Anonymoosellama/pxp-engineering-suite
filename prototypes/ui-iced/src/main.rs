// SPDX-License-Identifier: LGPL-2.1-or-later
//! ADR-0002 prototype: the same NX-style shell as `ui-egui`, in iced.
//!
//! Layout and data are identical (ribbon from `AppRegistry::commands()`,
//! Part Navigator, cue and status lines, Ctrl+M / Ctrl+Shift+D / Ctrl+Alt+M);
//! docking uses iced's `pane_grid` (resizable split panes; iced has no
//! tabbed, undockable docking). `PXP_PROTO_SCREENSHOT=FILE` saves a
//! screenshot after start-up and exits.

use iced::widget::{button, canvas, column, container, pane_grid, row, scrollable, text, tooltip};
use iced::{Element, Length, Subscription, Task, Theme, keyboard, mouse, window};
use pxp_appkit::{AppRegistry, Application, CommandSpec, UiContext};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Navigator,
    Graphics,
}

#[derive(Debug, Clone)]
enum Message {
    Tab(String),
    Command(String),
    Shortcut(&'static str),
    Resized(pane_grid::ResizeEvent),
    Shot(window::Screenshot),
    TakeShot,
}

struct Shell {
    registry: AppRegistry,
    ui_ctx: UiContext,
    panes: pane_grid::State<Pane>,
    ribbon_tab: String,
    cue: String,
    status: String,
    screenshot: Option<std::path::PathBuf>,
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
        let _ = registry.register(app);
    }
    registry
}

impl Shell {
    fn new() -> (Self, Task<Message>) {
        let (mut panes, nav) = pane_grid::State::new(Pane::Navigator);
        if let Some((_, split)) = panes.split(pane_grid::Axis::Vertical, nav, Pane::Graphics) {
            panes.resize(split, 0.22);
        }
        let mut shell = Self {
            registry: registry(),
            ui_ctx: UiContext::default(),
            panes,
            ribbon_tab: String::new(),
            cue: "Select a command or an object".into(),
            status: String::new(),
            screenshot: std::env::var_os("PXP_PROTO_SCREENSHOT").map(Into::into),
        };
        shell.switch("pxp.modeling");
        let task = if shell.screenshot.is_some() {
            Task::perform(async { std::thread::sleep(std::time::Duration::from_secs(2)) }, |()| Message::TakeShot)
        } else {
            Task::none()
        };
        (shell, task)
    }

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

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tab(t) => self.ribbon_tab = t,
            Message::Command(label) => self.cue = format!("{label}: select objects"),
            Message::Shortcut(keys) => {
                if let Some(c) = self.registry.by_shortcut(keys) {
                    let app = c.app.as_str().to_owned();
                    self.switch(&app);
                }
            }
            Message::Resized(e) => self.panes.resize(e.split, e.ratio),
            Message::TakeShot => return window::latest().and_then(window::screenshot).map(Message::Shot),
            Message::Shot(shot) => {
                if let Some(path) = &self.screenshot {
                    let (w, h) = (shot.size.width, shot.size.height);
                    if let Some(img) = image::RgbaImage::from_raw(w, h, shot.rgba.to_vec()) {
                        let _ = img.save(path);
                    }
                }
                return iced::exit();
            }
        }
        Task::none()
    }

    fn active_commands(&self) -> Vec<CommandSpec> {
        let active = self.registry.active_app();
        self.registry
            .commands()
            .iter()
            .filter(|c| Some(&c.app) == active.as_ref() && c.placement.tab.as_ref() == self.ribbon_tab)
            .cloned()
            .collect()
    }

    fn ribbon(&self) -> Element<'_, Message> {
        let tabs = row(self.ui_ctx.ribbon_tabs.iter().map(|t| {
            let style = if *t == self.ribbon_tab { button::primary } else { button::text };
            button(text(*t)).style(style).on_press(Message::Tab((*t).to_owned())).into()
        }))
        .spacing(4);
        let commands = self.active_commands();
        let mut groups: Vec<String> = commands.iter().map(|c| c.placement.group.to_string()).collect();
        groups.dedup();
        let groups = row(groups.into_iter().map(|g| {
            let buttons = row(commands.iter().filter(|c| c.placement.group == g.as_str()).map(|c| {
                let b = button(text(c.label.to_string()).size(13)).width(Length::Shrink).height(40);
                let b = if c.is_available() { b.on_press(Message::Command(c.label.to_string())) } else { b };
                tooltip(b, container(text(c.tooltip()).size(12)).padding(6).style(container::rounded_box), tooltip::Position::Bottom).into()
            }))
            .spacing(4);
            column![buttons, text(g).size(11)].spacing(2).into()
        }))
        .spacing(16);
        column![tabs, groups].spacing(6).padding(6).into()
    }

    fn view(&self) -> Element<'_, Message> {
        let grid = pane_grid(&self.panes, |_, pane, _| {
            let body: Element<'_, Message> = match pane {
                Pane::Navigator => scrollable(column![
                    text("Part Navigator").size(15),
                    text("Model History"),
                    text("  ✔ Datum Coordinate System (0)"),
                    text("  ✔ Block (1)"),
                    text("  ⚠ Cylinder (2)"),
                ]
                .spacing(4)
                .padding(8))
                .into(),
                Pane::Graphics => canvas(Cube).width(Length::Fill).height(Length::Fill).into(),
            };
            pane_grid::Content::new(body)
        })
        .on_resize(6, Message::Resized);
        column![
            self.ribbon(),
            container(grid).height(Length::Fill),
            text(&self.cue).size(14),
            text(&self.status).size(12),
        ]
        .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        keyboard::listen().filter_map(|event| match event {
            keyboard::Event::KeyPressed { key: keyboard::Key::Character(c), modifiers, .. } => {
                let c = c.to_lowercase();
                match (c.as_str(), modifiers.control(), modifiers.shift(), modifiers.alt()) {
                    ("m", true, false, false) => Some(Message::Shortcut("Ctrl+M")),
                    ("d", true, true, _) => Some(Message::Shortcut("Ctrl+Shift+D")),
                    ("m", true, _, true) => Some(Message::Shortcut("Ctrl+Alt+M")),
                    _ => None,
                }
            }
            _ => None,
        })
    }
}

/// A wireframe cube: the stand-in viewport, as in the egui prototype.
struct Cube;

impl canvas::Program<Message> for Cube {
    type State = ();
    fn draw(&self, _: &(), renderer: &iced::Renderer, _: &Theme, bounds: iced::Rectangle, _: mouse::Cursor) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        frame.fill_rectangle(iced::Point::ORIGIN, bounds.size(), iced::Color::from_rgb8(58, 66, 82));
        let c = frame.center();
        let s = bounds.height.min(bounds.width) * 0.18;
        let a: f32 = 0.6;
        let p = |x: f32, y: f32, z: f32| {
            let (sa, ca) = a.sin_cos();
            let (xr, zr) = (x * ca - z * sa, x * sa + z * ca);
            iced::Point::new(c.x + s * xr, c.y - s * (y * 0.9 - zr * 0.35))
        };
        let v: Vec<iced::Point> = (0..8).map(|i| p(if i & 1 == 0 { -1.0 } else { 1.0 }, if i & 2 == 0 { -1.0 } else { 1.0 }, if i & 4 == 0 { -1.0 } else { 1.0 })).collect();
        let stroke = canvas::Stroke::default().with_width(1.5).with_color(iced::Color::from_rgb8(230, 230, 235));
        for (i, j) in [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)] {
            if let (Some(&a), Some(&b)) = (v.get(i), v.get(j)) {
                frame.stroke(&canvas::Path::line(a, b), stroke);
            }
        }
        vec![frame.into_geometry()]
    }
}

fn main() -> iced::Result {
    iced::application(Shell::new, Shell::update, Shell::view)
        .title("PXP Engineering Suite — iced prototype")
        .subscription(Shell::subscription)
        .window_size((1600.0, 900.0))
        .theme(|_: &Shell| Theme::Light)
        .run()
}
