-- Stand-in for placing a construction, then refreshing its connection.
-- Loaded after FAKE_NETWORK. A refresh refuses if the first build added
-- track over the construction's own track, as Steam build 40408 did in
-- the relay playtest on 2026-10-01.
api.type.ComponentType.CONSTRUCTION = 2
CONSTRUCTIONS = {}
local get = api.engine.getComponent
api.engine.getComponent = function(e, kind)
    if kind == 2 then return CONSTRUCTIONS[e] end
    return get(e, kind)
end
api.engine.getEntitiesWithComponent = function(kind)
    local out = {}
    if kind == 2 then for e in pairs(CONSTRUCTIONS) do out[#out + 1] = e end end
    return out
end
local send = api.cmd.sendCommand
api.cmd.sendCommand = function(cmd, callback)
    local p = cmd.proposal
    local c = p and p.constructionsToAdd and p.constructionsToAdd[1]
    if c then
        CONSTRUCTIONS[5000] = { fileName = c.fileName,
            transf = { 1,0,0,0, 0,1,0,0, 0,0,1,0, c.transf[4][1],c.transf[4][2],c.transf[4][3],1 } }
        -- The depot generates its own six edges. For the bus station,
        -- only the country road's two replacement edges are external.
        DUPLICATE_TRACK = false
        for _, edge in ipairs(p.streetProposal.edgesToAdd or {}) do
            if edge.comp.roadTemplate ~= '::/street/country.street_template' then DUPLICATE_TRACK = true end
        end
    end
    if p and p.refreshed then
        FAILS = REFUSE_SNAP or DUPLICATE_TRACK
        CONNECTED = not FAILS
    end
    return send(cmd, callback)
end
api.engine.util.proposal = { refreshConstruction = function(e)
    return { refreshed = e, proposal = {
        addedSegments = { { entity = -7, comp = { node0 = 8912, node1 = -1 } } },
        removedSegments = { { entity = 6000 } },
    } }
end }

-- The six-edge tree of the user's depot proposal, including its existing
-- anchor 8912. Positions and connectivity are from hook.log; tangents are
-- straight stand-ins because this regression exercises graph selection.
function depot_on_track()
    NODES[8912] = { x = -72.4978, y = -352.4803, z = 1.05 }
    local track = '::/infrastructure/track/simple/simple_catenary.street_template'
    local oldFind, oldGet = api.res.streetTemplateRep.find, api.res.streetTemplateRep.get
    api.res.streetTemplateRep.find = function(name)
        if name == track then return 6 end return oldFind(name)
    end
    api.res.streetTemplateRep.get = function(id)
        if id == 6 then return { laneConfigs = {} } end return oldGet(id)
    end
    api.engine.system.streetSystem.getNodeTrackSegments = function(id)
        if id == 8912 then return { 9100 } end return {}
    end
    api.engine.system.streetSystem.getNode2TrackEdgeMap = function() return { [8912] = { 9100 } } end
    local points = {
        { -71.5671, -347.5676 }, { -68.7752, -332.8298 },
        { -54.3900, -300.9502 }, { -42.1052, -236.1035 },
        { -62.4467, -299.4239 }, { -54.8153, -259.1404 },
    }
    local nodes, edges = {}, {}
    for i, p in ipairs(points) do
        nodes[i] = { entity = -i, comp = { position = { x = p[1], y = p[2], z = 1.05 } } }
    end
    for i, ends in ipairs({ {8912,-1}, {-2,-1}, {-3,-1}, {-3,-4}, {-5,-2}, {-6,-5} }) do
        local a = ends[1] > 0 and NODES[ends[1]] or nodes[-ends[1]].comp.position
        local b = nodes[-ends[2]].comp.position
        local t = { x = b.x-a.x, y = b.y-a.y, z = b.z-a.z }
        edges[i] = { entity = -6-i, type = 1, comp = {
            node0 = ends[1], node1 = ends[2], type = 0, typeIndex = -1,
            tangent0 = t, tangent1 = t, roadTemplate = track, roadStyle = '',
        } }
    end
    return { toRemove = {}, toAdd = { {
        fileName = '::/depots/rail/rail_depot.con', name = 'Rail depot', playerEntity = 25,
        transf = { 1,0,0,0, 0,1,0,0, 0,0,1,0, -60,-300,1.05,1 }, params = { seed = 7 },
    } }, proposal = { addedNodes = nodes, addedSegments = edges,
        removedNodes = {}, removedSegments = {}, edgeObjectsToAdd = {} } }
end
