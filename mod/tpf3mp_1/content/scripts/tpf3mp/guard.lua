-- tpf3mp/guard.lua -- the room's guard on the commands the GUI sends.
--
-- Transport Fever 3's GUI sends most of what a player does as commands,
-- through api.cmd.sendCommand: buying, selling and assigning vehicles,
-- lines, loans (as script events), a construction's parameters, the speed
-- (docs/HOOKS.md, "The player's commands"). In the room's game a command
-- must run in every game at the same update or in none, so the guard sits in
-- front of sendCommand in the GUI's Lua state:
--
-- - a command of a kind in PASS is sent as the player gave it;
-- - a command CARRY makes an action of is handed to the room instead,
--   which orders it for every game, this one included; its callback hears
--   what became of it when this game has applied it (deliver()), with what
--   it made: the new vehicle a window then puts on a line, the new line it
--   opens;
-- - every other kind is refused, as docs/PLAN.md (Part 3) says of every
--   action the room does not carry yet: it is not sent, its callback hears
--   on the next frame that it failed, and the player is told.
--
-- A command's kind is the name of the factory that made it
-- (api.cmd.make<Kind>Cmd), which the guard wraps to note it; a command no
-- wrapped factory made is refused. Outside the room's game (before the room
-- begins, after it ends) every command is sent as it would be.
--
-- Only the GUI state's api.cmd is wrapped. The mod's game script applies the
-- room's actions through its own state's api.cmd, which is left alone.
--
-- Pure Lua; the tests hand install() a fake api.cmd.

local guard = {}

-- Lua 5.2 (the game's) has table.unpack; Lua 5.1 (the tests') unpack.
local unpackArgs = table.unpack or unpack

-- The kinds sent as they are in the room's game, and why.
guard.PASS = {
	-- The speed row. In the room's game the step gate runs the room's pace
	-- whatever the game's own speed says, and reads that speed as the
	-- player's request to the room (docs/HOOKS.md, "The step gate in the
	-- game").
	makeGameSetSpeedCmd = true,
}

-- The vehicle and line commands are made actions by tpf3mp/capture.lua.
local function capture() return require("tpf3mp.capture") end
local function by(name)
	return function(ctx, ...) return capture()[name](ctx, ...) end
end

-- The commands the room carries, by kind: each makes an action table
-- (tpf3mp_proto::action, in the game's units) of the command's arguments,
-- given the context naming what they name (env.context, see capture.lua);
-- nil, or an error, for one it does not carry, which is then refused.
guard.CARRY = {
	-- The finance window's loans (finances_loan_gui.tl): the loan script's
	-- events, with the loans as the script keeps them. The construction
	-- menu's prospecting: the company script's spawnIndustry
	-- (capture.prospect).
	makeScriptingSendEventCmd = function(ctx, _src, id, name, param)
		if id == "Loan" and type(param) == "table" then
			if name == "Obtain" and type(param[1]) == "table" and type(param[2]) == "table" then
				return { Loan = { Take = { next = param[1], offer = param[2] } } }
			elseif name == "Repay" and type(param[2]) == "table" then
				return { Loan = { Repay = { loan = param[2] } } }
			end
		elseif id == "Companies" and name == "spawnIndustry" then
			return capture().prospect(ctx, param)
		elseif id == "Notifications" and name == "initialSound" and type(param) == "table"
			and type(param.notificationId) == "number" and param.notificationId >= 0
			and param.notificationId == math.floor(param.notificationId) then
			-- A popup played a notification's first sound (the game's
			-- notification_popups.tl): marked so in every game.
			return { NotificationSeen = { notification = param.notificationId } }
		end
		-- Which event, for the log.
		error("the " .. tostring(id) .. " script's " .. tostring(name) .. " event", 0)
	end,
	makeVehicleBuyCmd = by("vehicleBuy"),
	makeVehicleReplaceCmd = by("vehicleReplace"),
	makeVehicleSetLineCmd = by("vehicleSetLine"),
	makeVehicleSellCmd = by("vehicleSell"),
	makeVehicleSetStoppedByUserCmd = by("vehicleStop"),
	makeVehicleSendToDepotCmd = by("vehicleToDepot"),
	makeVehicleReverseCmd = by("vehicleReverse"),
	makeVehicleTryToDepartCmd = by("vehicleDepart"),
	makeLineCreateCmd = by("lineCreate"),
	makeLineUpdateCmd = by("lineUpdate"),
	makeLineDestroyCmd = by("lineDestroy"),
	makeEntitySetNameCmd = by("setName"),
	makeEntitySetColorCmd = by("setColor"),
	-- A construction's parameters changed in its window: an edit of that
	-- construction, which every game replaces alike. Other builds a window
	-- sends stay refused.
	makeWorldBuildProposalCmd = by("windowBuild"),
}

-- What a window's callback reads of a command it made that went, by kind:
-- the entity it made (the game's own command data has it there).
guard.RESULT = {
	makeVehicleBuyCmd = function(entity, args)
		return { resultVehicleEntity = entity, playerEntity = args[1], depotEntity = args[2], config = args[3] }
	end,
	makeLineCreateCmd = function(entity) return { resultEntity = entity } end,
	-- The game's windows send a replacement without a callback (build 40408,
	-- vehicle_react_util.tl); one that has one hears the vehicle as it is
	-- after (VehicleReplaceCommandData, api/tealdef/api/cmd.d.tl).
	makeVehicleReplaceCmd = function(entity, args)
		return { vehicleEntity = entity, config = args[2] }
	end,
}

-- What the player is told a refused kind is, where "this" would not do.
guard.WHAT = {
	makeVehicleBuyCmd = "buying vehicles",
	makeVehicleSellCmd = "selling vehicles",
	makeVehicleReplaceCmd = "replacing vehicles",
	makeVehicleSetLineCmd = "assigning vehicles to lines",
	makeVehicleSendToDepotCmd = "sending vehicles to a depot",
	makeVehicleReverseCmd = "reversing vehicles",
	makeVehicleSetStoppedByUserCmd = "stopping vehicles",
	makeVehicleSetModifiersCmd = "changing vehicles",
	makeLineCreateCmd = "creating lines",
	makeLineUpdateCmd = "changing lines",
	makeLineDestroyCmd = "deleting lines",
	makeWorldBuildProposalCmd = "building from this window",
	makeEntitySetNameCmd = "renaming",
	makeEntitySetColorCmd = "changing colours",
	makeGameSetCalendarSpeedCmd = "changing the calendar speed",
}

-- The factories the game's API reference declares (build 40408,
-- api/tealdef/api/cmd.d.tl). They are wrapped by name as well as by what
-- pairs() finds, in case api.cmd serves some through a metatable.
guard.FACTORIES = {
	"makeAnimalSetStateCmd", "makeAnimalSpawnAtCmd", "makeClearLogbooksCmd",
	"makeComponentExchangeCmd", "makeCreateIndustryExtendProposalCmd",
	"makeCustomEntityCreateCmd", "makeCustomEntityDestroyCmd",
	"makeCustomEntityUpdateStateCmd", "makeCustomEntityUpdateTransformationCmd",
	"makeCustomVehicleCreateOrUpdateCmd", "makeEntitySetColorCmd",
	"makeEntitySetEmissionsCmd", "makeEntitySetNameCmd", "makeEntitySetPlayerCmd",
	"makeGameAddPlayerCmd", "makeGamePerformSimulationStepsCmd",
	"makeGameSetCalendarSpeedCmd", "makeGameSetCloudCoverageCmd",
	"makeGameSetDateCmd", "makeGameSetSpeedCmd", "makeGameSetTimeOfDayCmd",
	"makeIndustrySetDespawnTimeCmd", "makeIndustrySetManualDevelopmentCmd",
	"makeJournalBookAssetCmd", "makeJournalClearAllCmd", "makeJournalLogEntryCmd",
	"makeLineCreateCmd", "makeLineDestroyCmd", "makeLineUpdateCmd",
	"makeMaintenanceCostUpdateCmd", "makeScriptingSendEventCmd",
	"makeSimPersonSetStateCmd", "makeStockListDiscardCargoCmd",
	"makeStockListSetModifiersCmd", "makeStockListSetStocksCargoTypeCmd",
	"makeStockSetCargoAmountCmd", "makeTownAutoDetectConnectionsCmd",
	"makeTownBuildingSetBlockedDevelopmentCmd", "makeTownConnectWithIndustriesCmd",
	"makeTownCreateCmd", "makeTownCustomDistributionWeightsCmd",
	"makeTownDestroyCmd", "makeTownDevelopAtCmd", "makeTownSetDevelopmentActiveCmd",
	"makeTownSetInitialLandUseCapacitiesCmd", "makeTownUpdateCargoNeedsCmd",
	"makeTownUpdateSizeCmd", "makeVehicleBuyCmd", "makeVehicleReplaceCmd",
	"makeVehicleReverseCmd", "makeVehicleSellCmd", "makeVehicleSendToDepotCmd",
	"makeVehicleSetLineCmd", "makeVehicleSetManualDepartureCmd",
	"makeVehicleSetModifiersCmd", "makeVehicleSetStoppedByUserCmd",
	"makeVehicleTryToDepartCmd", "makeWorldBuildProposalCmd",
	"makeWorldChangeWindCmd", "makeWorldReplaceTerrainCmd",
	"makeWorldSetBulldozableCmd",
}

-- What the player is told when a command of `kind` is refused.
function guard.notice(kind)
	return "Not in multiplayer yet: " .. (guard.WHAT[kind] or "this action")
end

-- The api.cmd tables already guarded, so a second install() changes
-- nothing, the callbacks waiting on each one's commands, by ticket, and
-- the answers held back until the GUI sees what they made.
local guarded = setmetatable({}, { __mode = "k" })
local waiting = setmetatable({}, { __mode = "k" })
local held = setmetatable({}, { __mode = "k" })

-- Calls to deliver() an answer waits at most for the GUI to see the entity
-- it made (one a frame: a few seconds).
guard.HOLD = 240

-- Puts the guard in front of `cmd` (the GUI state's api.cmd). `env` is:
--   inRoom()      -> whether the room's game runs;
--   command(t)    -> hands an action table to the room: true and its ticket,
--                    or nil and why;
--   refused(kind, why) -> a command of `kind` (nil: made by no factory the
--                    guard knows) was refused;
--   later(fn)     -> runs fn on the next frame;
--   context       -> names what commands name (tpf3mp/capture.lua).
-- Returns the number of factories wrapped, or nil and why the guard could
-- not be put there.
function guard.install(cmd, env)
	if type(cmd) ~= "table" then return nil, "api.cmd is not a table" end
	if guarded[cmd] then return guarded[cmd] end
	local send = cmd.sendCommand
	if send == nil then return nil, "api.cmd has no sendCommand" end

	-- The factory each command came from, and its arguments, by the command
	-- itself.
	local kinds = setmetatable({}, { __mode = "k" })
	local calls = setmetatable({}, { __mode = "k" })
	local factories = {}
	for name, factory in pairs(cmd) do
		if type(name) == "string" and name:match("^make.+Cmd$") then
			factories[name] = factory
		end
	end
	for _, name in ipairs(guard.FACTORIES) do
		if factories[name] == nil then factories[name] = cmd[name] end
	end
	local wrapped = 0
	for name, factory in pairs(factories) do
		cmd[name] = function(...)
			local command = factory(...)
			local t = type(command)
			if t == "table" or t == "userdata" then
				kinds[command] = name
				if guard.CARRY[name] then calls[command] = { n = select("#", ...), ... } end
			end
			return command
		end
		wrapped = wrapped + 1
	end

	-- The arguments go on exactly as given: a callback left out is not the
	-- same, to the game, as one passed as nil.
	cmd.sendCommand = function(command, ...)
		if not env.inRoom() then
			return send(command, ...)
		end
		local kind = kinds[command]
		if kind ~= nil and guard.PASS[kind] then
			return send(command, ...)
		end
		local callback = ...
		local carry, args = kind and guard.CARRY[kind], calls[command]
		local made, action = false, nil
		if carry and args then
			made, action = pcall(carry, env.context, unpackArgs(args, 1, args.n))
		end
		if made and action then
			local ok, ticket = env.command(action)
			if ok then
				if callback ~= nil then
					if type(ticket) == "number" then
						waiting[cmd][ticket] = { callback = callback, command = command, kind = kind, args = args }
					else
						env.later(function() callback(command, true, {}) end)
					end
				end
				return
			end
			env.refused(kind, ticket)
		elseif carry and args and not made then
			-- Why the room cannot carry it: what the capture raised.
			env.refused(kind, tostring(action))
		else
			env.refused(kind)
		end
		if callback ~= nil then
			env.later(function() callback(command, false, {}) end)
		end
	end
	guarded[cmd] = wrapped
	waiting[cmd] = {}
	return wrapped
end

-- What became of the commands the guard handed to the room: `results` is
-- the hook's list ({ ticket =, ok =, entity =, why = }, bridge.lua's
-- results()). Each waiting callback hears it, with what the room's action
-- made, as the game's own command would have answered, once `sees(entity)`
-- says the GUI's world has what it made: the game script made it in the
-- simulation, and a window that hears of it opens it at once. Answers keep
-- their order; one held back holds those after it, for HOLD calls at most.
-- A command that should have made something and made nothing the game
-- could name is answered as failed, which the windows handle, not as made.
-- Returns how many heard.
function guard.deliver(cmd, results, sees)
	local pending = waiting[cmd]
	if pending == nil then return 0 end
	local queue = held[cmd] or {}
	for _, r in ipairs(results or {}) do queue[#queue + 1] = { r = r, calls = 0 } end
	local heard, later = 0, {}
	for _, h in ipairs(queue) do
		local r = h.r
		local w = r.ticket and pending[r.ticket]
		if w then
			local unseen = r.entity ~= nil and sees ~= nil and not sees(r.entity)
			if #later > 0 or (unseen and h.calls < guard.HOLD) then
				h.calls = h.calls + 1
				later[#later + 1] = h
			else
				pending[r.ticket] = nil
				local result = guard.RESULT[w.kind]
				if result and r.ok == true and r.entity == nil then
					pcall(w.callback, w.command, false, {})
				else
					local data = (result and r.entity) and result(r.entity, w.args) or w.command
					local entities = r.entity and { { r.entity, 0 } } or {}
					pcall(w.callback, data, r.ok == true, entities)
				end
				heard = heard + 1
			end
		end
	end
	held[cmd] = later
	return heard
end

return guard
