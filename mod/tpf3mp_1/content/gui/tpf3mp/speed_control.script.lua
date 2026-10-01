-- Guests' clock buttons show the room's accepted speed. Never write
-- that value into GAME_SPEED: the hook reads changes there as requests,
-- so copying a display value back would send another request to the room.
--
-- Uses build 40408's GameSpeedControl recipe and GameSpeedApi (the game's
-- gui/game_bar/game_bar_widgets.tl and gui/main/game_context.d.tl). The
-- stock speed shortcuts consult game_react_globals.getDisableFeatures;
-- guests get GameSpeedControl disabled there too, without changing the
-- game's feature table. Outside the room the original recipe and features
-- are used, including mission restrictions and the game's keyboard hints.
function data()
	local react = ug_require "::/gui/main/react.lua"
	local builtin = ug_require "::/gui/main/builtin.lua"
	local engine_react_util = ug_require "::/gui/main/engine_react_util.tl"
	local globals = ug_require "::/gui/main/game_react_globals.tl"
	-- game.tl's shortcut handlers use its own table; the public module above
	-- copies that table's functions. Both entry points must be wrapped.
	local gameGlobals = (ug_require "/gui/main/game.tl").game_react_globals
	local widgets = ug_require "::/gui/game_bar/game_bar_widgets.tl"
	local bridge = ug_require "tpf3mp_1::/scripts/tpf3mp/bridge.lua"
	local link = bridge.attach(bridge.find())
	local original = widgets.GameSpeedControl
	local getFeatures = globals.getDisableFeatures

	local function isHost(status)
		for _, player in ipairs(status and status.players or {}) do
			if player.me then return player.owner == true end
		end
		return false
	end

	local function roomNow()
		if not link or not link:room() then return nil end
		return link:status() or {} -- unknown room status grants no controls
	end

	local speeds = { 0, 100, 200, 400 }
	local icons = { "playback_pause", "playback_play", "playback_play_2", "playback_play_4" }
	local names = { "Pause", "Speed: 1x", "Speed: 2x", "Speed: 4x" }
	local RoomSpeed = react.RegisterRecipe("Tpf3mpRoomSpeed", function(params, userParam)
		local state = engine_react_util.useStepStateTimer(function()
			local status = roomNow()
			return {
				inRoom = status ~= nil,
				speed = status and status.speed,
				host = isHost(status),
			}
		end, 0.1, function(a, b)
			return a.inRoom == b.inRoom and a.speed == b.speed and a.host == b.host
		end)
		local current = state:old()
		if not current.inRoom or current.host then
			return builtin.BoxLayout{ children = { react.CallOriginalRecipe(original, params, userParam) } }
		end

		local selected, buttons = -1, {}
		for index, speed in ipairs(speeds) do
			if current.speed == speed then selected = index end
			buttons[index] = {
				meta = {
					tooltip = names[index] .. " — Host controls speed",
					enabled = false,
				},
				content = builtin.ImageView{ path = "::/gui/game_bar/icons/" .. icons[index] .. ".tga" },
			}
		end
		return builtin.BoxLayout{
			orientation = builtin.type.Orientation.Horizontal,
			meta = { tooltip = "Host controls speed" },
			children = {
				builtin.ToggleButtonGroup{
					meta = { enabled = false },
					buttons = buttons, selected = selected, deselectAllowed = selected == -1,
					onValueChange = function() end,
				},
			}
		}
	end)

	return {
		replace = function(replacementApi)
			if not link then return end
			local function roomFeatures(...)
				local features = getFeatures(...)
				local status = roomNow()
				if status == nil or isHost(status) then return features end
				local limited = {}
				for key, value in pairs(features) do limited[key] = value end
				limited.GameSpeedControl = true
				return limited
			end
			globals.getDisableFeatures = roomFeatures
			gameGlobals.getDisableFeatures = roomFeatures
			replacementApi.ReplaceRecipe(original, RoomSpeed)
			link:log("guest speed controls follow the room; only the host can change speed")
		end,
	}
end
