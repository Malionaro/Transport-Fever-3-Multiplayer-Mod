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
-- Pure Lua; the tests hand it a fake api.

local follow = {}

-- Replaces api.engine.util.getPlayer in this Lua state, once, with one that
-- answers `mine()`, the player entity of the company this player plays for,
-- or, when that is nil (outside the room, before the roster is read, the
-- room's first company), the game's own answer. Returns true, or false and
-- why.
function follow.install(api, mine)
	if type(package) == "table" and type(package.loaded) == "table" and package.loaded["tpf3mp.followed"] then
		return true
	end
	local ok, util = pcall(function() return api.engine.util end)
	if not ok then util = nil end
	-- A function, or a callable table, as the game's bindings are (build
	-- 40408: a table with a metatable).
	local original = ok and util ~= nil and select(2, pcall(function() return util.getPlayer end)) or nil
	if type(original) ~= "function" and type(original) ~= "table" and type(original) ~= "userdata" then
		return false, "no api.engine.util.getPlayer (" .. type(util) .. ", " .. type(original) .. ")"
	end
	local replaced, why = pcall(function()
		util.getPlayer = function(...)
			local got, entity = pcall(mine)
			if got and type(entity) == "number" then return entity end
			return original(...)
		end
	end)
	-- A binding may take the assignment and keep its own function.
	local took = replaced and select(2, pcall(function() return util.getPlayer ~= original end))
	if took ~= true then
		return false, "api.engine.util (" .. type(util) .. ") keeps its getPlayer"
			.. (why and (": " .. tostring(why)) or "")
	end
	if type(package) == "table" and type(package.loaded) == "table" then
		package.loaded["tpf3mp.followed"] = true
	end
	return true
end

-- The player entity of the company the player `me` (64 hex digits) plays
-- for in `roster` (tpf3mp/companies.lua), or nil: not in the roster, or
-- playing for the room's first, which is the save's own player anyway.
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
