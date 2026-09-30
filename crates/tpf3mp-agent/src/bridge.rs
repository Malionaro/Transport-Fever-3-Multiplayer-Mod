//! Connects a game, through its hook, to a room.
//!
//! Turns from the server become the messages the hook's
//! [`Gate`](tpf3mp_bridge::Gate) expects, released at the pace
//! [`Playout`] sets. What the hook reports becomes intents, progress,
//! checkpoints and saves for the room.
//!
//! A turn stream may start from a world the room agreed on (a player
//! joining a running game, one who could no longer resume, one rebased
//! after diverging). The bridge then fetches that world, has the game load
//! it, and only then lets the stream's turns through.

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use thiserror::Error;
use tokio::{
    sync::{mpsc, watch},
    task::AbortHandle,
};
use tpf3mp_bridge::{
    BridgeError, LobbyAction, LobbyView, MAX_MESSAGE, MAX_PATH, RoomInfo, RoomMember, ToAgent,
    ToHook, check_version, decode, encode,
};
use tpf3mp_net::close;
use tpf3mp_proto::{
    BoundedVec, ChatText, ContentDiff, ContentManifest, Event, EventBody, Invite, JoinRoom,
    LaneDigest, MAX_ROOM_MEMBERS, PlayerId, Request, RequestError, Resume, RoomPhase, RoomView,
    SavedWorld, SessionId, SnapshotId, Speed, Text, WorldOffer,
};
use tpf3mp_snapshot::ManifestId;
use tracing::{debug, info, warn};

use crate::{
    Action, Client, ClientError, ClientEvent, ConnectOptions, Events, FollowError, Playout,
    TurnFollower, Worlds, connect, transfer,
};

/// The agent's end of the link to the hook.
pub trait HookLink: Send {
    /// Sends one encoded message. `Ok(false)` means the hook is not reading
    /// fast enough; the bridge tries again later.
    fn send(&mut self, message: &[u8]) -> Result<bool, BridgeFault>;
    /// Receives one encoded message into `buf`, if one is waiting.
    fn recv(&mut self, buf: &mut Vec<u8>) -> Result<bool, BridgeFault>;
    /// Signals that the agent is alive.
    fn heartbeat(&mut self);
    /// The hook's heartbeat counter, which advances while it is alive.
    fn peer_heartbeat(&self) -> u64;
}

impl HookLink for tpf3mp_ipc::Link {
    fn send(&mut self, message: &[u8]) -> Result<bool, BridgeFault> {
        match tpf3mp_ipc::Link::send(self, message) {
            Ok(()) => Ok(true),
            Err(tpf3mp_ipc::SendError::Full) => Ok(false),
            Err(error) => Err(BridgeFault::Link(error.to_string())),
        }
    }

    fn recv(&mut self, buf: &mut Vec<u8>) -> Result<bool, BridgeFault> {
        buf.resize(MAX_MESSAGE, 0);
        match self.recv_into(buf) {
            Ok(Some(len)) => {
                buf.truncate(len);
                Ok(true)
            }
            Ok(None) => Ok(false),
            Err(error) => Err(BridgeFault::Link(error.to_string())),
        }
    }

    fn heartbeat(&mut self) {
        tpf3mp_ipc::Link::heartbeat(self);
    }

    fn peer_heartbeat(&self) -> u64 {
        tpf3mp_ipc::Link::peer_heartbeat(self)
    }
}

#[derive(Debug, Clone)]
pub struct BridgeOptions {
    /// Playout margin: each step plays this long after its seal arrives.
    pub playout_margin: Duration,
    /// How long a late arrival keeps the playout buffer grown.
    pub playout_memory: Duration,
    /// How often the hook's messages are read.
    pub poll: Duration,
    /// A hook whose heartbeat stands still this long while the game runs is
    /// gone.
    pub hook_timeout: Duration,
    /// The same, while the game loads its world, which can take minutes.
    pub load_timeout: Duration,
    /// The least time between two progress reports to the server.
    pub progress_every: Duration,
    /// Where worlds are kept. Without it, saves are reported as failed and
    /// a world the room offers cannot be loaded.
    pub worlds: Option<Worlds>,
    /// What a front end shows of the session, kept up to date by the
    /// bridge.
    pub status: Option<SharedStatus>,
    /// The launcher's lobby, for the game's main-menu window (D17).
    pub lobby: Option<LobbyLink>,
    /// A save of the player's own that the room's game starts from, when
    /// this player owns the room: the bridge hands it to the room in the
    /// lobby (`Request::StartWorld`), and every game, this one too, loads
    /// it from its main menu when the game starts. Without it, the owner's
    /// game has the world up and saves it for the room once the game began.
    pub start_world: Option<PathBuf>,
}

/// The launcher's lobby as a bridge passes it on: the lobby to show the
/// game's main-menu window, and where the window's actions go (D17).
#[derive(Debug, Clone)]
pub struct LobbyLink {
    pub views: watch::Receiver<LobbyView>,
    pub actions: mpsc::UnboundedSender<LobbyAction>,
}

impl Default for BridgeOptions {
    fn default() -> Self {
        Self {
            playout_margin: Duration::from_millis(20),
            playout_memory: Duration::from_secs(10),
            poll: Duration::from_millis(2),
            hook_timeout: Duration::from_secs(60),
            load_timeout: Duration::from_secs(600),
            progress_every: Duration::from_millis(20),
            worlds: None,
            status: None,
            lobby: None,
            start_world: None,
        }
    }
}

/// What a front end asks of the room session a bridge runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    Ready(bool),
    Start,
    Speed(Speed),
    Kick(PlayerId),
    Chat(ChatText),
    /// Leave the room, which ends the session.
    Leave,
    /// The game the front end started has exited. Once its hook attached,
    /// this ends the session as a hook that stopped responding does
    /// ([`BridgeFault::GameClosed`]), without waiting out the heartbeat
    /// limits, so the player can start the game again at once. Before
    /// then the game never joined, and the session waits for the next one.
    GameClosed,
}

/// Chat lines and notices a status keeps.
const STATUS_HISTORY: usize = 100;

/// What a front end shows of the room session a bridge runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// The room as last announced.
    pub room: Option<RoomView>,
    /// The game's build, once its hook attached.
    pub game: Option<String>,
    pub world: WorldStatus,
    /// The last step the game ran.
    pub step: Option<u64>,
    pub speed: Speed,
    /// The latest chat, oldest first.
    pub chat: VecDeque<(PlayerId, ChatText)>,
    /// What the player should know, oldest first: divergences, refusals,
    /// rejoins.
    pub notices: VecDeque<String>,
    /// How this player's game differs from the room's, while it does.
    pub content_diff: Option<ContentDiff>,
    /// The server's name for the current connection, which its log uses:
    /// what a player quotes to the server's operator.
    pub session: Option<SessionId>,
    /// The server speaks a newer protocol: this client must update to
    /// play there.
    pub outdated: bool,
    /// The operator's latest notice, such as a restart coming.
    pub announcement: Option<String>,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            room: None,
            game: None,
            world: WorldStatus::None,
            step: None,
            speed: Speed::NORMAL,
            chat: VecDeque::new(),
            notices: VecDeque::new(),
            content_diff: None,
            session: None,
            outdated: false,
            announcement: None,
        }
    }
}

impl Status {
    pub fn notice(&mut self, notice: impl Into<String>) {
        push_bounded(&mut self.notices, notice.into());
    }

    /// Takes the operator's notice `text`: the latest shown on its own, and
    /// every one among the notices.
    pub fn announce(&mut self, text: &str) {
        self.announcement = Some(text.to_owned());
        self.notice(format!("from the server: {text}"));
    }
}

/// Where the game's world stands, for display.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorldStatus {
    /// No game has begun.
    #[default]
    None,
    /// Fetching the world to load: bytes present of the whole.
    Fetching { bytes: u64, total: u64 },
    /// The game is loading its world.
    Loading,
    /// The game plays the room's world.
    Playing,
}

/// A status shared between a bridge and a front end.
pub type SharedStatus = Arc<Mutex<Status>>;

fn push_bounded<T>(list: &mut VecDeque<T>, item: T) {
    if list.len() == STATUS_HISTORY {
        list.pop_front();
    }
    list.push_back(item);
}

#[derive(Debug, Error)]
pub enum BridgeFault {
    #[error("the link to the hook failed: {0}")]
    Link(String),
    #[error(transparent)]
    Message(#[from] BridgeError),
    #[error("the hook sent {0} out of place")]
    Unexpected(&'static str),
    #[error("the hook stopped responding")]
    HookGone,
    #[error("Transport Fever 3 closed")]
    GameClosed,
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("the server broke a turn invariant: {0}")]
    Follow(#[from] FollowError),
    #[error("lost the server and could not rejoin: {0}")]
    Rejoin(String),
    #[error("the game loaded its world to run step {got} next, but step {expected} was ordered")]
    LoadedElsewhere { expected: u64, got: u64 },
    #[error("the room sent a world to load, but this agent keeps no worlds")]
    NoWorlds,
    #[error("the path {0} is too long for the link to the game")]
    PathTooLong(PathBuf),
}

/// How a bridged session ended.
#[derive(Debug)]
pub enum BridgeEnd {
    /// The connection to the server closed.
    Closed(quinn::ConnectionError),
    /// The room's owner removed this player.
    Kicked,
    /// The client's events ended.
    EventsEnded,
    /// The world to load could not be fetched. Rejoining gets a new offer.
    WorldUnavailable,
    /// The player left the room.
    Left,
}

/// Where the game's world stands with respect to the turn stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum World {
    /// The game runs the stream's world.
    Ready,
    /// The stream starts from this world, which is being fetched. Nothing
    /// reaches the game until it has loaded it.
    Fetching {
        offer: WorldOffer,
        next_step: u64,
        attempt: u64,
    },
    /// The game is loading a world, after which it runs `next_step`.
    Loading { next_step: u64 },
}

/// Work the bridge handed off, finished.
#[derive(Debug)]
enum Done {
    Ingested {
        event: u64,
        lanes: Vec<LaneDigest>,
        result: Result<(ManifestId, SavedWorld), String>,
    },
    Fetched {
        attempt: u64,
        result: Result<(PathBuf, ManifestId), String>,
    },
    Uploaded {
        snapshot: SnapshotId,
        result: Result<u64, String>,
    },
    /// The save the room starts from is cut into the store.
    StartCut {
        result: Result<(ManifestId, SavedWorld), String>,
    },
    /// The room would not take the save it was to start from.
    StartRefused { error: String },
}

/// Where the save the room starts from stands, when this player hands one
/// over (see [`BridgeOptions::start_world`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartWorld {
    /// None to hand over, or no longer: the room's game began, or handing
    /// it over failed and the room starts as without one.
    None,
    /// Named, waiting for the room's lobby with this player as its owner.
    Named,
    /// Being cut into the store.
    Cutting,
    /// Told to the room, which asks for it and receives it.
    Told(SnapshotId),
    /// The room has it: every game loads it when the game starts.
    Uploaded(SnapshotId),
}

/// Saves of this game the store keeps, newest last: the room asks for the
/// newest, and the one before may still be in flight.
const SAVES_KEPT: usize = 2;

/// Couples one game's hook to one client.
pub struct Bridge<L> {
    link: L,
    options: BridgeOptions,
    follower: Option<TurnFollower>,
    playout: Option<Playout>,
    /// Messages for the hook, in order, sent as the hook takes them.
    outbox: Outbox,
    hook_ready: bool,
    begun: bool,
    world: World,
    /// Whether the game has loaded a world since the last load was ordered.
    loaded: bool,
    /// The room's speed as the hook was last told it; none before the
    /// first turn.
    speed: Option<Speed>,
    /// Commands the hook has sent; numbers each one's intent.
    commands: u64,
    /// The last step the game ran, or before any, the step before its
    /// first: the progress to report.
    progress: Option<u64>,
    /// The progress last reported on the current connection.
    reported: Option<u64>,
    last_report: Instant,
    wait_until: Option<Instant>,
    hook_beat: (u64, Instant),
    buf: Vec<u8>,
    done_tx: mpsc::UnboundedSender<Done>,
    done_rx: mpsc::UnboundedReceiver<Done>,
    /// Counts fetches, so a result that arrives after a newer offer is
    /// ignored.
    attempts: u64,
    fetch: Option<AbortHandle>,
    /// Snapshots of the game's own saves, newest last.
    saved: VecDeque<ManifestId>,
    /// The world last received.
    received: Option<ManifestId>,
    /// The save the room starts from, when this player hands one over.
    start: StartWorld,
    /// Its snapshot, kept in the store for the upload and for this game's
    /// own load of it.
    start_kept: Option<ManifestId>,
    /// What a front end asks of the session.
    controls: Option<mpsc::Receiver<Control>>,
    /// The room as the server last showed it: the game's Multiplayer window
    /// shows it, and chat names its members.
    room: Option<RoomView>,
    /// The room's phase as last announced, if it was.
    room_phase: Option<RoomPhase>,
    /// The latest world the game said is up ([`ToAgent::WorldUp`]), or 0.
    world_up: u64,
    /// The latest world the player's readiness was decided for.
    readied: u64,
    /// The room's owner as last announced, if it was.
    room_owner: Option<PlayerId>,
    /// The latest menu arrival the game told ([`ToAgent::MenuUp`]), or 0.
    menu_up: u64,
    /// The latest menu arrival the player's readiness was decided for.
    menu_readied: u64,
    /// The game's build, once its hook said hello.
    build: Option<String>,
}

impl<L: HookLink> Bridge<L> {
    pub fn new(link: L, options: BridgeOptions) -> Self {
        let now = Instant::now();
        let (done_tx, done_rx) = mpsc::unbounded_channel();
        Self {
            hook_beat: (link.peer_heartbeat(), now),
            link,
            follower: None,
            playout: None,
            outbox: Outbox::default(),
            hook_ready: false,
            begun: false,
            world: World::Ready,
            loaded: false,
            speed: None,
            commands: 0,
            progress: None,
            reported: None,
            last_report: now,
            wait_until: None,
            buf: Vec::new(),
            done_tx,
            done_rx,
            attempts: 0,
            fetch: None,
            saved: VecDeque::new(),
            received: None,
            start: if options.start_world.is_some() {
                StartWorld::Named
            } else {
                StartWorld::None
            },
            start_kept: None,
            controls: None,
            room: None,
            room_phase: options.status.as_ref().and_then(|status| {
                let status = status.lock().unwrap_or_else(PoisonError::into_inner);
                status.room.as_ref().map(|room| room.phase)
            }),
            world_up: 0,
            readied: 0,
            room_owner: options.status.as_ref().and_then(|status| {
                let status = status.lock().unwrap_or_else(PoisonError::into_inner);
                status.room.as_ref().map(|room| room.owner)
            }),
            menu_up: 0,
            menu_readied: 0,
            options,
            build: None,
        }
    }

    /// Takes over a link whose hook has already said hello, as `build`, and
    /// was answered (the launcher's lobby link, `launcher::lobby`).
    pub fn greeted(mut self, build: &str) -> Self {
        self.hook_ready = true;
        self.build = Some(build.to_owned());
        self.status(|status| status.game = Some(build.to_owned()));
        self
    }

    /// Gives the link back, with the game's build if its hook said hello:
    /// the launcher keeps it for the game's next room.
    pub fn into_link(self) -> (L, Option<String>) {
        let build = self.build.filter(|_| self.hook_ready);
        (self.link, build)
    }

    /// Queues the launcher's lobby for the hook when it changed: only the
    /// newest waits to go.
    fn lobby_news(&mut self) {
        let Some(lobby) = &mut self.options.lobby else {
            return;
        };
        if !lobby.views.has_changed().unwrap_or(false) {
            return;
        }
        let view = lobby.views.borrow_and_update().clone();
        self.outbox
            .retain(|message| !matches!(message, ToHook::Lobby(_)));
        self.outbox.push_back(ToHook::Lobby(view));
    }

    /// The player acted in the game's main-menu window: the launcher does
    /// it.
    fn lobby_action(&self, action: LobbyAction) {
        match &self.options.lobby {
            Some(lobby) => {
                let _ = lobby.actions.send(action);
            }
            None => debug!(?action, "a lobby action with no launcher to take it"),
        }
    }

    /// Takes a front end's requests (ready, start, speed, chat, leave) from
    /// `controls` while the session runs.
    pub fn with_controls(mut self, controls: mpsc::Receiver<Control>) -> Self {
        self.controls = Some(controls);
        self
    }

    /// Updates the front end's view of the session, if it has one.
    fn status(&self, update: impl FnOnce(&mut Status)) {
        if let Some(status) = &self.options.status {
            update(&mut status.lock().unwrap_or_else(PoisonError::into_inner));
        }
    }

    /// Runs the session on one connection until it closes or something
    /// breaks. Waits for the hook to attach first; the room may start before
    /// it. The bridge keeps its state, so after a lost connection it can run
    /// again on a new one that resumes the room (see [`play`]). The hook
    /// is not told the session ended; [`Bridge::end`] does that.
    pub async fn run(
        &mut self,
        client: &Client,
        events: &mut Events,
    ) -> Result<BridgeEnd, BridgeFault> {
        loop {
            let now = Instant::now();
            self.link.heartbeat();
            self.check_hook(now)?;
            self.read_hook(client).await?;
            self.hand_over_start_world(client);
            self.report_progress(client, now).await?;
            self.lobby_news();
            // Turns wait while the world they continue is being fetched.
            if !matches!(self.world, World::Fetching { .. })
                && let (Some(follower), Some(playout)) = (&mut self.follower, &mut self.playout)
            {
                self.wait_until = pump(follower, playout, now, &mut self.outbox);
            }
            self.flush()?;
            let poll_at = now + self.options.poll;
            let wake = self.wait_until.map_or(poll_at, |at| at.min(poll_at));
            // While the hook falls behind, the room's turns wait in the
            // client, and past its bound on the server's stream.
            let taking = !self.outbox.is_full();
            tokio::select! {
                event = events.recv(), if taking => {
                    let Some(event) = event else {
                        return Ok(BridgeEnd::EventsEnded);
                    };
                    if let Some(end) = self.on_event(event, client)? {
                        return Ok(end);
                    }
                }
                Some(done) = self.done_rx.recv() => {
                    if let Some(end) = self.on_done(done, client).await? {
                        return Ok(end);
                    }
                }
                Some(control) = next_control(&mut self.controls) => {
                    if let Some(end) = self.on_control(control, client).await? {
                        return Ok(end);
                    }
                }
                () = tokio::time::sleep_until(wake.into()) => {}
            }
        }
    }

    /// Tells the hook the session is over, as far as the link still takes
    /// messages. A room left before its game began has nothing for the game
    /// to end: it stays as it was, the launcher's lobby link keeps going,
    /// and the game can follow the player into the next room.
    pub fn end(&mut self, reason: &str) {
        if let Some(fetch) = self.fetch.take() {
            fetch.abort();
        }
        if !self.begun {
            self.outbox
                .retain(|message| matches!(message, ToHook::Lobby(_)));
            let _ = self.flush();
            return;
        }
        self.outbox.push_back(ToHook::End {
            reason: Text::lossy(reason),
        });
        let _ = self.flush();
    }

    /// Keeps the hook waiting while the agent has no connection: without a
    /// beat it would give up on the agent.
    pub fn keep_alive(&mut self) {
        self.link.heartbeat();
    }

    /// Where to resume the room after reconnecting. `None` before the first
    /// turn stream, and while the world a stream starts from is still being
    /// fetched: the room then offers a world again.
    pub fn resume_point(&self) -> Option<Resume> {
        if matches!(self.world, World::Fetching { .. }) {
            return None;
        }
        self.follower.as_ref().map(TurnFollower::resume_point)
    }

    fn check_hook(&mut self, now: Instant) -> Result<(), BridgeFault> {
        let beat = self.link.peer_heartbeat();
        let limit = if self.loaded {
            self.options.hook_timeout
        } else {
            self.options.load_timeout
        };
        if beat != self.hook_beat.0 {
            self.hook_beat = (beat, now);
        } else if self.hook_ready && now.saturating_duration_since(self.hook_beat.1) > limit {
            return Err(BridgeFault::HookGone);
        }
        Ok(())
    }

    async fn read_hook(&mut self, client: &Client) -> Result<(), BridgeFault> {
        while self.link.recv(&mut self.buf)? {
            let message: ToAgent = decode(&self.buf)?;
            if !self.hook_ready && !matches!(message, ToAgent::Hello { .. }) {
                return Err(BridgeFault::Unexpected("a message before its hello"));
            }
            // What the game reports of a world that is being replaced
            // describes nothing the room plays.
            let current = self.world == World::Ready;
            match message {
                ToAgent::Hello { version, build } => {
                    if self.hook_ready {
                        return Err(BridgeFault::Unexpected("a second hello"));
                    }
                    check_version(version)?;
                    info!(%build, "the game's hook attached");
                    self.hook_ready = true;
                    self.build = Some(build.as_str().to_owned());
                    self.status(|status| status.game = Some(build.as_str().to_owned()));
                    // The hello goes first, ahead of a game that began
                    // before the hook attached.
                    self.outbox.push_front(ToHook::Hello {
                        version: tpf3mp_bridge::BRIDGE_VERSION,
                    });
                }
                ToAgent::Loaded { next_step } => {
                    let World::Loading {
                        next_step: expected,
                    } = self.world
                    else {
                        return Err(BridgeFault::Unexpected("a world nobody ordered"));
                    };
                    if next_step != expected {
                        return Err(BridgeFault::LoadedElsewhere {
                            expected,
                            got: next_step,
                        });
                    }
                    self.world = World::Ready;
                    self.status(|status| status.world = WorldStatus::Playing);
                    // Loaded counts as progress: the server holds the room
                    // until every member has loaded.
                    let progress = next_step.saturating_sub(1);
                    client.report_progress(progress).await?;
                    self.progress = Some(progress);
                    self.reported = Some(progress);
                    self.loaded = true;
                }
                ToAgent::Command { payload } => {
                    client.send_intent(self.commands, payload).await?;
                    self.commands += 1;
                }
                ToAgent::Ran { step } if current => {
                    self.progress = Some(step);
                    self.status(|status| status.step = Some(step));
                }
                ToAgent::Checkpoint { step, lanes } if current => {
                    client.report_checkpoint(step, lanes).await?;
                }
                ToAgent::Saved { event, lanes, file } if current => {
                    self.saved_world(event, lanes, file, client).await?;
                }
                ToAgent::Ran { .. } | ToAgent::Checkpoint { .. } | ToAgent::Saved { .. } => {}
                ToAgent::Chat { text } => self.request(client, Request::Chat(text)),
                // The game's speed row: the room's owner sets the room's
                // speed from it; anyone else's is refused, as a notice.
                ToAgent::Speed { speed } => self.request(client, Request::SetSpeed(speed)),
                ToAgent::WorldUp { world } => self.world_up(world, client),
                ToAgent::MenuUp { menu } => self.menu_up(menu, client),
                ToAgent::Log { message } => info!(hook = %message),
                ToAgent::Lobby(action) => self.lobby_action(action),
                ToAgent::Cursor(cursor) => {
                    let _ = client.send_advisory(&tpf3mp_proto::Datagram::Cursor(cursor));
                }
            }
        }
        Ok(())
    }

    /// The game has its world number `world` up, with the mod linked: in
    /// the room's lobby, the player is marked ready, as by the Ready button.
    /// Once a world: a player who then says Not ready stays so until another
    /// world is up. Never once the room's game began, nor while its world
    /// is being replaced; while the room's phase is not known yet, it waits
    /// for the room's announcement (fail closed).
    fn world_up(&mut self, world: u64, client: &Client) {
        if world <= self.world_up {
            debug!(world, "a world already told up");
            return;
        }
        self.world_up = world;
        self.ready_for_world(client);
    }

    /// Marks the player ready for the latest world up, if not done for it
    /// and the room is in its lobby. See [`Bridge::world_up`].
    fn ready_for_world(&mut self, client: &Client) {
        let world = self.world_up;
        if world <= self.readied || self.start_world_on_its_way(client) {
            return;
        }
        let lobby = match self.room_phase {
            // Not known yet: the room's announcement decides.
            None if !self.begun => return,
            Some(RoomPhase::Lobby) => !self.begun && self.world == World::Ready,
            _ => false,
        };
        // Decided for this world, either way.
        self.readied = world;
        if !lobby {
            debug!(world, "the game's world is up outside the room's lobby");
            return;
        }
        info!(world, "the game's world is up with the mod linked: ready");
        self.status(|status| {
            status.notice("your game has its world up: you are marked ready");
        });
        self.request(client, Request::SetReady(true));
    }

    /// The game is at its main menu, arrived there for the `menu`th time,
    /// and can load the room's world from there: in the room's lobby, a
    /// player other than the room's owner is marked ready, as by the Ready
    /// button, once per arrival. So is the owner once the room has the save
    /// the owner handed over to start from ([`BridgeOptions::start_world`]):
    /// the owner's game then loads it from the menu as every other does.
    /// Without one the owner is not: the room's first world is the owner's,
    /// which their game saves for the room from a world it has up. Nor is
    /// anyone whose agent keeps no worlds, as it could not fetch the room's.
    /// Otherwise as [`Bridge::world_up`].
    fn menu_up(&mut self, menu: u64, client: &Client) {
        if menu <= self.menu_up {
            debug!(menu, "a menu arrival already told");
            return;
        }
        self.menu_up = menu;
        self.ready_at_menu(client);
    }

    /// Marks the player ready for the latest menu arrival, if not decided
    /// for it and the room is in its lobby. See [`Bridge::menu_up`].
    fn ready_at_menu(&mut self, client: &Client) {
        let menu = self.menu_up;
        if menu <= self.menu_readied || self.start_world_on_its_way(client) {
            return;
        }
        let lobby = match self.room_phase {
            None if !self.begun => return,
            Some(RoomPhase::Lobby) => !self.begun && self.world == World::Ready,
            _ => false,
        };
        // Decided for this arrival, either way.
        self.menu_readied = menu;
        if !lobby {
            debug!(menu, "the game is at its menu outside the room's lobby");
            return;
        }
        // The room's announcement names its owner along with its phase;
        // without one, nobody is marked (fail closed).
        let Some(owner) = self.room_owner else {
            return;
        };
        if owner == client.player() && !matches!(self.start, StartWorld::Uploaded(_)) {
            info!(
                menu,
                "the game is at its main menu, but the room plays its owner's world: load it to be ready"
            );
            self.status(|status| {
                status.notice("load the world the room will play: your game saves it for the room");
            });
            return;
        }
        if self.options.worlds.is_none() {
            debug!(
                menu,
                "the game is at its main menu, but this agent keeps no worlds to hand it the room's"
            );
            return;
        }
        info!(
            menu,
            "the game is at its main menu and loads the room's world when the game starts: ready"
        );
        self.status(|status| {
            status.notice(
                "your game waits at its main menu for the room's world: you are marked ready",
            );
        });
        self.request(client, Request::SetReady(true));
    }

    /// Whether this player owns the room and the save it starts from is
    /// still on its way there: readiness waits for it, undecided, since the
    /// room cannot start before it arrives.
    fn start_world_on_its_way(&self, client: &Client) -> bool {
        matches!(
            self.start,
            StartWorld::Named | StartWorld::Cutting | StartWorld::Told(_)
        ) && self.room_owner == Some(client.player())
            && !self.begun
    }

    /// Hands the room the save it starts from, once in the room's lobby
    /// with this player as its owner: cut into the store off this task,
    /// then told to the room (see [`Bridge::on_done`]), which asks for it.
    /// Once the room's game began, the room plays what it has.
    fn hand_over_start_world(&mut self, client: &Client) {
        if self.start != StartWorld::Named {
            return;
        }
        if self.begun {
            self.start = StartWorld::None;
            return;
        }
        if self.room_phase != Some(RoomPhase::Lobby) || self.room_owner != Some(client.player()) {
            return;
        }
        let (Some(worlds), Some(file)) = (
            self.options.worlds.clone(),
            self.options.start_world.clone(),
        ) else {
            warn!("a save to start the room from, but this agent keeps no worlds to hand it over");
            self.start = StartWorld::None;
            return;
        };
        info!(file = %file.display(), "handing the room the save it starts from");
        self.status(|status| {
            status.notice(format!(
                "the room starts from your save {}: handing it over",
                file.display()
            ));
        });
        self.start = StartWorld::Cutting;
        let done = self.done_tx.clone();
        tokio::task::spawn_blocking(move || {
            let result = worlds
                .ingest_copy(&file)
                .map(|(manifest, world)| (manifest.id(), world))
                .map_err(|error| format!("cannot read the save {}: {error}", file.display()));
            let _ = done.send(Done::StartCut { result });
        });
    }

    /// The game saved its world at a save event: cut the save into the
    /// store off this task, then report it (see [`Bridge::on_done`]).
    async fn saved_world(
        &mut self,
        event: u64,
        lanes: Vec<LaneDigest>,
        file: Option<Text<MAX_PATH>>,
        client: &Client,
    ) -> Result<(), BridgeFault> {
        let file = file.map(|file| PathBuf::from(file.as_str()));
        let (Some(worlds), Some(file)) = (self.options.worlds.clone(), file) else {
            client.report_saved(event, lanes, None).await?;
            return Ok(());
        };
        let done = self.done_tx.clone();
        tokio::task::spawn_blocking(move || {
            // The hook saves where it was told. Any other file it names is
            // the player's, not the agent's to take in and delete.
            let result = if within(&file, worlds.saves()) {
                worlds
                    .ingest(&file)
                    .map(|(manifest, world)| (manifest.id(), world))
                    .map_err(|error| format!("cannot keep the save {}: {error}", file.display()))
            } else {
                Err(format!(
                    "the game reported a save outside {}: not taken",
                    worlds.saves().display()
                ))
            };
            let _ = done.send(Done::Ingested {
                event,
                lanes,
                result,
            });
        });
        Ok(())
    }

    /// Reports how far the game has run, at most every `progress_every`,
    /// and at once on a new connection, which knows nothing yet.
    async fn report_progress(&mut self, client: &Client, now: Instant) -> Result<(), BridgeFault> {
        let Some(progress) = self.progress else {
            return Ok(());
        };
        let Some(reported) = self.reported else {
            client.report_progress(progress).await?;
            self.reported = Some(progress);
            self.last_report = now;
            return Ok(());
        };
        let due = now.saturating_duration_since(self.last_report) >= self.options.progress_every;
        if due && progress > reported {
            client.report_progress(progress).await?;
            self.reported = Some(progress);
            self.last_report = now;
        }
        Ok(())
    }

    /// Handles one event from the server. Returns how the session ended, if
    /// it did.
    fn on_event(
        &mut self,
        event: ClientEvent,
        client: &Client,
    ) -> Result<Option<BridgeEnd>, BridgeFault> {
        match event {
            ClientEvent::TurnStream(start) => {
                // A new stream, perhaps on a new connection: tell it where
                // the game stands.
                self.reported = None;
                self.playout = Some(Playout::new(
                    start.steps_per_second,
                    self.options.playout_margin,
                    self.options.playout_memory,
                ));
                if !self.begun {
                    self.begun = true;
                    let saves = self.saves_dir();
                    self.outbox.push_back(ToHook::Begin {
                        rules: start.rules.clone(),
                        steps_per_second: start.steps_per_second,
                        checkpoint_interval: start.checkpoint_interval,
                        saves: path_text(&saves)?,
                        player: client.player(),
                    });
                    if let Some(room) = &self.room {
                        self.outbox.push_back(ToHook::Room(room_info(room)));
                    }
                }
                let next_step = start.sealed_through.saturating_add(1);
                match (start.world, &mut self.follower) {
                    (Some(offer), _) => {
                        self.follower = Some(TurnFollower::new(&start));
                        self.fetch_world(offer, next_step, client)?;
                    }
                    (None, None) => {
                        // A new game: every player loads the world it starts
                        // from.
                        self.follower = Some(TurnFollower::new(&start));
                        self.order_load(None, next_step)?;
                    }
                    (None, Some(follower)) => match follower.restart(&start) {
                        Ok(()) => {}
                        // The game from its first turn, from a server that
                        // keeps no worlds: start over from the first world.
                        Err(_) if start.next_event == 1 && start.sealed_through == 0 => {
                            self.follower = Some(TurnFollower::new(&start));
                            self.order_load(None, next_step)?;
                        }
                        Err(error) => return Err(error.into()),
                    },
                }
            }
            ClientEvent::Turn(turn) => {
                let follower = self
                    .follower
                    .as_mut()
                    .ok_or(BridgeFault::Unexpected("a turn before its stream"))?;
                follower.accept(turn)?;
                if let Some(playout) = &mut self.playout {
                    playout.on_turn(follower.sealed_through(), follower.speed(), Instant::now());
                }
                if let Some(speed) = speed_news(&mut self.speed, follower.speed()) {
                    self.outbox.push_back(ToHook::Speed(speed));
                    self.status(|status| status.speed = speed);
                }
            }
            ClientEvent::IntentRejected { client_seq, reason } => {
                self.outbox.push_back(ToHook::Refused {
                    command: client_seq,
                    reason,
                });
                self.status(|status| {
                    status.notice(format!("the room refused an action: {reason:?}"))
                });
            }
            ClientEvent::Diverged { step, lanes } => {
                self.status(|status| {
                    status.notice(format!(
                        "your world differed from the room's at step {step}; the room's replaces it"
                    ));
                });
                self.outbox.push_back(ToHook::Diverged { step, lanes });
            }
            ClientEvent::Upload { event, snapshot } => self.upload(event, snapshot, client),
            ClientEvent::Chat { from, text } => {
                // The game hears chat once its session began; the front end
                // hears all of it.
                if self.begun {
                    let name = self.name_of(&from);
                    self.outbox.push_back(ToHook::Chat {
                        from: name,
                        text: text.clone(),
                    });
                }
                self.status(|status| push_bounded(&mut status.chat, (from, text)));
            }
            ClientEvent::RoomUpdate(room) => {
                // The game's Multiplayer window shows the room once the game
                // began.
                if self.begun {
                    self.outbox.push_back(ToHook::Room(room_info(&room)));
                }
                self.room = Some(room.clone());
                self.room_phase = Some(room.phase);
                self.room_owner = Some(room.owner);
                self.ready_for_world(client);
                self.ready_at_menu(client);
                self.status(|status| status.room = Some(room));
            }
            ClientEvent::Notice(text) => self.status(|status| status.announce(text.as_str())),
            ClientEvent::ContentDiff(diff) => self.status(|status| {
                if let Some(diff) = &diff {
                    status.notice(format!("your game differs from the room's: {diff}"));
                }
                status.content_diff = diff;
            }),
            ClientEvent::Advisory(datagram) => match datagram {
                tpf3mp_proto::Datagram::Cursor(cursor) => {
                    self.outbox.retain(|message| match message {
                        ToHook::Cursor(c) => c.player != cursor.player,
                        _ => true,
                    });
                    self.outbox.push_back(ToHook::Cursor(cursor));
                }
            },
            ClientEvent::Kicked => return Ok(Some(BridgeEnd::Kicked)),
            ClientEvent::Closed(reason) => return Ok(Some(BridgeEnd::Closed(reason))),
        }
        Ok(None)
    }

    /// Handles work that finished off the bridge's task.
    async fn on_done(
        &mut self,
        done: Done,
        client: &Client,
    ) -> Result<Option<BridgeEnd>, BridgeFault> {
        match done {
            Done::Ingested {
                event,
                lanes,
                result,
            } => {
                let world = match result {
                    Ok((id, world)) => {
                        self.saved.push_back(id);
                        while self.saved.len() > SAVES_KEPT {
                            self.saved.pop_front();
                        }
                        self.tidy();
                        Some(world)
                    }
                    Err(error) => {
                        warn!(%error, "a save of the game could not be kept");
                        None
                    }
                };
                client.report_saved(event, lanes, world).await?;
            }
            Done::Fetched { attempt, result } => {
                let World::Fetching {
                    next_step,
                    attempt: current,
                    ..
                } = self.world
                else {
                    return Ok(None);
                };
                if attempt != current {
                    return Ok(None);
                }
                self.fetch = None;
                match result {
                    Ok((file, id)) => {
                        info!(file = %file.display(), "fetched the world to load");
                        self.received = Some(id);
                        self.tidy();
                        self.order_load(Some(&file), next_step)?;
                    }
                    Err(error) => {
                        warn!(%error, "fetching the world to load failed");
                        return Ok(Some(BridgeEnd::WorldUnavailable));
                    }
                }
            }
            Done::Uploaded { snapshot, result } => {
                let starting = self.start == StartWorld::Told(snapshot);
                match result {
                    Ok(bytes) if starting => {
                        info!(%snapshot, bytes, "uploaded the save the room starts from; the game waits at its menu to load it with everyone");
                        self.start = StartWorld::Uploaded(snapshot);
                        self.status(|status| {
                            status.notice(
                                "the room has your save: every game loads it from its main menu when the game starts",
                            );
                        });
                        self.ready_for_world(client);
                        self.ready_at_menu(client);
                    }
                    Ok(bytes) => info!(%snapshot, bytes, "uploaded a save the room asked for"),
                    Err(error) if starting => {
                        self.start_failed(&format!("uploading it failed: {error}"), client);
                    }
                    Err(error) => warn!(%snapshot, %error, "uploading a save failed"),
                }
            }
            Done::StartCut { result } => {
                if self.start != StartWorld::Cutting {
                    return Ok(None);
                }
                match result {
                    Ok((id, world)) => {
                        info!(snapshot = %world.snapshot, bytes = world.size, "told the room the save it starts from");
                        self.start_kept = Some(id);
                        self.start = StartWorld::Told(world.snapshot);
                        self.tell_start_world(client, world);
                    }
                    Err(error) => self.start_failed(&error, client),
                }
            }
            Done::StartRefused { error } => {
                if matches!(self.start, StartWorld::Told(_)) {
                    self.start_failed(&format!("the room refused it: {error}"), client);
                }
            }
        }
        Ok(None)
    }

    /// Tells the room the save it starts from, on a task of its own: the
    /// room asks for it with [`ClientEvent::Upload`].
    fn tell_start_world(&self, client: &Client, world: SavedWorld) {
        let requests = client.requests();
        let done = self.done_tx.clone();
        tokio::spawn(async move {
            if let Err(error) = requests.done(Request::StartWorld(world)).await {
                let _ = done.send(Done::StartRefused {
                    error: error.to_string(),
                });
            }
        });
    }

    /// Handing over the save the room starts from failed: the room starts
    /// as without one, from the owner's world up.
    fn start_failed(&mut self, why: &str, client: &Client) {
        warn!(reason = why, "the room cannot start from the save named");
        self.start = StartWorld::None;
        self.status(|status| {
            status.notice(format!(
                "the room cannot start from your save ({why}); load the world the room will play instead"
            ));
        });
        // Readiness waited for the save; it is decided as without one now.
        self.ready_for_world(client);
        self.ready_at_menu(client);
    }

    /// Starts fetching the world a stream starts from. Whatever the game
    /// was sent for the world it replaces is void.
    fn fetch_world(
        &mut self,
        offer: WorldOffer,
        next_step: u64,
        client: &Client,
    ) -> Result<(), BridgeFault> {
        let worlds = self.options.worlds.clone().ok_or(BridgeFault::NoWorlds)?;
        self.void_world();
        self.attempts += 1;
        let attempt = self.attempts;
        self.world = World::Fetching {
            offer,
            next_step,
            attempt,
        };
        info!(snapshot = %offer.snapshot, bytes = offer.size, "fetching the world to load");
        let total = offer.size;
        self.status(|status| status.world = WorldStatus::Fetching { bytes: 0, total });
        let opener = client.bulk();
        let done = self.done_tx.clone();
        let status = self.options.status.clone();
        let task = tokio::spawn(async move {
            let progress = |progress: tpf3mp_snapshot::Progress| {
                if let Some(status) = &status {
                    let mut status = status.lock().unwrap_or_else(PoisonError::into_inner);
                    status.world = WorldStatus::Fetching {
                        bytes: progress.bytes_present,
                        total: progress.bytes_total,
                    };
                }
            };
            let result = transfer::fetch_world(&opener, &worlds, offer, progress)
                .await
                .map(|(file, manifest)| (file, manifest.id()))
                .map_err(|error| error.to_string());
            let _ = done.send(Done::Fetched { attempt, result });
        });
        self.fetch = Some(task.abort_handle());
        Ok(())
    }

    /// Has the game load a world, then run `next_step`.
    fn order_load(&mut self, file: Option<&Path>, next_step: u64) -> Result<(), BridgeFault> {
        let file = file.map(path_text).transpose()?;
        self.void_world();
        self.outbox.push_back(ToHook::Load { file, next_step });
        self.world = World::Loading { next_step };
        self.status(|status| status.world = WorldStatus::Loading);
        Ok(())
    }

    /// Carries out a front end's request. Requests go out on a task of their
    /// own, so a round trip never holds up the game; leaving ends the
    /// session once the room has let the player go, and the game closing
    /// ends it as a fault.
    async fn on_control(
        &mut self,
        control: Control,
        client: &Client,
    ) -> Result<Option<BridgeEnd>, BridgeFault> {
        let request = match control {
            Control::Ready(ready) => Request::SetReady(ready),
            Control::Start => Request::StartGame,
            Control::Speed(speed) => Request::SetSpeed(speed),
            Control::Kick(player) => Request::Kick(player),
            Control::Chat(text) => Request::Chat(text),
            Control::Leave => {
                if let Err(error) = client.leave_room().await {
                    debug!(%error, "leaving the room failed; ending the session anyway");
                }
                return Ok(Some(BridgeEnd::Left));
            }
            // A game whose hook attached is the one this session plays
            // through: without it the session cannot go on, and a new game's
            // hook could not attach to it.
            Control::GameClosed if self.hook_ready => return Err(BridgeFault::GameClosed),
            Control::GameClosed => {
                info!("the game closed before its hook attached; waiting for the next one");
                return Ok(None);
            }
        };
        self.request(client, request);
        Ok(None)
    }

    /// Sends a request on a task of its own; a refusal becomes a notice.
    fn request(&self, client: &Client, request: Request) {
        let requests = client.requests();
        let status = self.options.status.clone();
        tokio::spawn(async move {
            if let Err(error) = requests.done(request).await
                && let Some(status) = status
            {
                status
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .notice(error.to_string());
            }
        });
    }

    /// A member's name, as the room last announced it.
    fn name_of(&self, player: &PlayerId) -> Text<32> {
        let own = self.room.as_ref().and_then(|room| {
            room.members
                .iter()
                .find(|member| member.player == *player)
                .map(|member| member.name.clone())
        });
        if let Some(name) = own {
            return name;
        }
        let known = self.options.status.as_ref().and_then(|status| {
            let status = status.lock().unwrap_or_else(PoisonError::into_inner);
            let room = status.room.as_ref()?;
            let member = room
                .members
                .iter()
                .find(|member| member.player == *player)?;
            Some(member.name.clone())
        });
        known.unwrap_or_else(|| Text::lossy(&player.to_string()))
    }

    /// Forgets what the game was sent for its current world and how far it
    /// got: a new one replaces it.
    fn void_world(&mut self) {
        if let Some(fetch) = self.fetch.take() {
            fetch.abort();
        }
        self.outbox.retain(|message| {
            !matches!(
                message,
                ToHook::Apply(_) | ToHook::Release { .. } | ToHook::Load { .. }
            )
        });
        self.progress = None;
        self.loaded = false;
    }

    /// Uploads a save the room asked for.
    fn upload(&mut self, event: u64, snapshot: SnapshotId, client: &Client) {
        let Some(worlds) = self.options.worlds.clone() else {
            warn!(
                event,
                "the room asked for a save, but this agent keeps no worlds"
            );
            return;
        };
        let opener = client.bulk();
        let done = self.done_tx.clone();
        tokio::spawn(async move {
            let result = transfer::upload_world(&opener, &worlds, snapshot)
                .await
                .map(|served| served.bytes)
                .map_err(|error| error.to_string());
            let _ = done.send(Done::Uploaded { snapshot, result });
        });
    }

    /// Keeps only the snapshots still useful: the game's newest saves and
    /// the world last received. Off the bridge's task.
    fn tidy(&self) {
        let Some(worlds) = self.options.worlds.clone() else {
            return;
        };
        let keep: Vec<ManifestId> = self
            .saved
            .iter()
            .copied()
            .chain(self.received)
            .chain(self.start_kept)
            .collect();
        tokio::task::spawn_blocking(move || {
            if let Err(error) = worlds.keep_only(&keep) {
                debug!(%error, "cannot tidy the world store");
            }
        });
    }

    /// Where the game writes its saves.
    fn saves_dir(&self) -> PathBuf {
        match &self.options.worlds {
            Some(worlds) => worlds.saves().to_owned(),
            None => std::env::temp_dir().join("tpf3mp-saves"),
        }
    }

    /// Sends what the hook will take now, in order. Nothing goes out before
    /// the hook's hello.
    fn flush(&mut self) -> Result<(), BridgeFault> {
        if !self.hook_ready {
            return Ok(());
        }
        while let Some(message) = self.outbox.front() {
            let bytes = encode(message)?;
            if !self.link.send(&bytes)? {
                break;
            }
            self.outbox.pop_front();
        }
        Ok(())
    }
}

/// Whether `file` is a file inside `dir`, links followed.
fn within(file: &Path, dir: &Path) -> bool {
    match (file.canonicalize(), dir.canonicalize()) {
        (Ok(file), Ok(dir)) => file != dir && file.starts_with(dir),
        _ => false,
    }
}

/// Messages waiting for the hook, and about how many bytes they hold.
///
/// A hook that does not read, as before it attaches or while the game loads
/// a world, must not make the agent hold whatever the server sends. Past
/// [`OUTBOX_BYTES`] the bridge takes nothing more from the room until the
/// hook catches up: the turns wait in the client, whose own bound then holds
/// back the server's stream.
#[derive(Debug, Default)]
pub(crate) struct Outbox {
    messages: VecDeque<ToHook>,
    bytes: usize,
}

/// About how much the outbox holds before the bridge stops taking turns.
const OUTBOX_BYTES: usize = 16 << 20;

/// The speed to tell the hook after a turn at `now`: the turn's, unless
/// the hook was last told it. The first turn always tells it, so the game's
/// Multiplayer window shows a room at normal speed too.
fn speed_news(told: &mut Option<Speed>, now: Speed) -> Option<Speed> {
    if *told == Some(now) {
        return None;
    }
    *told = Some(now);
    Some(now)
}

/// The room as the game's Multiplayer window shows it: its name, owner and
/// members, at most as many as a room holds.
fn room_info(room: &RoomView) -> RoomInfo {
    let members = room
        .members
        .iter()
        .take(usize::from(MAX_ROOM_MEMBERS))
        .map(|member| RoomMember {
            player: member.player,
            name: member.name.clone(),
            connected: member.connected,
        })
        .collect();
    RoomInfo {
        name: room.name.clone(),
        owner: room.owner,
        members: BoundedVec::new(members).unwrap_or_default(),
    }
}

impl Outbox {
    /// About how many bytes `message` takes.
    fn weight(message: &ToHook) -> usize {
        let carried = match message {
            ToHook::Apply(Event {
                body: EventBody::Command { payload, .. },
                ..
            }) => payload.len(),
            ToHook::Load { file, .. } => file.as_ref().map_or(0, |file| file.as_str().len()),
            ToHook::Begin { rules, saves, .. } => rules.as_str().len() + saves.as_str().len(),
            ToHook::Diverged { lanes, .. } => lanes.len() * 2,
            ToHook::Chat { from, text } => from.as_str().len() + text.as_str().len(),
            ToHook::Room(room) => room.members.len() * 64,
            ToHook::Lobby(view) => {
                view.chat.len() * 320 + view.room.as_ref().map_or(0, |room| room.members.len() * 80)
            }
            ToHook::Cursor(cursor) => cursor.label.as_ref().map_or(0, |l| l.as_str().len()),
            ToHook::End { reason } => reason.as_str().len(),
            _ => 0,
        };
        carried + 64
    }

    fn push_back(&mut self, message: ToHook) {
        self.bytes += Self::weight(&message);
        self.messages.push_back(message);
    }

    fn push_front(&mut self, message: ToHook) {
        self.bytes += Self::weight(&message);
        self.messages.push_front(message);
    }

    fn front(&self) -> Option<&ToHook> {
        self.messages.front()
    }

    fn pop_front(&mut self) -> Option<ToHook> {
        let message = self.messages.pop_front()?;
        self.bytes = self.bytes.saturating_sub(Self::weight(&message));
        Some(message)
    }

    fn retain(&mut self, keep: impl FnMut(&ToHook) -> bool) {
        self.messages.retain(keep);
        self.bytes = self.messages.iter().map(Self::weight).sum();
    }

    fn is_full(&self) -> bool {
        self.bytes >= OUTBOX_BYTES
    }
}

fn path_text(path: &Path) -> Result<Text<MAX_PATH>, BridgeFault> {
    Text::new(path.to_string_lossy().into_owned())
        .map_err(|_| BridgeFault::PathTooLong(path.to_owned()))
}

/// Where to find the room again after the connection drops.
#[derive(Debug, Clone)]
pub struct Rejoin {
    pub options: ConnectOptions,
    pub invite: Invite,
    pub password: Option<Text<64>>,
    /// This player's game build and mods, declared on every new
    /// connection: a running game can only be joined afresh with them.
    pub content: Option<ContentManifest>,
    /// Stop trying after this long without a connection.
    pub give_up_after: Duration,
}

/// Plays the room through the hook until it ends. After a lost connection
/// (a network drop, or the server restarting) it reconnects and resumes
/// the room where the game stands, so the game sees only a pause. Tells
/// the hook when the session is over.
pub async fn play<L: HookLink>(
    bridge: &mut Bridge<L>,
    mut client: Client,
    mut events: Events,
    rejoin: &Rejoin,
) -> Result<BridgeEnd, BridgeFault> {
    loop {
        let session = client.welcome().session_id;
        bridge.status(|status| status.session = Some(session));
        let ended = match bridge.run(&client, &mut events).await {
            // A request can find the connection gone before its closing
            // reaches the events: then it is the same loss.
            Err(BridgeFault::Client(
                error @ (ClientError::Disconnected | ClientError::Timeout),
            )) => match tokio::time::timeout(CLOSE_NOTICE, client.closed()).await {
                Ok(reason) => Ok(BridgeEnd::Closed(reason)),
                Err(_) => Err(BridgeFault::Client(error)),
            },
            other => other,
        };
        match ended {
            Ok(BridgeEnd::Closed(reason)) if worth_rejoining(&reason) => {
                warn!(%reason, "lost the server; rejoining the room");
                bridge.status(|status| status.notice("lost the server; rejoining the room"));
            }
            Ok(BridgeEnd::WorldUnavailable) => {
                warn!("the world to load was not available; rejoining the room for another");
            }
            Ok(end) => {
                bridge.end(&format!("{end:?}"));
                return Ok(end);
            }
            Err(fault) => {
                bridge.end(&fault.to_string());
                return Err(fault);
            }
        }
        let options = rejoin.options.again_after(&client);
        drop(client);
        match rejoin_room(bridge, rejoin, &options).await {
            Ok((new_client, new_events)) => {
                info!("rejoined the room");
                bridge.status(|status| status.notice("rejoined the room"));
                client = new_client;
                events = new_events;
            }
            Err(error) => {
                bridge.end(&error);
                return Err(BridgeFault::Rejoin(error));
            }
        }
    }
}

/// How long a request that failed for a lost connection waits for the
/// connection to say it closed, before the failure counts as a fault.
const CLOSE_NOTICE: Duration = Duration::from_secs(5);

/// Whether a lost connection is worth rejoining after: not when this side
/// closed it, another connection replaced it, or the protocol broke.
fn worth_rejoining(reason: &quinn::ConnectionError) -> bool {
    match reason {
        quinn::ConnectionError::LocallyClosed | quinn::ConnectionError::VersionMismatch => false,
        quinn::ConnectionError::ApplicationClosed(closed) => ![
            close::REPLACED,
            close::PROTOCOL_VIOLATION,
            close::VERSION_MISMATCH,
            close::IDLE,
        ]
        .contains(&closed.error_code),
        _ => true,
    }
}

/// Reconnects and rejoins, backing off between attempts and beating for
/// the hook all the while. A room that can no longer resume the game where
/// it stands is joined afresh, and sends a world to load.
async fn rejoin_room<L: HookLink>(
    bridge: &mut Bridge<L>,
    rejoin: &Rejoin,
    options: &ConnectOptions,
) -> Result<(Client, Events), String> {
    let deadline = Instant::now() + rejoin.give_up_after;
    let mut backoff = Duration::from_millis(250);
    let mut resume = bridge.resume_point();
    loop {
        let attempt = async {
            let (client, events) = connect(options.clone()).await.map_err(|error| {
                if error.client_is_older() {
                    Failed::Outdated(error.to_string())
                } else {
                    Failed::Retry(error.to_string())
                }
            })?;
            if let Some(content) = &rejoin.content {
                client
                    .declare_content(content.clone())
                    .await
                    .map_err(|error| Failed::Retry(error.to_string()))?;
            }
            client
                .join_room(JoinRoom {
                    invite: rejoin.invite,
                    password: rejoin.password.clone(),
                    resume,
                })
                .await
                .map_err(|error| {
                    if error == ClientError::Refused(RequestError::ResumeUnavailable) {
                        Failed::ResumeGone(error.to_string())
                    } else {
                        Failed::Retry(error.to_string())
                    }
                })?;
            Ok::<_, Failed>((client, events))
        };
        let outcome = keeping_alive(bridge, attempt).await;
        match outcome {
            Ok(rejoined) => return Ok(rejoined),
            // The server was updated past this client: no attempt can
            // succeed until the player updates too.
            Err(Failed::Outdated(error)) => {
                bridge.status(|status| status.outdated = true);
                return Err(error);
            }
            // The room no longer has these turns: join without them, for a
            // world to load. Joining without them cannot be refused so.
            Err(Failed::ResumeGone(error)) if resume.is_some() => {
                debug!(%error, "the room cannot resume here; joining afresh");
                resume = None;
            }
            Err(Failed::ResumeGone(error) | Failed::Retry(error))
                if Instant::now() + backoff >= deadline =>
            {
                return Err(error);
            }
            Err(Failed::ResumeGone(error) | Failed::Retry(error)) => {
                debug!(%error, "rejoining failed; trying again");
                keeping_alive(bridge, tokio::time::sleep(backoff)).await;
                backoff = (backoff * 2).min(Duration::from_secs(5));
            }
        }
    }
}

/// Why one attempt to rejoin failed.
enum Failed {
    /// Worth trying again.
    Retry(String),
    /// The room no longer has the turns asked for.
    ResumeGone(String),
    /// The server speaks a newer protocol.
    Outdated(String),
}

/// The next request of a front end, or never without one.
async fn next_control(controls: &mut Option<mpsc::Receiver<Control>>) -> Option<Control> {
    match controls {
        Some(controls) => controls.recv().await,
        None => std::future::pending().await,
    }
}

/// Runs `work` while beating for the hook.
async fn keeping_alive<L: HookLink, T>(
    bridge: &mut Bridge<L>,
    work: impl std::future::Future<Output = T>,
) -> T {
    let mut work = std::pin::pin!(work);
    let mut beat = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            done = &mut work => return done,
            _ = beat.tick() => bridge.keep_alive(),
        }
    }
}

/// Moves what the follower allows, at the pace the playout sets, into
/// messages for the hook, in the order its gate requires: every event for
/// step `s` after the release of step `s - 1` and before the release of step
/// `s`. Steps with no events between them go out as one release. Returns
/// when the next step falls due, if one is sealed but not yet due.
pub(crate) fn pump(
    follower: &mut TurnFollower,
    playout: &mut Playout,
    now: Instant,
    out: &mut Outbox,
) -> Option<Instant> {
    let mut released = None;
    let wait = loop {
        if out.is_full() {
            // The hook is behind; the follower keeps the rest.
            break None;
        }
        if let Some(step) = follower.next_step() {
            let due = playout.due(step, now).unwrap_or(now);
            if due > now {
                break Some(due);
            }
            playout.played(step, due);
            follower.next_action();
            released = Some(step);
            continue;
        }
        match follower.next_action() {
            Some(Action::Apply(event)) => {
                if let Some(through) = released.take() {
                    out.push_back(ToHook::Release { through });
                }
                out.push_back(ToHook::Apply(event));
            }
            Some(Action::Execute(step)) => released = Some(step),
            None => break None,
        }
    };
    if let Some(through) = released {
        out.push_back(ToHook::Release { through });
    }
    wait
}

#[cfg(test)]
mod tests {
    use tpf3mp_proto::{FixedBytes, MAX_PAYLOAD, Payload, RoomId, Turn, TurnStart};

    use super::*;

    fn start() -> TurnStart {
        TurnStart {
            room: RoomId(FixedBytes([0; 16])),
            rules: Text::new("native").unwrap(),
            next_turn: 1,
            next_event: 1,
            sealed_through: 0,
            steps_per_second: 10,
            checkpoint_interval: 10,
            history: 1,
            world: None,
        }
    }

    fn event(seq: u64, step: u64) -> Event {
        Event {
            seq,
            step,
            body: EventBody::PlayerLeft {
                player: PlayerId(FixedBytes([1; 32])),
                kicked: false,
            },
        }
    }

    fn turn(number: u64, sealed_through: u64, events: Vec<Event>) -> Turn {
        Turn {
            number,
            sealed_through,
            speed: Speed::NORMAL,
            events,
        }
    }

    /// A follower and a playout that has seen `turns` arrive at `now`.
    fn fed(turns: Vec<Turn>, now: Instant) -> (TurnFollower, Playout) {
        let mut follower = TurnFollower::new(&start());
        let mut playout = Playout::new(10, Duration::ZERO, Duration::from_secs(10));
        for turn in turns {
            follower.accept(turn).unwrap();
            playout.on_turn(follower.sealed_through(), follower.speed(), now);
        }
        (follower, playout)
    }

    #[test]
    fn the_game_hears_the_rooms_speed_from_its_first_turn() {
        let mut told = None;
        assert_eq!(
            speed_news(&mut told, Speed::NORMAL),
            Some(Speed::NORMAL),
            "even at normal speed"
        );
        assert_eq!(speed_news(&mut told, Speed::NORMAL), None, "once");
        assert_eq!(speed_news(&mut told, Speed(400)), Some(Speed(400)));
        assert_eq!(speed_news(&mut told, Speed::PAUSED), Some(Speed::PAUSED));
    }

    #[test]
    fn the_game_sees_the_room_by_its_members_names() {
        use tpf3mp_proto::{MemberView, Platform, RoomPhase, RoomSettings};
        let member = |n: u8, name: &str, connected: bool| MemberView {
            player: PlayerId(FixedBytes([n; 32])),
            name: Text::new(name).unwrap(),
            platform: Platform::current(),
            ready: true,
            content: None,
            connected,
        };
        let room = RoomView {
            id: RoomId(FixedBytes([7; 16])),
            name: Text::new("Sunday line").unwrap(),
            rules: Text::new("native").unwrap(),
            owner: PlayerId(FixedBytes([1; 32])),
            max_players: 4,
            has_password: false,
            phase: RoomPhase::Running,
            settings: RoomSettings::DEFAULT,
            members: vec![member(1, "Ann", true), member(2, "Bo", false)],
        };
        let info = room_info(&room);
        assert_eq!(info.name.as_str(), "Sunday line");
        assert_eq!(info.owner, room.owner);
        let members: Vec<(&str, bool)> = info
            .members
            .iter()
            .map(|m| (m.name.as_str(), m.connected))
            .collect();
        assert_eq!(members, [("Ann", true), ("Bo", false)]);
    }

    #[test]
    fn steps_without_events_between_them_go_out_as_one_release() {
        let now = Instant::now();
        let (mut follower, mut playout) = fed(vec![turn(1, 5, vec![])], now);
        let mut out = Outbox::default();
        // Steps 1 to 5 were sealed at once; reckoned at pace, all are due.
        let wait = pump(&mut follower, &mut playout, now, &mut out);
        assert_eq!(out.messages, [ToHook::Release { through: 5 }]);
        assert_eq!(wait, None, "nothing further is sealed");
    }

    #[test]
    fn an_event_splits_the_releases_around_it() {
        let now = Instant::now();
        let (mut follower, mut playout) = fed(
            vec![turn(1, 2, vec![event(1, 1)]), turn(2, 4, vec![event(2, 3)])],
            now,
        );
        let mut out = Outbox::default();
        // Both turns arrived together, so steps 3 and 4 play at pace after
        // step 2: a second later, all are due.
        pump(
            &mut follower,
            &mut playout,
            now + Duration::from_secs(1),
            &mut out,
        );
        assert_eq!(
            out.messages,
            [
                ToHook::Apply(event(1, 1)),
                ToHook::Release { through: 2 },
                ToHook::Apply(event(2, 3)),
                ToHook::Release { through: 4 },
            ]
        );
    }

    #[test]
    fn a_step_not_yet_due_waits_and_says_when() {
        let now = Instant::now();
        let (mut follower, mut playout) = fed(vec![turn(1, 1, vec![])], now);
        let mut out = Outbox::default();
        pump(&mut follower, &mut playout, now, &mut out);
        assert_eq!(out.messages, [ToHook::Release { through: 1 }]);
        // Step 2 arrives now; with steps 100 ms apart, it plays 100 ms after
        // step 1.
        follower.accept(turn(2, 2, vec![])).unwrap();
        playout.on_turn(2, Speed::NORMAL, now);
        out = Outbox::default();
        let wait = pump(&mut follower, &mut playout, now, &mut out);
        assert!(out.messages.is_empty());
        assert!(wait.is_some_and(|at| at > now), "{wait:?}");
        let wait = pump(
            &mut follower,
            &mut playout,
            now + Duration::from_millis(100),
            &mut out,
        );
        assert_eq!(out.messages, [ToHook::Release { through: 2 }]);
        assert_eq!(wait, None);
    }

    #[test]
    fn a_full_outbox_takes_nothing_more_from_the_room() {
        let now = Instant::now();
        let big = |seq| Event {
            seq,
            step: 1,
            body: EventBody::Command {
                player: PlayerId(FixedBytes([7; 32])),
                client_seq: seq,
                payload: Payload::new(vec![0; MAX_PAYLOAD]).unwrap(),
            },
        };
        // Far more than the outbox takes, all for the next step.
        let events: Vec<Event> = (1..=1000).map(big).collect();
        let (mut follower, mut playout) = fed(vec![turn(1, 0, events)], now);
        let mut out = Outbox::default();
        pump(&mut follower, &mut playout, now, &mut out);
        assert!(out.is_full());
        let taken = out.messages.len();
        assert!(taken < 1000 && out.bytes < OUTBOX_BYTES + MAX_PAYLOAD + 64);
        // Nothing more while it is full; the rest once the hook read some.
        pump(&mut follower, &mut playout, now, &mut out);
        assert_eq!(out.messages.len(), taken);
        while out.pop_front().is_some() && out.messages.len() > taken / 2 {}
        pump(&mut follower, &mut playout, now, &mut out);
        assert!(out.messages.len() > taken / 2);
    }

    fn lobby(name: &str) -> LobbyView {
        LobbyView {
            name: Text::lossy(name),
            ..LobbyView::default()
        }
    }

    #[test]
    fn a_greeted_bridge_passes_the_launchers_lobby_both_ways() {
        use crate::launcher::lobby::tests::FakeLink;

        let fake = FakeLink::default();
        let (views_tx, views) = watch::channel(LobbyView::default());
        let (actions, mut heard) = mpsc::unbounded_channel();
        let mut bridge = Bridge::new(
            fake.clone(),
            BridgeOptions {
                lobby: Some(LobbyLink { views, actions }),
                ..BridgeOptions::default()
            },
        )
        .greeted("40408");
        views_tx.send(lobby("A")).unwrap();
        views_tx.send(lobby("B")).unwrap();
        bridge.lobby_news();
        bridge.flush().unwrap();
        assert_eq!(
            fake.hook_hears(),
            vec![ToHook::Lobby(lobby("B"))],
            "the newest only, and no hello again"
        );
        bridge.lobby_news();
        bridge.flush().unwrap();
        assert!(fake.hook_hears().is_empty(), "unchanged");

        bridge.lobby_action(LobbyAction::Start);
        assert_eq!(heard.try_recv().unwrap(), LobbyAction::Start);

        // A room left before its game began ends nothing in the game, and
        // the launcher gets the link back, greeted.
        bridge.end("Left");
        assert!(fake.hook_hears().is_empty());
        let (_link, build) = bridge.into_link();
        assert_eq!(build.as_deref(), Some("40408"));
    }

    #[test]
    fn only_files_inside_the_saves_directory_are_within_it() {
        let dir = std::env::temp_dir().join(format!("tpf3mp-within-{}", std::process::id()));
        let saves = dir.join("saves");
        std::fs::create_dir_all(&saves).unwrap();
        let inside = saves.join("12.sav");
        let outside = dir.join("notes.txt");
        std::fs::write(&inside, b"save").unwrap();
        std::fs::write(&outside, b"notes").unwrap();
        assert!(within(&inside, &saves));
        assert!(!within(&outside, &saves));
        assert!(!within(&saves.join("..").join("notes.txt"), &saves));
        assert!(!within(&saves, &saves));
        assert!(!within(&saves.join("missing.sav"), &saves));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
