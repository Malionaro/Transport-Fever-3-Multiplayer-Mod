# Company progression in Transport Fever 3 (build 40408)

How a company ranks up, read from the game's own scripts without starting
the game, for splitting it between a room's companies (docs/DECISIONS.md
D23, proposed; `mod/tpf3mp_1/content/scripts/tpf3mp/progression.lua`).

Sources: the game's scripts in `base/content/game_mechanics.zip`
(`game_mechanics/company/...`, `game_mechanics/towns/...`,
`game_mechanics/subventions/...`) and `scripts.zip`
(`scripts/util/town_growth_function.tl`), paths inside the archives, line
numbers as extracted; the API reference shipped with the game,
`api/tealdef/api/...`. Each fact is marked **seen** (read in code, file and
lines) or **INFERRED**.

## 1. The score and the rank

- **seen** The score is called experience and is the world's population:
  the growth script's update reads
  `company_util.computeWorldPopulation()` (`company_growth.script.tl:86`),
  the sum of the first entry of every town in
  `townBuildingSystem.getTown2personCapacitiesMap()`
  (`company_util.tl:365-371`), and raises the company's experience to it
  when it is higher (`ensureState`, `company_growth.script.tl:25-27`). It
  never falls. Nothing in it looks at who serves a town: an unserved town
  counts as much as a served one. The only other way in is the debug event
  `_debugAddExperience` (`:144-151`).
- **seen** The rank a company has reached (`potentialLevel`) is
  `company_progression_util.getLevelAndFraction(basePopulation,
  experience)` (`company_growth.script.tl:36-53`,
  `company_progression_util.tl:97-122`); the rank it has taken (`level`)
  moves only by `applyLevel`, to a rank above it and at most the potential
  (`company_growth.script.tl:55-62`).
- **seen** `basePopulation` is the world's population when the game began
  (`company.script.tl:392-401`, on `initNewGameFromMap`/`initMission`; a
  legacy save takes today's, `company_legacy_util.tl:41`).
- **seen** The thresholds: rank 1 at `basePopulation`, rank r at
  `max(r, floor(p * F(r)) - p + basePopulation)` with `p` blended from 725
  towards `basePopulation` as r rises
  (`company_progression_util.tl:6-14`), F the world growth factor, a cubic
  with F(1) = 1 and F(15) = 12.5 (`scripts/util/town_growth_function.tl:13-20`).
  Fifteen ranks (`company_progression_util.tl:21-23`), named Junior to
  Tycoon (`company_static_util.tl:6-24`). So rank r needs about F(r) times
  the population the world began with.
- **seen** One company only. The growth script scores
  `api.engine.util.getPlayer()` alone (`company_growth.script.tl:110`), in
  the engine state the save's player; `applyLevel` applies to that player
  whatever the event says (`:156`). A second player entity (TPF3-MP's
  companies, D21) has no progression state at all, and
  `company_progression_util.getCompanyProgressionState` answers nil for it
  (`company_progression_util.tl:74-95`).
- **seen** `company_growth_config.res.lua`: `enforcedLevel = nil`
  (a mission's fixed rank), `ticketPriceMultiplierAtMaxLevel = 0.5`: the
  ticket price multiplier moves linearly from 1 at rank 1 to 0.5 at rank 15
  (`company_progression_util.tl:25-40`), applied through
  `OnCalcTicketPrice` for the save's player
  (`company_growth.script.tl:159-171`).

## 2. When it is recomputed

- **seen** The growth script is a game script with an update
  (`company_progression.gs.lua`); its update recomputes experience and
  potential on every call (`company_growth.script.tl:73-113`). INFERRED
  from the mod's own measurement of game script updates (tpf3mp_sim: once
  per simulation update) that this is every simulation update.
- **seen** A new potential rank sends the rank-up notification and
  `Companies` `OnLevelUp` (`:96-108`).
- **seen** The company window takes a rank with
  `makeScriptingSendEventCmd("", "Companies", "applyLevel", { level })`
  (`company.tl:421-422`), a GUI command: in one game only unless the room
  carries it.

## 3. "Company rating"

- **seen** There is no rating of a company. The game's rating is a town's:
  `TownState.authorityScore`, 0 to 1 (`towns.d.tl:54-68`), recomputed by
  the towns script (`towns.script.tl:148-150`) as the lowest of six parts
  (`town_util.calcAuthorityScore`, `town_util.tl:745-771`, the parts
  `town_util.GetRatings`, `:709-718`): reputation (`urban_care`), traffic,
  people's happiness, cargo delivery on time, noise and pollution. Each is
  kept in `TownState.cachedRatings[key].value`. It is shown in six levels,
  Very Poor to Excellent (`town_util.tl:955-972`).
- **seen** Two parts are made of what lines carry, and the game's
  statistics give both per line:
  - happiness: `getRatingHappinessFromStats` (`town_util.tl:478-502`) of
    unhappy and total people travelling, at least 15 counted, mapped from
    0.3..1 to 0..1 at sensitivity 1;
  - cargo delivery: `getRatingCargoDelivery` (`:504-518`) of late and
    delivered items over half a year, at least 10 counted, mapped the same.
  The other four are the town's (its reputation events, streets,
  emissions), not any company's.
- **seen** Sensitivities per town and per part:
  `town_util.getRatingSensitivity(townState, key)` (`town_util.tl:81-90`).

## 4. Measuring each company's deliveries per town

- **seen** `api.engine.util.town.getTownDeliveriesStats(town, interval,
  perLine, perCargoType)` (`api/engine/util.d.tl:635-641`): cargo delivered
  to the town, `{late, delivered}` per line and cargo type. The game asks
  for it per line and cargo over half a year for the town window
  (`town_util_parallel.script.tl:140`), and totals it with the key -1
  (`town_util.tl:509-511`). INFERRED that with `perCargoType` false a
  line's entry has the key -1 alone; the mod sums the cargo types where it
  has not.
- **seen** `getTownHappinessStats(town, numTopLines).byLine`
  (`util.d.tl:585-597, 631-634`): per line, residents and others travelling
  to or from the town, `{unhappy, total}` each; the town window asks for
  the top 5 (`town_util.tl:1596`). A snapshot of people under way, not a
  count of trips (INFERRED from "number of ... people travelling").
- **seen** A line is owned by a player (`PLAYER_OWNED`; TPF3-MP books every
  line a player creates to their company, `tpf3mp/apply.lua`), and
  `lineSystem.getLinesForPlayer(player)` lists a player's lines
  (`api/engine/system.d.tl:18`).
- **seen** Not per town: a line's `LOG_BOOK` `itemsTransported`
  (`gui/entity_window/line/line_eow.script.tl:619`,
  `api.engine.util.logbook.getLogValuePerYear`, `util.d.tl:261-266`), and
  `headquarters.getTransportedData()` for the player's company
  (`util.d.tl:896-898`). A logbook can hold logs per referenced entity
  (`refEntity2name2log`, `engine.d.tl:575`; used for a stock's deliveries
  to a target, `getCargoLogPerYearForTarget`, `gui/entity_window/town/town.tl:227`).
- **seen** `getTownLineUsage(town)`: the town's share of people using
  lines, the town's only (`util.d.tl:692-696`).

## 5. What a rank unlocks

- **seen** Permits: a construction's or prospection's company metadata
  names the ranks that grant permits (`rankAndPermits`), summed up to the
  company's rank (`company_util.tl:33-81`), plus extra permits per rank
  (`company_progression_util.tl:42-72`). Headquarters 1 at rank 1
  (`permitKeys/hq.res.lua`), marketing 1 at rank 4 with a five-year
  cooldown (`permitKeys/marketing.res.lua`).
- **seen** Prospecting: one permit each, at fish rank 2, clay 3, crude oil
  4, logs 5, coal and iron ore 6, stone and wool 8, sand 9, rubber 12
  (`explorations/exploration_*.res.lua`); grain and vegetables name no
  rank.
- **seen** Constructions locked below a rank ("Unlocked at Rank",
  `company_util.tl:171-191`).
- **seen** The permit checks are the GUI's: `company.lockPermits` and the
  build tools' proposals (`company.script.tl:519-611`), both through
  `getCompanyProgressionState(getPlayer())`.
- **seen** Subsidies spawn at a pace set by the save's player's rank
  (`subventions.script.tl:244-253`).
- **seen** The company script runs pending prospections for
  `getPlayer()` alone (`company.script.tl:284-285`): another player
  entity's prospection is kept and its permit used, and never drawn.

## 6. What this means for a room

- With one company, the game's score is already the same in every game
  (the world's population, a function of the simulation). Only
  `applyLevel` is a GUI command, which the room carries (`ApplyRank`).
- With more than one, the game gives no second company a rank. The mod
  keeps one per company in its game script's state, computed from the
  statistics in section 4, the town ratings in section 3 and the lines'
  owners, at game times every game reaches alike, and answers the GUI's
  `getCompanyProgressionState` from it (INFERRED that the game's windows
  and the mod's GUI script share the module table that
  `ug_require` returns in the GUI state; to check in the game, where
  `hook.log` says "the company window shows each company's own rank").
