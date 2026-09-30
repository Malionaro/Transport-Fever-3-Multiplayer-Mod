//! A model of the game that plays the portable action schema
//! ([`tpf3mp_proto::action`]): streets and tracks, constructions, stations,
//! depots, vehicles, lines and companies, with a small deterministic
//! simulation of vehicles carrying passengers along their lines. It stands
//! in for Transport Fever 3 in the regression harness until the hook applies
//! actions to the real game, and it is held to the same rules the hook is:
//!
//! - an action is applied whole or not at all. One that does not fit this
//!   world (an edge that is not there, a line of someone else's, too little
//!   money) changes nothing and is counted as ignored, the same on every
//!   replica, since every replica applies the same events to the same world;
//! - references resolve by position within the tolerances `docs/BUILDING.md`
//!   gives, and canonical ids count up in the order the room applied what
//!   created them.
//!
//! Its state is compared through checkpoint lanes, and
//! [`ModelWorld::with_drift`] perturbs its simulation the way a platform's
//! float difference would, so a test can show the harness catches it.

use std::collections::{BTreeMap, BTreeSet};

use ring::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};
use tpf3mp_proto::{
    Event, EventBody, FixedBytes, LaneDigest, PlayerId,
    action::{
        Action, Bulldoze, CompanyId, CompanyOp, ConstructionBuild, ConstructionRef, EdgeEnds,
        LineChange, LineId, LoanOp, Network, Polyline, Pos, Prospect, ReplaceVehicle, Resolve,
        Structure, Terraform, VehicleChange,
    },
};

use crate::rng::SplitMix64;

pub const START_MONEY: i64 = 10_000_000;
pub const ROAD_COST_PER_M: i64 = 20;
pub const TRACK_COST_PER_M: i64 = 100;
pub const STATION_COST: i64 = 40_000;
pub const DEPOT_COST: i64 = 20_000;
pub const CONSTRUCTION_COST: i64 = 10_000;
/// Per car of a consist.
pub const VEHICLE_COST: i64 = 30_000;
pub const TERRAFORM_COST_PER_CELL: i64 = 5;
/// Earned per passenger delivered.
pub const FARE: i64 = 20;
/// Each vehicle costs this every [`UPKEEP_EVERY`] steps.
pub const UPKEEP: i64 = 20;
pub const UPKEEP_EVERY: u64 = 100;
/// Passengers a car carries.
pub const CAR_CAPACITY: u32 = 40;
/// Most passengers waiting at one station.
pub const MAX_WAITING: u32 = 1_000;

/// Steps a prospection takes before its outcome, the model's stand-in for
/// TF3's six months.
pub const PROSPECTION_STEPS: u64 = 300;
/// In hundredths: how often a prospection finds an industry.
pub const PROSPECTION_CHANCE: u64 = 60;

/// Tolerances of `docs/BUILDING.md`, in millimetres.
const NODE_TOLERANCE: i64 = 1_500;
const EDGE_TOLERANCE: i64 = 1_000;
const CONSTRUCTION_TOLERANCE: i64 = 2_000;

/// Lanes a replica reports at checkpoints.
pub mod lane {
    /// Streets, tracks and terrain.
    pub const NETWORK: u16 = 0;
    /// Constructions, stations and stops.
    pub const CONSTRUCTIONS: u16 = 1;
    pub const LINES: u16 = 2;
    /// Vehicles where they are, and the passengers waiting.
    pub const VEHICLES: u16 = 3;
    /// Companies, money and the simulation's generator.
    pub const ECONOMY: u16 = 4;
}

type P = [i32; 3];
/// An edge: its network and its ends, the lesser first, so the key does not
/// depend on which end a game calls `node0`.
type EdgeKey = (u8, P, P);

fn p(pos: &Pos) -> P {
    [pos.x, pos.y, pos.z]
}

fn net(network: Network) -> u8 {
    match network {
        Network::Street => 0,
        Network::Track => 1,
    }
}

fn edge_key(network: u8, a: P, b: P) -> EdgeKey {
    if a <= b {
        (network, a, b)
    } else {
        (network, b, a)
    }
}

fn dist2(a: P, b: P) -> i128 {
    (0..3)
        .map(|i| {
            let d = i128::from(a[i]) - i128::from(b[i]);
            d * d
        })
        .sum()
}

fn within(a: P, b: P, tolerance: i64) -> bool {
    dist2(a, b) <= i128::from(tolerance) * i128::from(tolerance)
}

fn within_horizontally(a: P, b: P, tolerance: i64) -> bool {
    within([a[0], a[1], 0], [b[0], b[1], 0], tolerance)
}

/// Whole metres between two points, at least one.
fn metres(a: P, b: P) -> i64 {
    let mm = dist2(a, b).unsigned_abs().isqrt();
    i64::try_from(mm / 1000).unwrap_or(i64::MAX).max(1)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Company {
    name: String,
    money: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Edge {
    kind: String,
    structure: String,
    /// Nobody's, as a town's streets are, or a company's.
    owner: Option<u32>,
}

/// A stop, signal or waypoint on an edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct EdgeObject {
    model: String,
    left: bool,
    owner: u32,
    /// The station a stop is.
    station: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Kind {
    Station(u32),
    Depot,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Construction {
    kind: Kind,
    owner: u32,
    name: String,
    basis: [i32; 9],
    params: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Station {
    at: P,
    waiting: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Line {
    owner: u32,
    name: String,
    color: [i32; 3],
    stops: Vec<(u32, Option<u16>)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Vehicle {
    owner: u32,
    consist: Vec<String>,
    line: Option<u32>,
    next_stop: u16,
    progress: u32,
    load: u32,
}

/// A prospection under way: TF3's company script keeps these
/// (`pendingProspections`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Prospection {
    company: u32,
    town: u32,
    cargo: String,
    industries: Vec<String>,
    began: u64,
}

/// An industry a prospection found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Industry {
    kind: String,
    town: u32,
    at: P,
}

/// Everything a save holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct State {
    /// In the order they joined: the harness's actors by index.
    players: Vec<PlayerId>,
    member_of: BTreeMap<PlayerId, u32>,
    companies: BTreeMap<u32, Company>,
    edges: BTreeMap<EdgeKey, Edge>,
    objects: BTreeMap<(EdgeKey, P), EdgeObject>,
    constructions: BTreeMap<(String, P), Construction>,
    stations: BTreeMap<u32, Station>,
    lines: BTreeMap<u32, Line>,
    vehicles: BTreeMap<u32, Vehicle>,
    terrain: BTreeMap<(i32, i32), i32>,
    /// In the order they began.
    prospections: Vec<Prospection>,
    industries: BTreeMap<u32, Industry>,
    next_industry: u32,
    /// The last step simulated.
    now: u64,
    next_company: u32,
    next_station: u32,
    next_line: u32,
    next_vehicle: u32,
    delivered: u64,
    /// Actions that changed nothing.
    ignored: u64,
    rng: u64,
}

/// What an action that changes nothing says about why.
type Refusal = String;

macro_rules! refuse {
    ($($arg:tt)*) => {
        return Err(format!($($arg)*))
    };
}

impl State {
    fn join(&mut self, player: PlayerId) {
        if self.member_of.contains_key(&player) {
            return;
        }
        self.players.push(player);
        let company = self.found(format!("Company {}", self.next_company + 1));
        self.member_of.insert(player, company);
    }

    fn found(&mut self, name: String) -> u32 {
        let id = self.next_company;
        self.next_company += 1;
        self.companies.insert(
            id,
            Company {
                name,
                money: START_MONEY,
            },
        );
        id
    }

    fn charge(&mut self, company: u32, amount: i64) -> Result<(), Refusal> {
        let Some(entry) = self.companies.get_mut(&company) else {
            refuse!("company-{company} is gone");
        };
        if entry.money < amount {
            refuse!("company-{company} has {} and needs {amount}", entry.money);
        }
        entry.money -= amount;
        Ok(())
    }

    fn act(&mut self, player: &PlayerId, action: &Action) -> Result<(), Refusal> {
        if let Action::CompanyOp(op) = action {
            return self.company_op(player, op);
        }
        let Some(&company) = self.member_of.get(player) else {
            refuse!("the player has no company");
        };
        match action {
            Action::BuildRoad(road) => self.build(
                Network::Street,
                road.street.as_str(),
                &road.polyline,
                company,
                ROAD_COST_PER_M,
            ),
            Action::BuildTrack(track) => self.build(
                Network::Track,
                track.track.as_str(),
                &track.polyline,
                company,
                TRACK_COST_PER_M,
            ),
            Action::Bulldoze(bulldoze) => self.bulldoze(bulldoze, company),
            Action::BuildConstruction(build) => self.construct(build, company),
            Action::BuyVehicle(buy) => {
                let key = self.find_construction(&buy.depot)?;
                let depot = &self.constructions[&key];
                if depot.kind != Kind::Depot {
                    refuse!("{} is not a depot", buy.depot.file);
                }
                if depot.owner != company {
                    refuse!("the depot is company-{}'s", depot.owner);
                }
                if buy.consist.is_empty() {
                    refuse!("a vehicle of no cars");
                }
                let cars = i64::try_from(buy.consist.len()).unwrap_or(i64::MAX);
                self.charge(company, VEHICLE_COST.saturating_mul(cars))?;
                let id = self.next_vehicle;
                self.next_vehicle += 1;
                self.vehicles.insert(
                    id,
                    Vehicle {
                        owner: company,
                        consist: buy
                            .consist
                            .iter()
                            .map(|part| part.model.as_str().to_owned())
                            .collect(),
                        line: None,
                        next_stop: 0,
                        progress: 0,
                        load: 0,
                    },
                );
                Ok(())
            }
            Action::SellVehicle { vehicles } => {
                let ids = self.own_vehicles(vehicles.iter().map(|v| v.0), company)?;
                for id in ids {
                    let vehicle = self.vehicles.remove(&id).expect("checked above");
                    let cars = i64::try_from(vehicle.consist.len()).unwrap_or(i64::MAX);
                    let refund = VEHICLE_COST.saturating_mul(cars) / 2;
                    if let Some(owner) = self.companies.get_mut(&company) {
                        owner.money += refund;
                    }
                }
                Ok(())
            }
            Action::CreateLine(create) => {
                let stops = self.stops(create.line.stops.iter())?;
                let id = self.next_line;
                self.next_line += 1;
                self.lines.insert(
                    id,
                    Line {
                        owner: company,
                        name: create.name.as_str().to_owned(),
                        color: [create.color.r, create.color.g, create.color.b],
                        stops,
                    },
                );
                Ok(())
            }
            Action::EditLine(edit) => {
                let line = self.own_line(edit.line, company)?;
                match &edit.change {
                    LineChange::Rename(name) => {
                        self.lines.get_mut(&line).expect("checked").name = name.as_str().to_owned();
                    }
                    LineChange::Recolor(color) => {
                        self.lines.get_mut(&line).expect("checked").color =
                            [color.r, color.g, color.b];
                    }
                    LineChange::Update(line_data) => {
                        let stops = self.stops(line_data.stops.iter())?;
                        let count = stops.len();
                        self.lines.get_mut(&line).expect("checked").stops = stops;
                        for vehicle in self.vehicles.values_mut() {
                            if vehicle.line == Some(line) && usize::from(vehicle.next_stop) >= count
                            {
                                vehicle.next_stop = 0;
                                vehicle.progress = 0;
                            }
                        }
                    }
                    LineChange::Delete => {
                        self.lines.remove(&line);
                        for vehicle in self.vehicles.values_mut() {
                            if vehicle.line == Some(line) {
                                vehicle.line = None;
                            }
                        }
                    }
                }
                Ok(())
            }
            Action::AssignLine(assign) => {
                let ids = self.own_vehicles(assign.vehicles.iter().map(|v| v.0), company)?;
                if ids.is_empty() {
                    refuse!("an assignment of no vehicles");
                }
                if let (Some(line), Some(first)) = (assign.line, assign.first_stop) {
                    let line = self.own_line(line, company)?;
                    if usize::from(first) >= self.lines[&line].stops.len() {
                        refuse!("line-{line} has no stop {first}");
                    }
                } else if let Some(line) = assign.line {
                    self.own_line(line, company)?;
                }
                for id in ids {
                    let vehicle = self.vehicles.get_mut(&id).expect("checked above");
                    vehicle.line = assign.line.map(|line| line.0);
                    // The game's choice: the model's vehicles all reach
                    // every stop, so the first.
                    vehicle.next_stop = assign.first_stop.unwrap_or(0);
                    vehicle.progress = 0;
                }
                Ok(())
            }
            // The model keeps no vehicle state beyond its line: a vehicle
            // sent to its depot leaves its line, and one sold there goes, as
            // a sale.
            Action::VehicleOp(op) => {
                let ids = self.own_vehicles(std::iter::once(op.vehicle.0), company)?;
                if let VehicleChange::ToDepot { sell } = op.change {
                    for id in ids {
                        if sell {
                            let vehicle = self.vehicles.remove(&id).expect("checked above");
                            let cars = i64::try_from(vehicle.consist.len()).unwrap_or(i64::MAX);
                            if let Some(owner) = self.companies.get_mut(&company) {
                                owner.money += VEHICLE_COST.saturating_mul(cars) / 2;
                            }
                        } else {
                            self.vehicles.get_mut(&id).expect("checked above").line = None;
                        }
                    }
                }
                Ok(())
            }
            Action::ReplaceVehicle(replace) => self.replace(replace, company),
            Action::PlaceStop(stop) => {
                let key = self.find_edge(net(stop.edge.network), &stop.edge.ends)?;
                let at = p(&stop.at);
                if self
                    .objects
                    .keys()
                    .any(|(edge, there)| *edge == key && within(*there, at, EDGE_TOLERANCE))
                {
                    refuse!("something already stands there on the edge");
                }
                let model = stop.model.as_str().to_owned();
                let station = model.contains("stop").then(|| {
                    let id = self.next_station;
                    self.next_station += 1;
                    self.stations.insert(id, Station { at, waiting: 0 });
                    id
                });
                self.charge(company, CONSTRUCTION_COST)?;
                self.objects.insert(
                    (key, at),
                    EdgeObject {
                        model,
                        left: stop.left,
                        owner: company,
                        station,
                    },
                );
                Ok(())
            }
            Action::Terraform(terraform) => self.terraform(terraform, company),
            // A loan pays its amount in, and paying it back takes it out;
            // the model keeps no interest.
            Action::Loan(op) => match op.as_ref() {
                LoanOp::Take { offer, .. } => {
                    if offer.amount <= 0 {
                        refuse!("a loan of {}", offer.amount);
                    }
                    if let Some(entry) = self.companies.get_mut(&company) {
                        entry.money = entry.money.saturating_add(offer.amount);
                    }
                    Ok(())
                }
                LoanOp::Repay { loan } => self.charge(company, loan.amount.max(0)),
            },
            Action::Prospect(prospect) => self.prospect(prospect, company),
            // A notification's sound played: nothing the model keeps.
            Action::NotificationSeen { .. } => Ok(()),
            Action::CompanyOp(_) => unreachable!("handled above"),
        }
    }

    /// As TF3's company script: one prospection per company, town and cargo
    /// at a time; it costs a permit, which the model does not keep.
    fn prospect(&mut self, prospect: &Prospect, company: u32) -> Result<(), Refusal> {
        let (town, cargo) = (prospect.town.0, prospect.cargo.as_str());
        if self
            .prospections
            .iter()
            .any(|p| p.company == company && p.town == town && p.cargo == cargo)
        {
            refuse!("company-{company} prospects for {cargo} near town-{town} already");
        }
        self.prospections.push(Prospection {
            company,
            town,
            cargo: cargo.to_owned(),
            industries: prospect
                .industries
                .iter()
                .map(|kind| kind.as_str().to_owned())
                .collect(),
            began: self.now,
        });
        Ok(())
    }

    /// The prospections whose time is up: each finds an industry of one of
    /// its types near its town, or nothing, by the simulation's generator.
    fn prospections_end(&mut self, rng: &mut SplitMix64) {
        let now = self.now;
        let (due, going): (Vec<Prospection>, Vec<Prospection>) =
            std::mem::take(&mut self.prospections)
                .into_iter()
                .partition(|p| now.saturating_sub(p.began) >= PROSPECTION_STEPS);
        self.prospections = going;
        for prospection in due {
            let found = rng.below(100) < PROSPECTION_CHANCE;
            let count = u64::try_from(prospection.industries.len()).unwrap_or(0);
            if !found || count == 0 {
                continue;
            }
            let pick = usize::try_from(rng.below(count)).unwrap_or(0);
            let offset = |r: &mut SplitMix64| i32::try_from(r.below(2_000)).unwrap_or(0) * 1_000;
            let base = i32::try_from(prospection.town).unwrap_or(0) * 100_000;
            let at = [base + offset(rng), base + offset(rng), 0];
            let id = self.next_industry;
            self.next_industry += 1;
            self.industries.insert(
                id,
                Industry {
                    kind: prospection.industries[pick].clone(),
                    town: prospection.town,
                    at,
                },
            );
        }
    }

    fn company_op(&mut self, player: &PlayerId, op: &CompanyOp) -> Result<(), Refusal> {
        let current = self.member_of.get(player).copied();
        match op {
            CompanyOp::Create { name } => {
                let company = self.found(name.as_str().to_owned());
                self.member_of.insert(*player, company);
            }
            CompanyOp::Join(CompanyId(company)) => {
                if !self.companies.contains_key(company) {
                    refuse!("no company-{company}");
                }
                self.member_of.insert(*player, *company);
            }
            CompanyOp::Rename {
                company: CompanyId(company),
                name,
            } => {
                if current != Some(*company) {
                    refuse!("the player is not in company-{company}");
                }
                self.companies.get_mut(company).expect("a member's").name =
                    name.as_str().to_owned();
            }
            CompanyOp::Recolor {
                company: CompanyId(company),
                ..
            } => {
                if current != Some(*company) {
                    refuse!("the player is not in company-{company}");
                }
            }
            CompanyOp::Delete(CompanyId(company)) => {
                if current != Some(*company) {
                    refuse!("the player is not in company-{company}");
                }
                if self.member_of.values().filter(|c| *c == company).count() > 1 {
                    refuse!("company-{company} has other members");
                }
                let owns = self.edges.values().any(|e| e.owner == Some(*company))
                    || self.objects.values().any(|o| o.owner == *company)
                    || self.constructions.values().any(|c| c.owner == *company)
                    || self.lines.values().any(|l| l.owner == *company)
                    || self.vehicles.values().any(|v| v.owner == *company);
                if owns {
                    refuse!("company-{company} still owns something");
                }
                self.companies.remove(company);
                self.member_of.remove(player);
            }
        }
        Ok(())
    }

    fn build(
        &mut self,
        network: Network,
        kind: &str,
        polyline: &Polyline,
        company: u32,
        cost_per_m: i64,
    ) -> Result<(), Refusal> {
        let n = net(network);
        // Existing nodes are found before anything is removed: within one
        // build a node outlives its edges, as in the game's proposal.
        let mut at: Vec<Option<P>> = Vec::with_capacity(polyline.vertices.len());
        for vertex in polyline.vertices.iter() {
            let pos = p(&vertex.pos);
            at.push(match &vertex.resolve {
                Resolve::New => Some(pos),
                Resolve::Node(of) => match self.find_node(net(*of), pos) {
                    Some(node) => Some(node),
                    None => refuse!("no {of:?} node at {pos:?}"),
                },
                Resolve::Split(_) => None,
            });
        }
        // The nodes it removes, found before anything is.
        let mut gone = Vec::with_capacity(polyline.removed_nodes.len());
        for node in polyline.removed_nodes.iter() {
            let pos = p(&node.at);
            let Some(found) = self.find_node(net(node.network), pos) else {
                refuse!("no {:?} node to remove at {pos:?}", node.network);
            };
            let joined = polyline
                .vertices
                .iter()
                .zip(&at)
                .any(|(v, r)| matches!(v.resolve, Resolve::Node(_)) && *r == Some(found));
            if joined {
                refuse!("the build removes a node it joins");
            }
            gone.push((net(node.network), found));
        }
        for removal in polyline.removals.iter() {
            let key = self.find_edge(net(removal.network), &removal.ends)?;
            self.unobstructed(&key)?;
            self.edges.remove(&key);
        }
        // A node goes with its last edge; the game refuses to remove one
        // that still has any.
        for (network, node) in gone {
            let kept = self
                .edges
                .keys()
                .any(|(n, a, b)| *n == network && (*a == node || *b == node));
            if kept {
                refuse!("the node at {node:?} still has edges");
            }
        }
        for (vertex, slot) in polyline.vertices.iter().zip(at.iter_mut()) {
            if let Resolve::Split(edge) = &vertex.resolve {
                let pos = p(&vertex.pos);
                let key = self.find_edge(net(edge.network), &edge.ends)?;
                self.unobstructed(&key)?;
                if pos == key.1 || pos == key.2 {
                    refuse!("a split at the end of an edge");
                }
                let split = self.edges.remove(&key).expect("found above");
                self.edges
                    .insert(edge_key(key.0, key.1, pos), split.clone());
                self.edges.insert(edge_key(key.0, pos, key.2), split);
                *slot = Some(pos);
            }
        }
        let at: Vec<P> = at
            .into_iter()
            .map(|pos| pos.expect("every vertex resolved above"))
            .collect();
        let mut cost: i64 = 0;
        for link in polyline.links.iter() {
            let (a, b) = (at[usize::from(link.from)], at[usize::from(link.to)]);
            if a == b {
                refuse!("an edge from a point to itself");
            }
            // The build's own kind, or the kind the link keeps: a piece of
            // the street it joins, rebuilt through the new junction.
            let (ln, own) = match &link.kind {
                Some(other) => (net(other.network), other.template.as_str()),
                None => (n, kind),
            };
            let key = edge_key(ln, a, b);
            if self.edges.contains_key(&key) {
                refuse!("the edge {a:?}-{b:?} is already built");
            }
            let structure = match &link.structure {
                Structure::Ground => "ground".to_owned(),
                Structure::Bridge(kind) => format!("bridge:{kind}"),
                Structure::Tunnel(kind) => format!("tunnel:{kind}"),
            };
            self.edges.insert(
                key,
                Edge {
                    kind: own.to_owned(),
                    structure,
                    owner: Some(company),
                },
            );
            cost = cost.saturating_add(metres(a, b).saturating_mul(cost_per_m));
        }
        self.charge(company, cost)
    }

    fn bulldoze(&mut self, bulldoze: &Bulldoze, company: u32) -> Result<(), Refusal> {
        match bulldoze {
            Bulldoze::Edges { network, edges } => {
                if edges.is_empty() {
                    refuse!("a bulldoze of nothing");
                }
                for ends in edges.iter() {
                    let key = self.find_edge(net(*network), ends)?;
                    let owner = self.edges[&key].owner;
                    if owner.is_some_and(|owner| owner != company) {
                        refuse!("the edge is company-{}'s", owner.unwrap_or_default());
                    }
                    self.unobstructed(&key)?;
                    self.edges.remove(&key);
                }
            }
            Bulldoze::Construction(reference) => {
                let key = self.find_construction(reference)?;
                let construction = &self.constructions[&key];
                if construction.owner != company {
                    refuse!("the construction is company-{}'s", construction.owner);
                }
                if let Kind::Station(station) = construction.kind {
                    self.unserved(station)?;
                    self.stations.remove(&station);
                }
                self.constructions.remove(&key);
            }
            Bulldoze::EdgeObject { edge, at, model } => {
                let key = self.find_edge(net(edge.network), &edge.ends)?;
                let at = p(at);
                let found = self
                    .objects
                    .iter()
                    .filter(|((on, there), object)| {
                        *on == key
                            && object.model == model.as_str()
                            && within(*there, at, EDGE_TOLERANCE)
                    })
                    .min_by_key(|((_, there), _)| dist2(*there, at))
                    .map(|(k, object)| (*k, object.owner, object.station));
                let Some((object, owner, station)) = found else {
                    refuse!("no {model} there");
                };
                if owner != company {
                    refuse!("the {model} is company-{owner}'s");
                }
                if let Some(station) = station {
                    self.unserved(station)?;
                    self.stations.remove(&station);
                }
                self.objects.remove(&object);
            }
        }
        Ok(())
    }

    fn construct(&mut self, build: &ConstructionBuild, company: u32) -> Result<(), Refusal> {
        if build.name.as_str().is_empty() {
            refuse!("an unnamed construction");
        }
        let file = build.file.as_str().to_owned();
        let origin = p(&build.transform.origin);
        let mut kind = if file.contains("depot") {
            Kind::Depot
        } else if file.contains("station") {
            Kind::Station(u32::MAX)
        } else {
            Kind::Other
        };
        if let Some(replaced) = &build.replaces {
            let key = self.find_construction(replaced)?;
            let old = self.constructions.remove(&key).expect("found above");
            if old.owner != company {
                refuse!("the construction is company-{}'s", old.owner);
            }
            // A module edit keeps the station, and the lines stopping there.
            match (old.kind, kind) {
                (Kind::Station(id), Kind::Station(_)) => {
                    kind = Kind::Station(id);
                    if let Some(station) = self.stations.get_mut(&id) {
                        station.at = origin;
                    }
                }
                (Kind::Station(id), _) => {
                    self.unserved(id)?;
                    self.stations.remove(&id);
                }
                _ => {}
            }
        }
        if self
            .constructions
            .keys()
            .any(|(other, at)| *other == file && within(*at, origin, CONSTRUCTION_TOLERANCE))
        {
            refuse!("a {file} already stands there");
        }
        if kind == Kind::Station(u32::MAX) {
            let id = self.next_station;
            self.next_station += 1;
            self.stations.insert(
                id,
                Station {
                    at: origin,
                    waiting: 0,
                },
            );
            kind = Kind::Station(id);
        }
        let cost = match kind {
            Kind::Station(_) => STATION_COST,
            Kind::Depot => DEPOT_COST,
            Kind::Other => CONSTRUCTION_COST,
        };
        self.charge(company, cost)?;
        self.constructions.insert(
            (file, origin),
            Construction {
                kind,
                owner: company,
                name: build.name.as_str().to_owned(),
                basis: build.transform.basis,
                params: build.params.len(),
            },
        );
        // The streets the tool built with it, in the same proposal: every
        // link names its kind, a construction having none of its own.
        if let Some(connection) = &build.connection {
            let mut kinds = connection.links.iter().map(|link| link.kind.as_ref());
            let Some(Some(first)) = kinds.next() else {
                refuse!("a connection link of no kind");
            };
            if kinds.any(|kind| kind.is_none()) {
                refuse!("a connection link of no kind");
            }
            let cost = match first.network {
                Network::Street => ROAD_COST_PER_M,
                Network::Track => TRACK_COST_PER_M,
            };
            let template = first.template.as_str().to_owned();
            self.build(first.network, &template, connection, company, cost)?;
        }
        Ok(())
    }

    fn terraform(&mut self, terraform: &Terraform, company: u32) -> Result<(), Refusal> {
        let columns = i64::from(terraform.columns);
        let cell = i64::from(terraform.cell);
        for (index, cell_height) in terraform.cells.iter().enumerate() {
            let index = i64::try_from(index).unwrap_or(i64::MAX);
            let x = i64::from(terraform.origin.x) + (index % columns) * cell;
            let y = i64::from(terraform.origin.y) + (index / columns) * cell;
            let (Ok(x), Ok(y)) = (i32::try_from(x), i32::try_from(y)) else {
                refuse!("a terrain cell off the map");
            };
            self.terrain.insert((x, y), cell_height.target);
        }
        let cells = i64::try_from(terraform.cells.len()).unwrap_or(i64::MAX);
        self.charge(company, cells.saturating_mul(TERRAFORM_COST_PER_CELL))
    }

    /// The node of `network` nearest `pos`, within the node tolerance.
    fn find_node(&self, network: u8, pos: P) -> Option<P> {
        self.edges
            .keys()
            .filter(|(n, _, _)| *n == network)
            .flat_map(|(_, a, b)| [*a, *b])
            .filter(|end| within_horizontally(*end, pos, NODE_TOLERANCE))
            .min_by_key(|end| (dist2(*end, pos), *end))
    }

    fn find_edge(&self, network: u8, ends: &EdgeEnds) -> Result<EdgeKey, Refusal> {
        let (a, b) = (p(&ends.a), p(&ends.b));
        let matches = |x: P, y: P| {
            (within(x, a, EDGE_TOLERANCE) && within(y, b, EDGE_TOLERANCE))
                || (within(x, b, EDGE_TOLERANCE) && within(y, a, EDGE_TOLERANCE))
        };
        match self
            .edges
            .keys()
            .find(|(n, x, y)| *n == network && matches(*x, *y))
        {
            Some(key) => Ok(*key),
            None => refuse!("no edge {a:?}-{b:?}"),
        }
    }

    fn find_construction(&self, reference: &ConstructionRef) -> Result<(String, P), Refusal> {
        let at = p(&reference.at);
        self.constructions
            .keys()
            .filter(|(file, there)| {
                file == reference.file.as_str() && within(*there, at, CONSTRUCTION_TOLERANCE)
            })
            .min_by_key(|(_, there)| dist2(*there, at))
            .cloned()
            .ok_or_else(|| format!("no {} at {at:?}", reference.file))
    }

    /// Refuses when a stop or signal stands on the edge.
    fn unobstructed(&self, key: &EdgeKey) -> Result<(), Refusal> {
        if self.objects.keys().any(|(edge, _)| edge == key) {
            refuse!("something stands on the edge");
        }
        Ok(())
    }

    /// Refuses when a line stops at the station.
    fn unserved(&self, station: u32) -> Result<(), Refusal> {
        if let Some((id, _)) = self
            .lines
            .iter()
            .find(|(_, line)| line.stops.iter().any(|(s, _)| *s == station))
        {
            refuse!("line-{id} stops at station-{station}");
        }
        Ok(())
    }

    fn stops<'a>(
        &self,
        stops: impl Iterator<Item = &'a tpf3mp_proto::action::LineStop>,
    ) -> Result<Vec<(u32, Option<u16>)>, Refusal> {
        let stops: Vec<_> = stops
            .map(|s| (s.group.0, Some(s.terminal.terminal)))
            .collect();
        if stops.len() < 2 {
            refuse!("a line of fewer than two stops");
        }
        for (station, _) in &stops {
            if !self.stations.contains_key(station) {
                refuse!("no station-{station}");
            }
        }
        Ok(stops)
    }

    fn own_line(&self, line: LineId, company: u32) -> Result<u32, Refusal> {
        match self.lines.get(&line.0) {
            None => refuse!("no {line}"),
            Some(found) if found.owner != company => {
                refuse!("{line} is company-{}'s", found.owner)
            }
            Some(_) => Ok(line.0),
        }
    }

    /// A vehicle's consist swapped for another, as TF3's vehicle window
    /// does: the company's own vehicle, a consist of cars whose groups add
    /// up to it, each kept car one of the vehicle's own of the same model,
    /// kept once. New cars are paid for, cars left out are sold at half; the
    /// vehicle keeps its id and its line.
    fn replace(&mut self, replace: &ReplaceVehicle, company: u32) -> Result<(), Refusal> {
        let [id] = self.own_vehicles(std::iter::once(replace.vehicle.0), company)?[..] else {
            unreachable!("one vehicle named, one checked")
        };
        if replace.consist.is_empty() {
            refuse!("a replacement of no cars");
        }
        let grouped: usize = replace.groups.iter().map(|g| usize::from(*g)).sum();
        if grouped != replace.consist.len() || replace.groups.contains(&0) {
            refuse!(
                "groups of {grouped} cars for a consist of {}",
                replace.consist.len()
            );
        }
        if replace.multiple_units.len() != replace.groups.len() {
            refuse!(
                "{} multiple units for {} groups",
                replace.multiple_units.len(),
                replace.groups.len()
            );
        }
        let old = &self.vehicles[&id].consist;
        let mut kept = BTreeSet::new();
        for (index, part) in replace.consist.iter().enumerate() {
            let Some(from) = part.kept else { continue };
            match old.get(usize::from(from)) {
                None => refuse!("car {index} keeps car {from}, which vehicle-{id} does not have"),
                Some(model) if model != part.part.model.as_str() => {
                    refuse!(
                        "car {index} keeps car {from}, a {model}, as a {}",
                        part.part.model
                    )
                }
                Some(_) => {}
            }
            if !kept.insert(from) {
                refuse!("car {from} kept twice");
            }
        }
        let bought = i64::try_from(replace.consist.len() - kept.len()).unwrap_or(i64::MAX);
        let sold = i64::try_from(old.len() - kept.len()).unwrap_or(i64::MAX);
        // Net of what the cars left out bring: a negative charge pays in.
        self.charge(
            company,
            VEHICLE_COST
                .saturating_mul(bought)
                .saturating_sub(VEHICLE_COST.saturating_mul(sold) / 2),
        )?;
        let vehicle = self.vehicles.get_mut(&id).expect("checked above");
        vehicle.consist = replace
            .consist
            .iter()
            .map(|part| part.part.model.as_str().to_owned())
            .collect();
        vehicle.load = vehicle
            .load
            .min(CAR_CAPACITY.saturating_mul(u32::try_from(vehicle.consist.len()).unwrap_or(0)));
        Ok(())
    }

    fn own_vehicles(
        &self,
        ids: impl Iterator<Item = u32>,
        company: u32,
    ) -> Result<Vec<u32>, Refusal> {
        let mut seen = BTreeSet::new();
        for id in ids {
            match self.vehicles.get(&id) {
                None => refuse!("no vehicle-{id}"),
                Some(vehicle) if vehicle.owner != company => {
                    refuse!("vehicle-{id} is company-{}'s", vehicle.owner)
                }
                Some(_) => {}
            }
            if !seen.insert(id) {
                refuse!("vehicle-{id} named twice");
            }
        }
        Ok(seen.into_iter().collect())
    }

    fn simulate(&mut self, step: u64) {
        let mut rng = SplitMix64::new(self.rng);
        self.now = step;
        self.prospections_end(&mut rng);
        let served: BTreeSet<u32> = self
            .lines
            .values()
            .flat_map(|line| line.stops.iter().map(|(station, _)| *station))
            .collect();
        for id in served {
            if let Some(station) = self.stations.get_mut(&id) {
                let arrivals = u32::try_from(rng.below(3)).unwrap_or(0);
                station.waiting = (station.waiting + arrivals).min(MAX_WAITING);
            }
        }
        let Self {
            lines,
            stations,
            vehicles,
            companies,
            delivered,
            ..
        } = self;
        for vehicle in vehicles.values_mut() {
            let Some(line) = vehicle.line.and_then(|id| lines.get(&id)) else {
                continue;
            };
            let count = line.stops.len();
            if count < 2 {
                continue;
            }
            let next = usize::from(vehicle.next_stop) % count;
            let previous = (next + count - 1) % count;
            let (Some(from), Some(to)) = (
                stations.get(&line.stops[previous].0),
                stations.get(&line.stops[next].0),
            ) else {
                continue;
            };
            let segment = u32::try_from(metres(from.at, to.at) / 10)
                .unwrap_or(u32::MAX)
                .max(10);
            vehicle.progress += 1 + u32::try_from(rng.below(2)).unwrap_or(0);
            if vehicle.progress < segment {
                continue;
            }
            vehicle.progress = 0;
            *delivered += u64::from(vehicle.load);
            if let Some(owner) = companies.get_mut(&vehicle.owner) {
                owner.money += i64::from(vehicle.load) * FARE;
            }
            let capacity = CAR_CAPACITY
                .saturating_mul(u32::try_from(vehicle.consist.len()).unwrap_or(u32::MAX));
            vehicle.load = 0;
            if let Some(station) = stations.get_mut(&line.stops[next].0) {
                vehicle.load = capacity.min(station.waiting);
                station.waiting -= vehicle.load;
            }
            vehicle.next_stop = u16::try_from((next + 1) % count).unwrap_or(0);
        }
        if step.is_multiple_of(UPKEEP_EVERY) {
            for vehicle in vehicles.values() {
                if let Some(owner) = companies.get_mut(&vehicle.owner) {
                    owner.money -= UPKEEP;
                }
            }
        }
        self.rng = rng.state();
    }
}

/// A line as a check sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineView {
    pub name: String,
    pub stops: usize,
    pub vehicles: usize,
}

/// What a replica shows of its world, for the harness's checks. The real
/// game's hook answers the same questions from the game's own state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// The money of each player's company, in the order they joined, so
    /// actors first.
    pub money: Vec<Option<i64>>,
    /// Each player's company, in the order they joined.
    pub company_of: Vec<Option<CompanyId>>,
    pub companies: usize,
    pub street_edges: usize,
    pub track_edges: usize,
    pub constructions: usize,
    pub stations: usize,
    pub depots: usize,
    /// Stops, signals and waypoints on edges.
    pub edge_objects: usize,
    pub lines: BTreeMap<LineId, LineView>,
    pub vehicles: usize,
    /// Vehicles on no line.
    pub idle: usize,
    pub delivered: u64,
    pub ignored: u64,
    pub terrain_cells: usize,
    /// Prospections under way.
    pub prospections: usize,
    /// Industries prospecting found.
    pub industries: usize,
}

/// One replica of the model.
#[derive(Debug, Clone)]
pub struct ModelWorld {
    state: State,
    drift_at: Option<u64>,
    /// Why each ignored action was ignored, by event sequence number. Not
    /// part of the world: for the harness's report.
    ignored: Vec<(u64, String)>,
}

impl ModelWorld {
    pub fn new(seed: u64) -> Self {
        Self {
            state: State {
                rng: seed,
                ..State::default()
            },
            drift_at: None,
            ignored: Vec::new(),
        }
    }

    /// Makes this replica's simulation deviate once at `step`.
    pub fn with_drift(mut self, step: u64) -> Self {
        self.drift_at = Some(step);
        self
    }

    pub fn save(&self) -> Vec<u8> {
        postcard::to_stdvec(&self.state).expect("model state always encodes")
    }

    pub fn load(bytes: &[u8]) -> Option<Self> {
        Some(Self {
            state: postcard::from_bytes(bytes).ok()?,
            drift_at: None,
            ignored: Vec::new(),
        })
    }

    /// The players in the order they joined.
    pub fn players(&self) -> &[PlayerId] {
        &self.state.players
    }

    /// Why each action that changed nothing was ignored.
    pub fn ignored(&self) -> &[(u64, String)] {
        &self.ignored
    }

    pub fn apply(&mut self, event: &Event) {
        match &event.body {
            EventBody::PlayerJoined { player, .. } => self.state.join(*player),
            EventBody::PlayerLeft { .. } | EventBody::Save => {}
            EventBody::Command {
                player, payload, ..
            } => {
                let outcome = Action::from_payload(payload)
                    .map_err(|error| error.to_string())
                    .and_then(|action| {
                        let mut next = self.state.clone();
                        next.act(player, &action)?;
                        Ok(next)
                    });
                match outcome {
                    Ok(next) => self.state = next,
                    Err(why) => {
                        self.state.ignored += 1;
                        self.ignored.push((event.seq, why));
                    }
                }
            }
        }
    }

    pub fn step(&mut self, step: u64) {
        self.state.simulate(step);
        if self.drift_at == Some(step) {
            let mut rng = SplitMix64::new(self.state.rng);
            rng.next_u64();
            self.state.rng = rng.state();
        }
    }

    pub fn lanes(&self) -> Vec<LaneDigest> {
        let s = &self.state;
        let waiting: Vec<(u32, u32)> = s
            .stations
            .iter()
            .map(|(id, st)| (*id, st.waiting))
            .collect();
        let stations: Vec<(u32, P)> = s.stations.iter().map(|(id, st)| (*id, st.at)).collect();
        vec![
            lane_digest(lane::NETWORK, &(&s.edges, &s.terrain)),
            lane_digest(
                lane::CONSTRUCTIONS,
                &(
                    &s.constructions,
                    &s.objects,
                    &stations,
                    s.next_station,
                    &s.prospections,
                    &s.industries,
                    s.next_industry,
                ),
            ),
            lane_digest(lane::LINES, &(&s.lines, s.next_line)),
            lane_digest(lane::VEHICLES, &(&s.vehicles, &waiting, s.next_vehicle)),
            lane_digest(
                lane::ECONOMY,
                &(
                    &s.players,
                    &s.member_of,
                    &s.companies,
                    s.next_company,
                    s.delivered,
                    s.ignored,
                    s.rng,
                ),
            ),
        ]
    }

    pub fn observe(&self) -> Observation {
        let s = &self.state;
        let lines = s
            .lines
            .iter()
            .map(|(id, line)| {
                let vehicles = s.vehicles.values().filter(|v| v.line == Some(*id)).count();
                (
                    LineId(*id),
                    LineView {
                        name: line.name.clone(),
                        stops: line.stops.len(),
                        vehicles,
                    },
                )
            })
            .collect();
        let edges = |n: u8| s.edges.keys().filter(|(of, _, _)| *of == n).count();
        Observation {
            money: s
                .players
                .iter()
                .map(|player| {
                    let company = s.member_of.get(player)?;
                    Some(s.companies.get(company)?.money)
                })
                .collect(),
            company_of: s
                .players
                .iter()
                .map(|player| s.member_of.get(player).map(|c| CompanyId(*c)))
                .collect(),
            companies: s.companies.len(),
            street_edges: edges(net(Network::Street)),
            track_edges: edges(net(Network::Track)),
            constructions: s.constructions.len(),
            stations: s.stations.len(),
            depots: s
                .constructions
                .values()
                .filter(|c| c.kind == Kind::Depot)
                .count(),
            edge_objects: s.objects.len(),
            lines,
            vehicles: s.vehicles.len(),
            idle: s.vehicles.values().filter(|v| v.line.is_none()).count(),
            delivered: s.delivered,
            ignored: s.ignored,
            terrain_cells: s.terrain.len(),
            prospections: s.prospections.len(),
            industries: s.industries.len(),
        }
    }
}

fn lane_digest<T: Serialize>(lane: u16, value: &T) -> LaneDigest {
    let bytes = postcard::to_stdvec(value).expect("model state always encodes");
    let hash = digest(&SHA256, &bytes);
    let mut out = [0; 32];
    out.copy_from_slice(hash.as_ref());
    LaneDigest {
        lane,
        digest: FixedBytes(out),
    }
}

#[cfg(test)]
mod tests {
    use tpf3mp_proto::{Platform, Text, action::LoanTerms};

    use super::*;

    fn event(seq: u64, body: EventBody) -> Event {
        Event {
            seq,
            step: seq,
            body,
        }
    }

    fn loan(amount: i64) -> LoanTerms {
        LoanTerms {
            kind: Text::new("Small").unwrap(),
            amount,
            duration: 1_095_000,
            percentage: 30_000,
            birth_day: None,
            cooldown_until: None,
            last_pay_day: None,
            times_paid: None,
            id: None,
        }
    }

    fn act(world: &mut ModelWorld, seq: u64, player: PlayerId, action: &Action) {
        world.apply(&event(
            seq,
            EventBody::Command {
                player,
                client_seq: seq,
                payload: action.to_payload().unwrap(),
            },
        ));
    }

    #[test]
    fn a_loan_pays_its_amount_in_and_paying_it_back_takes_it_out() {
        let player = PlayerId(FixedBytes([1; 32]));
        let mut world = ModelWorld::new(1);
        world.apply(&event(
            1,
            EventBody::PlayerJoined {
                player,
                name: Text::new("p1").unwrap(),
                platform: Platform::current(),
            },
        ));
        let take = Action::Loan(Box::new(LoanOp::Take {
            next: loan(7_000_000),
            offer: loan(5_000_000),
        }));
        act(&mut world, 2, player, &take);
        assert_eq!(world.observe().money[0], Some(START_MONEY + 5_000_000));
        act(
            &mut world,
            3,
            player,
            &Action::Loan(Box::new(LoanOp::Repay {
                loan: loan(5_000_000),
            })),
        );
        assert_eq!(world.observe().money[0], Some(START_MONEY));
        // More than the company has is refused, and changes nothing.
        act(
            &mut world,
            4,
            player,
            &Action::Loan(Box::new(LoanOp::Repay {
                loan: loan(START_MONEY + 1),
            })),
        );
        assert_eq!(world.observe().money[0], Some(START_MONEY));
        assert_eq!(world.ignored().len(), 1);
    }

    /// A replacement is the company's own vehicle's, keeps only cars the
    /// vehicle has, each once and of its own model, and pays for the cars it
    /// buys net of those it leaves out; the vehicle keeps its id.
    #[test]
    fn a_replacement_keeps_the_vehicles_own_cars_and_pays_for_new_ones() {
        use crate::regress::library::{
            BUS_DEPOT, LOCOMOTIVE, WAGON, at, buy, construction, replace,
        };

        let (one, two) = (PlayerId(FixedBytes([1; 32])), PlayerId(FixedBytes([2; 32])));
        let mut world = ModelWorld::new(1);
        for (seq, (player, name)) in [(one, "p1"), (two, "p2")].into_iter().enumerate() {
            world.apply(&event(
                seq as u64 + 1,
                EventBody::PlayerJoined {
                    player,
                    name: Text::new(name).unwrap(),
                    platform: Platform::current(),
                },
            ));
        }
        act(
            &mut world,
            3,
            one,
            &construction(BUS_DEPOT, at(0, 0), "Yard"),
        );
        act(
            &mut world,
            4,
            one,
            &buy(BUS_DEPOT, at(0, 0), &[LOCOMOTIVE, WAGON]),
        );
        let money = world.observe().money[0].unwrap();
        let refusals = [
            // Someone else's vehicle; one that is not there.
            (two, replace(0, &[(LOCOMOTIVE, Some(0))])),
            (one, replace(5, &[(LOCOMOTIVE, Some(0))])),
            // No cars; a car the vehicle does not have; another model kept;
            // a car kept twice.
            (one, replace(0, &[])),
            (one, replace(0, &[(LOCOMOTIVE, Some(2))])),
            (one, replace(0, &[(WAGON, Some(0))])),
            (one, replace(0, &[(WAGON, Some(1)), (WAGON, Some(1))])),
        ];
        for (seq, (player, action)) in refusals.iter().enumerate() {
            act(&mut world, 5 + seq as u64, *player, action);
        }
        let why: Vec<String> = world.ignored().iter().map(|(_, w)| w.clone()).collect();
        assert_eq!(
            why,
            [
                "vehicle-0 is company-0's".to_owned(),
                "no vehicle-5".to_owned(),
                "a replacement of no cars".to_owned(),
                "car 0 keeps car 2, which vehicle-0 does not have".to_owned(),
                format!("car 0 keeps car 0, a {LOCOMOTIVE}, as a {WAGON}"),
                "car 1 kept twice".to_owned(),
            ]
        );
        assert_eq!(
            world.observe().money[0],
            Some(money),
            "refusals cost nothing"
        );
        // The locomotive kept, the coach left out, two new coaches: two
        // bought, one sold at half.
        act(
            &mut world,
            20,
            one,
            &replace(0, &[(LOCOMOTIVE, Some(0)), (WAGON, None), (WAGON, None)]),
        );
        assert_eq!(world.ignored().len(), 6, "{:?}", world.ignored());
        assert_eq!(
            world.observe().money[0],
            Some(money - 2 * VEHICLE_COST + VEHICLE_COST / 2)
        );
        assert_eq!(world.state.vehicles[&0].consist.len(), 3);
    }

    #[test]
    fn a_prospection_ends_alike_on_every_replica_after_its_time() {
        use crate::regress::library::prospect;

        let player = PlayerId(FixedBytes([1; 32]));
        let replica = || {
            let mut world = ModelWorld::new(7);
            world.apply(&event(
                1,
                EventBody::PlayerJoined {
                    player,
                    name: Text::new("p1").unwrap(),
                    platform: Platform::current(),
                },
            ));
            world
        };
        let (mut a, mut b) = (replica(), replica());
        // Many towns, so some prospections find an industry and some not.
        for (seq, town) in (2..).zip(0..20u32) {
            for world in [&mut a, &mut b] {
                act(
                    world,
                    seq,
                    player,
                    &prospect(town, "coal", &["mine", "pit"]),
                );
            }
        }
        // A second one for the same town and cargo changes nothing.
        act(&mut a, 30, player, &prospect(3, "coal", &["mine"]));
        assert_eq!(a.observe().prospections, 20);
        assert_eq!(a.ignored().len(), 1);
        act(&mut b, 30, player, &prospect(3, "coal", &["mine"]));
        for step in 1..PROSPECTION_STEPS {
            a.step(step);
            b.step(step);
        }
        assert_eq!(a.observe().prospections, 20, "not before its time");
        a.step(PROSPECTION_STEPS);
        b.step(PROSPECTION_STEPS);
        let seen = a.observe();
        assert_eq!(seen.prospections, 0);
        assert!(
            seen.industries > 0 && seen.industries < 20,
            "{} found",
            seen.industries
        );
        assert_eq!(
            a.lanes(),
            b.lanes(),
            "the same industries, at the same places"
        );
    }

    /// A street drawn onto another's middle, as TF3's street tool proposes
    /// it: the old street's node there goes with its two edges, and the old
    /// street is rebuilt through the new junction in its own kind.
    #[test]
    fn a_junction_rebuilds_the_street_it_joins_in_its_own_kind() {
        use tpf3mp_proto::{
            BoundedVec,
            action::{EdgeKind, EdgeRef, Link, NodeRef, RoadBuild, Tram},
        };

        use crate::regress::library::{at, new, node, polyline, road};

        let player = PlayerId(FixedBytes([1; 32]));
        let mut world = ModelWorld::new(1);
        world.apply(&event(
            1,
            EventBody::PlayerJoined {
                player,
                name: Text::new("p1").unwrap(),
                platform: Platform::current(),
            },
        ));
        // The street A-M-B.
        act(
            &mut world,
            2,
            player,
            &road(vec![new(at(400, 0)), new(at(450, 0)), new(at(500, 0))]),
        );
        assert_eq!(world.observe().street_edges, 2);
        let street_edge = |a, b| EdgeRef {
            network: Network::Street,
            ends: EdgeEnds { a, b },
        };
        let junction = |removals: Vec<EdgeRef>, removed_nodes: Vec<NodeRef>| {
            let mut lines = polyline(
                vec![
                    new(at(450, 100)),
                    new(at(452, 0)),
                    node(at(400, 0), Network::Street),
                    node(at(500, 0), Network::Street),
                ],
                &Structure::Ground,
            );
            // The new street's one link, then the old street rebuilt.
            let mut links = vec![lines.links.to_vec()[0].clone()];
            for (from, to) in [(2, 1), (1, 3)] {
                links.push(Link {
                    from,
                    to,
                    kind: Some(EdgeKind {
                        network: Network::Street,
                        template: Text::new("street/country.street_template").unwrap(),
                        style: None,
                    }),
                    ..links[0].clone()
                });
            }
            lines = Polyline::new(
                lines.vertices,
                BoundedVec::new(links).unwrap(),
                BoundedVec::new(removals).unwrap(),
            )
            .unwrap()
            .with_removed_nodes(BoundedVec::new(removed_nodes).unwrap());
            Action::BuildRoad(RoadBuild {
                street: Text::new("street/town.street_template").unwrap(),
                style: None,
                bus_lane: false,
                tram: Tram::None,
                polyline: lines,
            })
        };
        let both = || {
            vec![
                street_edge(at(400, 0), at(450, 0)),
                street_edge(at(450, 0), at(500, 0)),
            ]
        };
        let street_node = |pos| NodeRef {
            network: Network::Street,
            at: pos,
        };
        // Removing a node the build joins, or one that keeps an edge, is
        // refused and changes nothing.
        act(
            &mut world,
            3,
            player,
            &junction(both(), vec![street_node(at(400, 0))]),
        );
        act(
            &mut world,
            4,
            player,
            &junction(
                vec![street_edge(at(400, 0), at(450, 0))],
                vec![street_node(at(450, 0))],
            ),
        );
        assert_eq!(world.observe().street_edges, 2);
        let why: Vec<&str> = world.ignored().iter().map(|(_, w)| w.as_str()).collect();
        assert_eq!(why[0], "the build removes a node it joins");
        assert!(why[1].ends_with("still has edges"), "{why:?}");
        // The tool's own: the old junction's node goes.
        act(
            &mut world,
            5,
            player,
            &junction(both(), vec![street_node(at(450, 0))]),
        );
        assert_eq!(world.ignored().len(), 2, "{:?}", world.ignored());
        assert_eq!(world.observe().street_edges, 3);
        let kinds: Vec<&str> = world
            .state
            .edges
            .values()
            .map(|e| e.kind.as_str())
            .collect();
        assert_eq!(
            kinds
                .iter()
                .filter(|k| **k == "street/country.street_template")
                .count(),
            2,
            "{kinds:?}"
        );
    }
}
