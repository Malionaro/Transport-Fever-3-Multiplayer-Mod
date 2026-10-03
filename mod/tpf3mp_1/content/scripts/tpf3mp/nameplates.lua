-- tpf3mp/nameplates.lua -- the name of the member whose build preview this
-- game shows, at that preview (docs/HOOKS.md, "Build previews"). Advisory:
-- a name is drawn and nothing is built, sent or changed.
--
-- The other members' previews are kept by tpf3mp/previews.lua, each with the
-- point its geometry starts at; the room's roster (link:status().players)
-- says which member is which. This turns the two into labels the GUI can
-- place: the world point goes through the game's own projection
-- (api.gui.camera.world2Screen) to a place on the window, which
-- builtin.FloatingLayoutChild takes as a fraction of it.
--
-- Anything it cannot read shows no name rather than a wrong one: a member
-- the roster does not name, a preview without a point, a game that cannot
-- project, a point behind the camera or off the window.
--
-- Pure Lua; the tests hand it a roster, a table of previews and a fake api.

local nameplates = {}

-- How finely a label's place on the window is kept, as a fraction of it.
-- The game's projection answers whole pixels, which would make a name
-- jump about while the camera moves; a two-hundredth of the window is
-- finer than a pixel at every size the game runs in.
nameplates.STEPS = 200
-- The most labels at once: one per other member of the room.
nameplates.MAX = 8

-- Whether `v` is something the game's API answers with and fields can be
-- read off: a table, or the game's own objects (api.gui.camera, Vec2i,
-- Vec3f), which are userdata in the game.
local function readable(v)
	local t = type(v)
	return t == "table" or t == "userdata"
end

-- The point { x, y, z } on the game's own axes, in metres, or nil where
-- the value is not one.
local function point(p)
	if not readable(p) then return nil end
	local x, y, z = tonumber(p.x), tonumber(p.y), tonumber(p.z)
	if not x or not y or not z then return nil end
	return { x = x, y = y, z = z }
end

-- The point of a preview's action to hang a name at: where its geometry
-- starts, as the action says, in the game's metres. The four builds the
-- previews are shown for (tpf3mp/previews.lua, SHOWN); anything else shows
-- no name, rather than one at a place it does not mean.
function nameplates.anchor(action)
	if type(action) ~= "table" then return nil end
	local line = action.BuildRoad or action.BuildTrack
	if type(line) == "table" then
		local vertices = type(line.polyline) == "table" and line.polyline.vertices or nil
		local first = type(vertices) == "table" and vertices[1] or nil
		return point(type(first) == "table" and first.pos or nil)
	end
	local construction = action.BuildConstruction
	if type(construction) == "table" and type(construction.transform) == "table" then
		return point(construction.transform.origin)
	end
	local stop = action.PlaceStop
	if type(stop) == "table" then return point(stop.at) end
	return nil
end

-- The name the room's roster gives member `id`, or nil where it names no
-- one by it. The id is the 64 hex digits the hook gives both the roster
-- and a preview's sender.
function nameplates.nameOf(status, id)
	if type(status) ~= "table" or type(status.players) ~= "table" then return nil end
	for _, player in ipairs(status.players) do
		if type(player) == "table" and player.id == id then
			if type(player.name) == "string" and player.name ~= "" then return player.name end
			return nil
		end
	end
	return nil
end

-- Where the world point `at` is on the window, as a fraction of it; nil
-- where the game cannot say, or the point is not on the window (behind the
-- camera, which it answers with a point outside it: no name, rather than
-- one pinned to an edge).
function nameplates.place(api, at)
	local ok, place, why = pcall(function()
		if not readable(api) or not readable(at) then return nil, "no api or at" end
		local gui = api.gui
		if not readable(gui) then return nil, "no gui" end
		local camera = gui.camera
		if not readable(camera) then return nil, "no camera" end
		local size = camera.getSize()
		if not readable(size) then return nil, "no size" end
		local width, height = tonumber(size.x), tonumber(size.y)
		if not width or not height or width <= 0 or height <= 0 then return nil, "bad size" end

		-- When available, check whether the point is in front of the camera:
		-- (at - eye) . (center - eye) > 0.
		local okEye, eye = pcall(function() return camera.getEye() end)
		local okCenter, center = pcall(function() return camera.getCenter() end)
		if okEye and okCenter and readable(eye) and readable(center) then
			local forwardX, forwardY, forwardZ = center.x - eye.x, center.y - eye.y, center.z - eye.z
			local toX, toY, toZ = at.x - eye.x, at.y - eye.y, at.z - eye.z
			local dot = forwardX * toX + forwardY * toY + forwardZ * toZ
			if dot <= 0 then return nil, "behind camera" end
		end

		local at3 = api.type.Vec3f.new(at.x, at.y, at.z)
		local screen = camera.world2Screen(at3)
		if not readable(screen) then return nil, "no projection" end
		local x, y = tonumber(screen.x), tonumber(screen.y)
		if not x or not y then return nil, "projection unreadable" end
		if x < 0 or y < 0 or x > width or y > height then
			return nil, "off the window (" .. x .. "," .. y .. " of " .. width .. "x" .. height .. ")"
		end
		local steps = nameplates.STEPS
		return {
			h = math.floor(x * steps / width + 0.5) / steps,
			v = math.floor(y * steps / height + 0.5) / steps,
		}
	end)
	if not ok then return nil, tostring(place) end
	return place, why
end

-- The labels to draw now, one per other member whose preview shows and
-- whose name and place are both known, in the order of their ids however
-- Lua walks the table: the same labels then read the same on every frame,
-- and the GUI redraws only when one of them changed.
function nameplates.collect(remote, status, api)
	local labels = {}
	if type(remote) ~= "table" then return labels end
	for id, kept in pairs(remote) do
		if #labels >= nameplates.MAX then break end
		local at = type(kept) == "table" and kept.anchor or nil
		local name = nameplates.nameOf(status, id)
		local place = name and at and nameplates.place(api, at) or nil
		if name and place then
			labels[#labels + 1] = { id = id, name = name, h = place.h, v = place.v }
		end
	end
	table.sort(labels, function(one, other) return one.id < other.id end)
	return labels
end

-- Whether `labels` says the same as `before`: the same members, names and
-- places, in the same order.
function nameplates.same(labels, before)
	if type(labels) ~= "table" or type(before) ~= "table" then return false end
	if #labels ~= #before then return false end
	for index = 1, #labels do
		local one, other = labels[index], before[index]
		if one.id ~= other.id or one.name ~= other.name
			or one.h ~= other.h or one.v ~= other.v then
			return false
		end
	end
	return true
end

return nameplates