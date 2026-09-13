//! Board-independent gamepad state delivered by a platform backend.

/// Standard gamepad button positions. Values are stable bits in a snapshot.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GamepadButton {
    South = 0,
    East = 1,
    C = 2,
    North = 3,
    West = 4,
    Z = 5,
    LeftShoulder = 6,
    RightShoulder = 7,
    LeftTrigger = 8,
    RightTrigger = 9,
    Select = 10,
    Start = 11,
    Home = 12,
    LeftStick = 13,
    RightStick = 14,
    Auxiliary1 = 16,
    Auxiliary2 = 17,
    Auxiliary3 = 18,
    Auxiliary4 = 19,
    Auxiliary5 = 20,
}

/// One coherent gamepad state. Sticks are -32767..32767 with positive Y down;
/// triggers are 0..32767 and hats -1..1. A reset invalidates cached input state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GamepadEvent {
    pub device_id: u32,
    pub time_ns: u64,
    pub buttons: u32,
    pub left_x: i16,
    pub left_y: i16,
    pub right_x: i16,
    pub right_y: i16,
    pub left_trigger: u16,
    pub right_trigger: u16,
    pub hat_x: i8,
    pub hat_y: i8,
    pub reset: bool,
}
impl GamepadEvent {
    pub const fn pressed(self, button: GamepadButton) -> bool {
        !self.reset && self.buttons & (1 << button as u8) != 0
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn position_identity_and_reset_do_not_invent_navigation_keys() {
        let event = GamepadEvent {
            buttons: 1 << 1,
            ..GamepadEvent::default()
        };
        assert!(event.pressed(GamepadButton::East));
        assert!(!event.pressed(GamepadButton::South));
        assert!(
            !GamepadEvent {
                reset: true,
                ..event
            }
            .pressed(GamepadButton::East)
        );
    }
}
