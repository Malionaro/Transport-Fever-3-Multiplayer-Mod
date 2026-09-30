-- tpf3mp/engine.lua -- Transport Fever 3's street and track tools and its
-- network, behind the plain interfaces tpf3mp/roads.lua takes.
--
-- What it reads, as build 40408 has it (the game's api/tealdef and the build
-- probe, tools/probe/tf3/tpf3mp_buildprobe_1): the street and track tools
-- hand game scripts a proposal (builder.proposalCreate's first parameter)
-- whose .proposal is a StreetProposal:
--
-- - addedNodes: the new nodes, entity < 0, comp.position;
-- - addedSegments: the new edges, entity < 0, type 0 street / 1 track, comp a
--   BaseEdge (node0, node1, tangent0, tangent1, type NORMAL / BRIDGE /
--   TUNNEL, typeIndex, roadTemplate and roadStyle, resource names, and the
--   stops and signals on it, objects);
-- - removedSegments and removedNodes: the existing edges and nodes it
--   removes.
--
-- A street drawn onto another's middle (seen): the old street's node nearest
-- the new junction is removed with its two edges, and the old street is
-- rebuilt from its neighbours through the junction, in its own template.
--
-- The lists are the game's vectors: read by index, never with pairs().

local function module(name)
	local loaded = package and package.loaded and package.loaded["tpf3mp." .. name]
	if loaded then return loaded end
	if ug_require then return ug_require("tpf3mp_1::/scripts/tpf3mp/" .. name .. ".lua") end
	return require("tpf3mp." .. name)
end

local roads = module("roads")
local geom = module("geom")

local engine = {}

local function get(value, key)
	local ok, v = pcall(function() return value[key] end)
	if ok then return v end
	return nil
end

-- The game's vector (or a table) as a Lua array.
local function list(v)
	if v == nil then return {} end
	local ok, n = pcall(function() return #v end)
	if not ok or type(n) ~= "number" then error("a list it cannot read", 0) end
	local out = {}
	for i = 1, n do out[i] = v[i] end
	return out
end

local function vec3(v)
	if v == nil then return nil end
	local x, y, z = get(v, "x"), get(v, "y"), get(v, "z")
	if x == nil then x, y, z = get(v, 1), get(v, 2), get(v, 3) end
	return { x, y, z }
end

-- The game's enums, under api.type.enum ("enum" is a word in Teal, which
-- writes api.type["enum"]).
local function enum(name)
	local enums = api.type.enum
	local e = enums and enums[name]
	if e == nil then error("no api.type.enum." .. name, 0) end
	return e
end

local function nodePos(id)
	local p
	pcall(function()
		local c = api.engine.getComponent(id, api.type.ComponentType.BASE_NODE)
		if c and c.position then p = vec3(c.position) end
	end)
	return p
end

-- A node's edges in one network, from the street system.
local function nodeEdges(id, network)
	local edges
	pcall(function()
		local streets = api.engine.system.streetSystem
		if network == "Track" then edges = streets.getNodeTrackSegments(id) else edges = streets.getNodeStreetSegments(id) end
	end)
	return edges
end

-- The world as tpf3mp/roads.lua asks it. Only the nodes a proposal names
-- are read: no edge of the map is walked.
function engine.world()
	local world = {}
	world.nodePos = nodePos
	function world.nodeNetwork(id)
		for _, network in ipairs({ "Street", "Track" }) do
			local edges = nodeEdges(id, network)
			if edges ~= nil and #list(edges) > 0 then return network end
		end
		return nil
	end
	return world
end

local function networkOf(seg)
	local kind = get(seg, "type")
	if kind == 0 then return "Street" end
	if kind == 1 then return "Track" end
	error("an edge of type " .. tostring(kind), 0)
end

local function resName(v, what)
	if type(v) ~= "string" or v == "" then error(what .. " is not a resource name: " .. tostring(v), 0) end
	return v
end

-- A bridge's or tunnel's type, by its name.
local function typeName(rep, index)
	local name
	pcall(function() name = api.res[rep].getName(index) end)
	if type(name) ~= "string" or name == "" then error("no " .. rep .. " type " .. tostring(index), 0) end
	return name
end

-- A road style: "" is none.
local function styleName(v)
	if v == "" or v == nil then return nil end
	return resName(v, "roadStyle")
end

-- Refuses an edge that carries stops or signals: the room does not carry
-- them yet, and an edge replaced without them leaves them pointing nowhere
-- (on TPF2 that crashed every game at the same step; docs/BUILDING.md).
local function noObjects(c, what)
	local objects = get(c, "objects")
	if objects ~= nil and #list(objects) > 0 then error(what .. " with a stop or signal on it", 0) end
end

local function segment(seg)
	local c = get(seg, "comp")
	if c == nil then error("an edge with no component", 0) end
	noObjects(c, "a build that moves an edge")
	local e = {
		node0 = c.node0, node1 = c.node1,
		network = networkOf(seg),
		tangent0 = vec3(c.tangent0), tangent1 = vec3(c.tangent1),
		structure = "Ground",
		template = resName(c.roadTemplate, "roadTemplate"),
		style = styleName(c.roadStyle),
	}
	local types = enum("BaseEdgeType")
	if c.type == types.BRIDGE then
		e.structure = { Bridge = typeName("bridgeTypeRep", c.typeIndex) }
	elseif c.type == types.TUNNEL then
		e.structure = { Tunnel = typeName("tunnelTypeRep", c.typeIndex) }
	elseif c.type ~= types.NORMAL then
		error("an edge of structure " .. tostring(c.type), 0)
	end
	return e
end

-- A tool's proposal as tpf3mp/roads.lua takes it, or raises. `network` is
-- the tool's; nil for the construction tool's, whose street part is taken
-- alone (`constructions` true lets the proposal carry them) in the network
-- of its first new edge. Returns nil for a proposal of nothing (the tool
-- before its first point, a construction with no street part).
function engine.fromProposal(proposal, network, constructions)
	local street = get(proposal, "proposal")
	if street == nil then error("a proposal with no street proposal", 0) end
	if not constructions then
		for _, name in ipairs({ "toAdd", "toRemove" }) do
			if #list(get(proposal, name)) > 0 then error("a build with constructions", 0) end
		end
	end
	for _, name in ipairs({ "edgeObjectsToAdd", "edgeObjectsToRemove" }) do
		local v = get(street, name)
		if v ~= nil and #list(v) > 0 then error("a build with a stop or signal", 0) end
	end
	local added, segments, removed = list(get(street, "addedNodes")), list(get(street, "addedSegments")),
		list(get(street, "removedSegments"))
	local removedNodes = list(get(street, "removedNodes"))
	if #added == 0 and #segments == 0 and #removed == 0 and #removedNodes == 0 then return nil end

	local capture = { network = network, nodes = {}, edges = {}, removed = {}, removedNodes = {} }
	for _, n in ipairs(added) do
		capture.nodes[#capture.nodes + 1] = { id = n.entity, pos = vec3(n.comp.position) }
	end
	local first
	for _, seg in ipairs(segments) do
		local e = segment(seg)
		capture.edges[#capture.edges + 1] = e
		if network == nil then network = e.network capture.network = network end
		if not first and e.network == network then first = e end
	end
	for _, seg in ipairs(removed) do
		noObjects(seg.comp, "a build that removes an edge")
		capture.removed[#capture.removed + 1] = { node0 = seg.comp.node0, node1 = seg.comp.node1,
			network = networkOf(seg) }
	end
	for _, n in ipairs(removedNodes) do
		capture.removedNodes[#capture.removedNodes + 1] = { id = n.entity, pos = vec3(get(n.comp, "position")) }
	end

	-- The build's own kind: its first edge of the tool's network. The
	-- template names the edge whole on TF3: its lanes, bus lanes and tram
	-- tracks. The schema's bus lane and tram are TPF2's, none here.
	if first then
		if network == "Street" then
			capture.street, capture.bus_lane, capture.tram = first.template, false, "None"
		else
			capture.track, capture.catenary = first.template, false
		end
		capture.style = first.style
	end
	return capture
end

-- A tool's proposal in one line, for the log: nodes added (+n) and removed
-- (-n), edges added (+e) and removed (-e) with their ends, existing nodes
-- with their positions; a construction's own nodes and edges (its frozen
-- ones) marked "*".
function engine.describe(proposal)
	local ok, text = pcall(function()
		local street = get(proposal, "proposal")
		local out = {}
		-- The constructions' own nodes and edges.
		local frozen = {}
		for _, c in ipairs(list(get(proposal, "toAdd"))) do
			local con = get(c, "construction")
			for _, key in ipairs({ "frozenNodes", "frozenEdges" }) do
				for _, e in ipairs(list(con and get(con, key))) do frozen[e] = true end
			end
		end
		local function mark(e) return frozen[e] and "*" or "" end
		local function at(p)
			p = vec3(p)
			if p == nil or type(p[1]) ~= "number" then return "(?)" end
			return string.format("(%.1f,%.1f,%.1f)", p[1], p[2], p[3])
		end
		local function node(id)
			if type(id) == "number" and id >= 0 then return tostring(id) .. at(nodePos(id)) end
			return tostring(id)
		end
		for _, n in ipairs(list(get(street, "addedNodes"))) do
			out[#out + 1] = "+n" .. tostring(n.entity) .. mark(n.entity) .. at(n.comp.position)
		end
		for _, n in ipairs(list(get(street, "removedNodes"))) do
			out[#out + 1] = "-n" .. tostring(n.entity) .. at(n.comp and n.comp.position)
		end
		for _, s in ipairs(list(get(street, "addedSegments"))) do
			out[#out + 1] = "+e" .. tostring(s.entity) .. mark(s.entity) .. "/" .. tostring(s.type) .. ":"
				.. node(s.comp.node0) .. mark(s.comp.node0) .. ">" .. node(s.comp.node1) .. mark(s.comp.node1)
		end
		for _, s in ipairs(list(get(street, "removedSegments"))) do
			out[#out + 1] = "-e" .. tostring(s.entity) .. ":" .. node(s.comp.node0) .. ">" .. node(s.comp.node1)
		end
		-- Stops, signals and waypoints: whatever of their fields reads.
		for _, o in ipairs(list(get(street, "edgeObjectsToAdd"))) do
			local fields = {}
			for _, key in ipairs({ "resultEntity", "category", "left", "playerEntity", "edgeEntity", "param", "model" }) do
				local v = get(o, key)
				if v ~= nil then fields[#fields + 1] = key .. "=" .. tostring(v) end
			end
			local mi = get(o, "modelInstance")
			if mi ~= nil then
				local t = get(mi, "transf")
				fields[#fields + 1] = "modelId=" .. tostring(get(mi, "modelId"))
				if t ~= nil then fields[#fields + 1] = "at=" .. at({ get(t, 13), get(t, 14), get(t, 15) }) end
			end
			out[#out + 1] = "+o{" .. table.concat(fields, " ") .. "}"
		end
		for _, c in ipairs(list(get(proposal, "toAdd"))) do
			local con = get(c, "construction")
			out[#out + 1] = "+c" .. tostring(get(c, "fileName")) .. "{frozen "
				.. #list(con and get(con, "frozenNodes")) .. "n " .. #list(con and get(con, "frozenEdges")) .. "e}"
		end
		for _, c in ipairs(list(get(proposal, "toRemove"))) do
			out[#out + 1] = "-c" .. tostring(c)
		end
		return table.concat(out, " ")
	end)
	if ok then return text end
	return "unreadable: " .. tostring(text)
end

-- A stop's removal ("stops", below).
local removeStop

-- The bulldozer's proposal as a Bulldoze action (tpf3mp_proto
-- action::Bulldoze): one construction, by its file and position, whose own
-- entrance edge and node the game removes with it; or edges of one network,
-- by their ends, with the nodes they leave on their own (build 40408, the
-- bulldozer hovered and clicked). false for a proposal of nothing; nil and
-- why the room cannot carry it.
function engine.bulldoze(proposal)
	local ok, action = pcall(function()
		local street = get(proposal, "proposal")
		if street == nil then error("a proposal with no street proposal", 0) end
		-- A stop removed: its edge rebuilt without it, nothing else.
		if #list(get(proposal, "toAdd")) == 0 and #list(get(proposal, "toRemove")) == 0
			and #list(get(street, "addedSegments")) == 1 and #list(get(street, "addedNodes")) == 0 then
			return removeStop(street)
		end
		if #list(get(proposal, "toAdd")) > 0 or #list(get(street, "addedSegments")) > 0
			or #list(get(street, "addedNodes")) > 0 then
			error("a bulldozer proposal that builds", 0)
		end
		for _, name in ipairs({ "edgeObjectsToAdd", "edgeObjectsToRemove" }) do
			local v = get(street, name)
			if v ~= nil and #list(v) > 0 then error("removing a stop or signal", 0) end
		end
		local toRemove = list(get(proposal, "toRemove"))
		if #toRemove > 1 then error("removing more than one construction at once", 0) end
		if #toRemove == 1 then
			local c = api.engine.getComponent(toRemove[1], api.type.ComponentType.CONSTRUCTION)
			if c == nil then error("removing something that is no construction", 0) end
			local t = c.transf
			return { Bulldoze = { Construction = {
				file = resName(c.fileName, "a construction's file"),
				at = { x = t[13], y = t[14], z = t[15] },
			} } }
		end
		local segments = list(get(street, "removedSegments"))
		if #segments == 0 then return false end
		local network, edges = nil, {}
		for k, seg in ipairs(segments) do
			local n = networkOf(seg)
			if network == nil then
				network = n
			elseif n ~= network then
				error("removing streets and tracks at once", 0)
			end
			noObjects(seg.comp, "removing an edge")
			local a, b = nodePos(seg.comp.node0), nodePos(seg.comp.node1)
			if a == nil or b == nil then error("removed edge " .. k .. " has no position here", 0) end
			edges[k] = { a = { x = a[1], y = a[2], z = a[3] }, b = { x = b[1], y = b[2], z = b[3] } }
		end
		return { Bulldoze = { Edges = { network = network, edges = edges } } }
	end)
	if not ok then return nil, tostring(action) end
	return action
end

-- ------------------------------------------------------------------ stops
--
-- The stop tool (streetTerminalBuilder) and the bulldozer over a stop hand
-- game scripts the shape the game's own mission scripts check a stop by
-- (mission_task_build_construction_util.tl, checkStop, build 40408): one
-- existing edge removed, and the same edge added again (a new entity between
-- the same two nodes) whose `objects` list the edge's stops and signals as
-- { entity, EdgeObjectType }, as many as `edgeObjectsToAdd`. An object the
-- edge had keeps its entity there (re-parented, its station group and lines
-- kept; TPF2's tool did so, docs/BUILDING.md), so the new stop is the one
-- entity the old edge did not list, and a removed one the entity the new
-- edge no longer lists. INFERRED: the order of `objects` is the order of
-- `edgeObjectsToAdd`, as the mission scripts' checks take it.

-- The one existing edge a stop proposal rebuilds: its removed and added
-- records, and its network; or raises.
local function rebuiltEdge(street)
	if #list(get(street, "addedNodes")) > 0 or #list(get(street, "removedNodes")) > 0 then
		error("a stop build that adds or removes nodes", 0)
	end
	local added, removed = list(get(street, "addedSegments")), list(get(street, "removedSegments"))
	if #added ~= 1 or #removed ~= 1 then error("a stop build of " .. #removed .. " edges", 0) end
	local old, new = get(removed[1], "comp"), get(added[1], "comp")
	if old == nil or new == nil then error("an edge with no component", 0) end
	if get(old, "node0") ~= get(new, "node0") or get(old, "node1") ~= get(new, "node1") then
		error("a stop build that moves its edge", 0)
	end
	local network = networkOf(removed[1])
	if networkOf(added[1]) ~= network then error("a stop build that changes its edge's network", 0) end
	return old, new, network
end

-- The objects an edge lists, as { entity, type } pairs, by entity.
local function objectsOf(comp)
	local out, byEntity = {}, {}
	for _, o in ipairs(list(get(comp, "objects"))) do
		local entity, kind = get(o, 1), get(o, 2)
		if type(entity) ~= "number" then error("an edge object it cannot read", 0) end
		out[#out + 1] = { entity, kind }
		byEntity[entity] = kind
	end
	return out, byEntity
end

-- The edge between an existing edge's two nodes, as the schema names it
-- (action::EdgeRef), and its geometry for geom.lua; or raises.
local function edgeRef(comp, network)
	local a, b = nodePos(get(comp, "node0")), nodePos(get(comp, "node1"))
	if a == nil or b == nil then error("the stop's edge has no position here", 0) end
	local ta, tb = vec3(get(comp, "tangent0")), vec3(get(comp, "tangent1"))
	if ta == nil or tb == nil or type(ta[1]) ~= "number" or type(tb[1]) ~= "number" then
		error("the stop's edge has no tangents", 0)
	end
	return { network = network, ends = { a = { x = a[1], y = a[2], z = a[3] }, b = { x = b[1], y = b[2], z = b[3] } } },
		{ a = a, b = b, ta = ta, tb = tb }
end

-- A stop placed with the stop tool, as a PlaceStop action (tpf3mp_proto
-- action::PlaceStop): the edge by its ends, the place on its centreline,
-- the engine's `left`, the edge's direction there and the stop's
-- construction. Transport Fever 3 builds a stop as a construction
-- ("stations/street/small_stops/small_new.con") and its tool's proposal
-- does not name it (build 40408: the edge objects have no model), so
-- `noted` is the one the construction menu gave the tool
-- (tpf3mp/capture.lua, capture.stop); a proposal whose edge object has a
-- model names it itself. A two-sided stop is one click that adds an object
-- on each side. false for a proposal of nothing; nil and why the room
-- cannot carry it.
function engine.placeStop(proposal, noted)
	local ok, action = pcall(function()
		local street = get(proposal, "proposal")
		if street == nil then error("a proposal with no street proposal", 0) end
		if #list(get(proposal, "toAdd")) > 0 or #list(get(proposal, "toRemove")) > 0 then
			error("a stop build with constructions", 0)
		end
		local toAdd = list(get(street, "edgeObjectsToAdd"))
		if #toAdd == 0 and #list(get(street, "addedSegments")) == 0 and #list(get(street, "removedSegments")) == 0 then
			return false
		end
		local old, new, network = rebuiltEdge(street)
		local _, had = objectsOf(old)
		local now, has = objectsOf(new)
		if #now ~= #toAdd then error("a stop build whose objects it cannot pair", 0) end
		-- A stop dropped where one stood replaces it, and the game moves its
		-- lines to the new one, which a replay cannot say (docs/BUILDING.md).
		for entity in pairs(had) do
			if has[entity] == nil then error("a stop that replaces another", 0) end
		end
		local added = {}
		for k, o in ipairs(now) do
			if had[o[1]] == nil then added[#added + 1] = k end
		end
		if #added == 0 then return false end
		if #added > 2 then error("more than two stops at once", 0) end
		local types = enum("EdgeObjectType")
		for _, k in ipairs(added) do
			local eo = toAdd[k]
			if get(eo, "category") ~= 0 then error("a signal or waypoint", 0) end
			-- INFERRED: the engine lists a stop it calls left as STOP_LEFT.
			if now[k][2] ~= (get(eo, "left") == true and types.STOP_LEFT or types.STOP_RIGHT) then
				error("a stop whose side the room cannot say", 0)
			end
		end
		local twoSided = #added == 2
		if twoSided and (get(toAdd[added[1]], "left") == true) == (get(toAdd[added[2]], "left") == true) then
			error("two stops on one side", 0)
		end
		local index = added[1]
		local eo = toAdd[index]
		local left = get(eo, "left") == true
		-- The model and place of the first of its objects that has them.
		local instance
		for _, k in ipairs(added) do
			instance = instance or get(toAdd[k], "modelInstance")
		end
		local model
		if instance ~= nil then
			pcall(function() model = api.res.modelRep.getName(get(instance, "modelId")) end)
		end
		if type(model) ~= "string" or model == "" then model = noted end
		model = resName(model, "the stop's construction (the stop tool's, noted by the GUI)")
		local ref, curve = edgeRef(old, network)
		-- Where along the edge: the proposal's own parameter where it has
		-- one, else the point of the centreline nearest the stop's model,
		-- else nearest the ground under the cursor, where the tool puts the
		-- stop (build 40408's proposal has neither). Every game builds it
		-- where this one says.
		local u = get(eo, "param")
		if type(u) ~= "number" or u < 0 or u > 1 then
			local t = instance and get(instance, "transf")
			local x, y = t and get(t, 13), t and get(t, 14)
			if type(x) ~= "number" or type(y) ~= "number" then
				pcall(function()
					if api.gui.mouse.hasTerrainPosition() then
						local p = api.gui.mouse.getTerrainPosition()
						x, y = p.x, p.y
					end
				end)
			end
			if type(x) ~= "number" or type(y) ~= "number" then error("a stop with no place", 0) end
			u = geom.parameterAt(curve.a, curve.ta, curve.b, curve.tb, x, y)
		end
		local at = geom.hermitePos(curve.a, curve.ta, curve.b, curve.tb, u)
		local d = geom.hermiteTangent(curve.a, curve.ta, curve.b, curve.tb, u)
		local len = math.sqrt(d[1] * d[1] + d[2] * d[2] + d[3] * d[3])
		if len == 0 then error("the stop's edge has no direction there", 0) end
		return { PlaceStop = {
			edge = ref,
			at = { x = at[1], y = at[2], z = at[3] },
			left = left,
			direction = { x = d[1] / len, y = d[2] / len, z = d[3] / len },
			model = model,
			two_sided = twoSided,
		} }
	end)
	if not ok then return nil, tostring(action) end
	return action
end

-- The bulldozer over a stop: the edge rebuilt without it. A Bulldoze of the
-- edge object (action::Bulldoze::EdgeObject): the edge by its ends, where
-- the stop stands and its construction, as its EDGE_OBJECT component says;
-- or raises. INFERRED: the bulldozer proposes a stop's removal in the stop
-- tool's shape, less the stop (as TPF2's did).
function removeStop(street)
	local old, new, network = rebuiltEdge(street)
	local had = objectsOf(old)
	local _, has = objectsOf(new)
	local gone
	for _, o in ipairs(had) do
		if has[o[1]] == nil then
			if gone ~= nil then error("removing more than one stop at once", 0) end
			gone = o[1]
		end
	end
	if gone == nil or #had ~= #list(get(new, "objects")) + 1 then
		error("a bulldozer proposal that rebuilds an edge", 0)
	end
	local c = api.engine.getComponent(gone, api.type.ComponentType.EDGE_OBJECT)
	local t = c and get(c, "transf")
	local x, y, z = get(t, 13), get(t, 14), get(t, 15)
	if type(x) ~= "number" or type(y) ~= "number" or type(z) ~= "number" then
		error("a stop with no place", 0)
	end
	local ref = edgeRef(old, network)
	return { Bulldoze = { EdgeObject = {
		edge = ref,
		at = { x = x, y = y, z = z },
		model = resName(get(c, "edgeObjectConstruction"), "the stop's construction"),
	} } }
end

-- The action table of a street or track tool's proposal; false for a
-- proposal of nothing; or nil and why the room cannot carry it.
function engine.captureBuild(proposal, network)
	local ok, capture = pcall(engine.fromProposal, proposal, network)
	if not ok then return nil, tostring(capture) end
	if capture == nil then return false end
	return roads.capture(capture, engine.world())
end


return engine
