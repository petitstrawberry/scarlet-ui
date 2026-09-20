//! Press feedback belongs to the input source that began it.

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PressSources {
    mouse: bool,
    touch: Option<u64>,
}

impl PressSources {
    pub(crate) fn set_mouse(&mut self, pressed: bool) {
        self.mouse = pressed;
    }

    pub(crate) fn set_touch(&mut self, id: u64, pressed: bool) {
        if pressed {
            if self.touch.is_none() {
                self.touch = Some(id);
            }
        } else if self.touch == Some(id) {
            self.touch = None;
        }
    }

    pub(crate) fn is_pressed(&self) -> bool {
        self.mouse || self.touch.is_some()
    }

    pub(crate) fn is_mouse_pressed(&self) -> bool {
        self.mouse
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ending_touch_does_not_release_mouse_press() {
        let mut sources = PressSources::default();
        sources.set_mouse(true);
        sources.set_touch(7, true);
        sources.set_touch(8, false);
        assert!(sources.is_pressed());
        sources.set_touch(7, false);
        assert!(sources.is_pressed());
        assert!(sources.is_mouse_pressed());
        sources.set_mouse(false);
        assert!(!sources.is_pressed());
    }
}
