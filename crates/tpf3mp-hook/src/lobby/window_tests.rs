//! The window itself: the mod's `gui/menu/lobby.lua`, drawn in every state
//! against a stand-in for the game's main menu (`tests/lua/fake_menu.lua`),
//! its buttons clicked, and every action it sends parsed as the hook parses
//! it ([`parse_action`]).

use mlua::{Function, Lua, Table};
use tpf3mp_bridge::{
    LobbyAction, LobbyConnection, LobbyLine, LobbyListing, LobbyMember, LobbyPublicRoom, LobbyRoom,
    LobbyRoomList, LobbyRules, LobbyView, LobbyWorld,
};
use tpf3mp_proto::{BoundedVec, FixedBytes, PlayerId, Text};

use super::{LobbyState, parse_action};

const FAKE_MENU: &str = include_str!("../../tests/lua/fake_menu.lua");
const WINDOW: &str = include_str!("../../../../mod/tpf3mp_1/content/gui/menu/lobby.lua");

fn menu() -> Lua {
    let lua = Lua::new();
    lua.globals().set("LOBBY_SOURCE", WINDOW).unwrap();
    lua.load(FAKE_MENU)
        .set_name("@fake_menu.lua")
        .exec()
        .unwrap();
    lua
}

fn show(lua: &Lua, view: Option<&LobbyView>) {
    let literal = LobbyState::of(view, true).to_lua();
    lua.globals().set("STATE", literal).unwrap();
}

fn call(lua: &Lua, name: &str, args: impl mlua::IntoLuaMulti) {
    lua.globals()
        .get::<Function>(name)
        .unwrap()
        .call::<()>(args)
        .unwrap_or_else(|error| panic!("{name}: {error}"));
}

/// Draws the window as the game does: once, then a poll of the hook and a
/// redraw.
fn open(lua: &Lua, focus: Option<&str>) {
    call(lua, "render", focus);
    call(lua, "tick", ());
}

fn texts(lua: &Lua) -> String {
    lua.globals()
        .get::<Function>("texts")
        .unwrap()
        .call(())
        .unwrap()
}

fn click(lua: &Lua, label: &str) {
    call(lua, "click", label);
}

fn enabled(lua: &Lua, label: &str) -> bool {
    lua.globals()
        .get::<Function>("enabled")
        .unwrap()
        .call(label)
        .unwrap()
}

fn has_button(lua: &Lua, label: &str) -> bool {
    lua.globals()
        .get::<Function>("find")
        .unwrap()
        .call::<Option<Table>>(label)
        .unwrap()
        .is_some()
}

/// What the window sent, as the hook parses it, but the room list it asks
/// for by itself; and forgets it.
fn sent(lua: &Lua) -> Vec<LobbyAction> {
    sent_all(lua)
        .into_iter()
        .filter(|action| !matches!(action, LobbyAction::ListRooms { .. }))
        .collect()
}

/// Everything the window sent, as the hook parses it; and forgets it.
fn sent_all(lua: &Lua) -> Vec<LobbyAction> {
    let list: Table = lua.globals().get("SENT").unwrap();
    let actions = list
        .sequence_values::<String>()
        .map(|json| {
            let json = json.unwrap();
            parse_action(&json).unwrap_or_else(|error| panic!("{json}: {error}"))
        })
        .collect();
    lua.globals()
        .set("SENT", lua.create_table().unwrap())
        .unwrap();
    actions
}

fn player(n: u8) -> PlayerId {
    PlayerId(FixedBytes([n; 32]))
}

fn member(n: u8, name: &str, owner: bool, you: bool, ready: bool) -> LobbyMember {
    LobbyMember {
        player: player(n),
        name: Text::new(name).unwrap(),
        ready,
        connected: true,
        owner,
        you,
        same_content: Some(true),
        banner: None,
    }
}

fn online() -> LobbyView {
    LobbyView {
        connection: LobbyConnection::Connected,
        server: Text::new("EU").unwrap(),
        name: Text::new("Ann").unwrap(),
        rules: BoundedVec::new(vec![
            LobbyRules {
                name: Text::new("native").unwrap(),
                description: Text::new("The game's own economy").unwrap(),
            },
            LobbyRules {
                name: Text::new("canonical").unwrap(),
                description: Text::new("The server settles the economy").unwrap(),
            },
        ])
        .unwrap(),
        saves: BoundedVec::new(vec![
            Text::new("newest").unwrap(),
            Text::new("mptest").unwrap(),
        ])
        .unwrap(),
        start_save: Some(Text::new("mptest").unwrap()),
        ..LobbyView::default()
    }
}

fn in_room(members: Vec<LobbyMember>, you_own: bool) -> LobbyView {
    LobbyView {
        room: Some(LobbyRoom {
            name: Text::new("Friday trains").unwrap(),
            rules: Text::new("native").unwrap(),
            invite: Some(Text::new("K7QM2X").unwrap()),
            running: false,
            you_own,
            max_players: 4,
            has_password: true,
            members: BoundedVec::new(members).unwrap(),
            competitive: false,
        }),
        chat: BoundedVec::new(vec![LobbyLine {
            from: Text::new("Bob").unwrap(),
            text: Text::new("I'll take the coal line").unwrap(),
            you: false,
        }])
        .unwrap(),
        ..online()
    }
}

#[test]
fn before_the_hook_answers_the_window_waits_and_can_be_closed() {
    let lua = menu();
    open(&lua, None);
    let shown = texts(&lua);
    assert!(shown.contains("Waiting for the hook"), "{shown}");
    assert!(shown.contains("The hook did not answer"), "{shown}");
    click(&lua, "Close");
    assert_eq!(lua.globals().get::<u32>("CLOSED").unwrap(), 1);
}

#[test]
fn not_connected_it_connects_with_the_name_typed_to_the_launchers_server() {
    let lua = menu();
    show(&lua, Some(&LobbyView::default()));
    // The launcher's server, named as players see it (D12).
    let mut view = LobbyView {
        server: Text::new("EU").unwrap(),
        name: Text::new("Ann").unwrap(),
        ..LobbyView::default()
    };
    show(&lua, Some(&view));
    open(&lua, None);
    let shown = texts(&lua);
    assert!(shown.contains("Not connected"), "{shown}");
    assert!(
        shown.contains("Join a room") && shown.contains("Host a room"),
        "the first page's two choices: {shown}"
    );
    assert!(!card_enabled(&lua, "Join a room"), "only once connected");
    call(&lua, "type_into", ("Ann", "Ada"));
    click(&lua, "Connect to EU");
    assert_eq!(
        sent(&lua),
        [LobbyAction::Connect {
            name: Text::new("Ada").unwrap()
        }]
    );
    // Under way until the launcher answers, and not sent twice.
    assert!(texts(&lua).contains("Connecting to EU..."));
    assert!(!enabled(&lua, "Connect to EU"));
    view.connection = LobbyConnection::Connecting;
    show(&lua, Some(&view));
    call(&lua, "tick", ());
    assert!(texts(&lua).contains("Connecting"));
    assert!(!enabled(&lua, "Connecting..."));
}

#[test]
fn a_game_without_its_launcher_says_so_and_offers_nothing() {
    let lua = menu();
    let literal = LobbyState::of(None, false).to_lua();
    lua.globals().set("STATE", literal).unwrap();
    open(&lua, None);
    let shown = texts(&lua);
    assert!(shown.contains("no link to the TPF3-MP launcher"), "{shown}");
    assert!(!enabled(&lua, "Connect to the TPF3-MP server"));
}

#[test]
fn a_room_is_created_with_the_rules_players_and_save_picked() {
    let lua = menu();
    show(&lua, Some(&online()));
    open(&lua, None);
    call(&lua, "click_card", "Host a room");
    assert!(texts(&lua).contains("Private: invite only"));
    // The saves, the launcher's own first choice picked, and a way to load
    // a world by hand.
    let (values, chosen): (Vec<String>, String) = lua
        .globals()
        .get::<Function>("offered")
        .unwrap()
        .call("Start from this save")
        .unwrap();
    assert_eq!(values, ["newest", "mptest", ""]);
    assert_eq!(chosen, "mptest");
    call(&lua, "type_into", ("Ann's room", "Alps"));
    call(&lua, "choose", ("Players", 6));
    call(&lua, "choose", ("Rules", "canonical"));
    assert!(texts(&lua).contains("The server settles the economy"));
    click(&lua, "Create room");
    assert_eq!(
        sent(&lua),
        [LobbyAction::Create {
            room: Text::new("Alps").unwrap(),
            max_players: 6,
            password: None,
            rules: Some(Text::new("canonical").unwrap()),
            start_save: Some(Text::new("mptest").unwrap()),
            listing: None,
            competitive: false,
        }]
    );
    assert!(texts(&lua).contains("Creating the room..."));
}

#[test]
fn a_room_can_start_without_a_save_and_is_named_for_its_owner() {
    let lua = menu();
    show(
        &lua,
        Some(&LobbyView {
            start_save: None,
            ..online()
        }),
    );
    open(&lua, None);
    call(&lua, "click_card", "Host a room");
    let (_, chosen): (Vec<String>, String) = lua
        .globals()
        .get::<Function>("offered")
        .unwrap()
        .call("Start from this save")
        .unwrap();
    assert_eq!(chosen, "newest", "without the launcher's own, the newest");
    call(&lua, "choose", ("Start from this save", ""));
    assert!(texts(&lua).contains("Load a world in the game once in the room"));
    click(&lua, "Create room");
    let actions: [LobbyAction; 1] = sent(&lua).try_into().unwrap();
    let [
        LobbyAction::Create {
            room, start_save, ..
        },
    ] = actions
    else {
        panic!("not a create")
    };
    assert_eq!(room.as_str(), "Ann's room");
    assert_eq!(start_save.as_ref().map(Text::as_str), Some(""), "none");
}

#[test]
fn a_room_is_joined_with_a_code_from_its_popup() {
    let lua = menu();
    show(&lua, Some(&online()));
    open(&lua, Some("join"));
    let shown = texts(&lua);
    assert!(
        shown.contains("Public rooms on EU") && !shown.contains("Invite code"),
        "the Join page shows only the public rooms: {shown}"
    );
    click(&lua, "Join with code");
    let shown = texts(&lua);
    assert!(
        shown.contains("Invite code") && !shown.contains("Public rooms on EU"),
        "the popup over the list: {shown}"
    );
    // Cancel closes it.
    click(&lua, "Cancel");
    assert!(texts(&lua).contains("Public rooms on EU"));
    click(&lua, "Join with code");
    // Nothing typed: said, and nothing sent.
    click(&lua, "Join");
    assert!(sent(&lua).is_empty());
    assert!(texts(&lua).contains("Type the invite code"));
    click(&lua, "Join with code");
    call(&lua, "type_into", ("K7QM2X", " k7qm2x "));
    // The field without a placeholder: the room's password.
    call(&lua, "type_into", ("", "pw"));
    click(&lua, "Join");
    assert_eq!(
        sent(&lua),
        [LobbyAction::Join {
            invite: Text::new("K7QM2X").unwrap(),
            password: Some(Text::new("pw").unwrap()),
        }]
    );
}

#[test]
fn what_the_hook_refuses_is_shown_until_the_next_action() {
    let lua = menu();
    show(&lua, Some(&online()));
    open(&lua, None);
    call(&lua, "click_card", "Host a room");
    lua.globals()
        .set("REPLY", "error: that room name is too long")
        .unwrap();
    click(&lua, "Create room");
    let shown = texts(&lua);
    assert!(shown.contains("that room name is too long"), "{shown}");
    assert!(!shown.contains("Creating the room..."));
    // And the launcher's own errors, as it sends them.
    lua.globals().set("REPLY", "ok").unwrap();
    show(
        &lua,
        Some(&LobbyView {
            error: Some(Text::new("no room has that invite").unwrap()),
            ..online()
        }),
    );
    click(&lua, "Back");
    click(&lua, "Disconnect");
    call(&lua, "tick", ());
    assert!(texts(&lua).contains("no room has that invite"));
}

#[test]
fn in_the_room_the_owner_starts_once_everyone_is_ready() {
    let lua = menu();
    show(
        &lua,
        Some(&in_room(
            vec![
                member(1, "Ann", true, true, true),
                member(2, "Bob", false, false, false),
            ],
            true,
        )),
    );
    open(&lua, None);
    let shown = texts(&lua);
    for word in [
        "Friday trains",
        "K7QM2X",
        "2 of 4 players  ·  1 ready",
        "Owner",
        "You",
        "Not ready",
        "I'll take the coal line",
    ] {
        assert!(shown.contains(word), "{word}: {shown}");
    }
    assert!(!enabled(&lua, "Start the game"), "Bob is not ready");
    click(&lua, "Not ready");
    assert_eq!(sent(&lua), [LobbyAction::Ready { ready: false }]);
    show(
        &lua,
        Some(&in_room(
            vec![
                member(1, "Ann", true, true, true),
                member(2, "Bob", false, false, true),
            ],
            true,
        )),
    );
    call(&lua, "tick", ());
    click(&lua, "Start the game");
    assert_eq!(sent(&lua), [LobbyAction::Start]);
}

#[test]
fn removing_a_player_and_leaving_ask_first() {
    let lua = menu();
    show(
        &lua,
        Some(&in_room(
            vec![
                member(1, "Ann", true, true, true),
                member(2, "Bob", false, false, true),
            ],
            true,
        )),
    );
    open(&lua, None);
    click(&lua, "Remove Bob from the room");
    assert!(sent(&lua).is_empty(), "asked first");
    click(&lua, "Keep");
    click(&lua, "Remove Bob from the room");
    click(&lua, "Remove");
    assert_eq!(sent(&lua), [LobbyAction::Kick { player: player(2) }]);
    click(&lua, "Leave room");
    assert!(sent(&lua).is_empty(), "asked first");
    assert!(texts(&lua).contains("Leave the room?"));
    click(&lua, "Stay");
    click(&lua, "Leave room");
    click(&lua, "Leave");
    assert_eq!(sent(&lua), [LobbyAction::Leave]);
}

#[test]
fn a_guest_gets_ready_and_chats_but_neither_starts_nor_removes() {
    let lua = menu();
    show(
        &lua,
        Some(&in_room(
            vec![
                member(1, "Ann", true, false, true),
                member(2, "Bob", false, true, false),
            ],
            false,
        )),
    );
    open(&lua, None);
    assert!(!has_button(&lua, "Start the game"));
    assert!(!has_button(&lua, "Remove Ann from the room"));
    click(&lua, "Ready");
    assert_eq!(sent(&lua), [LobbyAction::Ready { ready: true }]);
    call(&lua, "type_into", ("Say something to the room", "hi all"));
    assert_eq!(
        sent(&lua),
        [LobbyAction::Chat {
            text: Text::new("hi all").unwrap()
        }]
    );
}

#[test]
fn while_the_rooms_world_comes_the_window_says_how_far_and_stays_usable() {
    let lua = menu();
    let mut view = in_room(
        vec![
            member(1, "Ann", true, false, true),
            member(2, "Bob", false, true, true),
        ],
        false,
    );
    view.room.as_mut().unwrap().running = true;
    view.world = LobbyWorld::Fetching {
        bytes: 5_000_000,
        total: 20_000_000,
    };
    view.differences = Some(Text::new("you lack stations 3").unwrap());
    show(&lua, Some(&view));
    open(&lua, None);
    let shown = texts(&lua);
    assert!(
        shown.contains("Receiving the room's world: 25% (5.0 MB of 20.0 MB)"),
        "{shown}"
    );
    assert!(shown.contains("you lack stations 3"));
    assert!(shown.contains("The room's game is under way."));
    assert!(!has_button(&lua, "Ready") && !has_button(&lua, "Start the game"));
    // The chat and Leave still work.
    call(
        &lua,
        "type_into",
        ("Say something to the room", "almost there"),
    );
    assert_eq!(sent(&lua).len(), 1);
    assert!(enabled(&lua, "Leave room"));
    view.world = LobbyWorld::Loading;
    show(&lua, Some(&view));
    call(&lua, "tick", ());
    assert!(texts(&lua).contains("Loading the room's world..."));
}

#[test]
fn the_cards_say_where_the_player_is() {
    let lua = menu();
    let line = |view: Option<&LobbyView>, linked: bool, what: &str| -> String {
        let literal = LobbyState::of(view, linked).to_lua();
        let state: Table = lua.load(format!("return {literal}")).eval().unwrap();
        let lobby: Table = lua.globals().get("LOBBY").unwrap();
        lobby.get::<Function>(what).unwrap().call(state).unwrap()
    };
    assert_eq!(
        line(None, false, "summary"),
        "Start the game from the TPF3-MP launcher"
    );
    assert_eq!(line(Some(&online()), true, "summary"), "Online on EU");
    let room = in_room(vec![member(1, "Ann", true, true, true)], true);
    assert_eq!(
        line(Some(&room), true, "summary"),
        "Friday trains · 1/4 players · 1 ready"
    );
    assert_eq!(
        line(Some(&room), true, "joinLine"),
        "Your room: invite K7QM2X"
    );
    assert_eq!(
        line(Some(&online()), true, "joinLine"),
        "With the invite code they send you"
    );
}

fn public_room(name: &str, map: &str, players: u8, password: bool) -> LobbyPublicRoom {
    LobbyPublicRoom {
        invite: Text::new(format!("INV{}", name.len())).unwrap(),
        name: Text::new(name).unwrap(),
        rules: Text::new("native").unwrap(),
        players,
        max_players: 4,
        has_password: password,
        running: false,
        map: Text::new(map).unwrap(),
        year: 1873,
        companies: 2,
        competitive: false,
    }
}

fn browsing(rooms: Vec<LobbyPublicRoom>, page: u16, more: bool) -> LobbyView {
    LobbyView {
        rooms: Some(LobbyRoomList {
            page,
            rooms: BoundedVec::new(rooms).unwrap(),
            more,
        }),
        ..online()
    }
}

fn card_enabled(lua: &Lua, title: &str) -> bool {
    all_cards(lua).iter().any(|card| {
        card.get::<String>("text").unwrap().contains(title) && card.get::<bool>("enabled").unwrap()
    })
}

/// The room cards shown: the first page's Join and Host are cards too, and
/// are left out.
fn cards(lua: &Lua) -> Vec<Table> {
    all_cards(lua)
        .into_iter()
        .filter(|card| {
            let text: String = card.get("text").unwrap();
            !text.starts_with("Join a room") && !text.starts_with("Host a room")
        })
        .collect()
}

fn all_cards(lua: &Lua) -> Vec<Table> {
    let list: Table = lua
        .globals()
        .get::<Function>("room_cards")
        .unwrap()
        .call(())
        .unwrap();
    list.sequence_values::<Table>()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn the_window_asks_for_the_room_list_and_shows_each_room_as_a_card() {
    let lua = menu();
    show(&lua, Some(&online()));
    open(&lua, None);
    assert!(sent_all(&lua).is_empty(), "the first page asks for no list");
    call(&lua, "click_card", "Join a room");
    call(&lua, "tick", ());
    assert_eq!(
        sent_all(&lua),
        [LobbyAction::ListRooms { page: 0 }],
        "asked for at once"
    );
    assert!(texts(&lua).contains("Asking the server for its rooms"));
    show(
        &lua,
        Some(&browsing(
            vec![
                public_room("Dry run", "dry", 2, false),
                public_room("Snowy", "subarctic", 1, true),
                public_room("Somewhere", "", 3, false),
                public_room("Fourth", "temperate", 1, false),
            ],
            0,
            true,
        )),
    );
    call(&lua, "tick", ());
    let shown = cards(&lua);
    assert_eq!(shown.len(), 4);
    let first = &shown[0];
    let text: String = first.get("text").unwrap();
    assert!(text.contains("Dry run"), "{text}");
    assert!(text.contains("2/4 players · 2 companies · 1873"), "{text}");
    assert!(text.contains("Dry"), "the climate's own name: {text}");
    assert_eq!(
        first.get::<String>("picture").unwrap(),
        "::/climates/dry/icon.tga",
        "the climate's own picture"
    );
    assert_eq!(
        shown[1].get::<String>("picture").unwrap(),
        "::/gui/menu/images/subarctic_ingame.tga",
        "the menu's picture of it"
    );
    assert_eq!(
        shown[2].get::<String>("picture").unwrap(),
        "::/gui/menu/images/m05_ingame.tga",
        "a map it does not know"
    );
    // A room without a password joins with a click.
    first
        .get::<Function>("click")
        .unwrap()
        .call::<()>(())
        .unwrap();
    call(&lua, "render", ());
    assert_eq!(
        sent(&lua),
        [LobbyAction::Join {
            invite: Text::new("INV7").unwrap(),
            password: None,
        }]
    );
}

#[test]
fn a_room_with_a_password_asks_for_it_and_pages_move_on() {
    let lua = menu();
    show(
        &lua,
        Some(&browsing(
            vec![public_room("Snowy", "subarctic", 1, true)],
            1,
            false,
        )),
    );
    open(&lua, Some("join"));
    sent_all(&lua);
    assert!(enabled(&lua, "Previous") && !enabled(&lua, "Next"));
    click(&lua, "Previous");
    assert_eq!(sent_all(&lua), [LobbyAction::ListRooms { page: 0 }]);
    cards(&lua)[0]
        .get::<Function>("click")
        .unwrap()
        .call::<()>(())
        .unwrap();
    call(&lua, "render", ());
    assert!(sent(&lua).is_empty(), "the password first");
    assert!(texts(&lua).contains("Snowy has a password"));
    call(&lua, "type_into", ("Password", "pw"));
    assert_eq!(
        sent(&lua),
        [LobbyAction::Join {
            invite: Text::new("INV5").unwrap(),
            password: Some(Text::new("pw").unwrap()),
        }]
    );
}

#[test]
fn a_public_room_is_listed_with_its_saves_climate_and_year() {
    let lua = menu();
    let saves: Table = lua.globals().get("SAVES").unwrap();
    let save = lua.create_table().unwrap();
    save.set("climate", "::/climates/dry/dry.clima").unwrap();
    save.set("year", 1900).unwrap();
    saves.set("mptest", save).unwrap();
    show(&lua, Some(&online()));
    open(&lua, None);
    call(&lua, "click_card", "Host a room");
    call(&lua, "choose", ("Who can find it", "public"));
    assert!(
        texts(&lua).contains("Listed for everyone on EU: Dry, 1900."),
        "{}",
        texts(&lua)
    );
    click(&lua, "Create room");
    let actions: [LobbyAction; 1] = sent(&lua).try_into().unwrap();
    let [LobbyAction::Create { listing, .. }] = actions else {
        panic!("not a create")
    };
    assert_eq!(
        listing,
        Some(LobbyListing {
            map: Text::new("dry").unwrap(),
            year: 1900,
        })
    );
    assert_eq!(lua.globals().get::<u32>("READS").unwrap(), 1, "read once");
}

/// The window itself is a wrapper recipe in the mod's `main_page.tl`
/// (Teal, which the stand-in cannot run): its widget's meta may hold its
/// class only. The game asserted and closed on a styleSheet there
/// ("Wrapper recipe must return child", 2026-09-30).
#[test]
fn the_menus_wrapper_recipes_pass_meta_for_their_class_only() {
    let page = include_str!("../../../../mod/tpf3mp_1/content/gui/menu/main_page.tl");
    let mut checked = 0;
    for block in page.split("RegisterWrapperRecipe(").skip(1) {
        let block = &block[..block.find("\nend)").expect("the recipe ends")];
        for meta in block.split("meta = {").skip(1) {
            let inside = &meta[..meta.find('}').expect("the meta table closes")];
            let keys: Vec<&str> = inside
                .split(',')
                .filter_map(|entry| entry.split_once('=').map(|(key, _)| key.trim()))
                .collect();
            assert!(
                keys.iter().all(|key| *key == "class"),
                "a wrapper recipe's meta holds {keys:?}"
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "the Multiplayer window's meta was checked");
}

#[test]
fn the_first_page_leads_to_join_or_host_and_back() {
    let lua = menu();
    show(&lua, Some(&online()));
    open(&lua, None);
    let shown = texts(&lua);
    assert!(shown.contains("Online on EU"), "{shown}");
    assert!(!shown.contains("Room name") && !shown.contains("Public rooms on EU"));
    call(&lua, "click_card", "Host a room");
    assert!(texts(&lua).contains("Room name"));
    click(&lua, "Back");
    assert!(!texts(&lua).contains("Room name"));
    call(&lua, "click_card", "Join a room");
    assert!(texts(&lua).contains("Public rooms on EU"));
    click(&lua, "Back");
    assert!(texts(&lua).contains("Host a room"));
    // In a room, the room's page, whatever was picked.
    show(
        &lua,
        Some(&in_room(vec![member(1, "Ann", true, true, false)], true)),
    );
    call(&lua, "tick", ());
    assert!(texts(&lua).contains("Your room"));
}

#[test]
fn your_mods_are_chosen_from_join_host_and_the_room() {
    let lua = menu();
    let mods = |view: LobbyView| LobbyView {
        mods: BoundedVec::new(vec![
            tpf3mp_bridge::LobbyMod {
                id: Text::new("schbrongx_minimap").unwrap(),
                name: Text::new("Minimap").unwrap(),
                class: tpf3mp_bridge::LobbyModClass::Personal,
                reason: Text::new("only what this player sees").unwrap(),
                chosen: false,
                choosable: true,
            },
            tpf3mp_bridge::LobbyMod {
                id: Text::new("vehicles_pack").unwrap(),
                name: Text::new("Vehicles").unwrap(),
                class: tpf3mp_bridge::LobbyModClass::Shared,
                reason: Text::new("every player needs it: it adds vehicles").unwrap(),
                chosen: true,
                choosable: false,
            },
        ])
        .unwrap(),
        room_mods: BoundedVec::new(vec![tpf3mp_bridge::LobbyRoomMod {
            id: Text::new("trees_pack").unwrap(),
            version: Text::new("2").unwrap(),
            have: tpf3mp_bridge::LobbyHave::No,
        }])
        .unwrap(),
        room_mods_more: 3,
        ..view
    };
    show(&lua, Some(&mods(online())));
    open(&lua, Some("join"));
    click(&lua, "Your mods (0 chosen)");
    let shown = texts(&lua);
    for word in [
        "Minimap",
        "only you see it",
        "Vehicles",
        "every player needs it",
        "trees_pack",
        "You lack it",
        "and 3 more",
    ] {
        assert!(shown.contains(word), "{word}: {shown}");
    }
    assert!(
        !enabled(&lua, "Needed"),
        "a shared mod cannot be turned off"
    );
    click(&lua, "Off");
    assert_eq!(
        sent(&lua),
        [LobbyAction::ChooseMod {
            id: Text::new("schbrongx_minimap").unwrap(),
            chosen: true,
        }]
    );
    click(&lua, "Back");
    assert!(texts(&lua).contains("Public rooms on EU"));
    // In the room's lobby too, but not once its game runs.
    let mut room = mods(in_room(vec![member(1, "Ann", true, true, true)], true));
    show(&lua, Some(&room));
    call(&lua, "tick", ());
    click(&lua, "Your mods (0 chosen)");
    assert!(enabled(&lua, "Off"));
    room.room.as_mut().unwrap().running = true;
    show(&lua, Some(&room));
    call(&lua, "tick", ());
    assert!(!enabled(&lua, "Off"));
}

#[test]
fn the_server_is_shown_changed_and_put_back_from_the_first_page() {
    let lua = menu();
    let on = |address: &str| LobbyView {
        server_address: Text::new(address).unwrap(),
        server_default: Text::new("relay.example:29470").unwrap(),
        ..online()
    };
    show(&lua, Some(&on("relay.example:29470")));
    open(&lua, None);
    click(&lua, "Server...");
    let shown = texts(&lua);
    assert!(shown.contains("Now: EU (default)"), "{shown}");
    assert!(shown.contains("Invites only join rooms on your own server."));
    assert!(
        !has_button(&lua, "Reset to default"),
        "already on the default"
    );
    // The launcher refuses an address it cannot use: said under the field.
    lua.globals()
        .set(
            "REPLY",
            "error: the server must be host:port, such as play.example:29470",
        )
        .unwrap();
    call(&lua, "type_into", ("relay.example:29470", "nonsense"));
    assert!(texts(&lua).contains("the server must be host:port"));
    lua.globals().set("REPLY", "ok").unwrap();
    call(
        &lua,
        "type_into",
        ("relay.example:29470", "lan.example:29470"),
    );
    assert!(
        !enabled(&lua, "Use this server"),
        "not again while it changes"
    );
    let asked = sent(&lua);
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert_eq!(
        asked[1],
        LobbyAction::SetServer {
            server: Text::new("lan.example:29470").unwrap()
        }
    );
    // On another server: Reset puts the default back.
    show(&lua, Some(&on("lan.example:29470")));
    call(&lua, "tick", ());
    let shown = texts(&lua);
    assert!(
        shown.contains("Now: another server") && !shown.contains("lan.example"),
        "the address only in the field: {shown}"
    );
    click(&lua, "Reset to default");
    assert_eq!(
        sent(&lua),
        [LobbyAction::SetServer {
            server: Text::new("").unwrap()
        }]
    );
    // Not from a room: the room's page is shown, and no server page.
    show(
        &lua,
        Some(&LobbyView {
            ..in_room(vec![member(1, "Ann", true, true, true)], true)
        }),
    );
    call(&lua, "tick", ());
    assert!(!has_button(&lua, "Use this server"));
}

#[test]
fn no_server_address_shows_in_the_window() {
    let lua = menu();
    let mut view = LobbyView {
        server: Text::new("127.0.0.1:29470").unwrap(),
        error: Some(Text::new("cannot reach 127.0.0.1:29470: timed out").unwrap()),
        ..online()
    };
    show(&lua, Some(&view));
    open(&lua, None);
    let shown = texts(&lua);
    assert!(!shown.contains("127.0.0.1"), "{shown}");
    assert!(shown.contains("another server"), "{shown}");
    assert!(
        shown.contains("cannot reach the server: timed out"),
        "{shown}"
    );
    // An invite with the server before its code shows the code alone.
    view = in_room(vec![member(1, "Ann", true, true, true)], true);
    view.room.as_mut().unwrap().invite = Some(Text::new("play.example:29470 K7QM2X").unwrap());
    show(&lua, Some(&view));
    call(&lua, "tick", ());
    let shown = texts(&lua);
    assert!(
        shown.contains("K7QM2X") && !shown.contains("play.example"),
        "{shown}"
    );
}

#[test]
fn the_windows_banners_are_the_servers_in_its_order() {
    let lua = menu();
    let lobby: Table = lua.globals().get("LOBBY").unwrap();
    let banners: Table = lobby.get("BANNERS").unwrap();
    let ids: Vec<String> = banners
        .sequence_values::<Table>()
        .map(|banner| banner.unwrap().get::<String>(1).unwrap())
        .collect();
    assert_eq!(
        ids,
        tpf3mp_proto::BANNERS,
        "the default is the same in every game"
    );
}

#[test]
fn the_players_show_as_cards_of_their_banners_or_their_default() {
    let lua = menu();
    let mut ann = member(1, "Ann", true, true, true);
    ann.banner = Some(Text::new("dry").unwrap());
    let bob = member(0x2a, "Bob", false, false, false);
    show(&lua, Some(&in_room(vec![ann, bob], true)));
    open(&lua, None);
    let cards = all_cards(&lua);
    let find = |name: &str| {
        cards
            .iter()
            .find(|card| card.get::<String>("text").unwrap().starts_with(name))
            .unwrap_or_else(|| panic!("no card for {name}"))
            .clone()
    };
    let ann_card = find("Ann");
    assert_eq!(
        ann_card.get::<String>("picture").unwrap(),
        "::/gui/menu/images/dry_ingame.tga",
        "her pick"
    );
    let text: String = ann_card.get("text").unwrap();
    assert!(
        text.contains("Owner") && text.contains("Ready") && text.contains("You"),
        "{text}"
    );
    // Bob's default, from his key: 0x2a2a2a2a modulo the set.
    let n = 0x2a2a_2a2a_usize % tpf3mp_proto::BANNERS.len();
    let bob_card = find("Bob");
    let expected: String = lua
        .load(format!(
            "return LOBBY.bannerPicture(\"{}\")",
            tpf3mp_proto::BANNERS[n]
        ))
        .eval()
        .unwrap();
    assert_eq!(bob_card.get::<String>("picture").unwrap(), expected);
    assert!(
        bob_card
            .get::<String>("text")
            .unwrap()
            .contains("Not ready")
    );
}

#[test]
fn a_banner_is_picked_from_the_first_page() {
    let lua = menu();
    show(&lua, Some(&online()));
    open(&lua, None);
    click(&lua, "Your banner");
    let cards = all_cards(&lua);
    assert_eq!(cards.len(), tpf3mp_proto::BANNERS.len());
    assert!(!enabled(&lua, "Default"), "already the default");
    cards[3]
        .get::<Function>("click")
        .unwrap()
        .call::<()>(())
        .unwrap();
    assert_eq!(
        sent(&lua),
        [LobbyAction::SetBanner {
            banner: Some(Text::new(tpf3mp_proto::BANNERS[3]).unwrap())
        }]
    );
    show(
        &lua,
        Some(&LobbyView {
            banner: Some(Text::new(tpf3mp_proto::BANNERS[3]).unwrap()),
            ..online()
        }),
    );
    call(&lua, "tick", ());
    assert!(texts(&lua).contains("Yours"));
    click(&lua, "Default");
    assert_eq!(sent(&lua), [LobbyAction::SetBanner { banner: None }]);
}

#[test]
fn the_host_picks_co_op_or_competitive_from_two_pictures() {
    let lua = menu();
    show(&lua, Some(&online()));
    open(&lua, None);
    call(&lua, "click_card", "Host a room");
    let styles: Vec<(String, String)> = all_cards(&lua)
        .iter()
        .map(|card| {
            (
                card.get::<String>("text").unwrap(),
                card.get::<String>("picture").unwrap(),
            )
        })
        .collect();
    assert!(
        styles
            .iter()
            .any(|(text, picture)| text.starts_with("> Co-op")
                && picture == "::/gui/menu/images/campaign.tga"),
        "co-op is picked first: {styles:?}"
    );
    assert!(
        styles
            .iter()
            .any(|(text, picture)| text.starts_with("Competitive")
                && picture.ends_with("m03_loadscreen.tga"))
    );
    call(&lua, "click_card", "Competitive");
    assert!(texts(&lua).contains("Each player founds a company of their own"));
    click(&lua, "Create room");
    let actions: [LobbyAction; 1] = sent(&lua).try_into().unwrap();
    let [LobbyAction::Create { competitive, .. }] = actions else {
        panic!("not a create")
    };
    assert!(competitive);
}

#[test]
fn a_rooms_play_style_shows_in_the_room_and_the_list() {
    let lua = menu();
    let mut view = in_room(vec![member(1, "Ann", true, true, true)], true);
    view.room.as_mut().unwrap().competitive = true;
    show(&lua, Some(&view));
    open(&lua, None);
    assert!(texts(&lua).contains("Competitive"));
    let mut listed = public_room("Race", "dry", 2, false);
    listed.competitive = true;
    show(&lua, Some(&browsing(vec![listed], 0, false)));
    call(&lua, "tick", ());
    call(&lua, "render", "join");
    let text: String = cards(&lua)[0].get("text").unwrap();
    assert!(text.contains("Competitive"), "{text}");
}
