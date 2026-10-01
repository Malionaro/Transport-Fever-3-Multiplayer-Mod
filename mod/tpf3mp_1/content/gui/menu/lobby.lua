-- The Multiplayer window's content, on the game's main menu (docs/LOBBY.md),
-- and the live lines of the menu's Multiplayer cards.
--
-- It shows the lobby the hook hands it and sends the player's choices back,
-- over the hook's request channel: `resolveutil.loadfile("tpf3mp_1::/tpf3mp/state.lua")`
-- answers with the launcher's lobby as a Lua table literal; an action is left as JSON in
-- `resolveutil.__tpf3mp_action` and then `resolveutil.loadfile("tpf3mp_1::/tpf3mp/act.lua")`
-- is called, which the hook answers after taking the JSON (the loader accepts
-- exactly one argument): "ok", or "error: " and why. Both files exist in the
-- mod, because the game checks that before it lets the loader run; the hook
-- answers before the loader reads them, so their contents never matter.
-- Without a hook (a game Steam started) this window is not reachable at all.
--
-- The hook passes everything on to the TPF3-MP launcher that started the
-- game, over its link (D17): the launcher connects, makes and joins rooms and
-- carries the chat; the window shows what the launcher says, a few times a
-- second. Connecting goes to the launcher's own server (D12). The server
-- lists no rooms: a room is joined by the invite its owner sends.
--
-- Plain Lua, loaded by the mod's main_page.tl through `ug_require`; it uses
-- the same react, builtin and helpers the menu does, the menu's own classes
-- (primary and secondary buttons, the font-scale-* sizes, the default style
-- sheet's colours and tapes: success, warning, error, info) and icons, so it
-- looks like the rest of the menu. crates/tpf3mp-hook/src/lobby.rs renders it
-- against a stand-in for the game's GUI (tests/lua/fake_menu.lua) in every
-- state and clicks its buttons.

local react = ug_require "::/gui/main/react.lua"
local builtin = ug_require "::/gui/main/builtin.lua"
local gui_react_util = ug_require "::/gui/main/gui_react_util.tl"
local button_react_util = ug_require "::/gui/main/button_react_util.tl"

local lobby = {}

local ICON = {
	ready = "::/gui/menu/icons/symbol_check.tga",
	host = "::/gui/menu/icons/symbol_crown_laurels.tga",
	away = "::/gui/menu/icons/symbol_x.tga",
	lock = "::/gui/menu/icons/symbol_lock_unlocked.tga",
	player = "::/gui/menu/icons/profile.tga",
	alert = "::/gui/menu/icons/alert.tga",
	kick = "::/gui/menu/icons/trash.tga",
	loading = "::/gui/menu/icons/loading.tga",
	save = "::/gui/menu/icons/load_game.tga",
	multiplayer = "tpf3mp_1::/gui/tpf3mp/icons/menu_multiplayer_50.tga",
}

-- The window's content size and the two columns of each view. The window
-- itself is centred on the menu (main_page.tl's Tpf3mpLobbyWindow).
local WIDTH, HEIGHT = 920, 600
local LEFT, RIGHT = 470, 400
local FIELD = 400
-- Player counts a room can be created for, as the launcher offers them.
local MIN_PLAYERS, MAX_PLAYERS, DEFAULT_PLAYERS = 2, 16, 4
-- How often the window asks the hook for the lobby, in seconds, and how
-- many of those asks an action is shown as under way at most.
local POLL = 0.4
local PENDING_POLLS = 20

-- The request channel -------------------------------------------------------

local function say(line)
	pcall(debugPrint, "[tpf3mp] lobby: " .. line)
end

local function ask(request)
	if type(resolveutil) ~= "table" or type(resolveutil.loadfile) ~= "function" then
		return nil, "no loader"
	end
	local ok, reply = pcall(resolveutil.loadfile, "tpf3mp_1::/tpf3mp/" .. request .. ".lua")
	if not ok then
		return nil, tostring(reply)
	end
	if type(reply) ~= "string" then
		return nil, "no hook answered"
	end
	return reply
end

-- A table literal in an empty environment: Lua 5.2's load, or 5.1's.
local function evaluate(text)
	local chunk, err
	if setfenv then
		chunk, err = loadstring(text, "=tpf3mp-state")
		if chunk then setfenv(chunk, {}) end
	else
		chunk, err = load(text, "=tpf3mp-state", "t", {})
	end
	if not chunk then
		return nil, err
	end
	local ok, value = pcall(chunk)
	if not ok then
		return nil, value
	end
	return value
end

local lastProblem = nil
local function fetchState()
	local reply, why = ask("state")
	if not reply then
		if why ~= lastProblem then
			lastProblem = why
			say("state request failed: " .. tostring(why))
		end
		return nil, why
	end
	local state, err = evaluate("return " .. reply)
	if type(state) ~= "table" then
		return nil, "bad state: " .. tostring(err or state)
	end
	return state
end
lobby.fetchState = fetchState

local function jsonString(text)
	text = tostring(text or "")
	return '"' .. text:gsub('[%c"\\]', function(c)
		if c == '"' then return '\\"' end
		if c == "\\" then return "\\\\" end
		if c == "\n" then return "\\n" end
		if c == "\r" then return "\\r" end
		if c == "\t" then return "\\t" end
		return string.format("\\u%04x", c:byte())
	end) .. '"'
end

-- Sends one action, given as a table with an `action` field and its fields.
-- Returns nil when the hook took it, or why it did not.
local function act(fields)
	local parts = {}
	local keys = {}
	for key in pairs(fields) do keys[#keys + 1] = key end
	table.sort(keys)
	for _i, key in ipairs(keys) do
		local value = fields[key]
		local encoded
		if type(value) == "boolean" then
			encoded = value and "true" or "false"
		elseif type(value) == "number" then
			encoded = string.format("%d", value)
		else
			encoded = jsonString(value)
		end
		parts[#parts + 1] = jsonString(key) .. ":" .. encoded
	end
	local json = "{" .. table.concat(parts, ",") .. "}"
	if type(resolveutil) ~= "table" then
		say("action " .. tostring(fields.action) .. " not sent: no loader")
		return "the game's loader is not there"
	end
	resolveutil.__tpf3mp_action = json
	local reply, why = ask("act")
	resolveutil.__tpf3mp_action = nil
	if not reply then
		say("action " .. tostring(fields.action) .. " not sent: " .. tostring(why))
		return "not sent: " .. tostring(why)
	elseif reply ~= "ok" then
		say("action " .. tostring(fields.action) .. ": " .. reply)
		return (reply:gsub("^error: ", ""))
	end
	return nil
end

-- What the lobby says, in words -----------------------------------------------

local function sizeText(bytes)
	bytes = tonumber(bytes) or 0
	if bytes >= 1e9 then return string.format("%.1f GB", bytes / 1e9) end
	if bytes >= 1e6 then return string.format("%.1f MB", bytes / 1e6) end
	if bytes >= 1e3 then return string.format("%d kB", math.floor(bytes / 1e3 + 0.5)) end
	return string.format("%d B", bytes)
end

-- The server's name as the window shows it: never its address. The
-- launcher names its default server ("EU"); any other is "another server",
-- as a name that looks like host:port or an IP address is.
local function serverName(state)
	local name = state.server
	local elsewhere = state.server_default and state.server_default ~= ""
		and state.server_address and state.server_address ~= state.server_default
	if elsewhere or type(name) ~= "string" or name == "" then
		return elsewhere and _("another server") or _("the TPF3-MP server")
	end
	if name:find(":%d+$") or name:find("^%d+%.%d+%.%d+%.%d+") or name:find("^%[") then
		return _("another server")
	end
	return name
end
lobby.serverName = serverName

-- `text` with any server address in it (an IP address, host:port) put as
-- "the server": the window names servers, never their addresses.
local function hideAddress(text)
	if type(text) ~= "string" then return text end
	text = text:gsub("%[[%x:]+%]:%d+", _("the server"))
	text = text:gsub("%d+%.%d+%.%d+%.%d+:%d+", _("the server"))
	text = text:gsub("%d+%.%d+%.%d+%.%d+", _("the server"))
	text = text:gsub("[%w%-]+%.[%w%.%-]+:%d+", _("the server"))
	text = text:gsub("localhost:%d+", _("the server"))
	return text
end
lobby.hideAddress = hideAddress

-- A room's invite code alone: without its own server, the launcher puts
-- the server's address before the code.
local function inviteCode(invite)
	if type(invite) ~= "string" then return "" end
	return invite:match("(%S+)%s*$") or invite
end

local function you(room)
	for _i, member in ipairs(room and room.members or {}) do
		if member.you then return member end
	end
	return nil
end

local function readyCount(room)
	local count = 0
	for _i, member in ipairs(room.members or {}) do
		if member.ready then count = count + 1 end
	end
	return count
end

local function everyoneReady(room)
	return #(room.members or {}) > 0 and readyCount(room) == #room.members
end

-- The room's world in this game, in words, and how far it is (0 to 1), or
-- nil when there is none of the room's.
local function worldText(state)
	if state.world == "fetching" then
		local total = tonumber(state.total) or 0
		local bytes = tonumber(state.bytes) or 0
		if total > 0 then
			local done = math.min(1, bytes / total)
			return string.format(_("Receiving the room's world: %d%% (%s of %s)"),
				math.floor(done * 100), sizeText(bytes), sizeText(total)), done
		end
		return _("Receiving the room's world..."), 0
	elseif state.world == "loading" then
		return _("Loading the room's world..."), 1
	elseif state.world == "playing" then
		return _("Playing the room's game"), 1
	end
	return nil
end
lobby.worldText = worldText

-- Where the player is, for the Multiplayer cards on the main menu: a line
-- under the card's title.
function lobby.summary(state)
	if not state then
		return _("Play together online")
	end
	if not state.linked then
		return _("Start the game from the TPF3-MP launcher")
	end
	if state.connection == "connecting" then
		return _("Connecting...")
	end
	if state.connection ~= "connected" then
		return _("Play together online")
	end
	local room = state.room
	if not room then
		return string.format(_("Online on %s"), serverName(state))
	end
	local text = worldText(state)
	if text then return text end
	return string.format(_("%s · %d/%d players · %d ready"), room.name, #room.members,
		room.max_players, readyCount(room))
end

-- Styling ---------------------------------------------------------------------

-- An inline style sheet: size as {w, h}, padding as {top, right, bottom,
-- left}. Those are what api.gui.StyleSheet offers (no margin, no minSize), so
-- spacing between elements is done with gap() below. A size of -1 leaves
-- that side to the content, as the game's style sheets write it; a size of
-- 0 is a size of nothing, and hides everything inside (the game drew the
-- create and join columns, sized {w, 0}, as two thin bars).
local AUTO = -1
local function style(t)
	local s = api.gui.StyleSheet.new()
	if t.size then s.size = api.type.Vec2f.new(t.size[1], t.size[2]) end
	if t.padding then s.padding = api.type.Vec4f.new(t.padding[1], t.padding[2], t.padding[3], t.padding[4]) end
	return s
end

local function gap(px)
	px = math.max(1, px or 12)
	return builtin.Component{
		meta = { styleSheet = style{ size = { px, px } } },
		mouseTransparent = true,
		layout = builtin.BoxLayout{ children = {} },
	}
end

-- A row or column's children with a gap between each pair.
local function spaced(children, px)
	local out = {}
	for i, child in ipairs(children) do
		if i > 1 then out[#out + 1] = gap(px or 8) end
		out[#out + 1] = child
	end
	return out
end

local function label(text, class, sheet)
	return builtin.TextView{
		meta = { class = class or "font-scale-body", styleSheet = sheet },
		text = text or "",
	}
end

-- A smaller, quieter line, as the menu's cards write their descriptions.
local function note(text, class)
	return label(text, "font-scale-annotation" .. (class and (", " .. class) or ""))
end

-- A word on a coloured tape, as the game marks states: success, warning,
-- error or info.
local function badge(text, tone)
	return label(" " .. text .. " ", "font-scale-annotation, " .. (tone or "info") .. "-tape")
end

local function icon(path, px)
	px = px or 20
	return builtin.ImageView{
		meta = { styleSheet = style{ size = { px, px } } },
		path = path,
		scaling = builtin.type.ImageViewScaling.AutoFit,
	}
end

local function row(children, sheet)
	return builtin.Component{
		meta = { styleSheet = sheet },
		mouseTransparent = true,
		layout = builtin.BoxLayout{
			orientation = builtin.type.Orientation.Horizontal,
			children = children,
		},
	}
end

local function column(children, sheet)
	return builtin.Component{
		meta = { styleSheet = sheet },
		mouseTransparent = true,
		layout = builtin.BoxLayout{
			orientation = builtin.type.Orientation.Vertical,
			children = children,
		},
	}
end

local function button(text, onClick, class, enabled, tooltip)
	return builtin.Button{
		meta = {
			class = class or "secondary",
			enabled = enabled ~= false,
			tooltip = tooltip,
		},
		content = builtin.TextView{ meta = { class = "font-scale-body" }, text = text },
		onClick = onClick,
	}
end

local function primary(text, onClick, enabled, tooltip)
	return button(text, onClick, "primary", enabled, tooltip)
end

local function input(ref, placeholder, width, params)
	params = params or {}
	return builtin.TextInputField{
		meta = { class = "font-scale-body", styleSheet = style{ size = { width or FIELD, 36 } } },
		value = ref:get(),
		placeholderText = placeholder,
		passwordMode = params.password or false,
		maxLength = params.maxLength,
		acceptOnFocusLoss = params.acceptOnFocusLoss ~= false,
		resetValueOnCancel = false,
		onValueChange = function(value)
			ref:set(value)
			if params.onEnter then params.onEnter(value) end
		end,
		onTyping = function(value) ref:set(value) end,
	}
end

local function field(caption, ref, placeholder, params)
	return column({
		note(caption),
		gap(4),
		input(ref, placeholder, FIELD, params),
		gap(12),
	})
end

-- A choice from a list: `items` as { value, text }.
local function choice(caption, value, items, onChange, explain)
	local entries = {}
	for _i, item in ipairs(items) do
		entries[#entries + 1] = builtin.ComboBoxItem{
			value = item[1],
			content = builtin.TextView{ meta = { class = "font-scale-body" }, text = item[2] },
		}
	end
	local children = {
		note(caption),
		gap(4),
		builtin.Component{
			meta = { styleSheet = style{ size = { FIELD, 36 } } },
			layout = builtin.BoxLayout{
				children = {
					builtin.ComboBox{ value = value, items = entries, onValueChange = onChange },
				},
			},
		},
	}
	if explain then
		children[#children + 1] = gap(4)
		children[#children + 1] = note(explain)
	end
	children[#children + 1] = gap(12)
	return column(children)
end

local function heading(text, sub)
	local children = { label(text, "font-scale-title-3") }
	if sub then
		children[#children + 1] = gap(2)
		children[#children + 1] = note(sub)
	end
	children[#children + 1] = gap(12)
	return column(children)
end

-- The room browser --------------------------------------------------------------

-- Cards a row of the room list holds, and a card's size: the game's
-- new-game climate cards (`small-rectangle-card`, 316 by 181) a little
-- smaller, three to the window's width.
local CARDS_PER_ROW = 3
local CARD_WIDTH, CARD_HEIGHT = 272, 156
-- The first page's two cards, Join and Host.
local CHOICE_WIDTH, CHOICE_HEIGHT = 420, 240
-- Polls between two asks for the room list while it is shown.
local LIST_POLLS = 25

-- The game's own card button and label, as its main menu builds them; nil
-- if the game has none, and a plain button with a picture is drawn instead.
local cards = (function()
	local ok, util = pcall(ug_require, "::/gui/menu/menu_icon_react_util.tl")
	if ok and type(util) == "table" and util.CardButton and util.makeCardLabelBottomComponent then
		return util
	end
	return nil
end)()

-- The game's pictures of each climate on its main menu's New Game card,
-- for a climate whose own description has no picture.
local CLIMATE_PICTURES = {
	temperate = "::/gui/menu/images/temperate_ingame.tga",
	subarctic = "::/gui/menu/images/subarctic_ingame.tga",
	tropical = "::/gui/menu/images/tropical_ingame.tga",
	dry = "::/gui/menu/images/dry_ingame.tga",
}
local UNKNOWN_PICTURE = "::/gui/menu/images/m05_ingame.tga"

-- The game's description of the climate `map` names (`temperate`), if it
-- has one: what its New Game page shows, its name and its picture.
local function climate(map)
	if type(map) ~= "string" or map == "" then return nil end
	local ok, desc = pcall(function()
		local rep = app.res.climateRep
		local id = rep.find("::/climates/" .. map .. "/" .. map .. ".clima")
		if id == nil or id < 0 then return nil end
		return rep.get(id).desc
	end)
	return ok and desc or nil
end

-- The climate's name, as players read it.
function lobby.climateName(map)
	local desc = climate(map)
	if desc and type(desc.name) == "string" and desc.name ~= "" then return desc.name end
	if type(map) ~= "string" or map == "" then return _("Unknown map") end
	return (map:gsub("^%l", string.upper))
end

-- The picture of the climate `map` names.
function lobby.climatePicture(map)
	local desc = climate(map)
	if desc and type(desc.icon) == "string" and desc.icon ~= "" then return desc.icon end
	return CLIMATE_PICTURES[map] or UNKNOWN_PICTURE
end

-- A save's climate and year, as the game's Load Game page reads them
-- (savegame_react_util.tl): the save's configDict "climate" and its
-- metadata's date. Read once, in the background (app.getSavegameInfo);
-- until then, or when the game cannot say, the map is "" and the year 0.
local saveDetailsRead = {}
local function yearOf(metadata)
	local ok, year = pcall(function() return api.type.Date.new(metadata.date).year end)
	if ok and type(year) == "number" and year > 1000 and year < 3000 then return math.floor(year) end
	if type(metadata.startYear) == "number" and metadata.startYear > 1000 then return math.floor(metadata.startYear) end
	return 0
end
function lobby.saveDetails(name)
	local read = saveDetailsRead[name]
	if not read then
		read = { map = "", year = 0 }
		saveDetailsRead[name] = read
		local ok, async = pcall(function()
			local namespace = app.SaveGameNamespace.getSavegame()
			for _i, info in ipairs(app.findAllSavegames(namespace) or {}) do
				if info.saveName == name or info.saveName == name .. ".sav" then
					local id = api.type.SavegameId.new()
					id.path = info.path
					id.saveGameName = info.saveName
					id.saveGameNamespace = namespace
					return app.getSavegameInfo(id)
				end
			end
			return nil
		end)
		read.async = ok and async or nil
		if not ok then say("the save " .. tostring(name) .. " could not be read: " .. tostring(async)) end
	end
	if read.async then
		local ok, done = pcall(function() return read.async:isCompleted() end)
		if ok and done then
			local got, data = pcall(function() return read.async:get() end)
			read.async = nil
			if got and data and data.info then
				for _i, pair in ipairs(data.info.configDict or {}) do
					if pair[1] == "climate" and type(pair[2]) == "string" then
						read.map = pair[2]:match("([%w_]+)%.clima$") or pair[2]
					end
				end
				if data.info.metadata then read.year = yearOf(data.info.metadata) end
			end
		elseif not ok then
			read.async = nil
		end
	end
	return read
end

-- Banners ---------------------------------------------------------------------

-- The pictures players show in rooms: the server's set of banner ids
-- (tpf3mp_proto::BANNERS, in its order), each one of the game's own
-- pictures: its campaign's and climates' menu cards, the map editor's and
-- the mods', the main menu's and its loading screens.
local BANNERS = {
	{ "m01", "::/gui/menu/images/m01_ingame.tga" },
	{ "m02", "::/gui/menu/images/m02_ingame.tga" },
	{ "m03", "::/gui/menu/images/m03_ingame.tga" },
	{ "m04", "::/gui/menu/images/m04_ingame.tga" },
	{ "m05", "::/gui/menu/images/m05_ingame.tga" },
	{ "m06", "::/gui/menu/images/m06_ingame.tga" },
	{ "m07", "::/gui/menu/images/m07_ingame.tga" },
	{ "m08", "::/gui/menu/images/m08_ingame.tga" },
	{ "temperate", "::/gui/menu/images/temperate_ingame.tga" },
	{ "subarctic", "::/gui/menu/images/subarctic_ingame.tga" },
	{ "tropical", "::/gui/menu/images/tropical_ingame.tga" },
	{ "dry", "::/gui/menu/images/dry_ingame.tga" },
	{ "mapeditor", "::/gui/menu/images/mapeditor_ingame.tga" },
	{ "mapeditor2", "::/gui/menu/images/mapeditor_ingame_2.tga" },
	{ "mod01", "::/gui/menu/images/mod01_ingame.tga" },
	{ "mod02", "::/gui/menu/images/mod02_ingame.tga" },
	{ "main", "::/gui/menu/images/main.tga" },
	{ "loadgame", "::/gui/menu/images/loadgame.tga" },
	{ "loading1", "::/gui/menu/images/loading_background_1.tga" },
	{ "loading2", "::/gui/menu/images/loading_background_2.tga" },
	{ "loading3", "::/gui/menu/images/loading_background_3.tga" },
	{ "loading4", "::/gui/menu/images/loading_background_4.tga" },
}
lobby.BANNERS = BANNERS
local BANNER_PATH = {}
for _i, banner in ipairs(BANNERS) do BANNER_PATH[banner[1]] = banner[2] end

-- The banner a player shows: the one they picked, or one chosen from their
-- key (its first eight hex digits, modulo the set), the same in every
-- player's game.
function lobby.bannerOf(member)
	if member.banner and BANNER_PATH[member.banner] then return member.banner end
	local n = tonumber(tostring(member.id or ""):sub(1, 8), 16) or 0
	return BANNERS[(n % #BANNERS) + 1][1]
end
function lobby.bannerPicture(id)
	return BANNER_PATH[id] or BANNERS[1][2]
end

-- A room member's size as a card, two to a row of the players' column.
local MEMBER_WIDTH, MEMBER_HEIGHT = 228, 128

-- A picture card in the main menu's style: title and a line under it, a
-- word on the right; `onClick` nil for a card that only shows.
local function pictureCard(picture, title, line, right, onClick, enabled, width, height, marks)
	local card
	if cards then
		card = cards.CardButton{
			bottomComponent = cards.makeCardLabelBottomComponent(title, line, right, nil, false),
			onClick = onClick or function() end,
			tooltip = title,
			images = { picture },
			initialImageIndex = 1,
			class = "small-rectangle-card",
			enabled = enabled ~= false,
			extraChildren = marks or {},
		}
	else
		card = builtin.Button{
			meta = { enabled = enabled ~= false },
			content = column({ icon(picture, height - 60), label(title, "font-scale-body"), note(line or "") }),
			onClick = onClick or function() end,
		}
	end
	return builtin.Component{
		meta = { styleSheet = style{ size = { width, height } } },
		layout = builtin.BoxLayout{ children = { card } },
	}
end
lobby.pictureCard = pictureCard

-- A room member as a card: their banner, name, and what marks them.
function lobby.memberCard(member, playing)
	local marks = {}
	if member.owner then marks[#marks + 1] = _("Owner") end
	if not member.connected then marks[#marks + 1] = _("Away") end
	if not playing then marks[#marks + 1] = member.ready and _("Ready") or _("Not ready") end
	if member.content == "differs" then marks[#marks + 1] = _("Other mods") end
	local ready = member.ready and not playing and builtin.FloatingLayoutChild{
		h = 0.95,
		v = 0.06,
		item = builtin.ImageView{
			meta = { mouseTransparent = true, styleSheet = style{ size = { 22, 22 } } },
			path = ICON.ready,
		},
	} or nil
	return pictureCard(lobby.bannerPicture(lobby.bannerOf(member)), member.name,
		table.concat(marks, " · "), member.you and _("You") or nil, nil, true,
		MEMBER_WIDTH, MEMBER_HEIGHT, ready and { ready } or {})
end

-- The pictures of the Host page's play styles. Co-op: the busy harbour of
-- the game's Campaign card, many vessels sharing one port. Competitive: the
-- rusted-out truck left in the desert on the loading screen of the
-- campaign's third mission, a built-in mod of the game; its path is the
-- mod's own (INFERRED to load at the main menu as the campaign's pictures
-- do).
local COOP_PICTURE = "::/gui/menu/images/campaign.tga"
local COMPETITIVE_PICTURE = "urbangames_campaign_mission_03::/gui/mission/m03_loadscreen.tga"
lobby.COOP_PICTURE = COOP_PICTURE
lobby.COMPETITIVE_PICTURE = COMPETITIVE_PICTURE

-- One play style as a card: picked, it says so.
function lobby.styleCard(competitive, picked, onClick, enabled)
	local title = competitive and _("Competitive") or _("Co-op")
	return pictureCard(competitive and COMPETITIVE_PICTURE or COOP_PICTURE,
		picked and ("> " .. title) or title,
		picked and _("Picked") or nil, nil, onClick, enabled, 190, 104)
end

-- A big choice of the first page (Join, Host), as a card in the main
-- menu's style.
function lobby.choiceCard(title, line, picture, onClick, enabled)
	local card
	if cards then
		card = cards.CardButton{
			bottomComponent = cards.makeCardLabelBottomComponent(title, line, nil, nil, true),
			onClick = onClick,
			tooltip = line,
			images = { picture },
			initialImageIndex = 1,
			class = "small-rectangle-card",
			enabled = enabled,
			extraChildren = {},
		}
	else
		card = builtin.Button{
			meta = { enabled = enabled },
			content = column({ icon(picture, CHOICE_HEIGHT - 70), label(title, "font-scale-title-3"), note(line) }),
			onClick = onClick,
		}
	end
	return builtin.Component{
		meta = { styleSheet = style{ size = { CHOICE_WIDTH, CHOICE_HEIGHT } } },
		layout = builtin.BoxLayout{ children = { card } },
	}
end

-- One public room of the list, as a card in the game's own style: the
-- picture of its map, its name, and players, companies and year under it.
function lobby.roomCard(listed, onClick, enabled)
	local title = listed.name
	local line = string.format(_("%d/%d players · %d %s · %s"), listed.players, listed.max_players,
		listed.companies, listed.companies == 1 and _("company") or _("companies"),
		listed.year > 0 and tostring(listed.year) or _("year unknown"))
	local right = (listed.competitive and _("Competitive") or _("Co-op")) .. " · "
		.. (listed.running and _("Playing") or lobby.climateName(listed.map))
	local lock = listed.has_password and builtin.FloatingLayoutChild{
		h = 0.95,
		v = 0.06,
		item = builtin.ImageView{
			meta = { mouseTransparent = true, styleSheet = style{ size = { 24, 24 } } },
			path = ICON.lock,
		},
	} or nil
	local card
	if cards then
		card = cards.CardButton{
			bottomComponent = cards.makeCardLabelBottomComponent(title, line, right, nil, false),
			onClick = onClick,
			tooltip = listed.has_password and _("Has a password") or _("Join this room"),
			images = { lobby.climatePicture(listed.map) },
			initialImageIndex = 1,
			class = "small-rectangle-card",
			enabled = enabled,
			extraChildren = lock and { lock } or {},
		}
	else
		card = builtin.Button{
			meta = { enabled = enabled },
			content = column({
				icon(lobby.climatePicture(listed.map), CARD_HEIGHT - 60),
				label(title, "font-scale-body"),
				note(line),
				note(right),
			}),
			onClick = onClick,
		}
	end
	return builtin.Component{
		meta = { styleSheet = style{ size = { CARD_WIDTH, CARD_HEIGHT } } },
		layout = builtin.BoxLayout{ children = { card } },
	}
end

-- The window's content, rendered inside the Tpf3mpLobbyWindow recipe. ------

-- `focus` is what the card that opened the window is about: "join" puts
-- the invite first.
function lobby.content(onClose, focus)
	local stateS = react.useState(nil)
	local problemS = react.useState(nil)
	-- An action on its way: { text, polls left, the state it was sent in }.
	local pendingS = react.useState(nil)
	-- Why the hook refused the last action, until the next one.
	local refusedS = react.useState(nil)
	-- A question before kicking or leaving: { kind, id, name }.
	local confirmS = react.useState(nil)
	local name = react.useRef("")
	local roomName = react.useRef("")
	local invite = react.useRef("")
	local createPassword = react.useRef("")
	local joinPassword = react.useRef("")
	local chatText = react.useRef("")
	local playersS = react.useState(DEFAULT_PLAYERS)
	local rulesS = react.useState(nil)
	local saveS = react.useState(nil)
	-- The page shown: "choose" (Join or Host), "join" (the public rooms
	-- and an invite) or "host" (the room's settings); in a room, always the
	-- room's. Your mods show over it while modsS is on.
	local pageS = react.useState(nil)
	local modsS = react.useState(false)
	-- The banner picker, from the first page.
	local bannerS = react.useState(false)
	-- The Join page's Join with code popup.
	local codeS = react.useState(focus == "code")
	-- The server page, from the first page: whether it shows, the address
	-- typed, and why the launcher refused the last one.
	local serverS = react.useState(false)
	local serverText = react.useRef(nil)
	local serverErrorS = react.useState(nil)
	local publicS = react.useState("private")
	-- The Host page's play style: co-op (false) or competitive.
	local competitiveS = react.useState(false)
	local joiningS = react.useState(nil)
	local listAtRef = react.useRef(LIST_POLLS)

	-- What the view shows, in one string: when it changes, an action sent
	-- has been answered.
	local function signature(state)
		if not state then return "" end
		local room = state.room
		local me = you(room)
		return table.concat({
			tostring(state.connection), tostring(room and room.name), tostring(room and room.phase),
			tostring(room and #room.members), tostring(me and me.ready), tostring(state.error),
			tostring(state.notice), tostring(#(state.chat or {})), tostring(state.server_address),
		}, "|")
	end

	-- The page `s` shows: the room's once in one; the first page until
	-- connected; otherwise the one picked, or the one the card that opened
	-- the window is about.
	local function pageOf(s)
		if s.room then return "room" end
		if s.connection ~= "connected" then return "choose" end
		local picked = pageS:old() or (focus == "join" and "join" or "choose")
		if picked == "room" then return "choose" end
		return picked
	end

	-- Poll the hook for the lobby a few times a second: the room and chat
	-- change without anything happening in this window.
	react.onStepTimer(function()
		local state, why = fetchState()
		if state then
			if problemS:old() ~= nil then problemS:set(nil) end
			local pending = pendingS:old()
			if pending then
				if pending[3] ~= signature(state) or pending[2] <= 1 then
					pendingS:set(nil)
				else
					pendingS:set({ pending[1], pending[2] - 1, pending[3] })
				end
			end
			stateS:set(state)
			-- The room list, while it is shown: asked for at once, then
			-- every LIST_POLLS polls (the server allows one a second).
			local browsing = state.linked and pageOf(state) == "join" and not modsS:old()
			if browsing then
				listAtRef:set(listAtRef:get() + 1)
				if state.rooms == nil and listAtRef:get() >= 3 or listAtRef:get() >= LIST_POLLS then
					listAtRef:set(0)
					act({ action = "list_rooms", page = state.rooms and state.rooms.page or 0 })
				end
			end
		elseif problemS:old() ~= why then
			problemS:set(why)
		end
	end, POLL, false)

	local state = stateS:old()

	-- Sends an action, and shows `doing` until the launcher answers.
	local function send(fields, doing)
		confirmS:set(nil)
		local refused = act(fields)
		refusedS:set(refused)
		if not refused and doing then
			pendingS:set({ doing, PENDING_POLLS, signature(stateS:old()) })
		end
	end

	local busy = pendingS:old() ~= nil

	-- The frame every view shares: the title and the connection, the steps,
	-- a line for what went wrong, what is under way or what just happened,
	-- the room's world when it is coming, the view, and a footer with the
	-- view's buttons.
	local function frame(title, status, body, footer)
		local children = {
			row({
				icon(ICON.multiplayer, 28),
				gap(10),
				label(title, "font-scale-title-3"),
				gui_react_util.makeHorizontalSpacer(),
				status,
			}),
			gap(10),
		}
		local problem = hideAddress(refusedS:old() or (state and state.error))
		if problem then
			children[#children + 1] = row({ icon(ICON.alert, 18), gap(6), label(problem, "font-scale-body, error") })
		elseif pendingS:old() then
			children[#children + 1] = row({ icon(ICON.loading, 18), gap(6), label(pendingS:old()[1], "font-scale-body, info") })
		elseif state and state.notice then
			children[#children + 1] = note(hideAddress(state.notice))
		else
			children[#children + 1] = gap(18)
		end
		if state and not state.linked then
			children[#children + 1] = label(
				_("This game has no link to the TPF3-MP launcher: close it and start Transport Fever 3 from the launcher."),
				"font-scale-body, error")
		elseif state and not state.heard then
			children[#children + 1] = note(_("Waiting for the launcher..."))
		end
		local world, done = state and worldText(state)
		if world then
			children[#children + 1] = gap(8)
			children[#children + 1] = row({
				label(world, "font-scale-body, info"),
				gap(12),
				builtin.Component{
					meta = { styleSheet = style{ size = { 260, 18 } } },
					layout = builtin.BoxLayout{ children = { builtin.ProgressBar{ value = done } } },
				},
			})
		end
		if state and state.differences then
			children[#children + 1] = gap(4)
			children[#children + 1] = label(_("Your game differs from the room's: ") .. state.differences,
				"font-scale-body, warning")
		end
		children[#children + 1] = gap(14)
		children[#children + 1] = body
		children[#children + 1] = gui_react_util.makeVerticalSpacer()
		children[#children + 1] = row(spaced(footer))
		return column(children, style{ size = { WIDTH, HEIGHT }, padding = { 16, 20, 16, 20 } })
	end

	-- No answer from the hook yet.
	if not state then
		local why = problemS:old()
		return frame(
			_("Multiplayer"),
			note(_("Waiting for the hook...")),
			label(why and (_("The hook did not answer: ") .. tostring(why)) or "", "font-scale-body, error"),
			{ gui_react_util.makeHorizontalSpacer(), button(_("Close"), onClose) }
		)
	end

	local canAct = state.linked and state.heard

	local connected = state.connection == "connected"
	local room = state.room
	local page = pageOf(state)
	local disconnect = function() send({ action = "disconnect" }, _("Disconnecting...")) end

	-- Where the player is, top right: online as whom, on which server.
	local status
	if connected then
		status = row({
			badge(_("Online"), "success"),
			gap(8),
			icon(ICON.player, 18),
			gap(4),
			label(tostring(state.name), "font-scale-body"),
			note("  @ " .. serverName(state)),
		})
	elseif state.connection == "connecting" then
		status = badge(_("Connecting"), "info")
	else
		status = badge(_("Not connected"), "warning")
	end

	local function back(to)
		return button(_("Back"), function()
			joiningS:set(nil)
			pageS:set(to)
		end)
	end
	local function modsButton()
		local chosen = 0
		for _i, m in ipairs(state.mods or {}) do
			if m.chosen and m.choosable then chosen = chosen + 1 end
		end
		return button(string.format(_("Your mods (%d chosen)"), chosen), function() modsS:set(true) end, nil,
			#(state.mods or {}) > 0 or #(state.room_mods or {}) > 0)
	end

	-- Your mods: over whichever page opened it, with Back to it.
	if modsS:old() and page ~= "choose" then
		local playing = room and room.phase == "playing"
		local rows = {}
		for _i, m in ipairs(state.mods or {}) do
			local tone = (m.class == "shared" and "info") or (m.class == "carried" and "warning") or "success"
			local cells = {}
			if m.choosable then
				cells[#cells + 1] = button(m.chosen and _("On") or _("Off"), function()
					send({ action = "choose_mod", id = m.id, chosen = not m.chosen }, nil)
				end, m.chosen and "primary" or "secondary", canAct and not playing, m.reason)
			else
				cells[#cells + 1] = button(_("Needed"), function() end, "secondary", false, m.reason)
			end
			cells[#cells + 1] = gap(10)
			cells[#cells + 1] = label(m.name, m.choosable and "font-scale-body" or "font-scale-body, info")
			cells[#cells + 1] = gap(8)
			cells[#cells + 1] = badge(m.class == "shared" and _("every player needs it")
				or m.class == "carried" and _("carried by the room") or _("only you see it"), tone)
			rows[#rows + 1] = row(cells)
			rows[#rows + 1] = gap(6)
		end
		if #rows == 0 then rows[1] = note(_("No mods installed besides the room's.")) end
		local needs = {}
		for _i, m in ipairs(state.room_mods or {}) do
			local have = (m.have == "yes" and badge(_("You have it"), "success"))
				or (m.have == "other_version" and badge(_("Another version"), "warning"))
				or badge(_("You lack it"), "error")
			needs[#needs + 1] = row({ label(m.id, "font-scale-body"), gap(6),
				note(m.version ~= "" and ("v" .. m.version) or ""), gap(10), have })
			needs[#needs + 1] = gap(4)
		end
		if (state.room_mods_more or 0) > 0 then
			needs[#needs + 1] = note(string.format(_("and %d more"), state.room_mods_more))
		end
		local children = {
			row({ button(_("Back"), function() modsS:set(false) end), gap(16),
				heading(_("Your mods"), playing and _("The room's game has started: your choice holds for its next world.")
					or _("Turn on the mods only you play with; the room's own every player needs.")) }),
			builtin.ScrollArea{
				meta = { styleSheet = style{ size = { WIDTH - 40, #needs > 0 and 200 or HEIGHT - 220 } } },
				horizontalPolicy = builtin.type.ScrollBarPolicy.AlwaysOff,
				verticalPolicy = builtin.type.ScrollBarPolicy.AsNeeded,
				content = column(rows),
			},
		}
		if #needs > 0 then
			children[#children + 1] = gap(10)
			children[#children + 1] = heading(_("The room's mods"), _("Every player needs these, from the room's start save."))
			children[#children + 1] = builtin.ScrollArea{
				meta = { styleSheet = style{ size = { WIDTH - 40, 140 } } },
				horizontalPolicy = builtin.type.ScrollBarPolicy.AlwaysOff,
				verticalPolicy = builtin.type.ScrollBarPolicy.AsNeeded,
				content = column(needs),
			}
		end
		return frame(_("Your mods"), status, column(children),
			{ gui_react_util.makeHorizontalSpacer(), button(_("Close"), onClose) })
	end

	-- Your banner, from the first page: the picture the others see on your
	-- card in a room. A click picks one; Default goes back to the one your
	-- key gives.
	if bannerS:old() and page == "choose" then
		local rows, cellsRow = {}, {}
		for _i, banner in ipairs(BANNERS) do
			local picked = state.banner == banner[1]
			if #cellsRow > 0 then cellsRow[#cellsRow + 1] = gap(10) end
			cellsRow[#cellsRow + 1] = pictureCard(banner[2], picked and _("Yours") or " ", nil, nil, function()
				send({ action = "set_banner", banner = banner[1] }, nil)
			end, canAct, 196, 110)
			if #cellsRow >= 7 then
				rows[#rows + 1] = row(cellsRow)
				rows[#rows + 1] = gap(10)
				cellsRow = {}
			end
		end
		if #cellsRow > 0 then rows[#rows + 1] = row(cellsRow) end
		return frame(_("Your banner"), status, column({
			row({
				button(_("Back"), function() bannerS:set(false) end),
				gap(16),
				heading(_("Your banner"), _("The picture the others see on your card in a room.")),
				gui_react_util.makeHorizontalSpacer(),
				button(_("Default"), function() send({ action = "set_banner", banner = "" }, nil) end, nil,
					canAct and state.banner ~= nil and state.banner ~= ""),
			}),
			builtin.ScrollArea{
				meta = { styleSheet = style{ size = { WIDTH - 40, HEIGHT - 220 } } },
				horizontalPolicy = builtin.type.ScrollBarPolicy.AlwaysOff,
				verticalPolicy = builtin.type.ScrollBarPolicy.AsNeeded,
				content = column(rows),
			},
		}), { gui_react_util.makeHorizontalSpacer(), button(_("Close"), onClose) })
	end

	-- The server this launcher plays on, from the first page: shown,
	-- changed, or put back to the launcher's own. Not while in a room.
	if serverS:old() and page == "choose" then
		local onDefault = state.server_address == state.server_default
		if serverText:get() == nil then serverText:set(state.server_address or "") end
		local usable = canAct and not room and not busy
		local function use(address)
			local refused = act({ action = "set_server", server = address })
			serverErrorS:set(refused)
			refusedS:set(nil)
			if not refused then
				serverText:set(address ~= "" and address or nil)
				pendingS:set({ _("Changing the server..."), PENDING_POLLS, signature(stateS:old()) })
			end
		end
		-- The launcher's own errors show at the top, as on every page.
		local problem = serverErrorS:old()
		local children = {
			row({
				button(_("Back"), function()
					serverS:set(false)
					serverErrorS:set(nil)
					serverText:set(nil)
				end),
				gap(16),
				heading(_("Server"), string.format(_("Now: %s%s"), serverName(state),
					onDefault and _(" (default)") or "")),
			}),
			field(_("Server address (host:port)"), serverText, state.server_default ~= "" and state.server_default or "host:port",
				{ maxLength = 128, onEnter = function(value) if usable then use(value) end end }),
			problem and label(problem, "font-scale-body, error") or gap(1),
			gap(8),
		}
		local buttons = {
			primary(_("Use this server"), function() use(serverText:get() or "") end, usable),
		}
		if not onDefault then
			buttons[#buttons + 1] = gap(8)
			buttons[#buttons + 1] = button(_("Reset to default"), function() use("") end, nil, usable)
		end
		children[#children + 1] = row(buttons)
		children[#children + 1] = gap(12)
		children[#children + 1] = note(_("Changing the server disconnects you and connects to the new one. Invites only join rooms on your own server."))
		return frame(_("Server"), status, column(children, style{ size = { LEFT + 100, AUTO } }),
			{ gui_react_util.makeHorizontalSpacer(), button(_("Close"), onClose) })
	end

	-- The first page: connect, then Join or Host.
	if page == "choose" then
		local connecting = state.connection == "connecting"
		local function connect()
			local typed = name:get()
			if typed == nil or typed:match("^%s*$") then typed = state.name end
			send({ action = "connect", name = typed }, _("Connecting to ") .. serverName(state) .. "...")
		end
		local top
		if connected then
			top = note(string.format(_("Online on %s. Join a room someone hosts, or host your own."), serverName(state)))
		else
			top = row({
				label(_("Your name"), "font-scale-body"),
				gap(8),
				input(name, state.name ~= "" and state.name or _("Your name"), 260,
					{ maxLength = 32, acceptOnFocusLoss = true }),
				gap(10),
				primary(connecting and _("Connecting...") or string.format(_("Connect to %s"), serverName(state)),
					connect, canAct and not connecting and not busy),
			})
		end
		return frame(
			_("Play Transport Fever 3 together"),
			status,
			column({
				top,
				gap(24),
				row({
					lobby.choiceCard(_("Join a room"),
						_("Browse the public rooms, or join a friend's with its invite"),
						"::/gui/menu/images/m05_ingame.tga", function() pageS:set("join") end, connected and canAct),
					gap(24),
					lobby.choiceCard(_("Host a room"),
						_("Your room, from one of your saves: you start its game"),
						"::/gui/menu/images/m02_ingame.tga", function() pageS:set("host") end, connected and canAct),
				}),
			}),
			{
				connected and button(_("Disconnect"), disconnect, nil, canAct) or gap(1),
				gap(8),
				button(_("Server..."), function() serverS:set(true) end, nil, canAct),
				gap(8),
				button(_("Your banner"), function() bannerS:set(true) end, nil, canAct),
				gui_react_util.makeHorizontalSpacer(),
				button(_("Close"), onClose),
			}
		)
	end

	local function joinBy(code, password)
		code = (code or ""):gsub("%s", ""):upper()
		if code == "" then
			refusedS:set(_("Type the invite code a friend sent you."))
			return
		end
		joiningS:set(nil)
		send({ action = "join", invite = code, password = password or "" }, _("Joining the room..."))
	end

	-- Join: the public rooms, as cards, and an invite.
	if page == "join" then
		local list = state.rooms
		local found = list and list.list or {}
		local at = list and list.page or 0
		local function askPage(n)
			listAtRef:set(0)
			send({ action = "list_rooms", page = n }, nil)
		end
		local shown = {}
		for _i, listed in ipairs(found) do
			shown[#shown + 1] = lobby.roomCard(listed, function()
				if listed.has_password then
					joiningS:set({ invite = listed.invite, name = listed.name })
				else
					joinBy(listed.invite, "")
				end
			end, canAct and not busy)
		end
		local rows = {}
		for first = 1, #shown, CARDS_PER_ROW do
			local cells = {}
			for i = first, math.min(first + CARDS_PER_ROW - 1, #shown) do
				if i > first then cells[#cells + 1] = gap(12) end
				cells[#cells + 1] = shown[i]
			end
			rows[#rows + 1] = row(cells)
			rows[#rows + 1] = gap(12)
		end
		if #rows == 0 then
			rows[1] = note(list and _("No public rooms right now. Host one, and make it public.")
				or _("Asking the server for its rooms..."))
		end
		local children = {
			row({
				back("choose"),
				gap(16),
				heading(string.format(_("Public rooms on %s"), serverName(state)),
					_("Click a room to join it.")),
				gui_react_util.makeHorizontalSpacer(),
				button(_("Previous"), function() askPage(at - 1) end, nil, canAct and at > 0),
				gap(6),
				button(_("Next"), function() askPage(at + 1) end, nil, canAct and list ~= nil and list.more),
				gap(6),
				button(_("Refresh"), function() askPage(at) end, nil, canAct),
				gap(6),
				button(_("Join with code"), function()
					joiningS:set(nil)
					codeS:set(true)
				end, nil, canAct),
			}),
			builtin.ScrollArea{
				meta = { styleSheet = style{ size = { WIDTH - 40, HEIGHT - 290 } } },
				horizontalPolicy = builtin.type.ScrollBarPolicy.AlwaysOff,
				verticalPolicy = builtin.type.ScrollBarPolicy.AsNeeded,
				content = column(rows),
			},
			gap(10),
		}
		local joining = joiningS:old()
		if joining then
			children[#children + 1] = row({
				label(string.format(_("%s has a password:"), joining.name), "font-scale-body"),
				gap(8),
				input(joinPassword, _("Password"), 220, {
					password = true, maxLength = 64,
					onEnter = function(value) joinBy(joining.invite, value) end,
				}),
				gap(8),
				primary(_("Join"), function() joinBy(joining.invite, joinPassword:get()) end, canAct and not busy),
				gap(6),
				button(_("Cancel"), function() joiningS:set(nil) end),
			})
		end
		local body = column(children)
		-- Join with code: a popup over the room list, with the invite, a
		-- password and Join or Cancel.
		if codeS:old() then
			body = column({
				row({
					heading(_("Join with code"), _("A friend's room: the invite code they sent you, and its password if it has one.")),
				}),
				field(_("Invite code"), invite, "K7QM2X", { maxLength = 128 }),
				field(_("Password (if the room has one)"), joinPassword, "", { password = true, maxLength = 64 }),
				row({
					primary(_("Join"), function()
						codeS:set(false)
						joinBy(invite:get(), joinPassword:get())
					end, canAct and not busy),
					gap(8),
					button(_("Cancel"), function() codeS:set(false) end),
				}),
			}, style{ size = { LEFT, AUTO }, padding = { 16, 16, 16, 16 } })
		end
		return frame(_("Join a room"), status, body, {
			modsButton(),
			gui_react_util.makeHorizontalSpacer(),
			button(_("Close"), onClose),
		})
	end

	-- Host: the room's settings, and Create.
	if page == "host" then
		local rules = state.rules or {}
		local rulesItems = {}
		for i, offered in ipairs(rules) do
			rulesItems[#rulesItems + 1] = { offered.name, i == 1 and (offered.name .. _(" (default)")) or offered.name }
		end
		local pickedRules = rulesS:old()
		local explainRules
		for _i, offered in ipairs(rules) do
			if offered.name == (pickedRules or (rules[1] and rules[1].name)) then explainRules = offered.description end
		end
		local saves = state.saves or {}
		local saveItems = {}
		for _i, save in ipairs(saves) do saveItems[#saveItems + 1] = { save, save } end
		saveItems[#saveItems + 1] = { "", _("None: I load a world myself") }
		local pickedSave = saveS:old()
		if pickedSave == nil then
			pickedSave = ""
			for _i, save in ipairs(saves) do
				if save == state.start_save then pickedSave = save end
			end
			if pickedSave == "" and saves[1] then pickedSave = saves[1] end
		end
		local playersItems = {}
		for n = MIN_PLAYERS, MAX_PLAYERS do
			playersItems[#playersItems + 1] = { n, string.format(_("%d players"), n) }
		end
		local public = publicS:old() == "public"
		local details = pickedSave ~= "" and lobby.saveDetails(pickedSave) or nil
		local function create()
			local named = roomName:get()
			if named == nil or named:match("^%s*$") then
				named = string.format(_("%s's room"), state.name)
			end
			local fields = {
				action = "create",
				room = named,
				password = createPassword:get() or "",
				max_players = playersS:old(),
				rules = pickedRules or "",
				start_save = pickedSave,
				public = public,
				competitive = competitiveS:old() == true,
			}
			if public then
				fields.map = details and details.map or ""
				fields.year = details and details.year or 0
			end
			send(fields, _("Creating the room..."))
		end
		local where = public
			and (details and details.map ~= ""
				and string.format(_("Listed for everyone on %s: %s, %s."), serverName(state),
					lobby.climateName(details.map), details.year > 0 and tostring(details.year) or _("year unknown"))
				or string.format(_("Listed for everyone on %s."), serverName(state)))
			or _("Only players you send the invite to can find it.")
		return frame(_("Host a room"), status, column({
			row({ back("choose"), gap(16),
				heading(_("Host a room"), _("You own it: you start its game, and can remove players.")) }),
			row({
				column({
					field(_("Room name"), roomName, string.format(_("%s's room"), state.name), { maxLength = 48 }),
					choice(_("Start from this save"), pickedSave, saveItems, function(value) saveS:set(value) end,
						pickedSave ~= "" and _("Every player's game loads it from the menu when you start.")
							or _("Load a world in the game once in the room: it is saved for everyone.")),
					choice(_("Players"), playersS:old(), playersItems, function(value) playersS:set(value) end),
				}, style{ size = { LEFT, AUTO } }),
				gap(30),
				column({
					note(_("How you play")),
					gap(4),
					row({
						lobby.styleCard(false, competitiveS:old() ~= true, function() competitiveS:set(false) end, canAct),
						gap(12),
						lobby.styleCard(true, competitiveS:old() == true, function() competitiveS:set(true) end, canAct),
					}),
					gap(4),
					note(competitiveS:old() and _("Each player founds a company of their own in the game.")
						or _("Everyone plays for the room's one company.")),
					gap(10),
					choice(_("Who can find it"), public and "public" or "private", {
						{ "private", _("Private: invite only") },
						{ "public", _("Public: in the room list") },
					}, function(value) publicS:set(value) end, where),
					#rulesItems > 1 and choice(_("Rules"), pickedRules or rulesItems[1][1], rulesItems,
						function(value) rulesS:set(value) end, explainRules) or gap(1),
					field(_("Password (optional)"), createPassword, "", { password = true, maxLength = 64 }),
				}, style{ size = { RIGHT, AUTO } }),
			}),
		}), {
			modsButton(),
			gui_react_util.makeHorizontalSpacer(),
			button(_("Close"), onClose),
			primary(_("Create room"), create, canAct and not busy),
		})
	end

	-- In a room: players on the left, chat on the right.
	local me = you(room)
	local playing = room.phase == "playing"
	local confirm = confirmS:old()
	-- The players as cards of their banners, two to a row; for the owner,
	-- a Remove under each other player's, asked first.
	local memberRows = {}
	local cells = {}
	local function flush()
		if #cells > 0 then
			memberRows[#memberRows + 1] = row(cells)
			memberRows[#memberRows + 1] = gap(10)
			cells = {}
		end
	end
	for _i, member in ipairs(room.members) do
		local parts = { lobby.memberCard(member, playing) }
		if room.you_own and not member.you then
			parts[#parts + 1] = gap(4)
			if confirm and confirm.kind == "kick" and confirm.id == member.id then
				parts[#parts + 1] = row({
					button(_("Remove"), function()
						send({ action = "kick", player = member.id }, string.format(_("Removing %s..."), member.name))
					end, "primary", canAct),
					gap(4),
					button(_("Keep"), function() confirmS:set(nil) end),
				})
			else
				parts[#parts + 1] = row({
					button_react_util.makeIconButton(nil, ICON.kick, function()
						confirmS:set({ kind = "kick", id = member.id, name = member.name })
					end, string.format(_("Remove %s from the room"), member.name)),
				})
			end
		end
		if #cells > 0 then cells[#cells + 1] = gap(12) end
		cells[#cells + 1] = column(parts)
		if #cells >= 3 then flush() end
	end
	flush()

	local roomHeader = column({
		row({
			label(room.name, "font-scale-title-3"),
			gap(8),
			room.has_password and icon(ICON.lock, 18) or gap(1),
			gap(8),
			badge(room.competitive and _("Competitive") or _("Co-op"), room.competitive and "warning" or "success"),
			gui_react_util.makeHorizontalSpacer(),
		}),
		gap(6),
		row({
			note(_("Invite code  ")),
			label(room.invite ~= "" and inviteCode(room.invite) or "-", "font-scale-title-4, info"),
		}),
		note(_("Send it to friends: they join with it from their game's Multiplayer window.")),
		gap(10),
		note(string.format(_("%d of %d players  ·  %d ready"), #room.members, room.max_players, readyCount(room))),
		gap(8),
	})

	local players = column({
		roomHeader,
		heading(_("Players")),
		builtin.ScrollArea{
			meta = { styleSheet = style{ size = { LEFT, HEIGHT - 330 } } },
			horizontalPolicy = builtin.type.ScrollBarPolicy.AlwaysOff,
			verticalPolicy = builtin.type.ScrollBarPolicy.AsNeeded,
			content = column(memberRows),
		},
	}, style{ size = { LEFT, AUTO } })

	local lines = state.chat or {}
	local chatRows = {}
	local first = math.max(1, #lines - 40)
	for i = first, #lines do
		local line = lines[i]
		chatRows[#chatRows + 1] = row({
			label(line.from .. ":", line.you and "font-scale-body, info" or "font-scale-body, success"),
			gap(6),
			label(line.text, "font-scale-body"),
		})
		chatRows[#chatRows + 1] = gap(3)
	end
	if #chatRows == 0 then
		chatRows[1] = note(_("Nothing said yet. Say hello!"))
	end
	local function sendChat()
		local msg = chatText:get()
		if msg and not msg:match("^%s*$") then
			chatText:set("")
			send({ action = "chat", text = msg }, nil)
		end
	end
	local chat = column({
		heading(_("Chat")),
		builtin.ScrollArea{
			meta = { styleSheet = style{ size = { RIGHT, HEIGHT - 330 } } },
			horizontalPolicy = builtin.type.ScrollBarPolicy.AlwaysOff,
			verticalPolicy = builtin.type.ScrollBarPolicy.AsNeeded,
			content = column(chatRows),
		},
		gap(8),
		row({
			input(chatText, _("Say something to the room"), RIGHT - 100,
				{ maxLength = 280, acceptOnFocusLoss = false, onEnter = function() sendChat() end }),
			gap(8),
			button(_("Send"), sendChat, nil, canAct),
		}),
	}, style{ size = { RIGHT, AUTO } })

	local footer = { modsButton() }
	if confirm and confirm.kind == "leave" then
		footer[#footer + 1] = label(_("Leave the room?"), "font-scale-body, warning")
		footer[#footer + 1] = button(_("Leave"), function() send({ action = "leave" }, _("Leaving the room...")) end,
			"primary", canAct)
		footer[#footer + 1] = button(_("Stay"), function() confirmS:set(nil) end)
	else
		footer[#footer + 1] = button(_("Leave room"), function() confirmS:set({ kind = "leave" }) end, nil, canAct)
	end
	footer[#footer + 1] = gui_react_util.makeHorizontalSpacer()
	footer[#footer + 1] = button(_("Close"), onClose)
	if playing then
		footer[#footer + 1] = note(_("The room's game is under way."))
	else
		if me and me.ready then
			footer[#footer + 1] = button(_("Not ready"), function()
				send({ action = "ready", ready = false }, nil)
			end, nil, canAct and not busy)
		else
			footer[#footer + 1] = primary(_("Ready"), function()
				send({ action = "ready", ready = true }, _("Getting ready..."))
			end, canAct and not busy)
		end
		if room.you_own then
			local all = everyoneReady(room)
			footer[#footer + 1] = primary(_("Start the game"), function()
				send({ action = "start" }, _("Starting the room's game..."))
			end, canAct and all and not busy,
				all and _("Every player's game loads the room's world") or _("Waiting for everyone to be ready"))
		end
	end

	return frame(_("Your room"), status, row({ players, gap(30), chat }), footer)
end

-- The live line under a Multiplayer card on the main menu, from the lobby
-- the hook has: its own recipe, so only it redraws when the lobby changes.
lobby.CardLine = react.RegisterRecipe("Tpf3mpCardLine", function(params)
	local lineS = react.useState(lobby.summary(nil))
	react.onStepTimer(function()
		local state = fetchState()
		local line = params and params.line and params.line(state) or lobby.summary(state)
		if line ~= lineS:old() then lineS:set(line) end
	end, 1.0, false)
	-- A recipe placed among a layout's children must return a layout: the
	-- game refused a bare TextView here ("Recipe child must be a layout",
	-- ReactFramework::Load, 2026-09-30), as its own recipes return one.
	return builtin.BoxLayout{
		children = {
			builtin.TextView{
				meta = { class = "font-scale-annotation, annotation" },
				text = lineS:old(),
			},
		},
	}
end)

-- What the "Join a friend" card says: the room once in one.
function lobby.joinLine(state)
	local room = state and state.room
	if room and room.invite ~= "" then
		return string.format(_("Your room: invite %s"), inviteCode(room.invite))
	end
	return _("With the invite code they send you")
end

return lobby
