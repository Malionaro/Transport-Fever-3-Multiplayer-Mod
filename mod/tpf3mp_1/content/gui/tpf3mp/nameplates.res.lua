-- Mounts the names of the other members whose build previews this game
-- shows, at those previews (gui/tpf3mp/tpf3mp.script.lua, recipe
-- Tpf3mpNameplates): the second plugin on the game bar's info display,
-- beside the Multiplayer one, drawing nothing else. It needs the game's
-- own GUI state, as that one does.
function data()
	return {
		type = "react-plugin ::GameBarInfoDisplayExtension",
		data = {
			filePath = "tpf3mp_1::/gui/tpf3mp/tpf3mp.script@Tpf3mpNameplates",
			priority = 6,
		}
	}
end