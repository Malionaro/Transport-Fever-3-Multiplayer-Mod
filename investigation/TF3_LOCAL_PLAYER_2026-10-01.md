# The engine's player, build 40408 (2026-10-01)

Why a company's own stops and stations act as another player's in its
player's game, and whether the hook can make the game's tools and GUI act
as the player's company while the simulation keeps the save's player.
Static findings with `tools/tpfre` on the installed executable
(sha256 `de1daad3…f23ef2`, the profile's build), unless marked otherwise.

## Seen in hook.log (2026-10-01, three players, competitive)

- **seen** Every proposal the game's own tools made in james's game, and in
  cat's, carries `playerEntity=118368`: the save's player, the room's first
  company, although each played for a company of their own (8 of 8 in
  james's log). The room rebuilds each with the acting company as owner
  (`apply.lua`, `company()`), so the stop is the company's and the tool's
  player is not its owner.
- **seen** `CMenuUI::m_game` is at `+0x6b0` (the hook's own reading of
  `DoStep`'s test, logged as `CMenuUI::m_game (+0x6b0)`).

## Where `api.engine.util.getPlayer` gets its answer

- **seen** The string `"getPlayer"` (rva `0x3760360`) is used once, in
  `sub_2536fc0` ("SetupUtilInterface"), at `0x2539fe4`. Its function is a
  copy (`sub_e27e80`) of a `std::function` the function takes as its second
  argument, handed to the registration `sub_24791a0`, which nothing else
  calls.
- **seen** That argument comes, through `sub_f301b0` ("SetupEngineInterface")
  and `sub_1087b40`, from each Lua state's setup: a
  `std::function<GameState const &()>`. There are three:
  - the engine's (game scripts'): `CGame::CGame`'s lambda_3/lambda_1,
    `sub_11ffd0`: `[[CGame+0x1f0] + 0x78 + 8 * i]`, with `i` the
    word at `+0x98`, or `1 - i` when the lambda's flag is set. Two
    `GameState` buffers.
  - the GUI's: `UI::CMenuUI::SwitchToGameUI`'s lambda_1/lambda_2,
    `sub_6aa800`: `[[CMenuUI+0x6b0] + 0x1e0]`, that is
    `[m_game + 0x1e0]`.
  - the React GUI's: `UI::react::ScriptComponentRoot::ReloadInterfaces`'s
    lambda_6, `sub_27c80a0`: a getter's object, then `+0x1e0`.
- **inferred** `getPlayer` answers a field of the `GameState` its state's
  getter gives. The engine's scripts read one of `CGame+0x1f0`'s two
  buffers, the GUI's read `CGame+0x1e0`.

## Not established

- Where in `GameState` the player entity is (the `getPlayer` closure that
  reads it is inside luabridge's template code; not found yet).
- Whether `CGame+0x1e0` is a `GameState` of its own or one of the two
  buffers at `CGame+0x1f0`, and whether the buffers swap or are copied
  between steps. If the GUI's is one of the two buffers, a value the hook
  wrote there for the GUI would become the simulation's at the next swap:
  the games would diverge.
- What the native street, track and construction tools and the line
  manager's picker (`newContext.player = true`, `line_util.tl`) read as
  their player: the same `GameState` field, or another copy (`DataLogger`
  keeps one of its own, `m_playerEntity` at `+0x24`, `sub_a7790`).

So it is not shown that the tools and the GUI can act as the player's
company while the simulation keeps the save's player, and the hook writes
nothing for it. What would show it, read only, in a real game: log
`CGame+0x1e0`, `[CGame+0x1f0]+0x78`, `+0x80` and `+0x98` at each
`CGame::Step` for a few frames (do they move, does `+0x1e0` equal either
buffer), then locate the player field by the value 118368 in each.

## Why the street tool will not split a road the room built (2026-10-02)

Reported in a competitive room: the street tool snaps into the middle of
the save's roads, but only to the ends of roads built during the session,
the player's own company's included; some of those roads cannot be
bulldozed either, and hook.log has no bulldoze for them. Static findings
with `tools/tpfre` on build 40408:

- **seen** The room's replay makes each road edge it builds the acting
  company's (`apply.lua`, `networkInto`: `link.owned` gives the edge a
  `PlayerOwned` of `company()`, e.g. 372363), where the native tool, in
  single player, makes it the save's player's. The engine adds that
  `PlayerOwned` as given (`street_util::AddToEngine` 0x25f9480, `0x25f957f`);
  the edge's `BaseEdgeStreet` is always added for a street (`0x25f9514`),
  so it is no other component that differs.
- **seen** `sub_610ea0` (`game\ui\actions\street_builder_util.cpp`), called
  here `IsOwnedByOtherPlayer(engine, player, entity)`: false for a
  negative player; looks the entity's `PlayerOwned` up and answers
  `owner != player`; false for an entity no player owns (a town's road).
- **seen** The street builder's snap,
  `CreateFindSnapPointRoadEarlyAbortContext` (0x60c360) through `0x5fb370`:
  for each candidate edge it reads whether each end node belongs to a
  construction (`sub_b4db80`, the flags at `[rsp+0xa4]`, `[rsp+0xa5]`), then
  calls `sub_610ea0` (0x5fc022) and, when it answers true, sets both flags:
  the edge is snapped to as a construction's is, at its ends only.
- **seen** The player it passes is the street builder's own,
  `UI::StreetBuilder+0xc0`, stored by its constructor (0x56a740, `mov
  [rsi+0xc0], eax` at 0x56a816) from an argument, and handed down by
  `StreetBuilder::Step` (`mov eax, [r14+0xc0]`, 0x586b4e) to `sub_571820`
  and `sub_25ef740`. The tools' proposals carry the save's player
  (`playerEntity=118368`, seen above), so that is the player it holds.
- **seen** The street bulldozer filters the same way:
  `UI::StreetBulldozerAction::vf2` (0x5f2e30) and its check `sub_5f2a00`
  call `sub_5f7db0(engine, players, entity, flag)`, true when the
  players list is empty or the entity's `PlayerOwned` owner is in it,
  else the edge is not offered ("Protected - Cannot Be Bulldozed" is the
  other branch). The module and street connector bulldozers call it too.
- **inferred** So for a player of any company but the room's first, every
  edge the room built for a company is another player's to the native
  tools: no split, no bulldoze, and (the earlier reports) no snapping onto
  the company's own stations and no tram track onto its rail. Ends still
  connect: a node has no `PlayerOwned`, and an edge marked fixed is still
  snapped to at its nodes. The save's roads are the save's player's or a
  town's (no owner), which the test lets through.

Chosen (2026-10-02): the tools act as the player's company, GUI objects
only (`crates/tpf3mp-hook/src/toolplayer.rs`; docs/HOOKS.md, "The tools'
player"). `sub_610ea0` itself stays as it is: `construction_builder_util`
(`MakeProposalRemove`, `CreateProposalReplace`, `MakeStreetProposal`)
reaches it too, and the simulation calls those.

- **seen** `UI::CGameUI::CGameUI` (0x647860) reads the player once
  (`[rbp-0x50]` from `+0x20c` of the object `sub_8692e0` returns, 0x648ace)
  and hands it to the street builder (0x649bd0, and the track builder,
  the same class, 0x64a058), the track modifier (0x64a2f5) and the other
  tools.
- **seen** The fields and their readers, each the class's own code:
  `UI::StreetBuilder+0xc0` (stored only by its ctor 0x56a816; read by
  0x56fc80, 0x5755b0, `CreateProposalAndUpdate` 0x577ed0, 0x580900, `Step`
  0x585e50, 0x589980, 0x58caf0); `UI::TrackModifier+0xa0` (stored only by
  its ctor 0x5b424c; read by `Step` 0x5bc120, `Build` 0x5bf880, vf5
  0x5cbf80, 0x5cf120; the snap gets it at 0x5cca24 through 0x5c3030,
  0x5d40b0, 0x25ef830, 0x60d410, 0x5fe0b0 to `sub_610ea0` at 0x5fe1d0);
  `UI::Bulldozer`'s `BulldozerFilter` (made only at 0x4c4c32, kept at
  `+0xc0`), whose player list at `+0x10` the ctor sets to the bulldozer's
  player (0x4c4f33..0x4c4fb7) and only `BulldozerFilter::vf1` (0x4d5070)
  copies into each action's query. Found with `tools/tpfre`: every dword
  read of each offset over the class's code range, and the vtables'
  only users (ctor, dtor).

To see it in one try: `TPF3MP_PROBE_PLAYER=1` logs each entity the test
takes for another player's, with the tool's player and whose the entity
is (docs/HOOKS.md, "The probe of the engine's player").

### The station tools and the bulldozer's own player (2026-10-02)

Game test of aaa331c: the street tool split the company's road; stations,
stops and bulldozing the company's road still did nothing, with no capture.

- **seen** `CGameUI` hands the same player ([rbp-0x50]) to
  `UI::ConstructionBuilder` (ctor `sub_50b3b0`, 0x649f28, by value; stored
  at `+0xa0`, 0x50b453), `UI::StreetTerminalBuilder` (ctor `sub_5906e0`
  through the factory `sub_645ee0`, by pointer, dereferenced at 0x646035;
  `+0xa0`, 0x590795; built twice, the stop builder and the signal and
  waypoint builder) and `UI::ModuleBuilder` (ctor `sub_540390` through
  `sub_645940`, dereferenced at 0x6459c0; `+0xa8`, 0x540456). Each field's
  readers are its class's own code (profile comments list them).
- **seen** `UI::Bulldozer` keeps its own player at `+0x28` (ctor 0x4c4a4f,
  its 4th argument); besides seeding the filter (0x4c4f33) it is read for
  the proposals it makes (`Step` 0x4d687e, 0x4d46b0, 0x4d2b70, and its
  lambda 0x4d2650, which hands it to `sub_ccf6d0` at 0x4d2952). The
  street bulldozer's own test reads the filter's copied list only (both
  `sub_5f7db0` calls, 0x5f2ae6 and 0x5f376f, take the query's list), so
  the filter was right and the bulldozer's player was not: the edge was
  offered, the proposal made for the save's player was empty.
- **inferred** That the proposal is what refused the bulldoze; the probe
  (TPF3MP_PROBE_PLAYER=1) logs both tests' answers to confirm it.

All four now get the company the same way (toolplayer.rs).

### The bulldozer's owner list, set again by the menu (2026-10-02)

Game test of 95127b2, p0 (company #1, 372630), `TPF3MP_PROBE_PLAYER=1`:
bulldozing the company's road still did nothing. The probe answered the
same edges both ways in the same 3 s, e.g. entity 325970: allowed with
owner list [372630] (72 times), refused with [214443] (2 times), for both
the edge test and the owner test, while the tool-company lines showed the
bulldozer's player and filter written at 16:43:10.

- **seen** The bulldozer keeps its own owner list at `+0xa8` (a
  `std::vector<Entity>`, 0x4c4ae3), which its ctor fills with its player
  (0x4c4f6f) and copies into the filter (0x4c4fb7). Every query it makes
  is built from it by `sub_4d51c0` (its 5th argument, `lea [this+0xa8]` at
  0x4d2865, 0x4d29d6, 0x4d2e7b, 0x4d4aa2, 0x4d6a60).
- **seen** `sub_4d6220(this, list)` assigns a list to `+0xa8` and to the
  filter's copy. Its only caller is `sub_6a1410` (0x6a14bc), from
  `sub_6a76e0` from `sub_68f070` from `CMenuUI::DoStep`'s lambda: it passes
  `{ [[game+0x1e0]+0x20c] }`, the GUI GameState's player (the save's), or
  an empty list when a flag (`cl`, or `[[game+0x1d8]+0x10]+0x4d9`) is set.
- **inferred** The menu sets the list back between the tool's frames, so
  queries made after it (the click) carried the save's player. The
  detour of the setter puts the company back right after each call.

### The GUI's views (2026-10-02)

Game test of 9ad8837: the line manager showed company #0's stations to a
company #1 player and not their own; their stops read as "another
company's". hook.log said getPlayer followed in all three of the mod's GUI
states.

- **seen** Every view named decides "mine" in Lua at call time: the
  game's `entity_util.isOwnedByPlayer` / `isOwnedByPlayerOrNotOwned`
  (manager_window.tl, line_util.tl, station_group.tl, manager_hud_util.tl,
  statistic_stations.tl, maintenance_station.tl) and
  `api.engine.util.getPlayer()` passed to native queries
  (`getLinesForPlayer`, `requireOwnedByPlayer`, `getLinesIssues`,
  `getPlayersBalance`). None reads a native player of its own.
- **seen** In p0's log the probe found the GUI's `GameState`
  (`CGame+0x1e0`) to be the engine's buffer [0], then [1], the player
  214443 at `+0x20c` in each: the GUI's player is the simulation's.
  Writing it is not lockstep-safe, so the fix stays in Lua.
- **inferred** A state's api is made anew after the mod's script installs
  (once per Lua state, by a `package.loaded` flag). follow.lua now
  re-installs on the current api whenever it is not there, from the
  ownership tests, and answers from the hook's company note where the
  roster cannot be read. Its log lines say which state answered what.

### The map's markers and overlays (2026-10-02)

Game test of bd3c695: the line manager followed the company, but the map's
station icons showed company #0's stations and not #1's (372631), and a
line company #1 made showed in the line manager but not on the map.

- **seen** No helper decides "the local player's": every native reader
  calls the GUI's `IGameStateProvider` (`[rax+8]`) and reads `+0x20c`
  inline. tpfre found 79 reads of `[reg+0x20c]`; the UI ones are
  `HudIconManager::PreemptiveOctreeTraversal` (0x67b7d5), `StationViewer`
  (0x83a602), the selector (0x839cd3, `ViewCreator::vf1` 0x86711c), the
  catchment overlay (0x8765ee, 0x876f28, 0x8770b4, 0x8770ef, 0x877b39), the
  layer colours (0x87b83a, 0x87b913, 0x87b9e9, 0x883075, 0x885c82), two
  React components (0x29f6894, 0x289e10f), plus the tools' (toolplayer.rs)
  and a few the debug panel, notifications and the asset manipulator make.
  The others are the simulation's (`CGame::RunGameSimLoop`, `GameState`
  load and replication, `DataLogger`), the construction apply, and the
  scripting bindings (`0x24f…`), which game scripts call too.
- Each UI read is spliced (guiplayer.rs). INFERRED: that the map's line
  overlay is the layer colours' (`LayerManagerColorMap`, the line layer's
  colour function); if a line still does not show, the next place is a
  reader this list does not have.

### Every company on the map, the map's lines, the depot (2026-10-02)

Game test of eba8614 (p1, company #1, 372426): station icons followed the
company; the player's own line was not drawn on the map although the
layer colours' owner test ran for the company; vehicles bought from the
line's store left a depot far away, not the road depot just built. The
owner then asked for every company's icons and lines on the map.

- **seen** The five scripting readers of `+0x20c` are the bindings
  `getVehicleProblems`, `getLineProblems`, `headquarters.getTransportedData`,
  `getCompaniesValue` and `getPlayer` itself (`sub_24ed220`, registered by
  `sub_24791a0` at 0x253a01a): each calls its state's `GameState` getter, a
  `std::function` at `+0x38` of the closure. The GUI's getters are
  `CMenuUI::SwitchToGameUI`'s (0x6aa800, vtable 0x36c9ad0) and
  `ScriptComponentRoot::ReloadInterfaces`'s (0x27c80a0, vtable 0x3787768);
  the engine's is `CGame::CGame`'s (0x11ffd0).
- **seen** The map's line overlay is a React `UI::LineViewer` fed from Lua
  (`params::LineViewer`, `LineVisualization` user data).
- **seen** The HUD icon pass `sub_674430` filters every icon entity by
  `PlayerOwned` owner == the pass's player (0x67490f); no owner passes.
- **seen** `findBestDepotForLine` takes a carrier, transport modes, the
  line and a position from Lua, no player (`line_util.tl` 1951).
- **inferred** The line overlay missed the company's line because its Lua
  state's getPlayer was the game's; the native getPlayer answer covers it.
  Not known yet: why the depot chosen was far away (the new depot not on
  the line's network, the depot build's "Collision" warning, or an owner
  filter inside `findBestDepotForLine`); hook.log now names the depot each
  purchase uses and its owner.

### The map's line, still not drawn (2026-10-02)

Game test of 55f81ed (p1, company #1, 372553): the native getPlayer, the
layer colours' owner test, the station viewer and the selector all
answered the company; CreateLine (action 6) and EditLine 7-9 went through,
the line's stops three room-placed two-sided stops each made the
company's with a station group of its own; the line was not drawn.

- **seen** Every map line is drawn by a `LineViewer` given its lines by a
  window's Lua (`showLines`); the native `UI::LineViewer` reads the `Line`
  component and no owner. The game's HUD has no always-on line overlay in
  its Lua (no other `LineViewer` user; the public transport layer colours
  stations by happiness).
- **seen** The map's other line renderer, `UI::LineRenderView`, is the map
  generator preview's (`map_preview_util::MakeStreetGeneratorLayerFn`).
- Not known: whether the company's line reaches a `LineViewer`. The probe
  (`follow.watchLines`) says, in one try, what each viewer is handed and
  what `getLinesForPlayer` answers.

### The line is listed, not drawn (2026-10-02, probe of 3d91e83)

- **seen** (p0, company #2, 372671) `getLinesForPlayer(372671)` answers
  338852, owned by 372671, and a viewer is handed it; it is not drawn. So
  the line reaches the viewer; ownership is not what stops it.
- **seen** `UI::LineViewer` draws from `LineSystem::GetData(line)`
  (`line2data`), comparing the data's revision (`+0x18`) with its own and
  the data's per-stop segment count with the line's stops before it draws
  (`sub_7f01d0`); no player is read on that path (tpfre, to depth 3).
- **inferred** The line system's data for a line whose stops are the
  room's placed stops is missing or does not match its stops: a stop naming
  a station or terminal the stop does not have (a two-sided stop is two
  stations in one group, each with its own terminals), or a route the line
  system could not compute. The probe now reports each stop and the
  engine's verdict for every line a viewer is handed, the first company's
  lines too, so one try compares them.

### Drawing depends on the owner, not in the viewer (2026-10-02, probe of 34a2edf)

- **seen** (p1) company #1's line 342589, its stops' groups, stations and
  terminals all there and the company's, listed at every terminal, "3
  detailed stop problem(s)", not drawn; the first company's new line
  329152 on room-placed stops, "2 detailed stop problem(s)", drawn.
- **seen** No player read and no `PlayerOwned` lookup six calls deep from
  the viewer (tpfre). The line system's own owner reads (`LineSystem` vf4,
  vf5, vf8, vf9, through `sub_95790`) keep its player-to-lines index
  (`+0x58`, what `getLinesForPlayer` reads), not route data. The sim
  loop's player read (`sub_157860`) is `player_util::ClearPositions`.
- **open** What makes the line system's data for a company's line differ.
  The probe now logs the viewer's route data test per line (revision and
  segment lists against what it expects), so one try shows whether the
  company's line fails it and how.

### Found: the line viewer's candidate lines are one player's (2026-10-02)

- **seen** (4415a50 probe, p1) a founded company's line and the first
  company's both got edge geometry with stop filter 0 only, and the first
  company's alone got the whole route (stop filter -1), and only while the
  player played for the first company; the owners' components are the
  same (ACCOUNT COLOR LOG_BOOK NAME PLAYER).
- **seen** The -1 call (0x7f6598 in `LineViewer::Update`) runs for the
  candidate lines `sub_7f3ea0` makes, as they age past 1.0 (the time since
  each line's last whole build, `+0x60`). `sub_7f3ea0` takes them from
  `sub_ad2620(index, [viewer+0x28])`, the line system's player-to-lines
  index (`[lineSystem+0x58]`, kept by `LineSystem` vf4/vf5/vf8/vf9).
- **seen** `[viewer+0x28]` is the player `sub_29f66d0` read when it made
  the viewer (`mov ebx,[GameState+0x20c]` at 0x29f6894, passed to
  `sub_7edbd0`, stored at 0x7eddc0).
- Fix: the call at 0x7f3f12 answers every company's lines in a room
  (guiplayer.rs, `lines_for`).

### The store's depot (2026-10-02, build 45b8ed5)

- **seen** p1 (company #2) built a road depot (action 2) and bought from
  the line window: `the store buys at depot entity 317114 (owned by
  214443)`.
- **seen** `findBestDepotForLine` (registered at 0x253c13f) and
  `findBestLineAndDepotForVehicle` (0x253c094) reach `sub_2689fd0` (owner
  test at 0x268a335) and `sub_2689dd0` (owner test at 0x2689eeb, through
  `sub_5ab270`, a `PlayerOwned` read), each comparing with
  `[GameState+0x20c]`, the save's player. Their only callers are the two
  bindings; their only Lua callers are `line_util.tl`,
  `manager_window.tl` and `vehicle_store_window.tl`.
- Fix: both tests spliced (guiplayer.rs), on the GUI's thread outside the
  step only.
- **closed in replay** A room action could name another company's depot
  because `BuyVehicle` checked only that the depot existed. The line-store
  view patch filters the GUI choice, and the replay now requires exact depot
  ownership by the acting company whenever more than one company is live;
  a missing or unreadable `PLAYER_OWNED` fails closed. A one-company room
  keeps the game's native purchase behavior. The D22 station-access decision
  explicitly does not grant use of another company's depots.
