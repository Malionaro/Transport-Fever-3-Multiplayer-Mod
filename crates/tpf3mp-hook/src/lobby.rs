//! The lobby as the main menu's Multiplayer window sees it (D17,
//! docs/LOBBY.md): the launcher's connection, room and chat, and the
//! player's actions back to the launcher, over the link to its agent.
//!
//! The window's Lua asks the hook for the state and hands it actions over
//! the request channel in `crate::menu_entry`. The state crosses as a Lua
//! table literal ([`LobbyState::to_lua`]), which the window evaluates with
//! `load` in an empty environment; an action crosses as one small JSON
//! object ([`parse_action`]), which the window builds by hand. Both are
//! plain text a person can read in a log.
//!
//! Behind the channel the launcher answers, not the hook: an action is
//! queued ([`queue`]) and handed to the agent as a `ToAgent::Lobby` by the
//! step driver ([`exchange`], `StepDriver::lobby`), and the lobby the agent
//! sends back (`ToHook::Lobby`) is what the window shows next. The exchange
//! runs whenever the window asks, which is how the link is read at the main
//! menu, where no step of the game runs, and after each of the game's
//! steps. A game whose hook has no link to its launcher (the step gate did
//! not install) says so, and takes no action (fail closed).

use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard, PoisonError},
};

use serde::Deserialize;
use tpf3mp_bridge::{LobbyAction, LobbyConnection, LobbyView};
use tpf3mp_proto::{FixedBytes, PlayerId, Text};

use crate::step::StepHandler;

/// Most actions waiting for the launcher.
const MAX_QUEUED: usize = 16;

/// Whether the launcher's connection to the server is up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Connection {
    #[default]
    Disconnected,
    Connecting,
    Connected,
}

impl Connection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
        }
    }
}

/// One player in the room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The player's key, as 64 hex digits: what a kick names.
    pub id: String,
    pub name: String,
    pub ready: bool,
    pub owner: bool,
    pub you: bool,
    pub connected: bool,
    /// Whether the player's game matches the owner's: `same`, `differs` or
    /// `unknown`.
    pub content: String,
}

/// The room the player is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Room {
    pub name: String,
    pub invite: String,
    /// `lobby` or `playing`.
    pub phase: String,
    pub you_own: bool,
    pub max_players: u32,
    pub has_password: bool,
    pub members: Vec<Member>,
}

/// One line of the room's chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatLine {
    pub from: String,
    pub text: String,
    pub you: bool,
}

/// Everything the lobby window shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LobbyState {
    pub connection: Connection,
    pub server: String,
    pub name: String,
    /// The last thing that went wrong, for the window to show.
    pub error: Option<String>,
    /// The last thing worth telling the player.
    pub notice: Option<String>,
    pub room: Option<Room>,
    pub chat: Vec<ChatLine>,
    /// Whether this game has a link to its launcher: without it the window
    /// can only say so.
    pub linked: bool,
    /// Whether the launcher has said anything yet.
    pub heard: bool,
}

/// What the window sends, as JSON: the tag `action` plus the fields, e.g.
/// `{"action":"create","room":"Alps","password":"","max_players":8}`. A
/// server the window names is not taken: the launcher plays on its own
/// (D12).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
enum WindowAction {
    Connect {
        name: String,
    },
    Disconnect,
    Create {
        #[serde(default)]
        room: String,
        #[serde(default = "default_max_players")]
        max_players: u32,
        #[serde(default)]
        password: String,
    },
    Join {
        invite: String,
        #[serde(default)]
        password: String,
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
}

fn default_max_players() -> u32 {
    8
}

fn text<const MAX: usize>(value: &str, what: &str) -> Result<Text<MAX>, String> {
    Text::new(value.trim()).map_err(|_| format!("that {what} is too long"))
}

fn password(value: &str) -> Result<Option<Text<64>>, String> {
    if value.is_empty() {
        Ok(None)
    } else {
        Text::new(value)
            .map(Some)
            .map_err(|_| "that password is too long".to_owned())
    }
}

/// A player named by 64 hex digits, as [`Member::id`] names them.
fn player(hex: &str) -> Option<PlayerId> {
    let hex = hex.trim();
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(PlayerId(FixedBytes(bytes)))
}

/// A player's id as the mod names it: 64 lowercase hex digits.
pub(crate) fn hex(player: &PlayerId) -> String {
    player
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Parses one action from the window's JSON into what the launcher takes.
pub fn parse_action(json: &str) -> Result<LobbyAction, String> {
    let action: WindowAction =
        serde_json::from_str(json).map_err(|error| format!("not an action: {error}"))?;
    Ok(match action {
        WindowAction::Connect { name } => LobbyAction::Connect {
            name: text(&name, "name")?,
        },
        WindowAction::Disconnect => LobbyAction::Disconnect,
        WindowAction::Create {
            room,
            max_players,
            password: given,
        } => LobbyAction::Create {
            room: text(&room, "room name")?,
            max_players: u8::try_from(max_players).unwrap_or(u8::MAX),
            password: password(&given)?,
        },
        WindowAction::Join {
            invite,
            password: given,
        } => LobbyAction::Join {
            invite: text(&invite, "invite")?,
            password: password(&given)?,
        },
        WindowAction::Ready { ready } => LobbyAction::Ready { ready },
        WindowAction::Start => LobbyAction::Start,
        WindowAction::Kick { player: id } => LobbyAction::Kick {
            player: player(&id).ok_or("that is not a player")?,
        },
        WindowAction::Chat { text: said } => LobbyAction::Chat {
            text: text(&said, "message")?,
        },
        WindowAction::Leave => LobbyAction::Leave,
    })
}

/// What kind of action this is, for the log: never its fields, since a join
/// carries an invite and a password (D13).
pub fn kind(action: &LobbyAction) -> &'static str {
    match action {
        LobbyAction::Connect { .. } => "connect",
        LobbyAction::Disconnect => "disconnect",
        LobbyAction::Create { .. } => "create",
        LobbyAction::Join { .. } => "join",
        LobbyAction::Ready { .. } => "ready",
        LobbyAction::Start => "start",
        LobbyAction::Kick { .. } => "kick",
        LobbyAction::Chat { .. } => "chat",
        LobbyAction::Leave => "leave",
    }
}

/// A Lua string literal for `text`, safe for any bytes.
fn lua_str(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for byte in text.bytes() {
        match byte {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0..=0x1f | 0x7f => out.push_str(&format!("\\{byte}")),
            // Bytes of UTF-8 above ASCII go through as decimal escapes, so
            // the literal is ASCII and the window's Lua gets the same bytes.
            0x80..=0xff => out.push_str(&format!("\\{byte}")),
            other => out.push(other as char),
        }
    }
    out.push('"');
    out
}

fn lua_opt(text: Option<&str>) -> String {
    text.map_or_else(|| "nil".to_owned(), lua_str)
}

impl LobbyState {
    /// The empty lobby: not connected, no room, nothing said, no launcher.
    /// Usable in a `static`.
    pub const fn new() -> Self {
        Self {
            connection: Connection::Disconnected,
            server: String::new(),
            name: String::new(),
            error: None,
            notice: None,
            room: None,
            chat: Vec::new(),
            linked: false,
            heard: false,
        }
    }

    /// What the window shows of the launcher's lobby `view`, if it sent one.
    pub fn of(view: Option<&LobbyView>, linked: bool) -> Self {
        let Some(view) = view else {
            return Self {
                linked,
                ..Self::new()
            };
        };
        Self {
            connection: match view.connection {
                LobbyConnection::Disconnected => Connection::Disconnected,
                LobbyConnection::Connecting => Connection::Connecting,
                LobbyConnection::Connected => Connection::Connected,
            },
            server: view.server.as_str().to_owned(),
            name: view.name.as_str().to_owned(),
            error: view.error.as_ref().map(|text| text.as_str().to_owned()),
            notice: view.notice.as_ref().map(|text| text.as_str().to_owned()),
            room: view.room.as_ref().map(|room| Room {
                name: room.name.as_str().to_owned(),
                invite: room
                    .invite
                    .as_ref()
                    .map(|invite| invite.as_str().to_owned())
                    .unwrap_or_default(),
                phase: if room.running { "playing" } else { "lobby" }.to_owned(),
                you_own: room.you_own,
                max_players: u32::from(room.max_players),
                has_password: room.has_password,
                members: room
                    .members
                    .iter()
                    .map(|member| Member {
                        id: hex(&member.player),
                        name: member.name.as_str().to_owned(),
                        ready: member.ready,
                        owner: member.owner,
                        you: member.you,
                        connected: member.connected,
                        content: match member.same_content {
                            Some(true) => "same",
                            Some(false) => "differs",
                            None => "unknown",
                        }
                        .to_owned(),
                    })
                    .collect(),
            }),
            chat: view
                .chat
                .iter()
                .map(|line| ChatLine {
                    from: line.from.as_str().to_owned(),
                    text: line.text.as_str().to_owned(),
                    you: line.you,
                })
                .collect(),
            linked,
            heard: true,
        }
    }

    /// The state as a Lua table literal: `{ connection = "…", … }`.
    pub fn to_lua(&self) -> String {
        let mut out = String::with_capacity(512);
        out.push_str("{ connection = ");
        out.push_str(lua_str(self.connection.as_str()).as_str());
        out.push_str(", server = ");
        out.push_str(&lua_str(&self.server));
        out.push_str(", name = ");
        out.push_str(&lua_str(&self.name));
        out.push_str(", error = ");
        out.push_str(&lua_opt(self.error.as_deref()));
        out.push_str(", notice = ");
        out.push_str(&lua_opt(self.notice.as_deref()));
        out.push_str(", linked = ");
        out.push_str(if self.linked { "true" } else { "false" });
        out.push_str(", heard = ");
        out.push_str(if self.heard { "true" } else { "false" });
        out.push_str(", chat = {");
        for line in &self.chat {
            out.push_str(&format!(
                " {{ from = {}, text = {}, you = {} }},",
                lua_str(&line.from),
                lua_str(&line.text),
                line.you
            ));
        }
        out.push_str(" }");
        match &self.room {
            None => out.push_str(", room = nil"),
            Some(room) => {
                out.push_str(&format!(
                    ", room = {{ name = {}, invite = {}, phase = {}, you_own = {}, max_players = {}, has_password = {}, members = {{",
                    lua_str(&room.name),
                    lua_str(&room.invite),
                    lua_str(&room.phase),
                    room.you_own,
                    room.max_players,
                    room.has_password
                ));
                for member in &room.members {
                    out.push_str(&format!(
                        " {{ id = {}, name = {}, ready = {}, owner = {}, you = {}, connected = {}, content = {} }},",
                        lua_str(&member.id),
                        lua_str(&member.name),
                        member.ready,
                        member.owner,
                        member.you,
                        member.connected,
                        lua_str(&member.content)
                    ));
                }
                out.push_str(" } }");
            }
        }
        out.push_str(" }");
        out
    }
}

/// What the window and the step driver share.
struct Menu {
    /// The launcher's lobby as last heard.
    view: Option<LobbyView>,
    /// Whether a step driver, and so the link to the launcher, is there.
    linked: bool,
    /// The player's actions, for the launcher, oldest first.
    actions: VecDeque<LobbyAction>,
    /// Why the last action was not taken, until the next is.
    refused: Option<String>,
}

static MENU: Mutex<Menu> = Mutex::new(Menu {
    view: None,
    linked: false,
    actions: VecDeque::new(),
    refused: None,
});

fn menu() -> MutexGuard<'static, Menu> {
    MENU.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Queues an action of the window's for the launcher.
pub fn queue(action: LobbyAction) -> Result<(), String> {
    let mut menu = menu();
    if menu.actions.len() >= MAX_QUEUED {
        return Err("the launcher has not taken the last actions yet".into());
    }
    menu.actions.push_back(action);
    menu.refused = None;
    Ok(())
}

/// The window's actions for the launcher, oldest first.
pub fn take_actions() -> Vec<LobbyAction> {
    menu().actions.drain(..).collect()
}

/// Hands the window's actions to the launcher through `driver`, and keeps
/// the lobby it sent back, if it sent a new one.
pub fn exchange(driver: &mut dyn StepHandler) {
    let actions = take_actions();
    let heard = driver.lobby(actions);
    let mut menu = menu();
    menu.linked = true;
    if let Some(view) = heard {
        menu.view = Some(view);
    }
}

/// This game has no link to its launcher: the window's actions go nowhere,
/// and it says so.
pub fn unlinked() {
    let mut menu = menu();
    menu.linked = false;
    if !menu.actions.is_empty() {
        menu.actions.clear();
        menu.refused = Some(
            "this game has no link to the TPF3-MP launcher; start it from the launcher".into(),
        );
    }
}

/// What the window shows now.
pub fn state() -> LobbyState {
    let menu = menu();
    let mut state = LobbyState::of(menu.view.as_ref(), menu.linked);
    if let Some(refused) = &menu.refused {
        state.error = Some(refused.clone());
    }
    state
}

#[cfg(test)]
pub(crate) fn reset() {
    *menu() = Menu {
        view: None,
        linked: false,
        actions: VecDeque::new(),
        refused: None,
    };
}

#[cfg(test)]
mod tests {
    use tpf3mp_bridge::{LobbyLine, LobbyMember, LobbyRoom};
    use tpf3mp_proto::BoundedVec;

    use super::*;

    #[test]
    fn actions_parse_from_the_windows_json() {
        assert_eq!(
            parse_action(r#"{"action":"connect","server":"play.example:4433","name":" Ada "}"#),
            Ok(LobbyAction::Connect {
                name: Text::new("Ada").unwrap()
            }),
            "the window's server is not taken (D12)"
        );
        assert_eq!(
            parse_action(r#"{"action":"create","room":"Alps"}"#),
            Ok(LobbyAction::Create {
                room: Text::new("Alps").unwrap(),
                max_players: 8,
                password: None,
            })
        );
        assert_eq!(
            parse_action(r#"{"action":"join","invite":"K7QM2X","password":"pw"}"#),
            Ok(LobbyAction::Join {
                invite: Text::new("K7QM2X").unwrap(),
                password: Some(Text::new("pw").unwrap()),
            })
        );
        let id = "ab".repeat(32);
        assert_eq!(
            parse_action(&format!(r#"{{"action":"kick","player":"{id}"}}"#)),
            Ok(LobbyAction::Kick {
                player: PlayerId(FixedBytes([0xab; 32]))
            })
        );
        assert_eq!(
            parse_action(r#"{"action":"ready","ready":true}"#),
            Ok(LobbyAction::Ready { ready: true })
        );
        assert_eq!(
            parse_action(r#"{"action":"leave"}"#),
            Ok(LobbyAction::Leave)
        );
        assert!(parse_action(r#"{"action":"fly"}"#).is_err());
        assert!(parse_action(r#"{"action":"kick","player":"7"}"#).is_err());
        let long = "x".repeat(300);
        assert!(parse_action(&format!(r#"{{"action":"chat","text":"{long}"}}"#)).is_err());
    }

    #[test]
    fn the_lua_literal_quotes_every_string() {
        let state = LobbyState {
            name: "A\"b\\c\nd é".into(),
            ..LobbyState::default()
        };
        let lua = state.to_lua();
        assert!(lua.contains(r#"name = "A\"b\\c\nd \195\169""#), "{lua}");
        assert!(lua.is_ascii());
        assert!(lua.starts_with("{ connection = \"disconnected\""));
        assert!(lua.contains("room = nil"));
        assert!(lua.ends_with(" }"));
    }

    fn view() -> LobbyView {
        LobbyView {
            connection: LobbyConnection::Connected,
            server: Text::new("EU").unwrap(),
            name: Text::new("Ann").unwrap(),
            error: None,
            notice: Some(Text::new("created the room").unwrap()),
            room: Some(LobbyRoom {
                name: Text::new("Alps").unwrap(),
                rules: Text::new("native").unwrap(),
                invite: Some(Text::new("K7QM2X").unwrap()),
                running: false,
                you_own: true,
                max_players: 4,
                has_password: false,
                members: BoundedVec::new(vec![LobbyMember {
                    player: PlayerId(FixedBytes([1; 32])),
                    name: Text::new("Ann").unwrap(),
                    ready: true,
                    connected: true,
                    owner: true,
                    you: true,
                    same_content: None,
                }])
                .unwrap(),
            }),
            chat: BoundedVec::new(vec![LobbyLine {
                from: Text::new("Bo").unwrap(),
                text: Text::new("hi").unwrap(),
                you: false,
            }])
            .unwrap(),
        }
    }

    #[test]
    fn the_window_shows_the_launchers_lobby_as_it_sent_it() {
        let state = LobbyState::of(Some(&view()), true);
        assert_eq!(state.connection, Connection::Connected);
        assert!(state.linked && state.heard);
        let room = state.room.as_ref().unwrap();
        assert_eq!(room.phase, "lobby");
        assert_eq!(room.invite, "K7QM2X");
        assert_eq!(room.members[0].id, "01".repeat(32));
        assert_eq!(room.members[0].content, "unknown");
        let lua = state.to_lua();
        assert!(lua.contains(r#"notice = "created the room""#), "{lua}");
        assert!(lua.contains(r#"{ from = "Bo", text = "hi", you = false }"#));
        // Nothing heard yet: not connected, and says whether it is linked.
        let quiet = LobbyState::of(None, true);
        assert!(quiet.linked && !quiet.heard && quiet.room.is_none());
    }

    /// The window evaluates the literal with a real Lua, as the menu's
    /// `load("return " .. reply)` does, and reads what it shows from it.
    #[test]
    fn the_windows_lua_reads_the_literal() {
        let mut shown = view();
        shown.name = Text::new("Ann \"the\" Bü\\").unwrap();
        let literal = LobbyState::of(Some(&shown), true).to_lua();
        let lua = mlua::Lua::new();
        let state: mlua::Table = lua.load(format!("return {literal}")).eval().unwrap();
        assert_eq!(state.get::<String>("connection").unwrap(), "connected");
        assert_eq!(state.get::<String>("name").unwrap(), "Ann \"the\" Bü\\");
        assert!(state.get::<bool>("linked").unwrap());
        let room: mlua::Table = state.get("room").unwrap();
        assert_eq!(room.get::<String>("invite").unwrap(), "K7QM2X");
        let members: mlua::Table = room.get("members").unwrap();
        let first: mlua::Table = members.get(1).unwrap();
        assert!(first.get::<bool>("you").unwrap());
        let chat: mlua::Table = state.get("chat").unwrap();
        let line: mlua::Table = chat.get(1).unwrap();
        assert_eq!(line.get::<String>("text").unwrap(), "hi");
    }

    /// A step driver that records what the window handed it and answers
    /// with a lobby.
    #[derive(Default)]
    struct Launcher {
        heard: Vec<LobbyAction>,
        answer: Option<LobbyView>,
    }

    impl StepHandler for Launcher {
        fn on_step(
            &mut self,
            _commands: Vec<(u64, tpf3mp_proto::Payload)>,
            _run: &mut crate::step::RunStep<'_>,
        ) -> crate::step::Outcome {
            unreachable!()
        }
        fn take_log(&mut self) -> Vec<String> {
            Vec::new()
        }
        fn take_refused(&mut self) -> Vec<(u64, String)> {
            Vec::new()
        }
        fn in_room(&self) -> bool {
            false
        }
        fn chosen_speed(&mut self, _speedup: u64) {}
        fn say(&mut self, _text: tpf3mp_proto::ChatText) {}
        fn cursor(&mut self, _cursor: tpf3mp_proto::Cursor) {}
        fn on_menu(&mut self) {}
        fn lobby(&mut self, actions: Vec<LobbyAction>) -> Option<LobbyView> {
            self.heard.extend(actions);
            self.answer.take()
        }
    }

    #[test]
    fn the_windows_actions_reach_the_launcher_and_its_lobby_comes_back() {
        let _serial = crate::lua::tests::SERIAL
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        reset();
        assert!(!state().linked);
        queue(LobbyAction::Start).unwrap();
        let mut launcher = Launcher {
            answer: Some(view()),
            ..Launcher::default()
        };
        exchange(&mut launcher);
        assert_eq!(launcher.heard, vec![LobbyAction::Start]);
        let shown = state();
        assert!(shown.linked && shown.heard);
        assert_eq!(shown.name, "Ann");
        // Nothing new: the window keeps what it had.
        exchange(&mut launcher);
        assert_eq!(state().name, "Ann");
        reset();
    }

    #[test]
    fn without_a_launcher_the_window_says_so_and_takes_no_action() {
        let _serial = crate::lua::tests::SERIAL
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        reset();
        queue(LobbyAction::Leave).unwrap();
        unlinked();
        assert!(take_actions().is_empty(), "dropped, never sent later");
        let shown = state();
        assert!(!shown.linked);
        assert!(
            shown
                .error
                .as_deref()
                .is_some_and(|e| e.contains("launcher")),
            "{shown:?}"
        );
        for _ in 0..MAX_QUEUED {
            queue(LobbyAction::Start).unwrap();
        }
        assert!(queue(LobbyAction::Start).is_err(), "bounded");
        reset();
    }
}
