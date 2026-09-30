//! The game's side of a session, which the hook runs on the game thread.
//!
//! Everything here is game-agnostic. The game-specific part of the hook
//! implements [`Game`] and calls the session from its detours:
//! [`Session::poll_step`] or [`Session::before_step`] before each simulation
//! step, [`Session::after_step`] after it, and [`Session::command`] when the
//! player acts. When the gate says [`StepGate::Load`], the game loads that
//! world and calls [`Session::loaded`].

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use thiserror::Error;
use tpf3mp_ipc::{IpcError, Link, Role, SendError};
use tpf3mp_proto::{
    ChatText, Cursor, Event, EventBody, IntentRejection, LaneDigest, Payload, PlayerId, RulesName,
    Speed, Text,
};

use crate::{
    BRIDGE_VERSION, BridgeError, Gate, GateError, Gated, LobbyAction, LobbyView, MAX_MESSAGE,
    RoomInfo, ToAgent, ToHook, check_version, decode, encode,
};

/// How long to sleep between polls while waiting for the agent.
const POLL: Duration = Duration::from_micros(200);
/// How often a session whose room ended before its game began looks for
/// the launcher's next room session on the link.
const PROBE: Duration = Duration::from_millis(100);

/// What the session needs from the game.
pub trait Game {
    /// Applies an event the room ordered. Called only between steps. Save
    /// events never get here: the session calls [`Game::save`] for them.
    fn apply(&mut self, event: &Event);
    /// The world's lane digests, taken at a checkpoint step or a save.
    fn lanes(&mut self) -> Vec<LaneDigest>;
    /// Saves the world as it stands, between two steps, to `file`. A save
    /// the room agrees on is what players who join later load, so it must
    /// hold everything needed to continue from here.
    fn save(&mut self, file: &Path) -> Result<(), String>;
    /// Something to show the player.
    fn notice(&mut self, notice: Notice) {
        let _ = notice;
    }
}

/// Something the game should tell the player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Speed(Speed),
    Diverged {
        step: u64,
        lanes: Vec<u16>,
    },
    /// The room refused a command; `command` is what [`Session::command`]
    /// returned for it.
    Refused {
        command: u64,
        reason: IntentRejection,
    },
    /// The session is over.
    Ended(Text<128>),
    /// A member of the room said something.
    Chat {
        from: Text<32>,
        text: ChatText,
    },
    /// The room as it stands: its name, owner and members.
    Room(RoomInfo),
    /// A member's pointer moved or their build tool is previewing.
    Cursor(Cursor),
}

/// What the game does about its next step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepGate {
    /// Run it.
    Run,
    /// Not yet: the room has not released it.
    Wait,
    /// The session is over; stop following the room.
    Ended,
    /// Replace the world with this one, then call [`Session::loaded`].
    Load(Load),
    /// Save the world as it stands into the order's file, then call
    /// [`Session::saved`]. No step runs until then. Only
    /// [`Session::poll_step`] answers this: [`Session::before_step`] saves
    /// at once through [`Game::save`].
    Save(SaveOrder),
}

/// A save the room ordered: see [`StepGate::Save`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveOrder {
    /// The save event's number, which the report names.
    pub event: u64,
    /// Where the save goes: in the directory [`Begin`] named.
    pub file: PathBuf,
}

/// A world to load: see [`ToHook::Load`](crate::ToHook::Load).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Load {
    /// The save to load, or `None` for the world every player starts from.
    pub file: Option<PathBuf>,
    /// The first step the loaded world runs: pass it to
    /// [`Session::loaded`].
    pub next_step: u64,
}

/// A game the room began.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Begin {
    /// The rules the room is played by (see `ToHook::Begin`).
    pub rules: RulesName,
    pub steps_per_second: u16,
    pub checkpoint_interval: u32,
    /// Where the game's saves go.
    pub saves: PathBuf,
    /// The local player, as the room's events name the actor.
    pub player: PlayerId,
}

#[derive(Debug, Error)]
pub enum SessionError {
    #[error(transparent)]
    Link(#[from] IpcError),
    #[error("the link failed: {0}")]
    Transfer(String),
    #[error(transparent)]
    Message(#[from] BridgeError),
    #[error(transparent)]
    Gate(#[from] GateError),
    #[error("the agent sent {0} out of place")]
    Unexpected(&'static str),
    #[error("the agent stopped responding")]
    AgentGone,
    #[error("the launcher re-created the link while the room's game was running")]
    LinkReset,
    #[error("the world was loaded to run step {got} next, but step {expected} was ordered")]
    LoadedElsewhere { expected: u64, got: u64 },
    #[error("the game loaded a world nobody ordered")]
    NotLoading,
    #[error("the game saved a world nobody ordered")]
    NotSaving,
}

/// The hook's end of one session with the agent.
pub struct Session {
    link: Link,
    gate: Gate,
    /// A load the agent ordered that the game has not finished.
    pending_load: Option<Load>,
    /// A save the room ordered that the game has not reported.
    pending_save: Option<SaveOrder>,
    saves: PathBuf,
    checkpoint_interval: u64,
    /// How long the agent's heartbeat may stand still.
    patience: Duration,
    agent_beat: (u64, Instant),
    buf: Vec<u8>,
    commands: u64,
    /// A message read ahead for a batch and left for the next step.
    peeked: Option<ToHook>,
    /// The link's name, to open it again for the launcher's next room.
    name: String,
    /// The game build, for the hello on a new link.
    build: Text<64>,
    /// The link generation the hellos were exchanged on.
    generation: u32,
    lobby: Lobby,
    /// A world that came up while the session was between rooms, told to
    /// the next room once hellos are exchanged there.
    world_between_rooms: Option<u64>,
    /// The menu arrival last told, and the link generation it was told on
    /// ([`Session::menu_up`]).
    menu_told: Option<(u32, u64)>,
    /// The launcher's lobby as last heard, until the menu's window takes it.
    lobby_view: Option<LobbyView>,
}

/// Where the session is before its room's game begins, and after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lobby {
    /// Hellos exchanged; waiting for the room to begin a game.
    Waiting,
    /// The room session ended before its game began, as when the player
    /// leaves the room in the launcher: waiting for the launcher to create
    /// the link again for its next room. `probed` is when the link was last
    /// looked for.
    Left {
        since: Instant,
        probed: Option<Instant>,
    },
    /// This side's hello went out on a new link generation; the agent's
    /// comes next.
    Greeting,
    /// The room began a game. From here on the session follows that game
    /// only, and a new link generation is refused.
    Begun,
}

impl Lobby {
    fn left() -> Self {
        Self::Left {
            since: Instant::now(),
            probed: None,
        }
    }
}

impl Session {
    /// Attaches to the agent's link, waiting up to `patience` for the agent
    /// to create it, and exchanges hellos. `build` names the game build in
    /// the agent's log.
    pub fn attach(name: &str, build: &str, patience: Duration) -> Result<Self, SessionError> {
        let deadline = Instant::now() + patience;
        let link = loop {
            match Link::open(name, Role::Hook) {
                Ok(link) => break link,
                Err(error) if Instant::now() >= deadline => return Err(error.into()),
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        };
        let build = Text::lossy(build);
        let mut session = Self {
            agent_beat: (link.peer_heartbeat(), Instant::now()),
            generation: link.session(),
            link,
            gate: Gate::new(1),
            pending_load: None,
            pending_save: None,
            saves: PathBuf::new(),
            checkpoint_interval: u64::MAX,
            patience,
            buf: vec![0; MAX_MESSAGE],
            commands: 0,
            peeked: None,
            name: name.to_owned(),
            build: build.clone(),
            lobby: Lobby::Waiting,
            world_between_rooms: None,
            menu_told: None,
            lobby_view: None,
        };
        session.send(&ToAgent::Hello {
            version: BRIDGE_VERSION,
            build,
        })?;
        match session.recv_blocking()? {
            ToHook::Hello { version } => check_version(version)?,
            _ => return Err(SessionError::Unexpected("something before its hello")),
        }
        Ok(session)
    }

    /// Waits for the room to begin a game. The first thing the gate then
    /// says is which world to load ([`StepGate::Load`]). A room that ends
    /// before its game began is followed into the launcher's next, as
    /// [`Session::try_begin`] does, for up to the session's patience.
    pub fn wait_for_begin(&mut self) -> Result<Begin, SessionError> {
        loop {
            if let Some(begin) = self.try_begin()? {
                return Ok(begin);
            }
            if let Lobby::Left { since, .. } = self.lobby
                && since.elapsed() > self.patience
            {
                return Err(SessionError::AgentGone);
            }
            std::thread::sleep(POLL);
        }
    }

    /// Without blocking: the game the room began, if the agent has said so
    /// yet. For a hook that asks from the game's own thread, which must not
    /// wait.
    ///
    /// Until then the session follows the launcher from room to room. When
    /// the room session ends before its game began (the agent says `End`,
    /// as when the player leaves the room, or the launcher re-creates the
    /// link for its next room), it waits for the link's next generation,
    /// exchanges hellos there as [`Session::attach`] does, and waits for
    /// that room's game. Once a game began, nothing of the kind is followed.
    pub fn try_begin(&mut self) -> Result<Option<Begin>, SessionError> {
        self.link.heartbeat();
        if self.lobby == Lobby::Begun {
            return Err(SessionError::Unexpected("a second game"));
        }
        if !matches!(self.lobby, Lobby::Left { .. }) && self.link.session() != self.generation {
            // Re-created in place: whatever the old room sent is gone.
            self.lobby = Lobby::left();
        }
        if matches!(self.lobby, Lobby::Left { .. }) && !self.follow()? {
            return Ok(None);
        }
        loop {
            let message = match self.try_recv() {
                Ok(Some(message)) => message,
                Ok(None) => break,
                // Re-created while this side read: follow on the next call.
                Err(SessionError::LinkReset) => {
                    self.lobby = Lobby::left();
                    return Ok(None);
                }
                Err(error) => return Err(error),
            };
            if self.lobby == Lobby::Greeting {
                let ToHook::Hello { version } = message else {
                    return Err(SessionError::Unexpected("something before its hello"));
                };
                check_version(version)?;
                self.lobby = Lobby::Waiting;
                if let Some(world) = self.world_between_rooms.take() {
                    self.send(&ToAgent::WorldUp { world })?;
                }
                continue;
            }
            match message {
                ToHook::Begin {
                    rules,
                    steps_per_second,
                    checkpoint_interval,
                    saves,
                    player,
                } => {
                    self.checkpoint_interval = u64::from(checkpoint_interval).max(1);
                    self.saves = PathBuf::from(saves.as_str());
                    self.lobby = Lobby::Begun;
                    return Ok(Some(Begin {
                        rules,
                        steps_per_second,
                        checkpoint_interval,
                        saves: self.saves.clone(),
                        player,
                    }));
                }
                // Talk in the lobby, and the lobby itself, are for the front
                // end.
                ToHook::Chat { .. } | ToHook::Room(_) => {}
                // The room session ended before its game began: the
                // launcher's next room comes on a new link generation.
                ToHook::End { .. } => {
                    self.lobby = Lobby::left();
                    return Ok(None);
                }
                ToHook::Lobby(view) => self.lobby_view = Some(view),
                _ => return Err(SessionError::Unexpected("something before the game began")),
            }
        }
        self.check_agent()?;
        Ok(None)
    }

    /// After the room session ended before its game began: looks, at most
    /// every [`PROBE`], for a new generation of the link, and on one says
    /// hello there. Returns whether the session is on a new link now.
    ///
    /// The link is opened again by name rather than read in place: on
    /// Windows the launcher re-creates the very mapping this side holds,
    /// but on Linux and macOS the old owner took the name with it, and the
    /// new link is another object.
    fn follow(&mut self) -> Result<bool, SessionError> {
        let Lobby::Left { probed, .. } = &mut self.lobby else {
            return Ok(true);
        };
        let now = Instant::now();
        if probed.is_some_and(|at| now.saturating_duration_since(at) < PROBE) {
            return Ok(false);
        }
        *probed = Some(now);
        // Not there yet, or still being created: look again later.
        let Ok(link) = Link::open(&self.name, Role::Hook) else {
            return Ok(false);
        };
        if link.session() == self.generation {
            return Ok(false);
        }
        self.generation = link.session();
        self.agent_beat = (link.peer_heartbeat(), now);
        self.link = link;
        self.peeked = None;
        self.lobby = Lobby::Greeting;
        self.send(&ToAgent::Hello {
            version: BRIDGE_VERSION,
            build: self.build.clone(),
        })?;
        self.log("the game followed the launcher into a new room session")?;
        Ok(true)
    }

    /// The world the gate ordered ([`StepGate::Load`]) is loaded, and
    /// `next_step` is the first step it will run. Beat
    /// [`Session::heartbeat`] while loading.
    pub fn loaded(&mut self, next_step: u64) -> Result<(), SessionError> {
        let Some(load) = &self.pending_load else {
            return Err(SessionError::NotLoading);
        };
        if load.next_step != next_step {
            return Err(SessionError::LoadedElsewhere {
                expected: load.next_step,
                got: next_step,
            });
        }
        self.pending_load = None;
        self.gate.loaded(next_step);
        self.send(&ToAgent::Loaded { next_step })
    }

    /// Without blocking: handles what the agent sent, applying events, and
    /// says whether the game may run its next step. For a game whose
    /// simulation shares a thread with its rendering, which must not block.
    pub fn poll_step(&mut self, game: &mut impl Game) -> Result<StepGate, SessionError> {
        self.link.heartbeat();
        loop {
            if self.gate.ended() {
                return Ok(StepGate::Ended);
            }
            if let Some(save) = &self.pending_save {
                return Ok(StepGate::Save(save.clone()));
            }
            if let Some(load) = &self.pending_load {
                return Ok(StepGate::Load(load.clone()));
            }
            if self.gate.may_run() {
                return Ok(StepGate::Run);
            }
            let Some(message) = self.try_recv()? else {
                self.check_agent()?;
                return Ok(StepGate::Wait);
            };
            self.handle(message, game)?;
        }
    }

    /// Blocks until the game may run its next step, applying events
    /// meanwhile, or must load a world. A pause can hold the game here for
    /// as long as it lasts; only an agent that stops beating ends the wait.
    pub fn before_step(&mut self, game: &mut impl Game) -> Result<StepGate, SessionError> {
        loop {
            match self.poll_step(game)? {
                StepGate::Wait => std::thread::sleep(POLL),
                StepGate::Save(order) => {
                    let outcome = game.save(&order.file);
                    self.saved(game, outcome)?;
                }
                other => return Ok(other),
            }
        }
    }

    /// The world was saved into the file [`StepGate::Save`] named, or saving
    /// failed: reports it, with the world's lanes there. The steps after it
    /// may then run. A failed save is reported too: the room decides
    /// without it.
    pub fn saved(
        &mut self,
        game: &mut impl Game,
        outcome: Result<(), String>,
    ) -> Result<(), SessionError> {
        let Some(order) = self.pending_save.take() else {
            return Err(SessionError::NotSaving);
        };
        let lanes = game.lanes();
        let file = match outcome {
            Ok(()) => match Text::new(order.file.to_string_lossy().into_owned()) {
                Ok(file) => Some(file),
                Err(error) => {
                    self.log(&format!(
                        "cannot report the save {}: {error}",
                        order.file.display()
                    ))?;
                    None
                }
            },
            Err(reason) => {
                self.log(&format!("saving the world failed: {reason}"))?;
                None
            }
        };
        self.send(&ToAgent::Saved {
            event: order.event,
            lanes,
            file,
        })
    }

    /// After [`Session::poll_step`] said [`StepGate::Run`]: how many steps
    /// from the next the game may run as one batch, at most `max`, before it
    /// calls [`Session::after_step`] once for each (see [`Gate::batch`]).
    ///
    /// The agent releases steps one message at a time, so this reads on
    /// while the messages waiting are releases or things to show the
    /// player, and stops at the first that must wait for the next step (an
    /// event, a load, the end), which the next [`Session::poll_step`] reads.
    pub fn batch(&mut self, game: &mut impl Game, max: u32) -> Result<u32, SessionError> {
        let max = u64::from(max);
        loop {
            let steps = self.gate.batch(max, self.checkpoint_interval);
            // Cut by the cap or a checkpoint: more releases change nothing.
            if steps == 0 || steps >= max || steps < self.gate.released_ahead() {
                return Ok(u32::try_from(steps).unwrap_or(u32::MAX));
            }
            let Some(message) = self.try_recv()? else {
                return Ok(u32::try_from(steps).unwrap_or(u32::MAX));
            };
            match message {
                ToHook::Release { .. }
                | ToHook::Speed(_)
                | ToHook::Chat { .. }
                | ToHook::Room(_)
                | ToHook::Lobby(_)
                | ToHook::Refused { .. }
                | ToHook::Diverged { .. }
                | ToHook::Cursor(_) => self.handle(message, game)?,
                other => {
                    self.peeked = Some(other);
                    return Ok(u32::try_from(steps).unwrap_or(u32::MAX));
                }
            }
        }
    }

    /// Records that the game ran its next step and reports it, with the
    /// world's lanes at checkpoint steps. Returns the step.
    pub fn after_step(&mut self, game: &mut impl Game) -> Result<u64, SessionError> {
        let step = self.gate.ran()?;
        self.send(&ToAgent::Ran { step })?;
        if step % self.checkpoint_interval == 0 {
            let lanes = game.lanes();
            self.send(&ToAgent::Checkpoint { step, lanes })?;
        }
        Ok(step)
    }

    /// Hands a player's action to the room. Returns its number, which a
    /// refusal names.
    pub fn command(&mut self, payload: Payload) -> Result<u64, SessionError> {
        let number = self.commands;
        self.send(&ToAgent::Command { payload })?;
        self.commands += 1;
        Ok(number)
    }

    /// Asks the room to run at the speed the player picked.
    pub fn request_speed(&mut self, speed: Speed) -> Result<(), SessionError> {
        self.send(&ToAgent::Speed { speed })
    }

    /// Before the room begins a game: the game's world number `world` is up,
    /// with the mod linked (see [`ToAgent::WorldUp`]). The agent marks the
    /// player ready in the room's lobby. Once the room began a game, the
    /// worlds are the room's to order, and nothing is told: returns whether
    /// the agent was.
    ///
    /// Between rooms (the last room ended before its game began, and the
    /// next room's hellos are not exchanged yet) the world is kept and told
    /// to the next room once they are: its link is the one the agent reads.
    pub fn world_up(&mut self, world: u64) -> Result<bool, SessionError> {
        match self.lobby {
            Lobby::Begun => Ok(false),
            Lobby::Left { .. } | Lobby::Greeting => {
                self.world_between_rooms = Some(world);
                Ok(false)
            }
            Lobby::Waiting => {
                self.send(&ToAgent::WorldUp { world })?;
                Ok(true)
            }
        }
    }

    /// Before the room begins a game: the game is at its main menu, arrived
    /// there for the `menu`th time (see [`ToAgent::MenuUp`]). Cheap to call
    /// every frame: each arrival is told once per room session, including
    /// to the launcher's next room when the player moves on while the game
    /// stays at its menu. Between rooms nothing is told; the next call once
    /// the next room's hellos are exchanged tells it. Returns whether the
    /// agent was told now.
    pub fn menu_up(&mut self, menu: u64) -> Result<bool, SessionError> {
        if self.lobby != Lobby::Waiting || self.menu_told == Some((self.generation, menu)) {
            return Ok(false);
        }
        self.send(&ToAgent::MenuUp { menu })?;
        self.menu_told = Some((self.generation, menu));
        Ok(true)
    }

    /// Says something to the room for the player.
    pub fn chat(&mut self, text: ChatText) -> Result<(), SessionError> {
        self.send(&ToAgent::Chat { text })
    }

    /// Hands the agent the player's pointer or preview position to broadcast
    /// as an advisory datagram.
    pub fn cursor(&mut self, cursor: Cursor) -> Result<(), SessionError> {
        self.send(&ToAgent::Cursor(cursor))
    }

    /// Hands the launcher an action the player took in the main menu's
    /// Multiplayer window (D17).
    pub fn lobby_act(&mut self, action: LobbyAction) -> Result<(), SessionError> {
        self.send(&ToAgent::Lobby(action))
    }

    /// The launcher's lobby, if the agent sent a new one since the last
    /// call. The session keeps it from whatever read it: the step gate, the
    /// wait for the room's game, or [`Session::poll_lobby`].
    pub fn take_lobby(&mut self) -> Option<LobbyView> {
        self.lobby_view.take()
    }

    /// Without blocking, for a game at its main menu, where no step reads
    /// the link: reads what the agent sent before the room's game, keeping
    /// the lobby. It stops at the first message the game must see at its
    /// gate (the room's `Begin`, or its end), which stays for
    /// [`Session::try_begin`]. After the room's game ended nothing more is
    /// followed, and only the lobby is kept. Between rooms, and while the
    /// hellos on the next room's link are exchanged, [`Session::try_begin`]
    /// alone reads the link.
    pub fn poll_lobby(&mut self) -> Result<(), SessionError> {
        self.link.heartbeat();
        let ended = self.gate.ended();
        let reads = match self.lobby {
            Lobby::Waiting => true,
            Lobby::Begun => ended,
            Lobby::Left { .. } | Lobby::Greeting => false,
        };
        if !reads || self.peeked.is_some() {
            return Ok(());
        }
        while let Some(message) = self.try_recv()? {
            match message {
                ToHook::Lobby(view) => self.lobby_view = Some(view),
                ToHook::Chat { .. } | ToHook::Room(_) => {}
                _ if ended => {}
                other => {
                    self.peeked = Some(other);
                    break;
                }
            }
        }
        Ok(())
    }

    /// The step the game runs next.
    pub fn next_step(&self) -> u64 {
        self.gate.next_step()
    }

    /// Tells the agent this side is alive. Call it while loading.
    pub fn heartbeat(&self) {
        self.link.heartbeat();
    }

    /// Puts a line in the agent's log.
    pub fn log(&mut self, message: &str) -> Result<(), SessionError> {
        self.send(&ToAgent::Log {
            message: Text::lossy(message),
        })
    }

    fn handle(&mut self, message: ToHook, game: &mut impl Game) -> Result<(), SessionError> {
        match self.gate.on_message(message)? {
            Gated::Apply(event) if matches!(event.body, EventBody::Save) => {
                self.pending_save = Some(SaveOrder {
                    event: event.seq,
                    file: self.saves.join(format!("save-{}.sav", event.seq)),
                });
            }
            Gated::Apply(event) => game.apply(&event),
            Gated::Load { file, next_step } => {
                self.pending_load = Some(Load {
                    file: file.map(|file| PathBuf::from(file.as_str())),
                    next_step,
                });
            }
            Gated::Speed(speed) => game.notice(Notice::Speed(speed)),
            Gated::Diverged { step, lanes } => game.notice(Notice::Diverged { step, lanes }),
            Gated::Refused { command, reason } => {
                game.notice(Notice::Refused { command, reason });
            }
            Gated::Ended(reason) => game.notice(Notice::Ended(reason)),
            Gated::Chat { from, text } => game.notice(Notice::Chat { from, text }),
            Gated::Room(room) => game.notice(Notice::Room(room)),
            Gated::Lobby(view) => self.lobby_view = Some(view),
            Gated::Cursor(cursor) => game.notice(Notice::Cursor(cursor)),
            Gated::Nothing => {}
        }
        Ok(())
    }

    fn send(&mut self, message: &ToAgent) -> Result<(), SessionError> {
        let bytes = encode(message)?;
        loop {
            self.link.heartbeat();
            match self.link.send(&bytes) {
                Ok(()) => return Ok(()),
                Err(SendError::Full) => {
                    self.check_agent()?;
                    std::thread::sleep(POLL);
                }
                Err(error) => return Err(SessionError::Transfer(error.to_string())),
            }
        }
    }

    fn try_recv(&mut self) -> Result<Option<ToHook>, SessionError> {
        if let Some(message) = self.peeked.take() {
            return Ok(Some(message));
        }
        // The rings were reset under this side: what they hold now is
        // another session's, never this one's.
        if self.link.session() != self.generation {
            return Err(SessionError::LinkReset);
        }
        match self.link.recv_into(&mut self.buf) {
            Ok(Some(len)) => Ok(Some(decode(&self.buf[..len])?)),
            Ok(None) => Ok(None),
            Err(error) => Err(SessionError::Transfer(error.to_string())),
        }
    }

    fn recv_blocking(&mut self) -> Result<ToHook, SessionError> {
        loop {
            self.link.heartbeat();
            if let Some(message) = self.try_recv()? {
                return Ok(message);
            }
            self.check_agent()?;
            std::thread::sleep(POLL);
        }
    }

    fn check_agent(&mut self) -> Result<(), SessionError> {
        let beat = self.link.peer_heartbeat();
        let now = Instant::now();
        if beat != self.agent_beat.0 {
            self.agent_beat = (beat, now);
        } else if now.saturating_duration_since(self.agent_beat.1) > self.patience {
            return Err(SessionError::AgentGone);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tpf3mp_ipc::Config;
    use tpf3mp_proto::{FixedBytes, PlayerId};

    use super::*;

    #[derive(Default)]
    struct World {
        applied: Vec<u64>,
        notices: Vec<Notice>,
    }

    impl Game for World {
        fn apply(&mut self, event: &Event) {
            self.applied.push(event.step);
        }
        fn lanes(&mut self) -> Vec<LaneDigest> {
            Vec::new()
        }
        fn save(&mut self, _file: &Path) -> Result<(), String> {
            Err("no saves here".into())
        }
        fn notice(&mut self, notice: Notice) {
            self.notices.push(notice);
        }
    }

    fn event(step: u64) -> Event {
        Event {
            seq: step,
            step,
            body: EventBody::PlayerLeft {
                player: PlayerId(FixedBytes([1; 32])),
                kicked: false,
            },
        }
    }

    /// A session over a real link, with the agent's end, past the hellos
    /// and the load of the world every player starts from.
    fn playing(tag: &str, checkpoint_interval: u32) -> (Session, Link, World) {
        let (mut session, agent, _) = lobby(tag);
        say(&agent, &begin(checkpoint_interval));
        say(
            &agent,
            &ToHook::Load {
                file: None,
                next_step: 1,
            },
        );
        assert!(session.try_begin().unwrap().is_some());
        let mut world = World::default();
        assert!(matches!(
            session.poll_step(&mut world).unwrap(),
            StepGate::Load(_)
        ));
        session.loaded(1).unwrap();
        (session, agent, world)
    }

    /// A session over a real link, with the agent's end and the link's
    /// name, past the hellos and waiting for the room to begin a game.
    fn lobby(tag: &str) -> (Session, Link, String) {
        let name = format!("test.session.{tag}.{}", std::process::id());
        let agent = Link::create(&Config::new(name.clone()), Role::Agent).unwrap();
        say(&agent, &agent_hello());
        let mut session = Session::attach(&name, "test", Duration::from_secs(10)).unwrap();
        assert_eq!(hook_hello(&agent), "test");
        assert!(session.try_begin().unwrap().is_none());
        (session, agent, name)
    }

    fn agent_hello() -> ToHook {
        ToHook::Hello {
            version: BRIDGE_VERSION,
        }
    }

    fn begin(checkpoint_interval: u32) -> ToHook {
        ToHook::Begin {
            rules: RulesName::new("native").unwrap(),
            steps_per_second: 5,
            checkpoint_interval,
            saves: Text::lossy("saves"),
            player: PlayerId(FixedBytes([1; 32])),
        }
    }

    fn say(agent: &Link, message: &ToHook) {
        agent.send(&encode(message).unwrap()).unwrap();
    }

    /// What the hook said next, if anything.
    fn heard_now(agent: &Link) -> Option<ToAgent> {
        let mut buf = vec![0; MAX_MESSAGE];
        let len = agent.recv_into(&mut buf).unwrap()?;
        Some(decode(&buf[..len]).unwrap())
    }

    /// The hook's hello, which must be the next thing it said: its build.
    fn hook_hello(agent: &Link) -> String {
        match heard_now(agent) {
            Some(ToAgent::Hello { version, build }) => {
                assert_eq!(version, BRIDGE_VERSION);
                build.as_str().to_owned()
            }
            other => panic!("expected the hook's hello, got {other:?}"),
        }
    }

    /// Polls the session before its game, as the game's step does, until
    /// the hook says something on `agent`'s link.
    fn until_the_hook_speaks(session: &mut Session, agent: &Link) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(session.try_begin().unwrap().is_none(), "no game yet");
            if agent.next_len().is_some() {
                return hook_hello(agent);
            }
            assert!(Instant::now() < deadline, "the hook never said hello");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Hellos exchanged on the new link: the room's game then begins there.
    fn begins_on(session: &mut Session, agent: &Link) {
        say(agent, &agent_hello());
        say(
            agent,
            &ToHook::Chat {
                from: Text::lossy("host"),
                text: ChatText::new("welcome").unwrap(),
            },
        );
        say(agent, &begin(10));
        let begun = session.try_begin().unwrap().expect("the new room's game");
        assert_eq!(begun.checkpoint_interval, 10);
    }

    #[test]
    fn a_room_left_before_its_game_is_followed_into_the_launchers_next() {
        let (mut session, agent, name) = lobby("left");
        // The player leaves the room in the launcher: the room session
        // tells the game it is over, and the launcher lets go of the link.
        say(
            &agent,
            &ToHook::End {
                reason: Text::lossy("Left"),
            },
        );
        assert!(session.try_begin().unwrap().is_none(), "no failing");
        drop(agent);
        assert!(session.try_begin().unwrap().is_none(), "no room yet");

        // The player creates another room: a new link generation.
        let agent = Link::create(&Config::new(name), Role::Agent).unwrap();
        assert_eq!(until_the_hook_speaks(&mut session, &agent), "test");
        assert!(matches!(heard_now(&agent), Some(ToAgent::Log { .. })));
        begins_on(&mut session, &agent);
    }

    /// A save loaded while the player is between rooms is told to the next
    /// room once its hellos are exchanged, so that room marks them ready.
    #[test]
    fn a_world_up_between_rooms_is_told_to_the_next() {
        let (mut session, agent, name) = lobby("between");
        say(
            &agent,
            &ToHook::End {
                reason: Text::lossy("Left"),
            },
        );
        assert!(session.try_begin().unwrap().is_none());
        drop(agent);
        assert!(!session.world_up(7).unwrap(), "no room to tell");

        let agent = Link::create(&Config::new(name), Role::Agent).unwrap();
        assert_eq!(until_the_hook_speaks(&mut session, &agent), "test");
        assert!(matches!(heard_now(&agent), Some(ToAgent::Log { .. })));
        say(&agent, &agent_hello());
        assert!(session.try_begin().unwrap().is_none());
        assert!(
            matches!(heard_now(&agent), Some(ToAgent::WorldUp { world: 7 })),
            "the kept world, once the new room said hello"
        );
        assert!(heard_now(&agent).is_none(), "once");
    }

    /// A game at its main menu is told once per arrival and room session:
    /// again to the launcher's next room, never between rooms or once the
    /// room's game began.
    #[test]
    fn a_menu_arrival_is_told_once_per_room_session() {
        let (mut session, agent, name) = lobby("menu");
        assert!(session.menu_up(1).unwrap());
        assert_eq!(heard_now(&agent), Some(ToAgent::MenuUp { menu: 1 }));
        assert!(!session.menu_up(1).unwrap(), "every frame, told once");
        assert!(heard_now(&agent).is_none());
        say(
            &agent,
            &ToHook::End {
                reason: Text::lossy("Left"),
            },
        );
        assert!(session.try_begin().unwrap().is_none());
        drop(agent);
        assert!(!session.menu_up(1).unwrap(), "no room to tell");

        let agent = Link::create(&Config::new(name), Role::Agent).unwrap();
        assert_eq!(until_the_hook_speaks(&mut session, &agent), "test");
        assert!(matches!(heard_now(&agent), Some(ToAgent::Log { .. })));
        assert!(!session.menu_up(1).unwrap(), "the new room's hello first");
        say(&agent, &agent_hello());
        assert!(session.try_begin().unwrap().is_none());
        assert!(session.menu_up(1).unwrap(), "the same arrival, a new room");
        assert_eq!(heard_now(&agent), Some(ToAgent::MenuUp { menu: 1 }));
        say(&agent, &begin(10));
        assert!(session.try_begin().unwrap().is_some());
        assert!(!session.menu_up(2).unwrap(), "the room's game began");
        assert!(heard_now(&agent).is_none());
    }

    #[test]
    fn a_link_re_created_in_the_lobby_is_followed() {
        let (mut session, old, name) = lobby("recreated");
        // The launcher's next room re-creates the link over the old one,
        // before the game read the old room's end.
        let agent = Link::create(&Config::new(name), Role::Agent).unwrap();
        assert_eq!(until_the_hook_speaks(&mut session, &agent), "test");
        begins_on(&mut session, &agent);
        drop(old);
    }

    #[test]
    fn the_new_room_must_say_hello_first() {
        let (mut session, old, name) = lobby("nohello");
        let agent = Link::create(&Config::new(name), Role::Agent).unwrap();
        until_the_hook_speaks(&mut session, &agent);
        say(&agent, &begin(10));
        assert!(matches!(
            session.try_begin(),
            Err(SessionError::Unexpected(_))
        ));
        drop(old);
    }

    #[test]
    fn a_second_hello_on_the_same_link_is_still_refused() {
        let (mut session, agent, _) = lobby("stray");
        say(&agent, &agent_hello());
        assert!(matches!(
            session.try_begin(),
            Err(SessionError::Unexpected(_))
        ));
    }

    #[test]
    fn a_link_re_created_during_the_game_is_refused() {
        let (mut session, old, mut world) = playing("midgame", 50);
        let name = format!("test.session.midgame.{}", std::process::id());
        let agent = Link::create(&Config::new(name), Role::Agent).unwrap();
        say(&agent, &agent_hello());
        say(&agent, &ToHook::Release { through: 1 });
        assert!(matches!(
            session.poll_step(&mut world),
            Err(SessionError::LinkReset)
        ));
        assert!(matches!(
            session.try_begin(),
            Err(SessionError::Unexpected(_))
        ));
        drop(old);
    }

    /// A session past the hellos, before any room's game, with the agent's
    /// end.
    fn at_the_menu(tag: &str) -> (Session, Link) {
        let name = format!("test.session.{tag}.{}", std::process::id());
        let agent = Link::create(&Config::new(name.clone()), Role::Agent).unwrap();
        say(
            &agent,
            &ToHook::Hello {
                version: BRIDGE_VERSION,
            },
        );
        let session = Session::attach(&name, "test", Duration::from_secs(10)).unwrap();
        (session, agent)
    }

    fn lobby_view(name: &str) -> LobbyView {
        LobbyView {
            name: Text::new(name).unwrap(),
            ..LobbyView::default()
        }
    }

    #[test]
    fn at_the_menu_the_lobby_is_read_and_the_rooms_game_waits_for_its_gate() {
        let (mut session, agent) = at_the_menu("menu-lobby");
        assert_eq!(session.take_lobby(), None);
        say(&agent, &ToHook::Lobby(lobby_view("first")));
        say(&agent, &ToHook::Lobby(lobby_view("second")));
        say(&agent, &begin(50));
        say(&agent, &ToHook::Lobby(lobby_view("after the begin")));
        say(
            &agent,
            &ToHook::Load {
                file: None,
                next_step: 1,
            },
        );
        session.poll_lobby().unwrap();
        assert_eq!(
            session.take_lobby(),
            Some(lobby_view("second")),
            "the newest"
        );
        assert_eq!(session.take_lobby(), None, "once");
        // The room's Begin was left where it was: the game takes it at its
        // gate, and the lobby behind it too.
        session.poll_lobby().unwrap();
        assert_eq!(session.take_lobby(), None);
        assert!(session.try_begin().unwrap().is_some());
        let mut world = World::default();
        assert!(matches!(
            session.poll_step(&mut world).unwrap(),
            StepGate::Load(_)
        ));
        assert_eq!(session.take_lobby(), Some(lobby_view("after the begin")));
        // Once the game began, only its gate reads the link.
        say(&agent, &ToHook::Lobby(lobby_view("in the game")));
        session.poll_lobby().unwrap();
        assert_eq!(session.take_lobby(), None);
        session.loaded(1).unwrap();
        say(&agent, &ToHook::Release { through: 1 });
        assert_eq!(session.poll_step(&mut world).unwrap(), StepGate::Run);
        assert_eq!(session.take_lobby(), Some(lobby_view("in the game")));
        assert!(world.notices.is_empty(), "the lobby is not the game's");
    }

    #[test]
    fn after_the_rooms_game_ended_the_menu_still_hears_the_lobby() {
        let (mut session, agent, mut world) = playing("ended-lobby", 50);
        say(
            &agent,
            &ToHook::End {
                reason: Text::lossy("over"),
            },
        );
        assert_eq!(session.poll_step(&mut world).unwrap(), StepGate::Ended);
        say(&agent, &begin(50));
        say(&agent, &ToHook::Lobby(lobby_view("next room")));
        session.poll_lobby().unwrap();
        assert_eq!(session.take_lobby(), Some(lobby_view("next room")));
    }

    #[test]
    fn the_players_lobby_actions_go_to_the_agent() {
        let (mut session, agent) = at_the_menu("lobby-act");
        session.lobby_act(LobbyAction::Start).unwrap();
        assert_eq!(heard(&agent), ToAgent::Lobby(LobbyAction::Start));
    }

    #[test]
    fn a_batch_reads_the_releases_ahead_and_stops_at_an_event() {
        let (mut session, agent, mut world) = playing("batch", 50);
        for through in 1..=3 {
            say(&agent, &ToHook::Release { through });
        }
        say(&agent, &ToHook::Speed(Speed(200)));
        say(&agent, &ToHook::Apply(event(4)));
        say(&agent, &ToHook::Release { through: 4 });

        assert_eq!(session.poll_step(&mut world).unwrap(), StepGate::Run);
        assert_eq!(
            session.batch(&mut world, 16).unwrap(),
            3,
            "steps 1 to 3, released one message at a time"
        );
        assert_eq!(world.notices, vec![Notice::Speed(Speed(200))]);
        assert!(world.applied.is_empty(), "step 4's event waits for step 4");
        for step in 1..=3 {
            assert_eq!(session.after_step(&mut world).unwrap(), step);
        }
        assert_eq!(session.poll_step(&mut world).unwrap(), StepGate::Run);
        assert_eq!(world.applied, vec![4], "the event read ahead, applied now");
        assert_eq!(session.batch(&mut world, 16).unwrap(), 1);
    }

    #[test]
    fn a_batch_stops_at_the_cap_and_at_a_checkpoint() {
        let (mut session, agent, mut world) = playing("cap", 5);
        for through in 1..=12 {
            say(&agent, &ToHook::Release { through });
        }
        assert_eq!(session.poll_step(&mut world).unwrap(), StepGate::Run);
        assert_eq!(session.batch(&mut world, 3).unwrap(), 3, "the cap");
        assert_eq!(session.batch(&mut world, 16).unwrap(), 5, "to checkpoint 5");
        for _ in 0..5 {
            session.after_step(&mut world).unwrap();
        }
        assert_eq!(session.poll_step(&mut world).unwrap(), StepGate::Run);
        assert_eq!(session.batch(&mut world, 16).unwrap(), 5, "6 to 10");
        for _ in 0..5 {
            session.after_step(&mut world).unwrap();
        }
        assert_eq!(session.poll_step(&mut world).unwrap(), StepGate::Run);
        assert_eq!(session.batch(&mut world, 16).unwrap(), 2, "11 and 12");
    }

    /// The next message the hook sent, past the hellos and loads.
    fn heard(agent: &Link) -> ToAgent {
        let mut buf = vec![0; MAX_MESSAGE];
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(len) = agent.recv_into(&mut buf).unwrap() {
                let message: ToAgent = decode(&buf[..len]).unwrap();
                if !matches!(message, ToAgent::Hello { .. } | ToAgent::Loaded { .. }) {
                    return message;
                }
            }
            assert!(Instant::now() < deadline, "the hook said nothing");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_world_up_is_told_in_the_lobby_and_not_once_the_room_began() {
        let name = format!("test.session.world-up.{}", std::process::id());
        let agent = Link::create(&Config::new(name.clone()), Role::Agent).unwrap();
        say(
            &agent,
            &ToHook::Hello {
                version: BRIDGE_VERSION,
            },
        );
        let mut session = Session::attach(&name, "test", Duration::from_secs(10)).unwrap();
        assert!(session.world_up(1).unwrap());
        assert_eq!(heard(&agent), ToAgent::WorldUp { world: 1 });
        say(
            &agent,
            &ToHook::Begin {
                rules: RulesName::new("native").unwrap(),
                steps_per_second: 5,
                checkpoint_interval: 50,
                saves: Text::lossy("saves"),
                player: PlayerId(FixedBytes([1; 32])),
            },
        );
        assert!(session.try_begin().unwrap().is_some());
        assert!(!session.world_up(2).unwrap(), "the room's game began");
        session.log("after").unwrap();
        assert!(
            matches!(heard(&agent), ToAgent::Log { .. }),
            "no world up once the room began"
        );
    }

    #[test]
    fn a_save_holds_the_steps_until_the_game_reports_it() {
        let (mut session, agent, mut world) = playing("save", 50);
        say(
            &agent,
            &ToHook::Apply(Event {
                seq: 5,
                step: 1,
                body: EventBody::Save,
            }),
        );
        say(&agent, &ToHook::Release { through: 1 });
        let order = match session.poll_step(&mut world).unwrap() {
            StepGate::Save(order) => order,
            other => panic!("expected a save, got {other:?}"),
        };
        assert_eq!(order.event, 5);
        assert!(
            order.file.ends_with("save-5.sav"),
            "{}",
            order.file.display()
        );
        // Not saved yet: still the save, and no step.
        assert_eq!(
            session.poll_step(&mut world).unwrap(),
            StepGate::Save(order.clone())
        );
        session.saved(&mut world, Ok(())).unwrap();
        match heard(&agent) {
            ToAgent::Saved { event, file, .. } => {
                assert_eq!(event, 5);
                assert_eq!(file.unwrap().as_str(), order.file.to_string_lossy());
            }
            other => panic!("expected the save's report, got {other:?}"),
        }
        assert_eq!(session.poll_step(&mut world).unwrap(), StepGate::Run);
        assert!(matches!(
            session.saved(&mut world, Ok(())),
            Err(SessionError::NotSaving)
        ));
    }

    #[test]
    fn a_failed_save_is_reported_without_a_file_and_the_game_goes_on() {
        let (mut session, agent, mut world) = playing("failed-save", 50);
        say(
            &agent,
            &ToHook::Apply(Event {
                seq: 9,
                step: 1,
                body: EventBody::Save,
            }),
        );
        say(&agent, &ToHook::Release { through: 1 });
        assert!(matches!(
            session.poll_step(&mut world).unwrap(),
            StepGate::Save(_)
        ));
        session
            .saved(&mut world, Err("the disk is full".into()))
            .unwrap();
        // The failure goes to the agent's log, then the report.
        let mut report = heard(&agent);
        if matches!(report, ToAgent::Log { .. }) {
            report = heard(&agent);
        }
        assert!(
            matches!(
                report,
                ToAgent::Saved {
                    event: 9,
                    file: None,
                    ..
                }
            ),
            "{report:?}"
        );
        assert_eq!(session.poll_step(&mut world).unwrap(), StepGate::Run);
    }

    #[test]
    fn before_step_saves_at_once() {
        let (mut session, agent, mut world) = playing("before-step-save", 50);
        say(
            &agent,
            &ToHook::Apply(Event {
                seq: 3,
                step: 1,
                body: EventBody::Save,
            }),
        );
        say(&agent, &ToHook::Release { through: 1 });
        assert_eq!(session.before_step(&mut world).unwrap(), StepGate::Run);
        let mut report = heard(&agent);
        if matches!(report, ToAgent::Log { .. }) {
            report = heard(&agent);
        }
        // The test world refuses to save: reported without a file.
        assert!(
            matches!(
                report,
                ToAgent::Saved {
                    event: 3,
                    file: None,
                    ..
                }
            ),
            "{report:?}"
        );
    }
}
