//! A scenario: what the actors do, in order, how long the world runs in
//! between, and what must hold at each point. Every replica walks the same
//! script with a [`Cursor`], driven only by the events the room orders and
//! the steps it runs, so every replica reaches each item at the same step
//! and judges each check on the same world. No side channel coordinates
//! the games; a scenario plays the same across processes and machines.

use std::{fmt, sync::Arc};

use thiserror::Error;
use tpf3mp_proto::{
    Event, EventBody, Payload, PlayerId,
    action::{Action, ActionError, CompanyId, LineId},
};

use super::model::{ModelWorld, Observation};

/// Something that must hold of every replica's world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    Junctions(usize),
    StreetEdges(usize),
    TrackEdges(usize),
    Constructions(usize),
    Stations(usize),
    Depots(usize),
    EdgeObjects(usize),
    /// Companies in the world. Every player founds one on joining, so a
    /// scenario that counts them needs [`Scenario::exact_players`].
    Companies(usize),
    /// The actor plays for this company, or none.
    CompanyOf {
        actor: usize,
        company: Option<CompanyId>,
    },
    Lines(usize),
    Line {
        line: LineId,
        stops: usize,
        vehicles: usize,
    },
    LineName {
        line: LineId,
        name: String,
    },
    Vehicles(usize),
    /// Vehicles on no line.
    Idle(usize),
    /// Actions the world ignored so far, because they did not fit it.
    Ignored(u64),
    DeliveredAtLeast(u64),
    /// The money of this actor's company.
    MoneyAtLeast {
        actor: usize,
        money: i64,
    },
    MoneyBelow {
        actor: usize,
        money: i64,
    },
    TerrainCells(usize),
    /// Prospections under way.
    Prospections(usize),
    /// Industries prospecting found: whether one is found is the
    /// simulation's draw, so a scenario bounds them.
    IndustriesAtMost(usize),
}

impl Check {
    /// Why the check fails on this world, if it does.
    pub fn failure(&self, seen: &Observation) -> Option<String> {
        let count = |what: &str, want: usize, got: usize| {
            (want != got).then(|| format!("{got} {what}, expected {want}"))
        };
        match self {
            Self::Junctions(n) => count("junction configurations", *n, seen.junctions),
            Self::StreetEdges(n) => count("street edges", *n, seen.street_edges),
            Self::TrackEdges(n) => count("track edges", *n, seen.track_edges),
            Self::Constructions(n) => count("constructions", *n, seen.constructions),
            Self::Stations(n) => count("stations", *n, seen.stations),
            Self::Depots(n) => count("depots", *n, seen.depots),
            Self::EdgeObjects(n) => count("objects on edges", *n, seen.edge_objects),
            Self::Companies(n) => count("companies", *n, seen.companies),
            Self::Lines(n) => count("lines", *n, seen.lines.len()),
            Self::Line {
                line,
                stops,
                vehicles,
            } => match seen.lines.get(line) {
                None => Some(format!("no {line}")),
                Some(view) if view.stops != *stops || view.vehicles != *vehicles => Some(format!(
                    "{line} has {} stops and {} vehicles",
                    view.stops, view.vehicles
                )),
                Some(_) => None,
            },
            Self::LineName { line, name } => match seen.lines.get(line) {
                None => Some(format!("no {line}")),
                Some(view) if view.name != *name => Some(format!("{line} is named {}", view.name)),
                Some(_) => None,
            },
            Self::Vehicles(n) => count("vehicles", *n, seen.vehicles),
            Self::Idle(n) => count("idle vehicles", *n, seen.idle),
            Self::Ignored(n) => {
                (seen.ignored != *n).then(|| format!("{} ignored, expected {n}", seen.ignored))
            }
            Self::DeliveredAtLeast(n) => {
                (seen.delivered < *n).then(|| format!("{} passengers delivered", seen.delivered))
            }
            Self::MoneyAtLeast { actor, money } => {
                match seen.money.get(*actor).copied().flatten() {
                    Some(has) if has >= *money => None,
                    has => Some(format!("actor {actor} has {has:?}")),
                }
            }
            Self::MoneyBelow { actor, money } => match seen.money.get(*actor).copied().flatten() {
                Some(has) if has < *money => None,
                has => Some(format!("actor {actor} has {has:?}")),
            },
            Self::CompanyOf { actor, company } => match seen.company_of.get(*actor) {
                Some(found) if found == company => None,
                found => Some(format!("actor {actor} plays for {found:?}")),
            },
            Self::TerrainCells(n) => count("terrain cells", *n, seen.terrain_cells),
            Self::Prospections(n) => count("prospections", *n, seen.prospections),
            Self::IndustriesAtMost(n) => {
                (seen.industries > *n).then(|| format!("{} industries found", seen.industries))
            }
        }
    }
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Junctions(n) => write!(f, "{n} junction configurations"),
            Self::StreetEdges(n) => write!(f, "{n} street edges"),
            Self::TrackEdges(n) => write!(f, "{n} track edges"),
            Self::Constructions(n) => write!(f, "{n} constructions"),
            Self::Stations(n) => write!(f, "{n} stations"),
            Self::Depots(n) => write!(f, "{n} depots"),
            Self::EdgeObjects(n) => write!(f, "{n} objects on edges"),
            Self::Companies(n) => write!(f, "{n} companies"),
            Self::Lines(n) => write!(f, "{n} lines"),
            Self::Line {
                line,
                stops,
                vehicles,
            } => write!(f, "{line} with {stops} stops and {vehicles} vehicles"),
            Self::LineName { line, name } => write!(f, "{line} named {name:?}"),
            Self::Vehicles(n) => write!(f, "{n} vehicles"),
            Self::Idle(n) => write!(f, "{n} idle vehicles"),
            Self::Ignored(n) => write!(f, "{n} actions ignored"),
            Self::DeliveredAtLeast(n) => write!(f, "at least {n} passengers delivered"),
            Self::MoneyAtLeast { actor, money } => write!(f, "actor {actor} has at least {money}"),
            Self::MoneyBelow { actor, money } => write!(f, "actor {actor} has less than {money}"),
            Self::CompanyOf {
                actor,
                company: Some(company),
            } => write!(f, "actor {actor} plays for {company}"),
            Self::CompanyOf {
                actor,
                company: None,
            } => write!(f, "actor {actor} has no company"),
            Self::TerrainCells(n) => write!(f, "{n} terrain cells set"),
            Self::Prospections(n) => write!(f, "{n} prospections under way"),
            Self::IndustriesAtMost(n) => write!(f, "at most {n} industries found"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// The actor, by the order players joined, does this. The next item
    /// waits until the room has ordered it. The world runs on meanwhile, as
    /// a game does, for as many steps as the round trip takes; so a check
    /// must hold however many that were.
    Act { actor: usize, action: Action },
    /// The world runs this many steps.
    Run(u64),
    /// Every replica checks its world here.
    Expect(Check),
}

impl fmt::Display for Item {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Act { actor, action } => {
                let name = format!("{action:?}");
                let name = name.split(['(', ' ', '{']).next().unwrap_or("");
                write!(f, "actor {actor}: {name}")
            }
            Self::Run(steps) => write!(f, "run {steps} steps"),
            Self::Expect(check) => write!(f, "expect {check}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Scenario {
    pub name: String,
    pub about: String,
    /// Part of the quick subset.
    pub smoke: bool,
    /// Players who act; a room plays it with at least this many games.
    pub actors: usize,
    /// The room has exactly the actors in it, no games that only watch: the
    /// scenario names companies by id, and every player's company takes
    /// one.
    pub exact_players: bool,
    pub items: Vec<Item>,
}

impl Scenario {
    /// Games a room plays this scenario with, when `wanted` are asked for:
    /// the actors, and watchers where the scenario allows them.
    pub fn players(&self, wanted: usize) -> usize {
        if self.exact_players {
            self.actors
        } else {
            wanted.max(self.actors)
        }
    }
}

#[derive(Debug, Error)]
pub enum ScriptError {
    #[error("item {item}: actor {actor}, but the scenario has {actors}")]
    NoSuchActor {
        item: usize,
        actor: usize,
        actors: usize,
    },
    #[error("item {item}: {error}")]
    Action { item: usize, error: ActionError },
}

/// How one check went on one replica.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckOutcome {
    pub item: usize,
    pub step: u64,
    pub check: String,
    pub failure: Option<String>,
}

/// Where a replica stands in the script.
#[derive(Debug, Clone)]
pub struct Cursor {
    scenario: Arc<Scenario>,
    payloads: Vec<Option<Payload>>,
    index: usize,
    /// The step the current item became current at.
    since: u64,
    /// The act this replica sent and awaits, with the number the session
    /// gave it.
    sent: Option<(usize, u64)>,
    checks: Vec<CheckOutcome>,
    finished_at: Option<u64>,
}

impl Cursor {
    pub fn new(scenario: Arc<Scenario>) -> Result<Self, ScriptError> {
        let mut payloads = Vec::with_capacity(scenario.items.len());
        for (item, entry) in scenario.items.iter().enumerate() {
            payloads.push(match entry {
                Item::Act { actor, action } => {
                    if *actor >= scenario.actors {
                        return Err(ScriptError::NoSuchActor {
                            item,
                            actor: *actor,
                            actors: scenario.actors,
                        });
                    }
                    Some(
                        action
                            .to_payload()
                            .map_err(|error| ScriptError::Action { item, error })?,
                    )
                }
                Item::Run(_) | Item::Expect(_) => None,
            });
        }
        Ok(Self {
            scenario,
            payloads,
            index: 0,
            since: 0,
            sent: None,
            checks: Vec::new(),
            finished_at: None,
        })
    }

    /// Starts the script on a world that has run `ran` steps.
    pub fn start(&mut self, world: &ModelWorld, ran: u64) {
        self.since = ran;
        self.advance(world, ran);
    }

    /// After the world applied `event`, with `ran` steps run.
    pub fn on_event(&mut self, world: &ModelWorld, event: &Event, ran: u64) {
        let Some(Item::Act { actor, .. }) = self.scenario.items.get(self.index) else {
            return;
        };
        let EventBody::Command {
            player, payload, ..
        } = &event.body
        else {
            return;
        };
        if world.players().get(*actor) == Some(player)
            && self.payloads[self.index].as_ref() == Some(payload)
        {
            self.index += 1;
            self.since = ran;
            self.sent = None;
            self.advance(world, ran);
        }
    }

    /// After the world ran step `ran`.
    pub fn on_step(&mut self, world: &ModelWorld, ran: u64) {
        self.advance(world, ran);
    }

    fn advance(&mut self, world: &ModelWorld, ran: u64) {
        while let Some(item) = self.scenario.items.get(self.index) {
            match item {
                Item::Act { .. } => break,
                Item::Run(steps) => {
                    let until = self.since.saturating_add(*steps);
                    if ran < until {
                        break;
                    }
                    self.since = until;
                }
                Item::Expect(check) => {
                    self.checks.push(CheckOutcome {
                        item: self.index,
                        step: ran,
                        check: check.to_string(),
                        failure: check.failure(&world.observe()),
                    });
                    self.since = ran;
                }
            }
            self.index += 1;
        }
        if self.index == self.scenario.items.len() && self.finished_at.is_none() {
            self.finished_at = Some(ran);
        }
    }

    /// The action `me` sends now, if it is its turn and it has not sent it.
    pub fn due(&self, world: &ModelWorld, me: &PlayerId) -> Option<(usize, Payload)> {
        let Some(Item::Act { actor, .. }) = self.scenario.items.get(self.index) else {
            return None;
        };
        let mine = world.players().get(*actor) == Some(me);
        let unsent = self.sent.is_none_or(|(item, _)| item != self.index);
        if !mine || !unsent {
            return None;
        }
        Some((self.index, self.payloads[self.index].clone()?))
    }

    pub fn sent(&mut self, item: usize, command: u64) {
        self.sent = Some((item, command));
    }

    /// The item a refused command was.
    pub fn refused(&self, command: u64) -> Option<usize> {
        self.sent
            .and_then(|(item, number)| (number == command).then_some(item))
    }

    /// The item the script waits at.
    pub fn index(&self) -> usize {
        self.index
    }

    /// Whether an act has waited more than `limit` steps for the room to
    /// order it, at step `ran`.
    pub fn stalled(&self, ran: u64, limit: u64) -> bool {
        matches!(self.scenario.items.get(self.index), Some(Item::Act { .. }))
            && ran.saturating_sub(self.since) > limit
    }

    pub fn checks(&self) -> &[CheckOutcome] {
        &self.checks
    }

    pub fn finished_at(&self) -> Option<u64> {
        self.finished_at
    }

    /// Where a replica that finished stops: the first checkpoint after the
    /// script ended, so the room compares every replica's final world.
    pub fn end_step(&self, checkpoint_interval: u64) -> Option<u64> {
        let interval = checkpoint_interval.max(1);
        self.finished_at.map(|at| (at / interval + 1) * interval)
    }
}
