-- A stand-in for Transport Fever 3's main menu, as much of it as the mod's
-- Multiplayer window (mod/tpf3mp_1/content/gui/menu/lobby.lua) uses:
-- ug_require with react, builtin, gui_react_util and button_react_util,
-- api.gui.StyleSheet, the loader's resolveutil.loadfile the hook answers,
-- _ and debugPrint. Run by the tests in crates/tpf3mp-hook/src/lobby.rs,
-- which define LOBBY_SOURCE (the window's file) and set STATE (the hook's
-- answer to a state request, a Lua table literal).
--
-- It holds the window to the rules the game enforces, which it learnt in
-- the game on 2026-09-30:
-- - a recipe returns a layout: the game refused a recipe returning a bare
--   TextView among a layout's children ("Recipe child must be a layout");
-- - a list of children has no holes: a nil in it cuts the Lua list short,
--   and whatever came after is silently not drawn;
-- - a size is -1 (left to the content) or more than 0: the game drew
--   columns sized {w, 0} as thin bars, with everything in them hidden;
-- - a text field or a combo box has a size, its own or its box's.
--
-- Defines LOG (debugPrint lines), SENT (the JSON of every action the
-- window sent), REPLY (what the hook answers an action, "ok" unless set),
-- and render(focus), tick(), texts(), find(text), click(text),
-- choose(caption, value), type_into(placeholder, text) and enabled(text).

LOG = {}
SENT = {}
REPLY = "ok"
STATE = nil

function debugPrint(line) LOG[#LOG + 1] = tostring(line) end
function _(text) return text end

api = {
	gui = { StyleSheet = { new = function() return {} end } },
	type = {
		Vec2f = { new = function(x, y) return { x = x, y = y } end },
		Vec4f = { new = function(a, b, c, d) return { a, b, c, d } end },
		SavegameId = { new = function() return {} end },
		-- A save's metadata.date, read as the game's Load Game page does.
		Date = { new = function(date) return { year = date } end },
	},
}

-- The game's saves and climates, as the menu's app gives them. SAVES maps
-- a save's name to its climate resource and year; READS counts the saves
-- read.
SAVES = {}
READS = 0
app = {
	SaveGameNamespace = { getSavegame = function() return "savegame" end },
	findAllSavegames = function(namespace)
		local found = {}
		for name in pairs(SAVES) do found[#found + 1] = { saveName = name, path = "saves/" .. name .. ".sav" } end
		return found
	end,
	getSavegameInfo = function(id)
		READS = READS + 1
		local save = assert(SAVES[id.saveGameName], "no save " .. tostring(id.saveGameName))
		return {
			isCompleted = function() return true end,
			get = function()
				return { info = {
					configDict = { { "climate", save.climate }, { "seed", "1" } },
					metadata = { date = save.year, startYear = save.year },
				} }
			end,
		}
	end,
	res = { climateRep = {
		find = function(res) return res == "::/climates/dry/dry.clima" and 3 or -1 end,
		get = function(id) return { desc = { name = "Dry", icon = "::/climates/dry/icon.tga" } } end,
	} },
}

-- The hook's answers, as crates/tpf3mp-hook/src/menu_entry.rs gives them.
resolveutil = {}
function resolveutil.loadfile(path)
	if path == "tpf3mp_1::/tpf3mp/state.lua" then
		return STATE
	elseif path == "tpf3mp_1::/tpf3mp/act.lua" then
		SENT[#SENT + 1] = resolveutil.__tpf3mp_action
		return REPLY
	end
	error("no such file: " .. tostring(path))
end

local mount = { refs = {}, index = 0, timers = {} }

local react = {}
function react.useRef(initial)
	mount.index = mount.index + 1
	local ref = mount.refs[mount.index]
	if not ref then
		ref = { value = initial }
		function ref:get() return self.value end
		function ref:set(v) self.value = v end
		function ref:old() return self.value end
		mount.refs[mount.index] = ref
	end
	return ref
end
react.useState = react.useRef
function react.onStepTimer(fn) mount.timers[#mount.timers + 1] = fn end
local LAYOUTS = { BoxLayout = true, FloatingLayout = true }
-- A wrapper recipe's widget takes meta for its class only: the game
-- asserted and closed on a window whose meta had a styleSheet ("Wrapper
-- recipe must return child", 2026-09-30).
function react.RegisterWrapperRecipe(name, wrapped, fn)
	return setmetatable({ name = name }, { __call = function(_, params)
		local node = fn(params)
		assert(type(node) == "table" and node.view == wrapped.viewName,
			"Wrapper recipe must return child: " .. name)
		for key in pairs(node.params.meta or {}) do
			assert(key == "class", "a wrapper recipe's meta may hold its class only, not " .. key .. ": " .. name)
		end
		return node
	end })
end
function react.RegisterRecipe(name, fn)
	return setmetatable({ name = name }, { __call = function(_, params)
		local node = fn(params)
		assert(type(node) == "table" and LAYOUTS[node.view],
			"Recipe child must be a layout: " .. name .. " returned " .. tostring(node and node.view))
		return node
	end })
end

local builtin = { type = {
	Orientation = { Horizontal = "Horizontal", Vertical = "Vertical" },
	ScrollBarPolicy = { AlwaysOff = "AlwaysOff", AsNeeded = "AsNeeded" },
	ImageViewScaling = { AutoFit = "AutoFit" },
} }
-- A list of children with no holes, every one a node.
local function whole(list, what)
	if list == nil then return end
	local most = table.maxn(list)
	assert(most == #list, what .. ": a nil among the children cuts the list at " .. #list .. " of " .. most)
	for i = 1, most do
		assert(type(list[i]) == "table", what .. ": child " .. i .. " is " .. type(list[i]))
	end
end

local function sized(params, what)
	local sheet = params and params.meta and params.meta.styleSheet
	local size = sheet and sheet.size
	if size then
		for _i, side in ipairs({ size.x, size.y }) do
			assert(side == -1 or side > 0, what .. ": a size of " .. tostring(side) .. " hides what is in it")
		end
	end
	return size
end

for _i, view in ipairs({ "BoxLayout", "Component", "TextView", "Button", "ImageView", "TextInputField",
		"ScrollArea", "ComboBox", "ComboBoxItem", "ProgressBar", "FloatingLayout", "FloatingLayoutChild",
		"ShaderQuad", "Window" }) do
	-- A view is a recipe the game has: called, it gives the node; its name
	-- says which view a wrapper recipe wraps.
	builtin[view] = setmetatable({ viewName = view }, { __call = function(_, params)
		whole(params.children, view)
		whole(params.items, view)
		sized(params, view)
		if view == "Component" and params.layout ~= nil then
			assert(LAYOUTS[params.layout.view], "a Component's layout must be a layout, not " .. tostring(params.layout.view))
		end
		return { view = view, params = params }
	end })
end

-- Every text field and combo box has a width and a height, its own or its
-- box's: the game gives an unsized one none.
local function checkInputs(node, box)
	if type(node) ~= "table" then return end
	if node.view then
		local size = node.params and sized(node.params, node.view)
		local own = size and size.x > 0 and size.y > 0 and size or nil
		if node.view == "TextInputField" or node.view == "ComboBox" then
			assert(own or box, node.view .. " without a size")
		elseif node.view == "Component" then
			-- The box a Component's own layout lays its children out in.
			box = own
		elseif node.view ~= "BoxLayout" then
			box = nil
		end
	end
	for _k, value in pairs(node) do
		if type(value) == "table" then checkInputs(value, box) end
	end
end

local gui_react_util = {
	makeHorizontalSpacer = function() return { view = "Spacer" } end,
	makeVerticalSpacer = function() return { view = "Spacer" } end,
}
local button_react_util = {
	makeIconButton = function(_ref, path, onClick, tooltip)
		return { view = "Button", params = { icon = path, onClick = onClick, meta = { tooltip = tooltip } } }
	end,
}

-- The main menu's card button, as menu_icon_react_util.tl builds it: a
-- recipe (so it returns a layout) around a Button.
CARD_CLICKS = {}
local menu_icon_react_util = {
	makeCardLabelBottomComponent = function(title, description, right)
		local children = {
			builtin.TextView{ text = title },
			description and builtin.TextView{ text = description } or builtin.TextView{ text = "" },
			right and builtin.TextView{ text = right } or builtin.TextView{ text = "" },
		}
		return builtin.FloatingLayout{ children = children }
	end,
}
menu_icon_react_util.CardButton = react.RegisterRecipe("CardButton", function(params)
	return builtin.BoxLayout{ children = {
		builtin.Button{
			meta = { tooltip = params.tooltip, enabled = params.enabled, class = "main-menu-card, " .. tostring(params.class) },
			content = builtin.Component{ layout = builtin.FloatingLayout{ children = { params.bottomComponent } } },
			onClick = params.onClick,
			card = true,
			images = params.images,
		},
	} }
end)

local modules = {
	["::/gui/main/react.lua"] = react,
	["::/gui/main/builtin.lua"] = builtin,
	["::/gui/main/gui_react_util.tl"] = gui_react_util,
	["::/gui/main/button_react_util.tl"] = button_react_util,
	["::/gui/menu/menu_icon_react_util.tl"] = menu_icon_react_util,
}
function ug_require(path)
	return assert(modules[path], "no module " .. path)
end

local lobby = assert(loadstring(LOBBY_SOURCE, "@lobby.lua"))()
LOBBY = lobby
CLOSED = 0
local tree
local focus

function render(f)
	if f ~= nil then focus = f end
	mount.index = 0
	mount.timers = {}
	tree = lobby.content(function() CLOSED = CLOSED + 1 end, focus)
	checkInputs(tree, nil)
	return tree
end

-- One poll of the window's timer, then a redraw, as the game does.
function tick()
	for _i, fn in ipairs(mount.timers) do fn() end
	return render()
end

local function walk(node, visit)
	if type(node) ~= "table" then return end
	visit(node)
	for _k, value in pairs(node) do
		if type(value) == "table" then walk(value, visit) end
	end
end

-- Every text shown, in one string, one per line.
function texts()
	local out = {}
	walk(tree, function(node)
		if node.view == "TextView" then out[#out + 1] = node.params.text end
		if node.view == "ComboBoxItem" then end
	end)
	return table.concat(out, "\n")
end

-- The button showing `text` (or with that tooltip), nil if none.
function find(text)
	local found
	walk(tree, function(node)
		if node.view == "Button" and not found then
			local content = node.params.content
			local shown = content and content.params and content.params.text
			if shown == text or (node.params.meta and node.params.meta.tooltip == text) then
				found = node.params
			end
		end
	end)
	return found
end

function enabled(text)
	local button = assert(find(text), "no button " .. text)
	return button.meta == nil or button.meta.enabled ~= false
end

function click(text)
	local button = assert(find(text), "no button " .. text)
	assert(button.meta == nil or button.meta.enabled ~= false, "button disabled: " .. text)
	button.onClick()
	return render()
end

-- Types into the field showing `placeholder`, and presses Enter.
function type_into(placeholder, text)
	local done = false
	walk(tree, function(node)
		if node.view == "TextInputField" and node.params.placeholderText == placeholder and not done then
			node.params.onTyping(text)
			node.params.onValueChange(text)
			done = true
		end
	end)
	assert(done, "no field " .. placeholder)
	return render()
end

-- Picks `value` in the combo box under the caption `caption`.
function choose(caption, value)
	local done = false
	walk(tree, function(node)
		if node.view == "Component" and not done then
			local children = node.params.layout and node.params.layout.params.children
			local first = children and children[1]
			if first and first.view == "TextView" and first.params.text == caption then
				walk(node, function(inner)
					if inner.view == "ComboBox" and not done then
						inner.params.onValueChange(value)
						done = true
					end
				end)
			end
		end
	end)
	assert(done, "no choice " .. caption)
	return render()
end

-- The values a choice under `caption` offers, and the one chosen.
function offered(caption)
	local values, chosen = {}, nil
	walk(tree, function(node)
		if node.view == "Component" and chosen == nil then
			local children = node.params.layout and node.params.layout.params.children
			local first = children and children[1]
			if first and first.view == "TextView" and first.params.text == caption then
				walk(node, function(inner)
					if inner.view == "ComboBox" and chosen == nil then
						chosen = inner.params.value
						for _i, item in ipairs(inner.params.items) do values[#values + 1] = item.params.value end
					end
				end)
			end
		end
	end)
	return values, chosen
end

-- The room cards shown: each the texts on it, its picture and its click.
function room_cards()
	local found = {}
	walk(tree, function(node)
		if node.view == "Button" and node.params.card then
			local texts = {}
			walk(node.params.content, function(inner)
				if inner.view == "TextView" then texts[#texts + 1] = inner.params.text end
			end)
			found[#found + 1] = {
				text = table.concat(texts, "\n"),
				picture = node.params.images[1],
				click = node.params.onClick,
				enabled = node.params.meta.enabled ~= false,
			}
		end
	end)
	return found
end

-- Clicks the card (Join, Host, a room) whose texts include `title`.
function click_card(title)
	for _i, card in ipairs(room_cards()) do
		if card.text:find(title, 1, true) then
			assert(card.enabled, "card disabled: " .. title)
			card.click()
			return render()
		end
	end
	error("no card " .. title)
end
