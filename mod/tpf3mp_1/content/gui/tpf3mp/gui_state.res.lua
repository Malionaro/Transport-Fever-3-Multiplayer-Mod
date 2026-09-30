-- Runs gui/tpf3mp/gui_state.script.lua in the Lua state the game renders
-- its recipes in: a react-replacement-config, which the game runs before
-- any recipe renders (gui/main/bootstrap_game.tl;
-- investigation/TF3_MODS_2026-09-27.md). It replaces no recipe.
function data()
	return {
		type = "react-replacement-config",
		data = {
			filePath = "tpf3mp_1::/gui/tpf3mp/gui_state.script",
			doReplaceFn = "prepare",
		}
	}
end
