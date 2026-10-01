//! The player's mods for rooms, found and chosen (docs/MODS.md, "Choosing
//! mods"): every mod this player has installed, scanned, and the personal
//! ones they chose to play with.
//!
//! - **Found by themselves**: Mod Hub's cache, the Steam accounts' local
//!   mods and the game's own `mods` and `dlcs` (`tpf3mp_modscan::roots`),
//!   each scanned for its class and the first reason for it. The launcher's
//!   `--mods` list overrides all of this (`crate::content::split`).
//! - **Chosen**: a personal mod (a carried one too with
//!   `--personal-game-scripts`) is played with when the player chose it; a
//!   shared mod is never theirs to choose: every player needs the room's.
//! - **The room's shared mods** come from the owner's start save
//!   ([`Mods::own_start`]): its mods, less the owner's personal ones. The
//!   owner declares them to the room. Another player learns them from what
//!   the room says their game lacks ([`Mods::learn`], the room's
//!   `ContentDiff`, in the room's load order) and declares those they have,
//!   so that a player with every one of them matches the owner.
//!
//! What this gives the room: the content manifest a player declares
//! ([`Mods::manifest`]) and the lists the room's world loads with in their
//! game ([`Mods::lists`]).

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use tpf3mp_bridge::{ModLists, ModName, mods::OWN_MOD};
use tpf3mp_modscan::{Class, roots, save::SaveMod};
use tpf3mp_proto::{BoundedVec, ContentDiff, ContentManifest, ModRef, Text};

/// Longest reason kept for a mod, in bytes.
const MAX_REASON: usize = 96;

/// One installed mod, as the scan saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// Its id (`modId`), as saves and the game name it.
    pub id: String,
    /// Its name for players (`_metadata/modinfo.json`), else its id.
    pub name: String,
    /// Its `revision`, as the room compares versions.
    pub version: String,
    pub class: Class,
    /// Why, in a line: the first reason that makes it shared, or what it is.
    pub reason: String,
    pub path: PathBuf,
}

/// Every mod in the places this player keeps them, scanned; the first of
/// each id counts.
pub fn discover(game: Option<&Path>, steam_roots: &[PathBuf]) -> Vec<Installed> {
    scanned(&roots::installed(&roots::default_roots(game, steam_roots)))
}

/// The mods `found`, scanned; the first of each id counts.
pub fn scanned(found: &[roots::Found]) -> Vec<Installed> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for mod_found in found {
        if !seen.insert(mod_found.id.clone()) {
            continue;
        }
        let report = tpf3mp_modscan::scan(&mod_found.path);
        let reason = match report.sharing().next() {
            Some(reason) => format!("{}: {}", kind_word(report.class), reason.detail),
            None => "only what this player sees".to_owned(),
        };
        out.push(Installed {
            id: mod_found.id.clone(),
            name: display_name(&mod_found.path).unwrap_or_else(|| mod_found.id.clone()),
            version: report.revision.map(|r| r.to_string()).unwrap_or_default(),
            class: report.class,
            reason: shorten(&reason),
            path: mod_found.path.clone(),
        });
    }
    out
}

fn kind_word(class: Class) -> &'static str {
    match class {
        Class::Carried => "decides in the simulation",
        Class::Personal | Class::Shared => "every player needs it",
    }
}

fn shorten(text: &str) -> String {
    if text.len() <= MAX_REASON {
        return text.to_owned();
    }
    let mut end = MAX_REASON - 3;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &text[..end])
}

/// The mod's name for players, from `_metadata/modinfo.json`.
fn display_name(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join("_metadata").join("modinfo.json")).ok()?;
    let info: serde_json::Value = serde_json::from_str(&text).ok()?;
    info.get("name")?
        .as_str()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// Whether a mod of `class` may be chosen, with `carried_personal`.
pub fn choosable(class: Class, carried_personal: bool) -> bool {
    match class {
        Class::Personal => true,
        Class::Carried => carried_personal,
        Class::Shared => false,
    }
}

/// One of the room's shared mods, and whether this player has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Required {
    pub id: String,
    /// The room's version of it.
    pub version: String,
    pub have: Have,
}

/// Whether this player has one of the room's shared mods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Have {
    Yes,
    No,
    /// Installed in another version.
    OtherVersion,
}

/// This player's mods for rooms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mods {
    build: Text<64>,
    installed: Vec<Installed>,
    chosen: BTreeSet<String>,
    carried_personal: bool,
    /// The room's shared mods in load order, with the room's versions, once
    /// known.
    room: Option<Vec<ModRef>>,
    /// Whether they came from this player's own start save.
    owner: bool,
}

impl Mods {
    /// `installed` for a game of `build`, with the mods the player chose
    /// before (those no longer installed or choosable are dropped).
    pub fn new(
        build: Text<64>,
        installed: Vec<Installed>,
        chosen: impl IntoIterator<Item = String>,
        carried_personal: bool,
    ) -> Self {
        let mut mods = Self {
            build,
            installed,
            chosen: BTreeSet::new(),
            carried_personal,
            room: None,
            owner: false,
        };
        for id in chosen {
            let _ = mods.choose(&id, true);
        }
        mods
    }

    pub fn installed(&self) -> &[Installed] {
        &self.installed
    }

    fn find(&self, id: &str) -> Option<&Installed> {
        self.installed.iter().find(|m| m.id == id)
    }

    /// Whether the mod `id` may be chosen: an installed personal mod, or a
    /// carried one with `--personal-game-scripts`.
    pub fn is_choosable(&self, id: &str) -> bool {
        self.find(id)
            .is_some_and(|m| choosable(m.class, self.carried_personal))
    }

    pub fn is_chosen(&self, id: &str) -> bool {
        self.chosen.contains(id)
    }

    /// The mods chosen, by id, to remember.
    pub fn chosen(&self) -> Vec<String> {
        self.chosen.iter().cloned().collect()
    }

    /// Chooses the mod `id`, or not; refused for one that is not choosable.
    pub fn choose(&mut self, id: &str, chosen: bool) -> Result<(), String> {
        if !chosen {
            self.chosen.remove(id);
            return Ok(());
        }
        match self.find(id) {
            None => Err(format!("no mod {id} is installed")),
            Some(m) if !choosable(m.class, self.carried_personal) => Err(match m.class {
                Class::Carried => format!(
                    "{} decides in the simulation: every player needs it (or start with --personal-game-scripts)",
                    m.name
                ),
                _ => format!("{} changes the world: every player needs it", m.name),
            }),
            Some(_) => {
                self.chosen.insert(id.to_owned());
                Ok(())
            }
        }
    }

    /// The room this player owns starts from a save listing `save`: its
    /// mods, less this player's personal ones (whether chosen or not), are
    /// the room's shared mods, each in this player's version (none when not
    /// installed here: still shared, fail closed).
    pub fn own_start(&mut self, save: &[SaveMod]) {
        let room = save
            .iter()
            .filter(|m| m.id == OWN_MOD || !self.is_choosable(&m.id))
            .map(|m| ModRef {
                id: Text::lossy(&m.id),
                version: Text::lossy(self.find(&m.id).map_or("", |i| i.version.as_str())),
            })
            .collect();
        self.room = Some(room);
        self.owner = true;
    }

    /// A room with no start save of this player's, or none at all.
    pub fn forget_room(&mut self) {
        self.room = None;
        self.owner = false;
    }

    /// Takes in what the room says this player's game lacks, or has in
    /// another version, compared with the owner's: the room's shared mods.
    /// Returns whether they changed, and so what this player declares. The
    /// owner's own list is never changed by it.
    pub fn learn(&mut self, diff: &ContentDiff) -> bool {
        if self.owner {
            return false;
        }
        let mut room = self.room.clone().unwrap_or_default();
        for missing in &diff.missing {
            if !room.iter().any(|m| m.id == missing.id) {
                room.push(missing.clone());
            }
        }
        for change in &diff.changed {
            match room.iter_mut().find(|m| m.id == change.id) {
                Some(m) => m.version = change.room.clone(),
                None => room.push(ModRef {
                    id: change.id.clone(),
                    version: change.room.clone(),
                }),
            }
        }
        // What the room does not run is not the room's.
        room.retain(|m| !diff.extra.iter().any(|extra| extra.id == m.id));
        let changed = self.room.as_ref() != Some(&room);
        if changed {
            self.room = Some(room);
        }
        changed
    }

    /// What this player declares to the room: the build, and those of the
    /// room's shared mods this player has, in the room's order, in this
    /// player's versions. None before the room's are known.
    pub fn manifest(&self) -> ContentManifest {
        let mods = self
            .room
            .iter()
            .flatten()
            .filter_map(|m| {
                let here = self.find(m.id.as_str())?;
                Some(ModRef {
                    id: m.id.clone(),
                    version: Text::lossy(&here.version),
                })
            })
            .collect();
        ContentManifest::new(self.build.clone(), mods)
    }

    /// The mods the room's worlds load with in this game: the room's shared
    /// ones and the chosen personal ones; none before the room's are known
    /// (a world then loads with its save's own).
    pub fn lists(&self) -> Option<ModLists> {
        let room = self.room.as_ref()?;
        let shared = room
            .iter()
            .map(|m| ModName::new(m.id.as_str()).ok())
            .collect::<Option<Vec<_>>>()?;
        let personal = self
            .installed
            .iter()
            .filter(|m| self.chosen.contains(&m.id) && self.is_choosable(&m.id))
            .map(|m| ModName::new(&m.id).ok())
            .collect::<Option<Vec<_>>>()?;
        Some(ModLists {
            shared: BoundedVec::new(shared).ok()?,
            personal: BoundedVec::new(personal).ok()?,
        })
    }

    /// The room's shared mods, and whether this player has each.
    pub fn required(&self) -> Vec<Required> {
        self.room
            .iter()
            .flatten()
            .map(|m| Required {
                id: m.id.as_str().to_owned(),
                version: m.version.as_str().to_owned(),
                have: match self.find(m.id.as_str()) {
                    None => Have::No,
                    Some(here)
                        if m.version.as_str().is_empty() || here.version == m.version.as_str() =>
                    {
                        Have::Yes
                    }
                    Some(_) => Have::OtherVersion,
                },
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use tpf3mp_proto::ModChange;

    use super::*;

    fn installed(id: &str, class: Class, version: &str) -> Installed {
        Installed {
            id: id.into(),
            name: id.into(),
            version: version.into(),
            class,
            reason: String::new(),
            path: PathBuf::new(),
        }
    }

    fn catalog() -> Vec<Installed> {
        vec![
            installed("tpf3mp_1", Class::Shared, "1"),
            installed("vehicles_pack", Class::Shared, "3"),
            installed("minimap", Class::Personal, "1"),
            installed("timetables", Class::Carried, "8"),
        ]
    }

    fn save(ids: &[&str]) -> Vec<SaveMod> {
        ids.iter()
            .map(|id| SaveMod {
                id: (*id).into(),
                source: "StagingArea".into(),
                name: (*id).into(),
            })
            .collect()
    }

    fn names(manifest: &ContentManifest) -> Vec<String> {
        manifest
            .mods
            .iter()
            .map(|m| format!("{} {}", m.id, m.version))
            .collect()
    }

    #[test]
    fn only_personal_mods_are_chosen_and_carried_ones_on_request() {
        let mut mods = Mods::new(
            Text::lossy("40408"),
            catalog(),
            ["minimap".into(), "timetables".into(), "gone".into()],
            false,
        );
        assert_eq!(
            mods.chosen(),
            ["minimap"],
            "remembered, less what is not choosable"
        );
        assert!(mods.choose("vehicles_pack", true).is_err());
        assert!(
            mods.choose("timetables", true)
                .unwrap_err()
                .contains("--personal-game-scripts")
        );
        let mut open = Mods::new(Text::lossy("40408"), catalog(), [], true);
        open.choose("timetables", true).unwrap();
        open.choose("timetables", false).unwrap();
        assert!(open.chosen().is_empty());
        mods.choose("minimap", false).unwrap();
        assert!(mods.chosen().is_empty());
    }

    #[test]
    fn the_owners_start_save_makes_the_rooms_shared_mods_and_a_guest_learns_them() {
        let mut owner = Mods::new(Text::lossy("40408"), catalog(), ["minimap".into()], false);
        assert_eq!(
            owner.lists(),
            None,
            "no room yet: saves load with their own"
        );
        assert!(owner.manifest().mods.is_empty());
        owner.own_start(&save(&["vehicles_pack", "tpf3mp_1", "minimap", "dlc_pack"]));
        assert_eq!(
            names(&owner.manifest()),
            ["vehicles_pack 3", "tpf3mp_1 1"],
            "the owner's minimap is theirs; a mod not installed here declares nothing"
        );
        let lists = owner.lists().unwrap();
        let shared: Vec<&str> = lists.shared.iter().map(|m| m.as_str()).collect();
        assert_eq!(shared, ["vehicles_pack", "tpf3mp_1", "dlc_pack"]);
        assert_eq!(lists.personal.len(), 1);
        assert_eq!(
            owner.required().iter().map(|r| r.have).collect::<Vec<_>>(),
            [Have::Yes, Have::Yes, Have::No]
        );

        // A guest with an older vehicle pack and no minimap: declares
        // nothing, hears what it lacks, declares what it has.
        let mut guest = Mods::new(
            Text::lossy("40408"),
            vec![
                installed("tpf3mp_1", Class::Shared, "1"),
                installed("vehicles_pack", Class::Shared, "2"),
            ],
            [],
            false,
        );
        let room_view = owner.manifest();
        let told = room_view.compare(&guest.manifest()).unwrap();
        assert!(guest.learn(&told));
        assert_eq!(names(&guest.manifest()), ["vehicles_pack 2", "tpf3mp_1 1"]);
        let told = room_view.compare(&guest.manifest()).unwrap();
        assert_eq!(
            told.changed,
            [ModChange {
                id: Text::lossy("vehicles_pack"),
                room: Text::lossy("3"),
                yours: Text::lossy("2"),
            }]
        );
        assert!(!guest.learn(&told), "nothing new: no second declaration");
        assert_eq!(
            guest.required().iter().map(|r| r.have).collect::<Vec<_>>(),
            [Have::OtherVersion, Have::Yes]
        );
        // The owner is never told what the room is.
        assert!(!owner.learn(&told));
    }

    #[test]
    fn a_long_reason_is_cut_at_a_character() {
        let long = "é".repeat(100);
        let cut = shorten(&long);
        assert!(cut.len() <= MAX_REASON && cut.ends_with("..."));
    }
}
