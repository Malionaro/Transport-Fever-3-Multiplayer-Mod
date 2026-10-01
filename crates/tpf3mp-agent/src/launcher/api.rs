//! What a launcher front end shows ([`State`]) and what it asks for
//! ([`Action`]): the web page as JSON, the native window as Rust values.

use serde::{Deserialize, Serialize};
use tpf3mp_proto::{
    Arch, ContentDiff, FixedBytes, ModRef, Os, Platform, PlayerId, RoomPhase, RulesOffer,
};

use crate::bridge::{Status, WorldStatus};

/// What the launcher itself knows, next to the session's [`Status`].
#[derive(Debug, Default)]
pub(crate) struct View {
    /// The banner this player picked, if any.
    pub(crate) banner: Option<String>,
    pub(crate) server: Option<String>,
    /// The server is the one this launcher plays on: no invite goes
    /// elsewhere (D12).
    pub(crate) server_fixed: bool,
    /// The launcher's default server, which "Reset to default" goes back to.
    pub(crate) server_default: Option<String>,
    /// What players see of the default server, such as `EU`.
    pub(crate) server_name: Option<String>,
    pub(crate) name: String,
    pub(crate) player: Option<PlayerId>,
    pub(crate) connecting: bool,
    pub(crate) connected: bool,
    /// The connection runs through a tunnel, not over UDP.
    pub(crate) tunneled: bool,
    pub(crate) server_version: Option<String>,
    /// The server's name for the connection outside a room, which its log
    /// uses.
    pub(crate) session: Option<String>,
    /// The rules the server offers new rooms, the default first.
    pub(crate) rules: Vec<RulesOffer>,
    pub(crate) in_room: bool,
    pub(crate) invite: Option<String>,
    pub(crate) error: Option<String>,
    /// Transport Fever 3 as Steam installed it.
    pub(crate) installed: Option<crate::steam::Installed>,
    /// The last server connected to speaks a newer protocol.
    pub(crate) outdated: bool,
    /// Whether the player's log goes to the server; `None` when this
    /// launcher has no diagnostics to send.
    pub(crate) diagnostics: Option<bool>,
    /// The player's saves, newest first, as last looked at.
    pub(crate) saves: Vec<String>,
    /// The save rooms this player creates start from, unless they pick
    /// another.
    pub(crate) start_save: Option<String>,
    /// The player's installed mods, and whether each is chosen.
    pub(crate) mods: Vec<ModRow>,
    /// The room's shared mods, and whether this player has each.
    pub(crate) room_mods: Vec<RoomModRow>,
    /// The page of public rooms last asked for, while connected.
    pub(crate) rooms: Option<RoomList>,
}

/// Something the player asks for.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    Connect {
        server: String,
        name: String,
    },
    Disconnect,
    Create {
        room: String,
        max_players: u8,
        password: Option<String>,
        /// One of the server's rules; the default without.
        #[serde(default)]
        rules: Option<String>,
        /// One of the player's saves (`State::saves`), by name, that the
        /// room starts from: every game loads it from its menu. Without,
        /// the launcher's own start save, if it has one; empty, none, and
        /// the owner's game loads a world and saves it for the room.
        #[serde(default)]
        start_save: Option<String>,
        /// `Some` lists the room in the server's room list for anyone to
        /// see and join; `None`, the default, keeps it private.
        #[serde(default)]
        listing: Option<Listing>,
        /// Competitive (each player for a company of their own) rather
        /// than co-op, the default.
        #[serde(default)]
        competitive: bool,
    },
    /// Asks the server for page `page` of its public rooms
    /// ([`State::rooms`]).
    ListRooms {
        page: u16,
    },
    Join {
        invite: String,
        password: Option<String>,
    },
    Ready {
        ready: bool,
    },
    Start,
    Kick {
        player: String,
    },
    Chat {
        text: String,
    },
    Leave,
    /// Sends this player's diagnostics to the server, or stops.
    Diagnostics {
        on: bool,
    },
    /// Starts Transport Fever 3 with TPF3-MP's hook in it, for this room.
    LaunchGame,
    /// Plays with an installed personal mod, or not (docs/MODS.md).
    ChooseMod {
        id: String,
        chosen: bool,
    },
    /// The player's server setting: play on `server`, a `host:port`, from
    /// now on; empty goes back to the default ([`State::server_default`]).
    /// Remembered; reconnects there if connected; refused in a room.
    /// Shows this banner in rooms: one of `tpf3mp_proto::BANNERS`, or
    /// `None` for the default. Remembered for next time.
    SetBanner {
        #[serde(default)]
        banner: Option<String>,
    },
    SetServer {
        server: String,
    },
}

/// What a public room's list entry says of its world, as the creating
/// player's game read it from the start save.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Listing {
    /// The climate, such as `temperate`.
    #[serde(default)]
    pub map: String,
    /// The start year; 0 unknown.
    #[serde(default)]
    pub year: u16,
}

/// A page of the server's public rooms.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RoomList {
    pub page: u16,
    pub rooms: Vec<PublicRoom>,
    /// A later page has more.
    pub more: bool,
}

/// One public room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublicRoom {
    pub invite: String,
    pub name: String,
    pub rules: String,
    pub players: u8,
    pub max_players: u8,
    pub has_password: bool,
    pub running: bool,
    pub map: String,
    pub year: u16,
    pub companies: u8,
    pub competitive: bool,
}

impl RoomList {
    pub(crate) fn of(page: &tpf3mp_proto::RoomPage) -> Self {
        Self {
            page: page.page,
            more: page.more,
            rooms: page
                .rooms
                .iter()
                .map(|room| PublicRoom {
                    invite: room.invite.to_string(),
                    name: room.name.as_str().to_owned(),
                    rules: room.rules.as_str().to_owned(),
                    players: room.players,
                    max_players: room.max_players,
                    has_password: room.has_password,
                    running: room.phase == RoomPhase::Running,
                    map: room.listing.map.as_str().to_owned(),
                    year: room.listing.year,
                    companies: room.listing.companies,
                    competitive: room.competitive,
                })
                .collect(),
        }
    }
}

/// Everything a launcher front end shows: the web page reads it as JSON,
/// the native window as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct State {
    pub name: String,
    /// The banner this player picked, if any ([`Action::SetBanner`]).
    pub banner: Option<String>,
    /// This player's short ID, as others see it.
    pub player: Option<String>,
    pub server: Option<String>,
    /// Whether `server` is the server this launcher plays on (D12, as
    /// amended): Connect then asks for no server, invites join on it and an
    /// invite to another is refused; the server setting
    /// ([`Action::SetServer`]) changes it.
    pub server_fixed: bool,
    /// The launcher's default server, `host:port`: the package's, or the
    /// project's relay. "Reset to default" goes back to it.
    pub server_default: Option<String>,
    /// What players see of the server, such as `EU`, in place of its
    /// address, while it is the default one; `None` shows the address.
    pub server_name: Option<String>,
    pub server_version: Option<String>,
    /// What the player quotes to the server's operator: the connection's
    /// name in the server's log.
    pub support_id: Option<String>,
    /// The rules the server offers new rooms, the default first.
    pub rules: Vec<RulesChoice>,
    pub connection: Connection,
    /// The connection runs through a tunnel, not over UDP.
    pub tunneled: bool,
    /// What went wrong last, until something succeeds.
    pub error: Option<String>,
    /// The server speaks a newer protocol than this TPF3-MP: it must be
    /// updated to play there.
    pub outdated: bool,
    pub room: Option<Room>,
    /// How this player's game differs from the room's, while it does.
    pub content_diff: Option<Differences>,
    pub game: Game,
    /// Transport Fever 3 on this machine, as Steam installed it.
    pub installed: Option<InstalledGame>,
    pub chat: Vec<ChatLine>,
    /// What the player should know, oldest first.
    pub notices: Vec<String>,
    /// The server operator's latest notice, such as a restart coming.
    pub announcement: Option<String>,
    /// Whether lines of this launcher's log, redacted, go to the server
    /// ("Diagnostics" in PROTOCOL.md); `None` when it sends none at all.
    pub diagnostics: Option<bool>,
    /// The player's saves, newest first: what a room they create can start
    /// from ([`Action::Create`]).
    pub saves: Vec<String>,
    /// The save rooms this player creates start from unless they pick
    /// another: the launcher's `--start-save`, or the one last picked.
    pub start_save: Option<String>,
    /// The page of the server's public rooms last asked for
    /// ([`Action::ListRooms`]).
    pub rooms: Option<RoomList>,
    /// The mods this player has installed, those they may choose first
    /// ([`Action::ChooseMod`]; docs/MODS.md).
    pub mods: Vec<ModRow>,
    /// The room's shared mods, from its owner's start save, and whether this
    /// player has each; empty while not known.
    pub room_mods: Vec<RoomModRow>,
}

/// One installed mod, as the front ends list it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModRow {
    pub id: String,
    pub name: String,
    pub class: ModClass,
    /// Why it is of its class, in a line.
    pub reason: String,
    pub chosen: bool,
    pub choosable: bool,
}

/// What the scan made of a mod.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModClass {
    Personal,
    Carried,
    Shared,
}

/// One of the room's shared mods, and whether this player has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoomModRow {
    pub id: String,
    pub version: String,
    pub have: ModHave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModHave {
    Yes,
    No,
    OtherVersion,
}

/// The picker's mods as the front ends list them: those the player may
/// choose first, then the rest, each group by name.
pub(crate) fn mod_rows(mods: &crate::picker::Mods) -> (Vec<ModRow>, Vec<RoomModRow>) {
    use tpf3mp_modscan::Class;
    let mut rows: Vec<ModRow> = mods
        .installed()
        .iter()
        .map(|m| ModRow {
            id: m.id.clone(),
            name: m.name.clone(),
            class: match m.class {
                Class::Personal => ModClass::Personal,
                Class::Carried => ModClass::Carried,
                Class::Shared => ModClass::Shared,
            },
            reason: m.reason.clone(),
            chosen: mods.is_chosen(&m.id),
            choosable: mods.is_choosable(&m.id),
        })
        .collect();
    rows.sort_by(|a, b| {
        (!a.choosable, a.name.to_lowercase()).cmp(&(!b.choosable, b.name.to_lowercase()))
    });
    let room = mods
        .required()
        .into_iter()
        .map(|r| RoomModRow {
            id: r.id,
            version: r.version,
            have: match r.have {
                crate::picker::Have::Yes => ModHave::Yes,
                crate::picker::Have::No => ModHave::No,
                crate::picker::Have::OtherVersion => ModHave::OtherVersion,
            },
        })
        .collect();
    (rows, room)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Connection {
    #[default]
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RulesChoice {
    pub name: String,
    pub description: String,
}

/// How this player's game differs from the room's, ready to show.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Differences {
    /// All of it in a sentence.
    pub summary: String,
    /// The room's build and this player's, when they differ.
    pub game: Option<(String, String)>,
    /// Mods the room runs and this player does not, as "name version".
    pub missing: Vec<String>,
    /// How many more there are than `missing` names.
    pub missing_more: u32,
    /// Mods this player runs and the room does not.
    pub extra: Vec<String>,
    pub extra_more: u32,
    /// Mod, the room's version, this player's.
    pub changed: Vec<(String, String, String)>,
    pub changed_more: u32,
    /// The same mods, loaded in another order.
    pub reordered: bool,
    /// The mods beyond the listed ones differ.
    pub unlisted: bool,
}

impl Differences {
    fn of(diff: &ContentDiff) -> Self {
        let named = |mods: &[ModRef]| -> Vec<String> {
            mods.iter()
                .map(|listed| format!("{} {}", listed.id, listed.version))
                .collect()
        };
        let more = |total: u32, listed: usize| {
            total.saturating_sub(u32::try_from(listed).unwrap_or(u32::MAX))
        };
        Self {
            summary: diff.to_string(),
            game: diff
                .game
                .as_ref()
                .map(|builds| (builds.room.to_string(), builds.yours.to_string())),
            missing: named(&diff.missing),
            missing_more: more(diff.missing_total, diff.missing.len()),
            extra: named(&diff.extra),
            extra_more: more(diff.extra_total, diff.extra.len()),
            changed: diff
                .changed
                .iter()
                .map(|change| {
                    (
                        change.id.to_string(),
                        change.room.to_string(),
                        change.yours.to_string(),
                    )
                })
                .collect(),
            changed_more: more(diff.changed_total, diff.changed.len()),
            reordered: diff.reordered,
            unlisted: diff.unlisted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Room {
    pub name: String,
    pub rules: String,
    pub phase: Phase,
    /// What to send friends: the server and the room's invite.
    pub invite: Option<String>,
    pub you_own: bool,
    pub max_players: u8,
    pub has_password: bool,
    pub members: Vec<Member>,
    /// Co-op (`false`) or competitive (`true`), as its owner chose.
    pub competitive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Lobby,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Member {
    /// The player's full key, which [`Action::Kick`] takes.
    pub id: String,
    pub name: String,
    pub platform: String,
    pub ready: bool,
    pub connected: bool,
    pub owner: bool,
    pub you: bool,
    /// Whether this member's game matches the owner's.
    pub content: MemberContent,
    /// The banner the member picked, if any (`tpf3mp_proto::BANNERS`).
    pub banner: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberContent {
    Same,
    Differs,
    /// The member or the owner has not declared theirs.
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Game {
    /// The game's build, once its hook attached.
    pub attached: Option<String>,
    pub world: World,
    /// While fetching: bytes received of `total`.
    pub bytes: u64,
    pub total: u64,
    /// The last step the game ran.
    pub step: Option<u64>,
    /// The room's speed in percent; 0 is paused.
    pub speed: u16,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum World {
    #[default]
    None,
    Fetching,
    Loading,
    Playing,
}

/// The game Steam installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstalledGame {
    pub dir: String,
    /// Steam's build ID.
    pub build: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatLine {
    pub from: String,
    pub text: String,
    pub you: bool,
}

/// What the launcher shows now.
pub(crate) fn snapshot(view: &View, status: &Status) -> State {
    let you = view.player;
    let connection = if view.connected {
        Connection::Connected
    } else if view.connecting {
        Connection::Connecting
    } else {
        Connection::Disconnected
    };
    let room = status.room.as_ref().filter(|_| view.in_room).map(|room| {
        let owners = room
            .members
            .iter()
            .find(|member| member.player == room.owner)
            .and_then(|owner| owner.content);
        Room {
            name: room.name.as_str().to_owned(),
            rules: room.rules.as_str().to_owned(),
            phase: match room.phase {
                RoomPhase::Lobby => Phase::Lobby,
                RoomPhase::Running => Phase::Running,
            },
            invite: view.invite.clone(),
            you_own: Some(room.owner) == you,
            max_players: room.max_players,
            has_password: room.has_password,
            members: room
                .members
                .iter()
                .map(|member| Member {
                    id: player_hex(&member.player),
                    name: member.name.as_str().to_owned(),
                    platform: platform_name(member.platform),
                    ready: member.ready,
                    connected: member.connected,
                    owner: member.player == room.owner,
                    you: Some(member.player) == you,
                    content: match (owners, member.content) {
                        (Some(owners), Some(theirs)) if owners == theirs => MemberContent::Same,
                        (Some(_), Some(_)) => MemberContent::Differs,
                        _ => MemberContent::Unknown,
                    },
                    banner: member.banner.as_ref().map(|id| id.as_str().to_owned()),
                })
                .collect(),
            competitive: room.competitive,
        }
    });
    let (world, bytes, total) = match status.world {
        WorldStatus::None => (World::None, 0, 0),
        WorldStatus::Fetching { bytes, total } => (World::Fetching, bytes, total),
        WorldStatus::Loading => (World::Loading, 0, 0),
        WorldStatus::Playing => (World::Playing, 0, 0),
    };
    let name_of = |player: &PlayerId| {
        status
            .room
            .as_ref()
            .and_then(|room| room.members.iter().find(|member| member.player == *player))
            .map_or_else(
                || player.to_string(),
                |member| member.name.as_str().to_owned(),
            )
    };
    State {
        name: view.name.clone(),
        banner: view.banner.clone(),
        player: you.map(|player| player.to_string()),
        server: view.server.clone(),
        server_fixed: view.server_fixed,
        server_default: view.server_default.clone(),
        // The name is the default server's: another shows its address.
        server_name: view.server_name.clone().filter(|_| {
            match (&view.server_default, &view.server) {
                (Some(default), Some(server)) => super::same_server(default, server),
                (Some(_), None) => false,
                (None, _) => true,
            }
        }),
        server_version: view.server_version.clone(),
        support_id: status
            .session
            .map(|session| session.to_string())
            .or_else(|| view.session.clone())
            .filter(|_| view.connected || view.in_room),
        rules: view
            .rules
            .iter()
            .map(|offer| RulesChoice {
                name: offer.name.as_str().to_owned(),
                description: offer.description.as_str().to_owned(),
            })
            .collect(),
        connection,
        tunneled: view.connected && view.tunneled,
        error: view.error.clone(),
        outdated: view.outdated || status.outdated,
        room,
        content_diff: status.content_diff.as_ref().map(Differences::of),
        game: Game {
            attached: status.game.clone(),
            world,
            bytes,
            total,
            step: status.step,
            speed: status.speed.0,
        },
        installed: view.installed.as_ref().map(|installed| InstalledGame {
            dir: installed.dir.display().to_string(),
            build: installed.build.clone(),
        }),
        chat: status
            .chat
            .iter()
            .map(|(from, text)| ChatLine {
                from: name_of(from),
                text: text.as_str().to_owned(),
                you: Some(*from) == you,
            })
            .collect(),
        notices: status.notices.iter().cloned().collect(),
        announcement: status.announcement.clone(),
        diagnostics: view.diagnostics,
        saves: view.saves.clone(),
        start_save: view.start_save.clone(),
        mods: view.mods.clone(),
        room_mods: view.room_mods.clone(),
        rooms: view.rooms.clone().filter(|_| view.connected),
    }
}

/// The state the page shows, as JSON.
pub(crate) fn render(view: &View, status: &Status) -> String {
    serde_json::to_string(&snapshot(view, status)).unwrap_or_else(|_| "{}".to_owned())
}

/// A player's full key as 64 hex digits, as the page names players.
pub(crate) fn player_hex(player: &PlayerId) -> String {
    player
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A player named by [`player_hex`].
pub(crate) fn parse_player(text: &str) -> Option<PlayerId> {
    let text = text.trim().trim_start_matches("p-");
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(PlayerId(FixedBytes(bytes)))
}

fn platform_name(platform: Platform) -> String {
    let os = match platform.os {
        Os::Windows => "Windows",
        Os::Linux => "Linux",
        Os::MacOs => "macOS",
        Os::Other => "other",
    };
    let arch = match platform.arch {
        Arch::X86_64 => "x86-64",
        Arch::Aarch64 => "arm64",
        Arch::Other => "other",
    };
    format!("{os} {arch}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_parse_from_the_pages_json() {
        let action: Action =
            serde_json::from_str(r#"{"action":"join","invite":"K7QM2X","password":null}"#).unwrap();
        assert_eq!(
            action,
            Action::Join {
                invite: "K7QM2X".into(),
                password: None
            }
        );
        let action: Action = serde_json::from_str(
            r#"{"action":"create","room":"R","max_players":4,"password":null,"rules":"native"}"#,
        )
        .unwrap();
        assert_eq!(
            action,
            Action::Create {
                room: "R".into(),
                max_players: 4,
                password: None,
                rules: Some("native".into()),
                start_save: None,
                listing: None,
                competitive: false,
            }
        );
        let action: Action = serde_json::from_str(
            r#"{"action":"create","room":"R","max_players":4,"password":null,"start_save":"mptest"}"#,
        )
        .unwrap();
        assert!(matches!(
            action,
            Action::Create { start_save: Some(save), .. } if save == "mptest"
        ));
        let action: Action = serde_json::from_str(r#"{"action":"start"}"#).unwrap();
        assert_eq!(action, Action::Start);
        let action: Action =
            serde_json::from_str(r#"{"action":"set_server","server":"eu.example:29470"}"#).unwrap();
        assert_eq!(
            action,
            Action::SetServer {
                server: "eu.example:29470".into()
            }
        );
        assert!(serde_json::from_str::<Action>(r#"{"action":"format_disk"}"#).is_err());
    }

    #[test]
    fn players_round_trip_through_their_page_names() {
        let player = PlayerId(FixedBytes([0xab; 32]));
        let hex = player_hex(&player);
        assert_eq!(hex.len(), 64);
        assert_eq!(parse_player(&hex), Some(player));
        assert_eq!(parse_player(&format!("p-{hex}")), Some(player));
        assert_eq!(parse_player("abc"), None);
        assert_eq!(parse_player(&"zz".repeat(32)), None);
    }

    #[test]
    fn the_state_says_which_mods_differ() {
        let mods = |list: &[&str]| {
            tpf3mp_proto::ContentManifest::new(
                tpf3mp_proto::Text::new("35924").unwrap(),
                list.iter()
                    .map(|id| ModRef {
                        id: tpf3mp_proto::Text::new(*id).unwrap(),
                        version: tpf3mp_proto::Text::new("1").unwrap(),
                    })
                    .collect(),
            )
        };
        let status = Status {
            content_diff: mods(&["trains", "stations"]).compare(&mods(&["trains", "trees"])),
            ..Status::default()
        };
        let json: serde_json::Value =
            serde_json::from_str(&render(&View::default(), &status)).unwrap();
        let diff = &json["content_diff"];
        assert_eq!(diff["missing"], serde_json::json!(["stations 1"]));
        assert_eq!(diff["extra"], serde_json::json!(["trees 1"]));
        assert_eq!(
            diff["summary"],
            "you lack stations 1; the room lacks trees 1"
        );
        let json: serde_json::Value =
            serde_json::from_str(&render(&View::default(), &Status::default())).unwrap();
        assert!(json["content_diff"].is_null());
    }

    #[test]
    fn the_state_names_the_connection_and_the_room() {
        let view = View {
            name: "Ann".into(),
            connected: true,
            ..View::default()
        };
        let json: serde_json::Value =
            serde_json::from_str(&render(&view, &Status::default())).unwrap();
        assert_eq!(json["connection"], "connected");
        assert_eq!(json["tunneled"], false);
        assert_eq!(json["name"], "Ann");
        assert!(json["room"].is_null());
        assert_eq!(json["game"]["world"], "none");
    }

    #[test]
    fn the_servers_name_is_shown_for_the_default_server_only() {
        let mut view = View {
            server: Some("tpf3mp.example.org:29470".into()),
            server_fixed: true,
            server_default: Some("TPF3MP.example.org:29470".into()),
            server_name: Some("Relay".into()),
            ..View::default()
        };
        let state = snapshot(&view, &Status::default());
        assert_eq!(state.server_name.as_deref(), Some("Relay"));
        assert_eq!(
            state.server_default.as_deref(),
            Some("TPF3MP.example.org:29470")
        );
        // The player chose another server: its address, not the name.
        view.server = Some("play.example.net:29470".into());
        assert_eq!(snapshot(&view, &Status::default()).server_name, None);
        // Without a default, the name given is the server's.
        view.server_default = None;
        assert_eq!(
            snapshot(&view, &Status::default()).server_name.as_deref(),
            Some("Relay")
        );
    }

    #[test]
    fn the_support_id_names_the_current_connection() {
        let mut view = View {
            connected: true,
            session: Some("s-11".into()),
            ..View::default()
        };
        let support_id = |view: &View, status: &Status| {
            let json: serde_json::Value = serde_json::from_str(&render(view, status)).unwrap();
            json["support_id"].as_str().map(str::to_owned)
        };
        assert_eq!(
            support_id(&view, &Status::default()).as_deref(),
            Some("s-11")
        );
        // In a room, the room's connection, which a rejoin renews.
        let status = Status {
            session: Some(tpf3mp_proto::SessionId("AB2CD3".parse().unwrap())),
            ..Status::default()
        };
        assert_eq!(support_id(&view, &status).as_deref(), Some("AB2CD3"));
        // Disconnected, there is none to quote.
        view.connected = false;
        assert_eq!(support_id(&view, &Status::default()), None);
    }
}
