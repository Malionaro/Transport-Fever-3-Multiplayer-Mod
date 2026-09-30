-- tpf3mp/apply.lua -- runs an action the room ordered, in the mod's game
-- script's postUpdate (docs/HOOKS.md, "Actions in the game").
--
-- A game script runs in an engine state, where a command runs at once (the
-- game's own api/tealdef/api/cmd.d.tl). The game takes no callback in
-- update ("Callbacks are currently disallowed", build 40408), but in
-- postUpdate, where this runs, it calls one at once, with the command's
-- result: the game's mission scripts read the line they made from it right
-- after sendCommand (mission_vehicle_util.tl). A command the game refuses
-- raises, or its callback hears that it failed.
-- Every game applies the room's action in the same simulation update, so
-- what this makes of an action may depend on nothing but the action and the
-- world, which every game has alike: no time of day, no camera, no GUI.
--
-- An action is the table the hook hands over (tpf3mp_proto::lua): one
-- entry, the action's name and its body, in the game's units (metres,
-- plain fractions).
--
-- Pure Lua against the game's `api`; the tests give it a fake one.

local apply = {}

-- A matrix from a Transform: its basis is elements 1-3, 5-7 and 9-11 of the
-- game's matrix and its origin elements 13-15 (columns of four).
local function matrix(transform)
	local b, o = transform.basis, transform.origin
	local column = api.type.Vec4f.new
	return api.type.Mat4f.new(
		column(b[1], b[2], b[3], 0),
		column(b[4], b[5], b[6], 0),
		column(b[7], b[8], b[9], 0),
		column(o.x, o.y, o.z, 1)
	)
end

-- A parameter's value: exactly one of Int, Fixed, Bool and Text.
local function paramValue(value)
	if value.Int ~= nil then return value.Int end
	if value.Fixed ~= nil then return value.Fixed end
	if value.Bool ~= nil then return value.Bool end
	if value.Text ~= nil then return value.Text end
	error("a parameter value of no kind")
end

-- The keys a flattened parameter path names: "modules[3801].name" is
-- modules, 3801, name.
local function pathKeys(path)
	local keys = {}
	for part in string.gmatch(path, "[^%.]+") do
		local name, rest = string.match(part, "^([^%[]*)(.*)$")
		if name ~= "" then keys[#keys + 1] = name end
		for index in string.gmatch(rest, "%[(%-?%d+)%]") do
			keys[#keys + 1] = tonumber(index)
		end
	end
	if #keys == 0 then error("an empty parameter path") end
	return keys
end

-- The construction's parameters, from their flattened paths.
local function params(list)
	local out = {}
	for _, param in ipairs(list) do
		local keys = pathKeys(param.key)
		local node = out
		for i = 1, #keys - 1 do
			local key = keys[i]
			if type(node[key]) ~= "table" then node[key] = {} end
			node = node[key]
		end
		node[keys[#keys]] = paramValue(param.value)
	end
	return out
end

-- A line for the hook's log; the game script sets apply.log once linked.
local function log(line)
	if apply.log then pcall(apply.log, line) end
end

-- A list of the action, afresh, for the game to copy. The game copies a
-- list it is handed into its own vector in the order `next` walks it, and
-- the actions reach postUpdate as the game's own copy of what update
-- returned, whose lists `next` walks in hash order (build 40408: a bus
-- line's stops set to load grain, one cargo over from passengers). A table
-- filled 1, 2, 3... walks in order.
local function seq(list)
	local out = {}
	for i = 1, #list do out[i] = list[i] end
	return out
end

-- Whether this Lua state's game takes a command's callback here; the game
-- runs game scripts on a pool of states, so each learns on its own.
local callbacks = true

-- Sends `command`, which runs at once, and returns what the game answered,
-- its command data and result entities (nil, where it answers nothing
-- here). A command the game refuses raises, and apply.run reports it.
local function send(command)
	if callbacks then
		local heard, went, data, entities = false, nil, nil, nil
		local sent, err = pcall(api.cmd.sendCommand, command, function(d, success, e)
			heard, went, data, entities = true, success, d, e
		end)
		if sent then
			if heard and went ~= true then error("the game refused it", 0) end
			return data, entities
		end
		-- The game refuses a callback before it runs anything: sent again
		-- without one, the command runs once.
		if not tostring(err):find("allbacks are currently disallowed", 1, true) then error(err, 0) end
		callbacks = false
		log("the game takes no command callbacks in this state: what an action makes is found by the registry alone")
	end
	api.cmd.sendCommand(command)
	return nil, nil
end

local function run(command)
	send(command)
	return true
end

-- For the game script's own commands (tpf3mp/companies.lua's monthly loan
-- payments): the same send, which answers what the game made.
apply.send = send

local require_companies

-- The action running now: `ctx` as apply.run was given it. Its `company` is
-- the player entity of the acting player's company (tpf3mp/companies.lua);
-- without one, the save's own player, as before companies.
local acting = nil

local function company()
	return (acting and acting.company) or api.engine.util.getPlayer()
end

-- Refuses changing `entity` when another company owns it, naming the owner,
-- the same in every game (tpf3mp/companies.lua).
local function mine(entity, what)
	local companies = require_companies()
	local ok, why = companies.mayTouch(acting and acting.roster, company(), entity, api, what)
	if not ok then error(why, 0) end
end

-- The entity a command made: its data's field `field`, else the first of
-- its result entities; nil when the game did not say.
local function madeBy(field, data, entities)
	local ok, e = pcall(function() return data[field] end)
	if ok and type(e) == "number" and e >= 0 then return e end
	local first = type(entities) == "table" and entities[1]
	e = type(first) == "table" and first[1] or nil
	if type(e) == "number" and e >= 0 then return e end
	return nil
end

-- Builds `proposal` as the player's own build. The game's verdict first, as
-- its tools ask it: a build it would refuse (a collision, too steep, not
-- enough money) fails here with its reasons, the same in every game, and is
-- never sent; sent without a callback, a refused build would fail unseen.
local function buildProposal(proposal, context)
	local proposals = api.engine.util.proposal
	if proposals and proposals.makeProposalData then
		local data = proposals.makeProposalData(proposal, context)
		local state = data and data.errorState
		local messages = {}
		for _, m in ipairs(state and state.messages or {}) do messages[#messages + 1] = tostring(m) end
		if state and state.critical then
			error("the game refuses the build: " .. table.concat(messages, "; "), 0)
		end
		if #messages > 0 then log("the game warns of the build: " .. table.concat(messages, "; ")) end
	end
	-- What is not critical the tool builds through once the player clicks,
	-- town buildings in the way included: ignoreErrors, as the player's own
	-- build (with it false the game drops such a build unseen).
	return run(api.cmd.makeWorldBuildProposalCmd(proposal, context, true, true))
end

local HANDLERS = {}

-- Fills a SimpleProposal's street proposal from a polyline ("roads", below).
local networkInto
-- The construction of a file at a place ("vehicles and lines", below).
local constructionAt
-- Removes a stop from its edge ("stops", below).
local removeEdgeObject

-- An edit of a construction (its modules or parameters, an upgrade): the
-- construction the action names removed and the new one built in one
-- proposal, the old mapped to the new (old2new), as the game's own upgrade
-- makes one (mission_framework_util_entity.tl, upgradeConstruction), so
-- what stood on the old one (its stations, their station groups and the
-- lines that stop there) passes to the new. The game's verdict first, and
-- built as the player's own build, paid by the player (buildProposal). The
-- new one stands where the old one stood, so the next edit, a depot or a
-- line finds it by the same file and place.
local function replaceConstruction(build, proposal, entity)
	if build.connection ~= nil then error("an edit that builds streets around the construction", 0) end
	local old = constructionAt(build.replaces)
	mine(old, "construction")
	proposal.constructionsToAdd = { entity }
	proposal.constructionsToRemove = { old }
	proposal.old2new = { [old] = 0 }
	log("replacing " .. tostring(old) .. " " .. tostring(build.replaces.file) .. " with " .. tostring(build.file))
	local context = api.type.Context.new()
	context.player = company()
	context.gatherBuildings = true
	context.gatherFields = true
	buildProposal(proposal, context)
	-- What it made, where the action says: this game could name it.
	return true, constructionAt({ file = build.file, at = build.transform.origin })
end

function HANDLERS.BuildConstruction(build)
	local proposal = api.type.SimpleProposal.new()
	local entity = api.type.SimpleProposal.ConstructionEntity.new()
	entity.fileName = build.file
	entity.transf = matrix(build.transform)
	entity.params = params(build.params)
	entity.name = build.name
	entity.playerEntity = company()
	if build.replaces ~= nil then return replaceConstruction(build, proposal, entity) end
	proposal.constructionsToAdd = { entity }
	-- The streets the tool built around it, in the same proposal: the
	-- street it joins rebuilt through a junction. Not the construction's own
	-- entrance edge, which the tool snapped onto that junction: the
	-- construction makes its entrance again itself, unsnapped, ending a few
	-- metres short (build 40408).
	if build.connection ~= nil then networkInto(proposal, nil, nil, nil, build.connection, true) end
	-- Paid by the player, and clearing town buildings in its way, as the
	-- construction tool builds (the game's bridge and tunnel window names the
	-- player so, gui/entity_window/bridge_and_tunnel.tl); without a context
	-- the game builds for free. playerInitiated true: as the player's own
	-- build (buildProposal).
	local context = api.type.Context.new()
	context.player = company()
	context.gatherBuildings = true
	context.gatherFields = true
	local built = buildProposal(proposal, context)
	if build.connection == nil then return built end
	-- A scripted build does not snap; the game's refresh of a construction
	-- does, as its tool does: the entrance then ends at the street node
	-- beside it, the junction built above (refreshConstruction, build 40408:
	-- the same edge the tool proposed). So every game refreshes it at once,
	-- for free, as part of this action.
	local con = constructionAt({ file = build.file, at = build.transform.origin })
	local refresh = api.engine.util.proposal.refreshConstruction(con)
	local street, shape = refresh.proposal, {}
	for i = 1, #street.addedSegments do
		local s = street.addedSegments[i]
		shape[#shape + 1] = "+e" .. s.entity .. ":" .. tostring(s.comp.node0) .. ">" .. tostring(s.comp.node1)
	end
	for i = 1, #street.removedSegments do shape[#shape + 1] = "-e" .. tostring(street.removedSegments[i].entity) end
	log("snapping " .. tostring(con) .. " " .. table.concat(shape, " "))
	-- The game's verdict takes simple proposals only ("SimpleProposal
	-- expected, got Proposal", build 40408): a refresh the game refuses
	-- fails in the command's own answer instead (run).
	return run(api.cmd.makeWorldBuildProposalCmd(refresh, nil, true, false))
end

-- ---------------------------------------------------------------- roads
--
-- A road or track build (tpf3mp_proto action::Polyline) as a SimpleProposal's
-- street proposal, as the game's own scripted track builder makes one
-- (mission/tasks/auto_builder/track_builder.tl): new nodes and edges with
-- negative ids, existing nodes by their own. A vertex resolves as the
-- originator's tool resolved it: New; the existing node of its network
-- within 1.5 m horizontally, the nearest; or a split of the existing edge
-- between the nodes at its ends, cut in two at the vertex into halves that
-- keep the edge's own component, their tangents scaled to the part of the
-- curve each covers. The edges and nodes a build removes are found the same
-- way: an edge by the nodes at its ends, a node by its position. Each link
-- is the build's street or track, or the kind it names: a piece of the
-- street it joins, rebuilt through the new junction, keeps that street's.
--
-- Every game has the same world, so every game resolves alike; anything that
-- resolves to nothing fails the whole build, in every game.

local function module(name)
	local loaded = package and package.loaded and package.loaded["tpf3mp." .. name]
	if loaded then return loaded end
	if ug_require then return ug_require("tpf3mp_1::/scripts/tpf3mp/" .. name .. ".lua") end
	return require("tpf3mp." .. name)
end

local geom = module("geom")

-- How near an existing node a vertex resolving to it is, horizontally.
local NODE_TOLERANCE = 1.5
-- How near its node each end of a named edge is: the ends are the node's own
-- positions, rounded to the millimetre.
local END_TOLERANCE = 0.5

local function arr(v) return { v.x or v[1], v.y or v[2], v.z or v[3] } end
local function vec(p) return api.type.Vec3f.new(p[1], p[2], p[3]) end
local function scaled(p, k) return { p[1] * k, p[2] * k, p[3] * k } end

-- The game's enums, under api.type.enum ("enum" is a word in Teal, which
-- writes api.type["enum"]).
local function enum(name)
	local e = api.type.enum and api.type.enum[name]
	if e == nil then error("no api.type.enum." .. name) end
	return e
end

-- A resource's id by its name; the game answers -1 for none.
local function find(rep, name)
	local id = api.res[rep].find(name)
	if type(id) ~= "number" or id < 0 then error("no " .. rep .. " resource " .. tostring(name)) end
	return id
end

-- The nodes of a network: each node with an edge of it, and its position.
-- Held while it is read (on TPF2 a pairs() straight off the call let the
-- GC free the map mid-loop).
local function readNodes(network)
	local streets = api.engine.system.streetSystem
	local map
	if network == "Track" then map = streets.getNode2TrackEdgeMap() else map = streets.getNode2StreetEdgeMap() end
	local nodes = {}
	for node in pairs(map) do
		local c = api.engine.getComponent(node, api.type.ComponentType.BASE_NODE)
		if c and c.position then nodes[#nodes + 1] = { id = node, pos = arr(c.position) } end
	end
	return nodes
end

-- The node of `nodes` nearest `p` horizontally within `tol`, the lower id
-- on a tie; nil when none is.
local function nearest(nodes, p, tol)
	local best, bestD
	for _, n in ipairs(nodes) do
		local dx, dy = n.pos[1] - p[1], n.pos[2] - p[2]
		local d = dx * dx + dy * dy
		if d <= tol * tol and (bestD == nil or d < bestD or (d == bestD and n.id < best.id)) then
			best, bestD = n, d
		end
	end
	return best
end

-- The existing edge of `network` between the nodes at `a` and `b`: its id,
-- component and geometry (ends a and b, tangents ta and tb, as geom.lua takes
-- edges), oriented as the game has it. The lowest id if there are several.
local function edgeBetween(nodes, network, a, b)
	local na, nb = nearest(nodes, a, END_TOLERANCE), nearest(nodes, b, END_TOLERANCE)
	if na == nil or nb == nil or na.id == nb.id then return nil end
	local streets = api.engine.system.streetSystem
	local ids
	if network == "Track" then ids = streets.getNodeTrackSegments(na.id) else ids = streets.getNodeStreetSegments(na.id) end
	local found
	for i = 1, (ids and #ids or 0) do
		local id = ids[i]
		local c = api.engine.getComponent(id, api.type.ComponentType.BASE_EDGE)
		if c and ((c.node0 == na.id and c.node1 == nb.id) or (c.node0 == nb.id and c.node1 == na.id))
			and (found == nil or id < found.id) then
			local n0, n1 = na, nb
			if c.node0 == nb.id then n0, n1 = nb, na end
			found = { id = id, comp = c, node0 = n0.id, node1 = n1.id, a = n0.pos, b = n1.pos,
				ta = arr(c.tangent0), tb = arr(c.tangent1) }
		end
	end
	return found
end

local STRUCTURE = { Ground = "NORMAL", Bridge = "BRIDGE", Tunnel = "TUNNEL" }

-- Adds the polyline's nodes and edges, and its removals, to `proposal`'s
-- street proposal. `network`, `templateName` and `style` are the build's
-- own kind, for the links that name none; nil for a construction's
-- streets, whose every link names its kind. With `dangling` false, a new
-- vertex at the end of a single link, and that link, are left out: in a
-- construction's streets, the construction's own entrance.
function networkInto(proposal, network, templateName, style, polyline, dangling)
	local degree = {}
	for _, link in ipairs(polyline.links) do
		degree[link.from] = (degree[link.from] or 0) + 1
		degree[link.to] = (degree[link.to] or 0) + 1
	end
	local function loose(i) return polyline.vertices[i + 1].resolve == "New" and degree[i] == 1 end
	local links = {}
	for _, link in ipairs(polyline.links) do
		if not (dangling and (loose(link.from) or loose(link.to))) then links[#links + 1] = link end
	end
	local skipped = {}
	if dangling then
		for i = 0, #polyline.vertices - 1 do skipped[i + 1] = loose(i) end
	end
	polyline = { vertices = polyline.vertices, links = links, removals = polyline.removals,
		removed_nodes = polyline.removed_nodes }
	local nodesOf = {}
	local function nodes(n)
		if nodesOf[n] == nil then nodesOf[n] = readNodes(n) end
		return nodesOf[n]
	end
	local templates = {}
	local function template(name)
		if templates[name] == nil then
			templates[name] = api.res.streetTemplateRep.get(find("streetTemplateRep", name))
		end
		return templates[name]
	end
	local edgeType = enum("BaseEdgeType")

	-- Ids: the edges from -1, the links first and then two halves per split;
	-- the new nodes after them.
	local splits = 0
	for _, v in ipairs(polyline.vertices) do
		if type(v.resolve) == "table" and v.resolve.Split then splits = splits + 1 end
	end
	local nextEdge, nextNode = -1, -(#polyline.links + 2 * splits) - 1

	local nodesToAdd, edgesToAdd, edgesToRemove = {}, {}, {}
	-- The nodes at the ends of the edges removed, in order: their lane
	-- configurations name those edges, and go with them (below).
	local ends, endSeen = {}, {}
	local function removeEdge(e)
		edgesToRemove[#edgesToRemove + 1] = e.id
		for _, node in ipairs({ e.comp.node0, e.comp.node1 }) do
			if not endSeen[node] then
				endSeen[node] = true
				ends[#ends + 1] = node
			end
		end
	end
	local function addNode(p)
		local n = api.type.NodeAndEntity.new()
		n.entity = nextNode
		nextNode = nextNode - 1
		n.comp.position = vec(p)
		nodesToAdd[#nodesToAdd + 1] = n
		return n.entity
	end
	local function addEdge(kind, node0, node1, p0, p1, t0, t1, comp)
		local s = api.type.SegmentAndEntity.new()
		s.entity = nextEdge
		nextEdge = nextEdge - 1
		if comp ~= nil then s.comp = comp end
		s.type = kind
		s.comp.node0, s.comp.node1 = node0, node1
		s.comp.position0, s.comp.position1 = vec(p0), vec(p1)
		s.comp.tangent0, s.comp.tangent1 = vec(t0), vec(t1)
		edgesToAdd[#edgesToAdd + 1] = s
		return s
	end
	local function kindOf(n) if n == "Track" then return 1 end return 0 end

	-- The links' edges first, as their ids were counted.
	local links = {}
	local ids, at = {}, {}
	for i, v in ipairs(polyline.vertices) do at[i] = arr(v.pos) end
	for k, link in ipairs(polyline.links) do
		local own = link.kind and link.kind.network or network
		if own == nil then error("link " .. k .. " names no kind", 0) end
		links[k] = addEdge(kindOf(own), 0, 0, at[link.from + 1], at[link.to + 1],
			arr(link.tangent0), arr(link.tangent1))
	end

	for i, v in ipairs(polyline.vertices) do
		local p, r = at[i], v.resolve
		if skipped[i] then
			-- Left out, with its link.
		elseif r == "New" then
			ids[i] = addNode(p)
		elseif type(r) == "table" and r.Node then
			local n = nearest(nodes(r.Node), p, NODE_TOLERANCE)
			if n == nil then error("no " .. r.Node .. " node at vertex " .. i) end
			ids[i] = n.id
		elseif type(r) == "table" and r.Split then
			local s = r.Split
			local e = edgeBetween(nodes(s.network), s.network, arr(s.ends.a), arr(s.ends.b))
			if e == nil then error("no " .. s.network .. " edge to split at vertex " .. i) end
			if #(e.comp.objects or {}) > 0 then
				error("vertex " .. i .. " splits an edge with a stop or signal on it")
			end
			local tol = s.network == "Track" and geom.SPLIT_EPS_TRACK or geom.SPLIT_EPS
			local u, off = geom.parameterAt(e.a, e.ta, e.b, e.tb, p[1], p[2])
			local function from(q) local dx, dy = p[1] - q[1], p[2] - q[2] return math.sqrt(dx * dx + dy * dy) end
			if off > tol then error("vertex " .. i .. " is not on the edge it splits") end
			if from(e.a) < geom.SPLIT_MIN_DIST or from(e.b) < geom.SPLIT_MIN_DIST then
				error("vertex " .. i .. " splits the edge at its end")
			end
			local tm = geom.hermiteTangent(e.a, e.ta, e.b, e.tb, u)
			local mid = addNode(p)
			ids[i] = mid
			removeEdge(e)
			-- Each half the split edge's own component, read afresh, as the
			-- game's electrify task rebuilds an edge (electrify.tl).
			local component = api.type.ComponentType.BASE_EDGE
			addEdge(kindOf(s.network), e.node0, mid, e.a, p, scaled(e.ta, u), scaled(tm, u),
				api.engine.getComponent(e.id, component))
			addEdge(kindOf(s.network), mid, e.node1, p, e.b, scaled(tm, 1 - u), scaled(e.tb, 1 - u),
				api.engine.getComponent(e.id, component))
		else
			error("vertex " .. i .. " resolves as nothing this mod knows")
		end
	end

	for k, link in ipairs(polyline.links) do
		local s = links[k]
		s.comp.node0, s.comp.node1 = ids[link.from + 1], ids[link.to + 1]
		local structure, name = link.structure, "Ground"
		if type(structure) == "table" then name = next(structure) end
		s.comp.type = edgeType[STRUCTURE[name] or error("a link of structure " .. tostring(name))]
		if name == "Bridge" then
			s.comp.typeIndex = find("bridgeTypeRep", structure.Bridge)
		elseif name == "Tunnel" then
			s.comp.typeIndex = find("tunnelTypeRep", structure.Tunnel)
		else
			s.comp.typeIndex = -1
		end
		-- The build's own kind, or the kind the link names.
		local kind = link.kind or { network = network, template = templateName, style = style }
		local t = template(kind.template)
		s.comp.laneConfigs = t.laneConfigs
		s.comp.roadTemplate = kind.template
		s.comp.roadStyle = kind.style or t.streetStyle
		s.comp.roadType = kind.network == "Track" and enum("RoadType").TRACK or enum("RoadType").STREET
	end

	for k, r in ipairs(polyline.removals or {}) do
		local e = edgeBetween(nodes(r.network), r.network, arr(r.ends.a), arr(r.ends.b))
		if e == nil then error("no " .. r.network .. " edge to remove (" .. k .. ")") end
		-- An edge removed with its stops or signals leaves them pointing
		-- nowhere: on TPF2 that crashed every game at the same step
		-- (docs/BUILDING.md). The room does not carry them yet.
		if #(e.comp.objects or {}) > 0 then error("removal " .. k .. " has a stop or signal on it") end
		removeEdge(e)
	end

	local nodesToRemove, removedNode = {}, {}
	for k, n in ipairs(polyline.removed_nodes or {}) do
		local found = nearest(nodes(n.network), arr(n.at), END_TOLERANCE)
		if found == nil then error("no " .. n.network .. " node to remove (" .. k .. ")") end
		nodesToRemove[#nodesToRemove + 1] = found.id
		removedNode[found.id] = true
	end

	-- A node's lane configuration (BASE_NODE_CONFIG) names the edges at it,
	-- and the game cannot read a proposal that removes an edge a
	-- configuration still names (build 40408: "Unknown exception" from
	-- makeProposalData). So the configurations at the ends of the removed
	-- edges go too, and the game makes new ones; a node removed takes its
	-- own with it, and may not be named for both.
	local configsToRemove = {}
	for _, node in ipairs(ends) do
		if not removedNode[node]
			and api.engine.getComponent(node, api.type.ComponentType.BASE_NODE_CONFIG) ~= nil then
			configsToRemove[#configsToRemove + 1] = node
		end
	end

	proposal.streetProposal.nodesToAdd = nodesToAdd
	proposal.streetProposal.edgesToAdd = edgesToAdd
	proposal.streetProposal.edgesToRemove = edgesToRemove
	if #nodesToRemove > 0 then proposal.streetProposal.nodesToRemove = nodesToRemove end
	if #configsToRemove > 0 then proposal.streetProposal.nodeConfigsToRemove = configsToRemove end

	-- What is sent, in the log before it goes: an exception from the game
	-- does not always come back through pcall.
	local shape = {}
	for _, n in ipairs(nodesToAdd) do
		local p = n.comp.position
		shape[#shape + 1] = string.format("+n%d(%.1f,%.1f,%.1f)", n.entity, p.x, p.y, p.z)
	end
	for _, s in ipairs(edgesToAdd) do
		shape[#shape + 1] = "+e" .. s.entity .. "/" .. tostring(s.type) .. ":" .. tostring(s.comp.node0) .. ">"
			.. tostring(s.comp.node1) .. " " .. tostring(s.comp.roadTemplate)
	end
	shape[#shape + 1] = "-e" .. table.concat(edgesToRemove, ",") .. " -n" .. table.concat(nodesToRemove, ",")
		.. " -c" .. table.concat(configsToRemove, ",")
	log("building " .. table.concat(shape, " "))
end

local function buildNetwork(network, templateName, style, polyline)
	local proposal = api.type.SimpleProposal.new()
	networkInto(proposal, network, templateName, style, polyline)
	-- Paid by the player, as the tool builds.
	local context = api.type.Context.new()
	context.player = company()
	return buildProposal(proposal, context)
end

function HANDLERS.BuildRoad(road)
	return buildNetwork("Street", road.street, road.style, road.polyline)
end

-- The bulldozer's removals, as the game makes them itself: a construction
-- with what is its own (createProposalRemove: its entrance edge and node, as
-- the bulldozer proposed them on build 40408), or edges with the nodes they
-- leave on their own (makeSegmentsRemoveProposal). Paid by the player, as
-- the tool removes.
function HANDLERS.Bulldoze(b)
	local context = api.type.Context.new()
	context.player = company()
	local proposals = api.engine.util.proposal
	local proposal
	if b.Construction then
		local con = constructionAt(b.Construction)
		mine(con, "construction")
		proposal = proposals.createProposalRemove(con, context)
		if proposal == nil then error("the game will not remove the " .. tostring(b.Construction.file), 0) end
		log("removing " .. tostring(con) .. " " .. tostring(b.Construction.file))
	elseif b.Edges then
		local network = b.Edges.network
		local nodes = readNodes(network)
		local ids = {}
		for k, ends in ipairs(b.Edges.edges) do
			local e = edgeBetween(nodes, network, arr(ends.a), arr(ends.b))
			if e == nil then error("no " .. network .. " edge to remove (" .. k .. ")", 0) end
			if #(e.comp.objects or {}) > 0 then error("edge " .. k .. " has a stop or signal on it", 0) end
			mine(e.id, "road or track")
			ids[#ids + 1] = e.id
		end
		proposal = proposals.makeSegmentsRemoveProposal(ids)
		log("removing " .. network .. " edges " .. table.concat(ids, ","))
	elseif b.EdgeObject then
		-- A simple proposal: the game's verdict first (buildProposal).
		return removeEdgeObject(b.EdgeObject, context)
	else
		return false, "a bulldoze of no kind"
	end
	-- The game's verdict takes simple proposals only: a removal it refuses
	-- fails in the command's own answer (run).
	return run(api.cmd.makeWorldBuildProposalCmd(proposal, context, true, true))
end

function HANDLERS.BuildTrack(track)
	return buildNetwork("Track", track.track, track.style, track.polyline)
end

-- ---------------------------------------------------------------- stops
--
-- A stop is placed, or removed, as the stop tool and the bulldozer propose
-- it (tpf3mp/engine.lua): the edge removed and added again between the same
-- nodes, its own component read afresh (as the game's electrify task
-- rebuilds an edge, electrify.tl), so every stop and signal it had stays
-- under its own entity, re-parented with its station group and lines. A new
-- stop is `edgeObjectsToAdd[1]`, named in the edge's objects as -1 (TPF2's
-- tool and scripts did so; INFERRED on TF3); a removed one goes into
-- `edgeObjectsToRemove`. The lane configurations at the edge's ends name it,
-- and go with it, as for any edge a replay removes (networkInto).

-- The existing edge a stop action names, as edgeBetween finds it.
local function stopEdge(ref)
	local e = edgeBetween(readNodes(ref.network), ref.network, arr(ref.ends.a), arr(ref.ends.b))
	if e == nil then error("no " .. ref.network .. " edge for the stop", 0) end
	return e
end

-- A proposal that removes edge `e` and adds it again with `objects`.
local function rebuildWith(e, network, objects)
	local proposal = api.type.SimpleProposal.new()
	local s = api.type.SegmentAndEntity.new()
	s.entity = -1
	s.comp = api.engine.getComponent(e.id, api.type.ComponentType.BASE_EDGE)
	s.type = network == "Track" and 1 or 0
	s.comp.objects = objects
	proposal.streetProposal.edgesToAdd = { s }
	proposal.streetProposal.edgesToRemove = { e.id }
	local configs = {}
	for _, node in ipairs({ e.comp.node0, e.comp.node1 }) do
		if api.engine.getComponent(node, api.type.ComponentType.BASE_NODE_CONFIG) ~= nil then
			configs[#configs + 1] = node
		end
	end
	if #configs > 0 then proposal.streetProposal.nodeConfigsToRemove = configs end
	return proposal
end

-- How near its edge's centreline a stop's place is: the originator's own
-- point of that centreline, rounded to the millimetre.
local STOP_TOLERANCE = 0.5

-- The entity a proposal gives its first new edge object (build 40408).
local NEW_EDGE_OBJECT = -400000000

function HANDLERS.PlaceStop(stop)
	local network = stop.edge.network
	local e = stopEdge(stop.edge)
	local u, off = geom.parameterAt(e.a, e.ta, e.b, e.tb, stop.at.x, stop.at.y)
	if off > STOP_TOLERANCE then error("the stop's place is not on its edge", 0) end
	-- The engine's side, flipped where this edge runs the other way.
	local left = stop.left == true
	local t, d = geom.hermiteTangent(e.a, e.ta, e.b, e.tb, u), stop.direction
	if t[1] * d.x + t[2] * d.y + t[3] * d.z < 0 then left = not left end
	local types = enum("EdgeObjectType")
	-- The sides it takes: one, or both for a two-sided stop, the
	-- originator's first side first, as its tool added them.
	local sides = { left }
	if stop.two_sided == true then sides[2] = not left end
	-- One stop a side: a second is a fatal assert in the game's lane
	-- creation (TPF2, docs/BUILDING.md).
	local objects = {}
	for i, o in ipairs(e.comp.objects or {}) do
		for _, l in ipairs(sides) do
			if o[2] == (l and types.STOP_LEFT or types.STOP_RIGHT) then
				error("the edge has a stop on that side already", 0)
			end
		end
		objects[i] = { o[1], o[2] }
	end
	local added = {}
	for k, l in ipairs(sides) do
		-- A new edge object is named by its place in edgeObjectsToAdd,
		-- from -400000000 down (build 40408: con_util_entity_index.h
		-- asserts the range, a fatal error; game_mechanics/towns/
		-- town_util.tl; the stop tool's own proposals).
		objects[#objects + 1] = { NEW_EDGE_OBJECT - (k - 1), l and types.STOP_LEFT or types.STOP_RIGHT }
		local eo = api.type.SimpleStreetProposal.EdgeObject.new()
		eo.edgeEntity = -1
		eo.param = u
		eo.left = l
		eo.oneWay = false
		eo.model = stop.model
		eo.playerEntity = company()
		eo.name = ""
		added[k] = eo
	end
	local proposal = rebuildWith(e, network, objects)
	proposal.streetProposal.edgeObjectsToAdd = added
	log(string.format("placing %s on %s edge %d at %.4f, %s", tostring(stop.model), network, e.id, u,
		stop.two_sided == true and "both sides" or (left and "left" or "right")))
	-- Paid by the player, as the tool builds.
	local context = api.type.Context.new()
	context.player = company()
	return buildProposal(proposal, context)
end

-- A stop the bulldozer removes: the object of that construction on the
-- edge, nearest where it stood, within 2 m.
function removeEdgeObject(ref, context)
	local network = ref.edge.network
	local e = stopEdge(ref.edge)
	local best, bestD
	for _, o in ipairs(e.comp.objects or {}) do
		local c = api.engine.getComponent(o[1], api.type.ComponentType.EDGE_OBJECT)
		local t = c and c.transf
		if c and c.edgeObjectConstruction == ref.model and t then
			local dx, dy, dz = t[13] - ref.at.x, t[14] - ref.at.y, t[15] - ref.at.z
			local dist = dx * dx + dy * dy + dz * dz
			if dist <= 4 and (bestD == nil or dist < bestD or (dist == bestD and o[1] < best)) then
				best, bestD = o[1], dist
			end
		end
	end
	if best == nil then error("no " .. tostring(ref.model) .. " there", 0) end
	mine(best, "stop")
	local objects = {}
	for _, o in ipairs(e.comp.objects) do
		if o[1] ~= best then objects[#objects + 1] = { o[1], o[2] } end
	end
	local proposal = rebuildWith(e, network, objects)
	proposal.streetProposal.edgeObjectsToRemove = { best }
	log("removing " .. tostring(ref.model) .. " " .. tostring(best) .. " from " .. network .. " edge " .. e.id)
	return buildProposal(proposal, context)
end

-- ------------------------------------------------------ vehicles and lines
--
-- Vehicles, lines and station groups are named by canonical id
-- (tpf3mp/registry.lua): `ctx.registry` is the game script's, up to date.
-- A depot is named by its construction's file and position.

local registry = module("registry")
local companiesModule = module("companies")
require_companies = function() return companiesModule end

local function entityOf(ctx, kind, id)
	local e = registry.entity(ctx and ctx.registry, kind, id)
	if e == nil then error("no " .. kind .. " " .. tostring(id) .. " in this world", 0) end
	return e
end

-- As entityOf, for one the acting company must own: its own vehicles and
-- lines, never another company's.
local OWNED_WHAT = { vehicles = "vehicle", lines = "line" }
local function ownOf(ctx, kind, id)
	local e = entityOf(ctx, kind, id)
	mine(e, OWNED_WHAT[kind] or kind)
	return e
end

-- The construction of `ref.file` whose origin is within 2 m of `ref.at`, the
-- nearest, the lower entity on a tie.
function constructionAt(ref)
	local CONSTRUCTION = api.type.ComponentType.CONSTRUCTION
	local list = api.engine.getEntitiesWithComponent(CONSTRUCTION)
	local best, bestD
	for i = 1, #list do
		local e = list[i]
		local c = api.engine.getComponent(e, CONSTRUCTION)
		if c and c.fileName == ref.file then
			local t = c.transf
			local dx, dy, dz = t[13] - ref.at.x, t[14] - ref.at.y, t[15] - ref.at.z
			local d = dx * dx + dy * dy + dz * dz
			if d <= 4 and (bestD == nil or d < bestD or (d == bestD and e < best)) then best, bestD = e, d end
		end
	end
	if best == nil then error("no " .. tostring(ref.file) .. " there", 0) end
	return best, api.engine.getComponent(best, CONSTRUCTION)
end

local function tint(c) return api.type.Vec3f.new(c.r, c.g, c.b) end

-- The game's time here, the same in every game: when a part is bought.
local function now()
	return api.engine.getComponent(api.engine.util.getWorld(), api.type.ComponentType.GAME_TIME).gameTime
end

-- A ConsistPart as the game's TransportVehiclePart, bought at `time`. Every
-- compartment loads automatically, as the store sends it
-- (vehicle_react_util.tl).
local function vehiclePart(p, time)
	local model = api.res.modelRep.find(p.model)
	if type(model) ~= "number" or model < 0 then error("no vehicle model " .. tostring(p.model), 0) end
	local part = api.type.TransportVehiclePart.new()
	part.part.modelId = model
	part.part.reversed = p.reversed == true
	local loads, auto = {}, {}
	for k, l in ipairs(p.loads) do
		local lc = api.type.LoadConfig.new()
		lc.loadConfigIndex = l.config
		lc.cargoTypeId = l.cargo
		loads[k], auto[k] = lc, true
	end
	part.part.compartment2loadConfig = loads
	part.part.color = tint(p.color)
	part.purchaseTime = time
	part.autoLoadConfig = auto
	return part
end

-- A TransportVehicleConfig of these parts, groups and multiple units.
local function vehicleConfig(vehicles, groups, units)
	local config = api.type.TransportVehicleConfig.new()
	config.vehicles = vehicles
	config.vehicleGroups = seq(groups)
	config.muFileNames = seq(units)
	return config
end

function HANDLERS.BuyVehicle(buy)
	local _, construction = constructionAt(buy.depot)
	local depot = construction.depots and construction.depots[1]
	if depot == nil then error("the construction there has no depot", 0) end
	local time = now()
	local vehicles = {}
	for i, p in ipairs(buy.consist) do vehicles[i] = vehiclePart(p, time) end
	local config = vehicleConfig(vehicles, buy.groups, buy.multiple_units)
	local data, entities = send(api.cmd.makeVehicleBuyCmd(company(), depot, config))
	local vehicle = madeBy("resultVehicleEntity", data, entities)
	-- With more than one company, in its company's colour.
	local roster = acting and acting.roster
	if vehicle and companiesModule.painting(roster) then
		companiesModule.paintVehicle(companiesModule.byEntity(roster, company()), vehicle, send, api)
	end
	return true, vehicle
end

-- Whether `e` is a vehicle in this world.
local function isVehicle(e)
	if type(e) ~= "number" or e < 0 then return false end
	local ok, c = pcall(api.engine.getComponent, e, api.type.ComponentType.TRANSPORT_VEHICLE)
	return ok and c ~= nil
end

-- A vehicle's consist replaced, as the store's HandleVehicleChanges sends it
-- (vehicle_react_util.tl): a part the vehicle keeps is its own part, its
-- purchase time and wear as this game has them now, with the facing, loads
-- and colour the player chose; a new part is bought now. The vehicle is
-- then the entity the game names (its command data, its result entities)
-- that is a vehicle, else the vehicle itself: TF3's API says the vehicle is
-- replaced (cmd.d.tl), and its command data has no result field. Returns
-- that entity, for the registry to keep the vehicle's id on.
function HANDLERS.ReplaceVehicle(replace, ctx)
	local vehicle = ownOf(ctx, "vehicles", replace.vehicle)
	local tv = api.engine.getComponent(vehicle, api.type.ComponentType.TRANSPORT_VEHICLE)
	local own = tv and tv.transportVehicleConfig and tv.transportVehicleConfig.vehicles
	if own == nil then error("vehicle " .. tostring(replace.vehicle) .. " has no parts to read", 0) end
	local time = now()
	local vehicles, kept = {}, {}
	for i, r in ipairs(replace.consist) do
		local part = vehiclePart(r.part, time)
		if r.kept ~= nil then
			local old = own[r.kept + 1]
			if old == nil then error("part " .. i .. " keeps a part the vehicle does not have", 0) end
			if old.part.modelId ~= part.part.modelId then
				error("part " .. i .. " keeps a part of another model", 0)
			end
			if kept[r.kept] then error("part " .. i .. " keeps a part kept already", 0) end
			kept[r.kept] = true
			part.purchaseTime = old.purchaseTime
			part.maintenanceState = old.maintenanceState
			part.maintenanceChange = old.maintenanceChange
		end
		vehicles[i] = part
	end
	local config = vehicleConfig(vehicles, replace.groups, replace.multiple_units)
	local count = 0
	for _ in pairs(kept) do count = count + 1 end
	log("replacing vehicle " .. tostring(replace.vehicle) .. " (entity " .. tostring(vehicle) .. "): "
		.. #vehicles .. " part(s), " .. count .. " kept")
	local data, entities = send(api.cmd.makeVehicleReplaceCmd(vehicle, config))
	local candidates = {}
	for _, pair in ipairs(type(entities) == "table" and entities or {}) do
		if type(pair) == "table" then candidates[#candidates + 1] = pair[1] end
	end
	local ok, named = pcall(function() return data.vehicleEntity end)
	if ok then candidates[#candidates + 1] = named end
	candidates[#candidates + 1] = vehicle
	for _, e in ipairs(candidates) do
		if isVehicle(e) then
			if e ~= vehicle then log("vehicle " .. tostring(replace.vehicle) .. " is entity " .. e .. " now, was " .. vehicle) end
			return true, e
		end
	end
	return true, nil
end

function HANDLERS.SellVehicle(sell, ctx)
	local vehicles = {}
	for i, v in ipairs(sell.vehicles) do vehicles[i] = ownOf(ctx, "vehicles", v) end
	return run(api.cmd.makeVehicleSellCmd(vehicles))
end

function HANDLERS.AssignLine(assign, ctx)
	if assign.line == nil then
		return false, "this version of the mod does not take vehicles off their line yet"
	end
	local line = ownOf(ctx, "lines", assign.line)
	-- No first stop: the game's choice, the next stop each can reach (-1).
	local first = assign.first_stop
	if first == nil then first = -1 end
	for _, v in ipairs(assign.vehicles) do
		run(api.cmd.makeVehicleSetLineCmd(ownOf(ctx, "vehicles", v), line, first))
	end
	return true
end

function HANDLERS.VehicleOp(op, ctx)
	local vehicle = ownOf(ctx, "vehicles", op.vehicle)
	local change = op.change
	if type(change) == "table" and change.Stop ~= nil then
		return run(api.cmd.makeVehicleSetStoppedByUserCmd(vehicle, change.Stop == true))
	elseif type(change) == "table" and change.ToDepot then
		return run(api.cmd.makeVehicleSendToDepotCmd(vehicle, change.ToDepot.sell == true))
	elseif change == "Reverse" then
		return run(api.cmd.makeVehicleReverseCmd(vehicle))
	elseif change == "Depart" then
		return run(api.cmd.makeVehicleTryToDepartCmd(vehicle))
	end
	return false, "a vehicle change of no kind"
end

-- The game's load modes, by the schema's names, as numbers.
local LOAD_MODES = { LoadIfAvailable = 0, FullLoadAny = 1, FullLoadAll = 2, LegacyUnloadOnly = 3 }

-- A LineData as the game's Line component.
local function lineComponent(data, ctx)
	local line = api.type.Line.new()
	local stops = {}
	for i, s in ipairs(data.stops) do
		local stop = api.type.Line.Stop.new()
		stop.stationGroup = entityOf(ctx, "groups", s.group)
		stop.station = s.terminal.station
		stop.terminal = s.terminal.terminal
		local alternatives = {}
		for k, a in ipairs(s.alternatives) do
			alternatives[k] = api.type.StationTerminal.new(a.station, a.terminal)
		end
		stop.alternativeTerminals = alternatives
		stop.loadMode = LOAD_MODES[s.load_mode] or error("a load mode " .. tostring(s.load_mode), 0)
		stop.minWaitingTime = s.min_wait
		stop.maxWaitingTime = s.max_wait
		stop.maxAdditionalWaitingTime = s.max_extra_wait
		local config = api.type.Line.StopConfig.new()
		config.load = seq(s.rules.load)
		config.maxLoad = seq(s.rules.max_load)
		config.forceUnload = s.rules.force_unload == true
		config.destroyForConfigChange = s.rules.destroy_for_config_change == true
		config.destroyForRefresh = s.rules.destroy_for_refresh == true
		stop.stopConfig = config
		stops[i] = stop
	end
	line.stops = stops
	local modes = {}
	for _, m in ipairs(data.modes) do modes[m] = true end
	line.vehicleInfo.transportModes = modes
	line.customFilters = data.custom_filters == true
	line.reservationPriority = data.reservation_priority
	return line
end

function HANDLERS.CreateLine(create, ctx)
	local line = lineComponent(create.line, ctx)
	local data, entities = send(api.cmd.makeLineCreateCmd(create.name, tint(create.color),
		company(), line))
	return true, madeBy("resultEntity", data, entities)
end

function HANDLERS.EditLine(edit, ctx)
	local line = ownOf(ctx, "lines", edit.line)
	local change = edit.change
	if change == "Delete" then
		return run(api.cmd.makeLineDestroyCmd(line))
	elseif type(change) == "table" and change.Update then
		return run(api.cmd.makeLineUpdateCmd(line, lineComponent(change.Update, ctx)))
	elseif type(change) == "table" and change.Rename then
		return run(api.cmd.makeEntitySetNameCmd(line, change.Rename))
	elseif type(change) == "table" and change.Recolor then
		return run(api.cmd.makeEntitySetColorCmd(line, tint(change.Recolor)))
	end
	return false, "a line change of no kind"
end

-- What an action makes (the new vehicle, the new line): its kind in the
-- registry, which the game script binds it in after the action. Its
-- handler returns the entity, where the game said which.
apply.CREATES = { BuyVehicle = "vehicles", CreateLine = "lines" }

-- What an action changes and names by canonical id, which keeps its id
-- whatever entity it is after: the replaced vehicle. Its kind in the
-- registry and the field of the action naming it; its handler returns the
-- entity it is now, where this game could name it.
apply.KEEPS = { ReplaceVehicle = { kind = "vehicles", field = "vehicle" } }

-- A loan's terms as the loan script keeps them (loan.d.tl): the action's
-- table has the script's own field names and fractions.
local function loanTerms(terms)
	local out = {}
	for key, value in pairs(terms) do out[key] = value end
	return out
end

-- Loans go through the loan script's own events, with the parameters the
-- game's finance window sends (game_mechanics/finance/finances_loan_gui.tl):
-- here they run at once, in every game at the same update.
function HANDLERS.Loan(op, ctx)
	-- Another company's loans are the room's (tpf3mp/companies.lua): on the
	-- terms the game offers, booked to that company.
	if company() ~= api.engine.util.getPlayer() then
		local roster = ctx and ctx.roster
		local mine = roster and companiesModule.byEntity(roster, company())
		if not mine then return false, "the acting company is not in the room's roster" end
		if op.Take then return companiesModule.borrow(roster, mine.id, op.Take.offer, send, api) end
		if op.Repay then return companiesModule.repay(roster, mine.id, op.Repay.loan and op.Repay.loan.id, send, api) end
		return false, "a loan is taken or paid back"
	end
	if op.Take then
		return run(api.cmd.makeScriptingSendEventCmd("", "Loan", "Obtain",
			{ loanTerms(op.Take.next), loanTerms(op.Take.offer) }))
	elseif op.Repay then
		local param = {}
		param[2] = loanTerms(op.Repay.loan)
		return run(api.cmd.makeScriptingSendEventCmd("", "Loan", "Repay", param))
	end
	return false, "a loan is taken or paid back"
end

-- A notification's popup played its first sound: the game's Notifications
-- script's own event marks it (game_mechanics/notifications/
-- notifications.script.tl, "initialSound"), in every game, so no game
-- plays it again.
function HANDLERS.NotificationSeen(n)
	if type(n.notification) ~= "number" then error("a notification by its id", 0) end
	return run(api.cmd.makeScriptingSendEventCmd("", "Notifications", "initialSound",
		{ notificationId = n.notification }))
end

-- Prospecting goes through the company script's own event, with the
-- parameters the construction menu sends it (gui/construction/
-- construction_react_util.tl): here it runs at once, in every game at the
-- same update, so every game's company script keeps the same prospection
-- from the same game time, and months later draws the same outcome and
-- builds the same industry at the same place: it seeds its draws, and the
-- game its placement, from the game time
-- (investigation/TPF3_PROSPECTING_2026-09-30.md). The company is the
-- player's, as the menu names it; the industry types go in the order the
-- originator's menu listed them.
function HANDLERS.Prospect(p, ctx)
	local town = entityOf(ctx, "towns", p.town)
	local types = seq(p.industries)
	if #types == 0 then error("a prospection that can find no industry", 0) end
	log("prospecting for " .. tostring(p.cargo) .. " near town-" .. tostring(p.town) .. " (" .. tostring(town)
		.. "): " .. table.concat(types, ", "))
	return run(api.cmd.makeScriptingSendEventCmd("", "Companies", "spawnIndustry", {
		companyEntity = company(),
		townEntity = town,
		types = types,
		permitKey = p.permit,
		cargoType = p.cargo,
	}))
end

-- The room's companies (tpf3mp/companies.lua): the acting player founds,
-- joins, renames, recolours or dissolves one, in `ctx.roster`.
function HANDLERS.CompanyOp(op, ctx)
	if not (ctx and ctx.roster and ctx.player) then return false, "no roster to change" end
	local ok, why = companiesModule.run(ctx.roster, ctx.player, op, send, api)
	if not ok then return false, why end
	return true
end

-- Runs one action. `ctx` is { registry = } (tpf3mp/registry.lua), for the
-- actions that name vehicles, lines and station groups; with companies, also
-- `roster`, `player` (who sent it) and `company` (their company's player
-- entity), which the action is booked to. Returns true, nil
-- and the entity it made or changed (for the kinds in CREATES and KEEPS,
-- where the game said), or false and why not; never raises.
function apply.run(action, ctx)
	if type(action) ~= "table" then return false, "an action is a table" end
	local kind, body = next(action)
	if kind == nil or next(action, kind) ~= nil then
		return false, "an action is a table of one entry"
	end
	local handler = HANDLERS[kind]
	if handler == nil then
		return false, "this version of the mod does not apply " .. tostring(kind) .. " yet"
	end
	acting = ctx
	local ok, applied, detail = pcall(handler, body, ctx)
	acting = nil
	if not ok then return false, tostring(applied) end
	if applied == true then return true, nil, detail end
	return false, detail
end

-- The actions this version applies, for tests and the log.
function apply.kinds()
	local kinds = {}
	for kind in pairs(HANDLERS) do kinds[#kinds + 1] = kind end
	table.sort(kinds)
	return kinds
end

return apply
