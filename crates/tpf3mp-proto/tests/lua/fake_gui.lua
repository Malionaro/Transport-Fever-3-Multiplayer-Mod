-- A stand-in for Transport Fever 3's GUI state, as much of it as the mod's
-- entry script (mod/tpf3mp_1/content/gui/tpf3mp/tpf3mp.script.lua) uses:
-- ug_require, react, builtin, the game bar's and the mods' buttons'
-- extension points, the window container's API and debugPrint. Its shape
-- follows mods made for build 40391 (investigation/TF3_MODS_2026-09-27.md)
-- and the game's own GUI (gui/game_bar/game_bar.tl opens its windows so).
-- Run by tests/lua_mod.rs, which defines mod_source(path): the text of a
-- file under the mod's content/.
--
-- Defines the globals the game has, plus LOG (every debugPrint line),
-- WINDOWS (the windows open, by recipe name, each a mount) and
-- mount(recipe, params), which renders a recipe and runs its steps.

-- The game's package table has no preload (build 40408's dump,
-- investigation/dayone-2026-09-29/probe/script_api_dump_gui.txt).
package.preload = nil

api = { gui = { StyleSheet = { new = function() return {} end } }, type = {
	Vec2f = { new = function(x, y) return { x = x, y = y } end },
	Vec4f = { new = function(x, y, z, w) return { x = x, y = y, z = z, w = w } end },
} }

LOG = {}
function debugPrint(line)
	LOG[#LOG + 1] = tostring(line)
end

local current   -- the mount a recipe is rendering in

local react = {}
function react.RegisterPluginRecipe(extension, name, fn)
	return { extension = extension, name = name, fn = fn }
end
function react.RegisterWrapperRecipe(name, wrapped, fn)
	return { name = name, wraps = wrapped, fn = fn }
end
function react.RegisterRecipe(name, fn)
	return { name = name, fn = fn }
end
-- Runs the recipe a replacement replaced, in the replacement's render, as
-- gui/main/react.lua does.
function react.CallOriginalRecipe(recipe, ...)
	return recipe.fn(...)
end
function react.useRef(initial)
	-- The same ref on every render of one mount, as React keeps it.
	local m = current
	m.refIndex = m.refIndex + 1
	local ref = m.refs[m.refIndex]
	if not ref then
		ref = { value = initial }
		function ref:get() return self.value end
		function ref:set(v) self.value = v end
		m.refs[m.refIndex] = ref
	end
	return ref
end
function react.useState(initial)
	-- A ref whose value the next render reads, as the game's state does.
	local state = react.useRef(initial)
	function state:old() return self.value end
	return state
end
function react.onStep(fn)
	current.onStep = fn
end

local builtin = { type = {
	Orientation = { Horizontal = "Horizontal", Vertical = "Vertical" },
	ScrollBarPolicy = { Simple = "Simple", AlwaysOff = "AlwaysOff", AsNeeded = "AsNeeded" },
} }
function builtin.BoxLayout(params)
	return { layout = "BoxLayout", params = params }
end
-- A view is a recipe the game has: called, it gives the node; its name
-- says which recipe a wrapper wraps.
for _, view in ipairs({ "TextView", "Button", "ScrollArea", "Component", "TextInputField", "Window",
		"ColorChooserButton", "ImageView", "ProposalViewer" }) do
	builtin[view] = setmetatable({ viewName = view }, {
		__call = function(_, params) return { view = view, params = params } end,
	})
end

local game_bar_widgets = { GameBarInfoDisplayExtension = "GameBarInfoDisplayExtension" }
local main_mod_button_area = { MainModButtonAreaExtension = "MainModButtonAreaExtension" }

-- The window container: a singleton window is added once and stays until
-- removed; moving it to the front of a window not there fails, as nothing
-- else in the fake would notice it.
WINDOWS = {}
local windows = {}
function windows.addSingletonWindow(recipe, params)
	assert(recipe.wraps == builtin.Window, "a window's recipe wraps builtin.Window")
	if WINDOWS[recipe.name] == nil then WINDOWS[recipe.name] = mount(recipe, params) end
end
function windows.moveSingletonWindowToFront(recipe)
	assert(WINDOWS[recipe.name], "no such window")
end
function windows.removeAllWindows(recipe)
	WINDOWS[recipe.name] = nil
end
local game_react_globals = { getDefaultWindowApi = function() return windows end }

-- A state read from the engine now and on later steps: here, on every
-- render.
local engine_react_util = {}
function engine_react_util.useStepStateTimer(get)
	local state = react.useRef(nil)
	state.value = get(state.value)
	function state:old() return self.value end
	return state
end

-- The game's recipe for a thing's marker on the map: it names the entity.
local hud_icon_toolbox = {}
hud_icon_toolbox.HudIconMasterGame = react.RegisterRecipe("HudIconMasterGame", function(params)
	return { view = "Marker", params = { entity = params.entity } }
end)

-- The game's ownership test, as its line manager asks it
-- (scripts/entity_util.tl): here, whether OWN_OR_NO_ONES[entity] is set.
local entity_util = {}
OWN_OR_NO_ONES = {}
function entity_util.isOwnedByPlayerOrNotOwned(entity)
	return OWN_OR_NO_ONES[entity] == true
end

local GAME = {
	["/scripts/entity_util.tl"] = entity_util,
	["::/gui/main/react.lua"] = react,
	["::/gui/main/builtin.lua"] = builtin,
	["::/gui/game_bar/game_bar_widgets.tl"] = game_bar_widgets,
	["::/gui/main/main_mod_button_area.tl"] = main_mod_button_area,
	["::/gui/main/game_react_globals.tl"] = game_react_globals,
	["::/gui/main/engine_react_util.tl"] = engine_react_util,
	["::/gui/main/hud_icon_toolbox.tl"] = hud_icon_toolbox,
}

-- The mod whose files mod_source reads: ours, or the one MOD_ID names.
local MOD = (MOD_ID or "tpf3mp_1") .. "::/"
local loaded = {}
UG_REQUIRED = {}
function ug_require(path)
	UG_REQUIRED[#UG_REQUIRED + 1] = path
	if GAME[path] then return GAME[path] end
	-- A test's stand-ins for more of the game's modules, by their path.
	if GAME_MODULES and GAME_MODULES[path] then return GAME_MODULES[path] end
	if loaded[path] then return loaded[path] end
	assert(path:sub(1, #MOD) == MOD, "ug_require of an unknown path " .. path)
	local rel = path:sub(#MOD + 1)
	local chunk = assert(loadstring(mod_source(rel), "@" .. rel))
	local module = chunk()
	loaded[path] = module
	return module
end

-- Renders a recipe once with its params, then returns the mount: render()
-- renders it again, step() runs its onStep as the game does every frame.
function mount(recipe, params)
	local m = { refs = {}, refIndex = 0 }
	function m.render()
		current, m.refIndex = m, 0
		m.layout = recipe.fn(params)
		current = nil
		-- A wrapper renders the recipe it wraps.
		if recipe.wraps then
			assert(type(m.layout) == "table" and m.layout.view == recipe.wraps.viewName,
				recipe.name .. " must render a " .. recipe.wraps.viewName)
		end
		return m.layout
	end
	function m.step()
		if m.onStep then m.onStep() end
	end
	m.render()
	return m
end

-- Runs a mod's entry script and returns its plugin, checking the name the
-- resource file gives and its extension point: ours
-- (tpf3mp.script@Tpf3mpPlugin, on the game bar) unless named.
function loadPlugin(script, recipe, extension)
	script = script or "gui/tpf3mp/tpf3mp.script.lua"
	recipe = recipe or "Tpf3mpPlugin"
	extension = extension or "GameBarInfoDisplayExtension"
	local entry = assert(loadstring(mod_source(script), "@" .. script))
	entry()
	local exported = data()
	local plugin = assert(exported[recipe], "no " .. recipe)
	assert(plugin.extension == extension, "on another extension point")
	return plugin
end

-- The views a rendered layout holds, depth first, as { view =, params = }.
function views(node, out)
	out = out or {}
	if type(node) ~= "table" then return out end
	if node.view then out[#out + 1] = node end
	local params = node.params or {}
	for _, key in ipairs({ "content", "layout", "child", "item" }) do views(params[key], out) end
	for _, child in ipairs(params.children or {}) do views(child, out) end
	return out
end

-- The lines the mod logged, one per line.
function logText()
	return table.concat(LOG, "\n")
end

-- What the nameplates need of the game's GUI, added here rather than above:
-- a test that reads a line number out of a logged refusal (the unknown
-- ug_require path) would otherwise move when this file grows.
api.type.Vec2i = { new = function(x, y) return { x = x, y = y } end }
api.type.Vec3f = { new = function(x, y, z) return { x = x, y = y, z = z } end }

-- The camera: where a world point is on the window. Here it takes the
-- point as it is, so a preview's place, in the game's metres, is off any
-- window unless a test sets a camera of its own.
CAMERA = { width = 1920, height = 1080 }
api.gui.camera = {
	getSize = function() return { x = CAMERA.width, y = CAMERA.height } end,
	world2Screen = function(at) return { x = at.x, y = at.y } end,
}

for _, view in ipairs({ "FloatingLayout", "FloatingLayoutChild" }) do
	builtin[view] = setmetatable({ viewName = view }, {
		__call = function(_, params) return { view = view, params = params } end,
	})
end
-- The layer over the whole window the game mounts a recipe in, which
-- belongs to the window and not to the extension point's own layout.
builtin.FullScreenComponent = function(params)
	return { view = "FullScreenComponent", params = params }
end

-- What a full-screen recipe says about itself: the game's own does (town.tl),
-- and a test reads that this one takes no mouse.
REACT_FLAGS = {}
function react.setMouseTransparent(on) REACT_FLAGS.mouseTransparent = on end
function react.setDisableFocusable(on) REACT_FLAGS.disableFocusable = on end

-- Mission markers (api.gui.mission.setMarkerAtPosition):
api.gui.mission = {
	markers = {},
	setMarkerAtPosition = function(key, pos, type, scaling, isSelectable, fireSelectEntity)
		api.gui.mission.markers[key] = {
			pos = pos,
			type = type,
			scaling = scaling,
			isSelectable = isSelectable,
			fireSelectEntity = fireSelectEntity,
		}
	end,
	removeMarker = function(key)
		api.gui.mission.markers[key] = nil
	end,
}
function react.onUnmount(fn)
	if current then current.onUnmount = fn end
end
