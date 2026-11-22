// SPDX-License-Identifier: LGPL-2.1-or-later
//! The NX mouse grammar as a toolkit-independent state machine.
//!
//! | Gesture | Action |
//! | --- | --- |
//! | MB1 click | select (the shell applies the selection filter) |
//! | MB2 drag | rotate (orbit) |
//! | MB2 + MB3 drag, or Shift + MB2 drag | pan |
//! | MB1 + MB2 drag, or Ctrl + MB2 drag | zoom |
//! | wheel | zoom about the cursor |
//! | MB2 click (no drag) | OK in the active dialog |
//! | MB3 click | context menu |
//!
//! The toolkit feeds button and motion events; [`MouseGrammar`] answers
//! with [`ViewAction`]s. A press becomes a drag only after it moves more
//! than [`DRAG_THRESHOLD`] pixels, so a slightly shaky click is still a
//! click (MB2 = OK must not orbit the view by a pixel).

/// Pixels a button must move while held before it counts as a drag.
pub const DRAG_THRESHOLD: f64 = 4.0;

/// Wheel zoom per notch (one "line" of scroll): 10 %, as NX.
pub const WHEEL_STEP: f64 = 1.1;

/// Mouse buttons in NX naming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// Left.
    Mb1,
    /// Middle (wheel press).
    Mb2,
    /// Right.
    Mb3,
}

/// Keyboard modifiers that change a gesture.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// Shift held.
    pub shift: bool,
    /// Ctrl (Cmd on macOS) held.
    pub ctrl: bool,
}

/// What the viewport should do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ViewAction {
    /// Nothing.
    None,
    /// Orbit by a pixel delta.
    Rotate {
        /// Pixels right.
        dx: f64,
        /// Pixels down.
        dy: f64,
    },
    /// Pan by a pixel delta.
    Pan {
        /// Pixels right.
        dx: f64,
        /// Pixels down.
        dy: f64,
    },
    /// Zoom by a factor (> 1 is in) about a viewport position.
    Zoom {
        /// Zoom factor.
        factor: f64,
        /// Cursor position (viewport pixels from the top-left).
        at: [f64; 2],
    },
    /// MB1 click: pick at the position.
    Select {
        /// Viewport pixels from the top-left.
        at: [f64; 2],
        /// Shift held: add to the selection.
        extend: bool,
    },
    /// MB2 click: OK in the active dialog.
    Ok,
    /// MB3 click: context menu at the position.
    Context {
        /// Viewport pixels from the top-left.
        at: [f64; 2],
    },
}

/// The gesture in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gesture {
    Idle,
    /// Buttons down, not yet moved past the threshold.
    Pressed,
    Rotate,
    Pan,
    Zoom,
}

/// Tracks buttons and turns events into [`ViewAction`]s.
#[derive(Debug, Clone)]
pub struct MouseGrammar {
    down: [bool; 3],
    press_at: [f64; 2],
    last: [f64; 2],
    gesture: Gesture,
    /// Every button that went down during the current press (a chord
    /// MB1+MB2 released one button at a time must not also click).
    chord: [bool; 3],
}

impl Default for MouseGrammar {
    fn default() -> Self {
        Self {
            down: [false; 3],
            press_at: [0.0; 2],
            last: [0.0; 2],
            gesture: Gesture::Idle,
            chord: [false; 3],
        }
    }
}

const fn idx(b: Button) -> usize {
    match b {
        Button::Mb1 => 0,
        Button::Mb2 => 1,
        Button::Mb3 => 2,
    }
}

impl MouseGrammar {
    /// A button went down at `at`.
    pub fn press(&mut self, b: Button, at: [f64; 2]) {
        if self.down.iter().all(|d| !d) {
            self.press_at = at;
            self.chord = [false; 3];
            self.gesture = Gesture::Pressed;
        }
        if let Some(d) = self.down.get_mut(idx(b)) {
            *d = true;
        }
        if let Some(c) = self.chord.get_mut(idx(b)) {
            *c = true;
        }
        self.last = at;
    }

    /// The pointer moved to `at`.
    pub fn motion(&mut self, at: [f64; 2], mods: Modifiers) -> ViewAction {
        let (dx, dy) = (at[0] - self.last[0], at[1] - self.last[1]);
        self.last = at;
        if self.gesture == Gesture::Pressed {
            let moved = (at[0] - self.press_at[0]).hypot(at[1] - self.press_at[1]);
            if moved <= DRAG_THRESHOLD {
                return ViewAction::None;
            }
            self.gesture = self.classify(mods);
        }
        match self.gesture {
            Gesture::Rotate => ViewAction::Rotate { dx, dy },
            Gesture::Pan => ViewAction::Pan { dx, dy },
            // Dragging down zooms in, as NX: 1 % per pixel.
            Gesture::Zoom => ViewAction::Zoom {
                factor: 1.01_f64.powf(dy),
                at: self.press_at,
            },
            Gesture::Idle | Gesture::Pressed => ViewAction::None,
        }
    }

    /// Which drag the held buttons and modifiers mean.
    fn classify(&self, mods: Modifiers) -> Gesture {
        let [b1, b2, b3] = self.down;
        match (b1, b2, b3) {
            (_, true, true) => Gesture::Pan,
            (true, true, false) => Gesture::Zoom,
            (false, true, false) if mods.shift => Gesture::Pan,
            (false, true, false) if mods.ctrl => Gesture::Zoom,
            (false, true, false) => Gesture::Rotate,
            // MB1 drag (rectangle selection) and MB3 drag are not view
            // gestures; Stage 1 adds rectangle selection.
            _ => Gesture::Idle,
        }
    }

    /// A button went up at `at`.
    pub fn release(&mut self, b: Button, at: [f64; 2], mods: Modifiers) -> ViewAction {
        if let Some(d) = self.down.get_mut(idx(b)) {
            *d = false;
        }
        let all_up = self.down.iter().all(|d| !d);
        let was = self.gesture;
        if all_up {
            self.gesture = Gesture::Idle;
        }
        // A click: the press never became a drag, and it was one button.
        let single = self.chord.iter().filter(|c| **c).count() == 1;
        if !(all_up && was == Gesture::Pressed && single) {
            return ViewAction::None;
        }
        match b {
            Button::Mb1 => ViewAction::Select {
                at,
                extend: mods.shift,
            },
            Button::Mb2 => ViewAction::Ok,
            Button::Mb3 => ViewAction::Context { at },
        }
    }

    /// Wheel by `lines` notches (positive = away from the user = zoom in).
    #[must_use]
    pub fn wheel(&self, lines: f64, at: [f64; 2]) -> ViewAction {
        if lines == 0.0 || !lines.is_finite() {
            return ViewAction::None;
        }
        ViewAction::Zoom {
            factor: WHEEL_STEP.powf(lines),
            at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
    };

    fn drag(g: &mut MouseGrammar, buttons: &[Button], mods: Modifiers) -> ViewAction {
        for &b in buttons {
            g.press(b, [100.0, 100.0]);
        }
        let _ = g.motion([103.0, 100.0], mods); // under the threshold
        let a = g.motion([110.0, 105.0], mods);
        let mut last = ViewAction::None;
        for &b in buttons {
            last = g.release(b, [110.0, 105.0], mods);
        }
        assert_eq!(last, ViewAction::None, "a drag is never also a click");
        a
    }

    #[test]
    fn the_nx_drags() {
        let mut g = MouseGrammar::default();
        assert!(matches!(
            drag(&mut g, &[Button::Mb2], NONE),
            ViewAction::Rotate { .. }
        ));
        assert!(matches!(
            drag(&mut g, &[Button::Mb2, Button::Mb3], NONE),
            ViewAction::Pan { .. }
        ));
        assert!(matches!(
            drag(
                &mut g,
                &[Button::Mb2],
                Modifiers {
                    shift: true,
                    ctrl: false
                }
            ),
            ViewAction::Pan { .. }
        ));
        assert!(matches!(
            drag(&mut g, &[Button::Mb1, Button::Mb2], NONE),
            ViewAction::Zoom { .. }
        ));
        assert!(matches!(
            drag(
                &mut g,
                &[Button::Mb2],
                Modifiers {
                    shift: false,
                    ctrl: true
                }
            ),
            ViewAction::Zoom { .. }
        ));
        assert_eq!(
            drag(&mut g, &[Button::Mb1], NONE),
            ViewAction::None,
            "MB1 drag is not a view gesture"
        );
    }

    #[test]
    fn clicks_select_ok_and_open_the_menu() {
        let mut g = MouseGrammar::default();
        g.press(Button::Mb1, [5.0, 6.0]);
        let _ = g.motion([7.0, 7.0], NONE); // a shaky click
        assert_eq!(
            g.release(
                Button::Mb1,
                [7.0, 7.0],
                Modifiers {
                    shift: true,
                    ctrl: false
                }
            ),
            ViewAction::Select {
                at: [7.0, 7.0],
                extend: true
            }
        );
        g.press(Button::Mb2, [5.0, 6.0]);
        assert_eq!(g.release(Button::Mb2, [5.0, 6.0], NONE), ViewAction::Ok);
        g.press(Button::Mb3, [5.0, 6.0]);
        assert_eq!(
            g.release(Button::Mb3, [5.0, 6.0], NONE),
            ViewAction::Context { at: [5.0, 6.0] }
        );
        // A chord released without moving is not a click of either button.
        g.press(Button::Mb1, [5.0, 6.0]);
        g.press(Button::Mb2, [5.0, 6.0]);
        assert_eq!(g.release(Button::Mb1, [5.0, 6.0], NONE), ViewAction::None);
        assert_eq!(g.release(Button::Mb2, [5.0, 6.0], NONE), ViewAction::None);
    }

    #[test]
    fn wheel_zooms_about_the_cursor() {
        let g = MouseGrammar::default();
        assert_eq!(
            g.wheel(1.0, [1.0, 2.0]),
            ViewAction::Zoom {
                factor: WHEEL_STEP,
                at: [1.0, 2.0]
            }
        );
        assert_eq!(g.wheel(0.0, [1.0, 2.0]), ViewAction::None);
    }
}
