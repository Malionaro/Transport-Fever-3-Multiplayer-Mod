-- tpf3mp/companies.lua -- the room's companies: which company each player
-- plays for, and the Transport Fever 3 player entity each company is.
--
-- A room starts as one company, the save's own player (company 0), which
-- every player plays for (co-op). A player can found a company of their own
-- (`CompanyOp.Create`), join another (`Join`), rename or recolour the one
-- they play for, and dissolve an empty one: any split, two players in one
-- company and one in another included. Every game applies these at the same
-- update, as every other action of the room's (tpf3mp_sim), so every game
-- keeps the same roster; the mod's game script keeps it in its state, which
-- the game saves with the world, so a player who joins or reloads has it.
--
-- A company is a TF3 player entity (`makeGameAddPlayerCmd(name, colour)`):
-- its money is that entity's ACCOUNT, and what it builds and buys is owned
-- by it (PLAYER_OWNED), as the game keeps ownership. Every game creates it
-- at the same update of the same world, so it is the same entity in every
-- game. What the room orders is booked to the acting player's company
-- (tpf3mp/apply.lua), and what another company owns is refused, the same in
-- every game.
--
-- Who may do what (DECISIONS.md, D22, proposed): a company's players build,
-- buy, run lines, borrow, rename and recolour it; its head (the player who
-- founded it while they play for it, else the one who has played for it
-- longest) alone gives it a password or takes it away, sends a player out of
-- it and opens or closes its stations to other companies' lines. Joining a
-- company with a password needs it: the room seals the password the player
-- typed (tpf3mp_proto::Secret) and every game compares that seal with the
-- one the company keeps, so no game ever holds the password. The room's
-- first company is everyone's: it has no head, no password, and its
-- stations stay open.
--
--   roster = {
--     next = n,                          -- the next company id
--     list = { { id =, entity =, name =, color = { r, g, b }, gone = true?,
--                founder = "<64 hex digits>"?, lock = { scope =, tag = }?,
--                closed = true? }, ... },
--     members = { { player = "<64 hex digits>", company = id }, ... },
--                                        -- in the order they joined
--   }
--
-- Lists of records, not tables keyed by id or player: a save keeps them as
-- they are. The ideas come from TpF2 Multiplayer's companies
-- (tpf2-multiplayer by silver2127, mp/companies.lua: companies as engine
-- players, a palette, create/switch/dissolve at their stamp on every
-- machine) and from TPF2MP's ownership rules (tf2mod: rival assets are
-- refused before they apply); the code is new.
--
-- Pure Lua against the game's `api` and a `send` that runs a command at once
-- and returns its data (tpf3mp/apply.lua); the tests give it fakes.

local companies = {}

-- The most companies a room keeps at once.
companies.MAX = 8

-- The companies' colours, in the order new ones take them: the first no
-- live company has. Plain fractions, as the game's colours.
companies.PALETTE = {
	{ 0.80, 0.16, 0.12 }, -- red
	{ 0.13, 0.42, 0.85 }, -- blue
	{ 0.18, 0.66, 0.27 }, -- green
	{ 0.95, 0.72, 0.08 }, -- yellow
	{ 0.56, 0.27, 0.78 }, -- purple
	{ 0.08, 0.70, 0.72 }, -- teal
	{ 0.95, 0.45, 0.10 }, -- orange
	{ 0.45, 0.45, 0.48 }, -- grey
}

local function copy(color) return { color[1], color[2], color[3] } end

local function vec(api, color) return api.type.Vec3f.new(color[1], color[2], color[3]) end

local function sameColor(a, b)
	return math.abs(a[1] - b[1]) < 1e-3 and math.abs(a[2] - b[2]) < 1e-3 and math.abs(a[3] - b[3]) < 1e-3
end

-- The name the game gives an entity, if it has one.
local function nameOf(api, entity)
	local ok, c = pcall(api.engine.getComponent, entity, api.type.ComponentType.NAME)
	if ok and type(c) == "table" and type(c.name) == "string" and c.name ~= "" then return c.name end
	return nil
end

-- The roster, begun if there is none: company 0, the save's own player,
-- which everyone plays for.
function companies.ensure(roster, api)
	if type(roster) == "table" and type(roster.list) == "table" and type(roster.members) == "table" then
		return roster
	end
	local player = api.engine.util.getPlayer()
	return {
		next = 1,
		list = { { id = 0, entity = player, name = nameOf(api, player) or "Company", color = copy(companies.PALETTE[1]) } },
		members = {},
	}
end

-- The company `id`, gone or not.
function companies.find(roster, id)
	for _, c in ipairs(roster.list) do
		if c.id == id then return c end
	end
	return nil
end

-- The companies still there, in the order they were founded.
function companies.live(roster)
	local out = {}
	for _, c in ipairs(roster.list) do
		if not c.gone then out[#out + 1] = c end
	end
	return out
end

-- The company `player` plays for: the one they joined, else company 0.
function companies.of(roster, player)
	for _, m in ipairs(roster.members) do
		if m.player == player then
			local c = companies.find(roster, m.company)
			if c and not c.gone then return c end
		end
	end
	return companies.find(roster, 0)
end

-- The company whose player entity is `entity`, if any.
function companies.byEntity(roster, entity)
	for _, c in ipairs(roster.list) do
		if c.entity == entity then return c end
	end
	return nil
end

-- The players who play for company `id`.
function companies.members(roster, id)
	local out = {}
	for _, m in ipairs(roster.members) do
		if m.company == id then out[#out + 1] = m.player end
	end
	-- Company 0 also has everyone who never chose; the caller knows who is
	-- in the room, the roster does not.
	return out
end

-- `player` plays for company `id` from now on, last in the join order.
local function setMember(roster, player, id)
	for i, m in ipairs(roster.members) do
		if m.player == player then
			table.remove(roster.members, i)
			break
		end
	end
	roster.members[#roster.members + 1] = { player = player, company = id }
end

-- `player` plays for the room's first company again.
local function leave(roster, player)
	for i, m in ipairs(roster.members) do
		if m.player == player then
			table.remove(roster.members, i)
			return
		end
	end
end

-- The head of company `id`: its founder while they play for it, else the
-- player who has played for it longest; nil for the room's first company,
-- which is everyone's, and for a company nobody plays for.
function companies.head(roster, id)
	if id == 0 then return nil end
	local c = companies.find(roster, id)
	if not c or c.gone then return nil end
	local players = companies.members(roster, id)
	for _, p in ipairs(players) do
		if p == c.founder then return p end
	end
	return players[1]
end

-- The roster in one line, for hook.log: each live company with its id, its
-- head (the first 8 hex digits), how many chose it, and whether it has a
-- password or closed stations. Never a seal.
function companies.describe(roster)
	local out = {}
	for _, c in ipairs(companies.live(roster)) do
		local head = companies.head(roster, c.id)
		local tags = { #companies.members(roster, c.id) .. " chose it" }
		if head then tags[#tags + 1] = "head " .. head:sub(1, 8) end
		if companies.locked(c) then tags[#tags + 1] = "password" end
		if not companies.open(c) then tags[#tags + 1] = "stations closed" end
		out[#out + 1] = tostring(c.name) .. " #" .. c.id .. " (" .. table.concat(tags, ", ") .. ")"
	end
	return table.concat(out, "; ")
end

-- Whether company `c` has a password.
function companies.locked(c)
	return type(c) == "table" and type(c.lock) == "table"
end

-- Whether other companies' lines may stop at company `c`'s stations: yes
-- unless its head closed them.
function companies.open(c)
	return not (type(c) == "table" and c.closed == true)
end

-- Who owns `entity` (its PLAYER_OWNED player), or nil: the game's own, or
-- no one's.
function companies.ownerOf(api, entity)
	if type(entity) ~= "number" or entity < 0 then return nil end
	local ok, c = pcall(api.engine.getComponent, entity, api.type.ComponentType.PLAYER_OWNED)
	if not ok or type(c) ~= "table" then return nil end
	local owner = c.player
	if type(owner) ~= "number" or owner < 0 then return nil end
	return owner
end

-- Whether `company` (a player entity) may change `entity`: what no company
-- owns, and what it owns itself. Else false and why, naming the owner.
function companies.mayTouch(roster, company, entity, api, what)
	local owner = companies.ownerOf(api, entity)
	if owner == nil or owner == company then return true end
	local other = roster and companies.byEntity(roster, owner)
	local name = other and other.name or "another company"
	return false, "the " .. (what or "thing") .. " belongs to " .. name
end

-- Whether `company` (a player entity) may have its lines stop at the
-- station group `group` (D22, proposed): one no company owns, its own, or
-- another company's that keeps its stations open. Else false and why,
-- naming the owner. Stopping at a station changes nothing of it, so it is
-- not `mayTouch`'s.
function companies.mayUse(roster, company, group, api)
	local owner = companies.ownerOf(api, group)
	if owner == nil or owner == company then return true end
	local other = roster and companies.byEntity(roster, owner)
	if other and not companies.open(other) then
		return false, "the station belongs to " .. other.name .. ", which keeps its stations to itself"
	end
	return true
end

-- Whether anything is owned by the player entity `entity`; nil when this
-- game cannot list what players own.
-- The callback is given the entity only (build 40408; TPF2's also had the
-- component), so each one's owner is read, and never raises: an error in the
-- callback ends the game.
function companies.owns(api, entity)
	local owned = {}
	local ok = pcall(api.engine.forEachEntityWithComponent, function(e)
		owned[#owned + 1] = e
	end, api.type.ComponentType.PLAYER_OWNED)
	if not ok then return nil end
	for _, e in ipairs(owned) do
		if companies.ownerOf(api, e) == entity then return true end
	end
	return false
end

local function trimmed(name)
	if type(name) ~= "string" then return nil end
	name = name:gsub("^%s+", ""):gsub("%s+$", "")
	if name == "" then return nil end
	return name
end

local function nameTaken(roster, name, except)
	local lower = name:lower()
	for _, c in ipairs(companies.live(roster)) do
		if c.id ~= except and c.name:lower() == lower then return true end
	end
	return false
end

local function freeColor(roster)
	for _, color in ipairs(companies.PALETTE) do
		local used = false
		for _, c in ipairs(companies.live(roster)) do
			if sameColor(c.color, color) then used = true break end
		end
		if not used then return copy(color) end
	end
	return copy(companies.PALETTE[#companies.PALETTE])
end


local function memberOf(roster, player, id)
	return companies.of(roster, player).id == id
end

-- Refuses unless `player` heads company `c`.
local function headOf(roster, player, c, doing)
	if c.id == 0 then return false, "the room's first company is everyone's: nobody " .. doing .. " it" end
	if companies.head(roster, c.id) ~= player then
		return false, "only the head of " .. c.name .. " " .. doing .. " it"
	end
	return true
end

-- Whether `seal` (the room's, { scope =, tag = }) is a password's for
-- company `id`.
local function sealFor(seal, id)
	return type(seal) == "table" and seal.scope == id and type(seal.tag) == "string" and #seal.tag == 64
end

-- A colour as the action carries it, { r =, g =, b = } in fractions, or nil
-- for one out of range.
local function colorOf(color)
	if type(color) ~= "table" then return nil end
	local out = { color.r, color.g, color.b }
	for k = 1, 3 do
		local v = out[k]
		if type(v) ~= "number" or v ~= v or v < 0 or v > 1 then return nil end
	end
	return out
end

-- The entity a command made, from its data's `field`, else its first result.
local function made(data, entities, field)
	local ok, e = pcall(function() return data[field] end)
	if ok and type(e) == "number" and e >= 0 then return e end
	local first = type(entities) == "table" and entities[1]
	e = type(first) == "table" and first[1] or nil
	if type(e) == "number" and e >= 0 then return e end
	return nil
end

-- ------------------------------------------------------------- colours
--
-- A company's colour is its vehicles': with more than one company in the
-- room, a vehicle it buys is painted in it (tpf3mp/apply.lua), and a new
-- colour repaints all of them, so a glance tells whose a bus is. With one
-- company the game's own colours stay, as in single player.

-- Whether vehicles take their company's colour: more than one company.
function companies.painting(roster)
	return type(roster) == "table" and #companies.live(roster) > 1
end

-- The index of the palette colour `color` is ({ r, g, b }, within what a
-- float keeps of it), or nil for any other colour. A vehicle painted in it
-- has its marker on the map in it too (gui/tpf3mp/tpf3mp.css.lua has a
-- class for each).
function companies.swatch(color)
	if type(color) ~= "table" then return nil end
	for i, p in ipairs(companies.PALETTE) do
		local same = true
		for k = 1, 3 do
			local v = color[k]
			if type(v) ~= "number" then return nil end
			if math.abs(v - p[k]) > 0.01 then same = false end
		end
		if same then return i end
	end
	return nil
end

-- The style class of the markers of vehicles in palette colour `index`.
function companies.markerClass(index)
	return "tpf3mp-company-" .. tostring(index)
end

-- The mod's game script, by the names the game gives it: game scripts are
-- entities, named by their file (the game's loan window finds the loan
-- script so).
companies.SCRIPTS = { "tpf3mp_1::/tpf3mp_sim/tpf3mp_sim.gs", "tpf3mp_1::/tpf3mp_sim.gs" }

-- The mod's game script's state as the game keeps it (the roster is its
-- `companies`), read from any GUI Lua state; nil before there is one.
function companies.scriptState(api)
	for _, name in ipairs(companies.SCRIPTS) do
		local ok, state = pcall(function()
			local entity = api.engine.system.gameScriptSystem.getEntityForGameScript(name)
			if type(entity) ~= "number" or entity < 0 then return nil end
			local c = api.engine.getComponent(entity, api.type.ComponentType.GAME_SCRIPT)
			return c and c.state
		end)
		if ok and type(state) == "table" then return state end
	end
	return nil
end

-- Paints `vehicle` in company `c`'s colour.
function companies.paintVehicle(c, vehicle, send, api)
	local color = c and c.color
	if type(color) ~= "table" or type(vehicle) ~= "number" then return end
	send(api.cmd.makeEntitySetColorCmd(vehicle, vec(api, color)))
end

-- Repaints every vehicle company `c` owns, in the engine's own order (the
-- same entities in the same order in every game). Returns how many.
function companies.paintFleet(c, send, api)
	local vehicles = {}
	pcall(api.engine.forEachEntityWithComponent, function(e)
		local owner = companies.ownerOf(api, e)
		if owner == c.entity then vehicles[#vehicles + 1] = e end
	end, api.type.ComponentType.TRANSPORT_VEHICLE)
	for _, e in ipairs(vehicles) do companies.paintVehicle(c, e, send, api) end
	return #vehicles
end

-- ------------------------------------------------------------- loans
--
-- The game's loan script (::/game_mechanics/finance/loan.gs) keeps the
-- loans of the room's first company only: it books them to the save's own
-- player. Another company's loans are the room's: taken on the same terms
-- the game offers (the loan script's availableLoans), booked to that
-- company as the game books a loan (a LOAN journal entry, which raises the
-- account's balance and its loan alike, seen on build 40408), and paid back
-- month by month as an annuity, the interest booked as INTEREST and the
-- rest as LOAN, until nothing is owed; or all at once.
--
--   roster.loans = { { id =, company =, amount =, remaining =, months =,
--                      paid =, rate = (a month), payment = }, ... }
--   roster.nextLoan = n
--   roster.month = the last month whose payments were booked

-- The month of the game's calendar now, counted from the game's start; nil
-- where the game does not say.
function companies.monthNow(api)
	local ok, month = pcall(function()
		local gt = api.engine.getComponent(api.engine.util.getWorld(), api.type.ComponentType.GAME_TIME)
		local length = api.util.getDefaultMonthDuration()
		if type(length) ~= "number" or length <= 0 or type(gt.gameTime) ~= "number" then return nil end
		return math.floor(gt.gameTime / length), length
	end)
	if ok then return month end
	return nil
end

local function monthLength(api)
	local ok, length = pcall(api.util.getDefaultMonthDuration)
	if ok and type(length) == "number" and length > 0 then return length end
	return nil
end

-- Books `amount` (negative: taken from it) to the company `entity`, as a
-- LOAN or INTEREST entry.
local function book(api, send, entity, amount, kind)
	local entry = api.type.JournalEntry.new()
	entry.amount = amount
	entry.time = -1
	entry.category.type = api.type.JournalEntry.Type[kind]
	send(api.cmd.makeJournalBookAssetCmd(entity, entry))
end

-- The monthly payment that pays `amount` back in `months` at `rate` a month.
local function annuity(amount, rate, months)
	if rate <= 0 then return math.ceil(amount / months) end
	return math.ceil(amount * rate / (1 - (1 + rate) ^ -months))
end

-- The loans of company `id`.
function companies.loansOf(roster, id)
	local out = {}
	for _, loan in ipairs(roster.loans or {}) do
		if loan.company == id then out[#out + 1] = loan end
	end
	return out
end

-- Company `id` takes the loan `terms` (the loan script's own: amount, the
-- duration in the game's milliseconds, the interest a year as a fraction).
function companies.borrow(roster, id, terms, send, api)
	local c = companies.find(roster, id)
	if not c or c.gone then return false, "there is no such company" end
	local amount = type(terms) == "table" and tonumber(terms.amount)
	local duration = type(terms) == "table" and tonumber(terms.duration)
	local percentage = type(terms) == "table" and tonumber(terms.percentage) or 0
	if not amount or amount <= 0 or not duration or duration <= 0 then return false, "a loan needs an amount and a duration" end
	local length = monthLength(api)
	if not length then return false, "this game does not say how long a month is" end
	local months = math.max(1, math.floor(duration / length + 0.5))
	local rate = math.max(0, percentage) / 12
	amount = math.floor(amount)
	book(api, send, c.entity, amount, "LOAN")
	roster.loans = roster.loans or {}
	roster.nextLoan = (roster.nextLoan or 1)
	roster.loans[#roster.loans + 1] = { id = roster.nextLoan, company = id, amount = amount, remaining = amount,
		months = months, paid = 0, rate = rate, payment = annuity(amount, rate, months) }
	roster.nextLoan = roster.nextLoan + 1
	roster.month = roster.month or companies.monthNow(api)
	return true
end

-- Company `id` pays loan `loanId` back, all that is still owed.
function companies.repay(roster, id, loanId, send, api)
	for i, loan in ipairs(roster.loans or {}) do
		if loan.id == loanId and loan.company == id then
			local c = companies.find(roster, id)
			book(api, send, c.entity, -loan.remaining, "LOAN")
			table.remove(roster.loans, i)
			return true
		end
	end
	return false, "the company has no such loan"
end

-- Books every month since the last one booked: each loan's payment, its
-- interest and the part that pays the loan down. Returns how many months.
function companies.chargeMonths(roster, month, send, api)
	if type(month) ~= "number" then return 0 end
	if roster.month == nil then roster.month = month return 0 end
	local months = 0
	while roster.month < month do
		roster.month = roster.month + 1
		months = months + 1
		local keep = {}
		for _, loan in ipairs(roster.loans or {}) do
			local c = companies.find(roster, loan.company)
			local interest = math.floor(loan.remaining * loan.rate + 0.5)
			local principal = math.min(loan.remaining, math.max(0, loan.payment - interest))
			if loan.paid + 1 >= loan.months then principal = loan.remaining end
			if c then
				if interest > 0 then book(api, send, c.entity, -interest, "INTEREST") end
				if principal > 0 then book(api, send, c.entity, -principal, "LOAN") end
			end
			loan.remaining = loan.remaining - principal
			loan.paid = loan.paid + 1
			if loan.remaining > 0 then keep[#keep + 1] = loan end
		end
		roster.loans = keep
	end
	return months
end

-- Whether a month's payments are due: a company owes something and a month
-- has begun since the last booked.
function companies.due(roster, month)
	return type(roster) == "table" and type(month) == "number" and roster.month ~= nil
		and month > roster.month and #(roster.loans or {}) > 0
end

-- Applies one `CompanyOp` for `player`. `send(command)` runs a command at
-- once and returns its data and result entities. `seal` is the room's seal
-- of the password sent with it, { scope =, tag = }, or nil. Returns true, or
-- false and why; the roster changes only when it returns true. A reason
-- never says more of a password than whether it fitted.
function companies.run(roster, player, op, send, api, seal)
	if type(op) ~= "table" then return false, "a company operation is a table" end
	local kind, body = next(op)
	if kind == "Create" then
		local name = trimmed(type(body) == "table" and body.name)
		if name == nil then return false, "a company needs a name" end
		if #companies.live(roster) >= companies.MAX then
			return false, "the room has " .. companies.MAX .. " companies already"
		end
		if nameTaken(roster, name) then return false, "a company is called " .. name .. " already" end
		local color = freeColor(roster)
		local data, entities = send(api.cmd.makeGameAddPlayerCmd(name, vec(api, color)))
		local entity = made(data, entities, "resultEntity")
		if entity == nil then return false, "the game made no company" end
		local id = roster.next
		roster.next = id + 1
		roster.list[#roster.list + 1] = { id = id, entity = entity, name = name, color = color, founder = player }
		setMember(roster, player, id)
		return true, nil, id
	elseif kind == "Join" then
		local c = companies.find(roster, body)
		if c == nil or c.gone then return false, "there is no company " .. tostring(body) end
		if memberOf(roster, player, c.id) then return true, nil, c.id end
		if companies.locked(c) then
			if not sealFor(seal, c.id) then return false, "joining " .. c.name .. " needs its password" end
			if seal.tag ~= c.lock.tag then return false, "the password for " .. c.name .. " is not right" end
		end
		if c.id == 0 then leave(roster, player) else setMember(roster, player, c.id) end
		return true, nil, c.id
	elseif kind == "Rename" then
		local c = type(body) == "table" and companies.find(roster, body.company)
		if not c or c.gone then return false, "there is no such company" end
		if not memberOf(roster, player, c.id) then return false, "only its players rename a company" end
		local name = trimmed(body.name)
		if name == nil then return false, "a company needs a name" end
		if nameTaken(roster, name, c.id) then return false, "a company is called " .. name .. " already" end
		send(api.cmd.makeEntitySetNameCmd(c.entity, name))
		c.name = name
		return true, nil, c.id
	elseif kind == "Recolor" then
		local c = type(body) == "table" and companies.find(roster, body.company)
		if not c or c.gone then return false, "there is no such company" end
		if not memberOf(roster, player, c.id) then return false, "only its players recolour a company" end
		local color = colorOf(body.color)
		if not color then return false, "a colour is { r, g, b }, each from 0 to 1" end
		for _, other in ipairs(companies.live(roster)) do
			if other.id ~= c.id and sameColor(other.color, color) then
				return false, other.name .. " wears that colour already"
			end
		end
		c.color = color
		companies.paintFleet(c, send, api)
		return true, nil, c.id
	elseif kind == "Delete" then
		-- Its last player dissolves it, when it owns nothing, and plays for
		-- the room's first company again (tpf3mp_testkit's regression model
		-- has the same rule).
		local c = companies.find(roster, body)
		if not c or c.gone then return false, "there is no company " .. tostring(body) end
		if c.id == 0 then return false, "the room's first company stays" end
		if not memberOf(roster, player, c.id) then return false, "only its players dissolve a company" end
		if #companies.members(roster, c.id) > 1 then return false, "others still play for " .. c.name end
		local owns = companies.owns(api, c.entity)
		if owns == nil then return false, "this game cannot tell what " .. c.name .. " owns" end
		if owns then return false, c.name .. " still owns something" end
		c.gone = true
		leave(roster, player)
		return true, nil, c.id
	elseif kind == "Lock" then
		-- Its head gives it a password, or a new one: every game keeps the
		-- room's seal of it, never the password.
		local c = companies.find(roster, body)
		if not c or c.gone then return false, "there is no company " .. tostring(body) end
		local ok, why = headOf(roster, player, c, "gives a password to")
		if not ok then return false, why end
		if not sealFor(seal, c.id) then return false, "a password for " .. c.name .. " comes sealed by the room" end
		c.lock = { scope = seal.scope, tag = seal.tag }
		return true, nil, c.id
	elseif kind == "Unlock" then
		local c = companies.find(roster, body)
		if not c or c.gone then return false, "there is no company " .. tostring(body) end
		local ok, why = headOf(roster, player, c, "takes the password from")
		if not ok then return false, why end
		c.lock = nil
		return true, nil, c.id
	elseif kind == "Dismiss" then
		-- Its head sends a player out: they play for the room's first
		-- company again. What they built stays the company's.
		local c = type(body) == "table" and companies.find(roster, body.company)
		if not c or c.gone then return false, "there is no such company" end
		local ok, why = headOf(roster, player, c, "sends players out of")
		if not ok then return false, why end
		if body.player == player then return false, "the head leaves by joining another company" end
		if not memberOf(roster, body.player, c.id) then return false, "that player does not play for " .. c.name end
		leave(roster, body.player)
		return true, nil, c.id
	elseif kind == "ShareStations" then
		local c = type(body) == "table" and companies.find(roster, body.company)
		if not c or c.gone then return false, "there is no such company" end
		if type(body.open) ~= "boolean" then return false, "stations are open or not" end
		local ok, why = headOf(roster, player, c, body.open and "opens the stations of" or "closes the stations of")
		if not ok then return false, why end
		c.closed = (not body.open) or nil
		return true, nil, c.id
	end
	return false, "a company operation of no kind"
end

return companies
