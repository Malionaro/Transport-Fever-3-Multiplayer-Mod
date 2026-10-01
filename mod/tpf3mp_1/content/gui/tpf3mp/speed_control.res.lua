-- Keep the game's speed row in step with the room.
function data()
	return {
		type = "react-replacement-config",
		data = {
			filePath = "tpf3mp_1::/gui/tpf3mp/speed_control.script",
			doReplaceFn = "replace",
		}
	}
end
