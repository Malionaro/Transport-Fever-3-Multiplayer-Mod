-- TPF3-MP in the game's GUI state: the plugin gui/tpf3mp/tpf3mp.res.lua
-- names. On the first step of a game it loads the mod's modules and links
-- to the hook (tpf3mp/bridge.lua). Without a hook, which is every game Steam
-- started, it logs one line and does nothing more. The room's actions are
-- applied by the mod's game script (tpf3mp_sim/), not here.
--
-- Every frame it does what the hook asks (docs/HOOKS.md, "The room's
-- world"): save the world under a name when the room orders a save, or load
-- the room's world from the game's save folder. It tells the hook each time
-- a world's GUI starts, which is how the hook sees a load finish.
--
-- Once linked, it puts the guard in front of the GUI's commands
-- (tpf3mp/guard.lua; docs/HOOKS.md, "The player's commands"): in the room's
-- game a command the room carries goes to the room, and its window hears
-- what became of it once this game has applied it; a command the room
-- cannot carry yet is refused, and the game bar says so for a few seconds.
-- The guard names vehicles, lines, station groups and towns by the canonical
-- ids the mod's game script keeps in its state (tpf3mp/registry.lua).
--
-- It follows what mods made for Transport Fever 3 build 40391 rely on
-- (investigation/TF3_MODS_2026-09-27.md): a .script.lua defines data();
-- ug_require loads the game's files ("::/...") and a mod's own
-- ("tpf3mp_1::/..."); a GameBarInfoDisplayExtension plugin with
-- react.onStep runs code every frame in a game; debugPrint writes to the
-- game's log. Each step logs "[tpf3mp]" lines, so the log shows how far a
-- game got on release day.
--
-- In the room's game the game bar also shows the room in one line (its
-- name, who is there, its speed, whether the worlds match), a button that
-- opens the Multiplayer window: the room, its players, its speed, whether
-- this world matches the room's, and the room's chat, which the player can
-- write to (docs/PLAN.md: the in-game Multiplayer panel for a game the
-- launcher started; the lobby stays in the launcher). The game's window
-- container shows the window, and the game's area for mods' buttons has a
-- second button for it (a second plugin, tpf3mp_button.res.lua). What
-- they show is kept in package.loaded["tpf3mp.ui"], which the game bar
-- plugin fills every frame from the hook (tpf3mp/bridge.lua: status, chat,
-- say).
function data()
	local MOD = "tpf3mp_1"
	-- Every module, in an order where each needs only those before it.
	local MODULES = { "geom", "roads", "engine", "registry", "companies", "follow", "capture", "bridge", "guard" }
	-- Frames a refusal's notice stays in the game bar.
	local NOTICE_FRAMES = 360

	local function say(line)
		pcall(debugPrint, "[tpf3mp] " .. line)
	end

	-- The link to the hook, once a world's GUI has found it.
	local link = nil

	-- What the Multiplayer window and the game bar show, shared by both
	-- plugins: the room (link:status()), the chat so far, whether the window
	-- is open, and a count that goes up whenever any of it changes; and the
	-- link, which the game bar plugin makes: the game may run this file
	-- once for each plugin, each with its own locals.
	local function ui()
		local shared = package.loaded["tpf3mp.ui"]
		if type(shared) ~= "table" then
			shared = { open = false, status = nil, lines = {}, unread = 0, version = 0 }
			package.loaded["tpf3mp.ui"] = shared
		end
		return shared
	end

	-- Callbacks of refused commands, for the next frame, as the game would
	-- call them.
	local pending = {}
	-- The notice of the last refusal, until the plugin shows it.
	local notice = nil
	-- Refusals so far, by kind, and the last reason logged of each, for the
	-- hook's log.
	local refusals, reasons = {}, {}

	local function refused(kind, why)
		local name = kind or "command no factory made"
		local count = (refusals[name] or 0) + 1
		refusals[name] = count
		local changed = why ~= nil and why ~= reasons[name]
		if changed then reasons[name] = why end
		if count == 1 or count % 100 == 0 or changed then
			link:log("refused the player's " .. name .. " in the room's game ("
				.. count .. " so far)" .. (why and (": " .. tostring(why)) or ""))
		end
		notice = require("tpf3mp.guard").notice(kind)
	end

	local function runPending()
		if #pending == 0 then return end
		local due = pending
		pending = {}
		for _, fn in ipairs(due) do
			local ok, err = pcall(fn)
			if not ok then say("a refused command's callback failed: " .. tostring(err)) end
		end
	end

	-- The mod's game script's state as the game keeps it
	-- (tpf3mp/companies.lua), which holds its registry (tpf3mp/registry.lua)
	-- and its companies. Read once the modules are loaded.
	local function scriptState()
		return require("tpf3mp.companies").scriptState(api)
	end
	local function registryNow()
		local state = scriptState()
		return state and state.registry
	end

	-- What the guard names things by (tpf3mp/capture.lua).
	local function idOf(kind)
		return function(entity)
			return require("tpf3mp.registry").id(registryNow(), kind, entity)
		end
	end
	local context = {
		vehicle = idOf("vehicles"),
		line = idOf("lines"),
		group = idOf("groups"),
		town = idOf("towns"),
		player = function()
			local ok, player = pcall(function() return api.engine.util.getPlayer() end)
			if ok and type(player) == "number" then return player end
			return nil
		end,
		depot = function(depot)
			local c
			pcall(function()
				local con = api.engine.system.streetConnectorSystem.getConstructionEntityForDepot(depot)
				c = con and api.engine.getComponent(con, api.type.ComponentType.CONSTRUCTION)
			end)
			if c == nil then return nil end
			local t = c.transf
			return { file = c.fileName, at = { x = t[13], y = t[14], z = t[15] } }
		end,
		model = function(id)
			local ok, name = pcall(function() return api.res.modelRep.getName(id) end)
			if ok and type(name) == "string" and name ~= "" then return name end
			return nil
		end,
		-- A vehicle's parts as its TRANSPORT_VEHICLE component has them.
		parts = function(vehicle)
			local ok, parts = pcall(function()
				local tv = api.engine.getComponent(vehicle, api.type.ComponentType.TRANSPORT_VEHICLE)
				local list = tv.transportVehicleConfig.vehicles
				local out = {}
				for i = 1, #list do
					out[i] = { model = list[i].part.modelId, purchased = list[i].purchaseTime }
				end
				return out
			end)
			if ok then return parts end
			return nil
		end,
	}

	-- The GUI state's api.cmd, which the guard is on.
	local guardedCmd = nil

	-- Whether the GUI's world has `entity` yet: what the room's action made
	-- in the simulation reaches it a moment later.
	local function sees(entity)
		local ok, there = pcall(function() return api.engine.entityExists(entity) end)
		return not ok or there == true
	end

	-- Puts the guard in front of the GUI's commands.
	local function guardCommands()
		local ok, cmd = pcall(function() return api.cmd end)
		guardedCmd = ok and cmd or nil
		local wrapped, why = require("tpf3mp.guard").install(guardedCmd, {
			inRoom = function() return link:room() end,
			command = function(action) return link:command(action) end,
			refused = refused,
			later = function(fn) pending[#pending + 1] = fn end,
			context = context,
		})
		if wrapped then
			link:log("the guard is on " .. wrapped .. " command factories")
		else
			link:log("the guard is not on: " .. tostring(why)
				.. "; the player's commands are not checked")
		end
	end

	-- The modules name each other `require "tpf3mp.<name>"`, as TPF2's
	-- did. The game's GUI state has `require` and package.loaded but no
	-- package.preload (build 40408's dump,
	-- investigation/dayone-2026-09-29/probe/script_api_dump_gui.txt), so
	-- each module is loaded here through ug_require, in order, into
	-- package.loaded, where the others' `require` finds it. Only names
	-- under "tpf3mp." are added, so nothing else in the state changes.
	local function installModules()
		if type(package) ~= "table" or type(package.loaded) ~= "table" then
			return nil, "this Lua state has no package.loaded"
		end
		for _, name in ipairs(MODULES) do
			local key = "tpf3mp." .. name
			if package.loaded[key] == nil then
				local path = MOD .. "::/scripts/tpf3mp/" .. name .. ".lua"
				local ok, module = pcall(ug_require, path)
				if not ok or module == nil then
					return nil, key .. " did not load: " .. tostring(module)
				end
				package.loaded[key] = module
			end
		end
		return true
	end

	-- The player entity of the company this player plays for, in the room's
	-- game (tpf3mp/companies.lua), or nil: outside the room, before the
	-- roster is read, and for the room's first company, which is the save's
	-- own player anyway.
	local function myCompany()
		local shared = ui()
		local status = shared.status
		return require("tpf3mp.follow").companyOf(shared.companies, status and status.me_id)
	end

	-- The GUI's "my company" in this Lua state (tpf3mp/follow.lua).
	local function followMyCompany()
		local ok, why = require("tpf3mp.follow").install(api, myCompany)
		link:log(ok and "the GUI's company follows the player's"
			or ("the GUI's company cannot follow the player's: " .. tostring(why)))
	end

	local function start()
		local ok, why = installModules()
		if not ok then
			say("not started: " .. why)
			return
		end
		say("modules loaded")

		local bridge = require "tpf3mp.bridge"
		local found, reason = bridge.attach(bridge.find())
		if not found then
			say(reason .. "; this is the plain game")
			return
		end
		link = found
		ui().link = link
		link:world()
		link:log("the GUI is linked")
		say("linked to the hook")
		guardCommands()
		followMyCompany()
		-- The stop the construction menu gives the stop tool, wherever the
		-- menu runs (gui/tpf3mp/gui_state.script.lua watches the other state).
		pcall(function()
			require("tpf3mp.capture").watchStopTool(ug_require "::/gui/construction/construction_react_util.tl", link)
		end)
	end

	-- Does what the hook asks: saving the world under the name it gives, or
	-- loading the room's world from the game's save folder.
	local function serve()
		if not link then return end
		local request = link:poll()
		if not request then return end
		if request.save then
			local name = request.save
			local ok, err = pcall(app.saveGame, name, function()
				link:saved(name, true)
			end, false, true)
			if not ok then link:saved(name, false, tostring(err)) end
		elseif request.load then
			local ok, err = pcall(function()
				local id = api.type.SavegameId.new()
				id.path = ""
				id.saveGameName = request.load
				id.saveGameNamespace = app.SaveGameNamespace.getSavegame()
				app.loadGame(id, false, nil)
			end)
			if ok then
				link:log("loading the room's world")
			else
				link:log("loading the room's world failed: " .. tostring(err))
			end
		end
	end

	local readCompanies
	local react = ug_require "::/gui/main/react.lua"
	local builtin = ug_require "::/gui/main/builtin.lua"
	local game_bar_widgets = ug_require "::/gui/game_bar/game_bar_widgets.tl"
	local main_mod_button_area = ug_require "::/gui/main/main_mod_button_area.tl"
	local game_react_globals = ug_require "::/gui/main/game_react_globals.tl"

	-- Chat lines kept, newest last, and how many of them the window shows.
	local CHAT_LINES = 50
	local CHAT_SHOWN = 12
	-- Frames between two readings of the room.
	local STATUS_FRAMES = 15

	-- The room's speed as the speed row says it.
	local function speedText(speed)
		if speed == nil then return "" end
		if speed == 0 then return "paused" end
		return string.format("%gx", speed / 100)
	end

	-- The room in one line, for the game bar.
	local function summary(status)
		local here, all = 0, 0
		for _, p in ipairs(status.players or {}) do
			all = all + 1
			if p.connected then here = here + 1 end
		end
		local parts = { "Multiplayer: " .. tostring(status.room), here .. "/" .. all .. " playing" }
		if status.speed then parts[#parts + 1] = speedText(status.speed) end
		if status.diverged then parts[#parts + 1] = "resyncing" end
		return table.concat(parts, " · ")
	end

	-- The room's companies as the game script keeps them (tpf3mp/companies.lua),
	-- each with its money now, and a text that changes when anything shown
	-- does. Nil before the room's first company exists.
	readCompanies = function()
		local state = scriptState()
		local roster = state and state.companies
		if type(roster) ~= "table" or type(roster.list) ~= "table" then return nil, "" end
		local out, sign = { list = {}, members = roster.members or {}, loans = roster.loans or {} }, {}
		-- The loans the game offers now (its loan script's), which another
		-- company takes on the same terms.
		pcall(function()
			local e = api.engine.system.gameScriptSystem.getEntityForGameScript("::/game_mechanics/finance/loan.gs")
			local c = type(e) == "number" and e >= 0 and api.engine.getComponent(e, api.type.ComponentType.GAME_SCRIPT)
			local offers = c and c.state and c.state.availableLoans
			if type(offers) == "table" then out.offers = offers end
		end)
		for _, offer in ipairs(out.offers or {}) do sign[#sign + 1] = tostring(offer.type) .. tostring(offer.amount) end
		for _, loan in ipairs(out.loans) do sign[#sign + 1] = loan.id .. ":" .. loan.remaining end
		for _, c in ipairs(roster.list) do
			if not c.gone then
				local balance, owed, name
				pcall(function()
					local account = api.engine.getComponent(c.entity, api.type.ComponentType.ACCOUNT)
					balance = account and account.balance
					owed = account and account.loan
				end)
				-- The name the game shows (the player entity's NAME, which a
				-- rename sets), else the roster's.
				pcall(function()
					local n = api.engine.getComponent(c.entity, api.type.ComponentType.NAME)
					if n and type(n.name) == "string" and n.name ~= "" then name = n.name end
				end)
				out.list[#out.list + 1] = { id = c.id, entity = c.entity, name = name or c.name, color = c.color,
					balance = balance, owed = owed }
				local color = type(c.color) == "table" and c.color or {}
				sign[#sign + 1] = table.concat({ c.id, name or c.name, tostring(balance), tostring(owed),
					tostring(color[1]), tostring(color[2]), tostring(color[3]) }, ":")
			end
		end
		for _, m in ipairs(out.members) do sign[#sign + 1] = tostring(m.player) .. "=" .. tostring(m.company) end
		return out, table.concat(sign, "|")
	end

	-- Reads the room and its chat from the hook into ui(): the room every
	-- STATUS_FRAMES frames, the chat every frame.
	local statusFrames = 0
	local function follow()
		if not link then return end
		local shared = ui()
		local changed = false
		statusFrames = statusFrames - 1
		if statusFrames <= 0 then
			statusFrames = STATUS_FRAMES
			local status = link:status()
			local before = shared.status and summary(shared.status) .. tostring(#(shared.status.players or {}))
			local after = status and summary(status) .. tostring(#(status.players or {}))
			shared.status = status
			if before ~= after then changed = true end
			local companies, sign = readCompanies()
			shared.companies = companies
			if sign ~= shared.companiesSign then
				shared.companiesSign = sign
				changed = true
			end
		end
		-- A new world's GUI gets the chat so far again, as old lines: they
		-- fill the window without counting as new.
		for _, line in ipairs(link:chat()) do
			shared.lines[#shared.lines + 1] = tostring(line.from) .. ": " .. tostring(line.text)
			if #shared.lines > CHAT_LINES then table.remove(shared.lines, 1) end
			if not shared.open and not line.old then shared.unread = shared.unread + 1 end
			changed = true
		end
		local cursors = link:cursors()
		if type(cursors) == "table" then
			shared.cursors = cursors
		end
		if changed then shared.version = shared.version + 1 end
	end

	-- What the Multiplayer window shows: the room, its speed, whether this
	-- world matches the room's, its players, and the chat with a field to
	-- write to it.
	-- Money as the game bar writes it, near enough.
	local function money(balance)
		if type(balance) ~= "number" then return "" end
		local sign, whole = balance < 0 and "-" or "", tostring(math.floor(math.abs(balance) + 0.5))
		whole = whole:reverse():gsub("(%d%d%d)", "%1,"):reverse():gsub("^,", "")
		return sign .. "$" .. whole
	end

	local function vec3(color)
		if type(color) ~= "table" then return nil end
		return api.type.Vec3f.new(color[1] or 0, color[2] or 0, color[3] or 0)
	end

	-- A company operation for the room, from the window. What became of it
	-- comes back with the player's other actions (follow the ticket).
	local function companyOp(shared, op, doing, action)
		local l = shared.link
		if not l then return end
		local ok, ticket = l:command(action or { CompanyOp = op })
		if ok then
			shared.asked = shared.asked or {}
			shared.asked[ticket] = doing
			shared.companyNote = doing .. "..."
		else
			shared.companyNote = "Not sent: " .. tostring(ticket)
		end
		shared.version = shared.version + 1
	end

	-- The companies: each with its money and players, the one you play for
	-- first, with its colour and name to change; the others to join; a
	-- company of your own to found.
	local function companyRows(rows, status, shared, drafts)
		local roster = shared.companies
		if not roster then return end
		local function line(text) rows[#rows + 1] = builtin.TextView{ text = text } end
		local names, companyOf, mine = {}, {}, nil
		for _, m in ipairs(roster.members) do companyOf[m.player] = m.company end
		for _, p in ipairs(status.players or {}) do
			local id = companyOf[p.id] or 0
			names[id] = names[id] or {}
			names[id][#names[id] + 1] = tostring(p.name) .. (p.me and " (you)" or "")
			if p.me then mine = id end
		end
		if mine == nil then mine = companyOf[status.me_id] or 0 end
		local ordered = {}
		for _, c in ipairs(roster.list) do if c.id == mine then ordered[#ordered + 1] = c end end
		for _, c in ipairs(roster.list) do if c.id ~= mine then ordered[#ordered + 1] = c end end
		line("")
		line("Companies")
		for _, c in ipairs(ordered) do
			local who = names[c.id] and table.concat(names[c.id], ", ") or "nobody"
			local children = {}
			if c.id == mine then
				children[#children + 1] = builtin.ColorChooserButton{
					meta = { tooltip = "Your company's colour" },
					colors = (function()
						local palette = {}
						for i, color in ipairs(require("tpf3mp.companies").PALETTE) do palette[i] = vec3(color) end
						return palette
					end)(),
					color = vec3(c.color),
					onValueChange = function(v)
						local r, g, b = v.x or v[1], v.y or v[2], v.z or v[3]
						companyOp(shared, { Recolor = { company = c.id, color = { r = r, g = g, b = b } } },
							"Recolouring " .. c.name)
					end,
					resetButton = false,
				}
			end
			children[#children + 1] = builtin.TextView{
				text = "  " .. tostring(c.name) .. "  " .. money(c.balance)
					.. ((type(c.owed) == "number" and c.owed > 0) and (" (owes " .. money(c.owed) .. ")") or "")
					.. "  " .. who .. (c.id == mine and "  (yours)" or ""),
			}
			if c.id ~= mine then
				children[#children + 1] = builtin.Button{
					meta = { tooltip = "Play for " .. tostring(c.name) .. " from now on" },
					content = builtin.TextView{ text = "Join" },
					onClick = function() companyOp(shared, { Join = c.id }, "Joining " .. c.name) end,
				}
			elseif c.id ~= 0 and #(names[c.id] or {}) <= 1 then
				-- Its last player dissolves it, once it owns nothing.
				children[#children + 1] = builtin.Button{
					meta = { tooltip = "Dissolve " .. tostring(c.name) .. " once it owns nothing,"
						.. " and play for the room's first company again" },
					content = builtin.TextView{ text = "Dissolve" },
					onClick = function() companyOp(shared, { Delete = c.id }, "Dissolving " .. c.name) end,
				}
			end
			rows[#rows + 1] = builtin.BoxLayout{ orientation = builtin.type.Orientation.Horizontal, children = children }
		end
		local function field(draft, placeholder, label, tooltip, act)
			rows[#rows + 1] = builtin.BoxLayout{
				orientation = builtin.type.Orientation.Horizontal,
				children = {
					builtin.TextInputField{
						placeholderText = placeholder,
						value = draft:get(),
						maxLength = 64,
						acceptOnFocusLoss = false,
						resetValueOnCancel = false,
						onTyping = function(text) draft:set(text) end,
						onCancel = function() shared.version = shared.version + 1 end,
						onValueChange = function(text) act(text) end,
					},
					builtin.Button{
						meta = { tooltip = tooltip },
						content = builtin.TextView{ text = label },
						onClick = function() act(draft:get()) end,
					},
				},
			}
		end
		local myName
		for _, c in ipairs(roster.list) do if c.id == mine then myName = c.name end end
		field(drafts.rename, "A new name for " .. tostring(myName), "Rename", "Rename the company you play for",
			function(text)
				if type(text) ~= "string" or text:match("^%s*$") then return end
				companyOp(shared, { Rename = { company = mine, name = text } }, "Renaming " .. tostring(myName))
				drafts.rename:set("")
			end)
		field(drafts.found, "A company of your own", "Found", "Found a company and play for it",
			function(text)
				if type(text) ~= "string" or text:match("^%s*$") then return end
				companyOp(shared, { Create = { name = text } }, "Founding " .. text)
				drafts.found:set("")
			end)
		-- Another company's loans are the room's (tpf3mp/companies.lua), on
		-- the terms the game offers; the first company's are in the game's
		-- own finance window.
		if mine ~= 0 then
			local loans = {}
			for _, loan in ipairs(roster.loans or {}) do if loan.company == mine then loans[#loans + 1] = loan end end
			for _, loan in ipairs(loans) do
				rows[#rows + 1] = builtin.BoxLayout{
					orientation = builtin.type.Orientation.Horizontal,
					children = {
						builtin.TextView{ text = "  Loan: " .. money(loan.remaining) .. " owed of " .. money(loan.amount)
							.. ", " .. money(loan.payment) .. " a month, " .. (loan.months - loan.paid) .. " months left" },
						builtin.Button{
							meta = { tooltip = "Pay back what is still owed now" },
							content = builtin.TextView{ text = "Repay" },
							onClick = function()
								companyOp(shared, nil, "Repaying " .. money(loan.remaining), { Loan = { Repay = { loan = {
									type = "Custom", amount = loan.amount, duration = 1, percentage = 0, id = loan.id } } } })
							end,
						},
					},
				}
			end
			local offers = {}
			for _, offer in ipairs(roster.offers or {}) do
				if type(offer) == "table" and type(offer.amount) == "number" then
					offers[#offers + 1] = builtin.Button{
						meta = { tooltip = string.format("Borrow %s at %g%% a year", money(offer.amount),
							(offer.percentage or 0) * 100) },
						content = builtin.TextView{ text = "Borrow " .. money(offer.amount) },
						onClick = function()
							local terms = { type = offer.type, amount = offer.amount, duration = offer.duration,
								percentage = offer.percentage }
							companyOp(shared, nil, "Borrowing " .. money(offer.amount),
								{ Loan = { Take = { next = terms, offer = terms } } })
						end,
					}
				end
			end
			if #offers > 0 then
				rows[#rows + 1] = builtin.BoxLayout{ orientation = builtin.type.Orientation.Horizontal, children = offers }
			end
		end
		if shared.companyNote then line(shared.companyNote) end
	end

	local function windowRows(status, draft, drafts)
		local shared = ui()
		local function send(text)
			local l = shared.link
			if not l or type(text) ~= "string" or text:match("^%s*$") then return end
			local ok, why = l:say(text)
			if ok then
				draft:set("")
			else
				shared.lines[#shared.lines + 1] = "(not sent: " .. tostring(why) .. ")"
			end
			shared.version = shared.version + 1
		end
		local rows = {}
		local function line(text) rows[#rows + 1] = builtin.TextView{ text = text } end
		line("Room: " .. tostring(status.room))
		if status.speed then line("Speed: " .. speedText(status.speed)) end
		if status.diverged then
			line("Your world differed from the room's at step " .. tostring(status.diverged)
				.. "; the room's is on its way")
		else
			line("Worlds match")
		end
		line("")
		line("Players")
		for _, p in ipairs(status.players or {}) do
			local tags = {}
			if p.owner then tags[#tags + 1] = "host" end
			if p.me then tags[#tags + 1] = "you" end
			if not p.connected then tags[#tags + 1] = "away" end
			line("  " .. tostring(p.name) .. (#tags > 0 and (" (" .. table.concat(tags, ", ") .. ")") or ""))
		end
		companyRows(rows, status, shared, drafts)
		line("")
		line("Chat")
		-- The newest lines only: the window sizes itself to what it holds.
		for i = math.max(1, #shared.lines - CHAT_SHOWN + 1), #shared.lines do line(shared.lines[i]) end
		if #shared.lines == 0 then line("Nobody said anything yet.") end
		rows[#rows + 1] = builtin.BoxLayout{
			orientation = builtin.type.Orientation.Horizontal,
			children = {
				builtin.TextInputField{
					placeholderText = "Say something to the room",
					value = draft:get(),
					maxLength = 280,
					acceptOnFocusLoss = false,
					-- Clicking away keeps what was typed, and a redraw shows
					-- it: Send sends what the field shows, never a line the
					-- field dropped (it emptied itself on a cancel, build
					-- 40408, while Send still had the text).
					resetValueOnCancel = false,
					onTyping = function(text) draft:set(text) end,
					onCancel = function() shared.version = shared.version + 1 end,
					onValueChange = function(text) send(text) end,
				},
				builtin.Button{
					content = builtin.TextView{ text = "Send" },
					onClick = function() send(draft:get()) end,
				},
			},
		}
		return rows
	end

	-- The Multiplayer window, which the game's window container shows, as
	-- the game bar shows its context help (game_bar.tl: the window API's
	-- addSingletonWindow; a window rendered anywhere else shows nothing,
	-- build 40408). One recipe for both plugins, kept in ui(): the game may
	-- run this file once for each.
	local function windowRecipe()
		local shared = ui()
		if shared.window == nil then
			shared.window = react.RegisterWrapperRecipe("Tpf3mpWindow", builtin.Window, function(params)
				local drawn = react.useState(0)
				local draft = react.useRef("")
				local drafts = { rename = react.useRef(""), found = react.useRef("") }
				react.onStep(function()
					local version = ui().version
					if version ~= drawn:old() then drawn:set(version) end
				end)
				local _ = drawn:old()
				local status = ui().status
				local rows
				if status then
					rows = windowRows(status, draft, drafts)
				else
					rows = { builtin.TextView{ text = "Not in a room." } }
				end
				return builtin.Window{
					id = "tpf3mp.multiplayer.window",
					title = "Multiplayer",
					closable = true,
					onClose = params.onClose,
					-- Where it opens, as a share of the screen (the entity
					-- windows open at 1, 0: top right): at the left, below
					-- the mods' buttons, which it would cover at 0, 0.
					initialX = 0,
					initialY = 0.15,
					content = builtin.BoxLayout{ orientation = builtin.type.Orientation.Vertical, children = rows },
				}
			end)
		end
		return shared.window
	end

	-- Opens the Multiplayer window, or closes it if open. A window the game
	-- will not show is said in the game bar and the log.
	local function toggleWindow()
		local shared = ui()
		local ok, why = pcall(function()
			local windows = game_react_globals.getDefaultWindowApi()
			local recipe = windowRecipe()
			local function close()
				shared.open = false
				shared.version = shared.version + 1
				windows.removeAllWindows(recipe)
			end
			if shared.open then
				close()
			else
				shared.open = true
				shared.unread = 0
				shared.version = shared.version + 1
				windows.addSingletonWindow(recipe, { onClose = close })
				windows.moveSingletonWindowToFront(recipe)
			end
		end)
		if not ok then
			shared.open = false
			shared.version = shared.version + 1
			say("the Multiplayer window did not open: " .. tostring(why))
			notice = "The Multiplayer window did not open"
		end
	end

	local Tpf3mpPlugin = react.RegisterPluginRecipe(game_bar_widgets.GameBarInfoDisplayExtension, "Tpf3mpPlugin", function()
		-- Once per game: the ref lives as long as this plugin is mounted.
		local started = react.useRef(false)
		-- The refusal notice shown, or false, and the frames it has left.
		local shown = react.useState(false)
		local frames = react.useRef(0)
		-- The room's version drawn, and the one last seen: a change redraws.
		local room = react.useState(0)
		local seen = react.useRef(0)
		react.onStep(function()
			if not started:get() then
				started:set(true)
				-- A new world's window container has no Multiplayer window.
				ui().open = false
				local ok, err = pcall(start)
				if not ok then say("start failed: " .. tostring(err)) end
			end
			local ok, err = pcall(serve)
			if not ok then say("serving the hook failed: " .. tostring(err)) end
			local followed, why = pcall(follow)
			if not followed then say("reading the room failed: " .. tostring(why)) end
			if ui().version ~= seen:get() then
				seen:set(ui().version)
				room:set(ui().version)
			end
			runPending()
			if link and guardedCmd then
				local delivered, why = pcall(function()
					local results = link:results()
					require("tpf3mp.guard").deliver(guardedCmd, results, sees)
					local shared = ui()
					for _, r in ipairs(results or {}) do
						local doing = shared.asked and r.ticket and shared.asked[r.ticket]
						if doing then
							shared.asked[r.ticket] = nil
							shared.companyNote = r.ok and (doing .. ": done")
								or (doing .. ": not done, " .. tostring(r.why))
							shared.version = shared.version + 1
						end
					end
				end)
				if not delivered then say("answering the player's commands failed: " .. tostring(why)) end
			end
			if notice then
				shown:set(notice)
				frames:set(NOTICE_FRAMES)
				notice = nil
			elseif frames:get() > 0 then
				frames:set(frames:get() - 1)
				if frames:get() == 0 then shown:set(false) end
			end
		end)
		-- Even empty, the layout keeps the plugin mounted, so onStep keeps
		-- running.
		local children = {}
		local _ = room:old()
		local shared = ui()
		if shared.status then
			local label = summary(shared.status)
			if shared.unread > 0 then label = label .. " · " .. shared.unread .. " new" end
			children[#children + 1] = builtin.Button{
				meta = { tooltip = "Open the Multiplayer window" },
				-- The game bar is low: the small font keeps the button in it.
				content = builtin.TextView{ meta = { class = "font-scale-annotation" }, text = label },
				onClick = toggleWindow,
			}
		end
		if shown:old() then
			children[#children + 1] = builtin.TextView{ text = shown:old() }
		end
		return builtin.BoxLayout{
			orientation = builtin.type.Orientation.Horizontal,
			children = children,
		}
	end)

	-- The Multiplayer button in the game's area for mods' buttons, in the
	-- room's game.
	local Tpf3mpButton = react.RegisterPluginRecipe(main_mod_button_area.MainModButtonAreaExtension, "Tpf3mpButton", function()
		local drawn = react.useState(0)
		react.onStep(function()
			local version = ui().version
			if version ~= drawn:old() then drawn:set(version) end
		end)
		local _ = drawn:old()
		local shared = ui()
		local children = {}
		if shared.status then
			children[1] = builtin.Button{
				meta = { tooltip = "Multiplayer: the room, its players and its chat" },
				content = builtin.TextView{
					text = "Multiplayer" .. (shared.unread > 0 and (" (" .. shared.unread .. ")") or ""),
				},
				onClick = toggleWindow,
			}
		end
		return builtin.BoxLayout{ orientation = builtin.type.Orientation.Horizontal, children = children }
	end)

	return {
		Tpf3mpPlugin = Tpf3mpPlugin,
		Tpf3mpButton = Tpf3mpButton,
	}
end
