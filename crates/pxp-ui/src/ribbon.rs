// SPDX-License-Identifier: LGPL-2.1-or-later
//! The ribbon, built from data: every button is a `CommandSpec` from the
//! application registry, placed by its `RibbonPlacement`. Nothing here
//! knows what any command does.
//!
//! ```text
//! registry.commands() ──filter: active app (+ every app's switch command on "Application")──►
//!   group by tab (UiContext order) ──► group by group (first-seen order) ──► RibbonTab { groups { commands } }
//!   role Essentials: the first tab only
//! ```

use pxp_appkit::{AppRegistry, CommandSpec};

/// How much of the ribbon a role shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The first (Home) tab only.
    Essentials,
    /// Every tab.
    Advanced,
}

impl Role {
    /// Parses a customer-default value (`essentials` / `advanced`).
    #[must_use]
    pub fn parse(text: &str) -> Self {
        if text.eq_ignore_ascii_case("essentials") {
            Self::Essentials
        } else {
            Self::Advanced
        }
    }

    /// Label in the Roles navigator.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Essentials => "Essentials",
            Self::Advanced => "Advanced",
        }
    }
}

/// A group of buttons with its caption.
#[derive(Debug, Clone)]
pub struct RibbonGroup {
    /// Caption under the buttons.
    pub name: String,
    /// Buttons, in registry order.
    pub commands: Vec<CommandSpec>,
}

/// One ribbon tab.
#[derive(Debug, Clone)]
pub struct RibbonTab {
    /// Tab name.
    pub name: String,
    /// Groups, in first-seen order.
    pub groups: Vec<RibbonGroup>,
}

/// Builds the ribbon for the active application's tabs.
#[must_use]
pub fn build(registry: &AppRegistry, tabs: &[&str], role: Role) -> Vec<RibbonTab> {
    let active = registry.active_app();
    let shown: &[&str] = match role {
        Role::Essentials => tabs.get(..1).unwrap_or(&[]),
        Role::Advanced => tabs,
    };
    shown
        .iter()
        .map(|tab| {
            let mut groups: Vec<RibbonGroup> = Vec::new();
            for c in registry
                .commands()
                .iter()
                .filter(|c| c.placement.tab.as_ref() == *tab)
            {
                // The Application tab lists every application's switch
                // command; other tabs only the active application's.
                let belongs = c.id.starts_with("app.switch.") || Some(&c.app) == active.as_ref();
                if !belongs {
                    continue;
                }
                match groups
                    .iter_mut()
                    .find(|g| g.name == c.placement.group.as_ref())
                {
                    Some(g) => g.commands.push(c.clone()),
                    None => groups.push(RibbonGroup {
                        name: c.placement.group.to_string(),
                        commands: vec![c.clone()],
                    }),
                }
            }
            RibbonTab {
                name: (*tab).to_owned(),
                groups,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use pxp_appkit::UiContext;

    fn registry() -> (AppRegistry, UiContext) {
        let mut r = AppRegistry::new();
        r.register(Box::new(pxp_app_modeling::Modeling::new()))
            .unwrap();
        r.register(Box::new(pxp_app_drafting::Drafting)).unwrap();
        let mut ui = UiContext::default();
        r.switch_to(&pxp_appkit::AppId::new("pxp.modeling").unwrap(), &mut ui)
            .unwrap();
        (r, ui)
    }

    #[test]
    fn groups_are_whole_and_planned_commands_stay_visible() {
        let (r, ui) = registry();
        let ribbon = build(&r, &ui.ribbon_tabs, Role::Advanced);
        assert_eq!(ribbon.len(), ui.ribbon_tabs.len());
        let home = &ribbon[0];
        // Each group name appears once, even when the registry interleaves.
        let mut names: Vec<&str> = home.groups.iter().map(|g| g.name.as_str()).collect();
        let before = names.len();
        names.dedup();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), before);
        // Without a kernel nothing is available, yet everything is listed.
        let all: Vec<&CommandSpec> = home.groups.iter().flat_map(|g| &g.commands).collect();
        assert!(all.iter().any(|c| c.label.as_ref() == "Block"));
        assert!(
            all.iter()
                .all(|c| !c.is_available() && c.tooltip().starts_with("Planned for Stage"))
        );
        // The Application tab offers every application.
        let app_tab = ribbon.iter().find(|t| t.name == "Application").unwrap();
        let switches: Vec<&str> = app_tab
            .groups
            .iter()
            .flat_map(|g| &g.commands)
            .map(|c| c.id.as_ref())
            .collect();
        assert!(switches.contains(&"app.switch.pxp.drafting"));
    }

    #[test]
    fn essentials_shows_the_home_tab_only() {
        let (r, ui) = registry();
        let ribbon = build(&r, &ui.ribbon_tabs, Role::parse("Essentials"));
        assert_eq!(ribbon.len(), 1);
        assert_eq!(ribbon[0].name, ui.ribbon_tabs[0]);
    }
}
