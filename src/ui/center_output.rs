//! The output the notification center is on. It stays on one output and moves only when it is
//! asked to open while the pointer is on another one, so a pointer that crosses monitors builds
//! no surface again. Without `crownos_output_info_v1` it stays on the first output it is put on,
//! until that output goes.

use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};

use crownui::surfaces::{Output, OutputId, Waker};

use crate::util::lock::lock;

#[derive(Clone, Default)]
pub struct CenterOutput(Arc<Shared>);

#[derive(Default)]
struct Shared {
    held: Mutex<Option<OutputId>>,
    focused: Mutex<Option<OutputId>>,
    on_screen: AtomicBool,
    runner_waker: OnceLock<Waker>,
}

impl CenterOutput {
    pub fn attach(&self, runner_waker: Waker) {
        let _ = self.0.runner_waker.set(runner_waker);
    }

    /// Whether the center belongs on `output`. Only while `opening` does it go to the focused
    /// output.
    pub fn hosts(&self, output: &Output, opening: bool) -> bool {
        let focused = output.crown.as_ref().map(|crown| crown.focused);
        if focused == Some(true) {
            *lock(&self.0.focused) = Some(output.id);
        }
        hosts(
            &mut lock(&self.0.held),
            output.id,
            focused.filter(|_| opening),
        )
    }

    /// Whether the panel is open or still sliding shut.
    pub fn on_screen(&self) -> bool {
        self.0.on_screen.load(Ordering::Relaxed)
    }

    /// Puts the center on `output` for as long as the returned seat lives.
    pub fn seat(&self, output: OutputId) -> CenterSeat {
        *lock(&self.0.held) = Some(output);
        CenterSeat {
            center: self.clone(),
            output,
        }
    }

    fn wake_runner(&self) {
        if let Some(wake_runner) = self.0.runner_waker.get() {
            wake_runner();
        }
    }
}

/// Whether `output` hosts a surface that follows the focus: `following_focus` is whether `output`
/// is focused, or `None` when the surface stays where it is held. A surface held nowhere is held
/// by the first output asked.
pub fn hosts<Id: Copy + PartialEq>(
    held: &mut Option<Id>,
    output: Id,
    following_focus: Option<bool>,
) -> bool {
    match following_focus {
        Some(focused) => focused,
        None => *held.get_or_insert(output) == output,
    }
}

/// The center window's hold on the output it was built for. Once the window goes, so does the
/// hold, and the center is put on another output.
pub struct CenterSeat {
    center: CenterOutput,
    output: OutputId,
}

impl CenterSeat {
    /// Whether the center may open on this output: the pointer is on it, or the compositor does
    /// not say where the pointer is.
    pub fn may_open(&self) -> bool {
        lock(&self.center.0.focused).is_none_or(|focused| focused == self.output)
    }

    pub fn set_on_screen(&self, on_screen: bool) {
        self.center.0.on_screen.store(on_screen, Ordering::Relaxed);
    }

    /// Asks the runner to put the center on the focused output.
    pub fn move_to_focus(&self) {
        self.center.wake_runner();
    }
}

impl Drop for CenterSeat {
    fn drop(&mut self) {
        let released = {
            let mut held = lock(&self.center.0.held);
            let ours = *held == Some(self.output);
            if ours {
                *held = None;
            }
            ours
        };
        if released {
            self.set_on_screen(false);
            self.center.wake_runner();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::hosts;

    #[test]
    fn the_first_output_asked_holds_the_surface_until_it_is_released() {
        let mut held = None;
        assert!(hosts(&mut held, 2, None));
        assert!(!hosts(&mut held, 1, None));
        assert!(hosts(&mut held, 2, None));
        held = None;
        assert!(hosts(&mut held, 1, None));
        assert!(!hosts(&mut held, 2, None));
    }

    #[test]
    fn a_surface_following_the_focus_is_on_the_focused_output_whatever_holds_it() {
        let mut held = Some(1);
        assert!(!hosts(&mut held, 1, Some(false)));
        assert!(hosts(&mut held, 2, Some(true)));
        assert_eq!(held, Some(1));
    }

    #[test]
    fn the_answer_does_not_depend_on_the_order_the_outputs_are_asked_in() {
        for order in [[1, 2, 3], [3, 2, 1], [2, 3, 1]] {
            let mut held = Some(3);
            let hosting: Vec<i32> = order
                .into_iter()
                .filter(|&output| hosts(&mut held, output, None))
                .collect();
            assert_eq!(hosting, [3]);
            let following: Vec<i32> = order
                .into_iter()
                .filter(|&output| hosts(&mut held, output, Some(output == 2)))
                .collect();
            assert_eq!(following, [2]);
        }
    }
}
