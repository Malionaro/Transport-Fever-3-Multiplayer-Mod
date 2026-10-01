//! Checked native memory capture, independent of the Lua proposal fixtures.
#![allow(clippy::unwrap_used)]

use tpf3mp_hook::{
    junctions::decode,
    modules::{Memory, layout},
};
use tpf3mp_proto::lua::LuaValue;

struct Image(Vec<u8>);
impl Memory for Image {
    fn read(&self, address: usize, len: usize) -> Option<Vec<u8>> {
        Some(self.0.get(address..address.checked_add(len)?)?.to_vec())
    }
}
impl Image {
    fn word(&mut self, at: usize, n: u64) {
        self.0[at..at + 8].copy_from_slice(&n.to_le_bytes());
    }
    fn int(&mut self, at: usize, n: i32) {
        self.0[at..at + 4].copy_from_slice(&n.to_le_bytes());
    }
    fn float(&mut self, at: usize, n: f32) {
        self.0[at..at + 4].copy_from_slice(&n.to_le_bytes());
    }
    fn vector(&mut self, at: usize, begin: usize, count: usize, stride: usize) {
        for (i, p) in [begin, begin + count * stride, begin + count * stride]
            .iter()
            .enumerate()
        {
            self.0[at + i * 8..at + i * 8 + 8].copy_from_slice(&(*p as u64).to_le_bytes());
        }
    }
}
const PROPOSAL: usize = 0x100;
const CONFIG: usize = 0x400;
const TURN: usize = 0x500;
const PHASE: usize = 0x600;

fn fixture() -> Image {
    let mut m = Image(vec![0; 0x900]);
    m.vector(PROPOSAL + 0x60, CONFIG, 1, 0x80);
    m.vector(PROPOSAL + 0x78, 0x800, 1, 4);
    m.int(0x800, 42);
    m.int(CONFIG + 0x78, 42);
    m.vector(CONFIG, TURN, 1, 0x14);
    m.int(TURN, 100);
    m.int(TURN + 4, 1);
    m.int(TURN + 8, 101);
    m.int(TURN + 12, 0);
    m.0[TURN + 0x10] = 1;
    m.word(CONFIG + 0x18, 0x850); // control bytes
    m.word(CONFIG + 0x20, 0x860); // int slots
    m.word(CONFIG + 0x28, 1); // occupied count
    m.word(CONFIG + 0x30, 7); // capacity, not a vector pointer
    m.0[0x850..0x858].fill(0x80);
    m.0[0x852] = 0x42;
    m.0[0x857] = 0xff;
    m.int(0x868, 100);
    m.int(CONFIG + 0x4c, 1);
    m.vector(CONFIG + 0x50, PHASE, 1, 0x28);
    m.int(CONFIG + 0x68, 55);
    m.0[CONFIG + 0x70] = 1;
    m.vector(PHASE, 0x820, 2, 4);
    m.int(0x820, 0);
    m.int(0x824, 1);
    m.float(PHASE + 0x18, 12.375);
    m.float(PHASE + 0x1c, 4.125);
    m.0[PHASE + 0x20] = 1;
    m
}
fn first(v: &LuaValue) -> &LuaValue {
    let LuaValue::Table(v) = v else {
        panic!("expected array");
    };
    &v[0].1
}

#[test]
fn reads_the_component_before_the_entity_and_preserves_phase_order_and_units() {
    let m = fixture();
    let v = decode(&m, PROPOSAL).unwrap().unwrap();
    assert_eq!(v.get("junctionEdit"), Some(&LuaValue::Boolean(true)));
    let street = v.get("proposal").unwrap();
    let add = first(street.get("nodeConfigsToAdd").unwrap());
    assert_eq!(add.get("entity"), Some(&LuaValue::Integer(42)));
    assert_eq!(
        first(street.get("nodeConfigsToRemove").unwrap()),
        &LuaValue::Integer(42)
    );
    let c = add.get("comp").unwrap();
    assert_eq!(first(c.get("crosswalks").unwrap()), &LuaValue::Integer(100));
    let turn = first(c.get("laneConnections").unwrap());
    assert_eq!(turn.get("segment0"), Some(&LuaValue::Integer(100)));
    assert_eq!(turn.get("lane0"), Some(&LuaValue::Integer(1)));
    assert_eq!(turn.get("withRoad"), Some(&LuaValue::Boolean(true)));
    assert_eq!(turn.get("withTram"), Some(&LuaValue::Boolean(false)));
    let lights = c.get("trafficLightConfig").unwrap();
    assert_eq!(lights.get("trafficLightType"), Some(&LuaValue::Integer(55)));
    let phase = first(lights.get("states").unwrap());
    assert_eq!(phase.get("duration"), Some(&LuaValue::Number(12.375)));
    assert_eq!(phase.get("minDuration"), Some(&LuaValue::Number(4.125)));
    assert_eq!(
        first(phase.get("lockedLanes").unwrap()),
        &LuaValue::Integer(0)
    );
}

#[test]
fn crosswalk_hash_set_reads_sparse_slots_and_ignores_deleted_slots() {
    let mut m = fixture();
    m.word(CONFIG + 0x28, 2);
    m.0[0x850] = 0xfe;
    m.int(0x860, -1); // deleted: this stale value must never be captured
    m.0[0x855] = 0x13;
    m.int(0x874, 101);
    let v = decode(&m, PROPOSAL).unwrap().unwrap();
    let add = first(v.get("proposal").unwrap().get("nodeConfigsToAdd").unwrap());
    assert_eq!(
        add.get("comp").unwrap().get("crosswalks").unwrap(),
        &LuaValue::Table(vec![
            (LuaValue::Integer(1), LuaValue::Integer(100)),
            (LuaValue::Integer(2), LuaValue::Integer(101)),
        ])
    );
    m.word(CONFIG + 0x28, 0);
    m.word(CONFIG + 0x30, 0);
    m.word(CONFIG + 0x18, u64::MAX); // empty global sentinel need not be read
    m.word(CONFIG + 0x20, 0);
    assert!(decode(&m, PROPOSAL).unwrap().is_some());
}

#[test]
fn malformed_crosswalk_hash_sets_are_refused() {
    for (at, value) in [
        (0x28, 257),
        (0x30, 6),
        (0x30, 2047),
        (0x20, 0),
        (0x18, 0x900),
        (0x28, 2),
    ] {
        let mut m = fixture();
        m.word(CONFIG + at, value);
        assert!(decode(&m, PROPOSAL).is_err());
    }
    for (at, value) in [(0x857, 0x80), (0x850, 0xff), (0x850, 0x81)] {
        let mut m = fixture();
        m.0[at] = value;
        assert!(decode(&m, PROPOSAL).is_err());
    }
    let mut m = fixture();
    m.int(0x868, -1);
    assert!(decode(&m, PROPOSAL).is_err());
    let mut m = fixture();
    m.word(CONFIG + 0x28, 2);
    m.0[0x855] = 0x11;
    m.int(0x874, 100);
    assert!(decode(&m, PROPOSAL).is_err());
}

#[test]
fn resets_are_captured_but_mixed_builds_remain_on_the_geometry_path() {
    let mut m = fixture();
    m.vector(PROPOSAL + 0x60, 0, 0, 0x80);
    assert!(decode(&m, PROPOSAL).unwrap().is_some());
    m.vector(PROPOSAL + 0x18, 0x800, 1, 0x350);
    assert!(decode(&m, PROPOSAL).unwrap().is_none());
    let mut m = fixture();
    m.vector(PROPOSAL + 0x258, 0x800, 1, 0xe48);
    assert!(decode(&m, PROPOSAL).unwrap().is_none());
}

#[test]
fn large_station_proposals_are_not_subject_to_standalone_junction_limits() {
    // Real eight-track station attempts reported 200 and 392 configs.
    // Their generated junctions belong to construction replay. Only the
    // proposal header is readable here: this reader must not inspect them.
    for junction_count in [200, 392] {
        for (offset, stride) in [
            (layout::ADDED_NODES, layout::NODE_SIZE),
            (layout::ADDED_SEGMENTS, layout::SEGMENT_SIZE),
            (layout::REMOVED_NODES, layout::NODE_SIZE),
            (layout::REMOVED_SEGMENTS, layout::SEGMENT_SIZE),
            (layout::EDGE_OBJECTS_TO_REMOVE, 4),
            (layout::EDGE_OBJECTS_TO_ADD, layout::EDGE_OBJECT_SIZE),
            (layout::TO_REMOVE, 4),
            (layout::TO_ADD, layout::ENTITY_SIZE),
        ] {
            let mut m = fixture();
            m.vector(PROPOSAL + 0x60, 0x1000, junction_count, 0x80);
            m.vector(PROPOSAL + 0x78, 0x2000, junction_count, 4);
            m.vector(PROPOSAL + offset, 0x3000, 1, stride);
            m.0.truncate(PROPOSAL + layout::HEAD_LEN);
            assert_eq!(
                decode(&m, PROPOSAL),
                Ok(None),
                "mixed proposal with {junction_count} configs at geometry offset {offset:#x}"
            );
        }
    }
}

#[test]
fn standalone_junction_adds_and_resets_keep_their_limit() {
    for (offset, stride, label) in [
        (0x60, 0x80, "junction configurations added"),
        (0x78, 4, "junction configurations removed"),
    ] {
        let mut m = fixture();
        m.vector(PROPOSAL + offset, 0x1000, 392, stride);
        let why = decode(&m, PROPOSAL).unwrap_err();
        assert!(why.contains(label), "{why}");
        assert!(why.contains("392, more than the 64"), "{why}");
    }
}

#[test]
fn malformed_vectors_unreadable_data_and_invalid_values_fail_closed() {
    let mut cases = Vec::new();
    let mut m = fixture();
    m.vector(CONFIG + 0x30, 0x850, 1, 4);
    cases.push(m);
    let mut m = fixture();
    m.vector(CONFIG, TURN, 257, 0x14);
    cases.push(m);
    let mut m = fixture();
    m.vector(CONFIG, 0x8f0, 2, 0x14);
    cases.push(m);
    let mut m = fixture();
    m.vector(CONFIG + 0x50, PHASE, 65, 0x28);
    cases.push(m);
    let mut m = fixture();
    m.int(CONFIG + 0x78, -1);
    cases.push(m);
    let mut m = fixture();
    m.int(TURN, -1);
    cases.push(m);
    let mut m = fixture();
    m.float(PHASE + 0x18, f32::NAN);
    cases.push(m);
    let mut m = fixture();
    m.float(PHASE + 0x18, f32::INFINITY);
    cases.push(m);
    let mut m = fixture();
    m.0[PHASE + 0x20] = 2;
    cases.push(m);
    let mut m = fixture();
    m.0.truncate(PROPOSAL + 0x200);
    cases.push(m);
    let mut m = fixture();
    m.vector(PROPOSAL + 0x60, CONFIG, 65, 0x80);
    cases.push(m);
    for (i, m) in cases.iter().enumerate() {
        assert!(
            decode(m, PROPOSAL).is_err(),
            "malformed case {i} was accepted"
        );
    }
}
