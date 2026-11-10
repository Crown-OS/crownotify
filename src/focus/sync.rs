//! Focus as the paired devices see it, and back.
//!
//! crownlink keeps the stamps that order focus changes across devices, so this
//! side only translates. A local change is reported as it is; a device's focus
//! is adopted as the closest mode crownotify has, and the change that adoption
//! causes is reported like any other. crownlink keeps the device's stamp when
//! the report repeats what the device said and mints a fresh one when it does
//! not, which is how both ends settle on the mode this computer really has.

use crownlink_linux::ipc::proto::{FocusInfo, FocusMode as DeviceFocus};

use crate::focus::FocusModes;

/// The focus crownlink publishes for the mode now in force.
pub fn reported(mode: Option<&str>) -> FocusInfo {
    let mode = match mode {
        None => DeviceFocus::Off,
        Some(FocusModes::DO_NOT_DISTURB) => DeviceFocus::DoNotDisturb,
        Some(name) => DeviceFocus::Named(name.to_string()),
    };
    FocusInfo {
        mode,
        until_unix_ms: None,
    }
}

/// The mode to put in force for a device's `focus`.
///
/// A named mode this computer does not define falls back to Do Not Disturb:
/// the device asked for quiet, and the preset is the quiet every set has.
pub fn adopted<'a>(focus: &'a FocusInfo, modes: &FocusModes) -> Option<&'a str> {
    match &focus.mode {
        DeviceFocus::Off => None,
        DeviceFocus::DoNotDisturb => Some(FocusModes::DO_NOT_DISTURB),
        DeviceFocus::Named(name) if modes.position(name).is_some() => Some(name),
        DeviceFocus::Named(_) => Some(FocusModes::DO_NOT_DISTURB),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{focus::FocusMode, model::InterruptionLevel};

    fn device(mode: DeviceFocus) -> FocusInfo {
        FocusInfo {
            mode,
            until_unix_ms: None,
        }
    }

    fn with_work() -> FocusModes {
        FocusModes::new(vec![FocusMode::new("work", InterruptionLevel::Critical)])
    }

    #[test]
    fn no_mode_is_reported_as_off_and_the_preset_as_do_not_disturb() {
        assert_eq!(reported(None), device(DeviceFocus::Off));
        assert_eq!(
            reported(Some(FocusModes::DO_NOT_DISTURB)),
            device(DeviceFocus::DoNotDisturb)
        );
        assert_eq!(
            reported(Some("work")),
            device(DeviceFocus::Named("work".to_string()))
        );
    }

    #[test]
    fn a_known_named_mode_is_adopted_by_its_own_name() {
        let focus = device(DeviceFocus::Named("Work".to_string()));
        assert_eq!(adopted(&focus, &with_work()), Some("Work"));
    }

    #[test]
    fn an_unknown_named_mode_falls_back_to_do_not_disturb() {
        let focus = device(DeviceFocus::Named("sleep".to_string()));
        assert_eq!(
            adopted(&focus, &with_work()),
            Some(FocusModes::DO_NOT_DISTURB)
        );
    }

    #[test]
    fn off_and_do_not_disturb_map_to_no_mode_and_the_preset() {
        let modes = FocusModes::default();
        assert_eq!(adopted(&device(DeviceFocus::Off), &modes), None);
        assert_eq!(
            adopted(&device(DeviceFocus::DoNotDisturb), &modes),
            Some(FocusModes::DO_NOT_DISTURB)
        );
    }

    #[test]
    fn adopting_a_report_reproduces_the_mode_it_came_from() {
        let modes = with_work();
        for mode in [None, Some(FocusModes::DO_NOT_DISTURB), Some("work")] {
            assert_eq!(adopted(&reported(mode), &modes), mode);
        }
    }
}
