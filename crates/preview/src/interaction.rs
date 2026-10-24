//! Pointer input to camera moves, for a preview a shell's window takes input
//! for.
//!
//! The viewer's gestures where a pane has room for them: the left button
//! orbits, the wheel zooms, `Alt` + right drags a zoom — at the viewer's own
//! rates ([`review_render::gesture`]). The right button pans rather than
//! looking around: the viewer's look drag exists to steer its WASD flycam, which
//! a preview has no keys for, and on its own it only turns the model out of
//! view. The middle button pans too, as in the viewer.

use glam::Vec2;
use review_render::gesture::{
    DRAG_ZOOM_SENSITIVITY, WHEEL_LINE_ZOOM_STEP, WHEEL_PIXELS_PER_ZOOM_STEP,
};

use crate::Viewport;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// The modifier keys held when a button went down.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drag {
    Orbit,
    Pan,
    Zoom,
}

/// The drag in progress, if any, and where the pointer was last.
#[derive(Debug, Default)]
pub struct Interaction {
    drag: Option<(Button, Drag)>,
    last: Option<Vec2>,
}

impl Interaction {
    /// A button went down at `position` (pixels from the view's top left).
    pub fn press(&mut self, button: Button, position: Vec2, modifiers: Modifiers) {
        let drag = match button {
            Button::Left => Drag::Orbit,
            Button::Right if modifiers.alt => Drag::Zoom,
            Button::Right | Button::Middle => Drag::Pan,
        };
        self.drag = Some((button, drag));
        self.last = Some(position);
    }

    /// A button came up. Only the one that started the drag ends it.
    pub fn release(&mut self, button: Button) {
        if self.drag.is_some_and(|(held, _)| held == button) {
            self.drag = None;
        }
    }

    /// Whether a button is down: what a shell holds the pointer capture for.
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// The pointer moved to `position` in a `view`-sized view. True when the
    /// camera moved.
    pub fn moved(&mut self, viewport: &mut Viewport, position: Vec2, view: Vec2) -> bool {
        let last = self.last.replace(position);
        let (Some((_, drag)), Some(last)) = (self.drag, last) else {
            return false;
        };
        let delta = position - last;
        if delta == Vec2::ZERO {
            return false;
        }
        match drag {
            Drag::Orbit => viewport.orbit(delta),
            Drag::Pan => viewport.pan(delta, view),
            // Pointer down zooms in, up zooms out: the wheel's positive-is-in.
            Drag::Zoom => viewport.zoom(delta.y * DRAG_ZOOM_SENSITIVITY),
        }
        true
    }

    /// The wheel turned `notches` (positive away from the user, which zooms in).
    pub fn wheel_notches(&mut self, viewport: &mut Viewport, notches: f32) {
        viewport.zoom(notches * WHEEL_LINE_ZOOM_STEP);
    }

    /// A pixel-precise scroll of `pixels` (a trackpad), at the same rate a wheel
    /// notch is worth.
    pub fn wheel_pixels(&mut self, viewport: &mut Viewport, pixels: f32) {
        viewport.zoom(pixels / WHEEL_PIXELS_PER_ZOOM_STEP);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_button_that_started_a_drag_ends_it() {
        let mut interaction = Interaction::default();
        interaction.press(Button::Left, Vec2::ZERO, Modifiers::default());
        interaction.release(Button::Right);
        assert!(interaction.is_dragging());
        interaction.release(Button::Left);
        assert!(!interaction.is_dragging());
    }

    #[test]
    fn alt_turns_the_right_drag_into_a_zoom() {
        let mut interaction = Interaction::default();
        interaction.press(
            Button::Right,
            Vec2::ZERO,
            Modifiers {
                alt: true,
                ..Modifiers::default()
            },
        );
        assert_eq!(interaction.drag, Some((Button::Right, Drag::Zoom)));
        interaction.press(Button::Right, Vec2::ZERO, Modifiers::default());
        assert_eq!(interaction.drag, Some((Button::Right, Drag::Pan)));
    }
}
