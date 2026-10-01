//! The Lua mod's road and track capture makes exactly the tables the action
//! schema takes: `tests/lua/road_capture.lua` runs the mod's own modules
//! against a world of tables, and the action tables it returns, in metres,
//! must convert (`tpf3mp_proto::lua`, as the hook converts them) to the
//! actions below, in millimetres. Tables changed to break the schema must
//! be refused. Lua 5.1 here (the workspace's vendored Lua); the mod targets
//! 5.2 and keeps to what both run.
//!
//! The modules are compiled into the test binary, so the test does not
//! depend on the working directory.

#![allow(clippy::unwrap_used)]

mod common;

use common::tree;
use mlua::{Lua, Table, Value};
use tpf3mp_proto::{
    BoundedVec, Text,
    action::{
        Action, EdgeEnds, EdgeKind, EdgeRef, Link, Network, NodeRef, Polyline, Pos, Resolve,
        RoadBuild, Structure, Tangent, TrackBuild, Tram, Vertex,
    },
    lua::{LuaValue, action_from_lua},
};

macro_rules! modules {
    ($($name:literal),* $(,)?) => {
        [$(($name, include_str!(concat!("../../../mod/tpf3mp_1/content/scripts/tpf3mp/", $name, ".lua")))),*]
    };
}

/// Every module of the mod, as `require "tpf3mp.<name>"` finds it.
const MODULES: [(&str, &str); 4] = modules!("geom", "roads", "junctions", "engine");

const TEST: &str = include_str!("lua/road_capture.lua");

struct Captured {
    road: LuaValue,
    track: LuaValue,
    must_refuse: Vec<(String, LuaValue)>,
}

/// Runs the Lua test and returns the tables it produced.
fn run() -> Captured {
    let lua = Lua::new();
    let preload: Table = lua
        .globals()
        .get::<Table>("package")
        .unwrap()
        .get("preload")
        .unwrap();
    for (name, source) in MODULES {
        let chunk = lua
            .load(source)
            .set_name(format!("@tpf3mp/{name}.lua"))
            .into_function()
            .unwrap();
        preload.set(format!("tpf3mp.{name}"), chunk).unwrap();
    }
    // Loading the engine adapter needs no game: its API calls happen only
    // when a capture runs.
    lua.load("require 'tpf3mp.engine'").exec().unwrap();
    let (road, track, refuse): (Value, Value, Table) = lua
        .load(TEST)
        .set_name("@road_capture.lua")
        .eval()
        .unwrap_or_else(|error| panic!("{error}"));
    let mut must_refuse: Vec<(String, LuaValue)> = refuse
        .pairs::<String, Value>()
        .map(|pair| {
            let (why, action) = pair.unwrap();
            (why, tree(&action))
        })
        .collect();
    must_refuse.sort_by(|a, b| a.0.cmp(&b.0));
    Captured {
        road: tree(&road),
        track: tree(&track),
        must_refuse,
    }
}

fn decode(table: LuaValue) -> Action {
    action_from_lua(&table).unwrap_or_else(|error| panic!("{error}"))
}

fn text<const N: usize>(value: &str) -> Text<N> {
    Text::new(value).unwrap()
}

fn list<T, const N: usize>(items: Vec<T>) -> BoundedVec<T, N> {
    BoundedVec::new(items).unwrap()
}

fn pos(x: i32, y: i32, z: i32) -> Pos {
    Pos { x, y, z }
}

fn tangent(x: i32, y: i32, z: i32) -> Tangent {
    Tangent { x, y, z }
}

fn link(from: u16, to: u16, t0: Tangent, t1: Tangent, structure: Structure) -> Link {
    Link {
        from,
        to,
        tangent0: t0,
        tangent1: t1,
        structure,
        kind: None,
        decorations: BoundedVec::default(),
        locked: false,
        owned: false,
        lanes: BoundedVec::default(),
    }
}

/// A link of another kind than the build's.
fn of(link: Link, network: Network, template: &str) -> Link {
    Link {
        kind: Some(EdgeKind {
            network,
            template: text(template),
            style: None,
        }),
        ..link
    }
}

fn node(x: i32, y: i32, z: i32, network: Network) -> Vertex {
    Vertex {
        pos: pos(x, y, z),
        resolve: Resolve::Node(network),
    }
}

fn new(x: i32, y: i32, z: i32) -> Vertex {
    Vertex {
        pos: pos(x, y, z),
        resolve: Resolve::New,
    }
}

fn street_edge(a: Pos, b: Pos) -> EdgeRef {
    EdgeRef {
        network: Network::Street,
        ends: EdgeEnds { a, b },
    }
}

const COUNTRY: &str = "street/country.street_template";

#[test]
fn a_captured_road_decodes_as_the_schema_says() {
    let road = run().road;
    let flat = |x| tangent(x, 0, 0);
    let expected = Action::BuildRoad(RoadBuild {
        street: text("street/town_medium.street_template"),
        style: Some(text("style/old_town.lua")),
        bus_lane: false,
        tram: Tram::None,
        polyline: Polyline::new(
            list(vec![
                node(100_000, 200_000, 10_000, Network::Street),
                new(160_000, 200_000, 12_000),
                new(220_000, 230_000, 15_000),
                new(452_000, 0, 2_000),
                node(400_000, 0, 2_000, Network::Street),
                node(500_000, 0, 2_000, Network::Street),
            ]),
            list(vec![
                link(
                    0,
                    1,
                    tangent(60_000, 0, 2_000),
                    tangent(60_000, 0, 2_000),
                    Structure::Ground,
                ),
                link(
                    1,
                    2,
                    tangent(55_000, 20_000, 0),
                    tangent(60_000, 28_250, -2_000),
                    Structure::Bridge(text("bridge/cement.lua")),
                ),
                link(
                    2,
                    3,
                    tangent(232_000, -230_000, -13_000),
                    tangent(232_000, -230_000, -13_000),
                    Structure::Ground,
                ),
                of(
                    link(4, 3, flat(52_000), flat(52_000), Structure::Ground),
                    Network::Street,
                    COUNTRY,
                ),
                of(
                    link(3, 5, flat(48_000), flat(48_000), Structure::Ground),
                    Network::Street,
                    COUNTRY,
                ),
            ]),
            list(vec![
                street_edge(pos(400_000, 0, 2_000), pos(450_000, 0, 2_000)),
                street_edge(pos(450_000, 0, 2_000), pos(500_000, 0, 2_000)),
                street_edge(pos(100_000, 300_000, 10_000), pos(100_000, 200_000, 10_000)),
            ]),
        )
        .unwrap()
        .with_removed_nodes(list(vec![NodeRef {
            network: Network::Street,
            at: pos(450_000, 0, 2_000),
        }])),
    });
    assert_eq!(decode(road), expected);
}

#[test]
fn a_captured_track_decodes_as_the_schema_says() {
    let track = run().track;
    let expected = Action::BuildTrack(TrackBuild {
        track: text("track/high_speed.street_template"),
        style: None,
        catenary: true,
        polyline: Polyline::new(
            list(vec![
                node(0, 0, 1_000, Network::Track),
                new(50_000, 1, 1_250),
                node(50_000, -40_000, 1_000, Network::Street),
                node(50_000, 40_000, 1_000, Network::Street),
                new(150_000, -1, 1_000),
                node(200_000, 0, 1_000, Network::Street),
            ]),
            list(vec![
                link(
                    0,
                    1,
                    tangent(50_000, 0, 250),
                    tangent(50_000, 0, 250),
                    Structure::Ground,
                ),
                of(
                    link(
                        2,
                        1,
                        tangent(0, 40_000, 0),
                        tangent(0, 40_000, 0),
                        Structure::Ground,
                    ),
                    Network::Street,
                    COUNTRY,
                ),
                of(
                    link(
                        1,
                        3,
                        tangent(0, 40_000, 0),
                        tangent(0, 40_000, 0),
                        Structure::Ground,
                    ),
                    Network::Street,
                    COUNTRY,
                ),
                link(
                    1,
                    4,
                    tangent(100_000, 0, 0),
                    tangent(100_000, 0, 0),
                    Structure::Tunnel(text("tunnel/concrete.lua")),
                ),
                link(
                    4,
                    5,
                    tangent(50_000, 0, 0),
                    tangent(50_000, 0, 0),
                    Structure::Ground,
                ),
            ]),
            list(vec![street_edge(
                pos(50_000, 40_000, 1_000),
                pos(50_000, -40_000, 1_000),
            )]),
        )
        .unwrap(),
    });
    assert_eq!(decode(track), expected);
}

#[test]
fn a_capture_the_schema_does_not_allow_is_refused() {
    let refused = run().must_refuse;
    assert_eq!(refused.len(), 10);
    for (why, table) in refused {
        assert!(
            action_from_lua(&table).is_err(),
            "{why}: the schema took it"
        );
    }
}

/// Reads a builder proposal with the mod's `engine.fromProposal`, in a
/// stand-in for the game's API, and returns `street`, `track`, `style` and
/// the first edge's network, joined by `|`, or the error.
fn read_proposal(api: &str, segment: &str, network: &str) -> Result<String, String> {
    let lua = Lua::new();
    let preload: Table = lua
        .globals()
        .get::<Table>("package")
        .unwrap()
        .get("preload")
        .unwrap();
    for (name, source) in MODULES {
        let chunk = lua.load(source).into_function().unwrap();
        preload.set(format!("tpf3mp.{name}"), chunk).unwrap();
    }
    lua.load(format!(
        "api = {api}
         local engine = require 'tpf3mp.engine'
         local function v(x, y, z) return {{ x = x, y = y, z = z }} end
         local seg = {segment}
         local proposal = {{ toAdd = {{}}, toRemove = {{}}, proposal = {{
             addedNodes = {{ {{ entity = -1, comp = {{ position = v(0, 0, 0) }} }},
                            {{ entity = -2, comp = {{ position = v(100, 0, 0) }} }} }},
             addedSegments = {{ seg }},
             removedSegments = {{}},
         }} }}
         local c = engine.fromProposal(proposal, '{network}')
         return table.concat({{ tostring(c.street), tostring(c.track), tostring(c.style), c.edges[1].network }}, '|')"
    ))
    .eval::<String>()
    .map_err(|error| error.to_string())
}

/// As much of TF3's api as reading a proposal needs.
const TF3_API: &str =
    "{ type = { enum = { BaseEdgeType = { NORMAL = 0, BRIDGE = 1, TUNNEL = 2 } } } }";

/// A segment of the tool's proposal: `kind` 0 for a street, 1 for a track.
fn tf3_segment(kind: u8, style: &str) -> String {
    format!(
        "{{ entity = -3, type = {kind}, comp = {{ node0 = -1, node1 = -2, \
         tangent0 = v(100, 0, 0), tangent1 = v(100, 0, 0), type = 0, typeIndex = -1, \
         roadType = {kind}, roadTemplate = 'road/town_medium.lua', roadStyle = {style} }} }}"
    )
}

#[test]
fn a_tf3_proposal_is_read_by_road_template_and_style() {
    let street = read_proposal(TF3_API, &tf3_segment(0, "'style/old_town.lua'"), "Street");
    assert_eq!(
        street.unwrap(),
        "road/town_medium.lua|nil|style/old_town.lua|Street"
    );
    let track = read_proposal(TF3_API, &tf3_segment(1, "''"), "Track");
    assert_eq!(
        track.unwrap(),
        "nil|road/town_medium.lua|nil|Track",
        "an empty style is none"
    );
}

#[test]
fn a_tf3_proposal_the_mod_cannot_place_fails_the_capture() {
    // An edge of neither network.
    let unknown = read_proposal(TF3_API, &tf3_segment(5, "nil"), "Track");
    assert!(unknown.unwrap_err().contains("an edge of type 5"));
    // A style that is not a resource name.
    let odd = read_proposal(TF3_API, &tf3_segment(0, "7"), "Street");
    assert!(odd.unwrap_err().contains("roadStyle"));
    // No enum to tell a bridge by.
    let no_enum = read_proposal("{ type = {} }", &tf3_segment(0, "nil"), "Street");
    assert!(no_enum.unwrap_err().contains("api.type.enum.BaseEdgeType"));
}

/// Captures a street tool's proposal with the mod's `engine.captureBuild`, in
/// a stand-in for TF3's api over the network below, and returns the action
/// table, `false` for a proposal of nothing, or why not.
fn capture_street(street: &str) -> Result<LuaValue, String> {
    capture_with(
        street,
        "captureBuild({ toAdd = {}, toRemove = {}, proposal = street }, 'Street')",
    )
}

/// As [`capture_street`], through the road modifiers' capture.
fn capture_modify(street: &str) -> Result<LuaValue, String> {
    capture_with(
        street,
        "captureModify({ toAdd = {}, toRemove = {}, proposal = street })",
    )
}

fn capture_with(street: &str, call: &str) -> Result<LuaValue, String> {
    let lua = Lua::new();
    let preload: Table = lua
        .globals()
        .get::<Table>("package")
        .unwrap()
        .get("preload")
        .unwrap();
    for (name, source) in MODULES {
        let chunk = lua.load(source).into_function().unwrap();
        preload.set(format!("tpf3mp.{name}"), chunk).unwrap();
    }
    let (action, why): (Value, Option<String>) = lua
        .load(format!(
            "local function v(x, y, z) return {{ x = x, y = y, z = z }} end
             -- The country street 8-9-10 runs north through (50, 0); street
             -- 20-21 east-west under (120, 0), where the new road's bridge ends.
             local NODES = {{ [7] = v(0, 0, 0), [8] = v(50, -40, 0), [9] = v(50, 1, 0), [10] = v(50, 40, 0),
                              [20] = v(120, -30, 0), [21] = v(120, 30, 0) }}
             local STREETS = {{ [7] = {{ 104 }}, [8] = {{ 100 }}, [9] = {{ 100, 101 }}, [10] = {{ 101 }},
                                [20] = {{ 102 }}, [21] = {{ 102 }} }}
             api = {{
                 type = {{ enum = {{ BaseEdgeType = {{ NORMAL = 0, BRIDGE = 1, TUNNEL = 2 }} }},
                          ComponentType = {{ BASE_NODE = 1, CONSTRUCTION = 2 }} }},
                 res = {{ bridgeTypeRep = {{ getName = function(i) if i == 3 then return 'bridge/stone.lua' end end }},
                          edgeDecorationRep = {{ getName = function(i)
                              if i == 3 then return '::/infrastructure/edge_addons/barrier_b.edge' end end }} }},
                 engine = {{
                     getComponent = function(id, kind)
                         if kind == 1 and NODES[id] then return {{ position = NODES[id] }} end
                         -- 900 a town's house, 901 a station.
                         if kind == 2 and id == 900 then return {{ townBuildings = {{ 1 }} }} end
                         if kind == 2 and id == 901 then return {{ townBuildings = {{}} }} end
                     end,
                     system = {{ streetSystem = {{
                         getNodeStreetSegments = function(n) return STREETS[n] or {{}} end,
                         getNodeTrackSegments = function() return {{}} end,
                     }} }},
                 }},
             }}
             local street = {street}
             return require('tpf3mp.engine').{call}"
        ))
        .eval()
        .map_err(|error| error.to_string())?;
    match (action, why) {
        (Value::Nil, why) => Err(why.unwrap_or_default()),
        (action, _) => Ok(tree(&action)),
    }
}

/// The street tool's proposal, as build 40408 hands it to game scripts: from
/// node 7 onto the country street 8-9, whose node 9 the new junction -1
/// replaces, rebuilding the street from 8 and 10 through it; then over a
/// bridge.
const SPLIT_PROPOSAL: &str = "{
    addedNodes = { { entity = -1, comp = { position = v(50, 0, 0) } },
                   { entity = -2, comp = { position = v(120, 0, 12) } } },
    addedSegments = {
        { entity = -3, type = 0, comp = { node0 = 7, node1 = -1, tangent0 = v(50, 0, 0), tangent1 = v(50, 0, 0),
          type = 0, typeIndex = -1, roadTemplate = 'street/town.street_template', roadStyle = '', objects = {} } },
        { entity = -4, type = 0, comp = { node0 = 8, node1 = -1, tangent0 = v(0, 40, 0), tangent1 = v(0, 40, 0),
          type = 0, typeIndex = -1, roadTemplate = 'street/country.street_template', roadStyle = '' } },
        { entity = -5, type = 0, comp = { node0 = -1, node1 = 10, tangent0 = v(0, 40, 0), tangent1 = v(0, 40, 0),
          type = 0, typeIndex = -1, roadTemplate = 'street/country.street_template', roadStyle = '' } },
        { entity = -6, type = 0, comp = { node0 = -1, node1 = -2, tangent0 = v(70, 0, 12), tangent1 = v(70, 0, 12),
          type = 1, typeIndex = 3, roadTemplate = 'street/town.street_template', roadStyle = '' } },
    },
    removedSegments = { { entity = 100, type = 0, comp = { node0 = 8, node1 = 9 } },
                        { entity = 101, type = 0, comp = { node0 = 9, node1 = 10 } } },
    removedNodes = { { entity = 9, comp = { position = v(50, 1, 0) } } },
}";

#[test]
fn a_tf3_proposal_travels_as_the_tool_made_it() {
    let action = decode(capture_street(SPLIT_PROPOSAL).unwrap());
    let expected = Action::BuildRoad(RoadBuild {
        street: text("street/town.street_template"),
        style: None,
        bus_lane: false,
        tram: Tram::None,
        polyline: Polyline::new(
            list(vec![
                node(0, 0, 0, Network::Street),
                new(50_000, 0, 0),
                node(50_000, -40_000, 0, Network::Street),
                node(50_000, 40_000, 0, Network::Street),
                new(120_000, 0, 12_000),
            ]),
            list(vec![
                link(
                    0,
                    1,
                    tangent(50_000, 0, 0),
                    tangent(50_000, 0, 0),
                    Structure::Ground,
                ),
                of(
                    link(
                        2,
                        1,
                        tangent(0, 40_000, 0),
                        tangent(0, 40_000, 0),
                        Structure::Ground,
                    ),
                    Network::Street,
                    COUNTRY,
                ),
                of(
                    link(
                        1,
                        3,
                        tangent(0, 40_000, 0),
                        tangent(0, 40_000, 0),
                        Structure::Ground,
                    ),
                    Network::Street,
                    COUNTRY,
                ),
                link(
                    1,
                    4,
                    tangent(70_000, 0, 12_000),
                    tangent(70_000, 0, 12_000),
                    Structure::Bridge(text("bridge/stone.lua")),
                ),
            ]),
            list(vec![
                street_edge(pos(50_000, -40_000, 0), pos(50_000, 1_000, 0)),
                street_edge(pos(50_000, 1_000, 0), pos(50_000, 40_000, 0)),
            ]),
        )
        .unwrap()
        .with_removed_nodes(list(vec![NodeRef {
            network: Network::Street,
            at: pos(50_000, 1_000, 0),
        }])),
    });
    assert_eq!(
        action, expected,
        "the bridge's end above 20-21 is new: only what the tool removes is removed"
    );
}

#[test]
fn a_street_proposal_of_nothing_or_of_more_is_not_carried() {
    // The tool before its first point.
    let nothing = capture_street("{ addedNodes = {}, addedSegments = {}, removedSegments = {} }");
    assert!(
        matches!(nothing, Ok(LuaValue::Boolean(false))),
        "{nothing:?}"
    );
    // A node the world does not have.
    let unknown = SPLIT_PROPOSAL.replace("node0 = 7,", "node0 = 77,");
    assert!(
        capture_street(&unknown)
            .unwrap_err()
            .contains("node 77 has no position")
    );
    // A stop or signal built with the road.
    let objects = SPLIT_PROPOSAL.replace(
        "removedSegments =",
        "edgeObjectsToAdd = { {} }, removedSegments =",
    );
    assert!(
        capture_street(&objects)
            .unwrap_err()
            .contains("a build with a stop or signal")
    );
    // An edge the tool moves with a stop on it: the stop would go onto an
    // edge that is not the one it stood on.
    let moved = SPLIT_PROPOSAL.replace(
        "roadStyle = '', objects = {}",
        "roadStyle = '', objects = { { 555, 1 } }",
    );
    let why = capture_street(&moved).unwrap_err();
    assert!(why.contains("a build that moves a stop or signal"), "{why}");
}

/// A road modifier's proposal as build 40408 hands it to game scripts (the
/// tram track tool, seen in a room): the country street 8-9-10 rebuilt edge
/// by edge between the same places, its middle node 9 removed and added
/// again there as -1, in the tram template; the edge 8-9 keeps its stop
/// 555, the edge 9-10 gets a noise barrier, and is locked and owned (the
/// player-owned tool).
const MODIFY_PROPOSAL: &str = "{
    addedNodes = { { entity = -1, comp = { position = v(50, 1, 0) } } },
    addedSegments = {
        { entity = -2, type = 0, comp = { node0 = 8, node1 = -1, tangent0 = v(0, 41, 0), tangent1 = v(0, 41, 0),
          type = 0, typeIndex = -1, roadTemplate = 'street/country_tram.street_template', roadStyle = '',
          objects = OBJECTS_A } },
        { entity = -3, type = 0, playerOwned = { player = 25 },
          comp = { node0 = -1, node1 = 10, tangent0 = v(0, 39, 0), tangent1 = v(0, 39, 0),
          type = 0, typeIndex = -1, roadTemplate = 'street/country_tram.street_template', roadStyle = '',
          objects = OBJECTS_B, edgeDecorations = { { 3, false } }, roadDevelopmentLocked = true } },
    },
    removedSegments = { { entity = 100, type = 0, comp = { node0 = 8, node1 = 9, objects = { { 555, 0 } } } },
                        { entity = 101, type = 0, comp = { node0 = 9, node1 = 10, objects = {} } } },
    removedNodes = { { entity = 9, comp = { position = v(50, 1, 0) } } },
}";

/// The modifier proposal with the stop 555 on the first new edge (as the
/// tool rebuilds it), on the second, or on neither.
fn modify_proposal(a: &str, b: &str) -> String {
    MODIFY_PROPOSAL
        .replace("OBJECTS_A", a)
        .replace("OBJECTS_B", b)
}

#[test]
fn a_road_modifier_travels_as_the_road_it_rebuilds() {
    let action = decode(capture_modify(&modify_proposal("{ { 555, 0 } }", "{}")).unwrap());
    let Action::BuildRoad(road) = action else {
        panic!("{action:?}")
    };
    assert_eq!(road.street.as_str(), "street/country_tram.street_template");
    let links = &road.polyline.links;
    assert_eq!(links.len(), 2);
    // The edge that keeps its stop, and the one dressed and locked.
    assert!(links[0].decorations.is_empty() && !links[0].locked && !links[0].owned);
    assert_eq!(links[1].decorations.len(), 1);
    assert_eq!(
        links[1].decorations[0].name.as_str(),
        "::/infrastructure/edge_addons/barrier_b.edge"
    );
    assert!(links[1].locked && links[1].owned);
    // Both old edges and the old middle node go.
    assert_eq!(road.polyline.removals.len(), 2);
    assert_eq!(road.polyline.removed_nodes.len(), 1);
}

#[test]
fn a_stop_moves_with_a_road_only_on_the_edge_rebuilt_in_place() {
    // Onto the other edge: refused.
    let why = capture_modify(&modify_proposal("{}", "{ { 555, 0 } }")).unwrap_err();
    assert!(why.contains("a build that moves a stop or signal"), "{why}");
    // Dropped from the rebuilt edge: refused.
    let why = capture_modify(&modify_proposal("{}", "{}")).unwrap_err();
    assert!(
        why.contains("removes an edge with a stop or signal"),
        "{why}"
    );
    // A new stop with the road: refused.
    let why =
        capture_modify(&modify_proposal("{ { 555, 0 }, { -400000000, 0 } }", "{}")).unwrap_err();
    assert!(why.contains("adds a stop or signal"), "{why}");
}

/// A road the tool draws through a town's house: the house goes with the
/// build, which every game's build clears again; a station in the way does
/// not.
#[test]
fn a_road_through_a_town_house_is_carried_and_through_a_station_is_not() {
    let through_house = capture_with(
        SPLIT_PROPOSAL,
        "captureBuild({ toAdd = {}, toRemove = { 900 }, proposal = street }, 'Street')",
    );
    assert!(through_house.is_ok(), "{through_house:?}");
    let why = capture_with(
        SPLIT_PROPOSAL,
        "captureBuild({ toAdd = {}, toRemove = { 901 }, proposal = street }, 'Street')",
    )
    .unwrap_err();
    assert!(why.contains("removes a construction"), "{why}");
}
