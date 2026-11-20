// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-appkit: applications, commands and the registry
//!
//! ## Responsibility
//!
//! - The [`Application`] trait from the master prompt: an NX-style
//!   application (Modeling, Drafting, Manufacturing, Post Processor
//!   Generation, Machine Kit Builder, CAE) is an *editor over one shared
//!   document*, never an owner of separate data.
//! - [`CommandSpec`]: one command, with its ribbon placement, default
//!   shortcut and status. The UI builds every ribbon from these; the CLI and
//!   MCP server expose the same commands headless.
//! - [`AppRegistry`]: registration with validation, application switching,
//!   the Command Finder, shortcut lookup, change fan-out to applications,
//!   and recompute dispatch to the application that owns each kind.
//!
//! ## Honest status
//!
//! Commands that are not implemented carry [`CommandStatus::Planned`]; the
//! UI shows them greyed with the tooltip "Planned for Stage N." and the CLI
//! refuses to run them. Nothing is wired to a stub that silently does
//! nothing.
//!
//! ## Contract additions (ADR-0006)
//!
//! Beyond the master prompt's trait, [`Application`] has
//! [`Application::update_policy`] and [`Application::recompute`], because
//! the recompute scheduler needs to know who regenerates each kind and when.
//! Their defaults are deliberately safe: [`UpdatePolicy::Manual`] (never
//! auto-run) and a refusal.
//!
//! ## Data flow
//!
//! ```text
//! app crates ──register──► AppRegistry ──commands()──► ribbon / CLI / MCP
//!            ◄─on_upstream_change(ChangeSet)── Document change bus
//!            ◄─recompute(kind)── Document::recompute(&registry)
//! ```
//!
//! ## Invariants
//!
//! - Application IDs, command IDs and owned kinds are unique across the
//!   registry and syntactically valid.
//! - Switching application never touches the document.
//!
//! ## Must never
//!
//! - Let one application depend on another (they share domain libraries).
//! - Copy Siemens help text into descriptions (all text here is original).
#![forbid(unsafe_code)]

pub use pxp_doc::{AppId, ChangeSet, ObjectKind, RecomputeOutput, UpdatePolicy};
use pxp_doc::{Object, ObjectId, RecomputeHandler, Snapshot};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Text that is usually a compile-time literal but may be supplied at run
/// time (WASM plugins, user-defined roles).
pub type Text = Cow<'static, str>;

/// Where a command sits in its application's ribbon.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RibbonPlacement {
    /// Ribbon tab (`"Home"`, `"Curve"`, `"Application"`).
    pub tab: Text,
    /// Group within the tab (`"Feature"`, `"Synchronous Modeling"`).
    pub group: Text,
}

/// Whether a command can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandStatus {
    /// Implemented and runnable.
    Available,
    /// Not implemented yet; scheduled for this stage of the master prompt.
    Planned {
        /// Stage number (0-5).
        stage: u8,
    },
}

/// One command contributed by an application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    /// Stable command ID (`"modeling.extrude"`). Journals and customised
    /// ribbons refer to it: never change it after release.
    pub id: Text,
    /// Label shown in the ribbon (`"Extrude"`).
    pub label: Text,
    /// Contributing application.
    pub app: AppId,
    /// Ribbon placement.
    pub placement: RibbonPlacement,
    /// Default keyboard shortcut (remappable), e.g. `"X"` or `"Ctrl+J"`.
    pub shortcut: Option<Text>,
    /// One-line description in PXP's own words (tooltip when available).
    pub description: Text,
    /// Implementation status.
    pub status: CommandStatus,
}

impl CommandSpec {
    /// True if the command can run.
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.status == CommandStatus::Available
    }

    /// Tooltip text: the description when available, otherwise the master
    /// prompt's exact wording, "Planned for Stage N."
    #[must_use]
    pub fn tooltip(&self) -> String {
        match self.status {
            CommandStatus::Available => self.description.to_string(),
            CommandStatus::Planned { stage } => format!("Planned for Stage {stage}."),
        }
    }
}

/// Lets an application request extra out-of-date marks while reacting to a
/// change (for example: post output becomes stale when a toolpath changes).
/// The shell applies the marks in a follow-up transaction.
#[derive(Debug, Default)]
pub struct ChangeCtx {
    marks: BTreeSet<ObjectId>,
}

impl ChangeCtx {
    /// Requests that `id` be marked out of date.
    pub fn mark_out_of_date(&mut self, id: ObjectId) {
        self.marks.insert(id);
    }

    /// Requested marks, in ID order.
    #[must_use]
    pub fn marks(&self) -> &BTreeSet<ObjectId> {
        &self.marks
    }
}

/// A toolkit-neutral description of what the shell should show for the
/// active application. The UI crate renders it with whatever toolkit
/// ADR-0002 selects.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UiContext {
    /// Currently active application.
    pub active_app: Option<AppId>,
    /// Ribbon tabs to show, left to right.
    pub ribbon_tabs: Vec<&'static str>,
    /// Navigators to show in the resource bar.
    pub navigators: Vec<&'static str>,
}

/// An NX-style application (Modeling, Drafting, Manufacturing, CAE...).
///
/// Applications are editors over ONE shared document; none owns model data
/// exclusively. Switching application in an open part therefore never copies
/// or converts data: it only swaps the active ribbon tabs, navigators and
/// commands. This is what lets a Drafting dimension or a CAM operation stay
/// live against the Modeling geometry it references.
pub trait Application: Send + Sync {
    /// Stable ID persisted in `.pxp` files, e.g. `"pxp.modeling"`. Never
    /// change it after release: saved files refer to it.
    fn id(&self) -> AppId;

    /// Name shown on the Application tab (`"Modeling"`).
    fn name(&self) -> &'static str;

    /// The stage in which this application's first real commands arrive.
    fn planned_stage(&self) -> u8;

    /// Object kinds whose schema this application owns (Drafting owns
    /// sheets, views, dimensions...).
    fn owned_kinds(&self) -> &[ObjectKind];

    /// Commands contributed to the global registry. The UI builds ribbons
    /// from these; the CLI and MCP server expose the same commands headless.
    fn commands(&self) -> Vec<CommandSpec>;

    /// Called when upstream objects this application depends on change.
    /// Must only MARK dependants out of date (through `ctx`); the recompute
    /// scheduler decides when and on which threads to regenerate them. The
    /// document already marks every transitive dependant out of date, so
    /// most applications only refresh their navigators here.
    fn on_upstream_change(&self, ctx: &mut ChangeCtx, change: &ChangeSet);

    /// Enters this application inside an open part.
    fn activate(&self, ui: &mut UiContext);

    /// Leaves this application.
    fn deactivate(&self, ui: &mut UiContext);

    /// How objects of `kind` react to stale inputs. Default: manual, so a
    /// kind is never regenerated automatically before its application
    /// implements regeneration.
    fn update_policy(&self, _kind: &ObjectKind) -> UpdatePolicy {
        UpdatePolicy::Manual
    }

    /// Regenerates one object of a kind this application owns, from a
    /// snapshot in which all of its inputs are up to date.
    ///
    /// # Errors
    /// A human-readable reason. The default refuses: an application that has
    /// not implemented regeneration says so instead of pretending.
    fn recompute(&self, _inputs: &Snapshot, object: &Object) -> Result<RecomputeOutput, String> {
        Err(format!(
            "{} cannot regenerate '{}' yet (planned for Stage {})",
            self.name(),
            object.kind,
            self.planned_stage()
        ))
    }
}

/// Registration and switching errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// An application ID is malformed or already registered.
    BadAppId(String),
    /// A command ID is used twice.
    DuplicateCommand(String),
    /// A kind is malformed or owned by two applications.
    BadKind(String),
    /// No such application.
    UnknownApp(String),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadAppId(a) => {
                write!(f, "application id '{a}' is malformed or already registered")
            }
            Self::DuplicateCommand(c) => write!(f, "command id '{c}' is registered twice"),
            Self::BadKind(k) => write!(f, "object kind '{k}' is malformed or owned twice"),
            Self::UnknownApp(a) => write!(f, "no application '{a}' is registered"),
        }
    }
}

impl std::error::Error for RegistryError {}

/// Default application-switch shortcuts from the master prompt.
const SWITCH_SHORTCUTS: [(&str, &str); 3] = [
    ("pxp.modeling", "Ctrl+M"),
    ("pxp.drafting", "Ctrl+Shift+D"),
    ("pxp.manufacturing", "Ctrl+Alt+M"),
];

/// All registered applications and their commands.
#[derive(Default)]
pub struct AppRegistry {
    apps: Vec<Box<dyn Application>>,
    owners: BTreeMap<ObjectKind, usize>,
    commands: Vec<CommandSpec>,
    active: Option<usize>,
}

impl fmt::Debug for AppRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppRegistry")
            .field(
                "apps",
                &self.apps.iter().map(|a| a.id()).collect::<Vec<_>>(),
            )
            .field("commands", &self.commands.len())
            .field("active", &self.active_app())
            .finish_non_exhaustive()
    }
}

impl AppRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an application and its commands, and adds an
    /// "switch to this application" command on the Application tab.
    ///
    /// # Errors
    /// [`RegistryError`] if the ID, a command ID or an owned kind is invalid
    /// or already taken. On error nothing is registered.
    pub fn register(&mut self, app: Box<dyn Application>) -> Result<(), RegistryError> {
        let id = app.id();
        if !id.is_valid() || self.apps.iter().any(|a| a.id() == id) {
            return Err(RegistryError::BadAppId(id.to_string()));
        }
        let mut new_commands = app.commands();
        new_commands.push(switch_command(&id, app.name()));
        let mut seen: BTreeSet<&str> = self.commands.iter().map(|c| c.id.as_ref()).collect();
        for c in &new_commands {
            if !seen.insert(c.id.as_ref()) {
                return Err(RegistryError::DuplicateCommand(c.id.to_string()));
            }
        }
        for kind in app.owned_kinds() {
            if !kind.is_valid() || self.owners.contains_key(kind) {
                return Err(RegistryError::BadKind(kind.to_string()));
            }
        }
        let index = self.apps.len();
        for kind in app.owned_kinds() {
            self.owners.insert(kind.clone(), index);
        }
        self.commands.extend(new_commands);
        self.apps.push(app);
        Ok(())
    }

    /// Registered applications, in registration order (the Application tab
    /// order).
    pub fn apps(&self) -> impl Iterator<Item = &dyn Application> {
        self.apps.iter().map(AsRef::as_ref)
    }

    /// Every command of every application.
    #[must_use]
    pub fn commands(&self) -> &[CommandSpec] {
        &self.commands
    }

    /// Looks up a command by ID.
    #[must_use]
    pub fn command(&self, id: &str) -> Option<&CommandSpec> {
        self.commands.iter().find(|c| c.id == id)
    }

    /// Command Finder: case-insensitive match on label, ID or description.
    /// Results include planned commands (so users learn where they will
    /// live) with available ones first.
    #[must_use]
    pub fn find(&self, query: &str) -> Vec<&CommandSpec> {
        let q = query.to_lowercase();
        let mut hits: Vec<&CommandSpec> = self
            .commands
            .iter()
            .filter(|c| {
                c.label.to_lowercase().contains(&q)
                    || c.id.contains(q.as_str())
                    || c.description.to_lowercase().contains(&q)
            })
            .collect();
        hits.sort_by_key(|c| !c.is_available());
        hits
    }

    /// The command bound to a shortcut by default (`"Ctrl+M"`).
    #[must_use]
    pub fn by_shortcut(&self, keys: &str) -> Option<&CommandSpec> {
        self.commands
            .iter()
            .find(|c| c.shortcut.as_deref() == Some(keys))
    }

    /// The application that owns `kind`.
    #[must_use]
    pub fn owner_of(&self, kind: &ObjectKind) -> Option<&dyn Application> {
        self.owners
            .get(kind)
            .and_then(|&i| self.apps.get(i))
            .map(AsRef::as_ref)
    }

    /// The active application.
    #[must_use]
    pub fn active_app(&self) -> Option<AppId> {
        self.active.and_then(|i| self.apps.get(i)).map(|a| a.id())
    }

    /// Switches application inside the open part: deactivates the current
    /// one, activates `target`. The document is not touched.
    ///
    /// # Errors
    /// [`RegistryError::UnknownApp`].
    pub fn switch_to(&mut self, target: &AppId, ui: &mut UiContext) -> Result<(), RegistryError> {
        let index = self
            .apps
            .iter()
            .position(|a| &a.id() == target)
            .ok_or_else(|| RegistryError::UnknownApp(target.to_string()))?;
        if let Some(current) = self.active.and_then(|i| self.apps.get(i)) {
            current.deactivate(ui);
        }
        if let Some(next) = self.apps.get(index) {
            next.activate(ui);
            ui.active_app = Some(next.id());
        }
        self.active = Some(index);
        Ok(())
    }

    /// Fans a change set out to every application and collects the extra
    /// marks they request.
    #[must_use]
    pub fn notify(&self, change: &ChangeSet) -> ChangeCtx {
        let mut ctx = ChangeCtx::default();
        for app in &self.apps {
            app.on_upstream_change(&mut ctx, change);
        }
        ctx
    }
}

/// The registry regenerates objects by asking the application that owns
/// their kind. Kinds with no registered owner (for example from a plugin
/// that is not installed) are left alone: manual policy, preserved untouched.
impl RecomputeHandler for AppRegistry {
    fn policy(&self, kind: &ObjectKind) -> UpdatePolicy {
        self.owner_of(kind)
            .map_or(UpdatePolicy::Manual, |a| a.update_policy(kind))
    }

    fn recompute(&self, inputs: &Snapshot, object: &Object) -> Result<RecomputeOutput, String> {
        match self.owner_of(&object.kind) {
            Some(app) => app.recompute(inputs, object),
            None => Err(format!(
                "no installed application owns kind '{}'",
                object.kind
            )),
        }
    }
}

/// The "switch to application" command added for every registered app. Its
/// ID is derived from the application ID (`pxp.modeling` →
/// `app.switch.pxp.modeling`), so it is unique whenever application IDs are.
fn switch_command(id: &AppId, name: &'static str) -> CommandSpec {
    let shortcut = SWITCH_SHORTCUTS
        .iter()
        .find(|(a, _)| *a == id.as_str())
        .map(|(_, k)| Text::from(*k));
    CommandSpec {
        id: Text::from(format!("app.switch.{id}")),
        label: Text::from(name),
        app: id.clone(),
        placement: RibbonPlacement {
            tab: "Application".into(),
            group: "Applications".into(),
        },
        shortcut,
        description: "Switch to this application; the part stays open.".into(),
        status: CommandStatus::Available,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        id: &'static str,
        kinds: Vec<ObjectKind>,
        cmd: &'static str,
    }

    impl Application for Fake {
        fn id(&self) -> AppId {
            AppId::from_static(self.id)
        }
        fn name(&self) -> &'static str {
            "Fake"
        }
        fn planned_stage(&self) -> u8 {
            9
        }
        fn owned_kinds(&self) -> &[ObjectKind] {
            &self.kinds
        }
        fn commands(&self) -> Vec<CommandSpec> {
            vec![CommandSpec {
                id: self.cmd.into(),
                label: "Frobnicate".into(),
                app: self.id(),
                placement: RibbonPlacement {
                    tab: "Home".into(),
                    group: "Fake".into(),
                },
                shortcut: None,
                description: "Does a fake thing.".into(),
                status: CommandStatus::Planned { stage: 9 },
            }]
        }
        fn on_upstream_change(&self, ctx: &mut ChangeCtx, change: &ChangeSet) {
            for id in &change.modified {
                ctx.mark_out_of_date(*id);
            }
        }
        fn activate(&self, ui: &mut UiContext) {
            ui.ribbon_tabs = vec!["Home", "Application"];
        }
        fn deactivate(&self, ui: &mut UiContext) {
            ui.ribbon_tabs.clear();
        }
    }

    fn fake(id: &'static str, kind: &'static str, cmd: &'static str) -> Box<dyn Application> {
        Box::new(Fake {
            id,
            kinds: vec![ObjectKind::from_static(kind)],
            cmd,
        })
    }

    #[test]
    fn registration_rejects_conflicts_atomically() {
        let mut r = AppRegistry::new();
        r.register(fake("pxp.a", "a.thing", "a.frob")).unwrap();
        assert!(matches!(
            r.register(fake("pxp.a", "b.thing", "b.frob")),
            Err(RegistryError::BadAppId(_))
        ));
        assert!(matches!(
            r.register(fake("pxp.b", "a.thing", "b.frob")),
            Err(RegistryError::BadKind(_))
        ));
        assert!(matches!(
            r.register(fake("pxp.b", "b.thing", "a.frob")),
            Err(RegistryError::DuplicateCommand(_))
        ));
        assert!(matches!(
            r.register(fake("Bad Id", "c.thing", "c.frob")),
            Err(RegistryError::BadAppId(_))
        ));
        assert_eq!(r.apps().count(), 1, "failed registrations left no trace");
        // Two non-built-in applications can coexist (unique switch commands).
        r.register(fake("plugin.one", "one.thing", "one.frob"))
            .unwrap();
        r.register(fake("plugin.two", "two.thing", "two.frob"))
            .unwrap();
        assert!(r.command("app.switch.plugin.two").is_some());
    }

    #[test]
    fn planned_commands_say_so() {
        let mut r = AppRegistry::new();
        r.register(fake("pxp.a", "a.thing", "a.frob")).unwrap();
        let c = r.command("a.frob").unwrap();
        assert!(!c.is_available());
        assert_eq!(c.tooltip(), "Planned for Stage 9.");
    }

    #[test]
    fn switching_swaps_ui_only() {
        let mut r = AppRegistry::new();
        r.register(fake("pxp.a", "a.thing", "a.frob")).unwrap();
        let mut ui = UiContext::default();
        r.switch_to(&AppId::from_static("pxp.a"), &mut ui).unwrap();
        assert_eq!(ui.active_app, Some(AppId::from_static("pxp.a")));
        assert_eq!(ui.ribbon_tabs, vec!["Home", "Application"]);
        assert!(
            r.switch_to(&AppId::from_static("pxp.zzz"), &mut ui)
                .is_err()
        );
    }

    #[test]
    fn finder_puts_available_first() {
        let mut r = AppRegistry::new();
        r.register(fake("pxp.a", "a.thing", "a.frob")).unwrap();
        let hits = r.find("fake");
        assert!(hits.len() >= 2);
        assert!(
            hits[0].is_available(),
            "switch command (available) sorts first"
        );
        assert!(r.find("FROBNICATE").iter().any(|c| c.id == "a.frob"));
    }

    #[test]
    fn unowned_kinds_are_left_alone() {
        let r = AppRegistry::new();
        assert_eq!(
            r.policy(&ObjectKind::from_static("plugin.widget")),
            UpdatePolicy::Manual
        );
    }
}
