-- tpf3mp/follow.lua -- the GUI's "my company", in each of the GUI's Lua
-- states.
--
-- TF3's windows and its HUD ask api.engine.util.getPlayer() whose money to
-- show, what is the player's own and what is "Foreign"
-- (entity_window/eow_extension_util.tl, entity_util.isOwnedByPlayer), and
-- every GUI script looks it up when it runs. The game's GUI runs in more
-- than one Lua state (build 40408: the Multiplayer plugin's, and the one the
-- HUD's icons and the line manager's depots are drawn in), each with its own
-- api table, so each state that shows the player's things is given the
-- answer here: the company this player plays for. The game scripts' states
-- keep the game's own answer, the save's player, so the simulation is the
-- same in every game; what the player does is booked to their company by
-- the room (tpf3mp/apply.lua) whatever the GUI named.
--
-- A GUI state's api can be made anew after the mod's scripts ran (a
-- window's React root reloading its interfaces, INFERRED from the line
-- manager filtering by the save's player in a room, 2026-10-02, while every
-- state had said it followed): follow.ensure puts the answer back in front,
-- and the game's ownership tests (entity_util) call it before they answer.
--
-- Pure Lua; the tests hand it a fake api.

local follow = {}

-- Where the answer comes from, in this Lua state: each install adds its
-- `mine`, asked in order; the first number wins. The hook's note
-- (COMPANY_NOTE, below) is a source too, where a state was given its link
-- (follow.noteSource): the Multiplayer plugin's state writes it from the
-- room's roster, and the GUI's native tools already act on it.
follow.sources = follow.sources or {}
-- getPlayer wrappers this module made, so an install over one of its own is
-- a no-op and one over a fresh api is told apart.
follow.wrappers = follow.wrappers or setmetatable({}, { __mode = "k" })
-- getComponent wrappers this module made, so finance-window following can
-- also be restored when the GUI gives the state a fresh api table.
follow.loanWrappers = follow.loanWrappers or setmetatable({}, { __mode = "k" })
-- How many times getPlayer was put in front of a game's own.
follow.installs = 0
-- What the wrappers answered last, for the log (follow.say).
follow.lastAnswer = nil

-- The player entity of the company this player plays for, from the first
-- source that knows it, or nil.
function follow.answer()
	for _, source in ipairs(follow.sources) do
		local got, entity = pcall(source)
		if got and type(entity) == "number" then return entity end
	end
	return nil
end

-- Says `line` through the logger the states gave (follow.say), once per
-- distinct line.
local said = {}
local function say(line)
	if said[line] or type(follow.say) ~= "function" then return end
	said[line] = true
	pcall(follow.say, line)
end

-- The current api of this Lua state: `api` when given, else the global the
-- game's scripts see at call time (a GUI root can be given a new one).
local function currentApi(given)
	if given ~= nil then return given end
	local ok, g = pcall(function() return api end)
	return ok and g or nil
end

local function ensureLoanWindow(api)
	local ok, installed, why = pcall(follow.loans, api, follow.answer)
	if not ok then return false, tostring(installed) end
	return installed, why
end

-- Puts the company in front of api.engine.util.getPlayer in `api` (or the
-- state's current api) unless it is there already: a wrapper that answers
-- follow.answer(), or, when that is nil (outside the room, before the roster
-- is read, the room's first company), the game's own answer. Returns true,
-- or false and why.
function follow.ensure(api)
	api = currentApi(api)
	local ok, util = pcall(function() return api.engine.util end)
	if not ok then util = nil end
	-- A function, or a callable table, as the game's bindings are (build
	-- 40408: a table with a metatable).
	local original = ok and util ~= nil and select(2, pcall(function() return util.getPlayer end)) or nil
	if follow.wrappers[original] then
		local loans, loanWhy = ensureLoanWindow(api)
		if not loans then return false, "the finance window's company loan view cannot follow this api: " .. tostring(loanWhy) end
		return true
	end
	if type(original) ~= "function" and type(original) ~= "table" and type(original) ~= "userdata" then
		return false, "no api.engine.util.getPlayer (" .. type(util) .. ", " .. type(original) .. ")"
	end
	local wrapper = function(...)
		local entity = follow.answer()
		if entity ~= nil then
			if entity ~= follow.lastAnswer then
				follow.lastAnswer = entity
				say("the GUI's getPlayer answers the player's company " .. tostring(entity))
			end
			return entity
		end
		return original(...)
	end
	follow.wrappers[wrapper] = true
	local replaced, why = pcall(function() util.getPlayer = wrapper end)
	-- A binding may take the assignment and keep its own function; read it
	-- back from the api, not from the table held, in case the api hands out
	-- a new one each time.
	local took = replaced and select(2, pcall(function() return api.engine.util.getPlayer == wrapper end))
	if took ~= true then
		return false, "api.engine.util (" .. type(util) .. ") keeps its getPlayer"
			.. (why and (": " .. tostring(why)) or "")
	end
	follow.installs = follow.installs + 1
	if follow.installs > 1 then
		say("the GUI's getPlayer was the game's own again (a new api in this state); it follows the player's company again")
	end
	local loans, loanWhy = ensureLoanWindow(api)
	if not loans then return false, "the finance window's company loan view cannot follow this api: " .. tostring(loanWhy) end
	return true
end

-- The game's finance window reads the loans it lists and offers from the
-- loan script's state (finances_loan_gui.tl, LoanBoard:
-- getComponent(getEntityForGameScript(LOAN_SCRIPT), GAME_SCRIPT).state),
-- which keeps the room's first company's loans only. In this GUI state the
-- loan script's component is answered, for a player of another company,
-- with that company's own loans and the offers it can take
-- (tpf3mp/companies.lua, loanTable); its Obtain and Repay then go to the
-- room as that company's (tpf3mp/guard.lua). The simulation's states, and
-- the loan script's own state, are left alone. Where the company's loans
-- cannot be read, the window shows none and offers none, never the first
-- company's.
follow.LOAN_SCRIPT = "::/game_mechanics/finance/loan.gs"
-- Seconds a company's loans are read for, at most.
follow.LOANS_EVERY = 0.5

function follow.loans(api, mine)
	local engine = api.engine
	local original = engine and engine.getComponent
	if original == nil then return false, "no api.engine.getComponent" end
	if follow.loanWrappers[original] then return true end
	if type(original) ~= "function" and type(original) ~= "table" and type(original) ~= "userdata" then
		return false, "no api.engine.getComponent (" .. type(original) .. ")"
	end
	local function companies()
		local loaded = type(package) == "table" and package.loaded and package.loaded["tpf3mp.companies"]
		if loaded then return loaded end
		return ug_require("tpf3mp_1::/scripts/tpf3mp/companies.lua")
	end
	local function now()
		local ok, t = pcall(os.clock)
		return ok and t or 0
	end
	local loanEntity, cached, cachedFor, cachedAt = nil, nil, nil, nil
	local function companyLoans(company)
		local t = now()
		if cached ~= nil and cachedFor == company and t - cachedAt < follow.LOANS_EVERY then return cached end
		local table0 = { availableLoans = {}, obtainedLoans = {}, freeId = 0 }
		local ok, built = pcall(function()
			local c = companies()
			local state = c.scriptState(api)
			local roster = state and state.companies
			local own = roster and c.byEntity(roster, company)
			if not own then return nil end
			local real = original(loanEntity, api.type.ComponentType.GAME_SCRIPT)
			real = real and real.state
			return c.loanTable(roster, own.id, real, api.util.getDefaultMonthDuration())
		end)
		cached, cachedFor, cachedAt = (ok and built) or table0, company, t
		return cached
	end
	local wrapper = function(entity, kind, ...)
		if kind ~= nil and entity ~= nil then
			local isLoans = false
			pcall(function()
				if loanEntity == nil or loanEntity < 0 then
					loanEntity = api.engine.system.gameScriptSystem.getEntityForGameScript(follow.LOAN_SCRIPT)
				end
				isLoans = type(loanEntity) == "number" and loanEntity >= 0 and entity == loanEntity
					and kind == api.type.ComponentType.GAME_SCRIPT
			end)
			if isLoans then
				local got, company = pcall(mine)
				if got and type(company) == "number" then
					return { state = companyLoans(company) }
				end
			end
		end
		return original(entity, kind, ...)
	end
	follow.loanWrappers[wrapper] = true
	local replaced, why = pcall(function() engine.getComponent = wrapper end)
	local took = replaced and select(2, pcall(function() return api.engine.getComponent == wrapper end))
	if took ~= true then
		return false, "api.engine keeps its getComponent" .. (why and (": " .. tostring(why)) or "")
	end
	return true
end

-- Gives this Lua state the GUI's "my company": `mine` (the player entity of
-- the company this player plays for, or nil) joins the sources, `log`
-- (optional) says what it answers and when it had to be put back, and
-- getPlayer and the game's ownership tests are put in front of the game's
-- own (follow.ensure, follow.entityUtil). Returns true, or false and why.
function follow.install(api, mine, log)
	local known = false
	for _, source in ipairs(follow.sources) do
		if source == mine then known = true end
	end
	if not known and mine ~= nil then follow.sources[#follow.sources + 1] = mine end
	if log ~= nil then follow.say = log end
	local ok, why = follow.ensure(api)
	if ok then follow.entityUtil() end
	return ok, why
end

-- A source answering the company the hook's note names (COMPANY_NOTE,
-- written by the Multiplayer plugin's state), for `link`.
function follow.noteSource(link)
	return function()
		if not (link and link.note) then return nil end
		local ok, text = pcall(function() return link:note(follow.COMPANY_NOTE) end)
		local entity = ok and tonumber(text) or nil
		if entity ~= nil and entity >= 0 and entity % 1 == 0 then return entity end
		return nil
	end
end

-- The game's ownership tests the windows ask (scripts/entity_util.tl:
-- isOwnedByPlayer, isOwnedByPlayerOrNotOwned; the line manager, station,
-- vehicle and depot windows), wrapped in each entity_util table of this
-- state so getPlayer is put back in front first, should the state's api
-- have been made anew since. They then answer as the game does, with the
-- company. Returns how many tables were wrapped.
follow.ENTITY_UTIL = { "/scripts/entity_util.tl", "::/scripts/entity_util.tl" }
follow.wrappedTests = follow.wrappedTests or setmetatable({}, { __mode = "k" })
function follow.entityUtil()
	local okRequire, require_ = pcall(function() return ug_require end)
	if not okRequire then return 0 end
	if type(require_) ~= "function" then return 0 end
	local count = 0
	for _, path in ipairs(follow.ENTITY_UTIL) do
		local ok, util = pcall(require_, path)
		if ok and type(util) == "table" then
			for _, name in ipairs({ "isOwnedByPlayer", "isOwnedByPlayerOrNotOwned" }) do
				local test = util[name]
				if type(test) == "function" and not follow.wrappedTests[test] then
					local wrapped = function(...)
						follow.ensure()
						return test(...)
					end
					follow.wrappedTests[wrapped] = true
					util[name] = wrapped
					count = count + 1
				end
			end
		end
	end
	return count
end

-- The player entity of the company the player `me` (64 hex digits) plays
-- for in `roster` (tpf3mp/companies.lua), or nil: not in the roster, or
-- playing for the room's first, which is the save's own player anyway.
-- The note (tpf3mp_native.note) that tells the hook the player entity of the
-- company this player plays for, "" for none (the room's first company, or
-- outside the room). The hook writes it into the GUI's native tools' own
-- player (crates/tpf3mp-hook/src/toolplayer.rs), so they take the company's
-- roads and constructions for the player's own, and its probe names it
-- (probe.rs); nothing the simulation does depends on it.
follow.COMPANY_NOTE = "tpf3mp.company"

-- Notes `entity` (or none) under COMPANY_NOTE through `link` when it is not
-- `last`, the text noted before. Returns the text noted now.
function follow.noteCompany(link, entity, last)
	local text = type(entity) == "number" and string.format("%d", entity) or ""
	if text ~= last and link and link.note then
		pcall(function() link:note(follow.COMPANY_NOTE, text) end)
	end
	return text
end

-- The note that tells the hook the room's companies' player entities,
-- comma separated ("" for none): the map's icons and line colours show
-- every company's (crates/tpf3mp-hook/src/guiplayer.rs).
follow.COMPANIES_NOTE = "tpf3mp.companies"

-- The room's live companies' entities as their note says them.
function follow.companiesText(roster)
	local out = {}
	for _, c in ipairs(type(roster) == "table" and type(roster.list) == "table" and roster.list or {}) do
		if type(c) == "table" and not c.gone and type(c.entity) == "number" and c.entity >= 0
			and c.entity % 1 == 0 then
			out[#out + 1] = string.format("%d", c.entity)
		end
	end
	return table.concat(out, ",")
end

-- Notes the room's companies under COMPANIES_NOTE through `link` when the
-- text is not `last`. Returns the text noted now.
function follow.noteCompanies(link, roster, last)
	local text = follow.companiesText(roster)
	if text ~= last and link and link.note then
		pcall(function() link:note(follow.COMPANIES_NOTE, text) end)
	end
	return text
end

-- The probe of the map's lines (with the hook's TPF3MP_PROBE_PLAYER=1, which
-- notes PROBE_NOTE): in each GUI state, what the game's LineViewer is
-- handed to draw (gui/main/builtin.lua; the line manager hands it
-- getLinesForPlayer(getPlayer()), manager_window.tl) and what
-- lineSystem.getLinesForPlayer answers, each line with its owner, once per
-- distinct answer and LINES_SAID at most. Nothing changes what is drawn.
follow.PROBE_NOTE = "tpf3mp.probe"
follow.LINES_SAID = 40
follow.BUILTIN = "::/gui/main/builtin.lua"

-- "373300 (owned by 372553)" for each line of `lines` (entities, or
-- LineVisualizations with an entity), the first 12.
function follow.linesText(api, lines)
	local out, n = {}, 0
	for _, l in ipairs(type(lines) == "table" and lines or {}) do
		n = n + 1
		if n <= 12 then
			local entity = l
			if type(l) ~= "number" then
				local ok, e = pcall(function() return l.entity end)
				entity = ok and e or nil
			end
			local owner
			pcall(function()
				local o = api.engine.getComponent(entity, api.type.ComponentType.PLAYER_OWNED)
				owner = o and o.player
			end)
			out[#out + 1] = tostring(entity) .. " (owned by " .. (owner ~= nil and tostring(owner) or "no one") .. ")"
		end
	end
	if n > 12 then out[#out + 1] = "and " .. (n - 12) .. " more" end
	return n, table.concat(out, ", ")
end

-- What the probe says of one line a viewer is handed: its owner, each
-- stop's station group, station and terminal as the line names them and
-- whether they exist (the group's stations, the station's terminals, their
-- owners), whether the line system lists the line at that terminal, and
-- what the engine says is wrong with the line (lineSystem.getProblemLines,
-- util.line.getLineProblems, util.line.getDetailedLineProblems). For a
-- company's line and the first company's alike, so the two compare.
-- A field of the game's userdata or table, or nil: an index the userdata
-- does not have raises, which is no answer.
local function field(value, key)
	if value == nil then return nil end
	local ok, v = pcall(function() return value[key] end)
	return ok and v or nil
end

-- "colour r g b" of an entity's COLOR component, or why there is none.
function follow.colourText(api, entity)
	local okType, kind = pcall(function() return api.type.ComponentType.COLOR end)
	if not okType or kind == nil then return "no COLOR component type" end
	local ok, c = pcall(function() return api.engine.getComponent(entity, kind) end)
	if not ok then return "colour unreadable: " .. tostring(c) end
	if c == nil then return "no colour" end
	local v = field(c, "color") or c
	local x, y, z = field(v, "x") or field(v, 1), field(v, "y") or field(v, 2), field(v, "z") or field(v, 3)
	if type(x) ~= "number" or type(y) ~= "number" or type(z) ~= "number" then
		return "colour of no numbers (" .. type(x) .. ")"
	end
	return string.format("colour %.3f %.3f %.3f", x, y, z)
end

follow.COMPONENT_TYPES = { "AIRCRAFT", "ANIMAL", "ASSET_GROUP", "BASE_EDGE", "BASE_EDGE_STREET", "BASE_NODE",
	"BASE_PARALLEL_STRIP", "CUSTOM_STATE", "EMISSION_EMITTER", "FIELD", "GAME_TIME", "GAME_SPEED",
	"MODEL_INSTANCE_LIST", "NAME", "LINE", "LOG_BOOK", "MODEL_PERSON", "MOVE_PATH", "MOVE_PATH_AIRCRAFT", "STATION",
	"STATION_GROUP", "SIM_PERSON", "SIM_PERSON_AT_TERMINAL", "SIM_PERSON_AT_VEHICLE", "SIM_CARGO",
	"SIM_ENTITY_AT_BUILDING", "SIM_ENTITY_AT_VEHICLE", "SIM_ENTITY_AT_TERMINAL", "SIM_ENTITY_IDLE",
	"SIM_ENTITY_MOVING", "SIGNAL_LIST", "TOWN", "INDUSTRY", "STOCK_LIST", "TOWN_BUILDING", "TRANSPORT_VEHICLE", "TRAIN",
	"CARRIAGE", "CARRIAGE_LIST", "VEHICLE_DEPOT", "COLOR", "BOUNDING_VOLUME", "CONSTRUCTION", "SUBCONSTRUCTION",
	"PERSON_CAPACITY", "PLAYER_OWNED", "ACCOUNT", "GAME_SCRIPT", "WAREHOUSE", "TRANSPORT_NETWORK", "MAINTENANCE_COST",
	"RAILROAD_CROSSING", "PLAYER", "WORLD", "BRIDGE", "BASE_NODE_CONFIG", "LAND_VEHICLE", "BASE_NODE_TRAFFIC_LIGHT",
	"EDGE_OBJECT", "EMISSION_GRID", "TERRAIN", "SHIP" }

-- The names of the component types `entity` has, from
-- api.type.ComponentType, sorted.
function follow.componentsOf(api, entity)
	local names = {}
	local okT, types = pcall(function() return api.type.ComponentType end)
	if not okT or types == nil then return names end
	-- The game's enum does not iterate (build 40408): its names, as
	-- api/tealdef/api/engine.d.tl lists them.
	for _, name in ipairs(follow.COMPONENT_TYPES) do
		local okK, kind = pcall(function() return types[name] end)
		if okK and kind ~= nil then
			local ok, c = pcall(function() return api.engine.getComponent(entity, kind) end)
			if ok and c ~= nil then names[#names + 1] = name end
		end
	end
	table.sort(names)
	if #names == 0 then names[1] = "(no component types it could list)" end
	return names
end

function follow.lineReport(api, line)
	local CT = api.type.ComponentType
	local parts = {}
	local function owner(e)
		local ok, o = pcall(function() return api.engine.getComponent(e, CT.PLAYER_OWNED) end)
		return ok and o and o.player or nil
	end
	local lineOwner = owner(line)
	parts[#parts + 1] = "line " .. tostring(line) .. " owned by " .. tostring(lineOwner)
	-- Its colour (the COLOR component the game paints it with) and how
	-- many vehicles run it.
	parts[#parts + 1] = follow.colourText(api, line)
	pcall(function()
		local vehicles = api.engine.system.transportVehicleSystem.getLineVehicles(line) or {}
		parts[#parts + 1] = #vehicles .. " vehicle(s)"
	end)
	local okL, comp = pcall(function() return api.engine.getComponent(line, CT.LINE) end)
	if not okL or comp == nil then
		parts[#parts + 1] = "no LINE component"
		return table.concat(parts, "; ")
	end
	local stops = {}
	pcall(function() for k = 1, #comp.stops do stops[k] = comp.stops[k] end end)
	parts[#parts + 1] = #stops .. " stop(s)"
	for k, stop in ipairs(stops) do
		local text = {}
		pcall(function()
			local group, station, terminal = stop.stationGroup, stop.station, stop.terminal
			text[#text + 1] = string.format("stop %d: group %s station %s terminal %s", k, tostring(group),
				tostring(station), tostring(terminal))
			local g = api.engine.getComponent(group, CT.STATION_GROUP)
			if g == nil then
				text[#text + 1] = "no STATION_GROUP"
				return
			end
			local stations = g.stations or {}
			text[#text + 1] = "group of " .. #stations .. " station(s) owned by " .. tostring(owner(group))
			local entity = stations[(station or -1) + 1]
			if entity == nil then
				text[#text + 1] = "no station " .. tostring(station) .. " in the group"
				return
			end
			local s = api.engine.getComponent(entity, CT.STATION)
			local terminals = s and s.terminals and #s.terminals or 0
			text[#text + 1] = string.format("station %s owned by %s with %d terminal(s)", tostring(entity),
				tostring(owner(entity)), terminals)
			if (terminal or -1) < 0 or terminal >= terminals then
				text[#text + 1] = "no terminal " .. tostring(terminal)
			end
			local listed = false
			for _, ls in ipairs(api.engine.system.lineSystem.getLineStopsForTerminal(entity, terminal) or {}) do
				if ls[1] == line then listed = true end
			end
			text[#text + 1] = listed and "listed at the terminal" or "not listed at the terminal"
		end)
		parts[#parts + 1] = table.concat(text, ", ")
	end
	pcall(function()
		for _, p in ipairs(api.engine.system.lineSystem.getProblemLines(lineOwner) or {}) do
			if p[1] == line then parts[#parts + 1] = "line system problem " .. tostring(p[2]) end
		end
	end)
	pcall(function()
		for _, p in ipairs(api.engine.util.line.getLineProblems() or {}) do
			if p[1] == line then parts[#parts + 1] = "path problem " .. tostring(p[2]) end
		end
	end)
	pcall(function()
		local states = api.engine.util.line.getDetailedLineProblems(line) or {}
		local n = 0
		for _, stopStates in ipairs(states) do
			for _ in ipairs(stopStates or {}) do n = n + 1 end
		end
		parts[#parts + 1] = n .. " detailed stop problem(s)"
	end)
	return table.concat(parts, "; ")
end

function follow.watchLines(api, require_, link, where)
	if not (link and link.note) then return 0 end
	local saidCount, saidText = 0, {}
	local checkedAt, on = nil, false
	local function probing()
		local ok, now = pcall(os.clock)
		now = ok and now or 0
		if checkedAt == nil or now - checkedAt >= 2 then
			checkedAt = now
			local okNote, v = pcall(function() return link:note(follow.PROBE_NOTE) end)
			on = okNote and v == "1"
		end
		return on
	end
	local function say(text)
		if saidCount >= follow.LINES_SAID or saidText[text] then return end
		saidText[text] = true
		saidCount = saidCount + 1
		pcall(function() link:log(text .. " (" .. tostring(where) .. ")") end)
	end
	local wrapped = 0
	local okB, builtin = pcall(require_, follow.BUILTIN)
	if okB and type(builtin) == "table" and type(builtin.LineViewer) == "function" and not follow.wrappedTests[builtin.LineViewer] then
		local original = builtin.LineViewer
		local viewer = function(params, ...)
			if probing() then
				pcall(function()
					local n, text = follow.linesText(api, params and params.showLines)
					say("probe: a line viewer is handed " .. n .. " line(s) to draw: " .. text)
					-- Each line's stops and the engine's verdict on it.
					for _, l in ipairs(params and params.showLines or {}) do
						local okE, entity = pcall(function() return type(l) == "number" and l or l.entity end)
						local okT, transparency = pcall(function() return type(l) ~= "number" and l.transparency or nil end)
						if okE and type(entity) == "number" then
							pcall(function()
								local o = api.engine.getComponent(entity, api.type.ComponentType.PLAYER_OWNED)
								local who = o and o.player
								if type(who) == "number" then
									say("probe: the line's owner " .. tostring(who) .. " has "
										.. table.concat(follow.componentsOf(api, who), " ")
										.. "; " .. follow.colourText(api, who))
								end
							end)
							say("probe: line to draw: " .. follow.lineReport(api, entity)
								.. (okT and transparency ~= nil and ("; handed at transparency " .. tostring(transparency)) or ""))
						end
					end
				end)
			end
			return original(params, ...)
		end
		follow.wrappedTests[viewer] = true
		builtin.LineViewer = viewer
		wrapped = wrapped + 1
	end
	local okS, system = pcall(function() return api.engine.system.lineSystem end)
	local get = okS and system and system.getLinesForPlayer
	if get ~= nil and not follow.wrappedTests[get] then
		local lister = function(player, ...)
			local lines = get(player, ...)
			if probing() then
				pcall(function()
					local n, text = follow.linesText(api, lines)
					say("probe: getLinesForPlayer(" .. tostring(player) .. ") answers " .. n .. " line(s): " .. text)
				end)
			end
			return lines
		end
		follow.wrappedTests[lister] = true
		local took = pcall(function() system.getLinesForPlayer = lister end)
		if took then wrapped = wrapped + 1 end
	end
	return wrapped
end

function follow.companyOf(roster, me)
	if type(roster) ~= "table" or type(roster.members) ~= "table" or type(me) ~= "string" then return nil end
	for _, m in ipairs(roster.members) do
		if m.player == me then
			for _, c in ipairs(roster.list or {}) do
				if c.id == m.company and not c.gone and c.id ~= 0 then return c.entity end
			end
		end
	end
	return nil
end

return follow
