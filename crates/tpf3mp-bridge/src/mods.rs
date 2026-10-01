//! Which mods the room's world loads with in this player's game
//! (docs/MODS.md, proposed D25).
//!
//! The room's players share some mods and may each have personal ones
//! (`tpf3mp-modscan`). The agent knows both lists: the shared ones are those
//! it declared to the room, which the room's content check made the same for
//! every player; the personal ones are this player's alone. It hands them to
//! the hook when the room's game begins ([`crate::ToHook::Begin`]).
//!
//! A save lists the mods of the game that wrote it, that player's personal
//! ones included, and the game loads a save with its own list unless told
//! otherwise (`app.loadGame(id, isMapEditor, info)`, where `info.mods`
//! replaces it: the game's own Load Game page does exactly this when the
//! player changes a save's mods, `gui/menu/savegame_react_util.tl`, build
//! 40408). [`plan`] says what to load instead: the save's mods that are
//! shared or this player's own, then this player's other personal mods.
//! Another player's personal mod is left out, in every game but theirs.

use serde::{Deserialize, Serialize};
use tpf3mp_proto::{BoundedVec, Text};

/// A mod's name as the game lists it (`Mod.ModId.name`).
pub type ModName = Text<96>;
/// Most shared mods the lists carry; a player with more loads the room's
/// world with its own list, as without lists.
pub const MAX_SHARED_MODS: usize = 256;
/// Most personal mods the lists carry.
pub const MAX_PERSONAL_MODS: usize = 64;
/// TPF3-MP's own mod, which every game of a room runs, listed or not.
pub const OWN_MOD: &str = "tpf3mp_1";

/// This player's mods for the room's world.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModLists {
    /// The mods every player of the room runs, in load order.
    pub shared: BoundedVec<ModName, MAX_SHARED_MODS>,
    /// This player's personal mods, in load order.
    pub personal: BoundedVec<ModName, MAX_PERSONAL_MODS>,
}

/// What [`plan`] made of a save's mods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The mods to load the save with, in load order.
    pub mods: Vec<String>,
    /// The save's mods left out: other players' personal mods, or mods in no
    /// list at all (then in no game of the room).
    pub dropped: Vec<String>,
    /// This player's personal mods the save did not have.
    pub added: Vec<String>,
}

/// The mods to load a save that lists `save` with: each of the save's mods
/// that is shared, this player's personal one, or TPF3-MP itself, in the
/// save's order, then this player's personal mods the save lacks, in the
/// order of `lists.personal`. A shared mod the save lacks is not added: the
/// room's world was made without it, in every game alike.
pub fn plan(save: &[String], lists: &ModLists) -> Plan {
    let listed = |list: &[ModName], name: &str| list.iter().any(|m| m.as_str() == name);
    let mut mods = Vec::new();
    let mut dropped = Vec::new();
    for name in save {
        if mods.contains(name) {
            continue;
        }
        if name == OWN_MOD || listed(&lists.shared, name) || listed(&lists.personal, name) {
            mods.push(name.clone());
        } else {
            dropped.push(name.clone());
        }
    }
    let mut added = Vec::new();
    for own in lists.personal.iter() {
        let own = own.as_str().to_owned();
        if !mods.contains(&own) {
            mods.push(own.clone());
            added.push(own);
        }
    }
    Plan {
        mods,
        dropped,
        added,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn lists(shared: &[&str], personal: &[&str]) -> ModLists {
        let to = |l: &[&str]| l.iter().map(|s| Text::new(*s).unwrap()).collect::<Vec<_>>();
        ModLists {
            shared: BoundedVec::new(to(shared)).unwrap(),
            personal: BoundedVec::new(to(personal)).unwrap(),
        }
    }

    #[test]
    fn the_owners_personal_mod_is_left_out_and_mine_added() {
        // The owner saved the start world with their minimap; this player
        // runs line colours instead.
        let save = names(&["vehicles_pack", "tpf3mp_1", "owner_minimap"]);
        let plan = plan(&save, &lists(&["vehicles_pack"], &["my_line_colours"]));
        assert_eq!(plan.mods, ["vehicles_pack", "tpf3mp_1", "my_line_colours"]);
        assert_eq!(plan.dropped, ["owner_minimap"]);
        assert_eq!(plan.added, ["my_line_colours"]);
    }

    #[test]
    fn a_personal_mod_both_have_stays_where_the_save_had_it() {
        let save = names(&["overlay", "tpf3mp_1"]);
        let plan = plan(&save, &lists(&[], &["overlay"]));
        assert_eq!(plan.mods, ["overlay", "tpf3mp_1"]);
        assert!(plan.dropped.is_empty() && plan.added.is_empty());
    }

    #[test]
    fn a_shared_mod_the_save_lacks_is_not_added_and_twice_listed_is_once() {
        let save = names(&["a", "a", "tpf3mp_1"]);
        let plan = plan(&save, &lists(&["a", "b"], &[]));
        assert_eq!(plan.mods, ["a", "tpf3mp_1"]);
    }

    #[test]
    fn tpf3mp_itself_is_kept_unlisted() {
        let plan = plan(&names(&["tpf3mp_1"]), &ModLists::default());
        assert_eq!(plan.mods, ["tpf3mp_1"]);
    }
}
