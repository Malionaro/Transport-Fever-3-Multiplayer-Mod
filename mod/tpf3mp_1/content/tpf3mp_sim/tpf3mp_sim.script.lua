-- TPF3-MP's game script (tpf3mp_sim.gs.lua names it): where every game
-- applies the actions the room ordered, all in the same simulation update,
-- and reads the world's lanes at checkpoints (docs/HOOKS.md, "Actions in
-- the game" and "The world's lanes").
--
-- The game runs a game script's `update` once per simulation update, in an
-- engine state, where a command runs at once (build 40408, measured: the
-- update count goes up by one from call to call, dt 0.2). It runs game
-- scripts on a pool of Lua states, so what this file keeps is kept once per
-- state; the link to the hook is looked up in each. The game's own scripts
-- decide in `update` and act in `postUpdate`, which the game calls with
-- what `update` returned, and not when that is nil: company.script.tl
-- reads its argument unchecked, and a postUpdate after an update that
-- returned nothing never read a lane on build 40408. This one does the
-- same, so the world changes only in `postUpdate`:
--
-- - `update` asks the hook for the actions the room ordered (`take`), which
--   the hook hands only to the first update of the step they were ordered
--   for, the same update on every game, and whether this update ends a
--   batch at a checkpoint step (`checkpoint`). It returns both, or nil.
-- - `postUpdate` applies the actions (tpf3mp/apply.lua) and, at a
--   checkpoint, reads the world's lanes (tpf3mp/lanes.lua) and hands them
--   to the hook (`lanes`), which reports them to the room. When the hook
--   asks (`dump`: after a divergence, or TPF3MP_HOOK_LANE_DUMP), it also
--   hands it the lanes asked for entry by entry, for hook.log (docs/HOOKS.md,
--   "Lane dumps").
--
-- The hook holds the world if nobody took the actions, or if a checkpoint's
-- lanes did not come.
--
-- `guiHandleEvent` runs in the GUI's state, where the game's own build
-- tools (streets, tracks, stations and depots, stops on streets, the
-- bulldozer) tell game scripts of every proposal they make
-- (`builder.proposalCreate`), and
-- honour an error returned for it, as the game's company script does with
-- its permits (docs/HOOKS.md, "The build tools"). In the room's game:
--
-- - where the hook stops the player's builds (`clicks` is not nil), a
--   proposal of a tool the room carries (CAPTURE) is kept as the action it
--   makes (tpf3mp/capture.lua), marked with the clicks counted so far, and
--   builds nothing here: the hook answers false when the game applies it.
--   `guiUpdate` hands the room the one each click saw last, and the room
--   orders it for every game, this one included;
-- - every other proposal gets an error, so those tools build nothing.
--
-- The module editor tells game scripts nothing on build 40408 (CAPTURE):
-- the hook reads its build natively at the click, and `guiUpdate` takes it
-- for that click (tpf3mp_native.built), ahead of any preview, and makes the
-- edit of it as of the construction tool's proposal. A click with neither
-- is stopped with "no proposal seen". Every event of the room's game the
-- script does not handle is logged by id and name, once each, a few dozen
-- at most.
--
-- `handleEvent` takes the event `command` of id "tpf3mp" (sent with
-- api.cmd.makeScriptingSendEventCmd) and hands its parameter, an action
-- table, to the room: a way to act from the console, for tests. The event
-- reaches this game's scripts only, so only this game hands the action over;
-- the room then orders it for every game.
--
-- It also hears the company script's `startProspection` and
-- `endProspection` (game_mechanics/company/company.script.tl), which every
-- game's company script sends at the same update, and says in the hook's
-- log when a prospection began and what it found. A prospection that found
-- an industry binds it in the registry at once, so every game names it by
-- the same id (docs/HOOKS.md, "Prospecting").
function data()
	local MOD = "tpf3mp_1"
	-- Per Lua state: tried once, then kept.
	local tried, link, apply, lanes, capture, registry, companies = false, nil, nil, nil, nil, nil, nil
	-- Lanes that could not be read, and kinds the registry could not list,
	-- logged once per state.
	local told, toldRegistry = false, false
	-- Events subscribed to from this state.
	local subscribed = false
	-- Says what a prospection did (below).
	local prospected

	-- The events the script needs: its console event, and the build tools'
	-- proposals. Each by name, since a save may carry an older mod's
	-- subscriptions.
	local EVENTS = { "command", "builder.proposalCreate", "builder.proposalPrepareForApply",
		"startProspection", "endProspection" }

	-- What a build tool shows in the room's game.
	local REFUSED = "Not in multiplayer yet: building with this tool"

	-- The tools whose builds the room carries, by the tool's id: the
	-- capture that makes each one's action. On build 40408 the game tells
	-- game scripts of the proposals of six tools only, each under the id
	-- the game's GUI names it by (UI::CGameUI's constructor, read from the
	-- binary): constructionBuilder, streetTerminalBuilder, streetBuilder,
	-- trackBuilder, streetTrackModifier (the upgrade tool, not carried yet)
	-- and bulldozer. The module editor (UI::ModuleBuilder) tells them
	-- nothing there. moduleBuilder and moduleBulldozer are its names in the
	-- construction menu's parameters (ConstructionActionParam); INFERRED
	-- that a later build would send its proposals under them.
	local CAPTURE = { constructionBuilder = "construction", streetBuilder = "street", trackBuilder = "track",
		bulldozer = "bulldoze", streetTerminalBuilder = "stop", moduleBuilder = "construction",
		moduleBulldozer = "bulldoze" }
	-- In the GUI: the last proposal seen at each count of the player's builds
	-- ({ action = t } or { why = text }), and the builds handed on so far.
	local snapshots, handled = {}, nil
	-- The last reason a proposal was refused for, and how many were logged.
	local refusedWhy, refusals = nil, 0
	-- The events of the room's game the mod does not handle, by id and
	-- name, logged once each, a few dozen at most: what reaches the script
	-- when a tool's build is "no proposal seen".
	local unhandled, unhandledCount = {}, 0
	local function note(l, id, name)
		local key = tostring(id) .. " " .. tostring(name)
		if unhandled[key] or unhandledCount >= 40 then return end
		unhandled[key], unhandledCount = true, unhandledCount + 1
		l:log("an event the mod does not handle: id " .. tostring(id) .. ", name " .. tostring(name))
	end

	-- The snapshot of a module editor's click, from its proposal as the hook
	-- read it (tpf3mp_native.built) or why that did not read: the edit the
	-- construction tool's capture makes of it, which must replace the
	-- construction edited.
	local function moduleEdit(proposal, why)
		local shape = "module editor"
		if proposal == nil then
			return { why = "the module editor's edit did not read: " .. tostring(why), shape = shape }
		end
		local removes = type(proposal.toRemove) == "table" and #proposal.toRemove > 0
		local ok, action, whyNot = true, nil, "an edit that replaces no construction"
		if removes then ok, action, whyNot = pcall(capture.construction, proposal) end
		if not ok then action, whyNot = nil, tostring(action) end
		if action and action.BuildConstruction.replaces == nil then
			action, whyNot = nil, "an edit that replaces no construction"
		end
		if not action then return { why = "the module editor's edit: " .. tostring(whyNot), shape = shape } end
		return { action = action, shape = shape }
	end

	-- Extracts ground-plane position from a build proposal for advisory cursor sync.
	local function proposalPos(proposal)
		if type(proposal) ~= "table" and type(proposal) ~= "userdata" then return nil end
		local ok, px, py = pcall(function()
			local street = type(proposal.proposal) == "table" and proposal.proposal or nil
			if street and type(street.addedNodes) == "table" and #street.addedNodes > 0 then
				local last = street.addedNodes[#street.addedNodes]
				local pos = last and last.comp and last.comp.position
				if pos then
					local x = pos.x or pos[1]
					local y = pos.y or pos[2]
					if type(x) == "number" and type(y) == "number" then return x, y end
				end
			end
			local toAdd = type(proposal.toAdd) == "table" and proposal.toAdd or nil
			if toAdd and #toAdd > 0 then
				local first = toAdd[1]
				local transf = first and (first.transf or first.transformation)
				if type(transf) == "table" and #transf >= 14 then
					return transf[13], transf[14]
				end
			end
			return nil
		end)
		if ok and px and py then return px, py end
		return nil
	end

	local activeProposalSeen = false
	local localCursorActive = false
	local lastRemoteCursor = {}

	local function linked()
		if not tried then
			tried = true
			local okBridge, bridge = pcall(ug_require, MOD .. "::/scripts/tpf3mp/bridge.lua")
			local okApply, applyModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/apply.lua")
			local okLanes, lanesModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/lanes.lua")
			local okCapture, captureModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/capture.lua")
			local okRegistry, registryModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/registry.lua")
			local okCompanies, companiesModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/companies.lua")
			if okBridge and okApply and okLanes and okCapture and okRegistry and okCompanies
				and type(companiesModule) == "table" and type(bridge) == "table"
				and type(applyModule) == "table" and type(lanesModule) == "table"
				and type(captureModule) == "table" and type(registryModule) == "table" then
				link = bridge.attach(bridge.find())
				apply = applyModule
				if link then
					local linked = link
					apply.log = function(line) linked:log(line) end
				end
				lanes = lanesModule
				capture = captureModule
				registry = registryModule
				companies = companiesModule
				if link then link:log("the game script is linked") end
			end
		end
		return link
	end

	-- The entity an event names: a number, or the game's { entity = }.
	local function entityIn(value)
		if type(value) == "table" then value = value.entity end
		if type(value) == "number" then return value end
		return nil
	end

	-- A town as the room names it, for the log.
	local function townName(reg, value)
		local e = entityIn(value)
		local id = e and registry.id(reg, "towns", e)
		if id then return "town-" .. id end
		return "town entity " .. tostring(e)
	end

	-- The company script's prospection events, in the room's game: said in
	-- the log, and a found industry bound in the registry.
	function prospected(state, name, param)
		local l = linked()
		if not l or not l:room() or type(param) ~= "table" then return end
		local saved = state and state.get and state:get()
		if type(saved) ~= "table" or saved.registry == nil then return end
		local cargo = tostring(param.cargoType)
		local began = tostring(param.initiatedTimestamp)
		if name == "startProspection" then
			l:log("prospecting began: " .. cargo .. " near " .. townName(saved.registry, param.entity)
				.. " at game time " .. began)
			return
		end
		local where = cargo .. " near " .. townName(saved.registry, param.entity) .. ", begun at game time " .. began
		if param.success ~= true then
			l:log("prospecting ended: " .. where .. ", found nothing")
			return
		end
		local reg, fresh = registry.sync(saved.registry)
		saved.registry = reg
		state:set(saved)
		local found = {}
		for _, f in ipairs(fresh) do
			if f[1] == "industries" then
				local text = "industry-" .. f[2]
				local ok, c = pcall(api.engine.getComponent, f[3], api.type.ComponentType.CONSTRUCTION)
				if ok and type(c) == "table" and c.transf then
					text = text .. string.format(" %s at (%.1f, %.1f)", tostring(c.fileName), c.transf[13], c.transf[14])
				end
				found[#found + 1] = text
			end
		end
		if #found == 0 then
			l:log("prospecting ended: " .. where .. ", found an industry this game could not name")
		else
			l:log("prospecting ended: " .. where .. ", found " .. table.concat(found, "; "))
		end
	end

	return {
		update = function(_params, state, _dt)
			local l = linked()
			if not l then return nil end
			-- The room step's seed for this state's math.random, the same in
			-- every game at the same step (crates/tpf3mp-hook/src/seeds.rs).
			local seed = l:seed()
			if seed then math.randomseed(seed) end
			if not subscribed and state and state.subscribeToEvent then
				subscribed = true
				for _, event in ipairs(EVENTS) do state:subscribeToEvent(event) end
			end
			local actions, origins = l:take()
			local checkpoint = l:checkpoint()
			-- The registry begins at the room's first update, the same in
			-- every game (tpf3mp/registry.lua), or at the first update since
			-- the registry gained a kind.
			local saved = state and state.get and state:get()
			local begin = l:room() and (type(saved) ~= "table" or registry.incomplete(saved.registry))
			-- A month begun since the companies' loans were last charged.
			local month = companies.monthNow(api)
			local monthly = l:room() and type(saved) == "table" and companies.due(saved.companies, month)
			if not actions and not checkpoint and not begin and not monthly then return nil end
			return { actions = actions, origins = origins, checkpoint = checkpoint, begin = begin,
				monthly = monthly and month or nil }
		end,

		postUpdate = function(_params, state, _dt, work)
			local l = linked()
			if not l or type(work) ~= "table" then return end
			if work.actions or work.begin or work.monthly then
				local saved = state:get()
				if type(saved) ~= "table" then saved = {} end
				local reg, _, failed = registry.sync(saved.registry)
				-- The room's companies: begun at its first update, as the
				-- registry, the same in every game (tpf3mp/companies.lua).
				local roster = companies.ensure(saved.companies, api)
				if #failed > 0 and not toldRegistry then
					toldRegistry = true
					l:log("the registry could not list " .. table.concat(failed, "; "))
				end
				-- The room's builds go through; the player's own the hook
				-- stops.
				if work.actions then l:replaying(true) end
				for i, action in ipairs(work.actions or {}) do
					-- Booked to the sender's company.
					local player = work.origins and work.origins[i]
					local company = player and companies.of(roster, player)
					local ok, why, made = apply.run(action, {
						registry = reg,
						roster = roster,
						player = player,
						company = company and company.entity,
					})
					local name = next(action)
					-- What it changed keeps its id on whatever entity it is
					-- now, bound before the sync would retire it.
					local keeps = ok and apply.KEEPS[name] or nil
					if keeps then
						local id = type(action[name]) == "table" and action[name][keeps.field]
						if made then
							registry.rebind(reg, keeps.kind, id, made)
						else
							l:log("action " .. i .. " of this step left " .. keeps.kind .. " " .. tostring(id)
								.. " as nothing this game could name")
						end
					end
					-- What it made, bound at once, for the player who ordered
					-- it: as the game answered the command, else as the
					-- registry found it.
					local kind = ok and apply.CREATES[name] or nil
					local fresh
					reg, fresh = registry.sync(reg, (kind and made) and { [kind] = { made } } or nil)
					local entity = (kind or keeps) and made or nil
					for _, f in ipairs((kind and not entity) and fresh or {}) do
						if f[1] == kind then entity = f[3] break end
					end
					if kind and not entity then
						l:log("action " .. i .. " of this step made no " .. kind .. " this game could name")
					end
					l:applied(i, ok, entity, why)
					if not ok then
						l:log("action " .. i .. " of this step was not applied: " .. tostring(why))
					end
				end
				if work.actions then l:replaying(false) end
				if work.monthly then
					local ok, why = pcall(companies.chargeMonths, roster, work.monthly, apply.send, api)
					if not ok then l:log("the companies' loans were not charged: " .. tostring(why)) end
				end
				saved.registry = reg
				saved.companies = roster
				state:set(saved)
			end
			if work.checkpoint then
				local read, failed = lanes.read(api)
				if #failed > 0 and not told then
					told = true
					l:log("lanes read as err: " .. table.concat(failed, "; "))
				end
				local ok, why = l:lanes(read)
				if not ok then l:log("the lanes were not taken: " .. tostring(why)) end
				local dump = l:dump()
				if dump then
					local saved = state and state.get and state:get()
					local reg = type(saved) == "table" and saved.registry or nil
					-- Every entry is handed over: the hook keeps the first
					-- few thousand and counts the rest.
					for _, lane in ipairs(dump.lanes) do
						for _, entry in ipairs(lanes.dump(api, lane, reg)) do l:dumped(lane, entry) end
					end
				end
			end
		end,

		guiHandleEvent = function(_params, _state, _guiState, _src, id, name, param)
			if name ~= "builder.proposalCreate" and name ~= "builder.proposalPrepareForApply" then
				local l = linked()
				if l and l:room() then note(l, id, name) end
				return nil
			end
			local l = linked()
			if not l or not l:room() then return nil end
			if type(param) == "table" and param[1] then
				local px, py = proposalPos(param[1])
				if px and py then
					activeProposalSeen = true
					localCursorActive = true
					l:cursor(px, py, true, tostring(id))
				end
			end
			local clicks = l:clicks()
			local kind = CAPTURE[id]
			if kind == nil then note(l, id, name) end
			if clicks ~= nil and kind ~= nil and type(param) == "table" then
				local ok, action, why = pcall(capture[kind], param[1])
				if not ok then action, why = nil, tostring(action) end
				if action == false then
					-- Nothing proposed yet: nothing to refuse, nothing to hand on.
					snapshots[clicks] = nil
					return nil
				end
				local shape
				do
					local described, text = pcall(capture.describe, param[1])
					if described and text ~= "" then shape = text end
				end
				if not action then
					-- The tool refuses it at once, so no click follows: the log
					-- has it when the reason changes, a few dozen times at most.
					if why ~= refusedWhy and refusals < 40 then
						refusedWhy, refusals = why, refusals + 1
						l:log("the room cannot carry this " .. id .. " build: " .. tostring(why)
							.. (shape and (" [" .. shape .. "]") or ""))
					end
				end
				snapshots[clicks] = { action = action, why = why, shape = shape }
				if action then return nil end
				return { errorMessages = { ["Not in multiplayer yet: " .. tostring(why)] = true } }
			end
			-- A tool the room does not carry: its proposals' shapes, for the
			-- log, when they change, a few dozen times at most.
			if refusals < 40 and type(param) == "table" and capture then
				local described, text = pcall(capture.describe, param[1])
				local key = tostring(id) .. " " .. (described and text or "")
				if described and text ~= "" and key ~= refusedWhy then
					refusedWhy, refusals = key, refusals + 1
					l:log("the room does not carry the " .. tostring(id) .. " tool yet [" .. text .. "]")
				end
			end
			return { errorMessages = { [REFUSED] = true } }
		end,

		guiUpdate = function(_params, _state, _guiState)
			local l = linked()
			if not l then return end
			if l:room() then
				if activeProposalSeen then
					activeProposalSeen = false
				elseif localCursorActive then
					localCursorActive = false
					l:cursor(nil)
				end
				local cursors = l:cursors()
				for player, cursor in pairs(cursors) do
					if cursor.building and cursor.x and cursor.y then
						local key = tostring(player) .. " " .. tostring(cursor.label or "")
						if key ~= lastRemoteCursor[player] then
							lastRemoteCursor[player] = key
							l:log("player " .. tostring(player):sub(1, 8) .. " previewing build with "
								.. tostring(cursor.label or "tool") .. " at ("
								.. string.format("%.1f", cursor.x) .. ", "
								.. string.format("%.1f", cursor.y) .. ")")
						end
					elseif not cursor.building and lastRemoteCursor[player] then
						lastRemoteCursor[player] = nil
					end
				end
			end
			local clicks = l:clicks()
			if clicks == nil then return end
			if handled == nil then handled = clicks end
			while handled < clicks do
				local seen = snapshots[handled]
				-- The module editor's click: its build as the hook read it,
				-- whatever preview another tool showed before.
				local native, whyNot = l:built(handled)
				if native ~= nil or whyNot ~= nil then seen = moduleEdit(native, whyNot) end
				if seen and seen.action then
					local ok, why = l:command(seen.action)
					if ok then
						l:log("handed the player's build to the room"
							.. (seen.shape and (" [" .. seen.shape .. "]") or ""))
					else
						l:log("the player's build was not handed to the room: " .. tostring(why))
					end
				else
					l:log("stopped a build the room cannot carry: "
						.. tostring(seen and seen.why or "no proposal seen (a tool that tells game scripts nothing)")
						.. ((seen and seen.shape) and (" [" .. seen.shape .. "]") or ""))
				end
				snapshots[handled] = nil
				handled = handled + 1
			end
			for count in pairs(snapshots) do
				if count < handled then snapshots[count] = nil end
			end
		end,

		handleEvent = function(_params, state, _src, id, name, param)
			if id == "Company" and (name == "startProspection" or name == "endProspection") then
				prospected(state, name, param)
				return
			end
			if id ~= "tpf3mp" or name ~= "command" then return end
			local l = linked()
			if not l then return end
			local ok, why = l:command(param)
			if ok then
				l:log("handed a test action to the room")
			else
				l:log("refused a test action: " .. tostring(why))
			end
		end,
	}
end
