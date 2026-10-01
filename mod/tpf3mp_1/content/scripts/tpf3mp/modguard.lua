-- tpf3mp/modguard.lua -- the room's guard on what a player's personal mods
-- send from their game scripts (docs/MODS.md, proposed D25).
--
-- A personal mod runs in its player's game only. Most are GUI mods, whose
-- commands the GUI's guard (tpf3mp/guard.lua) carries or refuses. Some,
-- like a timetable mod or a line namer, decide in a game script, which runs
-- in the simulation's own Lua states, where a command runs at once and no
-- other game would run it. In the room's game this guard sits in front of
-- sendCommand in those states, as the GUI's does in the GUI's:
--
-- - a command from the game's own scripts, from TPF3-MP (the room's actions
--   apply.lua runs), or from a mod every player shares goes on as it was;
-- - a command from a personal mod of a kind CARRY names is not run here: it
--   is handed to the room as an action, which the room orders for every
--   game, this one included, a few updates on, and which every game applies
--   only to the acting player's company's own vehicles and lines
--   (apply.lua's ownOf); one for another company's is not handed on;
-- - the same change to the same thing within REPEAT_MS of game time is
--   handed on once: a game script that sees its hold not yet applied asks
--   again;
-- - an event from one game script to others (makeScriptingSendEventCmd)
--   reaches this game's scripts only, and is dropped: another mod's game
--   script, shared by every game, must not hear what only this one says;
-- - anything else from a personal mod is refused.
--
-- Which mod a command came from is read from the stack (guard.caller): the
-- game names a mod's files "<modId>::/...". Which mods are personal comes
-- from the hook (tpf3mp_native.personal, the room's Begin). Without the
-- lists every mod is the room's, as before, and this guard lets all through.
--
-- Pure Lua; the tests hand install() a fake api.cmd.

local modguard = {}

-- Lua 5.2 (the game's) has table.unpack; Lua 5.1 (the tests') unpack.
local unpackArgs = table.unpack or unpack

-- The api.cmd tables already guarded, by the factories wrapped.
local guarded = setmetatable({}, { __mode = "k" })

-- Actions handed on logged one by one, then every LOG_EVERY-th.
modguard.LOG_FIRST = 20
modguard.LOG_EVERY = 100

-- A module of the mod's, in whichever way this Lua state loads them.
local function module(name)
	if ug_require then return ug_require("tpf3mp_1::/scripts/tpf3mp/" .. name .. ".lua") end
	return require("tpf3mp." .. name)
end
local function capture() return module("capture") end
local function guardModule() return module("guard") end

-- The kinds carried from a personal mod's game script, by the capture that
-- makes each one's action (tpf3mp/capture.lua). tpf3mp-modscan's CARRIED
-- list names the same kinds (a mod that sends only these may be personal).
modguard.CARRY = {
	makeVehicleSetManualDepartureCmd = "vehicleManualDeparture",
	makeVehicleTryToDepartCmd = "vehicleDepart",
	makeVehicleSetStoppedByUserCmd = "vehicleStop",
	makeEntitySetNameCmd = "setName",
	makeLineUpdateCmd = "lineUpdate",
}

-- The kinds dropped, as said above.
modguard.DROP = {
	makeScriptingSendEventCmd = true,
}

-- Game time (ms) within which the same change to the same thing is handed
-- to the room once.
modguard.REPEAT_MS = 5000

-- What an action changes, and to what: the thing, and the change, as text.
local function keyOf(action)
	local op = action.VehicleOp
	if op then
		local change = op.change
		if type(change) == "table" then
			local name, value = next(change)
			change = tostring(name) .. "=" .. tostring(value)
		end
		return "vehicle " .. tostring(op.vehicle), tostring(change)
	end
	local edit = action.EditLine
	if edit then
		local name, value = next(edit.change or {})
		if type(value) == "table" then value = "" end
		return "line " .. tostring(edit.line), tostring(name) .. "=" .. tostring(value)
	end
	return nil
end

-- The engine entity a command acts on, for the company check.
local function subjectOf(args)
	return args[1]
end

-- Puts the guard in front of `cmd` (a simulation state's api.cmd). `env`:
--   inRoom()          -> whether the room's game runs;
--   personal(mod)     -> whether `mod` is one of this player's personal mods;
--   command(t)        -> hands an action table to the room: true, or nil
--                        and why;
--   context           -> names what commands name (tpf3mp/capture.lua);
--   mayTouch(entity)  -> optional: whether the player's company may act on
--                        the entity (tpf3mp/companies.lua's mayTouch), and
--                        why not;
--   now()             -> game time, in milliseconds;
--   log(line)         -> a line for the hook's log;
--   callers()         -> optional: the mods on the stack (guard.callers).
-- Returns the number of factories wrapped, or nil and why.
function modguard.install(cmd, env)
	if type(cmd) ~= "table" then return nil, "api.cmd is not a table" end
	if guarded[cmd] then return guarded[cmd] end
	local send = cmd.sendCommand
	if send == nil then return nil, "api.cmd has no sendCommand" end

	local kinds = setmetatable({}, { __mode = "k" })
	local calls = setmetatable({}, { __mode = "k" })
	-- The mods on the stack when each command was made.
	local makers = setmetatable({}, { __mode = "k" })
	local function stack() return (env.callers or guardModule().callers)() end
	local wrapped = 0
	local names = {}
	for name in pairs(modguard.CARRY) do names[#names + 1] = name end
	for name in pairs(modguard.DROP) do names[#names + 1] = name end
	for _, name in ipairs(names) do
		local factory = cmd[name]
		if factory ~= nil then
			cmd[name] = function(...)
				local command = factory(...)
				local t = type(command)
				if t == "table" or t == "userdata" then
					kinds[command] = name
					calls[command] = { n = select("#", ...), ... }
					makers[command] = stack()
				end
				return command
			end
			wrapped = wrapped + 1
		end
	end

	-- The last change handed on for each thing, and when.
	local last = {}
	local handed = 0
	-- Each kind refused or dropped, by mod, logged once.
	local told = {}
	local function tell(mod, what)
		local key = tostring(mod) .. " " .. what
		if told[key] then return end
		told[key] = true
		env.log(what .. ", from the personal mod " .. tostring(mod))
	end

	cmd.sendCommand = function(command, ...)
		if not env.inRoom() then return send(command, ...) end
		-- A personal mod anywhere on the stack: one calling a shared mod's
		-- helper is still its own.
		local from
		for _, mods in ipairs({ makers[command] or {}, stack() }) do
			for _, mod in ipairs(mods) do
				if from == nil and env.personal(mod) then from = mod end
			end
		end
		if from == nil then return send(command, ...) end
		local kind = kinds[command]
		if kind ~= nil and modguard.DROP[kind] then
			tell(from, "dropped " .. kind .. " (heard by this game's scripts only)")
			return
		end
		local capturing = kind and modguard.CARRY[kind]
		local args = calls[command]
		if not capturing or not args then
			tell(from, "refused " .. tostring(kind or "a command no factory made"))
			return
		end
		if env.mayTouch then
			local may, why = env.mayTouch(subjectOf(args))
			if not may then
				tell(from, "refused " .. kind .. " for another company's: " .. tostring(why))
				return
			end
		end
		local made, action = pcall(capture()[capturing], env.context, unpackArgs(args, 1, args.n))
		if not made or type(action) ~= "table" then
			tell(from, "refused " .. kind .. ": " .. tostring(action))
			return
		end
		local thing, change = keyOf(action)
		local now = env.now() or 0
		if thing then
			local prev = last[thing]
			if prev and prev.change == change and now - prev.at < modguard.REPEAT_MS then return end
			last[thing] = { change = change, at = now }
		end
		local ok, why = env.command(action)
		if ok then
			handed = handed + 1
			if handed <= modguard.LOG_FIRST or handed % modguard.LOG_EVERY == 0 then
				env.log("handed " .. kind .. " from the personal mod " .. tostring(from)
					.. " to the room (" .. handed .. " so far)")
			end
		else
			if thing then last[thing] = nil end
			tell(from, "the room did not take " .. kind .. ": " .. tostring(why))
		end
	end
	guarded[cmd] = wrapped
	return wrapped
end

return modguard
