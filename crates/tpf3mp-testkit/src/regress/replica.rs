//! One game of a regression room: the model world behind the real
//! [`Session`], on the real shared-memory link, walking the scenario. The
//! actor it plays sends its actions through [`Session::command`], the path
//! a player's captured actions take; every replica applies them as the
//! events the room orders. The real hook plays a scenario the same way,
//! with the game's world in place of the model (`docs/REGRESSION.md`).

use std::{
    path::Path,
    sync::Arc,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use thiserror::Error;
use tpf3mp_bridge::{Game, Notice, Session, SessionError, StepGate};
use tpf3mp_proto::{Event, LaneDigest, PlayerId};

use super::{
    model::{ModelWorld, Observation},
    script::{CheckOutcome, Cursor, Scenario, ScriptError},
};

#[derive(Debug, Clone)]
pub struct ReplicaConfig {
    /// The link the agent created.
    pub link_name: String,
    /// The player this game belongs to.
    pub player: PlayerId,
    /// Seed of the shared world; the same for every game in a room.
    pub world_seed: u64,
    pub scenario: Arc<Scenario>,
    /// This replica's simulation deviates once at this step.
    pub drift_at: Option<u64>,
    /// How long to wait for the agent to appear, or to beat again.
    pub patience: Duration,
    /// The script fails when an act waits longer than this many steps for
    /// the room to order it.
    pub stall_steps: u64,
    /// The least time between two of this game's acts, to stay under the
    /// room's limit on a player's intents.
    pub min_gap: Duration,
}

#[derive(Debug, Clone)]
pub struct ReplicaReport {
    pub player: PlayerId,
    pub ran: u64,
    /// The world's lanes after the last step run.
    pub lanes: Vec<LaneDigest>,
    pub observation: Observation,
    pub checks: Vec<CheckOutcome>,
    /// The item the script reached; all of them when it finished.
    pub reached: usize,
    pub finished_at: Option<u64>,
    /// Actions the world ignored, with why.
    pub ignored: Vec<(u64, String)>,
    pub diverged: Vec<(u64, Vec<u16>)>,
    /// Items of this replica's actor that the room refused, with why.
    pub refused: Vec<(Option<usize>, String)>,
    /// The item waited longer than the stall limit.
    pub stalled: bool,
    /// The session ended before the script did.
    pub ended: bool,
    pub sent: usize,
    /// For each act this replica sent: from sending it to the step the
    /// room ordered it at, in steps and in time.
    pub lag: Vec<(u64, Duration)>,
}

#[derive(Debug, Error)]
pub enum ReplicaError {
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error(transparent)]
    Script(#[from] ScriptError),
    #[error("the room sent a saved world; the harness plays whole games only")]
    SavedWorld,
}

/// The model world as the session sees a game.
struct ScriptedGame {
    world: ModelWorld,
    cursor: Cursor,
    ran: u64,
    diverged: Vec<(u64, Vec<u16>)>,
    refused: Vec<(Option<usize>, String)>,
}

impl Game for ScriptedGame {
    fn apply(&mut self, event: &Event) {
        self.world.apply(event);
        self.cursor.on_event(&self.world, event, self.ran);
    }

    fn lanes(&mut self) -> Vec<LaneDigest> {
        self.world.lanes()
    }

    fn save(&mut self, file: &Path) -> Result<(), String> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        }
        std::fs::write(file, self.world.save()).map_err(|error| error.to_string())
    }

    fn notice(&mut self, notice: Notice) {
        match notice {
            Notice::Refused { command, reason } => self
                .refused
                .push((self.cursor.refused(command), format!("{reason:?}"))),
            Notice::Diverged { step, lanes } => self.diverged.push((step, lanes)),
            Notice::Speed(_)
            | Notice::Ended(_)
            | Notice::Chat { .. }
            | Notice::Room(_)
            | Notice::Cursor(_) => {}
        }
    }
}

/// Starts the game in a thread of its own.
pub fn spawn(config: ReplicaConfig) -> JoinHandle<Result<ReplicaReport, ReplicaError>> {
    std::thread::spawn(move || run(&config))
}

fn fresh(config: &ReplicaConfig) -> ModelWorld {
    let world = ModelWorld::new(config.world_seed);
    match config.drift_at {
        Some(step) => world.with_drift(step),
        None => world,
    }
}

fn run(config: &ReplicaConfig) -> Result<ReplicaReport, ReplicaError> {
    let mut session = Session::attach(&config.link_name, "regression replica", config.patience)?;
    let begin = session.wait_for_begin()?;
    let interval = u64::from(begin.checkpoint_interval);
    let mut game = ScriptedGame {
        world: fresh(config),
        cursor: Cursor::new(config.scenario.clone())?,
        ran: 0,
        diverged: Vec::new(),
        refused: Vec::new(),
    };
    let mut sent = 0;
    let mut lag = Vec::new();
    // The act in flight: its item, and the step and time it was sent at.
    let mut in_flight: Option<(usize, u64, Instant)> = None;
    let mut last_sent = None;
    let mut stalled = false;
    let mut ended = false;
    loop {
        match session.before_step(&mut game)? {
            StepGate::Run => {}
            StepGate::Load(load) => {
                if load.file.is_some() {
                    return Err(ReplicaError::SavedWorld);
                }
                game.world = fresh(config);
                game.cursor = Cursor::new(config.scenario.clone())?;
                game.ran = load.next_step - 1;
                game.cursor.start(&game.world, game.ran);
                session.loaded(load.next_step)?;
                continue;
            }
            // before_step saves by itself; were it to hand a save over,
            // this is what it asks.
            StepGate::Save(order) => {
                let outcome = game.save(&order.file);
                session.saved(&mut game, outcome)?;
                continue;
            }
            StepGate::Ended | StepGate::Wait => {
                ended = true;
                break;
            }
        }
        if let Some((item, step, at)) = in_flight
            && game.cursor.index() > item
        {
            lag.push((game.ran.saturating_sub(step), at.elapsed()));
            in_flight = None;
        }
        game.world.step(session.next_step());
        game.ran = session.after_step(&mut game)?;
        game.cursor.on_step(&game.world, game.ran);
        if !game.refused.is_empty() {
            break;
        }
        if game
            .cursor
            .end_step(interval)
            .is_some_and(|end| game.ran >= end)
        {
            break;
        }
        if game.cursor.stalled(game.ran, config.stall_steps) {
            stalled = true;
            break;
        }
        let rested = last_sent.is_none_or(|at: Instant| at.elapsed() >= config.min_gap);
        if rested && let Some((item, payload)) = game.cursor.due(&game.world, &config.player) {
            last_sent = Some(Instant::now());
            let command = session.command(payload)?;
            game.cursor.sent(item, command);
            in_flight = Some((item, game.ran, Instant::now()));
            sent += 1;
        }
    }
    Ok(ReplicaReport {
        player: config.player,
        ran: game.ran,
        lanes: game.world.lanes(),
        observation: game.world.observe(),
        checks: game.cursor.checks().to_vec(),
        reached: game.cursor.index(),
        finished_at: game.cursor.finished_at(),
        ignored: game.world.ignored().to_vec(),
        diverged: game.diverged,
        refused: game.refused,
        stalled,
        ended,
        sent,
        lag,
    })
}
