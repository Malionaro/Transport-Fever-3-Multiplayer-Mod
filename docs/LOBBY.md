# The Multiplayer entry on the main menu

D17 moves the room into the game (the owner lifted its hold on
2026-09-30): connecting, rooms, the lobby and chat in a Multiplayer window
reached from the main menu. This page says how the entry gets onto
Transport Fever 3's main menu at all, which took three tries on release
day, what the mod and the hook each contribute, and how the window talks
to the launcher that started the game. Players' steps are in
[PLAYING.md](PLAYING.md), "The Multiplayer menu in the game".

## What the game allows, and what it does not

TF3 draws its main menu from Teal scripts, `gui/menu/main_menu.tl` and
`gui/menu/main_page.tl`, loaded through the game's Lua loader. Three ways
in were tried against build 40408:

| route | result |
|---|---|
| A mod's own copy of `gui/menu/main_page.tl`, hoping the mod filesystem overlays the game's files at the menu | Not applied: mods are merged into the filesystem at startup but the game's `::/` files win until a game is loaded (as TPF2 applied mods per save). The menu has no mod extension point either. |
| The game's `--script <uri>` switch, pointing at a copy of `main_menu.tl` | It is a plain startup script runner (`Lua_Core`, before any menu exists): the file ran, but `_react.builtin` was empty and `react.lua` failed. A dead end for a menu. |
| **The hook, at the game's Lua loader** | Works. Below. |

## How it works

The game's `base/init.lua` resolves every `ug_require` and then calls
`resolveutil.loadfile(resolved)`, a Lua function whose body is a C++ lambda
(`framework/lua/Loader.cpp`, `lua::MakeState::<lambda_8>`). The hook, loaded
into the suspended game before any of its code runs (D11), detours that body
(`crates/tpf3mp-hook/src/menu_entry.rs`):

1. The detour's thunk saves the argument registers, reads the Lua state out
   of the lambda's closure (`**(closure + 0x10)`, the layout the body's own
   `lua_pcallk` call shows), and tail-jumps into the original body with the
   stack untouched. The loader runs exactly as before.
2. The first time a Lua state is seen, the hook runs a short Lua chunk in it
   through `lua_load` and `lua_pcallk`. The chunk wraps
   `resolveutil.loadfile`: a request for the game's `gui/menu/main_page.tl`
   is answered with the mod's `tpf3mp_1::/gui/menu/main_page.tl`; every
   other request passes through. The original resolved path stays the
   module's cache key, so the rest of the menu sees the same `MainPage`
   value it always did.
3. The mod's `main_page.tl` is the game's file with marked `TPF3-MP:`
   additions (below), and a `Tpf3mpLobbyWindow` opened through the menu's
   own window container (as the Deluxe Edition window is).

**Before the game runs.** The game loads its main menu within seconds of
starting, so the entry must be armed first. The launcher starts the game
suspended, loads the hook, and keeps it suspended until the hook sets the
event `tpf3mp_ipc::hook_ready_event` names for the game's process: the
hook arms the entry first in its bootstrap, sets the event, and only then
installs the step gate and the rest. A hook that never sets it lets the
game run after 30 seconds. Without this the entry was sometimes missing
(2026-09-30): the slower installs came first, and the game had loaded its
own main page before the patch was in.

The game log shows each step: `[tpf3mp] main menu: resolveutil.loadfile is
wrapped`, `... ::/gui/menu/main_page.tl is served from
tpf3mp_1::/gui/menu/main_page.tl`, `... TPF3-MP main_page.tl is in effect`.
The hook's `hook.log` shows the profile match, `main-menu Multiplayer entry
armed`, and `menu patch installed in Lua state ...`. Then `main_page.tl SERVED: ...` when the menu's page came from the mod,
or `main_page.tl MISSED: ...` when the game's own loaded first.

A game Steam started has no hook and keeps the plain menu.

## What the menu shows

- **Two cards**, in a column right of the game's own grid of cards, each
  a quarter of the menu wide and half high: the size the game gives its
  `level2b` cards, so the grid's rows stay as they are and the menu grows
  by one column. **Multiplayer** has the grid's top-right corner (the Map
  Editor card gives it up), two of the game's own pictures, the TPF3-MP
  glyph, and a live line under its title: not connected, online on EU,
  the room with its players and ready count, or the room's world on its
  way. **Join a friend** opens the window with joining first, and shows
  the room's invite once in one. The labels are the game's card label
  (`menu_icon_react_util.makeCardLabelBottomComponent`) with the live
  line (`lobby.CardLine`, its own recipe, so only it redraws, once a
  second) in place of the fixed description.
- **A button in the top bar**, next to Settings: a glyph drawn as the
  game's top-bar icons are, 100 px greyscale, white on black, for the game
  to tint (`gui/tpf3mp/icons/menu_multiplayer_50@2x.tga` and its 50 px
  copy, from `tools/art/icons/menu_icon.py`).
- **The Multiplayer window** (`gui/menu/lobby.lua`), 940 by 660, in the
  game's own classes: the `primary` and `secondary` buttons, the
  `font-scale-*` sizes, and the default style sheet's colours and tapes
  (`success`, `warning`, `error`, `info`). Along its top, the connection
  and the steps to playing together; under them, what is under way until
  the launcher answers, or what went wrong, or what just happened; the
  room's world while it comes and loads, with a progress bar; how the
  game differs from the room's. Then one of three views:
  - not connected: the name, and **Connect to EU**;
  - connected: a first page with two big cards, **Join a room** and
    **Host a room**, each opening its own page with Back to the first.
    Join: **Public rooms**, the server's room list
    (D26 proposed; PROTOCOL.md, "Rooms"), as cards in the main menu's
    style (the game's `menu_icon_react_util.CardButton`, class
    `small-rectangle-card`), each with its climate's picture (the game's
    own, `app.res.climateRep`, else its New Game card's), name,
    players/limit, companies, year and a lock for a password; a click
    joins, asking for a password first. The window asks for it when shown
    and every 10 seconds; under it, joining with an invite. Host: its name, the save it starts
    from, players, private or public (a public room is listed with the
    save's climate and year, read as the Load Game page reads them:
    `app.findAllSavegames`, `app.getSavegameInfo`), rules and a password.
    **Your mods** opens from Join, Host and the room: the player's
    installed mods to turn on or off (`choose_mod`), and in a room the
    room's own and whether the player has each;
  - in a room: its name, invite and counts, the players with their marks
    (owner, you, ready, away, other mods) and, for the owner, a Remove
    button that asks first; the chat; **Leave room** (asks first),
    **Ready** or **Not ready**, and, for the owner, **Start the game**,
    which waits until everyone is ready. Once the room's game runs, the
    chat and Leave stay and Ready and Start go.

  `crates/tpf3mp-hook/src/lobby/window_tests.rs` draws the window in every
  view against a stand-in for the menu (`tests/lua/fake_menu.lua`), clicks
  its buttons, and parses every action it sends as the hook does.

The pause menu has no Multiplayer entry: in the room's game, the game
bar's line and the Multiplayer window it opens are the room's (PLAYING.md,
"While you play"), and a copy of the pause menu would be one more game file
to carry over on every patch.

## The build profile

The entry's targets are in the release's built-in profile for the build
(`profiles/tf3_build40408_steam_windows.toml`, `docs/HOOKS.md`), next to the
step gate's: `lua_loadfile` is detoured, the others only called. The first
three are optional there: without them the menu stays the game's and the
step gate still installs. `tpf3mp-hookcore/tests/tf3_static_proof.rs` pins
their addresses in the installed game (`TPF3MP_TF3_EXE`).

| target | what | how to find it again |
|---|---|---|
| `lua_loadfile` (0x2fa1d50) | the `resolveutil.loadfile` body | `Loader.cpp`: the function with the strings `Could no load file`, `base/tl.lua`, `Error while pcalling` |
| `lua_load` (0x2fbdf70) | Lua 5.2 `lua_load` | the only caller of `luaD_protectedparser` (the function that references the `attempt to load a %s chunk` check); `luaZ_init`, a `"?"` default chunk name, then the `_ENV` upvalue fix-up. Not the nearby `lua_dump`, which checks for a Lua closure on the stack top and returns 1 |
| `lua_pcallk` (0x2fbe0c0) | Lua 5.2 `lua_pcallk` | called right before `Error while pcalling` in the loader body; reads `L->top`, `L->stack`, `L->nny`, calls `luaD_pcall` |
| `lua_settop`, `lua_pushlstring`, `lua_tolstring` | Lua 5.2 | already the step gate's (the Lua link, `docs/HOOKS.md`) |

On a patch: `tpfre index` the new exe, find them again with `tpfre q`
(`str`, `callers`, `dis`), regenerate the signatures with `sig --toml`, and
put them in the new build's profile. The hook tries the menu's targets in
every profile that matches the build, the data folder's first.

## Talking to the launcher

The window asks the hook for the lobby through the request channel above
(`tpf3mp/state.lua`, a few times a second) and sends its actions the same
way (`tpf3mp/act.lua`). The hook does not answer on its own: an action is
queued for the launcher that started the game and handed to its agent over
the link (`ToAgent::Lobby`), and the state is the launcher's lobby as the
agent last sent it (`ToHook::Lobby`, bridge version 10). Every request also
reads the link, since at the main menu no step of the game does
(`crates/tpf3mp-hook/src/lobby.rs`; `docs/HOOKS.md`, "The main menu's
Multiplayer window"). The launcher carries the actions out as if its own
window had asked; Connect goes to its own server (D12). A game whose hook
has no link to its launcher shows so in the window and sends nothing. The
hook's answer to an action is `ok`, or `error: ` and why it refused it
(such as a name too long), which the window shows.

**Mods** (docs/MODS.md, "Choosing mods"). The lobby carries the player's
installed mods (`mods`: `{ id, name, class, reason, chosen, choosable }`,
class `personal`, `carried` or `shared`, those the player may choose first,
64 at most) and the room's shared mods once known (`room_mods`: `{ id,
version, have }`, have `yes`, `no` or `other_version`, 32 at most, and
`room_mods_more` beyond). The window chooses one with
`{"action":"choose_mod","id":"<id>","chosen":true|false}`; the launcher
refuses a mod that is not choosable, with why. Bridge version 13.

**The server** (D12, proposed amendment). The lobby carries the server as
players see it (`server`, its name or address), its address
(`server_address`, `host:port`) and the launcher's default
(`server_default`; empty without one). The window changes the server with
`{"action":"set_server","server":"host:port"}`, or `"server":""` to go
back to the default: the launcher refuses anything but a `host:port`, and
any change while in a room, with why; otherwise it remembers the server,
and if connected it disconnects and connects there under the same name.
An invite never changes the server. Bridge version 15.

**The start save.** The lobby lists the player's saves, newest first, by
name: those `steam::find_save` finds by that name, in the save folder of
the Steam account playing (`steam::list_saves`, looked at every 5
seconds). Create names one of them, or none. The launcher takes only a
name it listed, never a path, finds the file and hands it to the room as
`--start-save` does (`BridgeOptions::start_world`); a save it cannot find
creates no room. The launcher's own `--start-save` is offered first, then
the save last picked.

## The mod's copies

`mod/tpf3mp_1/content/gui/menu/main_page.tl` is a copy of the game's file.
Every change is marked `TPF3-MP:`; the relative requires and asset paths are
made absolute (`::/...`), because a leading-slash path is resolved against
the requiring file's root, which for the mod's copy is `tpf3mp_1::/`. On a
game patch, take the new game file and re-apply the marked blocks. The copy
is listed in `_content.json` like any other file of the mod.

## Trying it

Start the launcher, then **Start Transport Fever 3** (before or in a
room), and click **Multiplayer** on the game's main menu. The mod must be
installed in the game's staging area and active. After changing
`main_page.tl`'s additions, run `tools/lobby/make_main_page.py` on the
game's file again rather than editing the copy.

`tools/lobby/launch-tf3-dev.bat` and `tpf3mp-launch` start the game with the
hook but without a launcher: the entry and window appear, and the window
says the game has no link to the launcher. They are for checking the entry
alone.
