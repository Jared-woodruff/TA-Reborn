//! Interface sounds. Widgets ask for one when they are used, and the game plays TA's own
//! interface sound for it (the event in `gamedata/allsound.tdf`, [`UiSound::event`]). One
//! sound a frame: a later request replaces an earlier one, so a menu entry can ask for its
//! own sound over the button's. Nothing is asked for until [`enable`] (the launcher is
//! silent).

use egui::{Context, Id};

/// What an interface sound is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiSound {
    /// A title-menu entry or the big call to action.
    BigButton,
    /// A button in a page or dialog.
    Button,
    /// A switch, a segment or a list row.
    Toggle,
    /// Back to the previous page.
    Back,
    /// A tab or a settings category.
    Tab,
    /// The title menu's own entries.
    Skirmish,
    Multiplayer,
    Options,
    Exit,
    /// A message comes in (a briefing's transmission, a chat line), or a warning.
    Message,
    /// An achievement is earned.
    Achievement,
    /// A slider moved.
    Slider,
    /// A dialog or panel opens.
    Panel,
    /// Something refused, or failed.
    Refuse,
    /// Something done (a download, an install, a check passed).
    Done,
}

impl UiSound {
    /// The event of TA's `gamedata/allsound.tdf` whose sound this plays.
    pub fn event(self) -> &'static str {
        match self {
            UiSound::BigButton => "BIGBUTTON",
            UiSound::Button => "SMLBUTTON",
            UiSound::Toggle => "SMALLBUTTON",
            UiSound::Back => "PREVIOUS",
            UiSound::Tab => "MORE",
            UiSound::Skirmish => "SKIRMISH",
            UiSound::Multiplayer => "MULTI",
            UiSound::Options => "OPTIONS",
            UiSound::Exit => "EXIT",
            UiSound::Message => "MessageArrived",
            UiSound::Achievement => "ENDGAMESCORE",
            UiSound::Slider => "SLIDER",
            UiSound::Panel => "PANEL",
            UiSound::Refuse => "SKIRMISHCHEAT",
            UiSound::Done => "ENDGAMESTATBAR",
        }
    }
}

fn enabled_id() -> Id {
    Id::new("ta_ui::sound::enabled")
}

fn queued_id() -> Id {
    Id::new("ta_ui::sound::queued")
}

/// Turns interface sounds on (the game) or off.
pub fn enable(ctx: &Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(enabled_id(), on));
}

/// Asks for a sound this frame (replacing one asked for earlier in the frame).
pub fn play(ctx: &Context, sound: UiSound) {
    if ctx.data(|d| d.get_temp::<bool>(enabled_id())).unwrap_or(false) {
        ctx.data_mut(|d| d.insert_temp(queued_id(), Some(sound)));
    }
}

/// The sound asked for since the last call, if any.
pub fn take(ctx: &Context) -> Option<UiSound> {
    ctx.data_mut(|d| d.remove_temp::<Option<UiSound>>(queued_id())).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_sound_a_frame_the_last_asked_for() {
        let ctx = Context::default();
        play(&ctx, UiSound::Button);
        assert_eq!(take(&ctx), None, "silent until enabled");
        enable(&ctx, true);
        play(&ctx, UiSound::BigButton);
        play(&ctx, UiSound::Skirmish);
        assert_eq!(take(&ctx), Some(UiSound::Skirmish));
        assert_eq!(take(&ctx), None);
    }
}
