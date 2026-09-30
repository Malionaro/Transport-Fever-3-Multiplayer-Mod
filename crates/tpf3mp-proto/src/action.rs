//! What a player did, in terms every replica can resolve against its own
//! world: the portable action schema. `docs/BUILDING.md` ("The action
//! schema") explains what each action carries and why.
//!
//! An action names nothing by engine entity id, which differs between games
//! and is recycled within one. It uses three kinds of reference only:
//!
//! - **world positions**, as fixed-point integers in millimetres ([`Pos`]);
//!   geometry is matched by position within the tolerances BUILDING.md gives;
//! - **resource file names** ([`ResName`]), such as a street type or a
//!   vehicle model, which are the same in every game with the same content;
//! - **canonical ids** ([`CompanyId`], [`LineId`], [`VehicleId`],
//!   [`StationId`]), which the server assigns to what an action creates.
//!
//! The acting player, and so their company, is not part of an action: the
//! event that carries it names the player.
//!
//! An action travels inside the opaque [`Payload`] of an intent, behind the
//! [`ACTION_SCHEMA_VERSION`] ([`Action::to_payload`]). Decoding enforces every
//! bound, and a polyline's links must name its own vertices, so a decoded
//! action is well-formed. Variants are identified by position: append, never
//! reorder, and bump the schema version when an existing variant changes.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    BoundedVec, Text,
    bytes::{Payload, PayloadTooLarge},
};

/// Version of the action schema, the first thing in an action's payload.
/// Players in one room run the same mod, so their versions match; a payload
/// of any other version is refused, never guessed at.
pub const ACTION_SCHEMA_VERSION: u32 = 9;

/// Most vertices, and most links, in one road or track build. A 23-segment
/// track was the longest single TPF2 build measured.
pub const MAX_VERTICES: usize = 512;
pub const MAX_LINKS: usize = 512;
/// Most edges one action removes or bulldozes.
pub const MAX_EDGES: usize = 256;
/// Most parameters of one construction, nested modules counted one by one.
pub const MAX_PARAMS: usize = 1024;
/// Most vehicle models in one consist.
pub const MAX_CONSIST: usize = 64;
/// Most vehicles one sell or line assignment names.
pub const MAX_VEHICLES: usize = 256;
/// Most stops on a line.
pub const MAX_LINE_STOPS: usize = 256;
/// Most cells in one terraform stroke. TPF2's largest measured, a smooth, was
/// 83 by 59.
pub const MAX_TERRAIN_CELLS: usize = 8192;
/// Most compartments of one vehicle.
pub const MAX_COMPARTMENTS: usize = 16;
/// Most cargo types a stop's loading rules list.
pub const MAX_CARGOS: usize = 64;
/// Most other terminals one line stop may use.
pub const MAX_ALTERNATIVES: usize = 32;
/// Most transport modes a line lists.
pub const MAX_MODES: usize = 32;
/// Most industry types one prospection may find. Build 40408's economy has
/// at most a handful per cargo.
pub const MAX_INDUSTRY_TYPES: usize = 32;

/// A resource file name as the game lists it, such as
/// `street/standard/town_medium_new.lua` or a vehicle's `.mdl`.
pub type ResName = Text<128>;
/// A name a player gives something: a construction, a line, a company.
pub type ObjectName = Text<64>;

/// A position in the world, in millimetres, on the game's own axes. `i32`
/// reaches ±2,147 km, far past the largest map (65.5 km on a side).
/// Capture rounds metres to the nearest millimetre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// A position on the ground plane, in millimetres.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pos2 {
    pub x: i32,
    pub y: i32,
}

/// A curve's Hermite tangent at one end, in millimetres: its length is part
/// of the curve's shape, so it is not normalised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tangent {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// A direction as a unit vector, in millionths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UnitDir {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

macro_rules! canonical_id {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        ///
        /// Assigned by the server when the thing is created; never an engine
        /// entity id. Each replica maps it to its own entity.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub struct $name(pub u32);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }
    };
}

canonical_id!(
    /// A company.
    CompanyId,
    "company-"
);
canonical_id!(
    /// A line.
    LineId,
    "line-"
);
canonical_id!(
    /// A vehicle, a whole consist.
    VehicleId,
    "vehicle-"
);
canonical_id!(
    /// A station group, which line stops name.
    StationId,
    "station-"
);
canonical_id!(
    /// A town. Towns come with the room's world, not from an action: every
    /// game binds them, lowest entity first, at the room's first update.
    TownId,
    "town-"
);

/// The two transport networks. A road node and a track node can stand at
/// the same place (a level crossing), so every reference to a node or edge
/// says which network it is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Network {
    Street,
    Track,
}

/// An existing edge, named by its end positions. Orientation is not part of
/// the identity: which end is `node0` differs between games.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EdgeEnds {
    pub a: Pos,
    pub b: Pos,
}

/// An existing edge in a given network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EdgeRef {
    pub network: Network,
    pub ends: EdgeEnds,
}

/// An existing node, named by its position, in a given network: matched
/// within 1.5 m horizontally, the nearest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeRef {
    pub network: Network,
    pub at: Pos,
}

/// An existing construction, named by its file and its position (the
/// transform's origin). Matched within 2 m.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConstructionRef {
    pub file: ResName,
    pub at: Pos,
}

/// What the originator's engine made of a polyline vertex. Receivers repeat
/// the decision instead of re-deriving it from a world that may have drifted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolve {
    /// A new node, attached to nothing that already existed.
    New,
    /// The existing node of this network at the vertex (within 1.5 m,
    /// horizontally). A track vertex on a street node is a level crossing.
    Node(Network),
    /// A new node splitting this existing edge at the vertex. The halves
    /// keep the split edge's own type and flags.
    Split(EdgeRef),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vertex {
    pub pos: Pos,
    pub resolve: Resolve,
}

/// What an edge is built as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Structure {
    Ground,
    /// A bridge of this bridge type.
    Bridge(ResName),
    /// A tunnel of this tunnel type.
    Tunnel(ResName),
}

/// What an edge is built as when it is not the build's own street or track:
/// a piece of an existing street or track that the tool rebuilds around a new
/// junction or crossing keeps that road's kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeKind {
    pub network: Network,
    /// Its road template (TF3's `BaseEdge.roadTemplate`).
    pub template: ResName,
    /// Its road style; none for the template's own.
    pub style: Option<ResName>,
}

/// One new edge between two vertices of its polyline, by index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub from: u16,
    pub to: u16,
    pub tangent0: Tangent,
    pub tangent1: Tangent,
    pub structure: Structure,
    /// None for the build's own street or track, in the build's style.
    pub kind: Option<EdgeKind>,
}

/// The geometry of one road or track build, as the tool proposed it: new
/// edges between vertices, and the existing edges and nodes it removes (an
/// upgrade's, a span the build passes under, the stretch of road TF3's tools
/// rebuild around a new junction or crossing). A split vertex names the edge
/// it splits, which is then no removal: the receiver removes it as it splits
/// it.
///
/// Decoding checks that there is at least one link and that every link joins
/// two different vertices of this polyline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PolylineFields")]
pub struct Polyline {
    pub vertices: BoundedVec<Vertex, MAX_VERTICES>,
    pub links: BoundedVec<Link, MAX_LINKS>,
    /// Existing edges removed, of either network.
    pub removals: BoundedVec<EdgeRef, MAX_EDGES>,
    /// Existing nodes removed: TF3's tools move a junction that was near the
    /// new one onto it.
    pub removed_nodes: BoundedVec<NodeRef, MAX_EDGES>,
}

#[derive(Deserialize)]
struct PolylineFields {
    vertices: BoundedVec<Vertex, MAX_VERTICES>,
    links: BoundedVec<Link, MAX_LINKS>,
    removals: BoundedVec<EdgeRef, MAX_EDGES>,
    removed_nodes: BoundedVec<NodeRef, MAX_EDGES>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PolylineError {
    #[error("a build with no edges")]
    NoLinks,
    #[error("link {0} names a vertex the polyline does not have")]
    NoSuchVertex(usize),
    #[error("link {0} joins a vertex to itself")]
    Loop(usize),
}

impl Polyline {
    /// A polyline that removes no nodes; see [`Polyline::with_removed_nodes`].
    pub fn new(
        vertices: BoundedVec<Vertex, MAX_VERTICES>,
        links: BoundedVec<Link, MAX_LINKS>,
        removals: BoundedVec<EdgeRef, MAX_EDGES>,
    ) -> Result<Self, PolylineError> {
        if links.is_empty() {
            return Err(PolylineError::NoLinks);
        }
        for (index, link) in links.iter().enumerate() {
            let count = vertices.len();
            if usize::from(link.from) >= count || usize::from(link.to) >= count {
                return Err(PolylineError::NoSuchVertex(index));
            }
            if link.from == link.to {
                return Err(PolylineError::Loop(index));
            }
        }
        Ok(Self {
            vertices,
            links,
            removals,
            removed_nodes: BoundedVec::empty(),
        })
    }

    /// This polyline, removing these existing nodes too.
    #[must_use]
    pub fn with_removed_nodes(mut self, nodes: BoundedVec<NodeRef, MAX_EDGES>) -> Self {
        self.removed_nodes = nodes;
        self
    }
}

impl TryFrom<PolylineFields> for Polyline {
    type Error = PolylineError;

    fn try_from(fields: PolylineFields) -> Result<Self, PolylineError> {
        Ok(Self::new(fields.vertices, fields.links, fields.removals)?
            .with_removed_nodes(fields.removed_nodes))
    }
}

/// The tram track a street carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tram {
    None,
    Plain,
    Electric,
}

/// Transport Fever 3 types every street and track by a road template and a
/// road style (`BaseEdge.roadTemplate`, `roadStyle`); TPF2 had one type
/// file. A build names its template where TPF2 named the type, and its
/// style where the game has one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoadBuild {
    /// The street's type: its road template on TF3.
    pub street: ResName,
    /// The street's road style, on TF3.
    pub style: Option<ResName>,
    pub bus_lane: bool,
    pub tram: Tram,
    pub polyline: Polyline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackBuild {
    /// The track's type: its road template on TF3.
    pub track: ResName,
    /// The track's road style, on TF3.
    pub style: Option<ResName>,
    pub catenary: bool,
    pub polyline: Polyline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bulldoze {
    /// Edges of one network, matched by their ends within 1 m. Nodes left
    /// with no edge go with them.
    Edges {
        network: Network,
        edges: BoundedVec<EdgeEnds, MAX_EDGES>,
    },
    Construction(ConstructionRef),
    /// The stop, signal or waypoint of this model on this edge, nearest to
    /// `at`.
    EdgeObject {
        edge: EdgeRef,
        at: Pos,
        model: ResName,
    },
}

/// Where a construction stands: the game's 4x4 matrix as its rotation and
/// scale part, in millionths, and its origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transform {
    /// Elements 1-3, 5-7 and 9-11 of the game's matrix, in its order.
    pub basis: [i32; 9],
    /// Elements 13-15.
    pub origin: Pos,
}

/// A construction parameter's value. Lua numbers with no fraction are
/// `Int`; others are `Fixed`, in millionths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParamValue {
    Int(i64),
    Fixed(i64),
    Bool(bool),
    Text(Text<128>),
}

/// One leaf of a construction's parameter table. Nested tables flatten
/// into paths: `modules[3801].name` is the `name` field of the entry at
/// integer key 3801 of `modules`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Param {
    pub key: Text<128>,
    pub value: ParamValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstructionBuild {
    pub file: ResName,
    pub transform: Transform,
    /// Every parameter, `seed` included: without it the game refuses.
    pub params: BoundedVec<Param, MAX_PARAMS>,
    /// Never empty in practice: an unnamed construction's children crash
    /// the game when clicked.
    pub name: ObjectName,
    /// The construction this one replaces: a module edit or an upgrade.
    pub replaces: Option<ConstructionRef>,
    /// The street and track changes the tool made with it, built in the same
    /// proposal: a station placed by a road joins it through a junction the
    /// road is rebuilt around, and an entrance edge to the station's own
    /// street node, which the construction then meets at the same place.
    /// Every link names its kind: a construction has no street of its own.
    pub connection: Option<Box<Polyline>>,
}

/// A fraction, in millionths: in Lua a plain number (0.25 is 250,000),
/// wherever it stands, a list included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Fraction(pub i32);

/// A colour as the game keeps one (a `Vec3f`): red, green and blue, each a
/// fraction in millionths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tint {
    pub r: i32,
    pub g: i32,
    pub b: i32,
}

/// What one compartment of a vehicle loads (the game's `LoadConfig`): which
/// of its model's load configurations, and the cargo type, by the game's
/// numbering of its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Load {
    pub config: i32,
    pub cargo: i32,
}

/// One vehicle of a consist, as the depot's store configures it (the game's
/// `VehiclePart`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsistPart {
    /// Its model, as the game lists it (`api.res.modelRep`).
    pub model: ResName,
    /// It faces backwards.
    pub reversed: bool,
    /// Each compartment's load, in order.
    pub loads: BoundedVec<Load, MAX_COMPARTMENTS>,
    pub color: Tint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuyVehicle {
    pub depot: ConstructionRef,
    /// The consist, front to back.
    pub consist: BoundedVec<ConsistPart, MAX_CONSIST>,
    /// Its groups, front to back: each the number of its vehicles (a multiple
    /// unit is one group).
    pub groups: BoundedVec<u8, MAX_CONSIST>,
    /// For each group, the multiple unit's file, or empty.
    pub multiple_units: BoundedVec<Text<128>, MAX_CONSIST>,
}

/// One vehicle of a replacement consist: the part as a purchase carries it,
/// and whether it is one the vehicle has already.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplacedPart {
    pub part: ConsistPart,
    /// The index, from 0, of the vehicle's own part this one keeps, with its
    /// age and wear, of the same model; none for a part bought new. TF3's
    /// store keeps a part the player left in the consist as it was (its
    /// purchase time), and buys the rest.
    pub kept: Option<u8>,
}

/// The vehicle window's "modify" and "replace" (`makeVehicleReplaceCmd`):
/// one vehicle's consist swapped for another, the vehicle staying the one
/// its line and orders name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplaceVehicle {
    pub vehicle: VehicleId,
    /// The new consist, front to back.
    pub consist: BoundedVec<ReplacedPart, MAX_CONSIST>,
    /// Its groups, as [`BuyVehicle::groups`].
    pub groups: BoundedVec<u8, MAX_CONSIST>,
    /// For each group, the multiple unit's file, or empty.
    pub multiple_units: BoundedVec<Text<128>, MAX_CONSIST>,
}

/// How long vehicles load at a stop (the game's `Line.LoadMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoadMode {
    LoadIfAvailable,
    FullLoadAny,
    FullLoadAll,
    LegacyUnloadOnly,
}

/// What vehicles load and unload at a stop (the game's `Line.StopConfig`),
/// cargo type by cargo type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopRules {
    pub load: BoundedVec<bool, MAX_CARGOS>,
    /// The most of a vehicle's capacity each cargo type may take.
    pub max_load: BoundedVec<Fraction, MAX_CARGOS>,
    pub force_unload: bool,
    pub destroy_for_config_change: bool,
    pub destroy_for_refresh: bool,
}

/// A terminal of a station group: the station's place in the group, and the
/// terminal's in the station, both from 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Terminal {
    pub station: u16,
    pub terminal: u16,
}

/// One stop of a line, as the game keeps it (`Line.Stop`), its station group
/// by canonical id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineStop {
    pub group: StationId,
    /// The terminal the line uses.
    pub terminal: Terminal,
    /// Other terminals it may use.
    pub alternatives: BoundedVec<Terminal, MAX_ALTERNATIVES>,
    pub load_mode: LoadMode,
    /// Waiting times, in the game's seconds, in millionths.
    pub min_wait: i64,
    pub max_wait: i64,
    pub max_extra_wait: i64,
    pub rules: StopRules,
}

/// A line as the game keeps it (`Line`): its stops, the transport modes that
/// may run it (by the game's numbering), and its settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineData {
    pub stops: BoundedVec<LineStop, MAX_LINE_STOPS>,
    pub modes: BoundedVec<u16, MAX_MODES>,
    pub custom_filters: bool,
    /// In millionths.
    pub reservation_priority: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateLine {
    pub name: ObjectName,
    pub color: Tint,
    pub line: LineData,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineChange {
    Rename(ObjectName),
    Recolor(Tint),
    /// The whole line anew, as the line editor built it.
    Update(LineData),
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditLine {
    pub line: LineId,
    pub change: LineChange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignLine {
    pub vehicles: BoundedVec<VehicleId, MAX_VEHICLES>,
    /// The line, or none to take the vehicles off their line.
    pub line: Option<LineId>,
    /// Index of the stop the vehicles head for first; none for the game's
    /// choice, the next stop each vehicle can reach (the line manager's
    /// "Next Reachable Stop", stop index -1 in TF3's command).
    pub first_stop: Option<u16>,
}

/// What the vehicle window does to one vehicle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VehicleChange {
    /// Stopped by the player (true), or running again (false).
    Stop(bool),
    /// To the nearest depot, sold on arrival if `sell`.
    ToDepot {
        sell: bool,
    },
    Reverse,
    /// Leaves its terminal now.
    Depart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VehicleOp {
    pub vehicle: VehicleId,
    pub change: VehicleChange,
}

/// A stop placed on an existing edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceStop {
    pub edge: EdgeRef,
    /// Where along the edge.
    pub at: Pos,
    /// The engine's side flag, which is not the geometric side.
    pub left: bool,
    /// The edge's direction at `at` on the originator; a receiver whose edge
    /// runs the other way flips `left`.
    pub direction: UnitDir,
    /// The stop's construction (Transport Fever 3 builds a stop as one,
    /// e.g. `stations/street/small_stops/small_new.con`).
    pub model: ResName,
    /// A stop on both sides at once (a `_twosided` construction): `left`
    /// names the side the originator's tool put first.
    #[serde(default)]
    pub two_sided: bool,
}

/// One terrain cell: the height it is set to and the height it had, in
/// millimetres.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainCell {
    pub target: i32,
    pub before: i32,
}

/// A terraform stroke as the grid the game computed. Decoding checks that
/// the cells fill whole rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "TerraformFields")]
pub struct Terraform {
    /// The corner of the first cell.
    pub origin: Pos2,
    /// The side of a cell, in millimetres.
    pub cell: u32,
    /// Cells per row.
    pub columns: u16,
    /// Row by row, heights in millimetres.
    pub cells: BoundedVec<TerrainCell, MAX_TERRAIN_CELLS>,
}

#[derive(Deserialize)]
struct TerraformFields {
    origin: Pos2,
    cell: u32,
    columns: u16,
    cells: BoundedVec<TerrainCell, MAX_TERRAIN_CELLS>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("{cells} cells do not fill rows of {columns}, or cells have no size")]
pub struct GridError {
    pub cells: usize,
    pub columns: u16,
}

impl Terraform {
    pub fn new(
        origin: Pos2,
        cell: u32,
        columns: u16,
        cells: BoundedVec<TerrainCell, MAX_TERRAIN_CELLS>,
    ) -> Result<Self, GridError> {
        let whole_rows = columns != 0 && cells.len().is_multiple_of(usize::from(columns));
        if cell == 0 || cells.is_empty() || !whole_rows {
            return Err(GridError {
                cells: cells.len(),
                columns,
            });
        }
        Ok(Self {
            origin,
            cell,
            columns,
            cells,
        })
    }
}

impl TryFrom<TerraformFields> for Terraform {
    type Error = GridError;

    fn try_from(fields: TerraformFields) -> Result<Self, GridError> {
        Self::new(fields.origin, fields.cell, fields.columns, fields.cells)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompanyOp {
    Create {
        name: ObjectName,
    },
    /// The player plays for this company from now on.
    Join(CompanyId),
    Rename {
        company: CompanyId,
        name: ObjectName,
    },
    Delete(CompanyId),
    /// The company's colour, for its vehicles and lines. Appended under
    /// schema version 8: the variants before it keep their bytes.
    Recolor {
        company: CompanyId,
        color: Tint,
    },
}

/// A loan on its terms, as Transport Fever 3's loan script keeps it
/// (`game_mechanics/finance/loan.d.tl`), field for field. Nothing in it
/// names an entity: a loan is the acting player's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoanTerms {
    /// "Small", "Medium", "Large", "ExtraLarge" or "Custom".
    #[serde(rename = "type")]
    pub kind: Text<16>,
    /// In the game's money.
    pub amount: i64,
    /// In the game's milliseconds.
    pub duration: i64,
    /// The interest a year, in millionths: 0.03 is 30 000.
    pub percentage: i64,
    #[serde(rename = "birthDay")]
    pub birth_day: Option<i64>,
    #[serde(rename = "cooldownUntil")]
    pub cooldown_until: Option<i64>,
    #[serde(rename = "lastPayDay")]
    pub last_pay_day: Option<i64>,
    #[serde(rename = "timesPaid")]
    pub times_paid: Option<i64>,
    pub id: Option<i64>,
}

/// Taking or paying back a loan: the loan script's two events, with the
/// parameters the game's finance window sends them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoanOp {
    /// Take the loan `offer`; the game's window also sends the offer it
    /// drew to follow it (`next`), whose kind the script puts on cooldown.
    Take { next: LoanTerms, offer: LoanTerms },
    /// Pay back a loan taken, named by its terms and id.
    Repay { loan: LoanTerms },
}

/// Prospecting near a town for one cargo: what TF3's construction menu sends
/// the company script when the player picks a town with a prospection
/// (`gui/construction/construction_react_util.tl`, the event `Companies`
/// `spawnIndustry`), field for field. The game decides the rest, months
/// later, from its own state and the game time, the same in every game
/// (investigation/TPF3_PROSPECTING_2026-09-30.md).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prospect {
    pub town: TownId,
    /// The cargo prospected for, a cargo resource (`cargoType`).
    pub cargo: ResName,
    /// The industry types that may be found, as the economy tags them, in
    /// the originator's order: the game shuffles them with draws taken in
    /// this order (`types`).
    pub industries: BoundedVec<ResName, MAX_INDUSTRY_TYPES>,
    /// The company permit it uses (`permitKey`), if it names one.
    pub permit: Option<ResName>,
}

/// One player action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    BuildRoad(RoadBuild),
    BuildTrack(TrackBuild),
    Bulldoze(Bulldoze),
    BuildConstruction(ConstructionBuild),
    BuyVehicle(BuyVehicle),
    SellVehicle {
        vehicles: BoundedVec<VehicleId, MAX_VEHICLES>,
    },
    CreateLine(CreateLine),
    EditLine(EditLine),
    AssignLine(AssignLine),
    PlaceStop(PlaceStop),
    Terraform(Terraform),
    CompanyOp(CompanyOp),
    /// Boxed: its terms are larger than every other action.
    Loan(Box<LoanOp>),
    VehicleOp(VehicleOp),
    ReplaceVehicle(ReplaceVehicle),
    Prospect(Prospect),
    /// A notification's popup played its first sound: the game's
    /// Notifications script marks it so (its `initialSound` event), in every
    /// game, so the sound is not played again.
    NotificationSeen {
        notification: u32,
    },
}

#[derive(Debug, Error)]
pub enum ActionError {
    #[error("action schema {found}; this game speaks {ACTION_SCHEMA_VERSION}")]
    Schema { found: u32 },
    #[error("malformed action: {0}")]
    Malformed(#[from] postcard::Error),
    #[error("{0} unexpected bytes after the action")]
    TrailingBytes(usize),
    #[error(transparent)]
    TooLarge(#[from] PayloadTooLarge),
}

impl Action {
    /// The payload of an intent carrying this action: the schema version,
    /// then the action, both postcard-encoded.
    pub fn to_payload(&self) -> Result<Payload, ActionError> {
        let mut bytes = postcard::to_stdvec(&ACTION_SCHEMA_VERSION)?;
        bytes.extend(postcard::to_stdvec(self)?);
        Ok(Payload::new(bytes)?)
    }

    /// The action a payload carries. Refuses another schema version, and
    /// bytes after the action.
    pub fn from_payload(payload: &Payload) -> Result<Self, ActionError> {
        let (version, rest): (u32, _) = postcard::take_from_bytes(payload.as_bytes())?;
        if version != ACTION_SCHEMA_VERSION {
            return Err(ActionError::Schema { found: version });
        }
        let (action, rest) = postcard::take_from_bytes(rest)?;
        if !rest.is_empty() {
            return Err(ActionError::TrailingBytes(rest.len()));
        }
        Ok(action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(x: i32, y: i32, z: i32) -> Pos {
        Pos { x, y, z }
    }

    fn link(from: u16, to: u16) -> Link {
        Link {
            from,
            to,
            tangent0: Tangent { x: 1, y: 0, z: 0 },
            tangent1: Tangent { x: 1, y: 0, z: 0 },
            structure: Structure::Ground,
            kind: None,
        }
    }

    fn vertices(count: usize) -> BoundedVec<Vertex, MAX_VERTICES> {
        let list = (0..count)
            .map(|i| Vertex {
                pos: pos(i32::try_from(i).unwrap() * 1000, 0, 0),
                resolve: Resolve::New,
            })
            .collect();
        BoundedVec::new(list).unwrap()
    }

    #[test]
    fn polyline_links_must_name_its_vertices() {
        let links = |list| BoundedVec::new(list).unwrap();
        assert_eq!(
            Polyline::new(vertices(2), links(vec![]), BoundedVec::empty()),
            Err(PolylineError::NoLinks)
        );
        assert_eq!(
            Polyline::new(vertices(2), links(vec![link(0, 2)]), BoundedVec::empty()),
            Err(PolylineError::NoSuchVertex(0))
        );
        assert_eq!(
            Polyline::new(
                vertices(2),
                links(vec![link(0, 1), link(1, 1)]),
                BoundedVec::empty()
            ),
            Err(PolylineError::Loop(1))
        );
        assert!(Polyline::new(vertices(2), links(vec![link(1, 0)]), BoundedVec::empty()).is_ok());
    }

    #[test]
    fn decoding_checks_the_polyline() {
        // Encode an invalid polyline through the unchecked field struct's
        // layout: the same fields, in the same order.
        #[derive(Serialize)]
        struct Raw {
            vertices: BoundedVec<Vertex, MAX_VERTICES>,
            links: BoundedVec<Link, MAX_LINKS>,
            removals: BoundedVec<EdgeRef, MAX_EDGES>,
            removed_nodes: BoundedVec<NodeRef, MAX_EDGES>,
        }
        let raw = Raw {
            vertices: vertices(1),
            links: BoundedVec::new(vec![link(0, 1)]).unwrap(),
            removals: BoundedVec::empty(),
            removed_nodes: BoundedVec::empty(),
        };
        let bytes = postcard::to_stdvec(&raw).unwrap();
        assert!(postcard::from_bytes::<Polyline>(&bytes).is_err());
    }

    #[test]
    fn terraform_cells_fill_rows() {
        let cells = |n| {
            BoundedVec::new(vec![
                TerrainCell {
                    target: 1,
                    before: 0
                };
                n
            ])
            .unwrap()
        };
        let origin = Pos2 { x: 0, y: 0 };
        assert!(Terraform::new(origin, 4000, 3, cells(6)).is_ok());
        assert!(Terraform::new(origin, 4000, 3, cells(7)).is_err());
        assert!(Terraform::new(origin, 4000, 0, cells(6)).is_err());
        assert!(Terraform::new(origin, 0, 3, cells(6)).is_err());
        assert!(Terraform::new(origin, 4000, 3, cells(0)).is_err());
    }

    /// Guards against accidental wire changes, which the Lua mod's encoder
    /// would not notice. Update deliberately, with ACTION_SCHEMA_VERSION.
    #[test]
    fn wire_format_is_stable() {
        let action = Action::SellVehicle {
            vehicles: BoundedVec::new(vec![VehicleId(3), VehicleId(300)]).unwrap(),
        };
        let payload = action.to_payload().unwrap();
        assert_eq!(
            payload.as_bytes(),
            [
                9, // schema version
                5, // Action::SellVehicle
                2, 3, 0xac, 0x02, // two ids, varints
            ]
        );
        let track = Action::BuildTrack(TrackBuild {
            track: Text::new("t").unwrap(),
            style: Some(Text::new("s").unwrap()),
            catenary: true,
            polyline: Polyline::new(
                BoundedVec::new(vec![
                    Vertex {
                        pos: pos(-1, 1, 0),
                        resolve: Resolve::Node(Network::Street),
                    },
                    Vertex {
                        pos: pos(0, 0, 0),
                        resolve: Resolve::New,
                    },
                ])
                .unwrap(),
                BoundedVec::new(vec![link(0, 1)]).unwrap(),
                BoundedVec::new(vec![EdgeRef {
                    network: Network::Street,
                    ends: EdgeEnds {
                        a: pos(1, 0, 0),
                        b: pos(0, 1, 0),
                    },
                }])
                .unwrap(),
            )
            .unwrap()
            .with_removed_nodes(
                BoundedVec::new(vec![NodeRef {
                    network: Network::Track,
                    at: pos(0, 0, 1),
                }])
                .unwrap(),
            ),
        });
        assert_eq!(
            track.to_payload().unwrap().as_bytes(),
            [
                9, // schema version
                1, // Action::BuildTrack
                1, b't', 1, 1, b's', 1, // track, style Some("s"), catenary
                2, // two vertices
                1, 2, 0, 1, 0, // (-1, 1, 0) zigzag, Resolve::Node(Street)
                0, 0, 0, 0, // (0, 0, 0), Resolve::New
                1, // one link
                0, 1, 2, 0, 0, 2, 0, 0, 0, // 0 -> 1, tangents, Structure::Ground
                0, // the build's own kind
                1, 0, 2, 0, 0, 0, 2, 0, // a removal: Street, (1, 0, 0), (0, 1, 0)
                1, 1, 0, 0, 2, // a removed node: Track, (0, 0, 1)
            ]
        );
        // Appended with Prospect under schema version 7: the variants
        // before them keep their bytes.
        let replace = Action::ReplaceVehicle(ReplaceVehicle {
            vehicle: VehicleId(3),
            consist: BoundedVec::new(vec![ReplacedPart {
                part: ConsistPart {
                    model: Text::new("m").unwrap(),
                    reversed: true,
                    loads: BoundedVec::empty(),
                    color: Tint { r: 1, g: 0, b: 0 },
                },
                kept: Some(2),
            }])
            .unwrap(),
            groups: BoundedVec::new(vec![1]).unwrap(),
            multiple_units: BoundedVec::new(vec![Text::new("").unwrap()]).unwrap(),
        });
        assert_eq!(
            replace.to_payload().unwrap().as_bytes(),
            [
                9,  // schema version
                14, // Action::ReplaceVehicle
                3,  // vehicle-3
                1, 1, b'm', 1, 0, 2, 0, 0, // one part: model, reversed, no loads, colour
                1, 2, // kept: Some(2)
                1, 1, 1, 0, // groups { 1 }, multiple units { "" }
            ]
        );
        let prospect = Action::Prospect(Prospect {
            town: TownId(3),
            cargo: Text::new("c").unwrap(),
            industries: BoundedVec::new(vec![Text::new("m").unwrap(), Text::new("q").unwrap()])
                .unwrap(),
            permit: None,
        });
        assert_eq!(
            prospect.to_payload().unwrap().as_bytes(),
            [
                9,  // schema version
                15, // Action::Prospect
                3,  // town-3
                1, b'c', // cargo
                2, 1, b'm', 1, b'q', // two industry types, in order
                0,    // no permit
            ]
        );
        let recolor = Action::CompanyOp(CompanyOp::Recolor {
            company: CompanyId(2),
            color: Tint { r: 1, g: 0, b: 0 },
        });
        assert_eq!(
            recolor.to_payload().unwrap().as_bytes(),
            [
                9,  // schema version
                11, // Action::CompanyOp
                4,  // CompanyOp::Recolor, appended under schema version 8
                2,  // company-2
                2, 0, 0, // the colour, zigzag
            ]
        );
    }

    #[test]
    fn payload_refuses_other_schemas_and_trailing_bytes() {
        let action = Action::CompanyOp(CompanyOp::Join(CompanyId(2)));
        let payload = action.to_payload().unwrap();
        assert_eq!(Action::from_payload(&payload).unwrap(), action);

        let mut other = payload.as_bytes().to_vec();
        other[0] = 1;
        assert!(matches!(
            Action::from_payload(&Payload::new(other).unwrap()),
            Err(ActionError::Schema { found: 1 })
        ));

        let mut padded = payload.as_bytes().to_vec();
        padded.push(0);
        assert!(matches!(
            Action::from_payload(&Payload::new(padded).unwrap()),
            Err(ActionError::TrailingBytes(1))
        ));
    }

    #[test]
    fn ids_display_with_their_kind() {
        assert_eq!(LineId(7).to_string(), "line-7");
        assert_eq!(StationId(12).to_string(), "station-12");
    }
}
