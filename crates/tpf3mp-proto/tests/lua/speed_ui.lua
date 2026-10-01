-- Exercise the real replacement recipe against the documented build 40408
-- GUI interfaces. The local speed deliberately differs from the room's.
local react = ug_require("::/gui/main/react.lua")
local builtin = ug_require("::/gui/main/builtin.lua")
local globals = ug_require("::/gui/main/game_react_globals.tl")
local widgets = ug_require("::/gui/game_bar/game_bar_widgets.tl")
for _, view in ipairs({ "ImageView", "ToggleButtonGroup" }) do
	builtin[view] = function(params) return { view = view, params = params } end
end

local features = { OtherFeature = true }
globals.getDisableFeatures = function() return features end
local readFeatures = globals.getDisableFeatures
-- The public globals module copies functions from game.tl; its local
-- shortcut handlers retain the separate original table.
local gameGlobals = { getDisableFeatures = readFeatures }
GAME_MODULES = { ["/gui/main/game.tl"] = { game_react_globals = gameGlobals } }
local localSpeed, requests = 4, {}
local helper = {
	getSpeed = function() return localSpeed end,
	setSpeed = function(speed) requests[#requests + 1] = speed; localSpeed = speed end,
}
local params = { gameSpeedHelper = { get = function()
	return { getApi = function() return helper end }
end } }
local original = react.RegisterRecipe("GameSpeedControl", function(given)
	assert(given == params)
	return { view = "StockSpeed", params = { speed = helper.getSpeed() } }
end)
widgets.GameSpeedControl = original
local recipe = original
assert(loadstring(mod_source("gui/tpf3mp/speed_control.script.lua")))()
data().replace({ ReplaceRecipe = function(old, new)
	assert(old == original)
	recipe = new
end })
local row = mount(recipe, params)
local function control()
	for _, view in ipairs(views(row.render())) do
		if view.view == "ToggleButtonGroup" then return view.params end
	end
end
local function stock()
	local all = views(row.render())
	assert(#all == 1 and all[1].view == "StockSpeed")
	assert(all[1].params.speed == localSpeed)
end
-- As gui/main/game.tl's gamePause, gameCycleSpeed and
-- IA_GAME_PAUSE_OR_CYCLE_SPEED do, consult features before invoking the helper.
local function shortcut(speed)
	if gameGlobals.getDisableFeatures().GameSpeedControl ~= true then helper.setSpeed(speed) end
end
stock()
assert(globals.getDisableFeatures() == features)
if not tpf3mp_native then
	assert(recipe == original and globals.getDisableFeatures == readFeatures
		and gameGlobals.getDisableFeatures == readFeatures)
	shortcut(0)
	assert(#requests == 1 and requests[1] == 0)
	return
end

HOOK.room = true
HOOK.status = { speed = 100, players = {
	{ name = "Host", owner = true, me = false }, { name = "Guest", owner = false, me = true },
} }
for index, speed in ipairs({ 0, 100, 200, 400 }) do
	HOOK.status.speed = speed
	local c = assert(control())
	assert(c.selected == index and not c.meta.enabled, "guest shows room speed, disabled")
	for _, button in ipairs(c.buttons) do
		assert(not button.meta.enabled and button.meta.tooltip:find("Host controls speed", 1, true))
	end
	c.onValueChange(3) -- stale/synthetic activation cannot request a speed either
	shortcut(2)
	assert(#requests == 0 and localSpeed == 4, "display never feeds a speed back into the game")
end
assert(globals.getDisableFeatures().GameSpeedControl == true)
assert(globals.getDisableFeatures().OtherFeature == true and features.GameSpeedControl == nil,
	"the game's original feature table is not modified")

-- Ownership changes without recreating the recipe.
HOOK.status.players[1].owner = false
HOOK.status.players[2].owner = true
stock()
assert(globals.getDisableFeatures() == features)
helper.setSpeed(1)
assert(#requests == 1 and requests[1] == 1)
shortcut(0)
assert(#requests == 2 and requests[2] == 0)

-- The host's stock controls retain the original mission/mod restrictions.
features.GameSpeedPause = true
assert(globals.getDisableFeatures().GameSpeedPause == true)
features.GameSpeedControl = true
shortcut(2)
assert(#requests == 2)
features.GameSpeedControl, features.GameSpeedPause = nil, nil
HOOK.status.players[2].owner = false
local c = control()
assert(not c.meta.enabled, "a former host now gets the guest display")
shortcut(4)
assert(#requests == 2)
HOOK.status.players = {}
assert(not control().meta.enabled and globals.getDisableFeatures().GameSpeedControl == true)

-- Unknown/unsupported room speeds must not masquerade as a known setting.
HOOK.status.speed = 150
c = control()
assert(c.selected == -1 and c.deselectAllowed)
HOOK.status = nil
c = control()
assert(c.selected == -1 and not c.meta.enabled)
assert(globals.getDisableFeatures().GameSpeedControl == true)

-- After leaving, stock UI and keyboard control return without a restart.
HOOK.room = false
stock()
assert(globals.getDisableFeatures() == features)
shortcut(2)
assert(#requests == 3 and requests[3] == 2)
