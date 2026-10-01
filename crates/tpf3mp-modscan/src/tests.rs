//! The scan on small mods written for these tests, each shaped like a real
//! kind of Transport Fever 3 mod (docs/MODS.md names the real ones and what
//! the scan said of them). None copies a real mod's code.

use std::path::Path;

use super::*;

/// A mod folder with `files`, each `(path, text)`.
fn mod_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir
}

fn kinds(report: &Report) -> Vec<Kind> {
    let mut kinds: Vec<Kind> = report.reasons.iter().map(|r| r.kind).collect();
    kinds.dedup();
    kinds
}

fn sharing(report: &Report) -> Vec<(Kind, String, Option<usize>)> {
    report
        .sharing()
        .map(|r| (r.kind, r.file.clone(), r.line))
        .collect()
}

const MANIFEST: &str = r#"{
  "modId": "test_mod_1", "revision": 3, "severityAdd": "None", "severityRemove": "None",
  "preRunScript": {"fileName": ""}, "runScript": {"fileName": ""}, "postRunScript": {"fileName": ""}
}"#;

const PLUGIN_RES: &str = r#"function data()
  return {
    type = "react-plugin ::GameBarInfoDisplayExtension",
    filePath = "test_mod_1::/gui/overlay/overlay.script@Overlay",
    priority = 10,
  }
end
"#;

/// A minimap or overlay: reads the engine, draws, keeps its settings in the
/// save, and edits a line through a command.
const OVERLAY_SCRIPT: &str = r#"
local react = ug_require "::/gui/main/react.lua"
-- api.cmd.sendCommand(x) in a comment is not a call; nor is "game.interface".
local M = {}
function M.Overlay()
  local started = os.clock()
  local lines = api.engine.system.lineSystem.getLines()
  api.gui.game.setGuiSaveData("test_mod_1", { zoom = 2 })
  local cmd = api.cmd.makeLineUpdateCmd(lines[1], api.type.Line.new())
  api.cmd.sendCommand(cmd, function(res, ok) end)
  return react.Component{}
end
return M
"#;

#[test]
fn a_gui_overlay_is_personal_and_says_what_it_does() {
    let dir = mod_with(&[
        ("mod.json", MANIFEST),
        ("_content.json", "{}"),
        ("_metadata/modinfo.json", "{}"),
        ("_metadata/0.png", "png"),
        ("README.md", "# overlay"),
        ("content/gui/overlay/overlay.res.lua", PLUGIN_RES),
        ("content/gui/overlay/overlay.script.lua", OVERLAY_SCRIPT),
        ("content/gui/overlay/overlay.css.lua", "return {}"),
        ("content/gui/overlay/icon@2x.tga", "tga"),
        ("content/gui/overlay/types.d.tl", "global record X end"),
    ]);
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Personal, "{report}");
    assert_eq!(report.id, "test_mod_1");
    assert_eq!(report.revision, Some(3));
    assert_eq!(
        kinds(&report),
        [Kind::Commands, Kind::Gui, Kind::SaveData, Kind::System]
    );
    let commands = report
        .reasons
        .iter()
        .find(|r| r.kind == Kind::Commands)
        .unwrap();
    assert_eq!(commands.detail, "makes makeLineUpdateCmd");
    assert_eq!(commands.line, Some(9), "the first command's line");
    assert_eq!(
        report.to_string().lines().next().unwrap(),
        "test_mod_1 (revision 3): personal"
    );
}

#[test]
fn a_game_script_that_sends_only_what_the_room_carries_is_carried() {
    // Shaped like Auto Line Namer: a game script renaming lines on a timer,
    // and a rename scheme for the line manager's button.
    let dir = mod_with(&[
        ("mod.json", MANIFEST),
        (
            "content/namer/namer.gs.lua",
            "function data() return { updateScript = { fileName = \"namer.script@update\" } } end",
        ),
        (
            "content/namer/namer.script.lua",
            "local M = {}\nfunction M.update(p, state)\n  if os.time() > 0 then\n    api.cmd.sendCommand(api.cmd.makeEntitySetNameCmd(1, 'Bus 1'))\n  end\n  state:set({})\nend\nreturn M",
        ),
        (
            "content/namer/scheme.res.lua",
            "function data() return { type = \"rename_scheme\", name = _(\"Auto\") } end",
        ),
    ]);
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Carried, "{report}");
    assert_eq!(
        sharing(&report),
        [(Kind::GameScript, "content/namer/namer.gs.lua".into(), None)]
    );
    assert_eq!(report.commands, ["makeEntitySetNameCmd"]);
    // The rename scheme alone is the GUI's.
    assert!(
        report
            .reasons
            .iter()
            .any(|r| r.kind == Kind::Gui && r.detail == "GUI resource rename_scheme")
    );
}

#[test]
fn run_scripts_and_modifiers_are_content() {
    // Shaped like Automatic Signal Spacing: a run script adding parameters
    // to the game's signals, and a game script placing more of them.
    let dir = mod_with(&[
        (
            "mod.json",
            r#"{"modId": "signals", "runScript": {"fileName": "signals::/mod.script@runFn"}}"#,
        ),
        (
            "content/mod.script.lua",
            "function data() return { runFn = function()\n addModifier(\"loadConstruction\", function(f, d) return d end)\nend } end",
        ),
        (
            "content/signals/signals.gs.lua",
            "function data() return {} end",
        ),
        ("make.sh", "cyan build"),
        ("src/signals.tl", "addModifier('x', nil)"),
    ]);
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Shared);
    assert_eq!(
        sharing(&report),
        [
            (Kind::RunScript, "mod.json".into(), None),
            (Kind::Modifier, "content/mod.script.lua".into(), Some(2)),
            (
                Kind::GameScript,
                "content/signals/signals.gs.lua".into(),
                None
            ),
        ]
    );
    // The build script and the Teal sources beside content/ are not loaded.
    let not_loaded: Vec<&str> = report
        .reasons
        .iter()
        .filter(|r| r.kind == Kind::NotLoaded)
        .map(|r| r.file.as_str())
        .collect();
    assert_eq!(not_loaded, ["make.sh", "src/signals.tl"]);
}

#[test]
fn a_cosmetic_flag_decides_nothing() {
    // Shaped like Timetables: "cosmetic": true, and a game script holding
    // vehicles at their stops.
    let dir = mod_with(&[
        (
            "mod.json",
            r#"{"modId": "timetables", "cosmetic": true, "runScript": {"fileName": ""}}"#,
        ),
        ("content/tt/tt.gs.lua", "function data() return {} end"),
        (
            "content/tt/tt_gs.script.tl",
            "local function hold(v: integer)\n  api.cmd.sendCommand(api.cmd.makeVehicleSetManualDepartureCmd(v, true))\nend",
        ),
    ]);
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Carried);
    assert!(report.reasons.iter().any(|r| r.kind == Kind::CosmeticFlag));
    assert_eq!(sharing(&report).len(), 1);
}

#[test]
fn a_game_script_that_sends_what_the_room_does_not_carry_is_shared() {
    // Shaped like the pre-release Big City mod, moved into a game script.
    let dir = mod_with(&[
        ("mod.json", MANIFEST),
        ("content/c/c.gs.lua", "function data() return {} end"),
        (
            "content/c/c.script.lua",
            "api.cmd.sendCommand(api.cmd.makeTownCreateCmd({}))",
        ),
    ]);
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Shared);
    assert_eq!(report.commands, ["makeTownCreateCmd"]);
}

#[test]
fn a_run_script_that_sets_the_game_config_is_shared() {
    let run = "function data() return { runFn = function(p, s)\n\
               local keep = game.config.x == 1\n\
               local read = game.config.economy\n\
               other = 2\n\
               game.config.economy.inflation[1] = 0\n\
               end } end";
    let dir = mod_with(&[
        (
            "mod.json",
            r#"{"modId": "t", "runScript": {"fileName": "t::/mod.script@runFn"}}"#,
        ),
        ("content/mod.script.lua", run),
        ("content/t.gs.lua", "function data() return {} end"),
    ]);
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Shared);
    let config: Vec<Option<usize>> = report
        .reasons
        .iter()
        .filter(|r| r.kind == Kind::ConfigWrite)
        .map(|r| r.line)
        .collect();
    assert_eq!(config, [Some(5)], "a comparison and a read are not writes");
}

/// The scan's list of what the room carries is the guards' own.
#[test]
fn the_room_carried_list_is_the_guards() {
    let scripts =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mod/tpf3mp_1/content/scripts/tpf3mp");
    let guard = std::fs::read_to_string(scripts.join("guard.lua")).unwrap();
    let modguard = std::fs::read_to_string(scripts.join("modguard.lua")).unwrap();
    // Each table's keys: `name = ...` lines between its `X = {` and `}`.
    let keys = |text: &str, table: &str| -> Vec<String> {
        let start = text.find(&format!("{table} = {{")).unwrap();
        let body = &text[start..];
        let body = &body[..body.find("\n}").unwrap()];
        body.lines()
            .skip(1)
            .filter_map(|l| {
                let l = l.trim();
                let name = l.split(['=', ' ']).next()?;
                (name.starts_with("make") && name.ends_with("Cmd")).then(|| name.to_owned())
            })
            .collect()
    };
    let mut carried: Vec<String> = keys(&guard, "guard.CARRY");
    carried.extend(keys(&guard, "guard.PASS"));
    carried.extend(keys(&modguard, "modguard.CARRY"));
    carried.extend(keys(&modguard, "modguard.DROP"));
    // The GUI's guard carries a build only as a construction window's
    // edit; every other build, a mod's included, is refused.
    carried.retain(|c| c != "makeWorldBuildProposalCmd");
    carried.sort();
    carried.dedup();
    assert_eq!(carried, ROOM_CARRIED);
}

#[test]
fn world_content_is_shared() {
    let dir = mod_with(&[
        ("mod.json", MANIFEST),
        (
            "content/vehicle/bus/new_bus.mdl",
            "function data() return {} end",
        ),
        (
            "content/construction/depot.con",
            "function data() return {} end",
        ),
        ("content/names/towns.lua", "return { 'Springfield' }"),
        ("content/vehicle/bus/new_bus.tga", "tga"),
    ]);
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Shared);
    let files: Vec<String> = sharing(&report).into_iter().map(|(_, f, _)| f).collect();
    assert_eq!(
        files,
        [
            "content/construction/depot.con",
            "content/names/towns.lua",
            "content/vehicle/bus/new_bus.mdl"
        ]
    );
}

#[test]
fn what_the_scan_cannot_read_or_the_guard_cannot_see_is_shared() {
    let dir = mod_with(&[
        ("mod.json", MANIFEST),
        (
            "content/gui/x.script.lua",
            "local send = api.cmd.sendCommand\n\
             local f = _G[\"lo\" .. \"ad\"]\n\
             local g = load(\"return 1\")\n\
             api.res.constructionRep.setAsTable(1, {})\n\
             api.res.modelRep.add(\"m.mdl\", {}, true)\n\
             game.interface.buildConstruction()\n\
             debug.setupvalue(f, 1, nil)\n\
             setmetatable(_G, {})\n\
             api.cmd.sendCommand = function() end\n\
             local text = 'a' .. load_more\n\
             board.load(1)\n",
        ),
        ("content/gui/helper.dll", "MZ"),
        (
            "content/gui/y.res.lua",
            "function data() return { type = \"construction\" } end",
        ),
    ]);
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Shared);
    assert_eq!(
        sharing(&report)
            .into_iter()
            .map(|(k, _, l)| (k, l))
            .collect::<Vec<_>>(),
        [
            (Kind::UnknownResource, Some(1)),
            (Kind::ResourceWrite, Some(4)),
            (Kind::ResourceWrite, Some(5)),
            (Kind::GameInterface, Some(6)),
            (Kind::Dynamic, Some(2)),
            (Kind::Dynamic, Some(3)),
            (Kind::Dynamic, Some(7)),
            (Kind::Dynamic, Some(8)),
            (Kind::CommandBypass, Some(1)),
            (Kind::CommandBypass, Some(9)),
            (Kind::UnknownFile, None),
        ]
    );
}

/// What reaches the game through another name, or a field named by a
/// string, is what it reaches as written.
#[test]
fn an_alias_or_a_string_index_hides_nothing() {
    let dir = mod_with(&[
        ("mod.json", MANIFEST),
        (
            "content/gui/x.script.lua",
            "local g = game
             g.interface.buildConstruction()
             local rep = api.res.modelRep
             rep.add(\"m.mdl\", {}, true)
             local c = g.config
             c.millisPerDay = 1
             local send = api[\"cmd\"]
             game[\"interface\"].upgradeConstruction()
             api.res.streetTypeRep.remove(\"s.lua\")
             local name = c.name
             local rows = params.config[\"rows\"]
",
        ),
    ]);
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Shared);
    assert_eq!(
        sharing(&report)
            .into_iter()
            .map(|(k, _, l)| (k, l))
            .collect::<Vec<_>>(),
        [
            (Kind::ResourceWrite, Some(4)),
            (Kind::ResourceWrite, Some(9)),
            (Kind::GameInterface, Some(2)),
            (Kind::ConfigWrite, Some(6)),
            (Kind::Dynamic, Some(7)),
            (Kind::Dynamic, Some(8)),
        ]
    );
}

#[test]
fn no_manifest_or_a_broken_one_is_shared() {
    let dir = mod_with(&[("content/gui/x.script.lua", "return {}")]);
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Shared);
    assert_eq!(sharing(&report)[0].0, Kind::Manifest);

    let dir = mod_with(&[("mod.json", "{ not json")]);
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Shared);
    assert!(report.id.starts_with(".tmp") || !report.id.is_empty());
}

#[test]
fn a_mod_folder_that_does_not_exist_is_shared() {
    let report = scan(Path::new("this folder is not there"));
    assert_eq!(report.class, Class::Shared);
    let kinds: Vec<Kind> = report.sharing().map(|r| r.kind).collect();
    assert_eq!(kinds, [Kind::Manifest, Kind::Unreadable]);
}

#[test]
fn text_that_is_not_utf8_is_unreadable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("mod.json"), MANIFEST).unwrap();
    std::fs::create_dir_all(dir.path().join("content")).unwrap();
    std::fs::write(dir.path().join("content/x.script.lua"), [0xff, 0xfe, 0x00]).unwrap();
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Shared);
    assert_eq!(sharing(&report)[0].0, Kind::Unreadable);
}

#[test]
fn reasons_serialize_for_other_tools() {
    let dir = mod_with(&[("mod.json", MANIFEST), ("content/a.gs.lua", "")]);
    let json = serde_json::to_value(scan(dir.path())).unwrap();
    assert_eq!(json["class"], "carried");
    assert_eq!(json["reasons"][0]["kind"], "game_script");
    assert_eq!(json["reasons"][0]["file"], "content/a.gs.lua");
}
