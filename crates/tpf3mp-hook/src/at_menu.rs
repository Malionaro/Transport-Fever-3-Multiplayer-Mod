//! Whether the game is at its main menu with no world, as the main menu's
//! frame sees it (`crate::install::menu_frame`; docs/HOOKS.md, "Loading from
//! the main menu").
//!
//! The menu's frame, `UI::CMenuUI::DoStep`, runs at the menu and in a world
//! alike, so it cannot say by itself where the game is. A world that stops
//! stepping (saving the room's world, held for another player) is no menu:
//! taking the room's session from the menu's frames there hung the owner's
//! game (measured 2026-09-30). The signal is the engine's own:
//! `CMenuUI::m_game`, the loaded world, which `CMenuUI::StartGame` sets
//! (asserting `!m_game` first) and `CMenuUI::StopGame` clears, the one
//! pointer `DoStep` tests before it hands its frame to the world's UI
//! (`investigation/TPF3_MENU_JOIN_2026-09-30.md`, section 8). Loading is the
//! menu's own sign: the progress monitor has a task.
//!
//! [`MenuGate`] is the rule, kept free of the game so it is tested on its
//! own:
//!
//! - a game that has had no world up yet is at its menu ([`Seen::Fresh`]),
//!   loading or not, as before this rule;
//! - a world loaded ([`Seen::WorldUp`]) blocks, before its first step too
//!   (the owner's save for the room comes then);
//! - after a world, the menu is back only once no world is loaded and
//!   nothing loads for [`QUIET_MS`] with no step between
//!   ([`Seen::BackAtMenu`]); a load after the world closed (the GUI's load
//!   stops the world first) blocks ([`Seen::Loading`], [`Seen::Closing`]);
//! - after a world, a game the hook cannot read (no `m_game` in the
//!   profile, no menu Lua state to ask) is never taken for the menu
//!   ([`Seen::Unknown`], fail closed).

/// How long, after a world closed, nothing may be loaded or loading before
/// the menu's frame follows the room again: the frames between the GUI's
/// load stopping the world and the game starting to load the next.
pub const QUIET_MS: u64 = 2_000;

/// Where the game is, as the main menu's frame sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seen {
    /// No world has been up in this game yet.
    Fresh,
    /// A world is loaded (`CMenuUI::m_game` is set): the step's detour
    /// follows the room.
    WorldUp,
    /// After a world, no world is loaded but the game is loading one.
    Loading,
    /// After a world, no world is loaded and nothing loads, for less than
    /// [`QUIET_MS`] so far.
    Closing,
    /// Back at the main menu after a world: nothing loaded or loading for
    /// [`QUIET_MS`].
    BackAtMenu,
    /// After a world, the hook cannot tell whether one is loaded.
    Unknown,
}

impl Seen {
    /// Whether the menu's frame follows the room.
    pub fn allows(self) -> bool {
        matches!(self, Self::Fresh | Self::BackAtMenu)
    }

    /// For the hook's log.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Fresh => "at the main menu, no world up yet in this game",
            Self::WorldUp => "a world is loaded (CMenuUI::m_game set)",
            Self::Loading => "no world loaded, but the game is loading one",
            Self::Closing => "the world just closed; waiting for the menu to be quiet",
            Self::BackAtMenu => {
                "back at the main menu after a world (no world loaded or loading for 2 s)"
            }
            Self::Unknown => {
                "after a world, the hook cannot tell whether one is loaded (fail closed)"
            }
        }
    }
}

/// What one of the menu's frames reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    /// Now, in milliseconds on the hook's clock (never 0).
    pub now_ms: u64,
    /// When the game's step last ran, on the same clock; 0 never.
    pub last_step_ms: u64,
    /// Whether any world's GUI has started in this game.
    pub world_gui_started: bool,
    /// `CMenuUI::m_game` is set; `None` when the hook cannot read it.
    pub world_loaded: Option<bool>,
    /// The progress monitor has a task; `None` when the hook cannot ask.
    pub loading: Option<bool>,
}

/// The rule of [`Seen`], over the menu's frames.
#[derive(Debug, Default)]
pub struct MenuGate {
    /// A world has been up in this game: stepped, its GUI started, or seen
    /// loaded.
    after_world: bool,
    /// Since when nothing has been loaded or loading after a world.
    quiet_since: Option<u64>,
}

impl MenuGate {
    /// A game that has had no world up yet.
    pub const fn new() -> Self {
        Self {
            after_world: false,
            quiet_since: None,
        }
    }

    /// Whether this game has had a world up.
    pub fn after_world(&self) -> bool {
        self.after_world
    }

    /// Where the game is at this frame.
    pub fn frame(&mut self, frame: &Frame) -> Seen {
        if frame.last_step_ms != 0 || frame.world_gui_started || frame.world_loaded == Some(true) {
            self.after_world = true;
        }
        let seen = if frame.world_loaded == Some(true) {
            Seen::WorldUp
        } else if !self.after_world {
            Seen::Fresh
        } else {
            match (frame.world_loaded, frame.loading) {
                (Some(true), _) => Seen::WorldUp,
                (None, _) | (_, None) => Seen::Unknown,
                (_, Some(true)) => Seen::Loading,
                (Some(false), Some(false)) => {
                    let since = *self.quiet_since.get_or_insert(frame.now_ms);
                    if frame.last_step_ms >= since {
                        // A step ran since the quiet began: a world was up.
                        self.quiet_since = Some(frame.now_ms);
                        Seen::Closing
                    } else if frame.now_ms.saturating_sub(since) >= QUIET_MS {
                        Seen::BackAtMenu
                    } else {
                        Seen::Closing
                    }
                }
            }
        };
        if !matches!(seen, Seen::Closing | Seen::BackAtMenu) {
            self.quiet_since = None;
        }
        seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(now_ms: u64) -> Frame {
        Frame {
            now_ms,
            last_step_ms: 0,
            world_gui_started: false,
            world_loaded: Some(false),
            loading: Some(false),
        }
    }

    #[test]
    fn a_game_that_never_had_a_world_is_at_its_menu_loading_or_not() {
        let mut gate = MenuGate::default();
        assert_eq!(gate.frame(&at(1)), Seen::Fresh);
        // Its own load from the menu, or the player's: as before the rule.
        let loading = Frame {
            loading: Some(true),
            ..at(2)
        };
        assert_eq!(gate.frame(&loading), Seen::Fresh);
        // Nothing to read in the game: as before the rule too.
        let unread = Frame {
            world_loaded: None,
            loading: None,
            ..at(3)
        };
        assert_eq!(gate.frame(&unread), Seen::Fresh);
        assert!(Seen::Fresh.allows());
        assert!(!gate.after_world());
    }

    #[test]
    fn a_world_up_blocks_then_its_close_allows_after_the_quiet_stretch() {
        let mut gate = MenuGate::default();
        assert_eq!(gate.frame(&at(1)), Seen::Fresh);
        // The world loads: loaded before its first step, then stepping.
        let loaded = Frame {
            world_loaded: Some(true),
            ..at(10)
        };
        assert_eq!(gate.frame(&loaded), Seen::WorldUp);
        let stepping = Frame {
            last_step_ms: 20,
            world_gui_started: true,
            world_loaded: Some(true),
            ..at(20)
        };
        assert_eq!(gate.frame(&stepping), Seen::WorldUp);
        assert!(!Seen::WorldUp.allows());
        // Saving the room's world, or held for another player: no step for
        // long, the world still loaded.
        let saving = Frame {
            last_step_ms: 20,
            world_gui_started: true,
            world_loaded: Some(true),
            loading: Some(true),
            ..at(20 + 10 * QUIET_MS)
        };
        assert_eq!(gate.frame(&saving), Seen::WorldUp);
        // The player goes back to the main menu: m_game cleared.
        let closed = |now| Frame {
            last_step_ms: 50_000,
            world_gui_started: true,
            ..at(now)
        };
        assert_eq!(gate.frame(&closed(50_100)), Seen::Closing);
        assert_eq!(gate.frame(&closed(50_100 + QUIET_MS - 1)), Seen::Closing);
        assert_eq!(gate.frame(&closed(50_100 + QUIET_MS)), Seen::BackAtMenu);
        assert!(Seen::BackAtMenu.allows());
        assert_eq!(gate.frame(&closed(90_000)), Seen::BackAtMenu);
        assert!(gate.after_world());
    }

    #[test]
    fn a_load_after_a_world_blocks_and_restarts_the_quiet_stretch() {
        let mut gate = MenuGate::default();
        let up = Frame {
            last_step_ms: 100,
            world_loaded: Some(true),
            ..at(100)
        };
        assert_eq!(gate.frame(&up), Seen::WorldUp);
        // The GUI's load: the world stops first, then the game loads.
        let stopped = |now, loading| Frame {
            last_step_ms: 100,
            loading: Some(loading),
            ..at(now)
        };
        assert_eq!(gate.frame(&stopped(200, false)), Seen::Closing);
        assert_eq!(gate.frame(&stopped(300, true)), Seen::Loading);
        assert_eq!(gate.frame(&stopped(60_000, true)), Seen::Loading);
        // A moment without a task mid-load starts the stretch afresh.
        assert_eq!(gate.frame(&stopped(60_100, false)), Seen::Closing);
        assert_eq!(gate.frame(&stopped(60_200, true)), Seen::Loading);
        assert_eq!(gate.frame(&stopped(60_300, false)), Seen::Closing);
        assert_eq!(
            gate.frame(&stopped(60_300 + QUIET_MS - 1, false)),
            Seen::Closing
        );
        // The load failed back to the menu, or ended there: quiet.
        assert_eq!(
            gate.frame(&stopped(60_300 + QUIET_MS, false)),
            Seen::BackAtMenu
        );
        // It started the world after all: blocked at once.
        let again = Frame {
            last_step_ms: 100,
            world_loaded: Some(true),
            ..at(70_000)
        };
        assert_eq!(gate.frame(&again), Seen::WorldUp);
    }

    #[test]
    fn a_step_during_the_quiet_stretch_starts_it_afresh() {
        let mut gate = MenuGate::default();
        let step = |now, last| Frame {
            last_step_ms: last,
            ..at(now)
        };
        assert_eq!(gate.frame(&step(1_000, 900)), Seen::Closing);
        // The step ran after the stretch began.
        assert_eq!(gate.frame(&step(1_500, 1_400)), Seen::Closing);
        assert_eq!(
            gate.frame(&step(1_500 + QUIET_MS - 1, 1_400)),
            Seen::Closing
        );
        assert_eq!(gate.frame(&step(1_500 + QUIET_MS, 1_400)), Seen::BackAtMenu);
    }

    #[test]
    fn a_world_whose_gui_started_before_its_first_step_blocks() {
        // The owner's save for the room comes before the world's first step:
        // the GUI started, no step yet. With m_game unread, fail closed.
        let mut gate = MenuGate::default();
        let gui = Frame {
            world_gui_started: true,
            world_loaded: None,
            loading: None,
            ..at(5)
        };
        assert_eq!(gate.frame(&gui), Seen::Unknown);
        assert!(!Seen::Unknown.allows());
        // With it read, the world is loaded.
        let read = Frame {
            world_gui_started: true,
            world_loaded: Some(true),
            ..at(6)
        };
        assert_eq!(gate.frame(&read), Seen::WorldUp);
    }

    #[test]
    fn after_a_world_what_cannot_be_read_never_counts_as_the_menu() {
        let mut gate = MenuGate::default();
        let stepped = |now, world_loaded, loading| Frame {
            last_step_ms: 10,
            world_loaded,
            loading,
            ..at(now)
        };
        for now in [100, 100 + QUIET_MS, 100 + 100 * QUIET_MS] {
            assert_eq!(gate.frame(&stepped(now, None, Some(false))), Seen::Unknown);
            assert_eq!(gate.frame(&stepped(now, Some(false), None)), Seen::Unknown);
        }
        // Readable again: the stretch starts only then.
        assert_eq!(
            gate.frame(&stepped(500_000, Some(false), Some(false))),
            Seen::Closing
        );
        assert_eq!(
            gate.frame(&stepped(500_000 + QUIET_MS, Some(false), Some(false))),
            Seen::BackAtMenu
        );
    }
}
