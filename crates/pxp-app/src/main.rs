// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-app: the desktop application binary (`pxp`)
//!
//! Process start-up, nothing more: install the local crash reporter, read
//! the customer defaults, choose and host the geometry kernel (the shell is
//! the only layer allowed to name a backend), register the applications,
//! and hand control to `pxp-ui`'s shell. Headless helpers live here too:
//! `pxp bench-orbit` runs the viewport benchmark without a window.
//!
//! ```text
//! pxp [FILE.pxp] [flags]      open the shell (optionally with a part)
//! pxp bench-orbit [...]       5-million-triangle orbit benchmark, headless
//! pxp --help | --version
//! ```
//!
//! ## Must never
//!
//! - Contain domain logic: it wires crates together.
#![forbid(unsafe_code)]

use pxp_appkit::{AppRegistry, Application};
use pxp_kernel::KernelHost;
use pxp_render::StandardView;
use pxp_ui::{Autostart, ShellConfig};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

const HELP: &str = "\
pxp — PXP Engineering Suite desktop application (pre-alpha, Stage 0)

USAGE
  pxp [FILE.pxp] [FLAGS]
  pxp bench-orbit [--triangles N] [--frames N] [--size WxH]
  pxp --help | --version

FLAGS (scripted start-up, used for screenshots and the verification packet)
  --kernel native|occt     geometry backend (default native: SolidCraft in-process;
                           occt starts the OCCT worker, see PXP_OCCT_PYTHON)
  --new FILE               create a millimetre part and open it
  --block \"OX OY OZ LX LY LZ\"   add a block (origin and edge lengths; repeatable)
  --save                   save the part after the blocks are added
  --dialog block|cylinder|cone|sphere   open a feature dialog
  --select-center          click the centre of the graphics window
  --view trimetric|isometric|top|front|right|back|bottom|left
  --section-x X            section plane at x = X mm
  --screenshot FILE.png    save a screenshot of the window, then exit

MOUSE (NX): MB2 rotate, MB2+MB3 or Shift+MB2 pan, MB1+MB2 or Ctrl+MB2 zoom,
            wheel zoom at the cursor, MB1 select, MB2 click = OK, MB3 menu.
KEYS: Home trimetric, End isometric, F8 snap view, Ctrl+F fit, Ctrl+Z/Y undo/redo,
      Ctrl+N/O/S new/open/save, Ctrl+M Modeling, Ctrl+Shift+D Drafting.";

fn main() -> ExitCode {
    pxp_diag::crash::install(pxp_diag::crash::default_dir(), "pxp-app");
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--help" | "-h") => {
            println!("{HELP}");
            ExitCode::SUCCESS
        }
        Some("--version" | "-V") => {
            println!(
                "pxp {} ({}-{})",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            ExitCode::SUCCESS
        }
        Some("bench-orbit") => bench(args.get(1..).unwrap_or(&[])),
        _ => match shell(&args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("pxp: {e}");
                ExitCode::FAILURE
            }
        },
    }
}

/// `--flag value` lookup helper over the argument list.
struct Args<'a> {
    list: &'a [String],
    used: Vec<bool>,
}

impl<'a> Args<'a> {
    fn new(list: &'a [String]) -> Self {
        Self {
            list,
            used: vec![false; list.len()],
        }
    }

    fn value(&mut self, flag: &str) -> Option<&'a str> {
        let i = self.list.iter().position(|a| a == flag)?;
        let v = self.list.get(i + 1)?;
        self.used.iter_mut().skip(i).take(2).for_each(|u| *u = true);
        Some(v.as_str())
    }

    fn values(&mut self, flag: &str) -> Vec<&'a str> {
        let mut out = Vec::new();
        let mut i = 0;
        while let Some(a) = self.list.get(i) {
            if a == flag
                && let Some(v) = self.list.get(i + 1)
            {
                out.push(v.as_str());
                self.used.iter_mut().skip(i).take(2).for_each(|u| *u = true);
                i += 2;
            } else {
                i += 1;
            }
        }
        out
    }

    fn flag(&mut self, flag: &str) -> bool {
        match self.list.iter().position(|a| a == flag) {
            Some(i) => {
                if let Some(u) = self.used.get_mut(i) {
                    *u = true;
                }
                true
            }
            None => false,
        }
    }

    /// Arguments nobody consumed: a lone file path is allowed, anything
    /// else is an error (a typo must not be ignored).
    fn rest(&self) -> Vec<&'a str> {
        self.list
            .iter()
            .zip(&self.used)
            .filter(|(_, u)| !**u)
            .map(|(a, _)| a.as_str())
            .collect()
    }
}

fn kernel(name: &str) -> Result<(Arc<KernelHost>, String), String> {
    match name {
        "native" => Ok((
            Arc::new(KernelHost::in_process(Box::new(
                pxp_kernel_native::NativeKernel::new(),
            ))),
            "SolidCraft kernel (in-process)".to_owned(),
        )),
        "occt" => {
            let k = pxp_kernel_occt::OcctKernel::new(pxp_kernel_occt::OcctConfig::from_env());
            k.connect()
                .map_err(|e| format!("cannot start the OCCT worker: {e}"))?;
            Ok((
                Arc::new(KernelHost::in_process(Box::new(k))),
                "OCCT 7.8.1 (worker process)".to_owned(),
            ))
        }
        other => Err(format!("--kernel must be native or occt, got {other}")),
    }
}

fn registry(kernel: &Arc<KernelHost>) -> Result<AppRegistry, String> {
    let mut registry = AppRegistry::new();
    let apps: [Box<dyn Application>; 6] = [
        Box::new(pxp_app_modeling::Modeling::with_kernel(Arc::clone(kernel))),
        Box::new(pxp_app_drafting::Drafting),
        Box::new(pxp_app_manufacturing::Manufacturing),
        Box::new(pxp_app_postgen::PostGeneration),
        Box::new(pxp_app_machine_kit::MachineKitBuilder),
        Box::new(pxp_app_cae::Cae),
    ];
    for app in apps {
        registry
            .register(app)
            .map_err(|e| format!("application registry: {e}"))?;
    }
    Ok(registry)
}

fn view(name: &str) -> Result<StandardView, String> {
    StandardView::ALL
        .into_iter()
        .find(|v| v.label().eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("unknown view '{name}'"))
}

fn shell(list: &[String]) -> Result<(), String> {
    let mut a = Args::new(list);
    let (kernel, kernel_label) = kernel(a.value("--kernel").unwrap_or("native"))?;
    let registry = registry(&kernel)?;
    let mut autostart = Autostart {
        new_part: a.value("--new").map(PathBuf::from),
        ..Autostart::default()
    };
    for b in a.values("--block") {
        let parts: Vec<String> = b.split_whitespace().map(str::to_owned).collect();
        let six: [String; 6] = parts
            .try_into()
            .map_err(|_| format!("--block needs six values, got '{b}'"))?;
        autostart.blocks.push(six);
    }
    autostart.dialog = match a.value("--dialog") {
        None => None,
        Some("block") => Some(pxp_app_modeling::session::Primitive::Block),
        Some("cylinder") => Some(pxp_app_modeling::session::Primitive::Cylinder),
        Some("cone") => Some(pxp_app_modeling::session::Primitive::Cone),
        Some("sphere") => Some(pxp_app_modeling::session::Primitive::Sphere),
        Some(other) => return Err(format!("--dialog: unknown dialog '{other}'")),
    };
    autostart.select_center = a.flag("--select-center");
    autostart.save = a.flag("--save");
    autostart.view = a.value("--view").map(view).transpose()?;
    autostart.section_x = a
        .value("--section-x")
        .map(|v| v.parse::<f64>().map_err(|e| format!("--section-x: {e}")))
        .transpose()?;
    autostart.screenshot = a.value("--screenshot").map(PathBuf::from);
    let rest = a.rest();
    let open = match rest.as_slice() {
        [] => None,
        [f] if !f.starts_with('-') => Some(PathBuf::from(f)),
        other => return Err(format!("unexpected arguments {other:?} (pxp --help)")),
    };
    let (defaults, warnings) = pxp_ui::defaults::Defaults::load();
    let cfg = ShellConfig {
        registry,
        kernel,
        kernel_label,
        defaults,
        warnings,
        open,
        autostart,
    };
    let options = pxp_ui::eframe::NativeOptions {
        renderer: pxp_ui::eframe::Renderer::Wgpu,
        viewport: pxp_ui::eframe::egui::ViewportBuilder::default()
            .with_inner_size([1600.0, 900.0])
            .with_title("PXP Engineering Suite"),
        ..Default::default()
    };
    pxp_ui::eframe::run_native(
        "pxp",
        options,
        Box::new(move |cc| Ok(Box::new(pxp_ui::Shell::new(cc, cfg)))),
    )
    .map_err(|e| e.to_string())
}

fn bench(list: &[String]) -> ExitCode {
    let mut a = Args::new(list);
    let parse = |v: Option<&str>, default: u64| v.map_or(Ok(default), str::parse::<u64>);
    let (Ok(triangles), Ok(frames)) = (
        parse(a.value("--triangles"), 5_000_000),
        parse(a.value("--frames"), 60),
    ) else {
        eprintln!("pxp bench-orbit: --triangles and --frames take whole numbers");
        return ExitCode::FAILURE;
    };
    let size = a.value("--size").unwrap_or("1920x1080");
    let Some((w, h)) = size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
    else {
        eprintln!("pxp bench-orbit: --size takes WIDTHxHEIGHT");
        return ExitCode::FAILURE;
    };
    if let Some(extra) = a.rest().first() {
        eprintln!("pxp bench-orbit: unexpected argument {extra}");
        return ExitCode::FAILURE;
    }
    let ctx = match pxp_render::GpuContext::headless() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("pxp bench-orbit: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(w) = ctx.software_warning() {
        eprintln!("warning: {w}");
    }
    match pxp_render::bench::orbit(
        &ctx,
        usize::try_from(triangles).unwrap_or(usize::MAX),
        u32::try_from(frames).unwrap_or(u32::MAX),
        [w, h],
    ) {
        Ok(r) => {
            println!("{r}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("pxp bench-orbit: {e}");
            ExitCode::FAILURE
        }
    }
}
