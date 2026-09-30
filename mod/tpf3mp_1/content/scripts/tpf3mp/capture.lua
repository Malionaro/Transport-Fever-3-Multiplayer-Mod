-- tpf3mp/capture.lua -- a build the player made with the game's own tools,
-- as the action the room orders instead (docs/HOOKS.md, "The build tools").
--
-- The tools show every proposal they make to game scripts
-- (builder.proposalCreate), with the proposal as the game will build it;
-- the mod's game script keeps the action each makes, and hands the room the
-- one the player clicked. Everything is read from the proposal as it is, in
-- the game's units: metres, and plain fractions for the matrix.
--
-- A proposal the room cannot carry yet is not guessed at: the capture says
-- why, and the tool shows it.
--
-- Pure Lua; the tests hand it proposals of the game's shape.

local capture = {}

local function get(value, key)
	local ok, v = pcall(function() return value[key] end)
	if ok then return v end
	return nil
end

local function length(list)
	if list == nil then return 0 end
	local ok, n = pcall(function() return #list end)
	if ok and type(n) == "number" then return n end
	return nil
end

local function sortedKeys(tbl)
	local keys = {}
	for key in pairs(tbl) do keys[#keys + 1] = key end
	table.sort(keys, function(a, b)
		if type(a) == type(b) then return a < b end
		return type(a) == "number"
	end)
	return keys
end

-- A construction's parameters as the schema's flat list (tpf3mp_proto
-- action::Param): nested tables become paths, "modules[3801].name"; a number
-- with no fraction is Int, any other Fixed; a boolean Bool, a string Text.
-- Returns the list, or nil and why.
function capture.params(tbl)
	local out = {}
	local function walk(node, path, depth)
		if depth > 8 then error("parameters nested deeper than 8") end
		for _, key in ipairs(sortedKeys(node)) do
			local value = node[key]
			local here
			if type(key) == "number" and key == math.floor(key) then
				here = path .. "[" .. string.format("%d", key) .. "]"
			elseif type(key) == "string" and key:match("^[%a_][%w_]*$") then
				here = path == "" and key or (path .. "." .. key)
			else
				error("a parameter named " .. tostring(key))
			end
			local kind = type(value)
			if kind == "table" then
				walk(value, here, depth + 1)
			elseif kind == "number" then
				if value == math.floor(value) then
					out[#out + 1] = { key = here, value = { Int = value } }
				else
					out[#out + 1] = { key = here, value = { Fixed = value } }
				end
			elseif kind == "boolean" then
				out[#out + 1] = { key = here, value = { Bool = value } }
			elseif kind == "string" then
				out[#out + 1] = { key = here, value = { Text = value } }
			else
				error("parameter " .. here .. " is a " .. kind)
			end
		end
	end
	local ok, why = pcall(walk, tbl, "", 0)
	if not ok then return nil, tostring(why) end
	return out
end

-- The schema's transform (action::Transform) of the game's 4x4 matrix: its
-- basis is elements 1-3, 5-7 and 9-11, its origin 13-15.
function capture.transform(m)
	local function at(i)
		local v = get(m, i)
		if type(v) ~= "number" then error("the matrix has no element " .. i) end
		return v
	end
	return {
		basis = { at(1), at(2), at(3), at(5), at(6), at(7), at(9), at(10), at(11) },
		origin = { x = at(13), y = at(14), z = at(15) },
	}
end

local function module(name)
	local loaded = package and package.loaded and package.loaded["tpf3mp." .. name]
	if loaded then return loaded end
	if ug_require then return ug_require("tpf3mp_1::/scripts/tpf3mp/" .. name .. ".lua") end
	return require("tpf3mp." .. name)
end

-- One construction placed with the construction tool: stations, depots and
-- the rest (tpf3mp_proto action::ConstructionBuild). Returns the action
-- table, or nil and why the room cannot carry it yet.
--
-- A proposal that replaces one construction of the player's with a new one
-- (an edit of its modules or parameters, an upgrade) is carried with
-- `replaces`, the old one by its file and place; every game removes it and
-- builds the new one in one proposal (tpf3mp/apply.lua).
--
-- The construction's own streets its script makes again wherever it is
-- built. The proposal's street part is what the tool built around it, and
-- travels with it (capture.connection): built without it, a station by a
-- road stood beside the road, its entrance a dead end, and no line could
-- reach it (seen on build 40408).
function capture.construction(proposal)
	local street = get(proposal, "proposal")
	for _, list in ipairs({ "addedNodes", "addedSegments", "removedNodes", "removedSegments",
		"edgeObjectsToAdd" }) do
		if length(street and get(street, list)) == nil then return nil, "a proposal it cannot read" end
	end
	-- Constructions in the way: town buildings the placement clears, which
	-- the replay clears again (gatherBuildings), and at most one other
	-- construction the new one replaces: a module edit or an upgrade,
	-- named by its file and where it stands (capture.replaced).
	local toRemove = get(proposal, "toRemove")
	local removed = length(toRemove)
	if removed == nil then return nil, "a proposal it cannot read" end
	local replaced, replaces
	for i = 1, removed do
		local entity = get(toRemove, i)
		local c = api.engine.getComponent(entity, api.type.ComponentType.CONSTRUCTION)
		if (length(c and get(c, "townBuildings")) or 0) == 0 then
			if replaced ~= nil then return nil, "a construction that replaces more than one" end
			local why
			replaces, why = capture.replaced(c)
			if not replaces then return nil, why end
			replaced = { entity = entity, component = c }
		end
	end
	local toAdd = get(proposal, "toAdd")
	if length(toAdd) ~= 1 then return nil, "more than one construction at once" end
	local con = get(toAdd, 1)
	local file = get(con, "fileName")
	if type(file) ~= "string" or file == "" then return nil, "a construction of no file" end
	local name = get(con, "name")
	if replaced and (type(name) ~= "string" or name == "") then
		-- An edit keeps the construction's name, as the game's own
		-- upgrade does (mission_framework_util_entity.tl, upgradeConstruction).
		local ok, old = pcall(function() return api.engine.util.getEntityName(replaced.entity) end)
		if ok then name = old end
	end
	if type(name) ~= "string" or name == "" then return nil, "an unnamed construction" end
	local params = get(con, "params")
	if type(params) ~= "table" then
		local construction = get(con, "construction")
		params = construction and get(construction, "params")
	end
	if type(params) ~= "table" then return nil, "a construction without its parameters" end
	local list, why = capture.params(params)
	if not list then return nil, why end
	local ok, transform = pcall(capture.transform, get(con, "transf"))
	if not ok then return nil, tostring(transform) end
	if replaced then
		-- The street part of an edit is the construction's own: every game
		-- makes the new one's again as it builds it. One that removes a
		-- street or track not its own changes the streets around it, which
		-- an edit does not carry.
		local own, why = capture.ownStreets(street, replaced.component)
		if not own then return nil, why end
		return { BuildConstruction = { file = file, transform = transform, params = list, name = name,
			replaces = replaces } }
	end
	local connection, whyNot = capture.connection(proposal)
	if connection == nil then return nil, whyNot end
	return { BuildConstruction = { file = file, transform = transform, params = list, name = name,
		connection = connection or nil } }
end

-- The construction an edit replaces, as actions name one (tpf3mp_proto
-- action::ConstructionRef): its file and where it stands. Every game finds
-- it there (tpf3mp/apply.lua, constructionAt), and finds the new one there
-- again for the next edit: an edit keeps the file and the place, and entity
-- ids are no name (docs/BUILDING.md, "Module edits and upgrades"). Returns
-- the reference, or nil and why the room cannot name it.
function capture.replaced(component)
	if component == nil then return nil, "removing something that is no construction" end
	local file = get(component, "fileName")
	local t = get(component, "transf")
	local x, y, z = get(t, 13), get(t, 14), get(t, 15)
	if type(file) ~= "string" or file == "" or type(x) ~= "number" or type(y) ~= "number"
		or type(z) ~= "number" then
		return nil, "a construction the room cannot name"
	end
	return { file = file, at = { x = x, y = y, z = z } }
end

-- Whether an edit's street part removes only the old construction's own
-- nodes and edges (its CONSTRUCTION component's frozenNodes and
-- frozenEdges). true, or nil and why not.
function capture.ownStreets(street, component)
	local own = {}
	for _, key in ipairs({ "frozenNodes", "frozenEdges" }) do
		local list = get(component, key)
		for i = 1, (length(list) or 0) do own[get(list, i)] = true end
	end
	for _, key in ipairs({ "removedSegments", "removedNodes" }) do
		local list = get(street, key)
		for i = 1, (length(list) or 0) do
			if not own[get(get(list, i), "entity")] then
				return nil, "a construction edit that changes the streets around it"
			end
		end
	end
	return true
end

-- Keeps of a construction's network part only the edges joined, through each
-- other, to a node that exists or to what the build removes, and the new
-- nodes they use. The construction's own tracks and streets come in its
-- proposal too, as new edges between new nodes that reach nothing existing
-- (build 40408: a rail station on open ground proposes its platform track,
-- 24 edges through 25 new nodes); the construction builds those itself, and
-- built beside it they block it ("Construction Not Possible").
local function joinedOnly(part)
	local parent = {}
	local function find(x)
		while parent[x] ~= x do x = parent[x] end
		return x
	end
	for _, e in ipairs(part.edges) do
		for _, n in ipairs({ e.node0, e.node1 }) do
			if parent[n] == nil then parent[n] = n end
		end
		local a, b = find(e.node0), find(e.node1)
		if a ~= b then parent[a] = b end
	end
	local joined = {}
	for n in pairs(parent) do
		if type(n) == "number" and n >= 0 then joined[find(n)] = true end
	end
	for _, r in ipairs(part.removed) do
		for _, n in ipairs({ r.node0, r.node1 }) do
			if parent[n] ~= nil then joined[find(n)] = true end
		end
	end
	local edges, used = {}, {}
	for _, e in ipairs(part.edges) do
		if joined[find(e.node0)] then
			edges[#edges + 1] = e
			used[e.node0], used[e.node1] = true, true
		end
	end
	local nodes = {}
	for _, n in ipairs(part.nodes) do
		if used[n.id] then nodes[#nodes + 1] = n end
	end
	part.edges, part.nodes = edges, nodes
end

-- The street and track changes a construction tool's proposal makes with its
-- construction (seen on build 40408: a bus station placed by a road rebuilds
-- the road through a new junction and adds an edge from the junction to the
-- station's own street node), as a polyline whose every link names its
-- kind; false when it makes none; nil and why the room cannot carry them.
function capture.connection(proposal)
	local engine = module("engine")
	local ok, part = pcall(engine.fromProposal, proposal, nil, true)
	if not ok then return nil, tostring(part) end
	if part == nil then return false end
	local removes = #part.removed > 0 or #part.removedNodes > 0
	joinedOnly(part)
	if #part.edges == 0 then
		if removes then return nil, "a construction that removes streets and builds none" end
		return false
	end
	part.explicit = true
	local first = part.edges[1]
	part.network = first.network
	if part.network == "Street" then part.street = first.template else part.track = first.template end
	part.style = first.style
	local action, why = module("roads").capture(part, engine.world())
	if not action then return nil, why end
	local build = action.BuildRoad or action.BuildTrack
	return build.polyline
end

-- A street or track tool's build (tpf3mp_proto action::RoadBuild,
-- TrackBuild), read off its proposal by tpf3mp/engine.lua and made an action
-- by tpf3mp/roads.lua. Returns the action table; false for a proposal of
-- nothing (the tool before its first point); or nil and why.
function capture.street(proposal)
	return module("engine").captureBuild(proposal, "Street")
end

function capture.track(proposal)
	return module("engine").captureBuild(proposal, "Track")
end

-- A stop placed on a street or track with the stop tool (tpf3mp_proto
-- action::PlaceStop), read off its proposal by tpf3mp/engine.lua. Returns
-- the action table; false for a proposal of nothing; or nil and why.
--
-- Transport Fever 3's proposal does not name the stop (build 40408: its
-- edge objects carry no model), which is a construction the construction
-- menu gave the tool; the GUI notes it (capture.STOP_NOTE,
-- gui/tpf3mp/gui_state.script.lua) and `link` reads the note.
capture.STOP_NOTE = "stop-tool"

-- In a GUI Lua state: notes the stop the construction menu gives the stop
-- tool, for capture.stop, which runs in another. The menu makes the tool's
-- action with construction_react_util.getActionParams (`util`), whose
-- EdgeObjectBuilder names the stop's construction (resName; build 40408,
-- gui/construction/construction_react_util.tl); each call is noted through
-- `link` (tpf3mp/bridge.lua). Once a state. Returns whether it watches.
function capture.watchStopTool(util, link)
	if type(package) == "table" and type(package.loaded) == "table" then
		if package.loaded["tpf3mp.stopToolWatched"] then return true end
	end
	if type(util) ~= "table" or type(util.getActionParams) ~= "function" or link == nil then return false end
	local original = util.getActionParams
	util.getActionParams = function(...)
		local result = original(...)
		pcall(function()
			local builder = result.constructionActionParams.edgeObjectBuilder
			local name = builder and builder.resName
			if type(name) == "string" and name ~= "" then link:note(capture.STOP_NOTE, name) end
		end)
		return result
	end
	if type(package) == "table" and type(package.loaded) == "table" then
		package.loaded["tpf3mp.stopToolWatched"] = true
	end
	return true
end

function capture.stop(proposal, link)
	local noted = link and link.note and link:note(capture.STOP_NOTE) or nil
	return module("engine").placeStop(proposal, noted)
end

-- The bulldozer's removal (tpf3mp_proto action::Bulldoze), read off its
-- proposal by tpf3mp/engine.lua: a construction, edges, or a stop. Returns the action table; false for a
-- proposal of nothing; or nil and why.
--
-- A proposal that removes a construction and adds one is an edit: a module
-- taken off with the module bulldozer, if that reaches game scripts as the
-- bulldozer's (INFERRED, not seen in the game), is carried as the edit it is
-- (capture.construction), or refused.
function capture.bulldoze(proposal)
	local toRemove = get(proposal, "toRemove")
	if (length(get(proposal, "toAdd")) or 0) > 0 and (length(toRemove) or 0) > 0 then
		for i = 1, length(toRemove) do
			local c = api.engine.getComponent(get(toRemove, i), api.type.ComponentType.CONSTRUCTION)
			if (length(c and get(c, "townBuildings")) or 0) == 0 then return capture.construction(proposal) end
		end
		return nil, "a bulldozer proposal that builds"
	end
	return module("engine").bulldoze(proposal)
end

-- A build a window sends itself (api.cmd.makeWorldBuildProposalCmd, as
-- tpf3mp/guard.lua's CARRY takes it): the room carries an edit of one
-- construction, as the construction menu's parameters and the station's
-- cargo buttons make one (api.engine.util.proposal
-- .createProposalReplaceConstruction, gui/construction/construction.tl and
-- gui/entity_window/entity_window_util.tl, build 40408). Every other build
-- from a window stays refused. Returns the action table, or raises why not.
function capture.windowBuild(_ctx, proposal)
	local action, why = capture.construction(proposal)
	if not action then error(why, 0) end
	if action.BuildConstruction.replaces == nil then error("building from this window", 0) end
	return action
end

-- A proposal's street part in one line, for the log (tpf3mp/engine.lua);
-- "" when it has none.
function capture.describe(proposal)
	return module("engine").describe(proposal)
end

-- ------------------------------------------------------ vehicles and lines
--
-- The vehicle and line windows' commands (api.cmd.make*Cmd, with the
-- arguments the windows give them) as actions: tpf3mp/guard.lua's CARRY.
-- `ctx` names what a command names by entity:
--
--   ctx.vehicle(e), ctx.line(e), ctx.group(e) -> canonical id, or nil
--                                               (tpf3mp/registry.lua)
--   ctx.depot(e) -> { file =, at = { x, y, z } } of the depot's
--                   construction, or nil
--   ctx.model(id) -> a vehicle model's file name, or nil
--   ctx.parts(e) -> a vehicle's parts, front to back, each
--                   { model = modelId, purchased = purchaseTime }, or nil
--   ctx.town(e)   -> a town's canonical id, or nil
--   ctx.player()  -> the player's company entity, or nil
--
-- Each returns the action table, or raises why the room cannot carry it.

local function named(what, id)
	if id == nil then error(what, 0) end
	return id
end

local function tintOf(v)
	local r, g, b = get(v, "x"), get(v, "y"), get(v, "z")
	if r == nil then r, g, b = get(v, 1), get(v, 2), get(v, 3) end
	if type(r) ~= "number" or type(g) ~= "number" or type(b) ~= "number" then
		error("a colour it cannot read", 0)
	end
	return { r = r, g = g, b = b }
end

local function each(list, fn)
	local out = {}
	for i = 1, (length(list) or 0) do out[i] = fn(get(list, i)) end
	return out
end

local function vehicleOf(ctx, entity)
	return named("a vehicle the room cannot name", ctx.vehicle(entity))
end

local function lineOf(ctx, entity)
	return named("a line the room cannot name", ctx.line(entity))
end

-- One TransportVehiclePart of a vehicle config as the schema's ConsistPart.
-- Each part's reversed flag rides along: a turned wagon (an ICE's tail head,
-- a cab car) stays turned (TPF2-MP learned it the hard way, release
-- 0.6.1.12, from tearded's fork).
local function consistPart(ctx, tvp)
	local part = get(tvp, "part")
	return {
		model = named("a vehicle model the room cannot name", ctx.model(get(part, "modelId"))),
		reversed = get(part, "reversed") == true,
		loads = each(get(part, "compartment2loadConfig"), function(lc)
			return { config = get(lc, "loadConfigIndex"), cargo = get(lc, "cargoTypeId") }
		end),
		color = tintOf(get(part, "color")),
	}
end

-- The depot's store: a vehicle config (TransportVehicleConfig) bought there.
function capture.vehicleBuy(ctx, _player, depot, config)
	return { BuyVehicle = {
		depot = named("a depot the room cannot name", ctx.depot(depot)),
		consist = each(get(config, "vehicles"), function(tvp) return consistPart(ctx, tvp) end),
		groups = each(get(config, "vehicleGroups"), function(n) return n end),
		multiple_units = each(get(config, "muFileNames"), function(name) return name end),
	} }
end

-- The vehicle window's "modify" and the store's "replace" (build 40408,
-- gui/line_vehicle_mgmt/vehicle_react_util.tl HandleVehicleChanges): one
-- makeVehicleReplaceCmd per vehicle, a group's vehicles one by one, with the
-- config the store built. A part the player left in the consist is the
-- vehicle's own, its purchase time kept; the store bought the rest, with
-- purchase time 0, which HandleVehicleChanges sets to the GUI's game time
-- before it sends. So a part is kept when it is one of the vehicle's own
-- parts, of the same model and purchase time, each own part matched once,
-- front to back. `ctx.parts(e)` lists the vehicle's parts now, each
-- { model = modelId, purchased = purchaseTime }.
function capture.vehicleReplace(ctx, vehicle, config)
	local id = vehicleOf(ctx, vehicle)
	local own = ctx.parts and ctx.parts(vehicle)
	if type(own) ~= "table" then error("a vehicle whose parts the room cannot read", 0) end
	local taken = {}
	local consist = each(get(config, "vehicles"), function(tvp)
		local out = { part = consistPart(ctx, tvp) }
		local model, purchased = get(get(tvp, "part"), "modelId"), get(tvp, "purchaseTime")
		if type(purchased) == "number" and purchased > 0 then
			for i, p in ipairs(own) do
				if not taken[i] and p.model == model and p.purchased == purchased then
					taken[i], out.kept = true, i - 1
					break
				end
			end
		end
		return out
	end)
	if #consist == 0 then error("a replacement of no vehicles", 0) end
	return { ReplaceVehicle = {
		vehicle = id,
		consist = consist,
		groups = each(get(config, "vehicleGroups"), function(n) return n end),
		multiple_units = each(get(config, "muFileNames"), function(name) return name end),
	} }
end

-- `stopIndex` -1 is the line manager's "Next Reachable Stop" (build 40408):
-- the game picks the stop, which the action carries as no first stop.
function capture.vehicleSetLine(ctx, vehicle, line, stopIndex)
	local first = nil
	if stopIndex ~= -1 then first = stopIndex end
	return { AssignLine = {
		vehicles = { vehicleOf(ctx, vehicle) }, line = lineOf(ctx, line), first_stop = first,
	} }
end

function capture.vehicleSell(ctx, vehicles)
	return { SellVehicle = { vehicles = each(vehicles, function(e) return vehicleOf(ctx, e) end) } }
end

function capture.vehicleStop(ctx, vehicle, stopped)
	return { VehicleOp = { vehicle = vehicleOf(ctx, vehicle), change = { Stop = stopped == true } } }
end

function capture.vehicleToDepot(ctx, vehicle, sell, jumpTo)
	if jumpTo ~= nil then error("moving a vehicle into a depot at once", 0) end
	return { VehicleOp = { vehicle = vehicleOf(ctx, vehicle), change = { ToDepot = { sell = sell == true } } } }
end

function capture.vehicleReverse(ctx, vehicle)
	return { VehicleOp = { vehicle = vehicleOf(ctx, vehicle), change = "Reverse" } }
end

function capture.vehicleDepart(ctx, vehicle)
	return { VehicleOp = { vehicle = vehicleOf(ctx, vehicle), change = "Depart" } }
end

-- The game's load modes (Line.LoadMode), numbers to the schema's names.
local LOAD_MODES = { [0] = "LoadIfAvailable", [1] = "FullLoadAny", [2] = "FullLoadAll", [3] = "LegacyUnloadOnly" }

-- A Line component as the schema's LineData.
function capture.lineData(ctx, line)
	local stops = each(get(line, "stops"), function(s)
		if (length(get(s, "waypoints")) or 0) > 0 then error("a line through waypoints", 0) end
		local config = get(s, "stopConfig")
		local mode = tonumber(get(s, "loadMode"))
		return {
			group = named("a station the room cannot name", ctx.group(get(s, "stationGroup"))),
			terminal = { station = get(s, "station"), terminal = get(s, "terminal") },
			alternatives = each(get(s, "alternativeTerminals"), function(a)
				return { station = get(a, "station"), terminal = get(a, "terminal") }
			end),
			load_mode = LOAD_MODES[mode] or error("a load mode " .. tostring(mode), 0),
			min_wait = get(s, "minWaitingTime"),
			max_wait = get(s, "maxWaitingTime"),
			max_extra_wait = get(s, "maxAdditionalWaitingTime"),
			rules = {
				load = each(get(config, "load"), function(b) return b == true end),
				max_load = each(get(config, "maxLoad"), function(f) return f end),
				force_unload = get(config, "forceUnload") == true,
				destroy_for_config_change = get(config, "destroyForConfigChange") == true,
				destroy_for_refresh = get(config, "destroyForRefresh") == true,
			},
		}
	end)
	local info = get(line, "vehicleInfo")
	local modes, transport = {}, info and get(info, "transportModes")
	if type(transport) ~= "table" then error("a line's transport modes it cannot read", 0) end
	for mode, on in pairs(transport) do
		if on == true then modes[#modes + 1] = mode end
	end
	table.sort(modes)
	return {
		stops = stops,
		modes = modes,
		custom_filters = get(line, "customFilters") == true,
		reservation_priority = get(line, "reservationPriority") or 0,
	}
end

function capture.lineCreate(ctx, name, color, _player, line)
	return { CreateLine = { name = name, color = tintOf(color), line = capture.lineData(ctx, line) } }
end

function capture.lineUpdate(ctx, lineEntity, line)
	return { EditLine = { line = lineOf(ctx, lineEntity), change = { Update = capture.lineData(ctx, line) } } }
end

function capture.lineDestroy(ctx, lineEntity)
	return { EditLine = { line = lineOf(ctx, lineEntity), change = "Delete" } }
end

-- ------------------------------------------------------------ prospecting
--
-- The construction menu's prospection (gui/construction/
-- construction_react_util.tl, ProspectionActionRecipe.onSelect): the event
-- `Companies` `spawnIndustry` to the company script, with the player's
-- company, the town picked, the industry types, the permit and the cargo
-- (investigation/TPF3_PROSPECTING_2026-09-30.md). The types keep the order
-- the menu listed them in, which the game's shuffle depends on.
function capture.prospect(ctx, param)
	if type(param) ~= "table" then error("a prospection it cannot read", 0) end
	local player = ctx.player and ctx.player()
	if player == nil or get(param, "companyEntity") ~= player then
		error("prospecting for another company", 0)
	end
	local cargo = get(param, "cargoType")
	if type(cargo) ~= "string" or cargo == "" then error("a prospection for no cargo", 0) end
	local permit = get(param, "permitKey")
	if permit ~= nil and type(permit) ~= "string" then error("a permit it cannot read", 0) end
	local types = get(param, "types")
	local n = length(types)
	if n == nil or n == 0 then error("a prospection that can find no industry", 0) end
	local industries = {}
	for i = 1, n do
		local t = get(types, i)
		if type(t) ~= "string" or t == "" then error("an industry type it cannot read", 0) end
		industries[i] = t
	end
	local town = ctx.town and ctx.town(get(param, "townEntity")) or nil
	return { Prospect = {
		town = named("a town the room cannot name", town),
		cargo = cargo,
		industries = industries,
		permit = permit,
	} }
end

-- Renaming and recolouring: lines so far.
function capture.setName(ctx, entity, name)
	return { EditLine = { line = named("renaming this", ctx.line(entity)), change = { Rename = name } } }
end

function capture.setColor(ctx, entity, color)
	return { EditLine = { line = named("recolouring this", ctx.line(entity)), change = { Recolor = tintOf(color) } } }
end

return capture
