//! What the agent and the in-game hook say to each other over the
//! shared-memory link (`tpf3mp-ipc`), and the step gate that keeps the
//! game's simulation in step with the room.
//!
//! The agent runs the network side: it follows the room's turn stream and
//! decides when each step may run (`tpf3mp_agent::Playout`). The hook runs
//! inside the game and does as little as it can. It applies events and runs
//! steps when the [`Gate`] allows, and reports what the player does and how
//! far the game has got.
//!
//! # Ordering
//!
//! Each direction is an ordered stream of messages. The agent sends every
//! event for step `s` after releasing step `s - 1` and before releasing step
//! `s`, and never releases several steps in one message across an event.
//! The hook reads messages only while its game waits before a step, and
//! stops reading once that step is released. Each event is therefore
//! applied exactly between the two steps the room ordered it for, and the
//! [`Gate`] refuses anything that breaks this.
//!
//! This crate has no async runtime and no network code, because the hook
//! links it into the game. [`Session`] is the hook's whole side of the
//! link; the game-specific part of the hook only implements [`Game`].

mod gate;
pub mod mods;
mod session;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;
use tpf3mp_proto::{
    BoundedVec, ChatText, Cursor, Event, IntentRejection, LaneDigest, MAX_ROOM_MEMBERS, Payload,
    PlayerId, RulesName, Secret, Speed, Text,
};

pub use gate::{Gate, GateError, Gated};
pub use mods::{ModLists, ModName, Plan};
pub use session::{Begin, Game, Load, Notice, SaveOrder, Session, SessionError, StepGate};

/// Version of these messages. Both sides send it first and refuse a peer
/// that speaks another. 7 added [`ToAgent::WorldUp`]; 8 added
/// [`ToAgent::MenuUp`]; 9 added the main menu's Multiplayer window's
/// [`LobbyAction::Create`]). 18 added the players' pointers: the player's own
/// to the room as an advisory datagram ([`ToAgent::Cursor`]) and the other
/// members' back to the game ([`ToHook::Cursor`], [`Gated::Cursor`]).
pub const BRIDGE_VERSION: u32 = 18;
/// The link name the agent creates and the hook opens, unless told
/// otherwise.
pub const DEFAULT_LINK: &str = "tpf3mp.default";
/// Largest encoded message, within the link's default frame limit
/// (`tpf3mp_ipc::DEFAULT_MAX_MESSAGE`). An event with the largest intent
/// payload fits.
pub const MAX_MESSAGE: usize = 60 * 1024;
/// Longest file path the link carries, in UTF-8 bytes.
pub const MAX_PATH: usize = 1024;

/// From the agent to the hook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToHook {
    /// The first message after the link opens.
    Hello { version: u32 },
    /// A game begins. A [`ToHook::Load`] follows. The hook writes the saves
    /// the room asks for into `saves`, a directory the agent made for this
    /// game. `rules` names the rules the room is played by: with `native`,
    /// the game's own economy runs as in single player. `player` is the
    /// local player, as the room's events name the actor: the hook knows its
    /// own commands by it when the room orders them.
    Begin {
        rules: RulesName,
        steps_per_second: u16,
        checkpoint_interval: u32,
        saves: Text<MAX_PATH>,
        player: PlayerId,
        /// The mods the room's worlds load with in this game: the room's
        /// shared ones and this player's personal ones ([`mods::plan`]).
        /// `None` when the agent does not know this player's mods: a world
        /// then loads with the mods its save lists.
        mods: Option<ModLists>,
    },
    /// Apply this event before running step `event.step`.
    Apply(Event),
    /// Steps up to and including `through` may run.
    Release { through: u64 },
    /// The room's speed, for the game's display. Pacing comes from
    /// [`ToHook::Release`] alone.
    Speed(Speed),
    /// At the checkpoint at `step`, this replica's `lanes` differed from the
    /// room's verdict.
    Diverged { step: u64, lanes: Vec<u16> },
    /// The room refused one of the player's commands, which then never
    /// happens. `command` counts the player's [`ToAgent::Command`]s from 0.
    Refused {
        command: u64,
        reason: IntentRejection,
    },
    /// The game session is over; the game stops waiting at the gate.
    End { reason: Text<128> },
    /// Load a world, then answer [`ToAgent::Loaded`] with `next_step`, the
    /// first step that world runs. Everything sent before this is void.
    ///
    /// The first load of a game may name no file: the game then loads the
    /// world every player starts from. Otherwise `file` is a save the room
    /// agreed on, for a player joining a running game, one who could no
    /// longer resume, or one whose world diverged.
    Load {
        file: Option<Text<MAX_PATH>>,
        next_step: u64,
    },
    /// A member of the room said something; `from` is their name.
    Chat { from: Text<32>, text: ChatText },
    /// The room as it stands, for the game's Multiplayer window: sent when
    /// the game begins and whenever the room changes.
    Room(RoomInfo),
    /// The launcher's lobby as it stands, for the main menu's Multiplayer
    /// window (D17): sent whenever it changes, before, during and after a
    /// room's game. Only the latest counts. Boxed: it is far larger than
    /// the other messages.
    Lobby(Box<LobbyView>),
    /// A member's pointer moved or their build tool is previewing.
    Cursor(Cursor),
}

/// Most chat lines a [`LobbyView`] carries: the newest.
pub const MAX_LOBBY_CHAT: usize = 40;
/// Most rules a [`LobbyView`] offers.
pub const MAX_LOBBY_RULES: usize = 8;
/// Most saves a [`LobbyView`] lists: the newest.
pub const MAX_LOBBY_SAVES: usize = 40;
/// Longest save name a [`LobbyView`] lists or a [`LobbyAction::Create`]
/// names, in UTF-8 bytes.
pub const MAX_SAVE_NAME: usize = 64;
/// A save in the game's save folder, by its name without `.sav`.
pub type SaveName = Text<MAX_SAVE_NAME>;

/// What the main menu's Multiplayer window shows: the launcher's connection,
/// room and chat, as the launcher window shows them (D17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyView {
    pub connection: LobbyConnection,
    /// The server the launcher plays on, as players see it.
    pub server: Text<128>,
    /// That server's address, `host:port`, as the server setting shows it;
    /// empty without one.
    pub server_address: Text<128>,
    /// The launcher's default server, `host:port`, which the setting's
    /// "Reset to default" goes back to; empty without one.
    pub server_default: Text<128>,
    /// The banner this player picked, if any.
    pub banner: Option<tpf3mp_proto::BannerId>,
    /// The player's name.
    pub name: Text<32>,
    /// What went wrong last, until something succeeds.
    pub error: Option<Text<256>>,
    /// The newest thing the player should know.
    pub notice: Option<Text<256>>,
    pub room: Option<LobbyRoom>,
    /// The room's chat, oldest first.
    pub chat: BoundedVec<LobbyLine, MAX_LOBBY_CHAT>,
    /// The rules the server offers new rooms, its default first.
    pub rules: BoundedVec<LobbyRules, MAX_LOBBY_RULES>,
    /// The player's saves, newest first: what a room they create can start
    /// from.
    pub saves: BoundedVec<SaveName, MAX_LOBBY_SAVES>,
    /// The save rooms this player creates start from unless they pick
    /// another (the launcher's `--start-save`).
    pub start_save: Option<SaveName>,
    /// The room's world in this player's game.
    pub world: LobbyWorld,
    /// How this player's game differs from the room's, while it does.
    pub differences: Option<Text<256>>,
    /// The page of the server's public rooms last asked for
    /// ([`LobbyAction::ListRooms`]), while connected.
    pub rooms: Option<LobbyRoomList>,
    /// The mods this player has installed, those they may choose first
    /// (docs/MODS.md, "Choosing mods"), as many as fit.
    pub mods: BoundedVec<LobbyMod, MAX_LOBBY_MODS>,
    /// The room's shared mods, from its owner's start save, and whether this
    /// player has each; empty while they are not known.
    pub room_mods: BoundedVec<LobbyRoomMod, MAX_LOBBY_ROOM_MODS>,
    /// The room's shared mods beyond those listed.
    pub room_mods_more: u32,
}

/// Most installed mods a [`LobbyView`] lists.
pub const MAX_LOBBY_MODS: usize = 64;
/// Most of the room's shared mods a [`LobbyView`] lists.
pub const MAX_LOBBY_ROOM_MODS: usize = 32;

/// One mod this player has installed, as the window lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyMod {
    /// Its id, as [`LobbyAction::ChooseMod`] names it.
    pub id: ModName,
    /// Its name for players.
    pub name: Text<48>,
    pub class: LobbyModClass,
    /// Why it is of its class, in a line ("every player needs it: ...").
    pub reason: Text<96>,
    /// Whether the player plays with it.
    pub chosen: bool,
    /// Whether the player may choose it: a personal mod, or a carried one
    /// with `--personal-game-scripts`. A shared mod never: every player
    /// needs the room's.
    pub choosable: bool,
}

/// What the scan made of a mod (`tpf3mp_modscan::Class`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LobbyModClass {
    /// Only what this player sees.
    Personal,
    /// Decides in the simulation through what the room carries.
    Carried,
    /// Every player needs it.
    Shared,
}

/// One of the room's shared mods, and whether this player has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyRoomMod {
    pub id: ModName,
    /// The room's version of it (empty when unknown).
    pub version: Text<32>,
    pub have: LobbyHave,
}

/// Whether this player has one of the room's shared mods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LobbyHave {
    Yes,
    No,
    /// Installed, in another version.
    OtherVersion,
}

/// A page of the server's public rooms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyRoomList {
    pub page: u16,
    pub rooms: BoundedVec<LobbyPublicRoom, { tpf3mp_proto::ROOMS_PER_PAGE }>,
    /// A later page has more.
    pub more: bool,
}

/// One public room, as the room browser shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyPublicRoom {
    pub invite: Text<128>,
    pub name: Text<48>,
    pub rules: RulesName,
    pub players: u8,
    pub max_players: u8,
    pub has_password: bool,
    pub running: bool,
    /// The climate, such as `temperate`; empty unknown.
    pub map: Text<32>,
    /// The game's year; 0 unknown.
    pub year: u16,
    pub companies: u8,
    pub competitive: bool,
}

/// What a public room's list entry says of its world.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyListing {
    /// The climate of the start save, such as `temperate`.
    pub map: Text<32>,
    /// The start save's year; 0 unknown.
    pub year: u16,
}

/// Rules a room can be played by, as the server offers them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyRules {
    pub name: RulesName,
    pub description: Text<200>,
}

/// Where the room's world is in this player's game.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LobbyWorld {
    /// No world of the room's yet.
    #[default]
    None,
    /// Coming from the room: `bytes` of `total` so far.
    Fetching { bytes: u64, total: u64 },
    /// The game loads it.
    Loading,
    /// The game plays it.
    Playing,
}

impl Default for LobbyView {
    /// Not connected, no room, nothing said.
    fn default() -> Self {
        Self {
            connection: LobbyConnection::Disconnected,
            server: Text::lossy(""),
            server_address: Text::lossy(""),
            server_default: Text::lossy(""),
            banner: None,
            name: Text::lossy(""),
            error: None,
            notice: None,
            room: None,
            chat: BoundedVec::empty(),
            rules: BoundedVec::empty(),
            saves: BoundedVec::empty(),
            start_save: None,
            world: LobbyWorld::None,
            differences: None,
            mods: BoundedVec::empty(),
            room_mods: BoundedVec::empty(),
            room_mods_more: 0,
            rooms: None,
        }
    }
}

/// Whether the launcher is connected to its server.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LobbyConnection {
    #[default]
    Disconnected,
    Connecting,
    Connected,
}

/// The room the player is in, as its lobby shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyRoom {
    pub name: Text<48>,
    pub rules: RulesName,
    /// What to send friends.
    pub invite: Option<Text<128>>,
    /// The room's game has begun.
    pub running: bool,
    pub you_own: bool,
    pub max_players: u8,
    pub has_password: bool,
    pub members: BoundedVec<LobbyMember, { MAX_ROOM_MEMBERS as usize }>,
    /// Co-op (`false`) or competitive (`true`).
    pub competitive: bool,
}

/// One member of the room, as its lobby shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyMember {
    pub player: PlayerId,
    pub name: Text<32>,
    pub ready: bool,
    pub connected: bool,
    pub owner: bool,
    pub you: bool,
    /// Whether this member's game matches the owner's: `None` while either
    /// has not said.
    pub same_content: Option<bool>,
    /// The banner this member picked (`tpf3mp_proto::BANNERS`), if any.
    pub banner: Option<tpf3mp_proto::BannerId>,
}

/// One line of the room's chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyLine {
    pub from: Text<32>,
    pub text: ChatText,
    pub you: bool,
}

/// What the player asks for in the main menu's Multiplayer window: the
/// launcher's own actions (D17). The launcher carries them out as if its
/// window had asked, on the server it plays on (D12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LobbyAction {
    Connect {
        name: Text<32>,
    },
    Disconnect,
    Create {
        room: Text<48>,
        max_players: u8,
        password: Option<Text<64>>,
        /// One of the server's rules; its default without.
        rules: Option<RulesName>,
        /// The save the room starts from, which every game loads from its
        /// menu; without, the launcher's own (`--start-save`), if any.
        start_save: Option<SaveName>,
        /// `Some` lists the room in the server's room list; `None` keeps it
        /// private.
        listing: Option<LobbyListing>,
        /// Competitive rather than co-op.
        competitive: bool,
    },
    /// Asks for page `page` of the server's public rooms.
    ListRooms {
        page: u16,
    },
    Join {
        invite: Text<128>,
        password: Option<Text<64>>,
    },
    Ready {
        ready: bool,
    },
    Start,
    Kick {
        player: PlayerId,
    },
    Chat {
        text: ChatText,
    },
    Leave,
    /// Play with the installed mod `id`, or not: a personal one, or a
    /// carried one with `--personal-game-scripts` (docs/MODS.md). The
    /// launcher remembers it for next time.
    ChooseMod {
        id: ModName,
        chosen: bool,
    },
    /// The player's server setting: play on `server`, a `host:port`, from
    /// now on; empty goes back to the launcher's default. The launcher
    /// checks it, remembers it, and reconnects there if connected. Refused
    /// in a room. Invites never change the server: only this does (D12).
    SetServer {
        server: Text<128>,
    },
    /// Show this banner in rooms; `None` for the default.
    SetBanner {
        banner: Option<tpf3mp_proto::BannerId>,
    },
}

/// The room as the game's Multiplayer window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomInfo {
    pub name: Text<48>,
    pub owner: PlayerId,
    pub members: BoundedVec<RoomMember, { MAX_ROOM_MEMBERS as usize }>,
}

/// One member of the room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomMember {
    pub player: PlayerId,
    pub name: Text<32>,
    pub connected: bool,
}

/// From the hook to the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToAgent {
    /// The first message after the link opens.
    Hello { version: u32, build: Text<64> },
    /// The world is loaded, and `next_step` is the first step it will run.
    Loaded { next_step: u64 },
    /// The local player did something: have the room order it, with the
    /// password it needs, if any (a company's), which the room seals.
    Command {
        payload: Payload,
        secret: Option<Secret>,
    },
    /// The game ran this step.
    Ran { step: u64 },
    /// The world's digests at a checkpoint step, taken after running it.
    Checkpoint { step: u64, lanes: Vec<LaneDigest> },
    /// A line for the agent's log.
    Log { message: Text<256> },
    /// The world was saved at the save event `event`, into `file`, or
    /// `None` if saving failed. `lanes` are its digests there.
    Saved {
        event: u64,
        lanes: Vec<LaneDigest>,
        file: Option<Text<MAX_PATH>>,
    },
    /// The player says something to the room.
    Chat { text: ChatText },
    /// The player picked this speed in the game's speed row: ask the room
    /// for it. Only the room's owner may change the room's speed; the server
    /// refuses anyone else, and the agent shows the refusal.
    Speed { speed: Speed },
    /// Before the room begins a game: a world is up in the game, with the
    /// mod linked to the hook, and the game steps it. `world` counts the
    /// worlds whose GUI started since the hook began, from 1, so a world is
    /// told once and a new one has a higher number. The agent marks the
    /// player ready in the room's lobby, once per world.
    WorldUp { world: u64 },
    /// Before the room begins a game: the game is at its main menu, with no
    /// world up, and can load the room's world from there when the room
    /// sends one ([`ToHook::Load`] with a file). `menu` counts the times the
    /// game came to its menu since the hook began, from 1. The agent marks
    /// a player other than the room's owner ready, once per `menu`, if it
    /// keeps worlds; the owner's game needs a world up, to save it for the
    /// room.
    MenuUp { menu: u64 },
    /// The player asked for this in the main menu's Multiplayer window.
    Lobby(LobbyAction),
    /// The player's pointer moved or their build tool is previewing.
    Cursor(Cursor),
}

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("the message is {0} bytes, over the {MAX_MESSAGE}-byte limit")]
    TooLarge(usize),
    #[error("the message does not decode: {0}")]
    Malformed(#[from] postcard::Error),
    #[error("the peer speaks bridge version {0}, not {BRIDGE_VERSION}")]
    Version(u32),
}

/// Encodes a message for the link.
pub fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, BridgeError> {
    let bytes = postcard::to_stdvec(message)?;
    if bytes.len() > MAX_MESSAGE {
        return Err(BridgeError::TooLarge(bytes.len()));
    }
    Ok(bytes)
}

/// Decodes a message from the link. Trailing bytes are refused.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, BridgeError> {
    if bytes.len() > MAX_MESSAGE {
        return Err(BridgeError::TooLarge(bytes.len()));
    }
    let (message, rest) = postcard::take_from_bytes(bytes)?;
    if !rest.is_empty() {
        return Err(BridgeError::Malformed(
            postcard::Error::DeserializeBadEncoding,
        ));
    }
    Ok(message)
}

/// Checks a peer's hello.
pub fn check_version(version: u32) -> Result<(), BridgeError> {
    if version == BRIDGE_VERSION {
        Ok(())
    } else {
        Err(BridgeError::Version(version))
    }
}

#[cfg(test)]
mod tests {
    use tpf3mp_proto::{EventBody, FixedBytes, MAX_PAYLOAD, PlayerId};

    use super::*;

    #[test]
    fn messages_round_trip() {
        let to_hook = ToHook::Apply(Event {
            seq: 7,
            step: 3,
            body: EventBody::Command {
                player: PlayerId(FixedBytes([1; 32])),
                client_seq: 9,
                payload: Payload::new(vec![4, 5, 6]).unwrap(),
                seal: None,
            },
        });
        assert_eq!(
            decode::<ToHook>(&encode(&to_hook).unwrap()).unwrap(),
            to_hook
        );
        let to_agent = ToAgent::Checkpoint {
            step: 50,
            lanes: vec![LaneDigest {
                lane: 2,
                digest: FixedBytes([8; 32]),
            }],
        };
        assert_eq!(
            decode::<ToAgent>(&encode(&to_agent).unwrap()).unwrap(),
            to_agent
        );
        for to_agent in [ToAgent::WorldUp { world: 2 }, ToAgent::MenuUp { menu: 3 }] {
            assert_eq!(
                decode::<ToAgent>(&encode(&to_agent).unwrap()).unwrap(),
                to_agent
            );
        }
    }

    #[test]
    fn the_largest_intent_fits_in_one_message() {
        let apply = ToHook::Apply(Event {
            seq: u64::MAX,
            step: u64::MAX,
            body: EventBody::Command {
                player: PlayerId(FixedBytes([0xff; 32])),
                client_seq: u64::MAX,
                payload: Payload::new(vec![0xab; MAX_PAYLOAD]).unwrap(),
                seal: Some(tpf3mp_proto::Seal {
                    scope: u64::MAX,
                    tag: FixedBytes([0xff; 32]),
                }),
            },
        });
        assert!(encode(&apply).is_ok());
    }

    #[test]
    fn malformed_and_oversized_messages_are_refused() {
        let mut bytes = encode(&ToAgent::Ran { step: 5 }).unwrap();
        bytes.push(0);
        assert!(matches!(
            decode::<ToAgent>(&bytes),
            Err(BridgeError::Malformed(_))
        ));
        assert!(matches!(
            decode::<ToAgent>(&[0xff; 3]),
            Err(BridgeError::Malformed(_))
        ));
        assert!(matches!(
            decode::<ToAgent>(&vec![0; MAX_MESSAGE + 1]),
            Err(BridgeError::TooLarge(_))
        ));
    }

    #[test]
    fn the_fullest_lobby_fits_in_one_message() {
        let member = |n: u8| LobbyMember {
            player: tpf3mp_proto::PlayerId(FixedBytes([n; 32])),
            name: Text::new("x".repeat(32)).unwrap(),
            ready: true,
            connected: true,
            owner: n == 0,
            you: n == 1,
            same_content: Some(true),
            banner: None,
        };
        let line = LobbyLine {
            from: Text::new("y".repeat(32)).unwrap(),
            text: Text::new("z".repeat(280)).unwrap(),
            you: false,
        };
        let view = ToHook::Lobby(Box::new(LobbyView {
            connection: LobbyConnection::Connected,
            server: Text::new("s".repeat(128)).unwrap(),
            server_address: Text::new("a".repeat(128)).unwrap(),
            server_default: Text::new("d".repeat(128)).unwrap(),
            banner: Some(Text::new("b".repeat(16)).unwrap()),
            name: Text::new("n".repeat(32)).unwrap(),
            error: Some(Text::new("e".repeat(256)).unwrap()),
            notice: Some(Text::new("o".repeat(256)).unwrap()),
            room: Some(LobbyRoom {
                name: Text::new("r".repeat(48)).unwrap(),
                rules: Text::new("native").unwrap(),
                invite: Some(Text::new("i".repeat(128)).unwrap()),
                running: false,
                you_own: true,
                max_players: 64,
                has_password: true,
                members: BoundedVec::new((0..MAX_ROOM_MEMBERS).map(member).collect()).unwrap(),
                competitive: false,
            }),
            chat: BoundedVec::new(vec![line; MAX_LOBBY_CHAT]).unwrap(),
            rules: BoundedVec::new(vec![
                LobbyRules {
                    name: Text::new("r".repeat(32)).unwrap(),
                    description: Text::new("d".repeat(200)).unwrap(),
                };
                MAX_LOBBY_RULES
            ])
            .unwrap(),
            saves: BoundedVec::new(vec![
                Text::new("s".repeat(MAX_SAVE_NAME)).unwrap();
                MAX_LOBBY_SAVES
            ])
            .unwrap(),
            start_save: Some(Text::new("s".repeat(MAX_SAVE_NAME)).unwrap()),
            world: LobbyWorld::Fetching {
                bytes: u64::MAX,
                total: u64::MAX,
            },
            differences: Some(Text::new("d".repeat(256)).unwrap()),
            mods: BoundedVec::new(vec![
                LobbyMod {
                    id: Text::new("m".repeat(96)).unwrap(),
                    name: Text::new("n".repeat(48)).unwrap(),
                    class: LobbyModClass::Carried,
                    reason: Text::new("r".repeat(96)).unwrap(),
                    chosen: true,
                    choosable: true,
                };
                MAX_LOBBY_MODS
            ])
            .unwrap(),
            room_mods: BoundedVec::new(vec![
                LobbyRoomMod {
                    id: Text::new("m".repeat(96)).unwrap(),
                    version: Text::new("v".repeat(32)).unwrap(),
                    have: LobbyHave::OtherVersion,
                };
                MAX_LOBBY_ROOM_MODS
            ])
            .unwrap(),
            room_mods_more: u32::MAX,
            rooms: Some(LobbyRoomList {
                page: u16::MAX,
                rooms: BoundedVec::new(vec![
                    LobbyPublicRoom {
                        invite: Text::new("i".repeat(128)).unwrap(),
                        name: Text::new("n".repeat(48)).unwrap(),
                        rules: Text::new("r".repeat(32)).unwrap(),
                        players: u8::MAX,
                        max_players: u8::MAX,
                        has_password: true,
                        running: true,
                        map: Text::new("m".repeat(32)).unwrap(),
                        year: u16::MAX,
                        companies: u8::MAX,
                        competitive: true,
                    };
                    tpf3mp_proto::ROOMS_PER_PAGE
                ])
                .unwrap(),
                more: true,
            }),
        }));
        let bytes = encode(&view).unwrap();
        assert_eq!(decode::<ToHook>(&bytes).unwrap(), view);
        let action = ToAgent::Lobby(LobbyAction::Create {
            room: Text::new("Alps").unwrap(),
            max_players: 4,
            password: None,
            rules: Some(Text::new("native").unwrap()),
            start_save: Some(Text::new("mptest").unwrap()),
            listing: Some(LobbyListing {
                map: Text::new("temperate").unwrap(),
                year: 1850,
            }),
            competitive: true,
        });
        assert_eq!(
            decode::<ToAgent>(&encode(&action).unwrap()).unwrap(),
            action
        );
        let choose = ToAgent::Lobby(LobbyAction::ChooseMod {
            id: Text::new("schbrongx_minimap").unwrap(),
            chosen: true,
        });
        assert_eq!(
            decode::<ToAgent>(&encode(&choose).unwrap()).unwrap(),
            choose
        );
        let set = ToAgent::Lobby(LobbyAction::SetServer {
            server: Text::new("s".repeat(128)).unwrap(),
        });
        assert_eq!(decode::<ToAgent>(&encode(&set).unwrap()).unwrap(), set);
    }

    #[test]
    fn only_the_same_version_is_accepted() {
        assert!(check_version(BRIDGE_VERSION).is_ok());
        assert!(check_version(BRIDGE_VERSION + 1).is_err());
    }
}
