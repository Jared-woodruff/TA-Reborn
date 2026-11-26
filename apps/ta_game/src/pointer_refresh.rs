//! When a Mac's window has its pointer set again after a change (`macos.rs`: the window coming into
//! view, taking the keyboard or changing size): at once, and a few times as the change settles, then
//! never until the next change. Never steadily: set again once a second, for as long as the game ran,
//! macOS held the game's mouse and keyboard input back (issue #154). A plain schedule, here so that
//! every platform's tests hold it.

use std::time::Duration;

/// After the change, when the pointer is set again (it is at once as well).
pub const FOLLOW_UPS: [Duration; 3] = [Duration::from_millis(250), Duration::from_millis(750), Duration::from_millis(1500)];

/// With `done` of the follow-ups made and `elapsed` since the change: whether the pointer is to be
/// set again now (one at a time, however many came due together), and how many are done then
/// (`None`: all of them, nothing more to come until the next change).
pub fn follow_up(elapsed: Duration, done: usize) -> (bool, Option<usize>) {
    let passed = FOLLOW_UPS.iter().take_while(|&&after| elapsed >= after).count();
    if passed <= done {
        return (false, Some(done));
    }
    (true, (passed < FOLLOW_UPS.len()).then_some(passed))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The follow-ups a run of frames makes after one change, the frames `step` apart for `run`.
    fn made(step: Duration, run: Duration) -> usize {
        let (mut state, mut elapsed, mut count) = (Some(0), Duration::ZERO, 0);
        while elapsed <= run {
            if let Some(done) = state {
                let (due, next) = follow_up(elapsed, done);
                count += due as usize;
                state = next;
            }
            elapsed += step;
        }
        count
    }

    /// After a change the pointer is set again three times in its first second and a half, and then
    /// never, however long the game runs: an hour at sixty frames a second makes no more (issue #154).
    #[test]
    fn the_pointer_is_set_again_a_few_times_after_a_change_and_never_steadily() {
        assert_eq!(made(Duration::from_micros(16_667), Duration::from_secs(3600)), FOLLOW_UPS.len());
        // (Slow frames, a second apart: each follow-up once all the same, never two at once.)
        assert_eq!(made(Duration::from_secs(1), Duration::from_secs(60)), 2, "the first two came due in one frame: one set made for them");
        assert_eq!(follow_up(Duration::from_millis(100), 0), (false, Some(0)), "none before the first is due");
        assert_eq!(follow_up(Duration::from_secs(10), 0), (true, None), "late: one, then nothing more");
    }
}
