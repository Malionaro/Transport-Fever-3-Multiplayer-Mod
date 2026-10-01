//! The Lua mod (`mod/tpf3mp_1`) as Transport Fever 3 loads it: laid out as
//! mods made for build 40391 are (`mod.json`, `_content.json`,
//! `_metadata/modinfo.json`, `content/`), and its entry script run in a
//! stand-in for the game's GUI state (`tests/lua/fake_gui.lua`), with and
//! without the hook. What the layout rests on is in
//! `investigation/TF3_MODS_2026-09-27.md`.

#![allow(clippy::unwrap_used)]

mod common;

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use mlua::{Function, Lua, Table};

const MOD_ID: &str = "tpf3mp_1";
const FAKE_GUI: &str = include_str!("lua/fake_gui.lua");

fn mod_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../mod")
        .join(MOD_ID)
}

fn json(path: &str) -> serde_json::Value {
    let text = std::fs::read_to_string(mod_dir().join(path)).unwrap();
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{path}: {error}"))
}

/// Every file under `content/`, by its path relative to it.
fn content_files() -> BTreeSet<String> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_string_lossy();
                out.insert(rel.replace('\\', "/"));
            }
        }
    }
    let root = mod_dir().join("content");
    let mut out = BTreeSet::new();
    walk(&root, &root, &mut out);
    out
}

#[test]
fn the_mod_is_laid_out_as_tf3_mods_are() {
    assert!(
        !mod_dir().join("mod.lua").exists(),
        "a TPF2 mod.lua is left over"
    );

    let manifest = json("mod.json");
    assert_eq!(manifest["modId"], MOD_ID, "modId is the folder's name");
    assert_eq!(manifest["severityAdd"], "None");
    assert_eq!(manifest["severityRemove"], "None");
    // Build 40408's ModRep::couldAchievementsBeEarned is true when any mod
    // of the save has this flag (framework/mod/modrep.cpp, 0x2f929f0), so
    // a game with TPF3-MP active still earns achievements.
    assert_eq!(manifest["forceActivateAchievements"], true);
    assert!(manifest["revision"].is_u64());
    for script in ["preRunScript", "runScript", "postRunScript"] {
        let file = manifest[script]["fileName"].as_str().unwrap();
        assert!(
            file.is_empty() || file.starts_with(&format!("{MOD_ID}::/")),
            "{script} names another mod's file: {file}"
        );
    }

    let info = json("_metadata/modinfo.json");
    assert_eq!(info["name"], "TPF3-MP");
    assert!(info["summary"].is_string() && info["description"].is_string());
    assert!(
        info["tags"]
            .as_array()
            .unwrap()
            .contains(&"Script Mod".into())
    );

    let listed: Vec<String> = json("_content.json")["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file.as_str().unwrap().to_owned())
        .collect();
    let listed_set: BTreeSet<String> = listed.iter().cloned().collect();
    assert_eq!(listed.len(), listed_set.len(), "a file is listed twice");
    assert_eq!(
        listed_set,
        content_files(),
        "_content.json must list exactly the files under content/"
    );
}

#[test]
fn every_resource_names_a_script_the_mod_has() {
    let files = content_files();
    let resources: Vec<&String> = files.iter().filter(|f| f.ends_with(".res.lua")).collect();
    assert!(!resources.is_empty(), "no GUI resource loads the mod");
    for resource in resources {
        let text = std::fs::read_to_string(mod_dir().join("content").join(resource)).unwrap();
        let prefix = format!("\"{MOD_ID}::/");
        let start = text
            .find(&prefix)
            .unwrap_or_else(|| panic!("{resource} names no file of this mod"))
            + prefix.len();
        let target = &text[start..];
        let target = &target[..target.find('"').unwrap()];
        // A plugin names "script@recipe"; a replacement config names its
        // script and, apart, the function the game calls (doReplaceFn).
        let (script, recipe) = match target.split_once('@') {
            Some(named) => named,
            None => {
                let key = "doReplaceFn = \"";
                let at = text
                    .find(key)
                    .unwrap_or_else(|| panic!("{resource} names no recipe or doReplaceFn"))
                    + key.len();
                (target, &text[at..at + text[at..].find('"').unwrap()])
            }
        };
        assert!(
            files.contains(&format!("{script}.lua")),
            "{resource} names {script}.lua, which is not in content/"
        );
        assert!(!recipe.is_empty());
    }
}

/// A Lua state standing in for the game's GUI state, with the mod's files
/// readable through `mod_source`.
fn gui() -> Lua {
    let lua = Lua::new();
    let source = lua
        .create_function(|_, rel: String| {
            assert!(!rel.contains(".."), "{rel} leaves the mod");
            let path = mod_dir().join("content").join(&rel);
            std::fs::read_to_string(&path)
                .map_err(|error| mlua::Error::external(format!("{rel}: {error}")))
        })
        .unwrap();
    lua.globals().set("mod_source", source).unwrap();
    // What the hook does with an action table: convert it with the schema.
    let schema_check = lua
        .create_function(|_, action: mlua::Value| {
            Ok(
                match tpf3mp_proto::lua::action_from_lua(&common::tree(&action)) {
                    Ok(_) => (true, None),
                    Err(error) => (false, Some(error.to_string())),
                },
            )
        })
        .unwrap();
    lua.globals().set("schema_check", schema_check).unwrap();
    lua.load(FAKE_GUI).set_name("@fake_gui.lua").exec().unwrap();
    lua
}

fn log(lua: &Lua) -> String {
    lua.load("return logText()").eval().unwrap()
}

/// Loads the plugin, mounts it and runs `steps` frames.
fn run_frames(lua: &Lua, steps: usize) {
    lua.load(format!(
        "local m = mount(loadPlugin()); for _ = 1, {steps} do m.render(); m.step() end"
    ))
    .set_name("@frames")
    .exec()
    .unwrap_or_else(|error| panic!("{error}\n{}", log(lua)));
}

#[test]
fn without_the_hook_the_mod_loads_and_does_nothing() {
    let lua = gui();
    let before: Vec<String> = loaded_names(&lua);
    run_frames(&lua, 3);
    let log = log(&lua);
    assert_eq!(
        log,
        "[tpf3mp] modules loaded\n\
         [tpf3mp] no hook in this game; this is the plain game",
        "started once, however many frames"
    );

    // Only the mod's own names were added to package.loaded.
    let added: Vec<String> = loaded_names(&lua)
        .into_iter()
        .filter(|name| !before.contains(name))
        .collect();
    assert_eq!(
        added,
        [
            "tpf3mp.bridge",
            "tpf3mp.capture",
            "tpf3mp.companies",
            "tpf3mp.engine",
            "tpf3mp.follow",
            "tpf3mp.geom",
            "tpf3mp.guard",
            "tpf3mp.progression",
            "tpf3mp.registry",
            "tpf3mp.roads",
            "tpf3mp.ui",
            "tpf3mp.worldload"
        ]
    );
}

#[test]
fn the_multiplayer_window_shows_the_room_and_sends_what_the_player_says() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_CMD).exec().unwrap();
    lua.load(
        "HOOK.room = true \
         HOOK.status = { room = 'Sunday line', speed = 200, players = { \
             { name = 'Julian', connected = true, owner = true, me = false }, \
             { name = 'Sam', connected = true, owner = false, me = true } } } \
         HOOK.heard = { { from = 'Julian', text = 'the bus is late' } } \
         BAR = mount(loadPlugin()) BAR.step() BAR.render() \
         MODS = mount(loadPlugin(nil, 'Tpf3mpButton', 'MainModButtonAreaExtension')) \
         function texts() \
             local out = {} \
             for _, v in ipairs(views(WINDOWS.Tpf3mpWindow.render())) do \
                 if v.view == 'TextView' then out[#out + 1] = v.params.text end \
             end \
             return out \
         end",
    )
    .exec()
    .unwrap_or_else(|error| panic!("{error}\n{}", log(&lua)));
    // The game bar: the room in one line, and a new chat line; the button
    // in the mods' button area says the same new line; no window yet.
    let (label, button, open): (String, String, bool) = lua
        .load(
            "return views(BAR.layout)[1].params.content.params.text, \
                    views(MODS.layout)[1].params.content.params.text, \
                    WINDOWS.Tpf3mpWindow ~= nil",
        )
        .eval()
        .unwrap();
    assert_eq!(label, "Multiplayer: Sunday line · 2/2 playing · 2x · 1 new");
    assert_eq!(button, "Multiplayer (1)");
    assert!(!open, "closed until a button is pressed");
    // The game bar's button opens the window in the game's window
    // container: the room, its players, the chat.
    let (title, texts): (String, Vec<String>) = lua
        .load(
            "views(BAR.layout)[1].params.onClick() \
             WINDOWS.Tpf3mpWindow.step() \
             return WINDOWS.Tpf3mpWindow.layout.params.title, texts()",
        )
        .eval()
        .unwrap_or_else(|error| panic!("{error}\n{}", log(&lua)));
    assert_eq!(title, "Multiplayer");
    assert_eq!(
        texts,
        [
            "Sunday line",
            "Speed: 2x",
            "Host controls speed",
            "Worlds match",
            "Players",
            "2 of 2 online",
            "Julian",
            "host",
            "Sam",
            "you",
            "Companies",
            "Choose who you build with",
            "Waiting for the companies...",
            "Chat",
            "Julian: the bus is late",
            "Send"
        ]
    );
    // What the player types goes to the room.
    let said: Vec<String> = lua
        .load(
            "for _, v in ipairs(views(WINDOWS.Tpf3mpWindow.layout)) do \
                 if v.view == 'TextInputField' then v.params.onValueChange('on my way') end \
             end \
             return HOOK.said",
        )
        .eval()
        .unwrap();
    assert_eq!(said, ["on my way"]);
    // A line typed and then left (a click elsewhere cancels the field) stays
    // in the field as typed, so Send sends what the field shows.
    let (kept, resets): (String, bool) = lua
        .load(
            "local function field() \
                 for _, v in ipairs(views(WINDOWS.Tpf3mpWindow.layout)) do \
                     if v.view == 'TextInputField' then return v end \
                 end \
             end \
             field().params.onTyping('half typed') \
             field().params.onCancel() \
             WINDOWS.Tpf3mpWindow.step() \
             WINDOWS.Tpf3mpWindow.render() \
             return field().params.value, field().params.resetValueOnCancel",
        )
        .eval()
        .unwrap();
    assert_eq!(kept, "half typed");
    assert!(!resets, "the field keeps what was typed");
    // The other button closes it, and opens it again; so does the window's
    // own close button.
    let (closed, reopened, closed_by_itself): (bool, bool, bool) = lua
        .load(
            "views(MODS.layout)[1].params.onClick() \
             local closed = WINDOWS.Tpf3mpWindow == nil \
             views(MODS.layout)[1].params.onClick() \
             local reopened = WINDOWS.Tpf3mpWindow ~= nil \
             WINDOWS.Tpf3mpWindow.layout.params.onClose() \
             return closed, reopened, WINDOWS.Tpf3mpWindow == nil",
        )
        .eval()
        .unwrap();
    assert!(closed && reopened && closed_by_itself);
}

#[test]
fn chat_a_new_world_is_given_again_is_not_new() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_CMD).exec().unwrap();
    let label: String = lua
        .load(
            "HOOK.room = true \
             HOOK.status = { room = 'r', players = {} } \
             HOOK.heard = { { from = 'Sam', text = 'before', old = true }, \
                            { from = 'Sam', text = 'after' } } \
             BAR = mount(loadPlugin()) BAR.step() BAR.render() \
             return views(BAR.layout)[1].params.content.params.text",
        )
        .eval()
        .unwrap_or_else(|error| panic!("{error}\n{}", log(&lua)));
    assert_eq!(label, "Multiplayer: r · 0/0 playing · 1 new");
}

#[test]
fn the_room_panel_keeps_large_rosters_and_unicode_chat_inside_scroll_areas() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_CMD).exec().unwrap();
    lua.load(
        r#"
        HOOK.room = true
        HOOK.status = { room = 'A busy room', me_id = 'me', players = {} }
        for i = 1, 8 do
            HOOK.status.players[i] = { name = string.rep('W', 64), id = 'p' .. i,
                connected = i ~= 8, me = i == 1 }
        end
        BAR = mount(loadPlugin()) BAR.step() BAR.render()
        local shared = package.loaded['tpf3mp.ui']
        shared.companies = { list = {}, members = {}, loans = {} }
        api.type.Vec3f = { new = function(x,y,z) return { x=x, y=y, z=z } end }
        for i = 0, 7 do
            shared.companies.list[i+1] = { id=i, entity=i+1, name=string.rep('界', 64),
                color={0.8,0.2,0.1}, balance=44149292, owed=50050007 }
        end
        shared.lines = { string.rep('界', 280) }
        views(BAR.layout)[1].params.onClick()
        LAYOUT = WINDOWS.Tpf3mpWindow.render()
        "#,
    )
    .exec()
    .unwrap();
    let (areas, cards, company_text, chat_text, composer_outside): (
        usize,
        usize,
        String,
        String,
        bool,
    ) = lua
        .load(
            r#"
            local areas, cards, companyText, chatText = {}, 0, '', ''
            for _, v in ipairs(views(LAYOUT)) do
                if v.view == 'ScrollArea' then
                    assert(v.params.horizontalPolicy == 'AlwaysOff')
                    assert(v.params.verticalPolicy == 'AsNeeded')
                    assert(v.params.meta.styleSheet.size.y > 0 and v.params.meta.styleSheet.size.y <= 310)
                    areas[#areas+1] = v
                end
            end
            for _, v in ipairs(views(areas[2])) do
                if v.view == 'TextInputField' and v.params.placeholderText:find('A new name', 1, true) then
                    assert(cards == 1, 'manage your own company before scrolling past the other companies')
                end
                if v.view == 'TextView' and v.params.meta.tooltip == string.rep('界',64) then
                    cards = cards + 1
                    companyText = v.params.text:gsub('\n','')
                end
            end
            for _, v in ipairs(views(areas[3])) do
                if v.view == 'TextView' then chatText = v.params.text:gsub('\n','') end
            end
            local composer, inside = false, false
            for _, v in ipairs(views(LAYOUT)) do
                if v.view == 'TextInputField' and v.params.placeholderText == 'Say something to the room' then
                    composer = true
                    for _, area in ipairs(areas) do
                        for _, child in ipairs(views(area)) do if child == v then inside = true end end
                    end
                end
            end
            return #areas, cards, companyText, chatText, composer and not inside
            "#,
        )
        .eval()
        .unwrap();
    assert_eq!((areas, cards), (3, 8));
    assert_eq!(company_text, "界".repeat(64));
    assert_eq!(chat_text, "界".repeat(280));
    assert!(
        composer_outside,
        "chat can be sent without scrolling past a roster"
    );
}

#[test]
fn only_the_newest_chat_lines_show_in_the_window() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_CMD).exec().unwrap();
    let chat: Vec<String> = lua
        .load(
            "HOOK.room = true \
             HOOK.status = { room = 'r', players = {} } \
             for i = 1, 60 do HOOK.heard[i] = { from = 'Sam', text = 'line ' .. i } end \
             BAR = mount(loadPlugin()) BAR.step() BAR.render() \
             views(BAR.layout)[1].params.onClick() \
             local out, after = {}, false \
             for _, v in ipairs(views(WINDOWS.Tpf3mpWindow.render())) do \
                 if v.view == 'TextView' and after and v.params.text ~= 'Send' then out[#out + 1] = v.params.text end \
                 if v.view == 'TextView' and v.params.text == 'Chat' then after = true end \
             end \
             return out",
        )
        .eval()
        .unwrap_or_else(|error| panic!("{error}\n{}", log(&lua)));
    let expected: Vec<String> = (49..=60).map(|i| format!("Sam: line {i}")).collect();
    assert_eq!(chat, expected);
}

#[test]
fn a_window_the_game_will_not_show_is_said_in_the_game_bar() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_CMD).exec().unwrap();
    let (shown, open): (String, bool) = lua
        .load(
            "HOOK.room = true \
             HOOK.status = { room = 'r', players = {} } \
             BAR = mount(loadPlugin()) BAR.step() BAR.render() \
             ug_require('::/gui/main/game_react_globals.tl').getDefaultWindowApi = function() error('no window container') end \
             views(BAR.layout)[1].params.onClick() \
             BAR.step() BAR.render() \
             local v = views(BAR.layout) \
             return v[#v].params.text, package.loaded['tpf3mp.ui'].open",
        )
        .eval()
        .unwrap_or_else(|error| panic!("{error}\n{}", log(&lua)));
    assert_eq!(shown, "The Multiplayer window did not open");
    assert!(!open);
    assert!(
        log(&lua).contains("the Multiplayer window did not open: "),
        "{}",
        log(&lua)
    );
}

/// The names in package.loaded, sorted.
fn loaded_names(lua: &Lua) -> Vec<String> {
    let loaded: Table = lua.load("return package.loaded").eval::<Table>().unwrap();
    let mut names: Vec<String> = loaded
        .pairs::<String, mlua::Value>()
        .map(|pair| pair.unwrap().0)
        .collect();
    names.sort();
    names
}

const FAKE_HOOK: &str = r#"
HOOK = { logged = {}, commands = {}, batch = nil, request = nil, saved = {}, worlds = 0,
         room = false, checkpoint = false, lanes = nil, clicks = nil, replaying = {},
         applied = {}, results = {}, status = nil, heard = {}, said = {}, built = {},
         dump = nil, dumped = {} }
tpf3mp_native = {
    version = 12,
    note = function(key, value)
        HOOK.notes = HOOK.notes or {}
        if value == nil then return HOOK.notes[key] end
        HOOK.notes[key] = value ~= "" and value or nil
    end,
    command = function(action, password)
        local ok, why = schema_check(action)
        if ok then
            HOOK.commands[#HOOK.commands + 1] = action
            -- What the hook would send the room beside it, by ticket.
            HOOK.passwords = HOOK.passwords or {}
            HOOK.passwords[#HOOK.commands] = password
            return true, #HOOK.commands
        end
        return ok, why
    end,
    take = function()
        local batch, origins, seals = HOOK.batch, HOOK.origins, HOOK.seals
        HOOK.batch, HOOK.origins, HOOK.seals = nil, nil, nil
        return batch, origins, seals
    end,
    log = function(line) HOOK.logged[#HOOK.logged + 1] = line end,
    poll = function()
        local request = HOOK.request
        HOOK.request = nil
        return request
    end,
    saved = function(name, ok, why)
        HOOK.saved[#HOOK.saved + 1] = tostring(name) .. ' ' .. tostring(ok) .. ' ' .. tostring(why)
    end,
    world = function() HOOK.worlds = HOOK.worlds + 1 end,
    room = function() return HOOK.room end,
    checkpoint = function() return HOOK.checkpoint end,
    seed = function() return HOOK.seed end,
    lanes = function(lanes)
        if not HOOK.checkpoint then return false, 'no checkpoint is due in this update' end
        HOOK.lanes = lanes
        HOOK.checkpoint = false
        return true
    end,
    clicks = function() return HOOK.clicks end,
    -- The module editor's builds the hook read, by click: { proposal = t }
    -- or { why = text }, each taken once.
    built = function(n)
        local b = HOOK.built[n]
        HOOK.built[n] = nil
        if b == nil then return nil end
        if b.why then return nil, b.why end
        return b.proposal
    end,
    replaying = function(on) HOOK.replaying[#HOOK.replaying + 1] = on end,
    applied = function(i, ok, entity, why)
        HOOK.applied[#HOOK.applied + 1] = { i = i, ok = ok, entity = entity, why = why }
    end,
    results = function()
        local results = HOOK.results
        HOOK.results = {}
        return results
    end,
    status = function() return HOOK.status end,
    chat = function()
        local heard = HOOK.heard
        HOOK.heard = {}
        return heard
    end,
    say = function(text)
        if text:match('^%s*$') then return false, 'nothing to say' end
        HOOK.said[#HOOK.said + 1] = text
        return true
    end,
    -- A lane dump the hook asks for ({ step =, lanes = }), once; the
    -- entries go to HOOK.dumped as the hook writes them to its log.
    dump = function()
        local order = HOOK.dump
        HOOK.dump = nil
        HOOK.dumping = order
        return order
    end,
      dumped = function(lane, entry)
          local order = HOOK.dumping
          if order == nil then return false end
          HOOK.dumped[#HOOK.dumped + 1] = 'lane ' .. lane .. ' step ' .. order.step .. ' ' .. entry
          return true
      end,
      -- The player's own pointer or build preview (cursor) and what the other
      -- members' are showing (cursors).
      cursor = function(x, y, building, label, curves)
          HOOK.cursor = { x = x, y = y, building = building, label = label, curves = curves }
      end,
      cursors = function()
          return HOOK.cursors or {}
      end,
  }
  "#;

/// The GUI state's api.cmd, as much of it as the guard's tests use: three
/// factories, and a sendCommand that keeps what it was sent.
const FAKE_CMD: &str = r#"
SENT = {}
api = api or {}
api.cmd = {
    makeGameSetSpeedCmd = function(speed) return { kind = 'speed', speed = speed } end,
    makeVehicleBuyCmd = function(player, depot, config) return { kind = 'buy', depot = depot } end,
    makeLineCreateCmd = function(line) return { kind = 'line' } end,
    makeScriptingSendEventCmd = function(src, id, name, param)
        return { kind = 'event', id = id, name = name, param = param }
    end,
    sendCommand = function(command, ...)
        SENT[#SENT + 1] = { command = command, extra = select('#', ...), callback = (...) }
    end,
}
"#;

#[test]
fn the_speed_row_shows_the_room_and_only_the_host_can_use_it() {
    for with_hook in [false, true] {
        let lua = gui();
        if with_hook {
            lua.load(FAKE_HOOK).exec().unwrap();
        }
        lua.load(include_str!("lua/speed_ui.lua"))
            .set_name("@speed_ui.lua")
            .exec()
            .unwrap_or_else(|error| panic!("hook={with_hook}: {error}"));
    }
}

/// The game's save and load, as the GUI state has them.
const FAKE_APP: &str = r#"
APP = { saves = {}, loads = {} }
app = {
    saveGame = function(name, callback, isMapEditor, skipSetName)
        APP.saves[#APP.saves + 1] = { name = name, callback = callback,
                                      isMapEditor = isMapEditor, skipSetName = skipSetName }
    end,
    loadGame = function(id, isMapEditor, info)
        APP.loads[#APP.loads + 1] = { id = id, isMapEditor = isMapEditor }
    end,
    SaveGameNamespace = { getSavegame = function() return "savegame" end },
}
api = { type = { SavegameId = { new = function() return {} end } } }
"#;

#[test]
fn with_the_hook_the_gui_links_once() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_CMD).exec().unwrap();
    run_frames(&lua, 2);
    assert_eq!(
        log(&lua),
        "[tpf3mp] modules loaded\n[tpf3mp] linked to the hook"
    );
    let logged: String = lua
        .load("return table.concat(HOOK.logged, '|')")
        .eval()
        .unwrap();
    assert_eq!(
        logged,
        "the GUI is linked|the guard is on 4 command factories|\
         the GUI's company cannot follow the player's: no api.engine.util.getPlayer (nil, nil)|\
         the company window shows the game's own rank only: the game's company progression did \
         not load: fake_gui.lua:149: ug_require of an unknown path \
         /game_mechanics/company/company_progression_util.tl|\
         the line manager offers other companies' open stations (1 entity_util table(s))"
    );
    let worlds: u32 = lua.load("return HOOK.worlds").eval().unwrap();
    assert_eq!(worlds, 1, "the world's GUI started once");
}

#[test]
fn the_gui_saves_what_the_hook_asks_and_answers_when_written() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_APP).exec().unwrap();
    lua.load("M = mount(loadPlugin())").exec().unwrap();
    lua.load("M.step() HOOK.request = { save = 'tpf3mp_77_5' } M.step()")
        .exec()
        .unwrap();
    let (name, map_editor, skip): (String, bool, bool) = lua
        .load("local s = APP.saves[1] return s.name, s.isMapEditor, s.skipSetName")
        .eval()
        .unwrap();
    assert_eq!(name, "tpf3mp_77_5");
    assert!(!map_editor);
    assert!(skip, "the player's own save name is left alone");
    let answered: usize = lua.load("return #HOOK.saved").eval().unwrap();
    assert_eq!(answered, 0, "not written yet");
    lua.load("APP.saves[1].callback()").exec().unwrap();
    let saved: Vec<String> = lua.load("return HOOK.saved").eval().unwrap();
    assert_eq!(saved, ["tpf3mp_77_5 true nil"]);
    // A save the game refuses at once is answered as failed.
    lua.load(
        "app.saveGame = function() error('no disk') end \
         HOOK.request = { save = 'tpf3mp_77_6' } M.step()",
    )
    .exec()
    .unwrap();
    let saved: Vec<String> = lua.load("return HOOK.saved").eval().unwrap();
    assert!(saved[1].starts_with("tpf3mp_77_6 false "), "{}", saved[1]);
    assert!(saved[1].ends_with("no disk"), "{}", saved[1]);
}

#[test]
fn the_gui_loads_the_rooms_world_from_the_save_folder() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_APP).exec().unwrap();
    lua.load(
        "M = mount(loadPlugin()) M.step() HOOK.request = { load = 'tpf3mp_room_77' } M.step()",
    )
    .exec()
    .unwrap();
    let loaded: String = lua
        .load(
            "local l = APP.loads[1] \
             return l.id.path .. '|' .. l.id.saveGameName .. '|' .. l.id.saveGameNamespace \
               .. '|' .. tostring(l.isMapEditor)",
        )
        .eval()
        .unwrap();
    assert_eq!(loaded, "|tpf3mp_room_77|savegame|false");
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert_eq!(logged.last().unwrap(), "loading the room's world");
}

/// The hook's `mods` as the room's lists make it (crates/tpf3mp-bridge,
/// `mods::plan`), and the game's save details, mods and ModId, for a load
/// with the room's mods (docs/MODS.md).
const FAKE_MODS: &str = r#"
SHARED = { vehicles_pack = true }
MINE = { 'my_colours' }
tpf3mp_native.mods = function(list)
    if list == nil then return true end
    local keep, dropped, added = {}, {}, {}
    for name in string.gmatch(list, '[^\n]+') do
        if SHARED[name] or name == 'tpf3mp_1' or name == MINE[1] then keep[#keep + 1] = name
        else dropped[#dropped + 1] = name end
    end
    keep[#keep + 1] = MINE[1] added[1] = MINE[1]
    return table.concat(keep, '\n'), table.concat(dropped, '\n'), table.concat(added, '\n')
end
SAVED = { 'vehicles_pack', 'tpf3mp_1', 'owner_minimap' }
INSTALLED = { vehicles_pack = true, tpf3mp_1 = true, my_colours = true }
READY = false
api.type.ModId = { new = function() return {} end }
api.type.SaveGameDetails = { new = function(info)
    local copy = {} for k, v in pairs(info) do copy[k] = v end return copy end }
app.getSavegameInfo = function(id)
    local mods = {}
    for i, name in ipairs(SAVED) do mods[i] = { name = name } end
    return { isCompleted = function() return READY end,
             get = function() return { errorMsg = '', info = { mods = mods } } end }
end
app.getUserProfile = function() return { getModRep = function() return {
    exists = function(_, m) return INSTALLED[m.name] == true end } end } end
local load = app.loadGame
app.loadGame = function(id, isMapEditor, info)
    load(id, isMapEditor, info)
    local names = {}
    for _, m in ipairs(info and info.mods or {}) do names[#names + 1] = m.name end
    APP.loads[#APP.loads].mods = table.concat(names, ',')
end
"#;

#[test]
fn the_gui_loads_the_rooms_world_with_the_rooms_mods_and_its_own() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_APP).exec().unwrap();
    lua.load(FAKE_MODS).exec().unwrap();
    lua.load(
        "M = mount(loadPlugin()) M.step() HOOK.request = { load = 'tpf3mp_room_77' } M.step()",
    )
    .exec()
    .unwrap();
    // The game reads the save's details over a few frames.
    let loads: usize = lua.load("M.step() return #APP.loads").eval().unwrap();
    assert_eq!(loads, 0);
    let (name, mods): (String, String) = lua
        .load("READY = true M.step() return APP.loads[1].id.saveGameName, APP.loads[1].mods")
        .eval()
        .unwrap();
    assert_eq!(name, "tpf3mp_room_77");
    assert_eq!(
        mods, "vehicles_pack,tpf3mp_1,my_colours",
        "the owner's minimap left out, this player's colours added"
    );
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert_eq!(logged.last().unwrap(), "loading the room's world");

    // A shared mod this player lacks: not loaded, and said why.
    lua.load("INSTALLED.vehicles_pack = nil HOOK.request = { load = 'tpf3mp_room_78' } M.step()")
        .exec()
        .unwrap();
    let (loads, logged): (usize, Vec<String>) =
        lua.load("return #APP.loads, HOOK.logged").eval().unwrap();
    assert_eq!(loads, 1);
    assert_eq!(
        logged.last().unwrap(),
        "loading the room's world failed: the room's world needs the mod vehicles_pack, which is not installed"
    );
}

#[test]
fn a_hook_of_another_version_is_not_used() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load("tpf3mp_native.version = 1").exec().unwrap();
    run_frames(&lua, 1);
    assert!(
        log(&lua).ends_with(
            "[tpf3mp] the hook speaks bridge version 1, the mod 12; this is the plain game"
        ),
        "{}",
        log(&lua)
    );
    let logged: usize = lua.load("return #HOOK.logged").eval().unwrap();
    assert_eq!(logged, 0, "nothing was said to a hook of another version");
}

/// The bridge on its own, loaded as the entry script loads it.
fn bridge(lua: &Lua) -> Table {
    lua.load("return ug_require('tpf3mp_1::/scripts/tpf3mp/bridge.lua')")
        .eval()
        .unwrap()
}

#[test]
fn the_bridge_hands_over_only_actions_the_schema_takes() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    let bridge = bridge(&lua);
    let attach: Function = bridge.get("attach").unwrap();
    let native: Table = lua.globals().get("tpf3mp_native").unwrap();
    let link: Table = attach.call(native).unwrap();
    lua.globals().set("LINK", link).unwrap();

    let refusals: Vec<String> = lua
        .load(
            "local out = {}
             local function try(p) local ok, why = LINK:command(p); out[#out + 1] = ok and 'ok' or why end
             try('bytes')
             try({ SellVehicle = { vehicles = { 7, 9 } } })
             try({ SellVehicle = { vehicles = { 0.5 } } })
             try({ SellVehicle = { vehicles = {}, colour = 'red' } })
             tpf3mp_native.command = function() return false end
             try({ SellVehicle = { vehicles = { 7 } } })
             tpf3mp_native.command = function() error('ring full') end
             try({ SellVehicle = { vehicles = { 7 } } })
             return out",
        )
        .eval()
        .unwrap();
    assert_eq!(refusals[0], "an action is a table");
    assert_eq!(refusals[1], "ok");
    assert_eq!(
        refusals[2],
        "the hook refused the action: SellVehicle.vehicles[1]: not a whole number: 0.5"
    );
    assert_eq!(
        refusals[3],
        "the hook refused the action: SellVehicle: variant has no field colour"
    );
    assert_eq!(refusals[4], "the hook refused the action: no reason given");
    assert!(refusals[5].starts_with("the hook refused: "));
    assert!(refusals[5].ends_with("ring full"));
    let sent: usize = lua.load("return #HOOK.commands").eval().unwrap();
    assert_eq!(sent, 1, "only the action the schema took reached the room");
}

#[test]
fn take_is_a_list_or_nothing_and_never_raises() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    let bridge = bridge(&lua);
    lua.globals().set("BRIDGE", bridge).unwrap();
    let results: Vec<String> = lua
        .load(
            "local link = BRIDGE.attach(tpf3mp_native)
             local out = {}
             out[#out + 1] = tostring(link:take())
             HOOK.batch = { { SellVehicle = { vehicles = { 7 } } } }
             out[#out + 1] = tostring(#link:take())
             out[#out + 1] = tostring(link:take())
             tpf3mp_native.take = function() error('boom') end
             out[#out + 1] = tostring(link:take())
             return out",
        )
        .eval()
        .unwrap();
    assert_eq!(results, ["nil", "1", "nil", "nil"]);
}

#[test]
fn attach_refuses_a_partial_hook() {
    let lua = gui();
    let bridge = bridge(&lua);
    lua.globals().set("BRIDGE", bridge).unwrap();
    let reasons: Vec<String> = lua
        .load(
            "local out = {}
             local function why(t) local _, r = BRIDGE.attach(t); out[#out + 1] = r end
             why(nil)
             why('hook')
             why({ version = 12, command = print, log = print })
             return out",
        )
        .eval()
        .unwrap();
    assert_eq!(
        reasons,
        [
            "no hook in this game",
            "tpf3mp_native is not a table",
            "the hook has no take()",
        ]
    );
}

/// The game bar's text, if the plugin shows any.
fn shown(lua: &Lua) -> Option<String> {
    lua.load(
        "local c = M.render().params.children[1] \
         return c and c.params.text",
    )
    .eval()
    .unwrap()
}

#[test]
fn in_the_rooms_game_the_gui_refuses_what_the_room_cannot_carry() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_CMD).exec().unwrap();
    lua.load("M = mount(loadPlugin()) M.step()").exec().unwrap();

    // Before the room's game every command is sent, arguments as given.
    lua.load("api.cmd.sendCommand(api.cmd.makeVehicleBuyCmd(1, 2, {}))")
        .exec()
        .unwrap();
    let (sent, extra): (usize, usize) = lua.load("return #SENT, SENT[1].extra").eval().unwrap();
    assert_eq!(
        (sent, extra),
        (1, 0),
        "a callback left out is not passed as nil"
    );

    // In the room's game the speed row's speed is sent; a vehicle bought at
    // a depot the room cannot name is refused, and its callback hears so on
    // the next frame.
    lua.load(
        "HOOK.room = true \
         api.cmd.sendCommand(api.cmd.makeGameSetSpeedCmd(4)) \
         CALLED = nil \
         BUY = api.cmd.makeVehicleBuyCmd(1, 2, {}) \
         api.cmd.sendCommand(BUY, function(data, ok, entities) \
             CALLED = { data = data, ok = ok, entities = #entities } end)",
    )
    .exec()
    .unwrap();
    let (sent, speed): (usize, u32) = lua
        .load("return #SENT, SENT[2].command.speed")
        .eval()
        .unwrap();
    assert_eq!((sent, speed), (2, 4), "the speed went, the vehicle did not");
    let called: bool = lua.load("return CALLED ~= nil").eval().unwrap();
    assert!(!called, "not within sendCommand");
    lua.load("M.step()").exec().unwrap();
    let (same, ok, entities): (bool, bool, usize) = lua
        .load("return CALLED.data == BUY, CALLED.ok, CALLED.entities")
        .eval()
        .unwrap();
    assert!(same && !ok, "the callback heard the command failed");
    assert_eq!(entities, 0);
    assert_eq!(
        shown(&lua).as_deref(),
        Some("Not in multiplayer yet: buying vehicles")
    );

    // A command no factory made is refused too.
    lua.load("api.cmd.sendCommand({ kind = 'forged' }) M.step()")
        .exec()
        .unwrap();
    assert_eq!(
        shown(&lua).as_deref(),
        Some("Not in multiplayer yet: this action")
    );
    let sent: usize = lua.load("return #SENT").eval().unwrap();
    assert_eq!(sent, 2);
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert!(
        logged.contains(
            &"refused the player's makeVehicleBuyCmd in the room's game (1 so far): \
               a depot the room cannot name"
                .to_owned()
        ),
        "{logged:?}"
    );
    assert!(
        logged.contains(
            &"refused the player's command no factory made in the room's game (1 so far)"
                .to_owned()
        ),
        "{logged:?}"
    );

    // The notice goes after a few seconds; after the room's game, commands
    // are sent again.
    lua.load("for _ = 1, 400 do M.step() end").exec().unwrap();
    assert_eq!(shown(&lua), None);
    lua.load("HOOK.room = false api.cmd.sendCommand(api.cmd.makeLineCreateCmd({}))")
        .exec()
        .unwrap();
    let sent: usize = lua.load("return #SENT").eval().unwrap();
    assert_eq!(sent, 3);
}

/// A loan offer as the game's loan script and finance window keep it.
const OFFER: &str = "{ type = 'Small', amount = 5000000, duration = 1095000, \
                       percentage = 0.03, birthDay = 400000 }";

#[test]
fn in_the_rooms_game_a_loan_goes_to_the_room_and_nothing_else_of_its_kind() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_CMD).exec().unwrap();
    lua.load("M = mount(loadPlugin()) M.step() HOOK.room = true")
        .exec()
        .unwrap();
    // The finance window's "Obtain", as it sends it.
    lua.load(format!(
        "NEXT = {OFFER} NEXT.amount = 7000000 \
         CALLED = nil \
         api.cmd.sendCommand(api.cmd.makeScriptingSendEventCmd('', 'Loan', 'Obtain', {{ NEXT, {OFFER} }}), \
             function(data, ok) CALLED = ok end) \
         M.step()"
    ))
    .exec()
    .unwrap();
    let (sent, handed, called): (usize, usize, bool) = lua
        .load("return #SENT, #HOOK.commands, CALLED ~= nil")
        .eval()
        .unwrap();
    assert_eq!(sent, 0, "not run here: the room orders it for every game");
    assert_eq!(handed, 1, "handed to the room, through the schema");
    assert!(!called, "the room has not applied it yet");
    // This game applied the room's action: the window hears it went.
    lua.load("HOOK.results = { { ticket = 1, ok = true } } M.step()")
        .exec()
        .unwrap();
    let called: bool = lua.load("return CALLED == true").eval().unwrap();
    assert!(called, "the window hears it went");
    let (take, amount): (bool, u32) = lua
        .load("local l = HOOK.commands[1].Loan return l.Take ~= nil, l.Take.offer.amount")
        .eval()
        .unwrap();
    assert!(take);
    assert_eq!(amount, 5_000_000);
    // Paying back goes too.
    lua.load(format!(
        "api.cmd.sendCommand(api.cmd.makeScriptingSendEventCmd('', 'Loan', 'Repay', {{ nil, {OFFER} }}))"
    ))
    .exec()
    .unwrap();
    let repay: bool = lua
        .load("return HOOK.commands[2].Loan.Repay.loan.amount == 5000000")
        .eval()
        .unwrap();
    assert!(repay);
    // Another script event is refused, as is a loan the schema does not
    // take.
    lua.load(
        "api.cmd.sendCommand(api.cmd.makeScriptingSendEventCmd('', 'MakeGreen', 'go', {})) \
         api.cmd.sendCommand(api.cmd.makeScriptingSendEventCmd('', 'Loan', 'Obtain', \
             { { type = 'Small' }, { type = 'Small', amount = 1.5 } }))",
    )
    .exec()
    .unwrap();
    let (sent, handed): (usize, usize) = lua.load("return #SENT, #HOOK.commands").eval().unwrap();
    assert_eq!((sent, handed), (0, 2));
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert!(
        logged
            .iter()
            .any(|l| l.contains("makeScriptingSendEventCmd")
                && l.contains("the hook refused the action: Loan.Take")),
        "{logged:?}"
    );
}

#[test]
fn the_guard_goes_on_once_and_a_hook_that_cannot_say_means_the_room() {
    let lua = gui();
    lua.load(FAKE_CMD).exec().unwrap();
    let results: Vec<String> = lua
        .load(
            "local guard = ug_require('tpf3mp_1::/scripts/tpf3mp/guard.lua')
             local env = { inRoom = function() return true end,
                           refused = function() end, later = function() end }
             local out = {}
             out[#out + 1] = tostring(guard.install(api.cmd, env))
             local send = api.cmd.sendCommand
             out[#out + 1] = tostring(guard.install(api.cmd, env))
             out[#out + 1] = tostring(api.cmd.sendCommand == send)
             out[#out + 1] = select(2, guard.install(nil, env))
             out[#out + 1] = select(2, guard.install({}, env))
             local bridge = ug_require('tpf3mp_1::/scripts/tpf3mp/bridge.lua')
             local native = { version = 12 }
             for _, n in ipairs({ 'command', 'take', 'log', 'poll', 'saved', 'world',
                                  'checkpoint', 'lanes', 'clicks', 'replaying', 'applied', 'results',
                                  'status', 'chat', 'say' }) do
                 native[n] = function() end
             end
             native.room = function() error('gone') end
             out[#out + 1] = tostring(bridge.attach(native):room())
             native.room = function() return 1 end
             out[#out + 1] = tostring(bridge.attach(native):room())
             return out",
        )
        .eval()
        .unwrap();
    assert_eq!(
        results,
        [
            "4",
            "4",
            "true",
            "api.cmd is not a table",
            "api.cmd has no sendCommand",
            "true",
            "false"
        ]
    );
}

#[test]
fn every_game_script_names_a_script_the_mod_has() {
    let files = content_files();
    let scripts: Vec<&String> = files.iter().filter(|f| f.ends_with(".gs.lua")).collect();
    assert!(
        !scripts.is_empty(),
        "no game script applies the room's actions"
    );
    for script in scripts {
        let folder = script.rsplit_once('/').map_or("", |(folder, _)| folder);
        let text = std::fs::read_to_string(mod_dir().join("content").join(script)).unwrap();
        let mut named = 0;
        for part in text.split("fileName = \"").skip(1) {
            let target = &part[..part.find('"').unwrap()];
            let (file, function) = target.split_once('@').unwrap();
            assert!(
                files.contains(&format!("{folder}/{file}.lua")),
                "{script} names {file}.lua, which is not in {folder}/"
            );
            assert!(!function.is_empty());
            named += 1;
        }
        assert!(named > 0, "{script} names no script");
    }
}

/// A stand-in for an engine (game script) state: the commands it is sent
/// run at once, as the game's do there. REFUSE makes sendCommand raise,
/// FAILS makes a callback hear that the command failed, NO_CALLBACKS
/// refuses every callback.
const FAKE_ENGINE: &str = r#"
SENT = {}
REFUSE, FAILS, NO_CALLBACKS = false, false, false
local function vec4(x, y, z, w) return { x, y, z, w } end
api = {
    type = {
        ComponentType = { CONSTRUCTION = 2, TRANSPORT_VEHICLE = 4, STATION_GROUP = 9 },
        Vec4f = { new = vec4 },
        Mat4f = { new = function(a, b, c, d) return { a, b, c, d } end },
        SimpleProposal = {
            new = function() return { constructionsToAdd = {} } end,
            ConstructionEntity = { new = function() return {} end },
        },
        Context = { new = function() return {} end },
        Vec3f = { new = function(x, y, z) return { x = x, y = y, z = z } end },
    },
    engine = {
        util = { getPlayer = function() return 25 end },
        -- Nothing to list, unless a test's world says otherwise.
        getEntitiesWithComponent = function() return {} end,
        system = { lineSystem = { getLines = function() return {} end } },
    },
    cmd = {
        makeWorldBuildProposalCmd = function(proposal, context, ignoreErrors, playerInitiated)
            return { proposal = proposal, context = context, ignoreErrors = ignoreErrors,
                     playerInitiated = playerInitiated }
        end,
        makeScriptingSendEventCmd = function(src, id, name, param)
            return { event = { src = src, id = id, name = name, param = param } }
        end,
        makeGameAddPlayerCmd = function(name, color)
            NEXT_PLAYER = (NEXT_PLAYER or 900) + 1
            return { addPlayer = name, color = color, resultEntity = NEXT_PLAYER }
        end,
        makeEntitySetNameCmd = function(entity, name) return { setName = name, entity = entity } end,
        sendCommand = function(command, callback)
            -- As the game in a game script: no callback in update; in
            -- postUpdate one is called at once, with the command's data (the
            -- command here), whether it went, and what it made.
            if callback ~= nil and (PHASE == 'update' or NO_CALLBACKS) then
                error('Callbacks are currently disallowed')
            end
            if REFUSE then error('the proposal collides') end
            SENT[#SENT + 1] = command
            if callback ~= nil then
                callback(command, not FAILS, command.made and { { command.made, 1 } } or {})
            end
        end,
    },
}
STATE = {
    subscribed = {},
    value = nil,
    get = function(self) return self.value end,
    set = function(self, value) self.value = value end,
    hasEventSubscriptions = function(self) return next(self.subscribed) ~= nil end,
    subscribeToEvent = function(self, name) self.subscribed[name] = true end,
}
"#;

/// The mod's game script in a stand-in engine state with the fake hook:
/// returns the state and the script's functions.
fn engine() -> (Lua, Table) {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_ENGINE).exec().unwrap();
    let source = std::fs::read_to_string(
        mod_dir()
            .join("content")
            .join("tpf3mp_sim")
            .join("tpf3mp_sim.script.lua"),
    )
    .unwrap();
    lua.load(&source)
        .set_name("@tpf3mp_sim.script.lua")
        .exec()
        .unwrap();
    let script: Table = lua.load("return data()").eval().unwrap();
    // One simulation update as the game runs it: update, then postUpdate
    // with what update returned, and not when that is nil.
    lua.globals().set("SCRIPT", script.clone()).unwrap();
    lua.load(
        "UPDATE = function(p, s, dt) \
             if BEFORE_UPDATE then BEFORE_UPDATE() end \
             PHASE = 'update' \
             local r = SCRIPT.update(p, s, dt) \
             PHASE = 'post' \
             if r ~= nil then SCRIPT.postUpdate(p, s, dt, r) end \
             PHASE = nil \
             return r \
         end",
    )
    .exec()
    .unwrap();
    (lua, script)
}

fn junction_game(offset: u32) -> Lua {
    let (lua, _) = engine();
    lua.load(FAKE_NETWORK).exec().unwrap();
    lua.globals().set("OFFSET", offset).unwrap();
    lua.load(include_str!("lua/junction_world.lua"))
        .exec()
        .unwrap();
    lua
}

fn lua_value(lua: &Lua, value: &tpf3mp_proto::lua::LuaValue) -> mlua::Value {
    use tpf3mp_proto::lua::LuaValue as V;
    match value {
        V::Nil => mlua::Value::Nil,
        V::Boolean(v) => mlua::Value::Boolean(*v),
        V::Integer(v) => mlua::Value::Integer(*v),
        V::Number(v) => mlua::Value::Number(*v),
        V::String(v) => mlua::Value::String(lua.create_string(v).unwrap()),
        V::Table(entries) => {
            let table = lua.create_table().unwrap();
            for (k, v) in entries {
                table.set(lua_value(lua, k), lua_value(lua, v)).unwrap();
            }
            mlua::Value::Table(table)
        }
    }
}

#[test]
fn junction_tools_round_trip_through_the_wire_and_apply_with_each_games_ids() {
    use tpf3mp_proto::{
        action::Action,
        lua::{action_from_lua, action_to_lua},
    };
    let source = junction_game(0);
    let captured: mlua::Value = source
        .load("return C.windowBuild(nil,PROPOSAL)")
        .eval()
        .unwrap();
    let action = action_from_lua(&common::tree(&captured)).unwrap();
    let Action::EditJunctions(edit) = &action else {
        panic!("{action:?}");
    };
    let config = edit.changes[0].config.as_ref().unwrap();
    assert_eq!(
        (config.phases[0].duration, config.phases[0].minimum),
        (12_375, 4_125)
    );
    assert_eq!(config.crosswalks.len(), 2);
    let portable = Action::from_payload(&action.to_payload().unwrap()).unwrap();
    assert_eq!(portable, action);
    for offset in [0, 5000] {
        let replica = junction_game(offset);
        replica
            .globals()
            .set(
                "ACTION",
                lua_value(&replica, &action_to_lua(&portable).unwrap()),
            )
            .unwrap();
        let ids: Vec<u32> = replica
            .load(
                r#"
            HOOK.batch = {ACTION} UPDATE({},STATE,0.2)
            assert(#SENT==1, table.concat(HOOK.logged,'\n'))
            local s = SENT[1].proposal.streetProposal
            local c = s.nodeConfigsToAdd[1].comp
            assert(c.trafficLightPreference==1 and c.userModifiedTrafficLightStates)
            assert(c.laneConnections[2].withTram)
            assert(c.trafficLightConfig.states[1].duration==12.375)
            assert(c.trafficLightConfig.states[1].minDuration==4.125)
            assert(c.trafficLightConfig.states[1].lockedLanes[2]==2)
            return {s.nodeConfigsToAdd[1].entity,s.nodeConfigsToRemove[1],
                c.laneConnections[1].segment0,c.laneConnections[1].segment1,
                c.crosswalks[2],c.trafficLightConfig.trafficLightType}
        "#,
            )
            .eval()
            .unwrap();
        assert_eq!(ids, [1, 1, 101, 102, 103, 40].map(|n| n + offset));
    }
}

#[test]
fn a_native_junction_click_overrides_an_older_tools_preview() {
    let lua = junction_game(0);
    lua.load(format!(r#"
        HOOK.room=true HOOK.clicks=0 SCRIPT.guiUpdate({{}},nil,nil)
        SCRIPT.guiHandleEvent({{}},nil,nil,'','constructionBuilder','builder.proposalCreate',{{{CONSTRUCTION_PROPOSAL}}})
        PROPOSAL.junctionEdit=true HOOK.built[0]={{proposal=PROPOSAL}}
        HOOK.clicks=1 SCRIPT.guiUpdate({{}},nil,nil)
        assert(#HOOK.commands==1,table.concat(HOOK.logged,'\n'))
    "#)).exec().unwrap();
    let sent: mlua::Value = lua.load("return HOOK.commands[1]").eval().unwrap();
    // The fake hook stores action tables unchanged; never replay the old building.
    assert!(
        common::tree(&sent).get("EditJunctions").is_some(),
        "{sent:?}"
    );
}

#[test]
fn junction_checkpoint_rows_ignore_entity_and_connection_order_but_detect_settings() {
    let a = junction_game(0);
    let b = junction_game(5000);
    let before = read_lanes(&a)[0].clone();
    assert_ne!(before.1, "err");
    assert_eq!(before, read_lanes(&b)[0]);
    b.load(
        r#"
        local c=CONFIGS[5001]
        c.laneConnections[1],c.laneConnections[2]=c.laneConnections[2],c.laneConnections[1]
        c.crosswalks[1],c.crosswalks[2]=c.crosswalks[2],c.crosswalks[1]
        c.trafficLightConfig.states[1].lockedLanes={1,3}
        c.trafficLightConfig.states[2].lockedLanes={0,2}
    "#,
    )
    .exec()
    .unwrap();
    assert_eq!(before, read_lanes(&b)[0]);
    for mutation in [
        "CONFIGS[1].trafficLightPreference=2",
        "CONFIGS[1].laneConnections[1].withTram=true",
        "CONFIGS[1].crosswalks[1]=104",
        "CONFIGS[1].trafficLightConfig.states[1].duration=13",
        "EDGES[101].laneConfigs[1].transportModes[2]=false",
    ] {
        let changed = junction_game(0);
        changed.load(mutation).exec().unwrap();
        let after = read_lanes(&changed)[0].clone();
        assert_ne!(after.1, "err", "{mutation}");
        assert_ne!(before, after, "{mutation}");
    }
}

#[test]
fn junction_replay_refuses_ambiguous_stale_private_and_mixed_edits() {
    for (mutation, expected) in [
        (
            "EDGES[999]=EDGES[101] STREETS[1][5]=999",
            "ambiguous junction edge",
        ),
        ("EDGES[101].laneConfigs={}", "junction's lanes changed"),
        ("NODES[2].z=1", "road or track changed"),
        (
            "ACTION.EditJunctions.changes[1].config.light='missing'",
            "missing traffic light resource",
        ),
        (
            "ACTION.EditJunctions.changes[1].config.phases[1].locked={99}",
            "invalid locked lane",
        ),
    ] {
        let lua = junction_game(0);
        lua.load("ACTION=C.junction(PROPOSAL)").exec().unwrap();
        lua.load(mutation).exec().unwrap();
        let (ok, why): (bool, String) = lua.load("return A.run(ACTION)").eval().unwrap();
        assert!(!ok && why.contains(expected), "{mutation}: {why}");
        assert_eq!(lua.load("return #SENT").eval::<usize>().unwrap(), 0);
    }
    let lua = junction_game(0);
    lua.load(r#"
        local changes=C.junction(PROPOSAL).EditJunctions.changes
        local ok,why=pcall(J.into,api.type.SimpleProposal.new(),changes,{},function() error('private edge') end)
        assert(not ok and tostring(why):find('private edge',1,true))
        PROPOSAL.proposal.addedSegments={{entity=-1}}
        ok,why=pcall(C.junction,PROPOSAL)
        assert(not ok and tostring(why):find('geometry',1,true))
        PROPOSAL.proposal.addedSegments=nil PROPOSAL.proposal.nodeConfigsToAdd={}
        local reset=C.junction(PROPOSAL)
        assert(reset.EditJunctions.changes[1].config==nil)
        assert(A.run(reset))
        local s=SENT[1].proposal.streetProposal
        assert(#s.nodeConfigsToRemove==1 and not s.nodeConfigsToAdd)
    "#).exec().unwrap();
}

#[test]
fn unverified_junction_tools_are_refused_on_capture_and_replay() {
    let lua = junction_game(0);
    lua.load(
        r#"
        local action=C.junction(PROPOSAL)
        J.strict_junctions=false
        local ok,why=pcall(C.junction,PROPOSAL)
        assert(not ok and tostring(why):find('strict_junctions',1,true))
        ok,why=A.run(action)
        assert(not ok and tostring(why):find('strict_junctions',1,true))
        assert(#SENT==0)
    "#,
    )
    .exec()
    .unwrap();
}

#[test]
fn rebuilding_a_curved_road_preserves_turns_crosswalks_and_light_phases() {
    let lua = junction_game(0);
    lua.load(
        r#"
        -- Split a curve whose chord changes but its tangent at the junction doesn't.
        EDGES[101].tangent0={x=80,y=40,z=0}
        local e={entity=-2,type=0,comp={node0=1,node1=-1,tangent0={x=40,y=20,z=0},
            tangent1={x=40,y=0,z=0},laneConfigs=EDGES[101].laneConfigs}}
        local p=api.type.SimpleProposal.new()
        p.streetProposal={nodesToAdd={{entity=-1,comp={position={x=45,y=20,z=0}}}},
            edgesToAdd={e},edgesToRemove={101}}
        J.into(p,{}, {1})
        local c=p.streetProposal.nodeConfigsToAdd[1].comp
        assert(c.laneConnections[1].segment0==-2)
        assert(c.crosswalks[1]==-2 and c.crosswalks[2]==103)
        assert(c.trafficLightConfig.states[1].lockedLanes[2]==2)
        assert(c.trafficLightConfig.states[1].duration==12.375)
        -- Adding a second possible replacement must refuse the whole proposal.
        p.streetProposal.edgesToAdd[2]={entity=-3,type=0,comp=e.comp}
        local ok,why=pcall(J.into,p,{}, {1})
        assert(not ok and tostring(why):find('ambiguous replacement',1,true))
    "#,
    )
    .exec()
    .unwrap();
}

/// A small world for the lanes, over the stand-in engine state: two edges,
/// two constructions, a line, two vehicles, a player, a town and people.
const FAKE_WORLD: &str = r#"
local CT = { BASE_EDGE = 1, CONSTRUCTION = 2, LINE = 3, TRANSPORT_VEHICLE = 4, PLAYER = 5,
             ACCOUNT = 6, TOWN = 7, SIM_PERSON = 8, MOVE_PATH = 9 }
WORLD = {
    [CT.BASE_EDGE] = {
        [101] = { position0 = { x = 0, y = 0, z = 0 }, position1 = { x = 100.04, y = 0, z = 1 },
                  roadTemplate = 'street/country.lua', laneConfigs = {} },
        [102] = { position0 = { x = 100, y = 0, z = 1 }, position1 = { x = 100, y = 80, z = 2 },
                  roadTemplate = 'street/country.lua', laneConfigs = {} },
    },
    [CT.CONSTRUCTION] = {
        [201] = { fileName = 'depot/road_depot.con', transf = { 1,0,0,0, 0,1,0,0, 0,0,1,0, 600,0,0.45,1 } },
        [202] = { fileName = 'station/bus_stop.con', transf = { 1,0,0,0, 0,1,0,0, 0,0,1,0, 40,8,0,1 } },
    },
    [CT.LINE] = { [301] = { stops = { {}, {} } } },
    [CT.TRANSPORT_VEHICLE] = { [401] = { state = 1, stopIndex = 0 }, [402] = { state = 2, stopIndex = 1 } },
    -- The simulation's path state, and the state as a frame began, which
    -- is each game's own.
    [CT.MOVE_PATH] = {
        [401] = { dyn = { pathPos = { edgeIndex = 3, pos = 10.2 }, speed = 5 },
                  dyn0 = { pathPos = { edgeIndex = 3, pos = 9.7 }, speed = 5 } },
        [402] = { dyn = { pathPos = { edgeIndex = 0, pos = 0 }, speed = 0 } },
    },
    [CT.PLAYER] = { [25] = true },
    [CT.ACCOUNT] = { [25] = { balance = 1234567 } },
    [CT.TOWN] = { [7] = true },
    [CT.SIM_PERSON] = { [801] = true, [802] = true, [803] = true },
}
REVERSED = false
api.type.ComponentType = CT
api.engine.getEntitiesWithComponent = function(kind)
    -- As the game: some components cannot be listed.
    if kind == CT.BASE_EDGE or kind == CT.LINE or kind == CT.PLAYER then
        error('Cannot loop over this component type')
    end
    local list = {}
    for e in pairs(WORLD[kind] or {}) do list[#list + 1] = e end
    table.sort(list, function(a, b) if REVERSED then return a > b end return a < b end)
    return list
end
api.engine.getComponent = function(e, kind)
    local c = (WORLD[kind] or {})[e]
    if c == true then return {} end
    return c
end
local function sorted(kind)
    local list = {}
    for e in pairs(WORLD[kind]) do list[#list + 1] = e end
    table.sort(list, function(a, b) if REVERSED then return a > b end return a < b end)
    return list
end
api.engine.system = {
    townBuildingSystem = { getTown2BuildingMap = function()
        return { [7] = { 901, 902, 903 } }
    end },
    -- Each edge under both its nodes, as the street system lists them.
    streetSystem = { getNode2StreetEdgeMap = function() return {} end,
        getNode2TrackEdgeMap = function() return {} end, getNode2SegmentMap = function()
        local edges = sorted(CT.BASE_EDGE)
        return { [11] = { edges[1] }, [12] = edges, [13] = { edges[#edges] } }
    end },
    lineSystem = { getLines = function() return sorted(CT.LINE) end },
}
"#;

/// The lanes the mod reads in the stand-in world.
fn read_lanes(lua: &Lua) -> Vec<(u16, String)> {
    let lanes: Table = lua
        .load("return ug_require('tpf3mp_1::/scripts/tpf3mp/lanes.lua').read(api)")
        .eval()
        .unwrap();
    let mut out: Vec<(u16, String)> = lanes
        .pairs::<u16, String>()
        .map(|pair| pair.unwrap())
        .collect();
    out.sort();
    out
}

#[test]
fn lanes_sum_up_the_world_part_by_part() {
    let (lua, _) = engine();
    lua.load(FAKE_WORLD).exec().unwrap();
    let lanes = read_lanes(&lua);
    assert_eq!(
        lanes.iter().map(|(lane, _)| *lane).collect::<Vec<_>>(),
        [0, 1, 2, 3, 4, 5, 6]
    );
    assert!(lanes.iter().all(|(_, text)| text != "err"), "{lanes:?}");
    assert!(lanes[0].1.starts_with("2:"), "two edges: {}", lanes[0].1);
    assert_eq!(lanes[6].1, "3", "three people");
    // The order the engine lists entities in changes nothing.
    lua.load("REVERSED = true").exec().unwrap();
    assert_eq!(read_lanes(&lua), lanes);
    // The state a frame began with, each game's own, changes nothing; a
    // vehicle 2 cm on along its path changes the vehicles' lane alone.
    lua.load("WORLD[9][401].dyn0.pathPos.pos = 10.1")
        .exec()
        .unwrap();
    assert_eq!(read_lanes(&lua), lanes);
    lua.load("WORLD[9][401].dyn.pathPos.pos = 10.22")
        .exec()
        .unwrap();
    let moved = read_lanes(&lua);
    for (before, after) in lanes.iter().zip(&moved) {
        assert_eq!(before.0 == 3, before.1 != after.1, "lane {}", before.0);
    }
    // Money spent changes the economy's lane.
    lua.load("WORLD[6][25].balance = 1234000").exec().unwrap();
    assert_ne!(read_lanes(&lua)[4], moved[4]);
    // A lane the engine cannot read is err, on every game alike, and says
    // why; the others still count.
    let (text, failed): (String, Vec<String>) = lua
        .load(
            "api.engine.system.townBuildingSystem = nil \
             local lanes, failed = ug_require('tpf3mp_1::/scripts/tpf3mp/lanes.lua').read(api) \
             return lanes[5], failed",
        )
        .eval()
        .unwrap();
    assert_eq!(text, "err");
    assert_eq!(failed.len(), 1);
    assert!(failed[0].starts_with("5: "), "{failed:?}");
}

#[test]
fn the_game_script_hands_the_lanes_over_at_a_checkpoint_only() {
    let (lua, _script) = engine();
    lua.load(FAKE_WORLD).exec().unwrap();
    // No checkpoint: nothing read.
    lua.load("UPDATE({}, STATE, 0.2)").exec().unwrap();
    let none: bool = lua.load("return HOOK.lanes == nil").eval().unwrap();
    assert!(none);
    // The last update of a batch that ends at a checkpoint: the lanes go
    // to the hook, the same the lanes module reads.
    lua.load("HOOK.checkpoint = true UPDATE({}, STATE, 0.2)")
        .exec()
        .unwrap();
    let handed: Table = lua.load("return HOOK.lanes").eval().unwrap();
    let mut handed: Vec<(u16, String)> = handed
        .pairs::<u16, String>()
        .map(|pair| pair.unwrap())
        .collect();
    handed.sort();
    assert_eq!(handed, read_lanes(&lua));
}

/// A simulation state without debug.getinfo cannot tell a personal mod's
/// command from the game's own: it notes so for the hook, which loads the
/// room's worlds without this player's personal mods from then on, and
/// says why in the log.
#[test]
fn a_simulation_state_that_cannot_guard_personal_mods_has_them_left_out() {
    let (lua, _script) = engine();
    lua.load(FAKE_WORLD).exec().unwrap();
    lua.load(
        "tpf3mp_native.personal = function() return 'celmi_timetables' end          debug = nil UPDATE({}, STATE, 0.2)",
    )
    .exec()
    .unwrap();
    let (noted, logged): (Option<String>, String) = lua
        .load(
            "return HOOK.notes and HOOK.notes['personal-mods-unguarded'],              table.concat(HOOK.logged, '|')",
        )
        .eval()
        .unwrap();
    assert_eq!(noted.as_deref(), Some("1"));
    assert!(
        logged.contains("personal mods are left out of the room's worlds"),
        "{logged}"
    );
}

/// A game of the room with the stand-in world: its registry begun at the
/// room's first update, the engine listing entities in its own order.
fn dumping_game(reversed: bool) -> Lua {
    let (lua, _script) = engine();
    lua.load(FAKE_WORLD).exec().unwrap();
    lua.load(format!(
        "REVERSED = {reversed} HOOK.room = true UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap();
    lua
}

/// The lines the game script hands the hook at a checkpoint whose lanes
/// the hook wants dumped.
fn dump_at_checkpoint(lua: &Lua, step: u64, lanes: &str) -> Vec<String> {
    lua.load(format!(
        "HOOK.dumped = {{}} HOOK.checkpoint = true HOOK.dump = {{ step = {step}, lanes = {{ {lanes} }} }} \
         UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap();
    lua.load("return HOOK.dumped").eval().unwrap()
}

#[test]
fn a_lane_dump_is_the_lanes_text_entry_by_entry_keyed_and_in_the_same_order_on_every_game() {
    let a = dumping_game(false);
    let b = dumping_game(true);
    let dump_a = dump_at_checkpoint(&a, 300, "3, 0, 1, 2, 4, 5, 6");
    let dump_b = dump_at_checkpoint(&b, 300, "3, 0, 1, 2, 4, 5, 6");
    assert_eq!(
        dump_a, dump_b,
        "the same world dumps the same lines, whatever order the engine lists it in"
    );
    // Vehicles by their registry ids, with the raw values the lane rounds.
    let vehicles: Vec<&String> = dump_a
        .iter()
        .filter(|l| l.starts_with("lane 3 step 300 "))
        .collect();
    assert_eq!(
        vehicles,
        [
            "lane 3 step 300 vehicle-0 state=1 stop=0 line=nil edge=3 pos=10.199999999999999 speed=5 \
             entity=401 row=1:0:3@10.20 v5.00",
            "lane 3 step 300 vehicle-1 state=2 stop=1 line=nil edge=0 pos=0 speed=0 entity=402 \
             row=2:1:0@0.00 v0.00",
            &format!("lane 3 step 300 summary {}", read_lanes(&a)[3].1),
        ],
        "keyed, full precision, then the text the hook hashes"
    );
    // Every lane ends with the text lanes.read reads for it.
    for (lane, text) in read_lanes(&a) {
        let summary = format!("lane {lane} step 300 summary {text}");
        assert!(dump_a.contains(&summary), "{summary} in {dump_a:#?}");
    }
    assert!(
        dump_a.contains(&"lane 5 step 300 town-0 buildings=3 entity=7 row=7:3".to_owned()),
        "{dump_a:#?}"
    );
    assert!(
        dump_a.iter().any(|l| l.starts_with(
            "lane 0 step 300 row:0,0,0>100,0,1:street/country.lua|lanes: p0=0,0,0 \
                                   p1=100.04000000000001,0,1"
        )),
        "an edge by its row, with its raw ends: {dump_a:#?}"
    );
    // Hashing changes nothing: the lanes are what they were.
    assert_eq!(read_lanes(&a), read_lanes(&b));

    // A vehicle a millimetre on in one game: the lanes still agree (1 cm),
    // the dumps name it and how.
    b.load("WORLD[9][402].dyn.pathPos.pos = 0.001")
        .exec()
        .unwrap();
    assert_eq!(read_lanes(&a), read_lanes(&b));
    let dump_a = dump_at_checkpoint(&a, 350, "3");
    let dump_b = dump_at_checkpoint(&b, 350, "3");
    let differ: Vec<(&String, &String)> =
        dump_a.iter().zip(&dump_b).filter(|(x, y)| x != y).collect();
    assert_eq!(differ.len(), 1, "{differ:#?}");
    assert!(differ[0].0.starts_with("lane 3 step 350 vehicle-1 "));
    assert!(differ[0].1.contains(" pos=0.001 "), "{}", differ[0].1);

    // No dump asked: nothing handed over at a checkpoint.
    a.load("HOOK.dumped = {} HOOK.checkpoint = true UPDATE({}, STATE, 0.2)")
        .exec()
        .unwrap();
    let none: Vec<String> = a.load("return HOOK.dumped").eval().unwrap();
    assert!(none.is_empty());
}

#[test]
fn a_lane_that_cannot_be_read_dumps_why_and_a_hook_without_dumps_is_left_alone() {
    let lua = dumping_game(false);
    lua.load("api.engine.system.townBuildingSystem = nil")
        .exec()
        .unwrap();
    let dump = dump_at_checkpoint(&lua, 50, "5");
    assert_eq!(dump.len(), 1, "{dump:?}");
    assert!(dump[0].starts_with("lane 5 step 50 err "), "{dump:?}");
    // An older hook has neither function: the lanes still go over.
    lua.load("tpf3mp_native.dump, tpf3mp_native.dumped = nil, nil")
        .exec()
        .unwrap();
    let dump = dump_at_checkpoint(&lua, 100, "5");
    assert!(dump.is_empty());
    let handed: bool = lua.load("return HOOK.lanes ~= nil").eval().unwrap();
    assert!(handed);
}

const DEPOT: &str = "{ BuildConstruction = { \
    file = 'depot/road_depot_era_a.con', \
    transform = { basis = { 0, 1, 0, -1, 0, 0, 0, 0, 1 }, origin = { x = 1250.5, y = -300, z = 20 } }, \
    params = { { key = 'seed', value = { Int = 1234 } }, \
               { key = 'modules[3801].name', value = { Text = 'depot/module.module' } }, \
               { key = 'paramX', value = { Fixed = 2.5 } }, \
               { key = 'lit', value = { Bool = true } } }, \
    name = 'Depot' } }";

#[test]
fn the_game_script_applies_the_rooms_actions_as_the_players_own_builds() {
    let (lua, _script) = engine();
    // No action ordered: nothing sent, and nothing for postUpdate.
    let work: mlua::Value = lua.load("return UPDATE({}, STATE, 0.2)").eval().unwrap();
    assert!(work.is_nil());
    assert_eq!(lua.load("return #SENT").eval::<usize>().unwrap(), 0);
    // update only takes the actions; the world changes in postUpdate, as
    // the game's own scripts change it.
    lua.load(format!(
        "HOOK.batch = {{ {DEPOT} }} WORK = SCRIPT.update({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap();
    assert_eq!(lua.load("return #SENT").eval::<usize>().unwrap(), 0);
    lua.load("SCRIPT.postUpdate({}, STATE, 0.2, WORK)")
        .exec()
        .unwrap();
    assert_eq!(lua.load("return #SENT").eval::<usize>().unwrap(), 1);
    lua.load("SENT = {}").exec().unwrap();
    lua.load(format!(
        "HOOK.batch = {{ {DEPOT} }} UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap();
    let built: String = lua
        .load(
            "local c = SENT[1] local e = c.proposal.constructionsToAdd[1]
             local t = e.transf
             return table.concat({ e.fileName, e.name, e.playerEntity,
                 t[1][1], t[1][2], t[2][1], t[4][1], t[4][2], t[4][3], t[4][4],
                 e.params.seed, e.params.modules[3801].name, e.params.paramX, tostring(e.params.lit),
                 tostring(c.ignoreErrors), tostring(c.playerInitiated), tostring(c.context.player),                  tostring(c.context.gatherBuildings), tostring(c.context.gatherFields) }, '|')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        built,
        "depot/road_depot_era_a.con|Depot|25|0|1|-1|1250.5|-300|20|1|1234|depot/module.module|2.5|true|true|true|25|true|true"
    );
    // Subscribed to its console event, linked once.
    assert!(
        lua.load(
            "return STATE.subscribed.command and STATE.subscribed['builder.proposalCreate'] \
                    and STATE.subscribed['builder.proposalPrepareForApply']"
        )
        .eval::<bool>()
        .unwrap()
    );
    assert_eq!(
        lua.load("return table.concat(HOOK.logged, '|')")
            .eval::<String>()
            .unwrap(),
        "the game script is linked"
    );
}

#[test]
fn the_game_script_takes_and_repays_loans_through_the_loan_scripts_events() {
    let (lua, _script) = engine();
    lua.load(format!(
        "HOOK.batch = {{ {{ Loan = {{ Take = {{ next = {OFFER}, offer = {OFFER} }} }} }}, \
                         {{ Loan = {{ Repay = {{ loan = {OFFER} }} }} }} }} \
         UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap();
    let events: String = lua
        .load(
            "local out = {} \
             for _, c in ipairs(SENT) do \
                 local e = c.event \
                 local p1 = e.param[1] and e.param[1].amount or 'nil' \
                 out[#out + 1] = e.src .. '|' .. e.id .. '|' .. e.name .. '|' .. tostring(p1) \
                     .. '|' .. e.param[2].amount .. '|' .. e.param[2].percentage .. '|' .. e.param[2].type \
             end \
             return table.concat(out, ' ')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        events,
        "|Loan|Obtain|5000000|5000000|0.03|Small |Loan|Repay|nil|5000000|0.03|Small"
    );
}

/// A construction tool's proposal, as build 40408 hands it to game scripts:
/// a maintenance building placed by the construction tool.
const CONSTRUCTION_PROPOSAL: &str = "{ \
    proposal = { addedNodes = {}, addedSegments = {}, removedNodes = {}, removedSegments = {}, \
                 edgeObjectsToAdd = {} }, \
    toRemove = {}, \
    toAdd = { { fileName = '::/depots/road/road_maint_station.con', \
                name = 'Okehampton Maintenance Building', playerEntity = 3869, \
                transf = { 0.707107, -0.707107, 0, 0, 0.707107, 0.707107, 0, 0, 0, 0, 1, 0, \
                           -421.93572998047, -252.93925476074, 0.50797754526138, 1 }, \
                params = { modules = { [3801] = { name = 'depot/module.module', variant = 2 } }, \
                           year = 1990, seed = 0, scale = 1.5, lit = true } } } }";

#[test]
fn a_construction_the_tool_placed_becomes_the_rooms_action() {
    let (lua, _script) = engine();
    let (file, name, origin_x, seed, module, ok): (String, String, f64, i64, String, bool) = lua
        .load(format!(
            "local capture = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua') \
             local action = capture.construction({CONSTRUCTION_PROPOSAL}) \
             local b = action.BuildConstruction \
             local seed, module \
             for _, p in ipairs(b.params) do \
                 if p.key == 'seed' then seed = p.value.Int end \
                 if p.key == 'modules[3801].name' then module = p.value.Text end \
             end \
             return b.file, b.name, b.transform.origin.x, seed, module, schema_check(action)"
        ))
        .eval()
        .unwrap();
    assert_eq!(file, "::/depots/road/road_maint_station.con");
    assert_eq!(name, "Okehampton Maintenance Building");
    assert!((origin_x + 421.935_729_980_47).abs() < 1e-9);
    assert_eq!(seed, 0);
    assert_eq!(module, "depot/module.module");
    assert!(ok, "the schema takes it");
    // What the room cannot carry yet says why.
    let refusals: Vec<String> = lua
        .load(format!(
            "local capture = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua') \
             local out = {{}} \
             local function why(p) local _, r = capture.construction(p) out[#out + 1] = r end \
             local two = {CONSTRUCTION_PROPOSAL} two.toAdd[2] = two.toAdd[1] \
             why(two) \
             local unnamed = {CONSTRUCTION_PROPOSAL} unnamed.toAdd[1].name = '' \
             why(unnamed) \
             local odd = {CONSTRUCTION_PROPOSAL} odd.toAdd[1].params.f = print \
             why(odd) \
             return out"
        ))
        .eval()
        .unwrap();
    assert_eq!(refusals[0], "more than one construction at once");
    assert_eq!(refusals[1], "an unnamed construction");
    assert!(
        refusals[2].contains("parameter f is a function"),
        "{}",
        refusals[2]
    );
    // A construction whose tool built no streets carries none.
    let (connection, ok): (bool, bool) = lua
        .load(format!(
            "local capture = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua') \
             local action = capture.construction({CONSTRUCTION_PROPOSAL}) \
             return action.BuildConstruction.connection ~= nil, schema_check(action)"
        ))
        .eval()
        .unwrap();
    assert!(!connection);
    assert!(ok);
    // Town buildings in the way go, as the replay clears them again, and
    // are no construction replaced; a construction the room cannot name
    // does not travel.
    let (cleared, replaced): (bool, String) = lua
        .load(format!(
            "api.type.ComponentType = {{ CONSTRUCTION = 2 }} \
             local CONSTRUCTIONS = {{ [5618] = {{ townBuildings = {{ 9001 }} }}, [77] = {{ townBuildings = {{}} }} }} \
             api.engine.getComponent = function(e, kind) return CONSTRUCTIONS[e] end \
             local capture = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua') \
             local town = {CONSTRUCTION_PROPOSAL} town.toRemove = {{ 5618 }} \
             local own = {CONSTRUCTION_PROPOSAL} own.toRemove = {{ 5618, 77 }} \
             local _, why = capture.construction(own) \
             local action = capture.construction(town) \
             return action ~= nil and action.BuildConstruction.replaces == nil, why"
        ))
        .eval()
        .unwrap();
    assert!(cleared);
    assert_eq!(replaced, "a construction the room cannot name");
}

/// The player's bus station 77 as the game has it, standing at (80, 0, 0):
/// its own entrance edge 6000 and node 6001, and no town building. In a
/// world of the stand-in engine state, whose constructions any test can
/// add to.
const FAKE_STATION: &str = r#"
api.type.ComponentType.CONSTRUCTION = 2
CONSTRUCTIONS = { [77] = { fileName = '::/stations/street/modular_street_station/modular_terminal.con',
    transf = { 1,0,0,0, 0,1,0,0, 0,0,1,0, 80,0,0,1 }, townBuildings = {},
    frozenEdges = { 6000 }, frozenNodes = { 6001 }, params = { seed = 7, length = 2 } } }
local get = api.engine.getComponent
api.engine.getComponent = function(e, kind)
    if kind == 2 then return CONSTRUCTIONS[e] end
    if get then return get(e, kind) end
end
api.engine.getEntitiesWithComponent = function(kind)
    local l = {}
    if kind == 2 then for e in pairs(CONSTRUCTIONS) do l[#l + 1] = e end end
    table.sort(l)
    return l
end
api.engine.util = api.engine.util or {}
api.engine.util.getEntityName = function(e)
    if e == 77 then return 'Okehampton Station' end
end
"#;

/// The station 77 of FAKE_STATION given a longer platform, as an edit of its
/// modules or parameters proposes it (docs/BUILDING.md: the old construction
/// in `toRemove`, the new one in `toAdd`, same file, new parameters): its
/// own entrance removed and made again.
const EDIT_PROPOSAL: &str = "{ toRemove = { 77 }, \
    toAdd = { { fileName = '::/stations/street/modular_street_station/modular_terminal.con', \
                name = 'Okehampton Station', playerEntity = 25, \
                transf = { 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 80, 0, 0, 1 }, \
                params = { seed = 7, length = 3, modules = { [12] = { name = 'station/platform.module' } } } } }, \
    proposal = { addedNodes = { { entity = -1, comp = { position = { x = 70, y = 0, z = 0 } } } }, \
                 addedSegments = { { entity = -2, type = 0, comp = { node0 = -1, node1 = 7 } } }, \
                 removedSegments = { { entity = 6000, type = 0, comp = { node0 = 6001, node1 = 7 } } }, \
                 removedNodes = { { entity = 6001, comp = { position = { x = 70, y = 0, z = 0 } } } }, \
                 edgeObjectsToAdd = {} } }";

#[test]
fn an_edit_of_a_station_travels_with_the_station_it_replaces() {
    let (lua, _script) = engine();
    lua.load(FAKE_STATION).exec().unwrap();
    let (carried, ok): (String, bool) = lua
        .load(format!(
            "local capture = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua') \
             local action, why = capture.construction({EDIT_PROPOSAL}) \
             if not action then error(why) end \
             local b = action.BuildConstruction \
             local length \
             for _, p in ipairs(b.params) do if p.key == 'length' then length = p.value.Int end end \
             return table.concat({{ b.replaces.file, b.replaces.at.x, b.replaces.at.y, b.replaces.at.z, \
                 b.file, b.name, length, tostring(b.connection) }}, '|'), schema_check(action)"
        ))
        .eval()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        carried,
        "::/stations/street/modular_street_station/modular_terminal.con|80|0|0\
         |::/stations/street/modular_street_station/modular_terminal.con|Okehampton Station|3|nil",
        "the station it replaces by its file and place, the new parameters, no connection"
    );
    assert!(ok, "the schema takes it");
    // An edit whose proposal leaves the name out keeps the station's; the
    // bulldozer's proposal of the same shape is the same edit.
    let (named, bulldozed): (String, String) = lua
        .load(format!(
            "local capture = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua') \
             local unnamed = {EDIT_PROPOSAL} unnamed.toAdd[1].name = '' \
             local b = capture.bulldoze({EDIT_PROPOSAL}) \
             return capture.construction(unnamed).BuildConstruction.name, b.BuildConstruction.replaces.file"
        ))
        .eval()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(named, "Okehampton Station");
    assert_eq!(
        bulldozed,
        "::/stations/street/modular_street_station/modular_terminal.con"
    );
    // What an edit the room cannot carry says.
    let refusals: Vec<String> = lua
        .load(format!(
            "local capture = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua') \
             CONSTRUCTIONS[78] = {{ fileName = 'depot/road_depot.con', transf = {{ 1,0,0,0, 0,1,0,0, 0,0,1,0, 9,9,0,1 }}, \
                 townBuildings = {{}} }} \
             CONSTRUCTIONS[79] = {{ townBuildings = {{}} }} \
             local out = {{}} \
             local function why(p) local _, r = capture.construction(p) out[#out + 1] = r end \
             local two = {EDIT_PROPOSAL} two.toRemove = {{ 77, 78 }} why(two) \
             local nameless = {EDIT_PROPOSAL} nameless.toRemove = {{ 79 }} why(nameless) \
             local nothing = {EDIT_PROPOSAL} nothing.toRemove = {{ 80 }} why(nothing) \
             local road = {EDIT_PROPOSAL} \
             road.proposal.removedSegments[2] = {{ entity = 100, type = 0, comp = {{ node0 = 8, node1 = 9 }} }} \
             why(road) \
             CONSTRUCTIONS[5618] = {{ townBuildings = {{ 9001 }} }} \
             local town = {EDIT_PROPOSAL} town.toRemove = {{ 5618 }} town.proposal.removedSegments = {{}} \
             town.proposal.removedNodes = {{}} \
             local _, b = capture.bulldoze(town) \
             out[#out + 1] = b \
             return out"
        ))
        .eval()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        refusals,
        [
            "a construction that replaces more than one",
            "a construction the room cannot name",
            "removing something that is no construction",
            "a construction edit that changes the streets around it",
            "a bulldozer proposal that builds"
        ]
    );
}

#[test]
fn station_edits_include_unfrozen_track_ends_but_not_external_connections() {
    let (lua, _script) = engine();
    lua.load(FAKE_STATION).exec().unwrap();
    // Steam 40408, 2026-10-01: the two-track station's edit removes 50
    // nodes and 48 edges, but only 46 nodes are frozen. Each of its four
    // unfrozen track ends has one incident edge, frozen in the station.
    // Keep only that boundary here; native module capture supplies IDs,
    // without the removed nodes' or edges' components.
    lua.load(
        "CONSTRUCTIONS[77].frozenNodes = { 7522, 7846, 7848, 7871 } \
         CONSTRUCTIONS[77].frozenEdges = { 7873, 7896, 7897, 7920 } \
         INCIDENT = { [7824] = {7873}, [7847] = {7896}, [7849] = {7897}, [7872] = {7920} } \
         api.engine.system.streetSystem = api.engine.system.streetSystem or {} \
         api.engine.system.streetSystem.getNodeSegments = function(n) return INCIDENT[n] end",
    )
    .exec()
    .unwrap();
    let outcomes: Vec<bool> = lua
        .load(format!(
            "local capture = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua') \
             local p = {EDIT_PROPOSAL} \
             p.proposal.removedSegments = {{ {{entity=7873}}, {{entity=7896}}, {{entity=7897}}, {{entity=7920}} }} \
             p.proposal.removedNodes = {{ {{entity=7824}}, {{entity=7847}}, {{entity=7849}}, {{entity=7872}} }} \
             local out = {{}} \
             local function check() local a = capture.construction(p) out[#out+1] = a ~= nil end \
             check() \
             INCIDENT[7847] = {{7896, 100}} check() \
             INCIDENT[7847] = {{100}} check() \
             INCIDENT[7847] = {{}} check() \
             INCIDENT[7847] = nil check() \
             INCIDENT[7847] = {{7896}} \
             table.remove(p.proposal.removedSegments, 2) check() \
             table.insert(p.proposal.removedSegments, 2, {{entity=7896}}) \
             api.engine.system.streetSystem.getNodeSegments = function() error('unavailable') end check() \
             api.engine.system.streetSystem.getNodeSegments = nil check() \
             return out"
        ))
        .eval()
        .unwrap();
    assert_eq!(
        outcomes,
        [true, false, false, false, false, false, false, false],
        "only endpoints attached exclusively to this construction's removed edges travel"
    );
}

#[test]
fn a_station_edit_a_click_saw_goes_to_the_room_and_unhandled_events_are_logged() {
    let (lua, _script) = engine();
    lua.load(FAKE_STATION).exec().unwrap();
    lua.load(format!(
        "HOOK.room = true HOOK.clicks = 0 \
         SCRIPT.guiUpdate({{}}, nil, nil) \
         R = SCRIPT.guiHandleEvent({{}}, nil, nil, '', 'constructionBuilder', 'builder.proposalCreate', \
             {{ {EDIT_PROPOSAL} }}) \
         HOOK.clicks = 1 SCRIPT.guiUpdate({{}}, nil, nil) \
         R2 = SCRIPT.guiHandleEvent({{}}, nil, nil, '', 'moduleBuilder', 'builder.proposalCreate', \
             {{ {EDIT_PROPOSAL} }}) \
         HOOK.clicks = 2 SCRIPT.guiUpdate({{}}, nil, nil)"
    ))
    .exec()
    .unwrap_or_else(|error| panic!("{error}\n{}", log(&lua)));
    let (refused, handed, file): (bool, usize, String) = lua
        .load(
            "return R ~= nil or R2 ~= nil, #HOOK.commands, \
                 HOOK.commands[2].BuildConstruction.replaces.file",
        )
        .eval()
        .unwrap();
    assert!(!refused, "neither tool is told no");
    assert_eq!(handed, 2, "each click's edit went to the room");
    assert_eq!(
        file,
        "::/stations/street/modular_street_station/modular_terminal.con"
    );
    // A click with no proposal before it, as the module editor's on build
    // 40408, is stopped and says so.
    lua.load("HOOK.clicks = 3 SCRIPT.guiUpdate({}, nil, nil)")
        .exec()
        .unwrap();
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert!(
        logged.iter().any(|l| l.starts_with(
            "stopped a build the room cannot carry: no proposal seen (a tool that tells game scripts nothing"
        )),
        "{logged:?}"
    );
    // Events the script does not handle, in the room's game: each id and
    // name once, a few dozen at most; outside it, none.
    lua.load(
        "SCRIPT.guiHandleEvent({}, nil, nil, '', 'someWindow', 'select', {}) \
         SCRIPT.guiHandleEvent({}, nil, nil, '', 'someWindow', 'select', {}) \
         SCRIPT.guiHandleEvent({}, nil, nil, '', 'moduleThing', 'builder.proposalCreate', {}) \
         for i = 1, 60 do SCRIPT.guiHandleEvent({}, nil, nil, '', 'window' .. i, 'idAdded', {}) end \
         HOOK.room = false \
         SCRIPT.guiHandleEvent({}, nil, nil, '', 'elsewhere', 'select', {})",
    )
    .exec()
    .unwrap();
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    let unhandled: Vec<&String> = logged
        .iter()
        .filter(|l| l.starts_with("an event the mod does not handle: "))
        .collect();
    assert_eq!(unhandled.len(), 40, "{unhandled:?}");
    assert_eq!(
        unhandled[0],
        "an event the mod does not handle: id someWindow, name select"
    );
    assert_eq!(
        unhandled[1],
        "an event the mod does not handle: id moduleThing, name builder.proposalCreate"
    );
    assert!(!logged.iter().any(|l| l.contains("elsewhere")));
}

#[test]
fn a_module_editor_click_goes_to_the_room_as_the_hook_read_it() {
    let (lua, _script) = engine();
    lua.load(FAKE_STATION).exec().unwrap();
    // A construction tool preview before the click, then the module
    // editor's click, whose build only the hook saw: the hook's wins.
    lua.load(format!(
        "HOOK.room = true HOOK.clicks = 0          SCRIPT.guiUpdate({{}}, nil, nil)          SCRIPT.guiHandleEvent({{}}, nil, nil, '', 'constructionBuilder', 'builder.proposalCreate',              {{ {CONSTRUCTION_PROPOSAL} }})          local edit = {EDIT_PROPOSAL} edit.toAdd[1].name = ''          HOOK.built[0] = {{ proposal = edit }}          HOOK.clicks = 1 SCRIPT.guiUpdate({{}}, nil, nil)          HOOK.built[1] = {{ why = 'the matrix does not read' }}          HOOK.clicks = 2 SCRIPT.guiUpdate({{}}, nil, nil)          local nothing = {EDIT_PROPOSAL} nothing.toRemove = {{}}          nothing.proposal.removedSegments = {{}} nothing.proposal.removedNodes = {{}}          HOOK.built[2] = {{ proposal = nothing }}          HOOK.clicks = 3 SCRIPT.guiUpdate({{}}, nil, nil)"
    ))
    .exec()
    .unwrap_or_else(|error| panic!("{error}\n{}", log(&lua)));
    let (handed, carried): (usize, String) = lua
        .load(
            "local b = HOOK.commands[1].BuildConstruction              return #HOOK.commands, table.concat({ b.file, b.name, b.replaces.file, b.replaces.at.x }, '|')",
        )
        .eval()
        .unwrap_or_else(|error| panic!("{error}\n{}", log(&lua)));
    assert_eq!(
        handed, 1,
        "the edit, not the stale preview, and nothing else"
    );
    assert_eq!(
        carried,
        "::/stations/street/modular_street_station/modular_terminal.con|Okehampton Station|::/stations/street/modular_street_station/modular_terminal.con|80"
    );
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    for line in [
        "handed the player's build to the room [module editor]",
        "stopped a build the room cannot carry: the module editor's edit did not read: the matrix does not read [module editor]",
        "stopped a build the room cannot carry: the module editor's edit: an edit that replaces no construction [module editor]",
    ] {
        assert!(logged.iter().any(|l| l == line), "{line}: {logged:?}");
    }
}

#[test]
fn every_game_replaces_the_edited_station_in_one_proposal() {
    let (lua, _script) = engine();
    lua.load(FAKE_STATION).exec().unwrap();
    // This game's station has its own entity, 910, where the action says;
    // the game's verdict is asked first, and a build replaces constructions
    // as the game would.
    lua.load(format!(
        "local capture = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua') \
         ACTION = capture.construction({EDIT_PROPOSAL}) \
         CONSTRUCTIONS[910] = CONSTRUCTIONS[77] CONSTRUCTIONS[77] = nil \
         ASKED = {{}} \
         api.engine.util.proposal = {{ makeProposalData = function(p, context) \
             ASKED[#ASKED + 1] = {{ sent = #SENT, removes = p.constructionsToRemove[1] }} \
             return {{ errorState = {{ critical = false, messages = {{}} }} }} end }} \
         local send = api.cmd.sendCommand \
         api.cmd.sendCommand = function(cmd, ...) \
             local p = cmd.proposal \
             for _, e in ipairs(p and p.constructionsToRemove or {{}}) do CONSTRUCTIONS[e] = nil end \
             local c = p and p.constructionsToAdd and p.constructionsToAdd[1] \
             if c then CONSTRUCTIONS[911] = {{ fileName = c.fileName, \
                 transf = {{ 1,0,0,0, 0,1,0,0, 0,0,1,0, c.transf[4][1], c.transf[4][2], c.transf[4][3], 1 }} }} end \
             return send(cmd, ...) \
         end \
         HOOK.batch = {{ ACTION }} UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap_or_else(|error| panic!("{error}\n{}", log(&lua)));
    let built: String = lua
        .load(
            "local c = SENT[1] local p = c.proposal local e = p.constructionsToAdd[1] \
             return table.concat({ #SENT, #ASKED, ASKED[1].sent, ASKED[1].removes, \
                 #p.constructionsToRemove, p.constructionsToRemove[1], p.old2new[910], \
                 e.fileName, e.name, e.params.length, e.params.modules[12].name, e.playerEntity, \
                 tostring(c.context.player), tostring(c.context.gatherBuildings), \
                 tostring(c.ignoreErrors), tostring(c.playerInitiated), \
                 tostring(HOOK.applied[1].ok), tostring(CONSTRUCTIONS[910]), tostring(CONSTRUCTIONS[911] ~= nil) }, '|')",
        )
        .eval()
        .unwrap_or_else(|error| panic!("{error}\n{}", log(&lua)));
    assert_eq!(
        built,
        "1|1|0|910|1|910|0\
         |::/stations/street/modular_street_station/modular_terminal.con|Okehampton Station|3\
         |station/platform.module|25|25|true|true|true|true|nil|true",
        "the verdict, then one proposal removing this game's station and adding the new one, \
         mapped old to new, as the player's own build"
    );
    // The next edit finds the new station where the old one stood.
    lua.load("SENT = {} HOOK.batch = { ACTION } UPDATE({}, STATE, 0.2)")
        .exec()
        .unwrap();
    let again: String = lua
        .load("return SENT[1].proposal.constructionsToRemove[1] .. '|' .. tostring(HOOK.applied[2].ok)")
        .eval()
        .unwrap_or_else(|error| panic!("{error}\n{}", log(&lua)));
    assert_eq!(again, "911|true");
    // A station that is not there is refused with why, and nothing is sent.
    lua.load("SENT = {} CONSTRUCTIONS = {} HOOK.batch = { ACTION } UPDATE({}, STATE, 0.2)")
        .exec()
        .unwrap();
    let (sent, ok, why): (usize, bool, String) = lua
        .load("return #SENT, HOOK.applied[3].ok, HOOK.applied[3].why")
        .eval()
        .unwrap();
    assert_eq!(sent, 0);
    assert!(!ok);
    assert_eq!(
        why,
        "no ::/stations/street/modular_street_station/modular_terminal.con there"
    );
}

#[test]
fn a_construction_edited_in_its_window_goes_to_the_room() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_CMD).exec().unwrap();
    lua.load(
        "api.cmd.makeWorldBuildProposalCmd = function(proposal, context, ignoreErrors, playerInitiated) \
             return { kind = 'build', proposal = proposal } end \
         api.type = { ComponentType = {} } api.engine = { util = {} }",
    )
    .exec()
    .unwrap();
    lua.load(FAKE_STATION).exec().unwrap();
    lua.load("M = mount(loadPlugin()) M.step() HOOK.room = true")
        .exec()
        .unwrap();
    // The construction menu's parameters, as it sends them: the game's
    // replacement proposal, no context, playerInitiated.
    lua.load(format!(
        "CALLED = nil \
         api.cmd.sendCommand(api.cmd.makeWorldBuildProposalCmd({EDIT_PROPOSAL}, nil, false, true), \
             function(data, ok) CALLED = ok end) \
         api.cmd.sendCommand(api.cmd.makeWorldBuildProposalCmd({CONSTRUCTION_PROPOSAL}, nil, false, true)) \
         M.step()"
    ))
    .exec()
    .unwrap_or_else(|error| panic!("{error}\n{}", log(&lua)));
    let (sent, handed, replaces): (usize, usize, String) = lua
        .load("return #SENT, #HOOK.commands, HOOK.commands[1].BuildConstruction.replaces.file")
        .eval()
        .unwrap();
    assert_eq!(sent, 0, "neither is built here");
    assert_eq!(handed, 1, "the edit went to the room");
    assert_eq!(
        replaces,
        "::/stations/street/modular_street_station/modular_terminal.con"
    );
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert!(
        logged
            .iter()
            .any(|l| l.contains("makeWorldBuildProposalCmd")
                && l.ends_with("building from this window")),
        "a build that edits nothing stays refused: {logged:?}"
    );
    // This game applied it: the window hears so.
    lua.load("HOOK.results = { { ticket = 1, ok = true } } M.step()")
        .exec()
        .unwrap();
    assert!(lua.load("return CALLED == true").eval::<bool>().unwrap());
}

/// A bus station placed by the street 8-9 of FAKE_NETWORK, as the
/// construction tool proposes it (build 40408's shape): the street rebuilt
/// through a new junction -2, and an entrance edge from the station's own
/// street node -1 to it.
const STATION_BY_ROAD: &str = "{ toRemove = {}, \
    toAdd = { { fileName = '::/stations/street/modular_street_station/modular_terminal.con', \
                name = 'Okehampton Station', playerEntity = 25, \
                transf = { 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 80, 0, 0, 1 }, \
                params = { seed = 7, length = 2 } } }, \
    proposal = { \
    addedNodes = { { entity = -1, comp = { position = { x = 70, y = 0, z = 0 } } }, \
                   { entity = -2, comp = { position = { x = 50, y = 0, z = 0 } } } }, \
    addedSegments = { \
        { entity = -3, type = 0, comp = { node0 = -1, node1 = -2, type = 0, typeIndex = -1, \
          tangent0 = { x = -20, y = 0, z = 0 }, tangent1 = { x = -20, y = 0, z = 0 }, \
          roadTemplate = '::/street/town_small.street_template', roadStyle = '' } }, \
        { entity = -4, type = 0, comp = { node0 = 8, node1 = -2, type = 0, typeIndex = -1, \
          tangent0 = { x = 0, y = 40, z = 0 }, tangent1 = { x = 0, y = 40, z = 0 }, \
          roadTemplate = '::/street/country.street_template', roadStyle = '' } }, \
        { entity = -5, type = 0, comp = { node0 = -2, node1 = 9, type = 0, typeIndex = -1, \
          tangent0 = { x = 0, y = 40, z = 0 }, tangent1 = { x = 0, y = 40, z = 0 }, \
          roadTemplate = '::/street/country.street_template', roadStyle = '' } } }, \
    removedSegments = { { entity = 100, type = 0, comp = { node0 = 8, node1 = 9 } } }, \
    removedNodes = {}, edgeObjectsToAdd = {} } }";

/// A rail station placed on open ground, as the construction tool proposes
/// it on build 40408: the station, and its own platform track as new edges
/// between new nodes, joined to nothing that exists. `{JOIN}` adds an edge
/// or not.
const RAIL_STATION_OPEN: &str = "{ toRemove = {}, \
    toAdd = { { fileName = '::/stations/rail/rail_station.con', \
                name = 'Okehampton Rail', playerEntity = 25, \
                transf = { 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 300, 0, 0, 1 }, \
                params = { seed = 7, length = 2 } } }, \
    proposal = { \
    addedNodes = { { entity = -1, comp = { position = { x = 250, y = 0, z = 0 } } }, \
                   { entity = -2, comp = { position = { x = 300, y = 0, z = 0 } } }, \
                   { entity = -3, comp = { position = { x = 350, y = 0, z = 0 } } } }, \
    addedSegments = { \
        { entity = -4, type = 1, comp = { node0 = -1, node1 = -2, type = 0, typeIndex = -1, \
          tangent0 = { x = 50, y = 0, z = 0 }, tangent1 = { x = 50, y = 0, z = 0 }, \
          roadTemplate = '::/track/standard.track_template', roadStyle = '' } }, \
        { entity = -5, type = 1, comp = { node0 = -2, node1 = -3, type = 0, typeIndex = -1, \
          tangent0 = { x = 50, y = 0, z = 0 }, tangent1 = { x = 50, y = 0, z = 0 }, \
          roadTemplate = '::/track/standard.track_template', roadStyle = '' } } {JOIN} }, \
    removedSegments = {}, removedNodes = {}, edgeObjectsToAdd = {} } }";

#[test]
fn a_rail_station_on_open_ground_leaves_its_own_track_to_the_station() {
    let (lua, _script) = engine();
    lua.load(FAKE_NETWORK).exec().unwrap();
    let open = RAIL_STATION_OPEN.replace("{JOIN}", "");
    let joined = RAIL_STATION_OPEN.replace(
        "{JOIN}",
        ", { entity = -6, type = 1, comp = { node0 = -3, node1 = 8, type = 0, typeIndex = -1, \
           tangent0 = { x = 50, y = 0, z = 0 }, tangent1 = { x = 50, y = 0, z = 0 }, \
           roadTemplate = '::/track/standard.track_template', roadStyle = '' } }",
    );
    let (alone, ok, links): (String, bool, usize) = lua
        .load(format!(
            "local capture = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua') \
             local open = capture.construction({open}) \
             local joined = capture.construction({joined}) \
             return tostring(open.BuildConstruction.connection), schema_check(open), \
                 #joined.BuildConstruction.connection.links"
        ))
        .eval()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        alone, "nil",
        "the platform track is the station's own: built beside it, it blocks it"
    );
    assert!(ok, "the schema takes it");
    assert_eq!(
        links, 3,
        "joined to an existing track, it travels as before"
    );
}

#[test]
fn a_station_by_a_road_travels_with_the_junction_that_joins_it() {
    let (lua, _script) = engine();
    lua.load(FAKE_NETWORK).exec().unwrap();
    let (carried, ok): (String, bool) = lua
        .load(format!(
            "local capture = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua') \
             ACTION = capture.construction({STATION_BY_ROAD}) \
             local c = ACTION.BuildConstruction.connection \
             local out = {{ #c.vertices, #c.links, #c.removals }} \
             for _, v in ipairs(c.vertices) do out[#out + 1] = type(v.resolve) == 'table' and v.resolve.Node or v.resolve end \
             for _, l in ipairs(c.links) do out[#out + 1] = l.from .. '>' .. l.to .. ':' .. l.kind.template end \
             out[#out + 1] = c.removals[1].ends.a.y .. ',' .. c.removals[1].ends.b.y \
             return table.concat(out, '|'), schema_check(ACTION)"
        ))
        .eval()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        carried,
        "4|3|1|New|New|Street|Street\
         |0>1:::/street/town_small.street_template\
         |2>1:::/street/country.street_template\
         |1>3:::/street/country.street_template|-40,40",
        "the entrance and the rebuilt street, every link with its kind, and the street it replaces"
    );
    assert!(ok, "the schema takes it");
    // Every game builds the station and the street rebuilt through the
    // junction in one proposal, leaving out the station's own entrance,
    // which the station makes again unsnapped; then the game's refresh of
    // the new station, which snaps its entrance onto the junction. As the
    // game: a built construction is listed, and refreshed on request.
    lua.load(
        "api.type.ComponentType.CONSTRUCTION = 2 \
         CONSTRUCTIONS = {} \
         local get = api.engine.getComponent \
         api.engine.getComponent = function(e, kind) \
             if kind == 2 then return CONSTRUCTIONS[e] end return get(e, kind) end \
         api.engine.getEntitiesWithComponent = function(kind) \
             local l = {} if kind == 2 then for e in pairs(CONSTRUCTIONS) do l[#l + 1] = e end end return l end \
         local send = api.cmd.sendCommand \
         api.cmd.sendCommand = function(cmd, ...) \
             local c = cmd.proposal and cmd.proposal.constructionsToAdd and cmd.proposal.constructionsToAdd[1] \
             if c then CONSTRUCTIONS[5000] = { fileName = c.fileName, \
                 transf = { 1,0,0,0, 0,1,0,0, 0,0,1,0, c.transf[4][1], c.transf[4][2], c.transf[4][3], 1 } } end \
             return send(cmd, ...) \
         end \
         api.engine.util.proposal = { refreshConstruction = function(e) return { refreshed = e, \
             proposal = { addedSegments = { { entity = -2, comp = { node0 = -1, node1 = 7777 } } }, \
                          removedSegments = { { entity = 6000 } } } } end } \
         HOOK.batch = { ACTION } UPDATE({}, STATE, 0.2)",
    )
    .exec()
    .unwrap();
    let built: String = lua
        .load(
            "local p = SENT[1].proposal local s = p.streetProposal \
             local out = { #SENT, p.constructionsToAdd[1].fileName, #s.nodesToAdd, #s.edgesToAdd, \
                 table.concat(s.edgesToRemove, ','), table.concat(s.nodeConfigsToRemove or {}, ',') } \
             for _, e in ipairs(s.edgesToAdd) do \
                 out[#out + 1] = e.comp.node0 .. '>' .. e.comp.node1 .. ':' .. e.comp.roadTemplate end \
             local r = SENT[2] \
             out[#out + 1] = r.proposal.refreshed .. ':' .. tostring(r.context) .. ':' .. tostring(r.ignoreErrors) \
                 .. ':' .. tostring(r.playerInitiated) \
             return table.concat(out, '|')",
        )
        .eval()
        .unwrap_or_else(|error| {
            panic!(
                "{error}\n{:?}",
                lua.load("return HOOK.logged").eval::<Vec<String>>()
            )
        });
    assert_eq!(
        built,
        "2|::/stations/street/modular_street_station/modular_terminal.con|1|2|100|8,9\
         |8>-3:::/street/country.street_template\
         |-3>9:::/street/country.street_template\
         |5000:nil:true:false",
        "the station and the rebuilt street, then its refresh, free, as no player's click"
    );
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert!(
        logged
            .iter()
            .any(|l| l == "snapping 5000 +e-2:-1>7777 -e6000"),
        "{logged:?}"
    );
}

#[test]
fn a_depot_placed_on_existing_track_leaves_all_its_internal_branches_to_the_construction() {
    for refuse_snap in [false, true] {
        let (lua, _script) = engine();
        lua.load(FAKE_NETWORK).exec().unwrap();
        lua.load(include_str!("lua/depot_snap.lua")).exec().unwrap();
        lua.load(format!(
            "local capture = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua')
             ACTION = assert(capture.construction(depot_on_track()))
             assert(schema_check(ACTION))
             assert(#ACTION.BuildConstruction.connection.links == 6)
             REFUSE_SNAP = {refuse_snap}
             HOOK.batch = {{ ACTION }} UPDATE({{}}, STATE, 0.2)"
        ))
        .exec()
        .unwrap();
        let (nodes, edges, sends, connected, applied): (usize, usize, usize, bool, bool) = lua
            .load(
                "local p = SENT[1].proposal.streetProposal
                 return #p.nodesToAdd, #p.edgesToAdd, #SENT, CONNECTED, HOOK.applied[1].ok",
            )
            .eval()
            .unwrap();
        assert_eq!(
            (nodes, edges),
            (0, 0),
            "do not duplicate the depot's own track"
        );
        assert_eq!(sends, 2, "place, then snap to the existing track");
        assert_eq!(connected, !refuse_snap);
        assert_eq!(
            applied, !refuse_snap,
            "never report a refused refresh as applied"
        );
    }
}

#[test]
fn a_station_with_a_long_entrance_keeps_the_external_junction_only() {
    let (lua, _script) = engine();
    lua.load(FAKE_NETWORK).exec().unwrap();
    lua.load(include_str!("lua/depot_snap.lua")).exec().unwrap();
    lua.load(format!(
        "local p = {STATION_BY_ROAD}
         local s = p.proposal
         s.addedNodes[#s.addedNodes+1] = {{ entity=-20, comp={{ position={{ x=90,y=0,z=0 }} }} }}
         s.addedSegments[#s.addedSegments+1] = {{ entity=-21, type=0, comp={{
             node0=-20, node1=-1, type=0, typeIndex=-1,
             tangent0={{ x=-20,y=0,z=0 }}, tangent1={{ x=-20,y=0,z=0 }},
             roadTemplate='::/street/town_small.street_template', roadStyle='' }} }}
         local capture = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua')
         ACTION = assert(capture.construction(p))
         assert(schema_check(ACTION))
         HOOK.batch = {{ ACTION }} UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap();
    let (nodes, edges, removed, first, second): (usize, usize, i64, i64, i64) = lua
        .load(
            "local s = SENT[1].proposal.streetProposal
             return #s.nodesToAdd, #s.edgesToAdd, s.edgesToRemove[1],
                 s.edgesToAdd[1].comp.node0, s.edgesToAdd[2].comp.node1",
        )
        .eval()
        .unwrap();
    assert_eq!((nodes, edges, removed, first, second), (1, 2, 100, 8, 9));
    assert!(
        lua.load("return CONNECTED and HOOK.applied[1].ok")
            .eval::<bool>()
            .unwrap()
    );
}

#[test]
fn the_build_a_click_saw_goes_to_the_room_and_other_tools_stay_refused() {
    let (lua, _script) = engine();
    let asked: Vec<String> = lua
        .load(format!(
            "HOOK.room = true HOOK.clicks = 0 \
             local out = {{}} \
             local function ask(id, proposal) \
                 local r = SCRIPT.guiHandleEvent({{}}, nil, nil, '', id, 'builder.proposalCreate', {{ proposal }}) \
                 if r == nil then return 'nil' end \
                 for text in pairs(r.errorMessages) do return text end \
             end \
             SCRIPT.guiUpdate({{}}, nil, nil) \
             local elsewhere = {CONSTRUCTION_PROPOSAL} \
             elsewhere.toAdd[1].transf[13] = 99 \
             out[#out + 1] = ask('constructionBuilder', elsewhere) \
             out[#out + 1] = ask('constructionBuilder', {CONSTRUCTION_PROPOSAL}) \
             out[#out + 1] = ask('unsupportedTool', {CONSTRUCTION_PROPOSAL}) \
             local unnamed = {CONSTRUCTION_PROPOSAL} unnamed.toAdd[1].name = '' \
             HOOK.clicks = 1 \
             out[#out + 1] = ask('constructionBuilder', unnamed) \
             return out"
        ))
        .eval()
        .unwrap();
    assert_eq!(
        asked,
        [
            "nil",
            "nil",
            "Not in multiplayer yet: building with this tool",
            "Not in multiplayer yet: an unnamed construction"
        ],
        "the construction tool builds through the room; a proposal it cannot carry says why"
    );
    // The click: the last proposal before it goes to the room.
    lua.load("SCRIPT.guiUpdate({}, nil, nil)").exec().unwrap();
    let (handed, x): (usize, f64) = lua
        .load("return #HOOK.commands, HOOK.commands[1].BuildConstruction.transform.origin.x")
        .eval()
        .unwrap();
    assert_eq!(handed, 1);
    assert!(
        (x + 421.935_729_980_47).abs() < 1e-9,
        "the last one, not the first"
    );
    // A click on a proposal it could not carry hands nothing over.
    lua.load("HOOK.clicks = 2 SCRIPT.guiUpdate({}, nil, nil)")
        .exec()
        .unwrap();
    let handed: usize = lua.load("return #HOOK.commands").eval().unwrap();
    assert_eq!(handed, 1);
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert!(
        logged.contains(
            &"handed the player's build to the room \
              [+c::/depots/road/road_maint_station.con{frozen 0n 0e}]"
                .to_owned()
        ),
        "what was handed over, for the log: {logged:?}"
    );
    assert!(
        logged.contains(
            &"stopped a build the room cannot carry: an unnamed construction \
              [+c::/depots/road/road_maint_station.con{frozen 0n 0e}]"
                .to_owned()
        ),
        "{logged:?}"
    );
    assert!(
        logged.contains(
            &"the room does not carry the unsupportedTool tool yet (?) \
              [+c::/depots/road/road_maint_station.con{frozen 0n 0e}]"
                .to_owned()
        ),
        "a tool the room does not carry logs what it proposed: {logged:?}"
    );
    // Where the hook cannot stop the player's builds, every tool is refused.
    let without: String = lua
        .load(format!(
            "HOOK.clicks = nil \
             local r = SCRIPT.guiHandleEvent({{}}, nil, nil, '', 'constructionBuilder', \
                 'builder.proposalCreate', {{ {CONSTRUCTION_PROPOSAL} }}) \
             for text in pairs(r.errorMessages) do return text end"
        ))
        .eval()
        .unwrap();
    assert_eq!(without, "Not in multiplayer yet: building with this tool");
}

#[test]
fn the_rooms_builds_are_applied_as_replays() {
    let (lua, _script) = engine();
    lua.load(format!(
        "HOOK.batch = {{ {DEPOT} }} UPDATE({{}}, STATE, 0.2) UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap();
    let replaying: Vec<bool> = lua.load("return HOOK.replaying").eval().unwrap();
    assert_eq!(
        replaying,
        [true, false],
        "on around the room's actions only"
    );
}

#[test]
fn in_the_rooms_game_the_build_tools_are_refused() {
    let (lua, _script) = engine();
    let refusals: Vec<String> = lua
        .load(
            "local out = {}
             local function ask(name)
                 local r = SCRIPT.guiHandleEvent({}, nil, nil, '', 'streetBuilder', name, {})
                 if r == nil then return 'nil' end
                 local texts = {}
                 for text in pairs(r.errorMessages or {}) do texts[#texts + 1] = text end
                 return table.concat(texts, ',')
             end
             out[#out + 1] = ask('builder.proposalCreate')
             HOOK.room = true
             out[#out + 1] = ask('builder.proposalCreate')
             out[#out + 1] = ask('builder.proposalPrepareForApply')
             out[#out + 1] = ask('builder.proposalApply')
             out[#out + 1] = ask('select')
             return out",
        )
        .eval()
        .unwrap();
    assert_eq!(
        refusals,
        [
            "nil",
            "Not in multiplayer yet: building with this tool",
            "Not in multiplayer yet: building with this tool",
            "nil",
            "nil"
        ],
        "outside the room's game nothing; in it every proposal a tool makes"
    );
}

#[test]
fn an_action_the_game_script_cannot_apply_is_logged_not_raised() {
    let (lua, _script) = engine();
    lua.load(
        "HOOK.batch = { { Terraform = {} } } UPDATE({}, STATE, 0.2) \
         REFUSE = true",
    )
    .exec()
    .unwrap();
    lua.load(format!(
        "HOOK.batch = {{ {DEPOT} }} UPDATE({{}}, STATE, 0.2) \
         REFUSE, FAILS = false, true \
         HOOK.batch = {{ {DEPOT} }} UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap();
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert_eq!(logged.len(), 4, "{logged:?}");
    // One the game ran and answered as failed.
    assert_eq!(
        logged[3],
        "action 1 of this step was not applied: the game refused it"
    );
    assert_eq!(logged[0], "the game script is linked");
    assert_eq!(
        logged[1],
        "action 1 of this step was not applied: this version of the mod does not apply Terraform yet"
    );
    // The game's own refusal, as it raised it.
    assert!(
        logged[2].starts_with("action 1 of this step was not applied: ")
            && logged[2].ends_with("the proposal collides"),
        "{}",
        logged[2]
    );
}

#[test]
fn the_console_event_hands_an_action_to_the_room() {
    let (lua, script) = engine();
    let handle: Function = script.get("handleEvent").unwrap();
    lua.globals().set("HANDLE", handle).unwrap();
    lua.load(format!(
        "HANDLE({{}}, STATE, 'console', 'tpf3mp', 'command', {DEPOT}) \
         HANDLE({{}}, STATE, 'console', 'other', 'command', {DEPOT}) \
         HANDLE({{}}, STATE, 'console', 'tpf3mp', 'command', {{ Nope = 1 }})"
    ))
    .exec()
    .unwrap();
    assert_eq!(
        lua.load("return #HOOK.commands").eval::<usize>().unwrap(),
        1,
        "only its own event, and only an action the schema takes"
    );
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert_eq!(logged[0], "the game script is linked");
    assert_eq!(logged[1], "handed a test action to the room");
    assert!(
        logged[2].starts_with("refused a test action: "),
        "{}",
        logged[2]
    );
}

/// A street network for the road tests, over the stand-in engine state: the
/// street 8-9 (edge 100) running north through (50, 0), and node 7 at the
/// origin with a street of its own (edge 101).
const FAKE_NETWORK: &str = r#"
local CT = { BASE_NODE = 11, BASE_EDGE = 12, BASE_NODE_CONFIG = 13 }
api.type.ComponentType = CT
api.type.enum = { BaseEdgeType = { NORMAL = 0, BRIDGE = 1, TUNNEL = 2 },
                  RoadType = { STREET = 0, TRACK = 1 }, TrafficLightPreference = { AUTO=0, YES=1, NO=2 } }
api.type.Vec3f = { new = function(x, y, z) return { x = x, y = y, z = z } end }
api.type.NodeAndEntity = { new = function() return { comp = {} } end }
api.type.SegmentAndEntity = { new = function() return { comp = {} } end }
api.type.SimpleProposal.new = function() return { constructionsToAdd = {}, streetProposal = {} } end
api.type.BaseNodeConfig = { new = function() return { laneConnections={}, crosswalks={},
    trafficLightPreference=0, trafficLightConfig={ states={}, trafficLightType=-1 },
    doubleSlipSwitch=false, userModifiedTrafficLightStates=false } end }
api.type.BaseNodeLaneConnectionAndEntity = { new = function() return {} end }
api.type.LaneConnection = { new = function() return {} end }
api.type.TrafficLightState = { new = function() return {} end }
local TEMPLATES = { ['::/street/town_small.street_template'] = 4, ['::/street/country.street_template'] = 5 }
api.res = {
    streetTemplateRep = {
        find = function(name) return TEMPLATES[name] or -1 end,
        get = function(id)
            if id == 4 then return { laneConfigs = { 'town lanes' }, streetStyle = '::/style/town.street_style' } end
            if id == 5 then return { laneConfigs = { 'country lanes' }, streetStyle = '::/style/country.street_style' } end
        end,
    },
    bridgeTypeRep = {
        find = function(name) if name == '::/bridge/stone.lua' then return 3 end return -1 end,
        getName = function(id) if id == 3 then return '::/bridge/stone.lua' end end,
    },
    tunnelTypeRep = { find = function() return -1 end, getName = function() end },
}
NODES = { [7] = { x = 0, y = 0, z = 0 }, [8] = { x = 50, y = -40, z = 0 }, [9] = { x = 50, y = 40, z = 0 },
          [10] = { x = -60, y = 0, z = 0 } }
EDGES = {
    [100] = { node0 = 8, node1 = 9, tangent0 = { x = 0, y = 80, z = 0 }, tangent1 = { x = 0, y = 80, z = 0 },
              objects = {}, roadTemplate = '::/street/country.street_template', laneConfigs = { 'country lanes' } },
    [101] = { node0 = 10, node1 = 7, tangent0 = { x = 60, y = 0, z = 0 }, tangent1 = { x = 60, y = 0, z = 0 },
              objects = {}, roadTemplate = '::/street/town_small.street_template' },
}
STREETS = { [7] = { 101 }, [8] = { 100 }, [9] = { 100 }, [10] = { 101 } }
-- The nodes with a lane configuration.
CONFIGS = { [8] = true, [9] = true, [11] = true }
api.engine.getComponent = function(id, kind)
    if kind == CT.BASE_NODE and NODES[id] then return { position = NODES[id] } end
    if kind == CT.BASE_NODE_CONFIG and CONFIGS[id] then return type(CONFIGS[id]) == "table" and CONFIGS[id] or api.type.BaseNodeConfig.new() end
    if kind == CT.BASE_EDGE and EDGES[id] then
        -- A copy, as the game hands out.
        local c = {}
        for k, v in pairs(EDGES[id]) do c[k] = v end
        return c
    end
end
api.engine.system = { lineSystem = { getLines = function() return {} end }, streetSystem = {
    getNode2StreetEdgeMap = function()
        local m = {}
        for node, edges in pairs(STREETS) do m[node] = edges end
        return m
    end,
    getNode2TrackEdgeMap = function() return {} end,
    getNodeStreetSegments = function(node) return STREETS[node] or {} end,
    getNodeTrackSegments = function() return {} end,
} }
"#;

/// A road the room ordered, as the hook hands it (metres): from node 7, onto
/// the middle of the street 8-9, and on over a bridge to open ground.
const ROAD: &str = "{ BuildRoad = { street = '::/street/town_small.street_template', \
    bus_lane = false, tram = 'None', polyline = { \
    vertices = { \
        { pos = { x = 0.0004, y = 0.001, z = 0 }, resolve = { Node = 'Street' } }, \
        { pos = { x = 50, y = 0, z = 0 }, resolve = { Split = { network = 'Street', \
            ends = { a = { x = 50, y = 40, z = 0 }, b = { x = 50, y = -40, z = 0 } } } } }, \
        { pos = { x = 120, y = 0, z = 12 }, resolve = 'New' } }, \
    links = { \
        { from = 0, to = 1, tangent0 = { x = 50, y = 0, z = 0 }, tangent1 = { x = 50, y = 0, z = 0 }, \
          structure = 'Ground' }, \
        { from = 1, to = 2, tangent0 = { x = 70, y = 0, z = 12 }, tangent1 = { x = 70, y = 0, z = 12 }, \
          structure = { Bridge = '::/bridge/stone.lua' } } }, \
    removals = {} } } }";

#[test]
fn the_game_script_builds_a_road_as_the_players_tool_would() {
    let (lua, _script) = engine();
    lua.load(FAKE_NETWORK).exec().unwrap();
    lua.load(format!(
        "HOOK.batch = {{ {ROAD} }} UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        lua.load("return table.concat(HOOK.logged, '|')")
            .eval::<String>()
            .unwrap(),
        "the game script is linked|building +n-5(50.0,0.0,0.0) +n-6(120.0,0.0,12.0) \
         +e-1/0:7>-5 ::/street/town_small.street_template \
         +e-2/0:-5>-6 ::/street/town_small.street_template \
         +e-3/0:8>-5 ::/street/country.street_template \
         +e-4/0:-5>9 ::/street/country.street_template -e100 -n -c8,9",
        "applied, and what was sent in the log: the split street's ends lose their lane \
         configurations with it"
    );
    let built: String = lua
        .load(
            "local c = SENT[1] local p = c.proposal.streetProposal
             local out = { #p.nodesToAdd, #p.edgesToAdd, table.concat(p.edgesToRemove, ','),
                           tostring(c.context.player), tostring(c.ignoreErrors), tostring(c.playerInitiated) }
             for _, n in ipairs(p.nodesToAdd) do
                 out[#out + 1] = n.entity .. '@' .. n.comp.position.x .. ',' .. n.comp.position.y .. ',' .. n.comp.position.z
             end
             for _, e in ipairs(p.edgesToAdd) do
                 local c = e.comp
                 out[#out + 1] = string.format('%d:%d>%d t%d/%s %s %s %.3f,%.3f %.3f,%.3f', e.entity, c.node0, c.node1,
                     e.type, tostring(c.type), tostring(c.typeIndex), tostring(c.roadTemplate),
                     c.tangent0.x, c.tangent0.y, c.tangent1.x, c.tangent1.y)
             end
             return table.concat(out, ' | ')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        built,
        "2 | 4 | 100 | 25 | true | true \
         | -5@50,0,0 | -6@120,0,12 \
         | -1:7>-5 t0/0 -1 ::/street/town_small.street_template 50.000,0.000 50.000,0.000 \
         | -2:-5>-6 t0/1 3 ::/street/town_small.street_template 70.000,0.000 70.000,0.000 \
         | -3:8>-5 t0/nil nil ::/street/country.street_template 0.000,40.000 0.000,40.000 \
         | -4:-5>9 t0/nil nil ::/street/country.street_template 0.000,40.000 0.000,40.000",
        "the links from -1, then the split's halves keeping the street's own template; \
         the new nodes after the edges"
    );
    // The links take the template's lanes and style; the halves keep theirs.
    let lanes: String = lua
        .load(
            "local e = SENT[1].proposal.streetProposal.edgesToAdd
             return e[1].comp.laneConfigs[1] .. '|' .. e[1].comp.roadStyle .. '|' .. e[3].comp.laneConfigs[1]",
        )
        .eval()
        .unwrap();
    assert_eq!(lanes, "town lanes|::/style/town.street_style|country lanes");
}

#[test]
fn a_road_that_resolves_to_nothing_is_built_nowhere() {
    let (lua, _script) = engine();
    lua.load(FAKE_NETWORK).exec().unwrap();
    // Node 7 has moved 3 m: nothing is within 1.5 m of the vertex.
    lua.load(format!(
        "NODES[7] = {{ x = 3, y = 0, z = 0 }} HOOK.batch = {{ {ROAD} }} UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap();
    // A stop on the street it splits.
    lua.load(format!(
        "NODES[7] = {{ x = 0, y = 0, z = 0 }} EDGES[100].objects = {{ {{ 555, 1 }} }} \
         HOOK.batch = {{ {ROAD} }} UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap();
    assert_eq!(lua.load("return #SENT").eval::<usize>().unwrap(), 0);
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert!(
        logged[1].ends_with("no Street node at vertex 1"),
        "{logged:?}"
    );
    assert!(
        logged[2].ends_with("vertex 2 splits an edge with a stop or signal on it"),
        "{logged:?}"
    );
}

/// The street tool's proposal for the road of ROAD, as build 40408 hands it
/// to game scripts: the split of 8-9 is the removed edge and its two halves.
const STREET_PROPOSAL: &str = "{ toAdd = {}, toRemove = {}, proposal = { \
    addedNodes = { { entity = -1, comp = { position = { x = 50, y = 0, z = 0 } } }, \
                   { entity = -2, comp = { position = { x = 120, y = 0, z = 12 } } } }, \
    addedSegments = { \
        { entity = -3, type = 0, comp = { node0 = 7, node1 = -1, type = 0, typeIndex = -1, \
          tangent0 = { x = 50, y = 0, z = 0 }, tangent1 = { x = 50, y = 0, z = 0 }, \
          roadTemplate = '::/street/town_small.street_template', roadStyle = '' } }, \
        { entity = -4, type = 0, comp = { node0 = 9, node1 = -1, type = 0, typeIndex = -1, \
          tangent0 = { x = 0, y = -40, z = 0 }, tangent1 = { x = 0, y = -40, z = 0 }, \
          roadTemplate = '::/street/country.street_template', roadStyle = '' } }, \
        { entity = -5, type = 0, comp = { node0 = -1, node1 = 8, type = 0, typeIndex = -1, \
          tangent0 = { x = 0, y = -40, z = 0 }, tangent1 = { x = 0, y = -40, z = 0 }, \
          roadTemplate = '::/street/country.street_template', roadStyle = '' } }, \
        { entity = -6, type = 0, comp = { node0 = -1, node1 = -2, type = 1, typeIndex = 3, \
          tangent0 = { x = 70, y = 0, z = 12 }, tangent1 = { x = 70, y = 0, z = 12 }, \
          roadTemplate = '::/street/town_small.street_template', roadStyle = '' } } }, \
    removedSegments = { { entity = 100, type = 0, comp = { node0 = 9, node1 = 8 } } }, \
    removedNodes = {}, edgeObjectsToAdd = {} } }";

#[test]
fn a_road_the_street_tool_proposed_goes_to_the_room() {
    let (lua, _script) = engine();
    lua.load(FAKE_NETWORK).exec().unwrap();
    let asked: Vec<String> = lua
        .load(format!(
            "HOOK.room = true HOOK.clicks = 0 \
             local out = {{}} \
             local function ask(proposal) \
                 local r = SCRIPT.guiHandleEvent({{}}, nil, nil, '', 'streetBuilder', 'builder.proposalCreate', {{ proposal }}) \
                 if r == nil then return 'nil' end \
                 for text in pairs(r.errorMessages) do return text end \
             end \
             SCRIPT.guiUpdate({{}}, nil, nil) \
             out[#out + 1] = ask({{ toAdd = {{}}, toRemove = {{}}, proposal = {{ addedNodes = {{}}, \
                 addedSegments = {{}}, removedSegments = {{}} }} }}) \
             out[#out + 1] = ask({STREET_PROPOSAL}) \
             return out"
        ))
        .eval()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        asked,
        ["nil", "nil"],
        "the tool builds through the room, and a proposal of nothing is not refused"
    );
    lua.load("HOOK.clicks = 1 SCRIPT.guiUpdate({}, nil, nil)")
        .exec()
        .unwrap();
    let handed: String = lua
        .load(
            "local b = HOOK.commands[1].BuildRoad local p = b.polyline
             return table.concat({ #HOOK.commands, b.street, tostring(b.style), #p.vertices, #p.links,
                 #p.removals, #p.removed_nodes, p.vertices[1].resolve.Node, tostring(p.vertices[2].resolve),
                 p.links[2].kind.template, p.links[4].structure.Bridge, p.removals[1].ends.a.y,
                 p.removals[1].ends.b.y }, '|')",
        )
        .eval()
        .unwrap_or_else(|error| {
            panic!(
                "{error}\n{:?}",
                lua.load("return HOOK.logged").eval::<Vec<String>>()
            )
        });
    assert_eq!(
        handed,
        "1|::/street/town_small.street_template|nil|5|4|1|0|Street|New\
         |::/street/country.street_template|::/bridge/stone.lua|40|-40",
        "the proposal as the tool made it: the street it joins rebuilt in its own kind"
    );
    // What the room orders, every game builds.
    lua.load("HOOK.batch = { HOOK.commands[1] } UPDATE({}, STATE, 0.2)")
        .exec()
        .unwrap();
    let (edges, removed): (usize, String) = lua
        .load(
            "local p = SENT[1].proposal.streetProposal \
             return #p.edgesToAdd, table.concat(p.edgesToRemove, ',')",
        )
        .eval()
        .unwrap();
    assert_eq!((edges, removed.as_str()), (4, "100"));
}

/// The street tool's build as the room orders it (metres): from node 7 onto
/// the country street 8-11-9, whose node 11 the new junction replaces, the
/// street rebuilt through it in its own kind.
const JUNCTION: &str = "{ BuildRoad = { street = '::/street/town_small.street_template', \
    bus_lane = false, tram = 'None', polyline = { \
    vertices = { \
        { pos = { x = 0, y = 0, z = 0 }, resolve = { Node = 'Street' } }, \
        { pos = { x = 50, y = 2, z = 0 }, resolve = 'New' }, \
        { pos = { x = 50, y = -40, z = 0 }, resolve = { Node = 'Street' } }, \
        { pos = { x = 50, y = 40, z = 0 }, resolve = { Node = 'Street' } } }, \
    links = { \
        { from = 0, to = 1, tangent0 = { x = 50, y = 2, z = 0 }, tangent1 = { x = 50, y = 2, z = 0 }, \
          structure = 'Ground' }, \
        { from = 2, to = 1, tangent0 = { x = 0, y = 42, z = 0 }, tangent1 = { x = 0, y = 42, z = 0 }, \
          structure = 'Ground', kind = { network = 'Street', template = '::/street/country.street_template' } }, \
        { from = 1, to = 3, tangent0 = { x = 0, y = 38, z = 0 }, tangent1 = { x = 0, y = 38, z = 0 }, \
          structure = 'Ground', kind = { network = 'Street', template = '::/street/country.street_template' } } }, \
    removals = { \
        { network = 'Street', ends = { a = { x = 50, y = -40, z = 0 }, b = { x = 50, y = 0, z = 0 } } }, \
        { network = 'Street', ends = { a = { x = 50, y = 0, z = 0 }, b = { x = 50, y = 40, z = 0 } } } }, \
    removed_nodes = { { network = 'Street', at = { x = 50, y = 0, z = 0 } } } } } }";

#[test]
fn the_game_script_rebuilds_a_street_through_a_new_junction() {
    let (lua, _script) = engine();
    lua.load(FAKE_NETWORK).exec().unwrap();
    // The country street through node 11: edges 100 (8-11) and 102 (11-9).
    lua.load(
        "NODES[11] = { x = 50, y = 0, z = 0 } \
         EDGES[100].node1 = 11 \
         EDGES[102] = { node0 = 11, node1 = 9, tangent0 = { x = 0, y = 40, z = 0 }, \
                        tangent1 = { x = 0, y = 40, z = 0 }, objects = {} } \
         STREETS[8], STREETS[11], STREETS[9] = { 100 }, { 100, 102 }, { 102 }",
    )
    .exec()
    .unwrap();
    lua.load(format!(
        "HOOK.batch = {{ {JUNCTION} }} UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap();
    let built: String = lua
        .load(
            "local p = SENT[1].proposal.streetProposal
             local out = { #p.nodesToAdd, table.concat(p.edgesToRemove, ','), table.concat(p.nodesToRemove, ','),
                           table.concat(p.nodeConfigsToRemove, ',') }
             for _, e in ipairs(p.edgesToAdd) do
                 out[#out + 1] = e.entity .. ':' .. e.comp.node0 .. '>' .. e.comp.node1 .. ' '
                     .. e.comp.roadTemplate .. ' ' .. e.comp.laneConfigs[1] .. ' ' .. e.comp.roadStyle
             end
             return table.concat(out, ' | ')",
        )
        .eval()
        .unwrap_or_else(|error| {
            panic!(
                "{error}\n{:?}",
                lua.load("return HOOK.logged").eval::<Vec<String>>()
            )
        });
    assert_eq!(
        built,
        "1 | 100,102 | 11 | 8,9 \
         | -1:7>-4 ::/street/town_small.street_template town lanes ::/style/town.street_style \
         | -2:8>-4 ::/street/country.street_template country lanes ::/style/country.street_style \
         | -3:-4>9 ::/street/country.street_template country lanes ::/style/country.street_style",
        "the old junction's node and edges removed, the street rebuilt in its own kind"
    );
    // A stop on an edge it removes: built nowhere.
    lua.load(format!(
        "SENT = {{}} EDGES[102].objects = {{ {{ 555, 1 }} }} HOOK.batch = {{ {JUNCTION} }} UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap();
    assert_eq!(lua.load("return #SENT").eval::<usize>().unwrap(), 0);
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert!(
        logged
            .last()
            .unwrap()
            .ends_with("removal 2 has a stop or signal on it and no link rebuilds it"),
        "{logged:?}"
    );
}

#[test]
fn a_street_build_the_room_cannot_carry_says_why() {
    let (lua, _script) = engine();
    lua.load(FAKE_NETWORK).exec().unwrap();
    let asked: Vec<String> = lua
        .load(format!(
            "HOOK.room = true HOOK.clicks = 0 \
             local out = {{}} \
             local function ask(proposal) \
                 local r = SCRIPT.guiHandleEvent({{}}, nil, nil, '', 'streetBuilder', 'builder.proposalCreate', {{ proposal }}) \
                 if r == nil then return 'nil' end \
                 for text in pairs(r.errorMessages) do return text end \
             end \
             local stop = {STREET_PROPOSAL} stop.proposal.edgeObjectsToAdd = {{ {{}} }} \
             out[#out + 1] = ask(stop) \
             local nowhere = {STREET_PROPOSAL} nowhere.proposal.addedSegments[1].comp.node0 = 12345 \
             out[#out + 1] = ask(nowhere) \
             return out"
        ))
        .eval()
        .unwrap();
    assert_eq!(
        asked,
        [
            "Not in multiplayer yet: a build with a stop or signal",
            "Not in multiplayer yet: node 12345 has no position"
        ]
    );
}

/// Vehicles, lines and station groups for the registry's tests, over the
/// stand-in engine state: VEHICLES, LINES and GROUPS list what exists; a
/// bought vehicle appears as NEXT_VEHICLE, a new line as NEXT_LINE. As the
/// game's line system, LINES lists a new line only from the next update:
/// until then it is in LATE_LINES, and exists all the same.
const FAKE_FLEET: &str = r#"
VEHICLES, LINES, GROUPS = { 401, 402 }, { 301 }, { 91, 90 }
LATE_LINES = {}
NEXT_VEHICLE, NEXT_LINE = 500, 600
BEFORE_UPDATE = function()
    for _, e in ipairs(LATE_LINES) do LINES[#LINES + 1] = e end
    LATE_LINES = {}
end
local function has(list, e)
    for _, x in ipairs(list) do if x == e then return true end end
    return false
end
local CT = { CONSTRUCTION = 2, LINE = 3, TRANSPORT_VEHICLE = 4, STATION_GROUP = 9, GAME_TIME = 10 }
api.type.ComponentType = CT
api.engine.getEntitiesWithComponent = function(kind)
    if kind == CT.TRANSPORT_VEHICLE then return VEHICLES end
    if kind == CT.STATION_GROUP then return GROUPS end
    if kind == CT.CONSTRUCTION then return { 201 } end
    return {}
end
api.engine.system = { lineSystem = { getLines = function() return LINES end } }
api.engine.util.getWorld = function() return 1 end
api.engine.getComponent = function(e, kind)
    if kind == CT.GAME_TIME then return { gameTime = 777000 } end
    if kind == CT.CONSTRUCTION and e == 201 then
        return { fileName = 'depot/bus_depot.con', depots = { 202 },
                 transf = { 1,0,0,0, 0,1,0,0, 0,0,1,0, 600,10,2,1 } }
    end
    if kind == CT.LINE and (has(LINES, e) or has(LATE_LINES, e)) then return { stops = {} } end
    if kind == CT.TRANSPORT_VEHICLE and has(VEHICLES, e) then return {} end
    if kind == CT.STATION_GROUP and has(GROUPS, e) then return {} end
end
api.res = { modelRep = {
    find = function(name) if name == 'vehicle/bus/city.mdl' then return 41 end return -1 end,
    getName = function(id) if id == 41 then return 'vehicle/bus/city.mdl' end end,
} }
api.type.Vec3f = { new = function(x, y, z) return { x = x, y = y, z = z } end }
api.type.TransportVehiclePart = { new = function() return { part = {} } end }
api.type.TransportVehicleConfig = { new = function() return {} end }
api.type.LoadConfig = { new = function() return {} end }
api.cmd.makeVehicleBuyCmd = function(player, depot, config)
    return { buy = { player = player, depot = depot, config = config } }
end
api.cmd.makeVehicleSetLineCmd = function(vehicle, line, stop)
    return { setLine = { vehicle = vehicle, line = line, stop = stop } }
end
local send = api.cmd.sendCommand
api.cmd.sendCommand = function(command, ...)
    -- As the game: a bought vehicle exists, and is listed, at once; a new
    -- line exists at once, and is listed from the next update. The
    -- command's data says which it made.
    if command.buy then
        VEHICLES[#VEHICLES + 1] = NEXT_VEHICLE
        command.resultVehicleEntity, command.made = NEXT_VEHICLE, NEXT_VEHICLE
    end
    if command.createLine then
        LATE_LINES[#LATE_LINES + 1] = NEXT_LINE
        command.resultEntity, command.made = NEXT_LINE, NEXT_LINE
    end
    send(command, ...)
end
"#;

/// A bus bought at the depot of FAKE_FLEET, as the hook hands it (metres).
const BUY_BUS: &str = "{ BuyVehicle = { \
    depot = { file = 'depot/bus_depot.con', at = { x = 600.4, y = 10, z = 2 } }, \
    consist = { { model = 'vehicle/bus/city.mdl', reversed = false, \
                  loads = { { config = 0, cargo = 3 } }, color = { r = 0.5, g = 0.25, b = 0 } } }, \
    groups = { 1 }, multiple_units = { '' } } }";

#[test]
fn the_registry_names_vehicles_in_the_order_they_came_and_never_again() {
    let (lua, _script) = engine();
    lua.load(FAKE_FLEET).exec().unwrap();
    let named: String = lua
        .load(
            "local registry = ug_require('tpf3mp_1::/scripts/tpf3mp/registry.lua')
             local reg, fresh = registry.sync(nil)
             local out = { #fresh, registry.id(reg, 'vehicles', 401), registry.id(reg, 'vehicles', 402),
                           registry.id(reg, 'groups', 90), registry.id(reg, 'groups', 91),
                           registry.id(reg, 'lines', 301) }
             -- 401 sold, 403 bought: 401's id is retired, 403 gets the next.
             VEHICLES = { 402, 403 }
             reg, fresh = registry.sync(reg)
             out[#out + 1] = tostring(registry.id(reg, 'vehicles', 401))
             out[#out + 1] = registry.id(reg, 'vehicles', 403)
             out[#out + 1] = registry.entity(reg, 'vehicles', 1)
             out[#out + 1] = #fresh .. ':' .. fresh[1][1] .. ':' .. fresh[1][2] .. ':' .. fresh[1][3]
             -- A line the line system leaves out, but which exists, keeps
             -- its id; one gone is retired.
             LINES, LATE_LINES = {}, { 301 }
             reg = registry.sync(reg)
             out[#out + 1] = registry.id(reg, 'lines', 301)
             LATE_LINES = {}
             reg = registry.sync(reg)
             out[#out + 1] = tostring(registry.id(reg, 'lines', 301))
             -- What an action made is bound at once, listed or not.
             LATE_LINES = { 600 }
             reg, fresh = registry.sync(reg, { lines = { 600 } })
             out[#out + 1] = #fresh .. ':' .. registry.id(reg, 'lines', 600)
             LINES, LATE_LINES = { 600 }, {}
             -- A kind it cannot list keeps its names.
             api.engine.system.lineSystem = nil
             local _, _, failed = registry.sync(reg)
             out[#out + 1] = #failed .. ':' .. registry.id(reg, 'lines', 600)
             return table.concat(out, ' ')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        named, "5 0 1 0 1 0 nil 2 402 1:vehicles:2:403 0 nil 1:1 1:1",
        "lowest entity first, per kind; a retired id never comes back"
    );
}

#[test]
fn the_game_script_buys_the_vehicle_and_tells_the_buyer_which() {
    let (lua, _script) = engine();
    lua.load(FAKE_FLEET).exec().unwrap();
    // The room's first update begins the registry; then the bus, and a
    // vehicle put on line 0.
    lua.load(format!(
        "HOOK.room = true UPDATE({{}}, STATE, 0.2) \
         HOOK.batch = {{ {BUY_BUS}, {{ AssignLine = {{ vehicles = {{ 2 }}, line = 0, first_stop = 1 }} }} }} \
         UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap();
    let bought: String = lua
        .load(
            "local b = SENT[1].buy local p = b.config.vehicles[1] \
             local s = SENT[2].setLine \
             return table.concat({ b.player, b.depot, p.part.modelId, tostring(p.part.reversed), \
                 p.part.compartment2loadConfig[1].cargoTypeId, p.part.color.y, p.purchaseTime, \
                 tostring(p.autoLoadConfig[1]), b.config.vehicleGroups[1], \
                 s.vehicle, s.line, s.stop }, '|')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        bought, "25|202|41|false|3|0.25|777000|true|1|500|301|1",
        "bought at the depot's construction there, as the store configured it; \
         then vehicle-2, the new one, on line-0"
    );
    let applied: String = lua
        .load(
            "local out = {} for _, a in ipairs(HOOK.applied) do \
                 out[#out + 1] = a.i .. ':' .. tostring(a.ok) .. ':' .. tostring(a.entity) end \
             return table.concat(out, ' ')",
        )
        .eval()
        .unwrap();
    assert_eq!(applied, "1:true:500 2:true:nil", "the buyer hears which");
    // The registry the GUI reads is in the script's state, saved with the
    // world.
    let saved: u32 = lua
        .load(
            "return ug_require('tpf3mp_1::/scripts/tpf3mp/registry.lua').id(STATE.value.registry, 'vehicles', 500)",
        )
        .eval()
        .unwrap();
    assert_eq!(saved, 2);
}

#[test]
fn a_bought_vehicle_goes_to_the_room_and_the_store_hears_which_it_is() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_CMD).exec().unwrap();
    // The GUI reads the game script's registry from its state, and the
    // depot's construction, as the game has them.
    lua.load(
        "api.cmd.makeVehicleSetLineCmd = function(vehicle, line, stop) return { kind = 'setLine' } end \
         api.type = { ComponentType = { GAME_SCRIPT = 7, CONSTRUCTION = 2 } } \
         api.engine = { \
             getComponent = function(e, kind) \
                 if kind == 7 and e == 77 then return { state = { registry = { \
                     vehicles = { next = 4, bound = { { 3, 500 } } }, \
                     lines = { next = 2, bound = { { 1, 600 } } }, groups = { next = 0, bound = {} } } } } end \
                 if kind == 2 and e == 201 then return { fileName = 'depot/bus_depot.con', \
                     transf = { 1,0,0,0, 0,1,0,0, 0,0,1,0, 600,10,2,1 } } end \
             end, \
             system = { \
                 gameScriptSystem = { getEntityForGameScript = function(name) \
                     if name == 'tpf3mp_1::/tpf3mp_sim/tpf3mp_sim.gs' then return 77 end return -1 end }, \
                 streetConnectorSystem = { getConstructionEntityForDepot = function(d) \
                     if d == 202 then return 201 end end }, \
             }, \
         } \
         api.res = { modelRep = { getName = function(id) if id == 41 then return 'vehicle/bus/city.mdl' end end } } \
         M = mount(loadPlugin()) M.step() HOOK.room = true",
    )
    .exec()
    .unwrap();
    // The store buys a bus at depot 202 and, told which it is, puts it on
    // line 600, as vehicle_react_util.tl does.
    lua.load(
        "CONFIG = { vehicles = { { part = { modelId = 41, reversed = true, \
             compartment2loadConfig = { { loadConfigIndex = 0, cargoTypeId = 3 } }, \
             color = { x = 1, y = 0, z = 0 } } } }, vehicleGroups = { 1 }, muFileNames = { '' } } \
         HEARD = nil \
         api.cmd.sendCommand(api.cmd.makeVehicleBuyCmd(25, 202, CONFIG), function(data, ok, entities) \
             HEARD = { vehicle = data.resultVehicleEntity, ok = ok, entity = entities[1] and entities[1][1] } \
             api.cmd.sendCommand(api.cmd.makeVehicleSetLineCmd(data.resultVehicleEntity, 600, 0)) \
         end) \
         M.step()",
    )
    .exec()
    .unwrap();
    let (handed, heard): (usize, bool) = lua
        .load("return #HOOK.commands, HEARD ~= nil")
        .eval()
        .unwrap();
    assert_eq!(handed, 1);
    assert!(!heard, "not before the room's action ran here");
    let buy: String = lua
        .load(
            "local b = HOOK.commands[1].BuyVehicle local p = b.consist[1] \
             return table.concat({ b.depot.file, b.depot.at.x, p.model, tostring(p.reversed), \
                 p.loads[1].cargo, p.color.r, b.groups[1] }, '|')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        buy,
        "depot/bus_depot.con|600|vehicle/bus/city.mdl|true|3|1|1"
    );
    // This game applied it and bought vehicle 500: the store hears so, and
    // its line assignment goes to the room by canonical ids.
    lua.load("HOOK.results = { { ticket = 1, ok = true, entity = 500 } } M.step()")
        .exec()
        .unwrap();
    let assigned: String = lua
        .load(
            "local a = HOOK.commands[2].AssignLine \
             return table.concat({ HEARD.vehicle, tostring(HEARD.ok), HEARD.entity, \
                 a.vehicles[1], a.line, a.first_stop }, '|')",
        )
        .eval()
        .unwrap();
    assert_eq!(assigned, "500|true|500|3|1|0");
}

#[test]
fn the_next_reachable_stop_travels_as_the_games_choice() {
    let (lua, _script) = engine();
    lua.load(FAKE_FLEET).exec().unwrap();
    // The line manager's "Next Reachable Stop" is stop -1 (build 40408): the
    // action carries no first stop, and every game is given -1 again.
    let (captured, ok): (String, bool) = lua
        .load(
            "local capture = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua') \
             local ctx = { vehicle = function() return 3 end, line = function() return 1 end } \
             ACTION = capture.vehicleSetLine(ctx, 500, 600, -1) \
             return tostring(ACTION.AssignLine.first_stop), schema_check(ACTION)",
        )
        .eval()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(captured, "nil");
    assert!(ok, "the schema takes it");
    lua.load(format!(
        "HOOK.room = true UPDATE({{}}, STATE, 0.2) \
         HOOK.batch = {{ {BUY_BUS}, {{ AssignLine = {{ vehicles = {{ 2 }}, line = 0 }} }} }} \
         UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap();
    let stop: i64 = lua.load("return SENT[2].setLine.stop").eval().unwrap();
    assert_eq!(stop, -1);
}

#[test]
fn a_line_travels_by_its_stations_ids_and_is_made_again_the_same() {
    let (lua, _script) = engine();
    lua.load(FAKE_FLEET).exec().unwrap();
    lua.load(
        "api.type.Line = { new = function() return { vehicleInfo = {} } end, \
             Stop = { new = function() return {} end }, StopConfig = { new = function() return {} end } } \
         api.type.StationTerminal = { new = function(s, t) return { station = s, terminal = t } end } \
         api.cmd.makeLineCreateCmd = function(name, color, player, line) \
             return { createLine = { name = name, color = color, player = player, line = line } } end",
    )
    .exec()
    .unwrap();
    // The line manager's line, as makeLineCreateCmd gets it: two stops at
    // station groups 90 and 91.
    let (ok, why): (bool, Option<String>) = lua
        .load(
            "HOOK.room = true UPDATE({}, STATE, 0.2) \
             local registry = ug_require('tpf3mp_1::/scripts/tpf3mp/registry.lua') \
             local reg = STATE.value.registry \
             local ctx = { group = function(e) return registry.id(reg, 'groups', e) end, \
                           line = function(e) return registry.id(reg, 'lines', e) end } \
             local function stop(group, mode) return { stationGroup = group, station = 0, terminal = 1, \
                 alternativeTerminals = { { station = 0, terminal = 2 } }, loadMode = mode, \
                 minWaitingTime = 0, maxWaitingTime = 180, maxAdditionalWaitingTime = 30.5, waypoints = {}, \
                 stopConfig = { load = { true, false }, maxLoad = { 1, 0.25 }, forceUnload = false, \
                     destroyForConfigChange = true, destroyForRefresh = false } } end \
             LINE = { stops = { stop(90, 0), stop(91, 2) }, customFilters = false, reservationPriority = 0.5, \
                 vehicleInfo = { transportModes = { [3] = true, [0] = true, [5] = false } } } \
             ACTION = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua').lineCreate(ctx, 'Line 1', \
                 { x = 0.8, y = 0.2, z = 0 }, 25, LINE) \
             return schema_check(ACTION)",
        )
        .eval()
        .unwrap();
    assert!(ok, "{why:?}");
    let carried: String = lua
        .load(
            "local l = ACTION.CreateLine.line local s = l.stops[2] \
             return table.concat({ l.stops[1].group, s.group, s.load_mode, s.terminal.terminal, \
                 s.alternatives[1].terminal, s.max_extra_wait, s.rules.max_load[2], \
                 table.concat(l.modes, ','), l.reservation_priority }, '|')",
        )
        .eval()
        .unwrap();
    assert_eq!(carried, "0|1|FullLoadAll|1|2|30.5|0.25|0,3|0.5");
    // Every game makes it again from the action: stations by their groups
    // here, and its new line named.
    lua.load("HOOK.batch = { ACTION } UPDATE({}, STATE, 0.2)")
        .exec()
        .unwrap();
    let made: String = lua
        .load(
            "local c = SENT[1].createLine local s = c.line.stops[2] \
             return table.concat({ c.name, c.color.x, c.player, c.line.stops[1].stationGroup, \
                 s.stationGroup, s.loadMode, s.alternativeTerminals[1].terminal, \
                 s.stopConfig.maxLoad[2], tostring(s.stopConfig.destroyForConfigChange), \
                 tostring(c.line.vehicleInfo.transportModes[3]), \
                 tostring(c.line.vehicleInfo.transportModes[5]), \
                 HOOK.applied[1].entity }, '|')",
        )
        .eval()
        .unwrap();
    assert_eq!(made, "Line 1|0.8|25|90|91|2|2|0.25|true|true|nil|600");
    // The line system lists it only from the next update; the registry
    // names it at once, and the next action finds it by that name.
    let named: String = lua
        .load(
            "local registry = ug_require('tpf3mp_1::/scripts/tpf3mp/registry.lua') \
             local out = { #LINES, registry.id(STATE.value.registry, 'lines', 600) } \
             api.cmd.makeEntitySetNameCmd = function(e, name) return { rename = { entity = e, name = name } } end \
             HOOK.batch = { { EditLine = { line = 1, change = { Rename = 'North' } } } } UPDATE({}, STATE, 0.2) \
             out[#out + 1] = SENT[2].rename.entity \
             out[#out + 1] = registry.id(STATE.value.registry, 'lines', 600) \
             return table.concat(out, ' ')",
        )
        .eval()
        .unwrap();
    assert_eq!(named, "1 1 600 1");
}

#[test]
fn the_bulldozer_removes_a_construction_or_edges_in_every_game() {
    let (lua, _script) = engine();
    lua.load(FAKE_NETWORK).exec().unwrap();
    // A depot, as the game has it; the game's own remove proposals.
    lua.load(
        "api.type.ComponentType.CONSTRUCTION = 2 \
         CONSTRUCTIONS = { [5000] = { fileName = '::/depots/road/road_depot/road_depot.con', \
             transf = { 1,0,0,0, 0,1,0,0, 0,0,1,0, 80,0,0,1 } } } \
         local get = api.engine.getComponent \
         api.engine.getComponent = function(e, kind) \
             if kind == 2 then return CONSTRUCTIONS[e] end return get(e, kind) end \
         api.engine.getEntitiesWithComponent = function(kind) \
             local l = {} if kind == 2 then for e in pairs(CONSTRUCTIONS) do l[#l + 1] = e end end return l end \
         api.engine.util.proposal = { \
             createProposalRemove = function(e, context) return { removes = e, player = context.player } end, \
             makeSegmentsRemoveProposal = function(ids) return { removesEdges = table.concat(ids, ',') } end }",
    )
    .exec()
    .unwrap();
    // The bulldozer over the depot: the depot, its entrance edge and node;
    // over the street 8-9: that edge.
    let (depot, street): (String, String) = lua
        .load(
            "local capture = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua') \
             local function part(t) t.addedNodes = t.addedNodes or {} t.addedSegments = t.addedSegments or {} \
                 t.removedSegments = t.removedSegments or {} t.removedNodes = t.removedNodes or {} \
                 t.edgeObjectsToAdd = {} return t end \
             DEPOT = capture.bulldoze({ toAdd = {}, toRemove = { 5000 }, proposal = part({ \
                 removedSegments = { { entity = 6967, type = 0, comp = { node0 = 7, node1 = 2066, objects = {} } } }, \
                 removedNodes = { { entity = 2066, comp = { position = { x = 70, y = 0, z = 0 } } } } }) }) \
             STREET = capture.bulldoze({ toAdd = {}, toRemove = {}, proposal = part({ \
                 removedSegments = { { entity = 100, type = 0, comp = { node0 = 8, node1 = 9, objects = {} } } } }) }) \
             local c, e = DEPOT.Bulldoze.Construction, STREET.Bulldoze.Edges \
             return c.file .. '@' .. c.at.x .. ':' .. tostring(schema_check(DEPOT)), \
                 e.network .. ':' .. e.edges[1].a.y .. '>' .. e.edges[1].b.y .. ':' .. tostring(schema_check(STREET))",
        )
        .eval()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(depot, "::/depots/road/road_depot/road_depot.con@80:true");
    assert_eq!(street, "Street:-40>40:true");
    // Every game removes them as the game itself would: the depot with its
    // own entrance, the edge with the nodes it leaves alone. The player pays.
    lua.load("HOOK.batch = { DEPOT, STREET } UPDATE({}, STATE, 0.2)")
        .exec()
        .unwrap();
    let removed: String = lua
        .load(
            "return SENT[1].proposal.removes .. '|' .. SENT[1].proposal.player .. '|' \
                 .. SENT[2].proposal.removesEdges .. '|' .. tostring(SENT[2].context.player) \
                 .. '|' .. tostring(SENT[2].ignoreErrors) .. '|' .. #HOOK.applied",
        )
        .eval()
        .unwrap_or_else(|error| {
            panic!(
                "{error}\n{:?}",
                lua.load("return HOOK.logged").eval::<Vec<String>>()
            )
        });
    assert_eq!(removed, "5000|25|100|25|true|2");
    // A removal the room cannot carry says why.
    let why: String = lua
        .load(
            "local capture = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua') \
             local _, why = capture.bulldoze({ toAdd = {}, toRemove = {}, proposal = { addedNodes = {}, \
                 addedSegments = {}, removedNodes = {}, edgeObjectsToAdd = {}, removedSegments = { \
                 { entity = 101, type = 0, comp = { node0 = 10, node1 = 7, objects = { { 1, 0 } } } } } } }) \
             return why",
        )
        .eval()
        .unwrap();
    assert_eq!(why, "removing an edge with a stop or signal on it");
}

/// Stops over FAKE_NETWORK: the game's edge object types, the stop's model
/// and construction, and the script proposal's edge object record. Edge
/// 100 runs north from node 8 (50, -40) to node 9 (50, 40).
const FAKE_STOPS: &str = r#"
api.type.ComponentType.EDGE_OBJECT = 14
api.type.enum.EdgeObjectType = { STOP_LEFT = 0, STOP_RIGHT = 1, SIGNAL = 2 }
api.type.SimpleStreetProposal = { EdgeObject = { new = function() return {} end } }
api.res.modelRep = { getName = function(id)
    if id == 77 then return '::/stations/street/small_stops/small_new.con' end
end }
-- Edge objects, as their EDGE_OBJECT component has them.
OBJECTS = {}
local get = api.engine.getComponent
api.engine.getComponent = function(id, kind)
    if kind == 14 then return OBJECTS[id] end
    return get(id, kind)
end
"#;

/// The stop tool's proposal for a stop left of edge 100, 6 m east of its
/// middle, as build 40408 hands it to game scripts: the edge removed and
/// added again between the same nodes, the new stop in its objects and in
/// `edgeObjectsToAdd`. `old` are the objects the edge had, `kept` those the
/// tool keeps on it and `kept_records` their `edgeObjectsToAdd` records,
/// each followed by a comma.
fn stop_proposal(old: &str, kept: &str, kept_records: &str) -> String {
    let edge = |entity: i64, objects: String| {
        format!(
            "{{ entity = {entity}, type = 0, comp = {{ node0 = 8, node1 = 9, \
             tangent0 = {{ x = 0, y = 80, z = 0 }}, tangent1 = {{ x = 0, y = 80, z = 0 }}, \
             objects = {{ {objects} }} }} }}"
        )
    };
    let removed = edge(100, old.to_owned());
    let added = edge(-1, format!("{kept} {{ -400000000, 0 }}"));
    format!(
        "{{ toAdd = {{}}, toRemove = {{}}, proposal = {{ addedNodes = {{}}, removedNodes = {{}}, \
         removedSegments = {{ {removed} }}, addedSegments = {{ {added} }}, \
         edgeObjectsToAdd = {{ {kept_records} {{ category = 0, left = true, \
             modelInstance = {{ modelId = 77, \
                 transf = {{ 1,0,0,0, 0,1,0,0, 0,0,1,0, 56,0,0,1 }} }} }} }} }} }}"
    )
}

#[test]
fn a_stop_the_stop_tool_placed_goes_to_the_room_and_every_game_places_it() {
    let (lua, _script) = engine();
    lua.load(FAKE_NETWORK).exec().unwrap();
    lua.load(FAKE_STOPS).exec().unwrap();
    let proposal = stop_proposal("", "", "");
    let asked: String = lua
        .load(format!(
            "HOOK.room = true HOOK.clicks = 0 SCRIPT.guiUpdate({{}}, nil, nil) \
             local r = SCRIPT.guiHandleEvent({{}}, nil, nil, '', 'streetTerminalBuilder', \
                 'builder.proposalCreate', {{ {proposal} }}) \
             if r == nil then return 'nil' end \
             for text in pairs(r.errorMessages) do return text end"
        ))
        .eval()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(asked, "nil", "the stop tool builds through the room");
    lua.load("HOOK.clicks = 1 SCRIPT.guiUpdate({}, nil, nil)")
        .exec()
        .unwrap();
    let handed: String = lua
        .load(
            "local s = HOOK.commands[1].PlaceStop
             local function n(v) return string.format('%.3f', v) end
             return table.concat({ #HOOK.commands, tostring(schema_check(HOOK.commands[1])),
                 s.edge.network, n(s.edge.ends.a.y), n(s.edge.ends.b.y), n(s.at.x), n(s.at.y),
                 tostring(s.left), n(s.direction.x), n(s.direction.y), s.model }, '|')",
        )
        .eval()
        .unwrap_or_else(|error| {
            panic!(
                "{error}\n{:?}",
                lua.load("return HOOK.logged").eval::<Vec<String>>()
            )
        });
    assert_eq!(
        handed,
        "1|true|Street|-40.000|40.000|50.000|0.000|true|0.000|1.000\
         |::/stations/street/small_stops/small_new.con",
        "the edge by its ends, the stop's place on its centreline, the engine's side, \
         the edge's direction there and the stop's construction"
    );

    // What the room orders, every game places: the edge rebuilt with the
    // stop, as the tool does, paid by the player.
    lua.load("HOOK.batch = { HOOK.commands[1] } UPDATE({}, STATE, 0.2)")
        .exec()
        .unwrap();
    let placed: String = lua
        .load(
            "local c = SENT[1] local p = c.proposal.streetProposal local e = p.edgesToAdd[1]
             local o = p.edgeObjectsToAdd[1]
             return table.concat({ #SENT, e.entity, e.type, e.comp.node0, e.comp.node1,
                 e.comp.roadTemplate, #e.comp.objects, e.comp.objects[1][1], e.comp.objects[1][2],
                 table.concat(p.edgesToRemove, ','), table.concat(p.nodeConfigsToRemove, ','),
                 o.edgeEntity, string.format('%.4f', o.param), tostring(o.left), o.model,
                 o.playerEntity, tostring(c.context.player), tostring(c.ignoreErrors),
                 tostring(c.playerInitiated) }, '|')",
        )
        .eval()
        .unwrap_or_else(|error| {
            panic!(
                "{error}\n{:?}",
                lua.load("return HOOK.logged").eval::<Vec<String>>()
            )
        });
    assert_eq!(
        placed,
        "1|-1|0|8|9|::/street/country.street_template|1|-400000000|0|100|8,9|-1|0.5000|true\
         |::/stations/street/small_stops/small_new.con|25|25|true|true"
    );
}

/// As build 40408 proposes a stop to game scripts: no model and no place on
/// its edge objects (seen in a room: `+o{resultEntity=-1 category=0
/// left=false playerEntity=3869}`). The stop is the construction the
/// construction menu gave the tool, which the GUI noted, where the cursor
/// is; a two-sided one is one click on both sides, and every game builds
/// both.
#[test]
fn a_stop_as_the_game_proposes_it_is_the_noted_construction_under_the_cursor() {
    let (lua, _script) = engine();
    lua.load(FAKE_NETWORK).exec().unwrap();
    lua.load(FAKE_STOPS).exec().unwrap();
    lua.load(
        "api.gui = { mouse = { hasTerrainPosition = function() return true end, \
                               getTerrainPosition = function() return { x = 56, y = 0, z = 0 } end } }",
    )
    .exec()
    .unwrap();
    // Both sides, neither object with a model or a place.
    let proposal = stop_proposal("", "{ -400000001, 1 },", "{ category = 0, left = false },")
        .replace(
            ", \
             modelInstance = { modelId = 77, \
                 transf = { 1,0,0,0, 0,1,0,0, 0,0,1,0, 56,0,0,1 } }",
            "",
        );
    assert!(!proposal.contains("modelInstance"), "{proposal}");
    let ask = |proposal: &str| -> String {
        lua.load(format!(
            "HOOK.room = true HOOK.clicks = 0 SCRIPT.guiUpdate({{}}, nil, nil) \
             local r = SCRIPT.guiHandleEvent({{}}, nil, nil, '', 'streetTerminalBuilder', \
                 'builder.proposalCreate', {{ {proposal} }}) \
             if r == nil then return 'nil' end \
             for text in pairs(r.errorMessages) do return text end"
        ))
        .eval()
        .unwrap_or_else(|error| panic!("{error}"))
    };
    // Nothing noted: which stop it is, the room cannot say.
    assert!(
        ask(&proposal).starts_with("Not in multiplayer yet: the stop's construction"),
        "{}",
        ask(&proposal)
    );
    // The menu gave the tool the two-sided stop: noted in the GUI's state.
    lua.load(
        "local capture = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua') \
         local util = { getActionParams = function(definition) \
             return { constructionActionParams = { edgeObjectBuilder = { resName = definition } } } end } \
         package.loaded['tpf3mp.stopToolWatched'] = nil \
         assert(capture.watchStopTool(util, ug_require('tpf3mp_1::/scripts/tpf3mp/bridge.lua').attach(tpf3mp_native))) \
         util.getActionParams('stations/street/small_stops/small_new_twosided.con')",
    )
    .exec()
    .unwrap();
    assert_eq!(
        ask(&proposal),
        "nil",
        "the stop tool builds through the room"
    );
    lua.load("HOOK.clicks = 1 SCRIPT.guiUpdate({}, nil, nil)")
        .exec()
        .unwrap();
    let handed: String = lua
        .load(
            "local s = HOOK.commands[1].PlaceStop
             local function n(v) return string.format('%.3f', v) end
             return table.concat({ #HOOK.commands, tostring(schema_check(HOOK.commands[1])),
                 n(s.at.x), n(s.at.y), tostring(s.left), s.model, tostring(s.two_sided) }, '|')",
        )
        .eval()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        handed,
        "1|true|50.000|0.000|false|stations/street/small_stops/small_new_twosided.con|true"
    );
    // Every game builds it on both sides of the edge, in one proposal.
    lua.load("HOOK.batch = { HOOK.commands[1] } UPDATE({}, STATE, 0.2)")
        .exec()
        .unwrap();
    let placed: String = lua
        .load(
            "local p = SENT[1].proposal.streetProposal local e = p.edgesToAdd[1]
             local out = {}
             for _, o in ipairs(e.comp.objects) do out[#out + 1] = o[1] .. ':' .. o[2] end
             for _, o in ipairs(p.edgeObjectsToAdd) do
                 out[#out + 1] = tostring(o.left) .. ':' .. o.model
             end
             return table.concat(out, '|')",
        )
        .eval()
        .unwrap_or_else(|error| {
            panic!(
                "{error}\n{:?}",
                lua.load("return HOOK.logged").eval::<Vec<String>>()
            )
        });
    assert_eq!(
        placed,
        "-400000000:1|-400000001:0|false:stations/street/small_stops/small_new_twosided.con\
         |true:stations/street/small_stops/small_new_twosided.con"
    );
}

#[test]
fn a_stop_the_room_cannot_carry_says_why() {
    let (lua, _script) = engine();
    lua.load(FAKE_NETWORK).exec().unwrap();
    lua.load(FAKE_STOPS).exec().unwrap();
    let capture = |proposal: String| -> String {
        lua.load(format!(
            "local a, why = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua').stop({proposal})
             if a == nil then return why end
             if type(a) == 'table' then return 'table' end
             return tostring(a)"
        ))
        .eval()
        .unwrap()
    };
    // Dropped where a stop stood: the old one is gone from the new edge.
    assert_eq!(
        capture(stop_proposal("{ 555, 0 },", "", "")),
        "a stop that replaces another"
    );
    // A two-sided stop: a new object on each side, one click.
    let two = stop_proposal("", "{ -400000001, 1 },", "{ category = 0, left = false },");
    assert_eq!(capture(two), "table");
    // A signal the engine lists as a stop, and a side the engine lists
    // other than `left` says.
    let signal = stop_proposal("", "", "").replace("category = 0", "category = 2");
    assert_eq!(capture(signal), "a signal the engine lists as no signal");
    // A signal (the engine's SIGNAL, 2): carried as one, one-way as noted.
    let signal = stop_proposal("", "", "")
        .replace("category = 0", "category = 2")
        .replace("{ -400000000, 0 }", "{ -400000000, 2 }");
    let carried: String = lua
        .load(format!(
            "local a = ug_require('tpf3mp_1::/scripts/tpf3mp/engine.lua').placeStop({signal}, nil, true) \
             return a.PlaceStop.object .. ' ' .. tostring(a.PlaceStop.one_way) .. ' ' .. tostring(schema_check(a))"
        ))
        .eval()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(carried, "Signal true true");
    let side = stop_proposal("", "", "").replace("left = true", "left = false");
    assert_eq!(capture(side), "a stop whose side the room cannot say");
    // With a stop on the other side, kept: carried.
    let beside = stop_proposal(
        "{ 555, 1 },",
        "{ 555, 1 },",
        "{ category = 0, left = false },",
    );
    assert_eq!(capture(beside), "table");
    // The tool before its first click: nothing.
    assert_eq!(
        capture(
            "{ toAdd = {}, toRemove = {}, proposal = { addedNodes = {}, removedNodes = {}, \
             addedSegments = {}, removedSegments = {}, edgeObjectsToAdd = {} } }"
                .into()
        ),
        "false"
    );
}

#[test]
fn a_stop_is_placed_beside_the_edges_others_and_never_on_a_taken_side() {
    let (lua, _script) = engine();
    lua.load(FAKE_NETWORK).exec().unwrap();
    lua.load(FAKE_STOPS).exec().unwrap();
    // A stop on the right already; the room orders one on the left, from a
    // game whose edge ran the other way (its direction south): here it is
    // the right, taken.
    let stop = "{ PlaceStop = { edge = { network = 'Street', ends = { a = { x = 50, y = -40, z = 0 }, \
        b = { x = 50, y = 40, z = 0 } } }, at = { x = 50, y = 0, z = 0 }, left = true, \
        direction = { x = 0, y = -1, z = 0 }, model = '::/stations/street/small_stops/small_new.con' } }";
    lua.load(format!(
        "EDGES[100].objects = {{ {{ 555, 1 }} }} HOOK.batch = {{ {stop} }} UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap();
    assert_eq!(lua.load("return #SENT").eval::<usize>().unwrap(), 0);
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert!(
        logged
            .last()
            .unwrap()
            .ends_with("the edge has a stop on that side already"),
        "{logged:?}"
    );
    // Facing north it is the left: placed beside the kept one.
    lua.load(format!(
        "HOOK.batch = {{ {} }} UPDATE({{}}, STATE, 0.2)",
        stop.replace("y = -1", "y = 1")
    ))
    .exec()
    .unwrap();
    let objects: String = lua
        .load(
            "local p = SENT[1].proposal.streetProposal
             local out = {}
             for _, o in ipairs(p.edgesToAdd[1].comp.objects) do out[#out + 1] = o[1] .. ':' .. o[2] end
             return table.concat(out, ',') .. '|' .. tostring(p.edgeObjectsToAdd[1].left)",
        )
        .eval()
        .unwrap();
    assert_eq!(
        objects, "555:1,-400000000:0|true",
        "the kept stop under its own entity"
    );
    // A place off the edge, as another world would have it: placed nowhere.
    lua.load(format!(
        "SENT = {{}} EDGES[100].objects = {{}} HOOK.batch = {{ {} }} UPDATE({{}}, STATE, 0.2)",
        stop.replace("at = { x = 50,", "at = { x = 53,")
    ))
    .exec()
    .unwrap();
    assert_eq!(lua.load("return #SENT").eval::<usize>().unwrap(), 0);
}

#[test]
fn the_bulldozer_removes_a_stop_in_every_game() {
    let (lua, _script) = engine();
    lua.load(FAKE_NETWORK).exec().unwrap();
    lua.load(FAKE_STOPS).exec().unwrap();
    lua.load(
        "OBJECTS[555] = { transf = { 1,0,0,0, 0,1,0,0, 0,0,1,0, 56,0.2,0,1 }, \
             edgeObjectConstruction = '::/stations/street/small_stops/small_new.con' } \
         OBJECTS[556] = { transf = { 1,0,0,0, 0,1,0,0, 0,0,1,0, 44,0.2,0,1 }, \
             edgeObjectConstruction = '::/stations/street/small_stops/small_new.con' }",
    )
    .exec()
    .unwrap();
    // The bulldozer over stop 555: edge 100 rebuilt with 556 alone.
    let removal: String = lua
        .load(
            "local capture = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua')
             local function seg(e, objects) return { entity = e, type = 0, comp = { node0 = 8, node1 = 9,
                 tangent0 = { x = 0, y = 80, z = 0 }, tangent1 = { x = 0, y = 80, z = 0 },
                 objects = objects } } end
             STOP = capture.bulldoze({ toAdd = {}, toRemove = {}, proposal = { addedNodes = {},
                 removedNodes = {}, removedSegments = { seg(100, { { 555, 0 }, { 556, 1 } }) },
                 addedSegments = { seg(-1, { { 556, 1 } }) }, edgeObjectsToAdd = { { category = 0 } } } })
             local b = STOP.Bulldoze.EdgeObject
             return table.concat({ tostring(schema_check(STOP)), b.edge.network, b.edge.ends.a.y,
                 b.at.x, b.model }, '|')",
        )
        .eval()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        removal,
        "true|Street|-40|56|::/stations/street/small_stops/small_new.con"
    );
    lua.load(
        "EDGES[100].objects = { { 555, 0 }, { 556, 1 } } HOOK.batch = { STOP } \
         UPDATE({}, STATE, 0.2)",
    )
    .exec()
    .unwrap();
    let removed: String = lua
        .load(
            "local p = SENT[1].proposal.streetProposal local e = p.edgesToAdd[1]
             return table.concat({ #SENT, #e.comp.objects, e.comp.objects[1][1],
                 table.concat(p.edgesToRemove, ','), table.concat(p.edgeObjectsToRemove, ','),
                 tostring(SENT[1].context.player) }, '|')",
        )
        .eval()
        .unwrap_or_else(|error| {
            panic!(
                "{error}\n{:?}",
                lua.load("return HOOK.logged").eval::<Vec<String>>()
            )
        });
    assert_eq!(
        removed, "1|1|556|100|555|25",
        "the other stop kept under its own entity"
    );
}

#[test]
fn a_stops_loading_flags_reach_the_game_in_order_however_it_copied_them() {
    let (lua, _script) = engine();
    lua.load(FAKE_FLEET).exec().unwrap();
    // As the game: its binding copies a list it is handed in the order
    // `next` walks it, and the actions reach postUpdate as its own copy of
    // what update returned, whose lists `next` walks in hash order.
    lua.load(
        "api.type.Line = { new = function() return { vehicleInfo = {} } end, \
             Stop = { new = function() return {} end }, \
             StopConfig = { new = function() return setmetatable({}, { __newindex = function(t, k, v) \
                 if type(v) == 'table' then \
                     local copy, key = {}, next(v) \
                     while key ~= nil do copy[#copy + 1] = v[key] key = next(v, key) end \
                     v = copy \
                 end \
                 rawset(t, k, v) end }) end } } \
         api.type.StationTerminal = { new = function(s, t) return { station = s, terminal = t } end } \
         api.cmd.makeLineCreateCmd = function(name, color, player, line) \
             return { createLine = { line = line } } end \
         HOOK.room = true UPDATE({}, STATE, 0.2)",
    )
    .exec()
    .unwrap();
    // Passengers loaded, the other 36 cargos not, in a table with no array
    // part, as the game's copy has it.
    let load = lua.create_table_with_capacity(0, 40).unwrap();
    let max_load = lua.create_table_with_capacity(0, 40).unwrap();
    for cargo in 1..=37 {
        load.raw_set(cargo, cargo == 1).unwrap();
        max_load.raw_set(cargo, f64::from(cargo) / 100.0).unwrap();
    }
    lua.globals().set("LOAD", load).unwrap();
    lua.globals().set("MAX_LOAD", max_load).unwrap();
    let flags: String = lua
        .load(
            "local stop = { group = 0, terminal = { station = 0, terminal = 1 }, alternatives = {}, \
                 load_mode = 'LoadIfAvailable', min_wait = 0, max_wait = 180, max_extra_wait = 30, \
                 rules = { load = LOAD, max_load = MAX_LOAD, force_unload = false, \
                           destroy_for_config_change = false, destroy_for_refresh = false } } \
             HOOK.batch = { { CreateLine = { name = 'Line 1', color = { r = 1, g = 0, b = 0 }, \
                 line = { stops = { stop }, modes = { 3 }, custom_filters = false, \
                          reservation_priority = 0 } } } } \
             UPDATE({}, STATE, 0.2) \
             local c = SENT[1].createLine.line.stops[1].stopConfig \
             local on = {} for i = 1, #c.load do if c.load[i] then on[#on + 1] = i end end \
             return #c.load .. ' on@' .. table.concat(on, ',') .. ' max[5]=' .. c.maxLoad[5]",
        )
        .eval()
        .unwrap_or_else(|error| {
            panic!(
                "{error}\n{:?}",
                lua.load("return HOOK.logged").eval::<Vec<String>>()
            )
        });
    assert_eq!(
        flags, "37 on@1 max[5]=0.05",
        "passengers, as the player set them"
    );
}

#[test]
fn without_callbacks_the_registry_alone_finds_what_an_action_made() {
    let (lua, _script) = engine();
    lua.load(FAKE_FLEET).exec().unwrap();
    lua.load(format!(
        "NO_CALLBACKS = true HOOK.room = true UPDATE({{}}, STATE, 0.2) \
         HOOK.batch = {{ {BUY_BUS} }} UPDATE({{}}, STATE, 0.2) \
         HOOK.batch = {{ {BUY_BUS} }} UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap();
    let (sent, applied, logged): (usize, String, Vec<String>) = lua
        .load(
            "local out = {} for _, a in ipairs(HOOK.applied) do \
                 out[#out + 1] = a.i .. ':' .. tostring(a.ok) .. ':' .. tostring(a.entity) end \
             return #SENT, table.concat(out, ' '), HOOK.logged",
        )
        .eval()
        .unwrap();
    assert_eq!(sent, 2, "each bus bought once, without a callback");
    assert_eq!(
        applied, "1:true:500 1:true:nil",
        "the first found as the registry's new vehicle; the second, the same entity \
         again in this fake, is no new one"
    );
    assert_eq!(
        logged
            .iter()
            .filter(|l| l.contains("takes no command callbacks"))
            .count(),
        1,
        "{logged:?}"
    );
    assert!(
        logged
            .iter()
            .any(|l| l == "action 1 of this step made no vehicles this game could name"),
        "{logged:?}"
    );
}

/// A personal timetable mod's game script (docs/MODS.md): its holds and
/// releases of its own company's vehicles go to the room, once each; what
/// the game's own scripts and shared mods send runs as before.
#[test]
fn a_personal_mods_game_script_hands_its_holds_to_the_room() {
    let lua = gui();
    let (sent, handed, wrapped, logged): (String, Vec<String>, u32, Vec<String>) = lua
        .load(
            r#"
            local modguard = ug_require('tpf3mp_1::/scripts/tpf3mp/modguard.lua')
            SENT, HANDED, LOGGED, CALLERS, NOW, ROOM = {}, {}, {}, {}, 0, true
            local cmd = {
                makeVehicleSetManualDepartureCmd = function(v, m) return { kind = 'manual', v = v, m = m } end,
                makeVehicleTryToDepartCmd = function(v) return { kind = 'depart', v = v } end,
                makeScriptingSendEventCmd = function() return { kind = 'event' } end,
                makeVehicleSellCmd = function() return { kind = 'sell' } end,
                sendCommand = function(c) SENT[#SENT + 1] = c.kind end,
            }
            local wrapped = modguard.install(cmd, {
                inRoom = function() return ROOM end,
                personal = function(mod) return mod == 'celmi_timetables' end,
                callers = function() return CALLERS end,
                command = function(a)
                    local ok, why = schema_check(a)
                    if not ok then error(why) end
                    local c = a.VehicleOp.change
                    if type(c) == 'table' then c = 'ManualDeparture=' .. tostring(c.ManualDeparture) end
                    HANDED[#HANDED + 1] = a.VehicleOp.vehicle .. ':' .. c
                    return true
                end,
                context = { vehicle = function(e) if e == 500 then return 7 elseif e == 600 then return 8 end end },
                mayTouch = function(e)
                    if e == 600 then return false, 'the vehicle belongs to Blue Line' end
                    return true
                end,
                now = function() return NOW end,
                log = function(line) LOGGED[#LOGGED + 1] = line end,
            })
            -- The game's own script and a shared mod: run here, as before.
            cmd.sendCommand(cmd.makeVehicleSetManualDepartureCmd(500, true))
            CALLERS = { 'auto_signals_1' }
            cmd.sendCommand(cmd.makeVehicleTryToDepartCmd(500))
            -- The personal mod: a hold, the same hold again at once, a
            -- release, and a hold again later.
            CALLERS = { 'celmi_timetables' }
            cmd.sendCommand(cmd.makeVehicleSetManualDepartureCmd(500, true))
            cmd.sendCommand(cmd.makeVehicleSetManualDepartureCmd(500, true))
            cmd.sendCommand(cmd.makeVehicleSetManualDepartureCmd(500, false))
            NOW = 6000
            cmd.sendCommand(cmd.makeVehicleSetManualDepartureCmd(500, true))
            -- Another company's vehicle; its events; what it may not do.
            cmd.sendCommand(cmd.makeVehicleTryToDepartCmd(600))
            cmd.sendCommand(cmd.makeScriptingSendEventCmd('', 'celmiTT_held', 'celmiTT_held', {}))
            cmd.sendCommand(cmd.makeVehicleSellCmd({ 500 }))
            -- Through a shared mod's helper, still the personal mod's.
            CALLERS, NOW = { 'shared_lib', 'celmi_timetables' }, 20000
            cmd.sendCommand(cmd.makeVehicleTryToDepartCmd(500))
            -- Outside the room's game, as the game would.
            ROOM = false
            cmd.sendCommand(cmd.makeVehicleTryToDepartCmd(500))
            return table.concat(SENT, ','), HANDED, wrapped, LOGGED
            "#,
        )
        .eval()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(sent, "manual,depart,depart");
    assert_eq!(
        handed,
        [
            "7:ManualDeparture=true",
            "7:ManualDeparture=false",
            "7:ManualDeparture=true",
            "7:Depart"
        ]
    );
    assert_eq!(wrapped, 3, "the factories the fake api.cmd has");
    let refused: Vec<&String> = logged.iter().filter(|l| !l.starts_with("handed")).collect();
    assert_eq!(
        refused,
        [
            "refused makeVehicleTryToDepartCmd for another company's: the vehicle belongs to Blue Line, from the personal mod celmi_timetables",
            "dropped makeScriptingSendEventCmd (heard by this game's scripts only), from the personal mod celmi_timetables",
            "refused a command no factory made, from the personal mod celmi_timetables",
        ]
    );
    assert_eq!(
        logged[0],
        "handed makeVehicleSetManualDepartureCmd from the personal mod celmi_timetables to the room (1 so far)"
    );
}

/// The GUI's guard (guard.lua) and the personal mods' guard (modguard.lua)
/// each keep the sendCommand they found, so on one api.cmd, in either order,
/// neither swallows the other: outside the room's game a command runs once;
/// a personal mod's hold goes to the room once and does not run here; a
/// command the room does not carry is refused once.
#[test]
fn the_gui_guard_and_the_personal_mods_guard_chain_in_either_order() {
    let lua = gui();
    for guard_first in [true, false] {
        let (sent, handed, refused): (String, u32, u32) = lua
            .load(format!(
                r#"
                local guard = ug_require('tpf3mp_1::/scripts/tpf3mp/guard.lua')
                local modguard = ug_require('tpf3mp_1::/scripts/tpf3mp/modguard.lua')
                -- The GUI state's modules, as its script puts them there.
                package.loaded['tpf3mp.capture'] = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua')
                SENT, HANDED, REFUSED, CALLERS, ROOM = {{}}, 0, 0, {{}}, false
                local cmd = {{
                    makeVehicleSetManualDepartureCmd = function(v, m) return {{ kind = 'manual', v = v, m = m }} end,
                    makeVehicleTryToDepartCmd = function(v) return {{ kind = 'depart', v = v }} end,
                    makeScriptingSendEventCmd = function() return {{ kind = 'event' }} end,
                    sendCommand = function(c) SENT[#SENT + 1] = c.kind end,
                }}
                local context = {{ vehicle = function(e) if e == 500 then return 7 end end }}
                local function hand(a)
                    local ok, why = schema_check(a)
                    if not ok then error(why) end
                    HANDED = HANDED + 1
                    return true
                end
                local function onGuard()
                    guard.install(cmd, {{
                        inRoom = function() return ROOM end,
                        command = hand,
                        refused = function() REFUSED = REFUSED + 1 end,
                        later = function(fn) fn() end,
                        context = context,
                        personal = function(mod) return mod == 'celmi_timetables' end,
                        caller = function() return CALLERS[1] end,
                    }})
                end
                local function onModguard()
                    modguard.install(cmd, {{
                        inRoom = function() return ROOM end,
                        personal = function(mod) return mod == 'celmi_timetables' end,
                        callers = function() return CALLERS end,
                        command = hand,
                        context = context,
                        now = function() return 0 end,
                        log = function() end,
                    }})
                end
                if {guard_first} then onGuard() onModguard() else onModguard() onGuard() end
                -- Outside the room's game: runs here, once.
                cmd.sendCommand(cmd.makeVehicleTryToDepartCmd(500))
                ROOM = true
                -- A personal mod's hold: to the room once, not run here.
                CALLERS = {{ 'celmi_timetables' }}
                cmd.sendCommand(cmd.makeVehicleSetManualDepartureCmd(500, true))
                -- A shared mod's event: the room carries none, refused once.
                CALLERS = {{ 'auto_signals_1' }}
                cmd.sendCommand(cmd.makeScriptingSendEventCmd('', 'x', 'y', {{}}))
                return table.concat(SENT, ','), HANDED, REFUSED
                "#
            ))
            .eval()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            (sent.as_str(), handed, refused),
            ("depart", 1, 1),
            "guard first: {guard_first}"
        );
    }
}

/// The GUI's guard names the mod a refused command came from, and lets a
/// personal mod's event to its own game script through, in this game only.
#[test]
fn the_guard_names_the_mod_and_lets_a_personal_mods_events_reach_its_script() {
    let lua = gui();
    lua.load(FAKE_CMD).exec().unwrap();
    let (callers, refused, sent): (String, Vec<String>, String) = lua
        .load(
            r#"
            local guard = ug_require('tpf3mp_1::/scripts/tpf3mp/guard.lua')
            local stack = { { source = '@::/gui/main/engine_react_util.tl' },
                            { source = 'tpf3mp_1::/scripts/tpf3mp/guard.lua' },
                            { source = '@celmi_timetables::/timetable/plugins/shared/helpers.script.tl' },
                            { source = 'gw_big_city_1::/gui/x.script.tl' },
                            { source = '@celmi_timetables::/timetable/x.tl' } }
            local callers = table.concat(guard.callers(function(level) return stack[level] end), ',')
            REFUSED, FROM = {}, 'celmi_timetables'
            guard.install(api.cmd, {
                inRoom = function() return true end,
                refused = function(kind, why, from)
                    REFUSED[#REFUSED + 1] = tostring(kind) .. ' ' .. tostring(from)
                end,
                later = function() end,
                context = {},
                personal = function(mod) return mod == 'celmi_timetables' end,
                shared = function() return { 'tpf3mp_1' } end,
                caller = function() return FROM end,
            })
            api.cmd.sendCommand(api.cmd.makeScriptingSendEventCmd('', 'TimetablesEdit', 'setArrDep', {}))
            FROM = 'gw_big_city_1'
            api.cmd.sendCommand(api.cmd.makeScriptingSendEventCmd('', 'TimetablesEdit', 'setArrDep', {}))
            FROM = nil
            api.cmd.sendCommand(api.cmd.makeScriptingSendEventCmd('', 'TimetablesEdit', 'setArrDep', {}))
            -- Made by the mod, sent by the game's own helper (a window's
            -- commit): still the mod's.
            FROM = 'celmi_timetables'
            local made = api.cmd.makeScriptingSendEventCmd('', 'TimetablesEdit', 'setMinWait', {})
            FROM = nil
            api.cmd.sendCommand(made)
            local sent = {}
            for _, s in ipairs(SENT) do sent[#sent + 1] = s.command.kind .. ':' .. tostring(s.command.id) end
            return callers, REFUSED, table.concat(sent, ',')
            "#,
        )
        .eval()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(callers, "celmi_timetables,gw_big_city_1");
    assert_eq!(
        sent, "event:TimetablesEdit,event:TimetablesEdit",
        "the personal mod's own events only"
    );
    assert_eq!(
        refused,
        [
            "makeScriptingSendEventCmd gw_big_city_1",
            "makeScriptingSendEventCmd nil"
        ]
    );
}

/// A personal mod's events to the game's own scripts (a rank, prospecting,
/// a loan) take the room's way, carried or refused, as a click's would; only
/// an event addressed to the mod's own game script passes, here alone.
#[test]
fn a_personal_mods_events_to_the_games_scripts_take_the_rooms_way() {
    let lua = gui();
    lua.load(FAKE_CMD).exec().unwrap();
    let (sent, handed, refused, own): (String, Vec<String>, Vec<String>, Vec<bool>) = lua
        .load(
            r#"
            local guard = ug_require('tpf3mp_1::/scripts/tpf3mp/guard.lua')
            HANDED, REFUSED = {}, {}
            guard.install(api.cmd, {
                inRoom = function() return true end,
                command = function(action)
                    local kind = next(action)
                    HANDED[#HANDED + 1] = kind
                    return true
                end,
                refused = function(kind, why) REFUSED[#REFUSED + 1] = tostring(why) end,
                later = function() end,
                context = { town = function() return nil end, player = function() return 1 end },
                personal = function(mod) return mod == 'celmi_timetables' end,
                shared = function() return { 'tpf3mp_1', 'other_mod_1' } end,
                caller = function() return 'celmi_timetables' end,
            })
            local ev = api.cmd.makeScriptingSendEventCmd
            api.cmd.sendCommand(ev('', 'Companies', 'applyLevel', { level = 2 }))
            api.cmd.sendCommand(ev('', 'Loan', 'Obtain', { { amount = 1 }, { amount = 2 } }))
            api.cmd.sendCommand(ev('', 'Companies', 'spawnIndustry', { companyEntity = 1 }))
            -- Under its own id, but a name the company script hears whatever
            -- the id, and an id of the game's own: not its own.
            api.cmd.sendCommand(ev('', 'TimetablesEdit', 'company.lockPermits', {}))
            api.cmd.sendCommand(ev('', 'tpf3mp', 'command', {}))
            -- Its own script's.
            api.cmd.sendCommand(ev('', 'TimetablesEdit', 'setArrDep', {}))
            local sent = {}
            for _, s in ipairs(SENT) do sent[#sent + 1] = tostring(s.command.id) .. ':' .. tostring(s.command.name) end
            local shared = { 'tpf3mp_1' }
            local own = {
                guard.ownEvent('celmi_timetables', 'TimetablesEdit', 'setArrDep', shared),
                guard.ownEvent('celmi_timetables', 'celmi_timetables', 'x', shared),
                guard.ownEvent('celmi_timetables', 'Notifications', 'add', shared),
                guard.ownEvent('celmi_timetables', 'OtherModChannel', 'x', shared),
                guard.ownEvent('gw_big_city_1', 'big', 'x', shared),
                -- A shared mod hears the same id: not the personal mod's alone.
                guard.ownEvent('timetables_ui_tweak', 'TimetablesEdit', 'x', { 'celmi_timetables' }),
                -- Without the room's shared list, nothing is its own.
                guard.ownEvent('celmi_timetables', 'TimetablesEdit', 'setArrDep', nil),
            }
            return table.concat(sent, ','), HANDED, REFUSED, own
            "#,
        )
        .eval()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        sent, "TimetablesEdit:setArrDep",
        "only its own event runs here"
    );
    assert_eq!(handed, ["ApplyRank", "Loan"], "carried as a click's are");
    assert_eq!(refused.len(), 3, "{refused:?}");
    assert_eq!(own, [true, true, false, false, false, false, false]);
}

#[test]
fn a_window_hears_of_what_its_command_made_once_its_world_has_it() {
    let lua = gui();
    let heard: String = lua
        .load(
            "local guard = ug_require('tpf3mp_1::/scripts/tpf3mp/guard.lua') \
             guard.CARRY.makeLineCreateCmd = function() return { CreateLine = {} } end \
             local cmd = { makeLineCreateCmd = function(name) return { kind = 'line', name = name } end, \
                           sendCommand = function() end } \
             local tickets = 0 \
             guard.install(cmd, { inRoom = function() return true end, \
                 command = function() tickets = tickets + 1 return true, tickets end, \
                 refused = function() end, later = function(fn) fn() end, context = {} }) \
             HEARD = {} \
             local function hear(data, ok, entities) \
                 HEARD[#HEARD + 1] = tostring(ok) .. ':' .. tostring(entities[1] and entities[1][1]) \
                     .. ':' .. tostring(data.resultEntity) end \
             cmd.sendCommand(cmd.makeLineCreateCmd('Line 1'), hear) \
             cmd.sendCommand(cmd.makeLineCreateCmd('Line 2'), hear) \
             EXISTS = {} \
             local function sees(e) return EXISTS[e] == true end \
             local out = {} \
             out[#out + 1] = guard.deliver(cmd, { { ticket = 1, ok = true, entity = 600 }, \
                                                  { ticket = 2, ok = true } }, sees) \
             out[#out + 1] = #HEARD \
             EXISTS[600] = true \
             out[#out + 1] = guard.deliver(cmd, {}, sees) \
             out[#out + 1] = table.concat(HEARD, ' ') \
             return table.concat(out, ' ')",
        )
        .eval()
        .unwrap();
    // Line 1 made 600, which this world did not show yet; line 2 made
    // nothing the game could name.
    assert_eq!(
        heard, "0 0 2 true:600:600 false:nil:nil",
        "held, in order, until the world has it; a creation that made nothing \
         is answered as failed"
    );
}

/// The GUI of a room with vehicles 500 and 501 bound to ids 3 and 4, each a
/// locomotive (model 41, bought at game time 1000) and a coach (model 42,
/// bought at 2000), as their TRANSPORT_VEHICLE components have them; 502 is
/// a vehicle the registry has no id for.
const FAKE_TRAINS_GUI: &str = r#"
api.cmd.makeVehicleReplaceCmd = function(vehicle, config)
    return { kind = 'replace', vehicle = vehicle, config = config }
end
api.type = { ComponentType = { GAME_SCRIPT = 7, TRANSPORT_VEHICLE = 4 } }
local function train()
    return { transportVehicleConfig = { vehicles = {
        { part = { modelId = 41 }, purchaseTime = 1000 },
        { part = { modelId = 42 }, purchaseTime = 2000 },
    } } }
end
api.engine = {
    getComponent = function(e, kind)
        if kind == 7 and e == 77 then return { state = { registry = {
            vehicles = { next = 5, bound = { { 3, 500 }, { 4, 501 } } },
            lines = { next = 0, bound = {} }, groups = { next = 0, bound = {} } } } } end
        if kind == 4 and (e == 500 or e == 501 or e == 502) then return train() end
    end,
    system = {
        gameScriptSystem = { getEntityForGameScript = function(name)
            if name == 'tpf3mp_1::/tpf3mp_sim/tpf3mp_sim.gs' then return 77 end return -1 end },
    },
}
local NAMES = { [41] = 'vehicle/train/loco.mdl', [42] = 'vehicle/waggon/coach.mdl' }
api.res = { modelRep = { getName = function(id) return NAMES[id] end } }
-- A part as the store hands it on: the vehicle's own, or new, bought at
-- the GUI's game time (HandleVehicleChanges sets it before it sends).
function PART(model, purchased, reversed)
    return { part = { modelId = model, reversed = reversed == true,
                      compartment2loadConfig = { { loadConfigIndex = 0, cargoTypeId = 0 } },
                      color = { x = 1, y = 0.5, z = 0 } },
             purchaseTime = purchased, autoLoadConfig = { true } }
end
"#;

/// The GUI with FAKE_TRAINS_GUI, in the room's game.
fn trains_gui() -> Lua {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_CMD).exec().unwrap();
    lua.load(FAKE_TRAINS_GUI).exec().unwrap();
    lua.load("M = mount(loadPlugin()) M.step() HOOK.room = true")
        .exec()
        .unwrap();
    lua
}

#[test]
fn a_group_replacement_goes_to_the_room_vehicle_by_vehicle_by_canonical_ids() {
    let lua = trains_gui();
    // The vehicle window's "modify" on two trains at once, as
    // vehicle_react_util.tl sends it: one command per vehicle, no callback.
    // The first keeps its locomotive (turned round) and coach and gets a
    // new coach; the second has its own two parts the other way round.
    lua.load(
        "local first = { vehicles = { PART(41, 1000, true), PART(42, 2000), PART(42, 5000) }, \
                         vehicleGroups = { 1, 1, 1 }, muFileNames = { '', '', '' } } \
         local second = { vehicles = { PART(42, 2000), PART(41, 1000) }, \
                          vehicleGroups = { 1, 1 }, muFileNames = { '', '' } } \
         api.cmd.sendCommand(api.cmd.makeVehicleReplaceCmd(500, first)) \
         api.cmd.sendCommand(api.cmd.makeVehicleReplaceCmd(501, second)) \
         M.step()",
    )
    .exec()
    .unwrap();
    let (handed, sent): (usize, usize) = lua.load("return #HOOK.commands, #SENT").eval().unwrap();
    assert_eq!(handed, 2, "every vehicle of the group goes to the room");
    assert_eq!(sent, 0, "none is sent here: the room orders it");
    let captured: String = lua
        .load(
            "local out = {} \
             for _, c in ipairs(HOOK.commands) do \
                 local r = c.ReplaceVehicle local parts = {} \
                 for _, p in ipairs(r.consist) do \
                     parts[#parts + 1] = p.part.model .. (p.part.reversed and '<' or '>') \
                         .. tostring(p.kept) end \
                 out[#out + 1] = r.vehicle .. '=' .. table.concat(parts, ',') \
                     .. '/' .. #r.groups .. '/' .. r.consist[1].part.color.g end \
             return table.concat(out, ' ')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        captured,
        "3=vehicle/train/loco.mdl<0,vehicle/waggon/coach.mdl>1,vehicle/waggon/coach.mdl>nil/3/0.5 \
         4=vehicle/waggon/coach.mdl>1,vehicle/train/loco.mdl>0/2/0.5",
        "by canonical id; a part the vehicle has is kept by its index, a bought one is new"
    );
}

#[test]
fn a_replacement_the_room_cannot_name_is_refused_with_why() {
    let lua = trains_gui();
    // A vehicle with no canonical id; a model with no name; no parts.
    lua.load(
        "CALLED = nil \
         api.cmd.sendCommand(api.cmd.makeVehicleReplaceCmd(502, { vehicles = { PART(41, 1000) }, \
             vehicleGroups = { 1 }, muFileNames = { '' } }), function(_, ok) CALLED = ok end) \
         api.cmd.sendCommand(api.cmd.makeVehicleReplaceCmd(500, { vehicles = { PART(99, 5000) }, \
             vehicleGroups = { 1 }, muFileNames = { '' } })) \
         api.cmd.sendCommand(api.cmd.makeVehicleReplaceCmd(500, { vehicles = {}, \
             vehicleGroups = {}, muFileNames = {} })) \
         M.step()",
    )
    .exec()
    .unwrap();
    let (handed, sent, called): (usize, usize, bool) = lua
        .load("return #HOOK.commands, #SENT, CALLED")
        .eval()
        .unwrap();
    assert_eq!((handed, sent), (0, 0), "nothing goes anywhere");
    assert!(!called, "a callback hears it failed");
    assert_eq!(
        shown(&lua).as_deref(),
        Some("Not in multiplayer yet: replacing vehicles")
    );
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    for why in [
        "(1 so far): a vehicle the room cannot name",
        "(2 so far): a vehicle model the room cannot name",
        "(3 so far): a replacement of no vehicles",
    ] {
        let line = format!("refused the player's makeVehicleReplaceCmd in the room's game {why}");
        assert!(logged.contains(&line), "{line} not in {logged:?}");
    }
}

#[test]
fn a_replacement_with_a_callback_hears_the_vehicle_as_it_is_after() {
    let lua = trains_gui();
    lua.load(
        "api.engine.entityExists = function() return true end \
         HEARD = nil \
         api.cmd.sendCommand(api.cmd.makeVehicleReplaceCmd(500, { vehicles = { PART(41, 1000) }, \
             vehicleGroups = { 1 }, muFileNames = { '' } }), function(data, ok, entities) \
             HEARD = data.vehicleEntity .. ':' .. tostring(ok) .. ':' .. entities[1][1] \
                 .. ':' .. #data.config.vehicles end) \
         M.step() \
         HOOK.results = { { ticket = 1, ok = true, entity = 500 } } M.step()",
    )
    .exec()
    .unwrap();
    let heard: String = lua.load("return HEARD").eval().unwrap();
    assert_eq!(heard, "500:true:500:1");
}

/// FAKE_FLEET's vehicle 401 as a train: a locomotive (model 41) bought at
/// 1000 and worn to 0.7, and a coach (42). A replacement is sent as the
/// game's command; REPLACE_AS makes the game give the vehicle a new entity.
const FAKE_TRAINS: &str = r#"
REPLACE_AS = nil
OWN = { [401] = {
    { part = { modelId = 41 }, purchaseTime = 1000, maintenanceState = 0.7, maintenanceChange = 0.01 },
    { part = { modelId = 42 }, purchaseTime = 2000, maintenanceState = 0.9, maintenanceChange = 0.02 },
} }
local component = api.engine.getComponent
api.engine.getComponent = function(e, kind)
    local c = component(e, kind)
    if c and kind == api.type.ComponentType.TRANSPORT_VEHICLE then
        c.transportVehicleConfig = { vehicles = OWN[e] or {} }
    end
    return c
end
local find = api.res.modelRep.find
api.res.modelRep.find = function(name)
    if name == 'vehicle/train/loco.mdl' then return 41 end
    if name == 'vehicle/waggon/coach.mdl' then return 42 end
    return find(name)
end
api.cmd.makeVehicleReplaceCmd = function(vehicle, config)
    return { replace = { vehicle = vehicle, config = config }, vehicleEntity = vehicle }
end
local send = api.cmd.sendCommand
api.cmd.sendCommand = function(command, ...)
    if command.replace and REPLACE_AS then
        for i, e in ipairs(VEHICLES) do
            if e == command.replace.vehicle then table.remove(VEHICLES, i) break end
        end
        VEHICLES[#VEHICLES + 1] = REPLACE_AS
        command.made = REPLACE_AS
    end
    send(command, ...)
end
"#;

/// A replacement of `vehicle`: its locomotive kept (as part `kept`) and
/// turned, as `loco`; its coach left out, and a new coach of model `coach`.
fn replace_train(vehicle: u32, loco: &str, kept: u32, coach: &str) -> String {
    format!(
        "{{ ReplaceVehicle = {{ vehicle = {vehicle}, \
            consist = {{ \
                {{ part = {{ model = '{loco}', reversed = true, loads = {{}}, \
                            color = {{ r = 1, g = 0, b = 0 }} }}, kept = {kept} }}, \
                {{ part = {{ model = '{coach}', reversed = false, \
                            loads = {{ {{ config = 1, cargo = 0 }} }}, color = {{ r = 0, g = 0, b = 1 }} }} }} }}, \
            groups = {{ 1, 1 }}, multiple_units = {{ '', '' }} }} }}"
    )
}

const LOCO: &str = "vehicle/train/loco.mdl";
const COACH: &str = "vehicle/waggon/coach.mdl";

/// The game script in a room whose first update bound 401 and 402 to
/// vehicles 0 and 1, with `setup` run before the room orders `batch`.
fn replay(setup: &str, batch: &[String]) -> Lua {
    let (lua, _script) = engine();
    lua.load(FAKE_FLEET).exec().unwrap();
    lua.load(FAKE_TRAINS).exec().unwrap();
    lua.load(format!(
        "HOOK.room = true UPDATE({{}}, STATE, 0.2) {setup} \
         HOOK.batch = {{ {} }} UPDATE({{}}, STATE, 0.2)",
        batch.join(", ")
    ))
    .exec()
    .unwrap();
    lua
}

#[test]
fn every_game_replaces_the_vehicle_with_its_own_parts_kept_and_new_ones_bought() {
    let lua = replay("", &[replace_train(0, LOCO, 0, COACH)]);
    let sent: String = lua
        .load(
            "local r = SENT[1].replace local out = { r.vehicle } \
             for _, p in ipairs(r.config.vehicles) do \
                 out[#out + 1] = p.part.modelId .. (p.part.reversed and '<' or '>') .. p.purchaseTime \
                     .. '@' .. tostring(p.maintenanceState) .. '/' .. tostring(p.autoLoadConfig[1]) end \
             out[#out + 1] = r.config.vehicleGroups[2] .. r.config.muFileNames[2] \
             return table.concat(out, ' ')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        sent, "401 41<1000@0.7/nil 42>777000@nil/true 1",
        "the locomotive keeps its purchase time and wear, turned as the player chose; \
         the coach is bought now"
    );
    let (applied, id, next, logged): (String, u32, u32, Vec<String>) = lua
        .load(
            "local registry = ug_require('tpf3mp_1::/scripts/tpf3mp/registry.lua') \
             local reg = STATE.value.registry local a = HOOK.applied[1] \
             return tostring(a.ok) .. ':' .. tostring(a.entity), registry.id(reg, 'vehicles', 401), \
                 reg.vehicles.next, HOOK.logged",
        )
        .eval()
        .unwrap();
    assert_eq!(applied, "true:401", "the vehicle is itself still");
    assert_eq!((id, next), (0, 2), "it keeps its id; no id is used up");
    assert!(
        logged.contains(&"replacing vehicle 0 (entity 401): 2 part(s), 1 kept".to_owned()),
        "{logged:?}"
    );
}

#[test]
fn a_vehicle_the_game_makes_anew_keeps_its_id() {
    let lua = replay("REPLACE_AS = 450", &[replace_train(0, LOCO, 0, COACH)]);
    let named: String = lua
        .load(
            "local registry = ug_require('tpf3mp_1::/scripts/tpf3mp/registry.lua') \
             local reg = STATE.value.registry \
             return table.concat({ tostring(HOOK.applied[1].entity), registry.id(reg, 'vehicles', 450), \
                 tostring(registry.id(reg, 'vehicles', 401)), registry.id(reg, 'vehicles', 402), \
                 reg.vehicles.next }, ' ')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        named, "450 0 nil 1 2",
        "vehicle-0 names the new entity; the old is gone, and no new id is made"
    );
}

#[test]
fn a_replacement_that_does_not_fit_this_world_is_applied_nowhere() {
    // Keeping a part of another model, a part the vehicle does not have; a
    // model this game lacks; a vehicle with no id.
    let lua = replay(
        "",
        &[
            replace_train(0, COACH, 0, COACH),
            replace_train(0, LOCO, 5, COACH),
            replace_train(0, LOCO, 0, "vehicle/waggon/tender.mdl"),
            replace_train(9, LOCO, 0, COACH),
        ],
    );
    let (sent, why): (usize, Vec<String>) = lua
        .load(
            "local why = {} for _, a in ipairs(HOOK.applied) do \
                 why[#why + 1] = tostring(a.ok) .. ': ' .. tostring(a.why) end \
             return #SENT, why",
        )
        .eval()
        .unwrap();
    assert_eq!(sent, 0, "nothing sent");
    assert_eq!(
        why,
        [
            "false: part 1 keeps a part of another model",
            "false: part 1 keeps a part the vehicle does not have",
            "false: no vehicle model vehicle/waggon/tender.mdl",
            "false: no vehicles 9 in this world",
        ]
    );
}

/// The construction menu's prospection, as it sends it to the company
/// script (`construction_react_util.tl`, `ProspectionActionRecipe`): coal
/// near town 7, the industry types in the menu's order.
const SPAWN_INDUSTRY: &str = "{ companyEntity = 25, townEntity = 7, \
    types = { 'coal_mine_large', 'coal_mine' }, \
    permitKey = 'game_mechanics/company/explorations/exploration_coal.res', \
    cargoType = '::/cargos/coal/coal.cargo' }";

#[test]
fn a_prospection_goes_to_the_room_by_its_towns_id_and_its_types_in_order() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_CMD).exec().unwrap();
    // The GUI reads the game script's registry from its state: town 7 is
    // town-3.
    lua.load(
        "api.type = { ComponentType = { GAME_SCRIPT = 7 } } \
         api.engine = { \
             util = { getPlayer = function() return 25 end }, \
             getComponent = function(e, kind) \
                 if kind == 7 and e == 77 then return { state = { registry = { \
                     vehicles = { next = 0, bound = {} }, lines = { next = 0, bound = {} }, \
                     groups = { next = 0, bound = {} }, towns = { next = 4, bound = { { 3, 7 } } }, \
                     industries = { next = 0, bound = {} } } } } end \
             end, \
             system = { gameScriptSystem = { getEntityForGameScript = function(name) \
                 if name == 'tpf3mp_1::/tpf3mp_sim/tpf3mp_sim.gs' then return 77 end return -1 end } }, \
         } \
         M = mount(loadPlugin()) M.step() HOOK.room = true",
    )
    .exec()
    .unwrap();
    lua.load(format!(
        "UNLOCKED = nil \
         api.cmd.sendCommand(api.cmd.makeScriptingSendEventCmd('', 'Companies', 'spawnIndustry', {SPAWN_INDUSTRY}), \
             function() UNLOCKED = true end) \
         M.step()"
    ))
    .exec()
    .unwrap();
    let (sent, handed, unlocked): (usize, usize, bool) = lua
        .load("return #SENT, #HOOK.commands, UNLOCKED ~= nil")
        .eval()
        .unwrap();
    assert_eq!(sent, 0, "not run here: the room orders it for every game");
    assert_eq!(handed, 1, "handed to the room, through the schema");
    assert!(
        !unlocked,
        "the menu's permits stay reserved until it ran here"
    );
    let prospect: String = lua
        .load(
            "local p = HOOK.commands[1].Prospect \
             return table.concat({ p.town, p.cargo, p.industries[1], p.industries[2], p.permit }, '|')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        prospect,
        "3|::/cargos/coal/coal.cargo|coal_mine_large|coal_mine|\
         game_mechanics/company/explorations/exploration_coal.res"
    );
    // This game applied it: the menu hears so and gives back its reservation.
    lua.load("HOOK.results = { { ticket = 1, ok = true } } M.step()")
        .exec()
        .unwrap();
    assert!(lua.load("return UNLOCKED == true").eval::<bool>().unwrap());

    // A town the room cannot name, another company's prospection, or one
    // that can find nothing, is refused, and says why.
    lua.load(format!(
        "local p = {SPAWN_INDUSTRY} p.townEntity = 8 \
         api.cmd.sendCommand(api.cmd.makeScriptingSendEventCmd('', 'Companies', 'spawnIndustry', p)) \
         local q = {SPAWN_INDUSTRY} q.companyEntity = 26 \
         api.cmd.sendCommand(api.cmd.makeScriptingSendEventCmd('', 'Companies', 'spawnIndustry', q)) \
         local r = {SPAWN_INDUSTRY} r.types = {{}} \
         api.cmd.sendCommand(api.cmd.makeScriptingSendEventCmd('', 'Companies', 'spawnIndustry', r)) \
         M.step()"
    ))
    .exec()
    .unwrap();
    let (sent, handed): (usize, usize) = lua.load("return #SENT, #HOOK.commands").eval().unwrap();
    assert_eq!((sent, handed), (0, 1));
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    for why in [
        "a town the room cannot name",
        "prospecting for another company",
        "a prospection that can find no industry",
    ] {
        assert!(
            logged
                .iter()
                .any(|l| l.contains("makeScriptingSendEventCmd") && l.ends_with(why)),
            "{why}: {logged:?}"
        );
    }
    // Taking a rank goes to the room too (tpf3mp/progression.lua); the
    // company's other events stay refused.
    lua.load(
        "api.cmd.sendCommand(api.cmd.makeScriptingSendEventCmd('', 'Companies', 'applyLevel', { level = 2 }))          api.cmd.sendCommand(api.cmd.makeScriptingSendEventCmd('', 'Companies', 'applyLevel', { level = 2.5 }))          api.cmd.sendCommand(api.cmd.makeScriptingSendEventCmd('', 'Companies', 'MakeGreen', {}))",
    )
    .exec()
    .unwrap();
    let (handed, level): (usize, u32) = lua
        .load("return #HOOK.commands, HOOK.commands[2].ApplyRank.level")
        .eval()
        .unwrap();
    assert_eq!((handed, level), (2, 2));
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert!(
        logged.iter().any(|l| l.ends_with("a rank of 2.5")),
        "{logged:?}"
    );
}

/// Towns and industries for the prospecting tests, over the stand-in engine
/// state: towns 7 and 5, and a coal mine whose INDUSTRY part 931 is in
/// construction 930.
const FAKE_TOWNS: &str = r#"
local CT = { CONSTRUCTION = 2, TOWN = 12, INDUSTRY = 13, GAME_TIME = 10 }
api.type.ComponentType = CT
TOWNS, PARTS = { 7, 5 }, { 931 }
CONS = { [930] = { fileName = 'industry/coal_mine.con',
                   transf = { 1,0,0,0, 0,1,0,0, 0,0,1,0, 1234.5, -250.25, 10, 1 } } }
api.engine.getEntitiesWithComponent = function(kind)
    if kind == CT.TOWN then return TOWNS end
    if kind == CT.INDUSTRY then return PARTS end
    return {}
end
api.engine.getComponent = function(e, kind)
    if kind == CT.CONSTRUCTION then return CONS[e] end
    if kind == CT.TOWN then for _, t in ipairs(TOWNS) do if t == e then return {} end end end
end
api.engine.system = {
    lineSystem = { getLines = function() return {} end },
    streetConnectorSystem = { getConstructionEntityForSubconstruction = function(part)
        if part == 931 then return 930 end
        if part == 941 then return 940 end
        return -1
    end },
}
"#;

#[test]
fn every_game_prospects_through_the_company_scripts_own_event() {
    let (lua, _script) = engine();
    lua.load(FAKE_TOWNS).exec().unwrap();
    // The room's first update binds the towns, lowest entity first: 5 is
    // town-0, 7 town-1. Then the room's prospection near town-1, and one
    // near a town this world has not.
    lua.load(
        "HOOK.room = true UPDATE({}, STATE, 0.2) \
         HOOK.batch = { { Prospect = { town = 1, cargo = '::/cargos/coal/coal.cargo', \
             industries = { 'coal_mine_large', 'coal_mine' }, permit = 'coal.res' } }, \
             { Prospect = { town = 9, cargo = 'c', industries = { 'x' } } } } \
         UPDATE({}, STATE, 0.2)",
    )
    .exec()
    .unwrap();
    let event: String = lua
        .load(
            "local e = SENT[1].event local p = e.param \
             return table.concat({ e.src, e.id, e.name, p.companyEntity, p.townEntity, \
                 p.types[1], p.types[2], #p.types, p.permitKey, p.cargoType }, '|')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        event,
        "|Companies|spawnIndustry|25|7|coal_mine_large|coal_mine|2|coal.res|::/cargos/coal/coal.cargo",
        "the player's company, town 7, the types in order"
    );
    let applied: String = lua
        .load(
            "local out = {} for _, a in ipairs(HOOK.applied) do \
                 out[#out + 1] = a.i .. ':' .. tostring(a.ok) .. ':' .. tostring(a.why) end \
             return table.concat(out, ' ')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        applied, "1:true:nil 2:false:no towns 9 in this world",
        "a town this world has not is refused, the same in every game"
    );
    assert_eq!(lua.load("return #SENT").eval::<usize>().unwrap(), 1);
    // The industry that stood at the start is industry-0.
    let first: u32 = lua
        .load(
            "return ug_require('tpf3mp_1::/scripts/tpf3mp/registry.lua').id(STATE.value.registry, 'industries', 930)",
        )
        .eval()
        .unwrap();
    assert_eq!(first, 0);
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert!(
        logged.contains(
            &"prospecting for ::/cargos/coal/coal.cargo near town-1 (7): coal_mine_large, coal_mine"
                .to_owned()
        ),
        "{logged:?}"
    );
}

#[test]
fn a_prospection_found_is_said_and_its_industry_named_alike_in_every_game() {
    let (lua, script) = engine();
    lua.load(FAKE_TOWNS).exec().unwrap();
    let handle: Function = script.get("handleEvent").unwrap();
    lua.globals().set("HANDLE", handle).unwrap();
    // Subscribed to the company script's two events.
    lua.load("HOOK.room = true UPDATE({}, STATE, 0.2)")
        .exec()
        .unwrap();
    assert!(
        lua.load("return STATE.subscribed.startProspection and STATE.subscribed.endProspection")
            .eval::<bool>()
            .unwrap()
    );
    // As the company script sends them: begun, one found nothing, and one
    // found a new industry (part 941 of construction 940).
    lua.load(
        "HANDLE({}, STATE, '', 'Company', 'startProspection', \
             { entity = 7, initiatedTimestamp = 3600000, cargoType = 'coal' }) \
         HANDLE({}, STATE, '', 'Company', 'endProspection', \
             { entity = { entity = 5, index = 0 }, initiatedTimestamp = 100, cargoType = 'grain', success = false }) \
         PARTS = { 931, 941 } \
         CONS[940] = { fileName = 'industry/coal_mine_large.con', \
                       transf = { 1,0,0,0, 0,1,0,0, 0,0,1,0, -40, 80.04, 3, 1 } } \
         HANDLE({}, STATE, '', 'Company', 'endProspection', \
             { entity = { entity = 7, index = 0 }, initiatedTimestamp = 3600000, cargoType = 'coal', success = true }) \
         HANDLE({}, STATE, '', 'Loan', 'endProspection', { success = true })",
    )
    .exec()
    .unwrap();
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    let said: Vec<&String> = logged
        .iter()
        .filter(|l| l.starts_with("prospecting"))
        .collect();
    assert_eq!(
        said,
        [
            "prospecting began: coal near town-1 at game time 3600000",
            "prospecting ended: grain near town-0, begun at game time 100, found nothing",
            "prospecting ended: coal near town-1, begun at game time 3600000, \
             found industry-1 industry/coal_mine_large.con at (-40.0, 80.0)",
        ]
    );
    let named: u32 = lua
        .load(
            "return ug_require('tpf3mp_1::/scripts/tpf3mp/registry.lua').id(STATE.value.registry, 'industries', 940)",
        )
        .eval()
        .unwrap();
    assert_eq!(named, 1, "bound in the saved registry at once");
}

#[test]
fn a_registry_from_an_older_mod_gains_the_towns_at_the_rooms_next_update() {
    let (lua, _script) = engine();
    lua.load(FAKE_TOWNS).exec().unwrap();
    lua.load(
        "STATE.value = { registry = { vehicles = { next = 2, bound = {} }, \
             lines = { next = 0, bound = {} }, groups = { next = 0, bound = {} } } } \
         HOOK.room = true UPDATE({}, STATE, 0.2)",
    )
    .exec()
    .unwrap();
    let (town, vehicles): (u32, u32) = lua
        .load(
            "local reg = STATE.value.registry \
             return ug_require('tpf3mp_1::/scripts/tpf3mp/registry.lua').id(reg, 'towns', 7), reg.vehicles.next",
        )
        .eval()
        .unwrap();
    assert_eq!((town, vehicles), (1, 2), "the towns bound, the rest kept");
    let work: mlua::Value = lua.load("return UPDATE({}, STATE, 0.2)").eval().unwrap();
    assert!(work.is_nil(), "once");
}

#[test]
fn the_game_script_seeds_math_random_with_the_room_steps_seed_each_update() {
    let (lua, _script) = engine();
    // Two games at the same step draw the same numbers; outside the room's
    // steps nothing is seeded.
    let draws: (f64, f64, f64) = lua
        .load(
            "HOOK.seed = 12345 UPDATE({}, STATE, 0.2) local a = math.random()              math.randomseed(999) math.random()              UPDATE({}, STATE, 0.2) local b = math.random()              HOOK.seed = nil math.randomseed(7) UPDATE({}, STATE, 0.2)              local c = math.random() math.randomseed(7)              return a, b, c - math.random()",
        )
        .eval()
        .unwrap();
    assert_eq!(draws.0, draws.1, "the same step's seed, the same draws");
    assert_eq!(draws.2, 0.0, "no seed: the state's own sequence goes on");
}

/// Pure: the roster of companies (tpf3mp/companies.lua).
#[test]
fn companies_are_founded_joined_renamed_recoloured_and_dissolved_alike() {
    let lua = gui();
    lua.load(
        r#"
        COMP = { [2] = { [700] = { player = 901 }, [701] = { player = 25 }, [702] = { player = -1 } } }
        api = {
            engine = {
                util = { getPlayer = function() return 25 end },
                getComponent = function(e, kind) return COMP[kind] and COMP[kind][e] end,
                -- The entity only, as build 40408 calls the function (seen
                -- in its console: the second argument is nil).
                forEachEntityWithComponent = function(fn, kind)
                    for e in pairs(COMP[kind] or {}) do fn(e) end
                end,
            },
            type = { ComponentType = { NAME = 1, PLAYER_OWNED = 2 },
                     Vec3f = { new = function(x, y, z) return { x, y, z } end } },
            cmd = {
                makeGameAddPlayerCmd = function(name, color) return { add = name, color = color } end,
                makeEntitySetNameCmd = function(e, name) return { rename = e, name = name } end,
            },
        }
        SENT, NEXT = {}, 900
        function send(cmd)
            SENT[#SENT + 1] = cmd
            if cmd.add then NEXT = NEXT + 1 return { resultEntity = NEXT } end
        end
        C = ug_require("tpf3mp_1::/scripts/tpf3mp/companies.lua")
        A, B, D = string.rep("a", 64), string.rep("b", 64), string.rep("d", 64)
        R = C.ensure(nil, api)
        "#,
    )
    .exec()
    .unwrap();
    let eval = |code: &str| -> String {
        lua.load(code)
            .eval::<String>()
            .unwrap_or_else(|error| panic!("{code}: {error}"))
    };
    // Everyone plays for the save's own company until they choose.
    assert_eq!(
        eval("return C.of(R, A).id .. ' ' .. C.of(R, A).entity"),
        "0 25"
    );
    // A founds Rival: a new player entity, the next colour, and A plays for it.
    assert_eq!(
        eval(
            "local ok, why, id = C.run(R, A, { Create = { name = ' Rival ' } }, send, api) \
             return tostring(ok) .. ' ' .. tostring(id)"
        ),
        "true 1"
    );
    assert_eq!(
        eval(
            "local c = C.of(R, A) \
             return c.name .. ' ' .. c.entity .. ' ' .. SENT[1].add .. ' ' .. SENT[1].color[3]"
        ),
        "Rival 901 Rival 0.85"
    );
    assert_eq!(
        eval("return tostring(C.of(R, B).id)"),
        "0",
        "B still plays for the first"
    );
    // B joins Rival: two players in one company, D alone in the first.
    assert_eq!(
        eval("return tostring(C.run(R, B, { Join = 1 }, send, api))"),
        "true"
    );
    assert_eq!(eval("return C.of(R, B).id .. ' ' .. C.of(R, D).id"), "1 0");
    // Only its players rename or recolour a company; names stay unique.
    assert_eq!(
        eval(
            "local ok, why = C.run(R, D, { Rename = { company = 1, name = 'Mine' } }, send, api) \
             return why"
        ),
        "only its players rename a company"
    );
    assert_eq!(
        eval("local ok, why = C.run(R, A, { Create = { name = 'rival' } }, send, api) return why"),
        "a company is called rival already"
    );
    assert_eq!(
        eval(
            "C.run(R, B, { Rename = { company = 1, name = 'Blue Line' } }, send, api) \
             return C.find(R, 1).name .. ' ' .. SENT[#SENT].rename"
        ),
        "Blue Line 901"
    );
    assert_eq!(
        eval(
            "C.run(R, A, { Recolor = { company = 1, color = { r = 0.1, g = 0.2, b = 0.3 } } }, send, api) \
             return tostring(C.find(R, 1).color[2])"
        ),
        "0.2"
    );
    // What another company owns is refused, naming it; its own and no
    // one's are not.
    assert_eq!(
        eval(
            "local ok, why = C.mayTouch(R, 25, 700, api, 'vehicle') \
             return tostring(ok) .. ' ' .. why"
        ),
        "false the vehicle belongs to Blue Line"
    );
    assert_eq!(
        eval(
            "return tostring(C.mayTouch(R, 25, 701, api)) .. tostring(C.mayTouch(R, 25, 702, api)) \
             .. tostring(C.mayTouch(R, 25, 703, api))"
        ),
        "truetruetrue"
    );
    // Its last player dissolves a company that owns nothing, and plays for
    // the first again; nobody dissolves the first.
    assert_eq!(
        eval("local ok, why = C.run(R, D, { Delete = 1 }, send, api) return why"),
        "only its players dissolve a company"
    );
    assert_eq!(
        eval("local ok, why = C.run(R, A, { Delete = 1 }, send, api) return why"),
        "others still play for Blue Line"
    );
    assert_eq!(
        eval("local ok, why = C.run(R, D, { Delete = 0 }, send, api) return why"),
        "the room's first company stays"
    );
    assert_eq!(
        eval(
            "C.run(R, B, { Join = 0 }, send, api) \
             local ok, why = C.run(R, A, { Delete = 1 }, send, api) return why"
        ),
        "Blue Line still owns something"
    );
    assert_eq!(
        eval(
            "COMP[2][700] = nil \
             return tostring(C.run(R, A, { Delete = 1 }, send, api)) .. ' ' .. #C.live(R) \
             .. ' ' .. C.of(R, A).id"
        ),
        "true 1 0"
    );
    assert_eq!(
        eval("local ok, why = C.run(R, A, { Join = 1 }, send, api) return why"),
        "there is no company 1"
    );
    // At most MAX companies.
    assert_eq!(
        eval(
            "for i = 1, C.MAX do C.run(R, A, { Create = { name = 'C' .. i } }, send, api) end \
             return #C.live(R) .. ' ' .. select(2, C.run(R, A, { Create = { name = 'X' } }, send, api))"
        ),
        "8 the room has 8 companies already"
    );
}

/// Pure: who may do what to a company (DECISIONS.md, D22, proposed). A
/// password is the room's seal of it; its head alone locks and unlocks it,
/// sends players out and opens or closes its stations; the head's place
/// passes to the longest-standing player when the founder leaves; the room's
/// first company is everyone's.
#[test]
fn a_companys_head_locks_it_and_only_its_password_opens_it() {
    let lua = gui();
    lua.load(
        r#"
        COMP = { [2] = { [800] = { player = 901 }, [801] = { player = 25 }, [802] = {} } }
        api = {
            engine = {
                util = { getPlayer = function() return 25 end },
                getComponent = function(e, kind) return COMP[kind] and COMP[kind][e] end,
                forEachEntityWithComponent = function(fn, kind)
                    for e in pairs(COMP[kind] or {}) do fn(e) end
                end,
            },
            type = { ComponentType = { NAME = 1, PLAYER_OWNED = 2, TRANSPORT_VEHICLE = 4 },
                     Vec3f = { new = function(x, y, z) return { x, y, z } end } },
            cmd = {
                makeGameAddPlayerCmd = function(name, color) return { add = name } end,
                makeEntitySetColorCmd = function(e, color) return { paint = e } end,
            },
        }
        SENT, NEXT = {}, 900
        function send(cmd)
            SENT[#SENT + 1] = cmd
            if cmd.add then NEXT = NEXT + 1 return { resultEntity = NEXT } end
        end
        C = ug_require("tpf3mp_1::/scripts/tpf3mp/companies.lua")
        JAMES, BOB, CAT = string.rep("a", 64), string.rep("b", 64), string.rep("c", 64)
        R = C.ensure(nil, api)
        function seal(scope, byte) return { scope = scope, tag = string.rep(byte, 64) } end
        function why(player, op, s)
            local ok, reason = C.run(R, player, op, send, api, s)
            return ok and "ok" or reason
        end
        -- James founds Rival (company 1, entity 901).
        C.run(R, JAMES, { Create = { name = 'Rival' } }, send, api)
        "#,
    )
    .exec()
    .unwrap();
    let eval = |code: &str| -> String {
        lua.load(code)
            .eval::<String>()
            .unwrap_or_else(|error| panic!("{code}: {error}"))
    };
    assert_eq!(eval("return C.head(R, 1)"), eval("return JAMES"));
    // Only its head locks it, and only with the room's seal for it.
    assert_eq!(
        eval("return why(BOB, { Lock = 1 }, seal(1, 'e'))"),
        "only the head of Rival gives a password to it"
    );
    assert_eq!(
        eval("return why(JAMES, { Lock = 1 })"),
        "a password for Rival comes sealed by the room"
    );
    assert_eq!(
        eval("return why(JAMES, { Lock = 1 }, seal(2, 'e'))"),
        "a password for Rival comes sealed by the room",
        "a seal made for another company"
    );
    assert_eq!(eval("return why(JAMES, { Lock = 1 }, seal(1, 'e'))"), "ok");
    // Joining it needs the password: none, a wrong one, then the right one.
    assert_eq!(
        eval("return why(BOB, { Join = 1 })"),
        "joining Rival needs its password"
    );
    assert_eq!(
        eval("return why(BOB, { Join = 1 }, seal(1, 'f'))"),
        "the password for Rival is not right"
    );
    assert_eq!(eval("return why(BOB, { Join = 1 }, seal(1, 'e'))"), "ok");
    assert_eq!(eval("return why(CAT, { Join = 1 }, seal(1, 'e'))"), "ok");
    // A player is no head: Bob cannot send Cat out or close the stations.
    assert_eq!(
        eval("return why(BOB, { Dismiss = { company = 1, player = CAT } })"),
        "only the head of Rival sends players out of it"
    );
    assert_eq!(
        eval("return why(BOB, { ShareStations = { company = 1, open = false } })"),
        "only the head of Rival closes the stations of it"
    );
    // The head sends Cat out: she plays for the first company again, and
    // gets back in only with the password.
    assert_eq!(
        eval("return why(JAMES, { Dismiss = { company = 1, player = CAT } })"),
        "ok"
    );
    assert_eq!(eval("return tostring(C.of(R, CAT).id)"), "0");
    assert_eq!(
        eval("return why(JAMES, { Dismiss = { company = 1, player = JAMES } })"),
        "the head leaves by joining another company"
    );
    // James leaves: Bob, who has played for Rival longest, is its head now.
    assert_eq!(eval("return why(JAMES, { Join = 0 })"), "ok");
    assert_eq!(eval("return C.head(R, 1)"), eval("return BOB"));
    assert_eq!(
        eval("return why(BOB, { ShareStations = { company = 1, open = false } })"),
        "ok"
    );
    assert_eq!(eval("return tostring(C.open(C.find(R, 1)))"), "false");
    // The founder returns with the password, and heads it again.
    assert_eq!(eval("return why(JAMES, { Join = 1 }, seal(1, 'e'))"), "ok");
    assert_eq!(eval("return C.head(R, 1)"), eval("return JAMES"));
    assert_eq!(eval("return why(JAMES, { Unlock = 1 })"), "ok");
    assert_eq!(eval("return why(CAT, { Join = 1 })"), "ok");
    // The room's first company is everyone's: no head, no password, and its
    // stations stay open.
    assert_eq!(eval("return tostring(C.head(R, 0))"), "nil");
    assert_eq!(
        eval("return why(JAMES, { Lock = 0 }, seal(0, 'e'))"),
        "the room's first company is everyone's: nobody gives a password to it"
    );
    assert_eq!(
        eval("return why(JAMES, { ShareStations = { company = 0, open = false } })"),
        "the room's first company is everyone's: nobody closes the stations of it"
    );
    // Using a station: no one's, one's own and an open company's are fine;
    // a closed company's is refused, naming it. Using is not changing.
    assert_eq!(
        eval(
            "return tostring(C.mayUse(R, 25, 802, api)) .. tostring(C.mayUse(R, 901, 800, api)) \
             .. tostring(C.mayUse(R, 901, 801, api))"
        ),
        "truetruetrue"
    );
    assert_eq!(
        eval("local ok, why = C.mayUse(R, 25, 800, api) return tostring(ok) .. ' ' .. why"),
        "false the station belongs to Rival, which keeps its stations to itself"
    );
    assert_eq!(
        eval(
            "why(JAMES, { ShareStations = { company = 1, open = true } }) return tostring(C.mayUse(R, 25, 800, api))"
        ),
        "true"
    );
    // A colour is fractions from 0 to 1, and no two companies wear one.
    assert_eq!(
        eval("return why(JAMES, { Recolor = { company = 1, color = { r = 2, g = 0, b = 0 } } })"),
        "a colour is { r, g, b }, each from 0 to 1"
    );
    assert_eq!(
        eval(
            "local first = C.find(R, 0).color \
             return why(JAMES, { Recolor = { company = 1, color = { r = first[1], g = first[2], b = first[3] } } })"
        ),
        "Company wears that colour already"
    );
}

/// Through the game script: a line of one company stops at another's
/// station while that one keeps its stations open, and is refused in every
/// game once they are closed; a join travels with its seal.
#[test]
fn a_line_stops_at_another_companys_station_while_it_is_open() {
    let (lua, _script) = engine();
    lua.load(FAKE_FLEET).exec().unwrap();
    lua.load(
        r#"
        api.type.Line = { new = function() return { vehicleInfo = {} } end,
            Stop = { new = function() return {} end }, StopConfig = { new = function() return {} end } }
        api.type.StationTerminal = { new = function(s, t) return { station = s, terminal = t } end }
        api.cmd.makeLineCreateCmd = function(name, color, player, line)
            return { createLine = { name = name, color = color, player = player, line = line } } end
        JAMES, BOB, CAT = string.rep("a", 64), string.rep("b", 64), string.rep("c", 64)
        -- Station group 90 is Rival's (901); 91 is no company's.
        OWNERS = {}
        local get = api.engine.getComponent
        api.type.ComponentType.PLAYER_OWNED = 55
        api.engine.getComponent = function(e, kind)
            if kind == 55 then return OWNERS[e] and { player = OWNERS[e] } or nil end
            return get(e, kind)
        end
        HOOK.room = true
        HOOK.batch = { { CompanyOp = { Create = { name = 'Rival' } } } } HOOK.origins = { JAMES }
        UPDATE({}, STATE, 0.2)
        RIVAL = STATE.value.companies.list[2]
        HOOK.batch = { { CompanyOp = { Lock = RIVAL.id } } } HOOK.origins = { JAMES }
        HOOK.seals = { { scope = RIVAL.id, tag = string.rep("e", 64) } }
        UPDATE({}, STATE, 0.2)
        HOOK.batch = { { CompanyOp = { Join = RIVAL.id } }, { CompanyOp = { Join = RIVAL.id } } }
        HOOK.origins = { BOB, BOB }
        HOOK.seals = { { scope = RIVAL.id, tag = string.rep("f", 64) }, { scope = RIVAL.id, tag = string.rep("e", 64) } }
        UPDATE({}, STATE, 0.2)
        HOOK.seals = nil
        "#,
    )
    .exec()
    .unwrap();
    let applied: String = lua
        .load(
            "local out = {} for _, a in ipairs(HOOK.applied) do \
                 out[#out + 1] = tostring(a.ok) .. (a.why and (':' .. a.why) or '') end \
             return table.concat(out, ',')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        applied, "true,true,false:the password for Rival is not right,true",
        "founded, locked, a wrong password refused, the right one let in"
    );
    let members: String = lua
        .load("local r = STATE.value.companies return #r.members .. ' ' .. r.members[2].company")
        .eval()
        .unwrap();
    assert_eq!(members, "2 1");
    // hook.log says what became of the companies, and never a seal.
    let logged: String = lua
        .load("return table.concat(HOOK.logged, '|')")
        .eval()
        .unwrap();
    assert!(
        logged.contains(
            "company: Lock by aaaaaaaa (with a password's seal): \
             Company #0 (0 chose it); Rival #1 (1 chose it, head aaaaaaaa, password)"
        ),
        "{logged}"
    );
    assert!(
        logged.contains("was not applied: the password for Rival is not right"),
        "{logged}"
    );
    assert!(!logged.contains("eeeeeeee"), "{logged}");
    // Cat, of the room's first company, runs a line from Rival's station 90
    // to 91: allowed while Rival's stations are open, refused once its head
    // closes them, in every game alike.
    lua.load(
        r#"
        OWNERS[90] = RIVAL.entity
        local registry = ug_require('tpf3mp_1::/scripts/tpf3mp/registry.lua')
        local reg = STATE.value.registry
        local ctx = { group = function(e) return registry.id(reg, 'groups', e) end,
                      line = function(e) return registry.id(reg, 'lines', e) end }
        local function stop(group) return { stationGroup = group, station = 0, terminal = 1,
            alternativeTerminals = {}, loadMode = 0, minWaitingTime = 0, maxWaitingTime = 180,
            maxAdditionalWaitingTime = 0, waypoints = {},
            stopConfig = { load = {}, maxLoad = {}, forceUnload = false,
                destroyForConfigChange = false, destroyForRefresh = false } } end
        LINE_ACTION = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua').lineCreate(ctx, 'Shared',
            { x = 0.8, y = 0.2, z = 0 }, 25, { stops = { stop(90), stop(91) }, customFilters = false,
            reservationPriority = 0, vehicleInfo = { transportModes = { [3] = true } } })
        HOOK.applied = {}
        HOOK.batch = { LINE_ACTION } HOOK.origins = { CAT }
        UPDATE({}, STATE, 0.2)
        HOOK.batch = { { CompanyOp = { ShareStations = { company = RIVAL.id, open = false } } }, LINE_ACTION }
        HOOK.origins = { JAMES, CAT }
        UPDATE({}, STATE, 0.2)
        "#,
    )
    .exec()
    .unwrap();
    let lines: String = lua
        .load(
            "local out = {} for _, a in ipairs(HOOK.applied) do \
                 out[#out + 1] = tostring(a.ok) .. (a.why and (':' .. a.why) or '') end \
             return table.concat(out, ',')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        lines,
        "true,true,false:stop 1: the station belongs to Rival, which keeps its stations to itself"
    );
}

/// Through the game script: a player founds a company, and what they do is
/// booked to it; the roster is kept in the script's state.
#[test]
fn what_a_player_does_is_booked_to_their_company() {
    let (lua, _script) = engine();
    lua.load(
        r#"
        -- A month is 1000 ms of game time here.
        GAME_T = 0
        api.type.ComponentType.GAME_TIME = 99
        api.engine.util.getWorld = function() return 1 end
        api.engine.getComponent = function(e, kind)
            if kind == 99 then return { gameTime = GAME_T } end
        end
        api.util = { getDefaultMonthDuration = function() return 1000 end }
        api.type.JournalEntry = { new = function() return { category = {} } end,
                                  Type = { LOAN = 'LOAN', INTEREST = 'INTEREST' } }
        api.cmd.makeJournalBookAssetCmd = function(e, entry) return { journal = entry, entity = e } end
        A, B = string.rep("a", 64), string.rep("b", 64)
        HOOK.batch = { { CompanyOp = { Create = { name = 'Rival' } } } } HOOK.origins = { A }
        UPDATE({}, STATE, 0.2)
        -- A's loan, for Rival: 1200 over 12 months at 12 % a year; and B's,
        -- who plays for the first company, through the game's loan script.
        OFFER = { type = 'Small', amount = 1200, duration = 12000, percentage = 0.12 }
        HOOK.batch = { { Loan = { Take = { next = OFFER, offer = OFFER } } },
                       { Loan = { Take = { next = OFFER, offer = OFFER } } } }
        HOOK.origins = { A, B }
        UPDATE({}, STATE, 0.2)
        -- A month later, Rival pays its first instalment, with no action,
        -- in the room's game.
        HOOK.room = true
        GAME_T = 1000
        UPDATE({}, STATE, 0.2)
        "#,
    )
    .exec()
    .unwrap();
    let roster: String = lua
        .load(
            "local r = STATE.value.companies local c = r.list[2] \
             return #r.list .. ' ' .. c.name .. ' ' .. c.entity .. ' ' .. r.members[1].company",
        )
        .eval()
        .unwrap();
    assert_eq!(
        roster, "2 Rival 901 1",
        "the roster is saved with the world"
    );
    let sent: String = lua
        .load(
            "local out = {} for _, c in ipairs(SENT) do \
                 out[#out + 1] = c.addPlayer or (c.event and c.event.name) \
                     or (c.journal and (c.journal.category.type .. c.journal.amount .. '@' .. c.entity)) or '?' end \
             return table.concat(out, ',')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        sent, "Rival,LOAN1200@901,Obtain,INTEREST-12@901,LOAN-95@901",
        "the company; Rival's loan booked to it, B's through the loan script; \
         a month later Rival's instalment of 107: 12 interest, 95 paid down"
    );
    let loan: String = lua
        .load("local l = STATE.value.companies.loans[1] return l.remaining .. ' ' .. l.paid .. '/' .. l.months")
        .eval()
        .unwrap();
    assert_eq!(loan, "1105 1/12");
}

/// The GUI's "my company" is the player's: api.engine.util.getPlayer answers
/// the company they play for, in the GUI state only, and the game's own
/// answer for the room's first company.
#[test]
fn the_guis_company_is_the_one_the_player_plays_for() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_CMD).exec().unwrap();
    lua.load(
        r#"
        ME = string.rep("b", 64)
        ROSTER = { next = 2, list = { { id = 0, entity = 25, name = "First", color = { 1, 0, 0 } },
                                      { id = 1, entity = 901, name = "Rival", color = { 0, 0, 1 } } },
                   members = {} }
        -- The game's binding is a callable table (build 40408).
        api.engine = api.engine or {}
        api.engine.util = { getPlayer = setmetatable({}, { __call = function() return 25 end }) }
        api.engine.system = api.engine.system or {}
        api.engine.system.gameScriptSystem = { getEntityForGameScript = function(name)
            if name == "tpf3mp_1::/tpf3mp_sim/tpf3mp_sim.gs" then return 77 end return -1 end }
        api.type = api.type or {}
        api.type.ComponentType = api.type.ComponentType or {}
        api.type.ComponentType.GAME_SCRIPT = 7
        api.engine.getComponent = function(e, kind)
            if e == 77 and kind == 7 then return { state = { companies = ROSTER } } end
        end
        HOOK.status = { room = "r", players = { { name = "b", id = ME, me = true, connected = true } }, me_id = ME }
        "#,
    )
    .exec()
    .unwrap();
    run_frames(&lua, 20);
    let first: i64 = lua
        .load("return api.engine.util.getPlayer()")
        .eval()
        .unwrap();
    assert_eq!(first, 25, "playing for the first company: the game's own");
    lua.load("ROSTER.members = { { player = ME, company = 1 } }")
        .exec()
        .unwrap();
    run_frames(&lua, 20);
    let mine: i64 = lua
        .load("return api.engine.util.getPlayer()")
        .eval()
        .unwrap();
    assert_eq!(mine, 901, "playing for Rival: Rival");
    let logged: String = lua
        .load("return table.concat(HOOK.logged, '|')")
        .eval()
        .unwrap();
    assert!(
        logged.contains("the GUI's company follows the player's"),
        "{logged}"
    );
}

/// The Multiplayer window's companies (D22, proposed): the head of the
/// player's company sets its password, opens or closes its stations and
/// sends players out; another player joins a company with a password by
/// typing it, and the password goes to the hook beside the action, never
/// into the window's notes or the log. The line manager offers another
/// company's station while that company keeps its stations open.
#[test]
fn the_window_lets_a_head_lock_the_company_and_others_join_with_its_password() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_CMD).exec().unwrap();
    lua.load(
        r#"
        JAMES, BOB = string.rep("a", 64), string.rep("b", 64)
        ROSTER = { next = 2,
            list = { { id = 0, entity = 25, name = "First", color = { 0.8, 0.16, 0.12 } },
                     { id = 1, entity = 901, name = "Rival", color = { 0.13, 0.42, 0.85 }, founder = JAMES } },
            members = { { player = JAMES, company = 1 }, { player = BOB, company = 1 } } }
        api.engine = api.engine or {}
        api.engine.util = { getPlayer = function() return 25 end }
        api.engine.system = api.engine.system or {}
        api.engine.system.gameScriptSystem = { getEntityForGameScript = function(name)
            if name == "tpf3mp_1::/tpf3mp_sim/tpf3mp_sim.gs" then return 77 end return -1 end }
        api.type = api.type or {}
        api.type.ComponentType = { GAME_SCRIPT = 7, STATION_GROUP = 9, CONSTRUCTION = 2, PLAYER_OWNED = 55 }
        api.type.Vec3f = { new = function(x, y, z) return { x = x, y = y, z = z } end }
        -- Station group 90 is Rival's.
        api.engine.getComponent = function(e, kind)
            if e == 77 and kind == 7 then return { state = { companies = ROSTER } } end
            if e == 90 and kind == 9 then return {} end
            if e == 90 and kind == 55 then return { player = 901 } end
        end
        HOOK.room = true
        function as(me)
            HOOK.status = { room = "r", me_id = me, players = {
                { name = "james", id = JAMES, me = me == JAMES, connected = true, owner = true },
                { name = "bob", id = BOB, me = me == BOB, connected = true } } }
        end
        as(JAMES)
        BAR = mount(loadPlugin())
        for _ = 1, 20 do BAR.step() end
        BAR.render()
        views(BAR.layout)[1].params.onClick()
        function window() WINDOWS.Tpf3mpWindow.step() return WINDOWS.Tpf3mpWindow.render() end
        function find(view, pick)
            for _, v in ipairs(views(window())) do
                if v.view == view and pick(v.params) then return v.params end
            end
        end
        function button(label)
            return find("Button", function(p) return p.content and p.content.params.text == label end)
        end
        function texts()
            local out = {}
            for _, v in ipairs(views(window())) do
                if v.view == 'TextView' then out[#out + 1] = v.params.text end
            end
            return table.concat(out, "\n")
        end
        function last() return HOOK.commands[#HOOK.commands], HOOK.passwords[#HOOK.commands] end
        "#,
    )
    .exec()
    .unwrap_or_else(|error| panic!("{error}\n{}", log(&lua)));
    let eval = |code: &str| -> String {
        lua.load(code)
            .eval::<String>()
            .unwrap_or_else(|error| panic!("{code}: {error}\n{}", log(&lua)))
    };
    assert!(
        eval("return texts()")
            .contains("Rival\nYour company · head: james\nPlayers  james (you), bob"),
        "{}",
        eval("return texts()")
    );
    // The head types a password: it goes beside the action, and the field
    // hides it.
    assert_eq!(
        eval(
            "local f = find('TextInputField', function(p) return p.passwordMode end) \
             f.onValueChange('s3cret') \
             local action, password = last() \
             return f.placeholderText .. '|' .. action.CompanyOp.Lock .. '|' .. password"
        ),
        "A password to join|1|s3cret"
    );
    assert!(!eval("return texts()").contains("s3cret"));
    // The head closes the stations and sends Bob out.
    assert_eq!(
        eval(
            "button('Close').onClick() \
             local close = last().CompanyOp.ShareStations \
             button('Send out').onClick() \
             local out = last().CompanyOp.Dismiss \
             return tostring(close.open) .. '|' .. out.player"
        ),
        format!("false|{}", "b".repeat(64))
    );
    // The line manager offers Rival's station while its stations are open.
    assert_eq!(
        eval(
            "local util = ug_require('/scripts/entity_util.tl') \
             local open = util.isOwnedByPlayerOrNotOwned(90) \
             ROSTER.list[2].closed = true \
             for _ = 1, 20 do BAR.step() end \
             return tostring(open) .. '|' .. tostring(util.isOwnedByPlayerOrNotOwned(90))"
        ),
        "true|false"
    );
    // Bob, of the first company now, joins Rival, which has a password.
    assert_eq!(
        eval(
            "ROSTER.list[2].lock = { scope = 1, tag = string.rep('e', 64) } \
             ROSTER.members = { { player = JAMES, company = 1 } } \
             as(BOB) for _ = 1, 20 do BAR.step() end \
             local n = #HOOK.commands \
             button('Join').onClick() \
             local refused = #HOOK.commands == n \
             local f = find('TextInputField', function(p) return p.passwordMode end) \
             f.onTyping('s3cret') \
             button('Join').onClick() \
             local action, password = last() \
             return tostring(refused) .. '|' .. action.CompanyOp.Join .. '|' .. password"
        ),
        "true|1|s3cret"
    );
    let shown = eval("return texts()");
    assert!(
        shown.contains("Rival\nhead: james · password · stations\nclosed\nPlayers  james"),
        "{shown}"
    );
    assert!(shown.contains("Joining Rival..."), "{shown}");
    assert!(!shown.contains("s3cret"), "{shown}");
    let logged = eval("return table.concat(HOOK.logged, '|')");
    assert!(!logged.contains("s3cret"), "{logged}");
    assert!(
        logged.contains("the line manager offers other companies' open stations"),
        "{logged}"
    );
}

/// The game's company window renames the player's company by its player
/// entity: that goes to the room as the company's rename.
#[test]
fn the_company_windows_rename_goes_to_the_room_as_the_companys() {
    let lua = gui();
    lua.load("C = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua')")
        .exec()
        .unwrap();
    let (company, line): (String, String) = lua
        .load(
            "local ctx = { company = function(e) if e == 901 then return 1 end end, \
                           line = function(e) if e == 600 then return 4 end end } \
             local a = C.setName(ctx, 901, 'Blue Line') \
             local b = C.setName(ctx, 600, 'North') \
             return a.CompanyOp.Rename.company .. ' ' .. a.CompanyOp.Rename.name, \
                    b.EditLine.line .. ' ' .. b.EditLine.change.Rename",
        )
        .eval()
        .unwrap();
    assert_eq!(company, "1 Blue Line");
    assert_eq!(line, "4 North");
}

/// With more than one company, vehicles wear their company's colour: a new
/// colour repaints the company's fleet, and only its own.
#[test]
fn a_companys_colour_repaints_its_vehicles() {
    let lua = gui();
    lua.load(
        r#"
        COMP = { [2] = { [500] = { player = 901 }, [501] = { player = 25 }, [502] = { player = 901 } },
                 [4] = { [500] = {}, [501] = {}, [502] = {} } }
        api = {
            engine = {
                util = { getPlayer = function() return 25 end },
                getComponent = function(e, kind) return COMP[kind] and COMP[kind][e] end,
                forEachEntityWithComponent = function(fn, kind)
                    local keys = {}
                    for e in pairs(COMP[kind] or {}) do keys[#keys + 1] = e end
                    table.sort(keys)
                    for _, e in ipairs(keys) do fn(e) end
                end,
            },
            type = { ComponentType = { NAME = 1, PLAYER_OWNED = 2, TRANSPORT_VEHICLE = 4 },
                     Vec3f = { new = function(x, y, z) return { x, y, z } end } },
            cmd = {
                makeGameAddPlayerCmd = function(name, color) return { add = name } end,
                makeEntitySetColorCmd = function(e, color) return { paint = e, color = color } end,
            },
        }
        SENT, NEXT = {}, 900
        function send(cmd)
            SENT[#SENT + 1] = cmd
            if cmd.add then NEXT = NEXT + 1 return { resultEntity = NEXT } end
        end
        C = ug_require("tpf3mp_1::/scripts/tpf3mp/companies.lua")
        A = string.rep("a", 64)
        R = C.ensure(nil, api)
        "#,
    )
    .exec()
    .unwrap();
    let painting: bool = lua.load("return C.painting(R)").eval().unwrap();
    assert!(!painting, "one company: the game's own colours");
    let painted: String = lua
        .load(
            "C.run(R, A, { Create = { name = 'Rival' } }, send, api) \
             SENT = {} \
             C.run(R, A, { Recolor = { company = 1, color = { r = 0, g = 0.5, b = 1 } } }, send, api) \
             local out = {} for _, c in ipairs(SENT) do out[#out + 1] = c.paint .. ':' .. c.color[2] end \
             return tostring(C.painting(R)) .. ' ' .. table.concat(out, ',')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        painted, "true 500:0.5,502:0.5",
        "Rival's two vehicles, not the first company's"
    );
}

/// A vehicle's marker on the map wears its company's colour: the mod
/// replaces the game's marker recipe with one that, while the room has more
/// than one company, puts the marker of a vehicle painted in a company
/// colour in that colour's class (the room paints a company's vehicles so),
/// as TPF2's vehicle icons followed the vehicle's paint, and leaves every
/// other marker as the game made it.
#[test]
fn a_vehicles_marker_wears_its_companys_colour() {
    let lua = gui();
    lua.load(
        r#"
        -- Vehicles by their first part's colour, as build 40408 gives it (a
        -- float's digits; -1 for the model's own colours).
        local function painted(x, y, z)
            return { transportVehicleConfig = { vehicles = { { part = { color = { x = x, y = y, z = z } } } } } }
        end
        COMP = { [4] = {
            [500] = painted(0.12999999523163, 0.41999998688698, 0.85000002384186), -- blue
            [501] = painted(0.80000001192093, 0.15999999642372, 0.11999999731779), -- red
            [502] = painted(0.9, 0.9, 0.9),                                        -- a player's own
            [503] = painted(-1, -1, -1),                                           -- unpainted
        } }
        -- The roster is in the mod's game script's state (entity 77).
        ROSTER = nil
        api = {
            engine = {
                getComponent = function(e, kind)
                    if kind == 7 then return e == 77 and { state = { companies = ROSTER } } or nil end
                    return COMP[kind] and COMP[kind][e]
                end,
                system = { gameScriptSystem = { getEntityForGameScript = function(name)
                    return name == "tpf3mp_1::/tpf3mp_sim/tpf3mp_sim.gs" and 77 or -1
                end } },
            },
            type = { ComponentType = { TRANSPORT_VEHICLE = 4, GAME_SCRIPT = 7 } },
        }
        CLOCK = 0
        os.clock = function() return CLOCK end
        local script = "gui/tpf3mp/company_markers.script.lua"
        assert(loadstring(mod_source(script), "@" .. script))()
        local exported = data()
        local toolbox = ug_require("::/gui/main/hud_icon_toolbox.tl")
        exported.replace({ ReplaceRecipe = function(original, replacement)
            assert(original == toolbox.HudIconMasterGame, "replaces the game's marker")
            MARKER = replacement
        end })
        C = ug_require("tpf3mp_1::/scripts/tpf3mp/companies.lua")
        -- The game script's companies, a while later (the game gives a new
        -- table on each read).
        function companies(list) ROSTER = { list = list, members = {} } CLOCK = CLOCK + 5 end
        -- The HUD takes only a layout from a marker's recipe (build 40408:
        -- "Recipe child must be a layout"), so the game's marker is always
        -- inside one.
        function marker(entity)
            local node = mount(MARKER, { entity = entity }).layout
            assert(node.layout == "BoxLayout", "a marker's recipe gives a layout")
            local inner = node.params.children[1]
            assert(#node.params.children == 1 and inner.view == "Marker", "around the game's marker")
            local class = node.params.meta and node.params.meta.class
            return (class and (class .. " around ") or "") .. "game's " .. inner.params.entity
        end
        "#,
    )
    .exec()
    .unwrap();
    let marker = |entity: i64| -> String {
        lua.load(format!("return marker({entity})"))
            .eval()
            .unwrap_or_else(|error| panic!("{entity}: {error}"))
    };
    // No roster yet, and one company: the game's markers.
    assert_eq!(marker(500), "game's 500");
    lua.load("companies({ { id = 0, entity = 25, color = C.PALETTE[1] } })")
        .exec()
        .unwrap();
    assert_eq!(
        marker(500),
        "game's 500",
        "one company: as in single player"
    );
    // Two companies: a vehicle in a company colour wears it.
    lua.load(
        "companies({ { id = 0, entity = 25, color = C.PALETTE[1] },                      { id = 1, entity = 901, color = C.PALETTE[2] } })",
    )
    .exec()
    .unwrap();
    assert_eq!(marker(500), "tpf3mp-company-2 around game's 500");
    assert_eq!(marker(501), "tpf3mp-company-1 around game's 501");
    // Any other paint, no paint, and what is no vehicle: the game's.
    assert_eq!(marker(502), "game's 502");
    assert_eq!(marker(503), "game's 503");
    assert_eq!(marker(600), "game's 600");
    // The roster is read again only every two seconds: a company dissolved
    // shows once it is read, back to the game's markers.
    lua.load(
        "ROSTER = { members = {}, list = { ROSTER.list[1],                     { id = 1, entity = 901, color = C.PALETTE[2], gone = true } } }",
    )
    .exec()
    .unwrap();
    assert_eq!(marker(500), "tpf3mp-company-2 around game's 500");
    lua.load("CLOCK = CLOCK + 2").exec().unwrap();
    assert_eq!(marker(500), "game's 500");

    // The style sheet has a class for every colour of the palette.
    let classes: String = lua
        .load(
            r#"
            local rules = {}
            local ssu = { makeAdder = function(result)
                return function(selector, style) result[#result + 1] = selector end
            end }
            local real = require
            require = function(path)
                if path == "::/gui/main/stylesheetutil.lua" then return ssu end
                return ug_require(path)
            end
            local css = "gui/tpf3mp/tpf3mp.css.lua"
            assert(loadstring(mod_source(css), "@" .. css))()
            local result = data()
            require = real
            return table.concat(result, "|")
            "#,
        )
        .eval()
        .unwrap();
    for i in 1..=8 {
        assert!(
            classes.contains(&format!("!tpf3mp-company-{i} VehicleItem::Icon")),
            "{classes}"
        );
    }
}

/// A world for the companies' progression (tpf3mp/progression.lua), over the
/// stand-in engine state: towns 5, 7 and 9 of 400, 1000 and 50 people; lines
/// 301 of the save's player (25), 302 and 303 of the company founded next
/// (901), 304 of a player the room has not; the game's delivery and
/// passenger statistics per line, the towns' ratings, and the game's own
/// modules. A game month is 4000 ms of game time: a sample every 1000.
/// REVERSED lists everything the other way round, as another game's hash
/// tables might.
const FAKE_PROGRESSION: &str = r#"
GAME_T = 0
local CT = api.type.ComponentType
CT.GAME_TIME, CT.PLAYER_OWNED, CT.TOWN = 99, 98, 12
api.engine.util.getWorld = function() return 1 end
OWNERS = { [301] = 25, [302] = 901, [303] = 901, [304] = 555 }
api.engine.getComponent = function(e, kind)
    if kind == CT.GAME_TIME then return { gameTime = GAME_T } end
    if kind == CT.PLAYER_OWNED then
        local o = OWNERS[e]
        if o then return { player = o } end
        return nil
    end
    if kind == CT.TOWN and (e == 5 or e == 7 or e == 9) then return {} end
end
api.engine.getEntitiesWithComponent = function(kind)
    if kind == CT.TOWN then return { 5, 7, 9 } end
    return {}
end
api.util = { getDefaultMonthDuration = function() return 4000 end,
             getDefaultYearDuration = function() return 48000 end }
local function keyed(pairsList)
    local t = {}
    local from, to, by = 1, #pairsList, 1
    if REVERSED then from, to, by = #pairsList, 1, -1 end
    for i = from, to, by do t[pairsList[i][1]] = pairsList[i][2] end
    return t
end
local function listed(list)
    if not REVERSED then return list end
    local out = {}
    for i = #list, 1, -1 do out[#out + 1] = list[i] end
    return out
end
api.engine.system.townBuildingSystem = { getTown2personCapacitiesMap = function()
    return keyed({ { 7, { 1000, 0 } }, { 5, { 400, 0 } }, { 9, { 50, 0 } } })
end }
DELIVERIES = function(town)
    if town == 7 then return keyed({ { 301, { [-1] = { 2, 60 } } }, { 302, { [-1] = { 0, 40 } } } }) end
    if town == 5 then return keyed({ { 302, { [-1] = { 0, 10 } } }, { 304, { [-1] = { 0, 99 } } } }) end
    return {}
end
HAPPY = function(town)
    if town == 7 then
        return listed({ { 301, { resident = { 1, 20 }, nonResident = { 0, 0 } } },
                        { 303, { resident = { 0, 10 }, nonResident = { 5, 10 } } } })
    end
    return {}
end
api.engine.util.town = {
    getTownDeliveriesStats = function(town, interval, perLine, perCargo)
        ASKED = { interval, perLine, perCargo }
        return DELIVERIES(town)
    end,
    getTownHappinessStats = function(town, lines) return { byLine = HAPPY(town) } end,
}
TOWN_STATES = { townStates = listed({
    { townEntity = { entity = 7 }, authorityScore = 0.7, cachedRatings = {
        urban_care = { value = 0.9 }, traffic_congestion = { value = 0.8 },
        noise = { value = 1 }, pollution = { value = 1 }, people_happiness = { value = 0.1 } } },
    { townEntity = { entity = 5 }, authorityScore = 0.5 },
    { townEntity = { entity = 9 }, authorityScore = 1 },
}) }
OWN = { [25] = { experience = 1500, level = 1, potentialLevel = 7 } }
GAME_MODULES = {
    ['/game_mechanics/company/company_progression_util.tl'] = {
        getLevelAndFraction = function(base, exp) return math.floor(exp / base), 0 end,
        getCompanyProgressionState = function(e) return OWN[e] end,
    },
    ['/game_mechanics/towns/town_util.tl'] = {
        getRatingSensitivity = function(state, key) return 1 end,
        externalGetTownsState = function() return TOWN_STATES end,
    },
    ['/game_mechanics/company/company_util.tl'] = { getBasePopulation = function() return 200 end },
}
A, B = string.rep('a', 64), string.rep('b', 64)
"#;

/// The progression lines of the hook's log.
fn progression_log(lua: &Lua) -> Vec<String> {
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    logged
        .into_iter()
        .filter(|l| l.starts_with("progression"))
        .collect()
}

/// A room of two companies after its first sample: A founded Rival (901),
/// B plays for the room's first company (25).
fn two_companies(reversed: bool) -> Lua {
    let (lua, _script) = engine();
    lua.load(FAKE_PROGRESSION).exec().unwrap();
    lua.globals().set("REVERSED", reversed).unwrap();
    lua.load(
        "HOOK.room = true \
         HOOK.batch = { { CompanyOp = { Create = { name = 'Rival' } } } } HOOK.origins = { A } \
         UPDATE({}, STATE, 0.2) \
         UPDATE({}, STATE, 0.2)",
    )
    .exec()
    .unwrap();
    lua
}

#[test]
fn with_two_companies_each_takes_its_share_of_every_town_by_its_deliveries_and_rating() {
    let lua = two_companies(false);
    let records: String = lua
        .load(
            "local out = {} for _, r in ipairs(STATE.value.progression.records) do \
                 out[#out + 1] = r.company .. ':' .. r.entity .. ':' .. r.experience .. ':' \
                     .. r.potential .. ':' .. r.level end \
             return table.concat(out, ' ')",
        )
        .eval()
        .unwrap();
    // Town 5: Rival carried all its cargo; rating the town's 50 (its own
    // parts are perfect): 400 x 1 x 50 / 100 = 200. Town 7: cargo 60 to 40,
    // passengers 20 to 20, so shares 0.55 and 0.45; the first company's
    // rating is the town's 80 (its passengers' happiness 93, its cargo on
    // time 95): 440; Rival's is its unhappy passengers' 64.29: 289.29.
    // Rival: 489. The first company keeps the game's own 1500, which it
    // earned before the room had two companies. Ranks: experience / 200.
    assert_eq!(records, "0:25:1500:7:1 1:901:489:2:1");
    let log = progression_log(&lua);
    assert_eq!(log.len(), 5, "{log:?}");
    assert_eq!(
        log[0],
        "progression at game time 0: 3 towns, weights cargo 1 passengers 1"
    );
    assert_eq!(
        log[1],
        "progression at game time 0: town-0 population 400: company-1 share 1.0000 \
         rating 50.0000 part 200.0000"
    );
    assert!(
        log[2].starts_with(
            "progression at game time 0: town-1 population 1000: company-0 share 0.5500 \
             rating 80.0000 part 440.0000, company-1 share 0.4500 rating 64.28"
        ),
        "{log:?}"
    );
    assert_eq!(
        log[3],
        "progression at game time 0: company-0 score 440.0000, experience 1500, rank 7 reached, 1 taken"
    );
    assert!(
        log[4].starts_with("progression at game time 0: company-1 score 489.28")
            && log[4].ends_with(", experience 489, rank 2 reached, 1 taken"),
        "{log:?}"
    );
    // The game's delivery statistics per line, over half a year.
    let asked: String = lua
        .load(
            "return table.concat({ tostring(ASKED[1]), tostring(ASKED[2]), tostring(ASKED[3]) }, ' ')",
        )
        .eval()
        .unwrap();
    assert_eq!(asked, "24000 true false");
    // The next sample waits for the next quarter of a month.
    lua.load("UPDATE({}, STATE, 0.2) GAME_T = 999 UPDATE({}, STATE, 0.2)")
        .exec()
        .unwrap();
    assert_eq!(progression_log(&lua).len(), 5);
    lua.load("GAME_T = 1000 UPDATE({}, STATE, 0.2)")
        .exec()
        .unwrap();
    assert_eq!(progression_log(&lua).len(), 10);
}

#[test]
fn every_game_scores_the_companies_alike_however_its_tables_are_ordered() {
    let run = |reversed: bool| {
        let lua = two_companies(reversed);
        lua.load("for _ = 1, 5 do GAME_T = GAME_T + 1000 UPDATE({}, STATE, 0.2) end")
            .exec()
            .unwrap();
        let records: String = lua
            .load(
                "local out = {} for _, r in ipairs(STATE.value.progression.records) do \
                     out[#out + 1] = r.company .. ':' .. r.experience .. ':' .. r.potential end \
                 for _, p in ipairs(STATE.value.progression.passengers) do \
                     out[#out + 1] = p.company .. '@' .. p.town .. '=' .. string.format('%.17g', p.value) end \
                 return table.concat(out, ' ')",
            )
            .eval()
            .unwrap();
        (records, progression_log(&lua))
    };
    let (a, log_a) = run(false);
    let (b, log_b) = run(true);
    assert_eq!(a, b);
    assert_eq!(log_a, log_b);
    assert_eq!(log_a.len(), 30, "six samples of five lines");
}

#[test]
fn with_two_companies_a_company_takes_the_ranks_it_reached() {
    let lua = two_companies(false);
    lua.load(
        "SENT = {} HOOK.applied = {} \
         HOOK.batch = { { ApplyRank = { level = 2 } }, { ApplyRank = { level = 3 } }, \
                        { ApplyRank = { level = 2 } }, { ApplyRank = { level = 5 } } } \
         HOOK.origins = { A, A, B, B } \
         UPDATE({}, STATE, 0.2)",
    )
    .exec()
    .unwrap();
    let applied: String = lua
        .load(
            "local out = {} for _, a in ipairs(HOOK.applied) do \
                 out[#out + 1] = a.i .. ':' .. tostring(a.ok) .. ':' .. tostring(a.why) end \
             return table.concat(out, ' ')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        applied, "1:true:nil 2:false:Rival has reached rank 2, not 3 3:true:nil 4:true:nil",
        "Rival takes rank 2 and not 3; the first company takes 2, then 5"
    );
    let levels: String = lua
        .load(
            "local out = {} for _, r in ipairs(STATE.value.progression.records) do \
                 out[#out + 1] = r.company .. ':' .. r.level end \
             return table.concat(out, ' ')",
        )
        .eval()
        .unwrap();
    assert_eq!(levels, "0:5 1:2");
    // The room's first company takes its ranks in the game's own state too,
    // for when the room is one company again; Rival's are the room's alone.
    let events: String = lua
        .load(
            "local out = {} for _, c in ipairs(SENT) do if c.event then \
                 out[#out + 1] = c.event.id .. '.' .. c.event.name .. '=' .. c.event.param.level end end \
             return table.concat(out, ' ')",
        )
        .eval()
        .unwrap();
    assert_eq!(events, "Companies.applyLevel=2 Companies.applyLevel=5");
}

#[test]
fn with_one_company_the_game_keeps_its_own_score_and_takes_its_own_ranks() {
    let (lua, _script) = engine();
    lua.load(FAKE_PROGRESSION).exec().unwrap();
    lua.load(
        "HOOK.room = true UPDATE({}, STATE, 0.2) \
         for _ = 1, 3 do GAME_T = GAME_T + 1000 UPDATE({}, STATE, 0.2) end \
         OWN[25] = { experience = 1500, level = 2, potentialLevel = 4 } \
         SENT = {} HOOK.applied = {} \
         HOOK.batch = { { ApplyRank = { level = 3 } }, { ApplyRank = { level = 5 } }, \
                        { ApplyRank = { level = 2 } } } \
         HOOK.origins = { A, A, B } \
         UPDATE({}, STATE, 0.2)",
    )
    .exec()
    .unwrap();
    assert!(
        progression_log(&lua).is_empty(),
        "no score of the room's own"
    );
    let none: bool = lua
        .load("return STATE.value.progression.records[1] == nil")
        .eval()
        .unwrap();
    assert!(none);
    let applied: String = lua
        .load(
            "local out = {} for _, a in ipairs(HOOK.applied) do \
                 out[#out + 1] = a.i .. ':' .. tostring(a.ok) .. ':' .. tostring(a.why) end \
             return table.concat(out, ' ')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        applied,
        "1:true:nil 2:false:the company has reached rank 4, not 5 \
         3:false:the company has rank 2 already"
    );
    let events: String = lua
        .load(
            "local out = {} for _, c in ipairs(SENT) do \
                 out[#out + 1] = c.event.src .. '|' .. c.event.id .. '|' .. c.event.name .. '|' .. c.event.param.level end \
             return table.concat(out, ' ')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        events, "|Companies|applyLevel|3",
        "the growth script's own event, as the company window sends it"
    );
}

#[test]
fn a_sample_the_game_cannot_measure_changes_no_rank() {
    let (lua, _script) = engine();
    lua.load(FAKE_PROGRESSION).exec().unwrap();
    lua.load(
        "GAME_MODULES['/game_mechanics/towns/town_util.tl'] = nil \
         HOOK.room = true \
         HOOK.batch = { { CompanyOp = { Create = { name = 'Rival' } } } } HOOK.origins = { A } \
         UPDATE({}, STATE, 0.2) UPDATE({}, STATE, 0.2)",
    )
    .exec()
    .unwrap();
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert!(
        logged.iter().any(|l| l.starts_with(
            "the companies' scores were not sampled: /game_mechanics/towns/town_util.tl did not load"
        )),
        "{logged:?}"
    );
    let none: bool = lua
        .load("return STATE.value.progression.records[1] == nil")
        .eval()
        .unwrap();
    assert!(none, "nothing guessed");
}

#[test]
fn the_rules_formulas_are_the_games() {
    let lua = gui();
    let out: String = lua
        .load(
            "local p = ug_require('tpf3mp_1::/scripts/tpf3mp/progression.lua') \
             local w = { cargo = 1, passengers = 1 } \
             return table.concat({ \
                 string.format('%.4f', p.happiness(1, 20, 1)), \
                 string.format('%.4f', p.happiness(0, 0, 1)), \
                 string.format('%.4f', p.happiness(9, 10, 0)), \
                 string.format('%.4f', p.onTime(5, 10, 1)), \
                 string.format('%.4f', p.onTime(1, 2, 1)), \
                 string.format('%.4f', p.share(60, 100, 1, 2, w)), \
                 string.format('%.4f', p.share(0, 0, 1, 4, w)), \
                 tostring(p.share(5, 0, 0, 0, w)), \
                 string.format('%.4f', p.share(60, 100, 1, 2, { cargo = 3, passengers = 1 })), \
                 string.format('%.1f', p.part(1000, 0.5, 80)) }, ' ')",
        )
        .eval()
        .unwrap();
    // The game's own: happiness and on-time cargo map 0.3..1 to 0..1 at
    // sensitivity 1, and fewer than 15 people or 10 items count as that many.
    assert_eq!(
        out,
        "0.9286 1.0000 1.0000 0.2857 0.8571 0.5500 0.2500 nil 0.5750 400.0"
    );
}

#[test]
fn the_company_window_reads_each_companys_own_rank_with_two_companies() {
    let lua = gui();
    let out: String = lua
        .load(
            "GAME_MODULES = { ['/game_mechanics/company/company_progression_util.tl'] = { \
                 getCompanyProgressionState = function(e) return { level = 9, potentialLevel = 9, experience = e } end } } \
             local p = ug_require('tpf3mp_1::/scripts/tpf3mp/progression.lua') \
             local roster = { next = 2, members = {}, list = { { id = 0, entity = 25, name = 'First' }, \
                 { id = 1, entity = 901, name = 'Rival' } } } \
             STATEV = { companies = roster, progression = { records = { \
                 { company = 0, entity = 25, experience = 1500, potential = 7, level = 3 }, \
                 { company = 1, entity = 901, experience = 489, potential = 2, level = 1 } } } } \
             assert(p.follow(function() return STATEV end)) \
             local util = GAME_MODULES['/game_mechanics/company/company_progression_util.tl'] \
             local function show(e) local s = util.getCompanyProgressionState(e) \
                 return s.level .. '/' .. s.potentialLevel .. '/' .. s.experience end \
             local two = show(25) .. ' ' .. show(901) .. ' ' .. show(555) \
             roster.list[2].gone = true \
             return two .. ' | ' .. show(25) .. ' ' .. show(901)",
        )
        .eval()
        .unwrap();
    assert_eq!(
        out, "3/7/1500 1/2/489 9/9/555 | 9/9/25 9/9/901",
        "each company's own with two; the game's own with one"
    );
}

/// A notification's popup plays its first sound and tells the game's
/// Notifications script (its `initialSound` event): in the room's game that
/// goes to the room, and every game's script marks the same notification.
#[test]
fn a_notifications_first_sound_is_marked_in_every_game() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_CMD).exec().unwrap();
    lua.load("M = mount(loadPlugin()) M.step() HOOK.room = true")
        .exec()
        .unwrap();
    lua.load(
        "api.cmd.sendCommand(api.cmd.makeScriptingSendEventCmd('', 'Notifications', 'initialSound', \
             { notificationId = 12 })) \
         api.cmd.sendCommand(api.cmd.makeScriptingSendEventCmd('', 'Notifications', 'initialSound', \
             { notificationId = 1.5 })) \
         M.step()",
    )
    .exec()
    .unwrap();
    let (sent, handed, seen): (usize, usize, i64) = lua
        .load("return #SENT, #HOOK.commands, HOOK.commands[1].NotificationSeen.notification")
        .eval()
        .unwrap();
    assert_eq!(sent, 0, "not run here: the room orders it for every game");
    assert_eq!(handed, 1, "the whole-numbered one, through the schema");
    assert_eq!(seen, 12);

    let (lua, _script) = engine();
    lua.load(
        "HOOK.room = true UPDATE({}, STATE, 0.2) \
         HOOK.batch = { { NotificationSeen = { notification = 12 } } } \
         UPDATE({}, STATE, 0.2)",
    )
    .exec()
    .unwrap();
    let event: String = lua
        .load("local e = SENT[1].event return table.concat({ e.src, e.id, e.name, e.param.notificationId }, '|')")
        .eval()
        .unwrap();
    assert_eq!(event, "|Notifications|initialSound|12");
}

/// In the GUI's other Lua state (where the HUD and the line manager's
/// depots are drawn) the GUI's company is the player's too: read from the
/// hook (who this player is) and the game script's roster, the room's
/// first company answered as the game answers it.
#[test]
fn the_huds_state_follows_the_players_company() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(
        r#"
        ME = string.rep("b", 64)
        ROSTER = { next = 2, list = { { id = 0, entity = 25, name = "First", color = { 1, 0, 0 } },
                                      { id = 1, entity = 901, name = "Rival", color = { 0, 0, 1 } } },
                   members = {} }
        api = api or {}
        api.engine = { util = { getPlayer = setmetatable({}, { __call = function() return 25 end }) },
                       system = { gameScriptSystem = { getEntityForGameScript = function(name)
                           return name == "tpf3mp_1::/tpf3mp_sim/tpf3mp_sim.gs" and 77 or -1 end } },
                       getComponent = function(e, kind)
                           if e == 77 and kind == 7 then return { state = { companies = ROSTER } } end
                       end }
        api.type = { ComponentType = { GAME_SCRIPT = 7 } }
        HOOK.status = { room = "r", players = { { name = "b", id = ME, me = true, connected = true } }, me_id = ME }
        CLOCK = 0
        os.clock = function() return CLOCK end
        local script = "gui/tpf3mp/gui_state.script.lua"
        assert(loadstring(mod_source(script), "@" .. script))()
        GAME_UTIL = { getActionParams = function() return {} end }
        local real = ug_require
        ug_require = function(path)
            if path == "::/gui/construction/construction_react_util.tl" then return GAME_UTIL end
            return real(path)
        end
        data().prepare({})
        ug_require = real
        "#,
    )
    .exec()
    .unwrap();
    let first: i64 = lua
        .load("return api.engine.util.getPlayer()")
        .eval()
        .unwrap();
    assert_eq!(first, 25, "playing for the first company: the game's own");
    lua.load(
        "ROSTER = { list = ROSTER.list, members = { { player = ME, company = 1 } } } CLOCK = 3",
    )
    .exec()
    .unwrap();
    let mine: i64 = lua
        .load("return api.engine.util.getPlayer()")
        .eval()
        .unwrap();
    assert_eq!(mine, 901, "playing for Rival: Rival");
    let logged: String = lua
        .load("return table.concat(HOOK.logged, '|')")
        .eval()
        .unwrap();
    assert!(
        logged.contains("the GUI's company follows the player's in the HUD's state")
            && logged.contains("the stop tool's stop is noted"),
        "{logged}"
    );
}

/// A road modifier's build, as the room orders it: the street 8-9 rebuilt in
/// place with the lanes, decoration, lock and owner the tool gave it. Every
/// game gives the lanes their modes as the game takes them, a Lua array
/// from 1 (build 40408: keyed from 0 they land one mode off, and a sidewalk
/// that carries vehicles crashed the simulation), its decoration by the id
/// its name has here, and the acting company as its owner.
#[test]
fn a_road_modifier_is_built_with_its_lanes_decorations_lock_and_owner() {
    let (lua, _script) = engine();
    lua.load(FAKE_NETWORK).exec().unwrap();
    lua.load(
        r#"
        -- Each read of a template's lanes gives new copies, as the game's.
        api.res.streetTemplateRep.get = function(id)
            return setmetatable({ streetStyle = '::/style/country.street_style' }, { __index = function(_, k)
                if k == 'laneConfigs' then
                    return { { speed = 1, width = 1, height = 0, offset = 0, forward = true, transportModes = {} },
                             { speed = 1, width = 1, height = 0, offset = 0, forward = true, transportModes = {} } }
                end
            end })
        end
        api.res.edgeDecorationRep = { find = function(name)
            if name == '::/infrastructure/edge_addons/barrier_b.edge' then return 3 end return -1 end }
        api.type.PlayerOwned = { new = function() return {} end }
        "#,
    )
    .exec()
    .unwrap();
    let action = "{ BuildRoad = { street = '::/street/country.street_template', bus_lane = false, tram = 'None', \
        polyline = { vertices = { \
            { pos = { x = 50, y = -40, z = 0 }, resolve = { Node = 'Street' } }, \
            { pos = { x = 50, y = 40, z = 0 }, resolve = { Node = 'Street' } } }, \
          links = { { from = 0, to = 1, tangent0 = { x = 0, y = 80, z = 0 }, tangent1 = { x = 0, y = 80, z = 0 }, \
            structure = 'Ground', \
            lanes = { { speed = 22.22, width = 2, height = 0, offset = -1, forward = false, modes = 3 }, \
                      { speed = 22.22, width = 5, height = 0, offset = -0.5, forward = false, modes = 124 } }, \
            decorations = { { name = '::/infrastructure/edge_addons/barrier_b.edge', flag = false } }, \
            locked = true, owned = true } }, \
          removals = { { network = 'Street', ends = { a = { x = 50, y = -40, z = 0 }, b = { x = 50, y = 40, z = 0 } } } }, \
          removed_nodes = {} } } }";
    lua.load(format!(
        "HOOK.batch = {{ {action} }} UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap();
    let built: String = lua
        .load(
            "local e = SENT[1].proposal.streetProposal.edgesToAdd[1]
             local function modes(l)
                 local on = {}
                 for i = 1, 16 do if l.transportModes[i] then on[#on + 1] = i - 1 end end
                 return table.concat(on, ',') .. (l.transportModes[0] == nil and '' or ' zero-keyed!')
             end
             return table.concat({ modes(e.comp.laneConfigs[1]), modes(e.comp.laneConfigs[2]),
                 e.comp.laneConfigs[2].width, e.comp.edgeDecorations[1][1],
                 tostring(e.comp.edgeDecorations[1][2]), tostring(e.comp.roadDevelopmentLocked),
                 e.playerOwned.player }, '|')",
        )
        .eval()
        .unwrap_or_else(|error| {
            panic!(
                "{error}\n{:?}",
                lua.load("return HOOK.logged").eval::<Vec<String>>()
            )
        });
    assert_eq!(built, "0,1|2,3,4,5,6|5|3|false|true|25");
}

#[test]
fn bridge_syncs_pointer_and_build_preview_cursors() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    let bridge = bridge(&lua);
    lua.globals().set("BRIDGE", bridge).unwrap();
    lua.load(
        r#"
        local link = assert(BRIDGE.attach(tpf3mp_native))
        local testCurves = { { 10.0, 20.0, 30.0, 40.0, 5.0, 5.0, 5.0, 5.0 } }
        link:cursor(123.5, 456.25, true, "streetTerminalBuilder", testCurves)
        assert(HOOK.cursor ~= nil)
        assert(HOOK.cursor.x == 123.5)
        assert(HOOK.cursor.y == 456.25)
        assert(HOOK.cursor.building == true)
        assert(HOOK.cursor.label == "streetTerminalBuilder")
        assert(type(HOOK.cursor.curves) == "table")
        assert(#HOOK.cursor.curves == 1)

        HOOK.cursors = {
            ["0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"] = {
                x = 100.0,
                y = 200.0,
                building = true,
                label = "trackBuilder",
                curves = testCurves,
            }
        }
        local cursors = link:cursors()
        local remote = cursors["0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"]
        assert(remote ~= nil)
        assert(remote.x == 100.0)
        assert(remote.y == 200.0)
        assert(remote.building == true)
        assert(remote.label == "trackBuilder")
        assert(type(remote.curves) == "table")
        assert(#remote.curves == 1)

        link:cursor(nil)
        assert(HOOK.cursor.x == nil)
        "#,
    )
    .exec()
    .unwrap();
}

#[test]
fn sim_gui_draws_and_cleans_up_holograms_for_remote_cursors() {
    let (lua, _script) = engine();
    lua.load(
        r#"
        HOOK.room = true
        local zones = {}
        local removed = {}
        api = api or {}
        api.gui = {
            mission = {
                setZone = function(key, poly, draw, color, prohibit, anim, zOffset)
                    zones[key] = { poly = poly, color = color }
                end,
                setZoneCircle = function(key, center, radius, draw, color, prohibit, anim, zOffset)
                    zones[key] = { center = center, radius = radius, color = color }
                end,
                removeZone = function(key)
                    zones[key] = nil
                    removed[#removed + 1] = key
                end,
            }
        }
        api.type = {
            Vec2f = { new = function(x, y) return { x = x, y = y } end },
            Vec4f = { new = function(r, g, b, a) return { r = r, g = g, b = b, a = a } end },
        }

        local remoteHex = string.rep("ab", 32)
        HOOK.cursors = {
            [remoteHex] = {
                x = 100.0,
                y = 200.0,
                building = true,
                label = "streetBuilder",
                curves = {
                    { 0.0, 0.0, 100.0, 200.0, 10.0, 20.0, 10.0, 20.0 }
                }
            }
        }

        -- Run guiUpdate
        SCRIPT.guiUpdate(nil, nil, nil)
        assert(zones["tpf3mp_holo_" .. remoteHex .. "_c1"] ~= nil, "road curve ribbon zone drawn")
        assert(zones["tpf3mp_holo_" .. remoteHex .. "_ptr"] ~= nil, "cursor pointer circle drawn")

        -- Now remote player stops building:
        HOOK.cursors[remoteHex].building = false
        SCRIPT.guiUpdate(nil, nil, nil)
        assert(zones["tpf3mp_holo_" .. remoteHex .. "_c1"] == nil, "curve zone cleaned up")
        assert(zones["tpf3mp_holo_" .. remoteHex .. "_ptr"] == nil, "circle zone cleaned up")
        assert(#removed >= 2, "removed zones called")
        "#,
    )
    .exec()
    .unwrap();
}
