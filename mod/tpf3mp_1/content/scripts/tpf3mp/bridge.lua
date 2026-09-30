-- tpf3mp/bridge.lua -- the Lua half of the link between the mod and the hook.
--
-- The hook (crates/tpf3mp-hook) runs only in a game the TPF3-MP launcher
-- started (DECISIONS.md, D11). There it gives every Lua state that calls
-- `print` one global table, so the mod prints before it looks for it:
--
--   tpf3mp_native = {
--     version = 10,                 -- bridge.VERSION; anything else is refused
--     command = function(action),   -- the player acted: an action table, for
--                                   -- the room to order -> true, ticket |
--                                   -- false, why
--     take    = function(),         -- in a game script's update: the actions
--                                   -- the room ordered for this update, or nil,
--                                   -- and who sent each (64 hex digits)
--     log     = function(line),     -- a line for hook.log
--     poll    = function(),         -- in the GUI, every frame: what the hook
--                                   -- asks, { save = name } or { load = name }
--                                   -- (the game's own save folder), or nil
--     saved   = function(name, ok, why), -- the GUI's answer to a save
--     world   = function(),         -- a world's GUI started
--     room    = function(),         -- whether the room's game runs -> boolean
--     checkpoint = function(),      -- in a game script's postUpdate: whether
--                                   -- to read the world's lanes now
--     lanes   = function(t),        -- those lanes, { [lane] = text }
--                                   -- -> true | false, why
--     clicks  = function(),         -- in the GUI: the player's builds queued
--                                   -- in the room's game so far, or nil where
--                                   -- the hook cannot take them to the room
--     built   = function(n),        -- optional; in the GUI: the build the
--                                   -- module editor queued at click n, as
--                                   -- game scripts see a proposal | nil, why
--                                   -- | nil (not the module editor's)
--     replaying = function(on),     -- the game script applies the room's
--                                   -- actions (true) or is done (false)
--     applied = function(i, ok, entity, why), -- in a game script's postUpdate:
--                                   -- what became of the batch's action i
--     results = function(),         -- in the GUI: what became of the player's
--                                   -- own actions since the last call,
--                                   -- { { ticket =, ok =, entity =, why = } }
--     status  = function(),         -- the room, for the Multiplayer window, or
--                                   -- nil before its game: { room =, speed =,
--                                   -- diverged =, me_id =, players = { {
--                                   -- name =, connected =, owner =, me =,
--                                   -- id = } } }
--     chat    = function(),         -- what the room's members said since the
--                                   -- last call, { { from =, text = } }
--     say     = function(text),     -- says text to the room -> true |
--                                   -- false, why
--     dump    = function(),         -- optional; in a game script's
--                                   -- postUpdate at a checkpoint: the lanes
--                                   -- to dump, { step =, lanes = { n, ... } },
--                                   -- once, or nil
--     dumped  = function(lane, entry), -- optional; one entry of a lane
--                                   -- dumped, for hook.log -> true | false
--                                   -- (no more taken)
--     cursor  = function(x, y, b, l),  -- optional; reports pointer/build preview
--     cursors = function(),         -- optional; other players' pointers/previews
--   }
--
-- An action table mirrors tpf3mp_proto::action::Action field for field, in
-- the game's units: metres, and plain fractions for directions. The hook
-- converts it to and from the schema (tpf3mp_proto::lua), so the rounding,
-- the bounds and the checks live in Rust alone; `command` returns false and
-- a reason for a table the schema refuses.
--
-- `command` is Session::command (docs/HOOKS.md, "The hook's session"). An
-- action the room orders comes back to every game, the one that sent it
-- included, through `take`: the hook hands it to the first simulation update
-- of the step it was ordered for, and the mod's game script
-- (tpf3mp_sim/tpf3mp_sim.script.lua) applies it there, where a command runs
-- at once, so every game applies it in the same update.
--
-- Without the table the game is the plain game, and attach() says so. The
-- mod then does nothing. With a table of another version, or one missing a
-- function, attach() refuses it rather than guessing (fail closed).
--
-- Pure Lua; the tests hand attach() a fake table.

local bridge = {}

-- 10: companies: `take` also names who sent each action, `status` each
-- player's id (`id`, `me_id`);
-- 9: the Multiplayer window: the room, its chat (`status`, `chat`, `say`);
-- 8: the player hears what became of their actions (`command`'s ticket,
-- `applied`, `results`);
-- 7: the build tools through the room (`clicks`, `replaying`);
-- 6: the game script reads the world's lanes at checkpoints (`checkpoint`,
-- `lanes`);
-- 5: the GUI asks whether the room's game runs (`room`), for the guard;
-- 4: the GUI saves and loads the room's world (`poll`, `saved`, `world`);
-- 3: the room's actions are taken by the game script (`take`); 2 called the
-- GUI's handlers; 1 passed bytes the mod encoded itself.
bridge.VERSION = 10
bridge.GLOBAL = "tpf3mp_native"

local Link = {}
Link.__index = Link

-- The link to the hook, or nil and why there is none.
function bridge.attach(native)
	if native == nil then return nil, "no hook in this game" end
	if type(native) ~= "table" then return nil, bridge.GLOBAL .. " is not a table" end
	if native.version ~= bridge.VERSION then
		return nil, "the hook speaks bridge version " .. tostring(native.version)
			.. ", the mod " .. bridge.VERSION
	end
	for _, name in ipairs({ "command", "take", "log", "poll", "saved", "world", "room",
			"checkpoint", "lanes", "clicks", "replaying", "applied", "results", "status", "chat",
			"say" }) do
		if type(native[name]) ~= "function" then
			return nil, "the hook has no " .. name .. "()"
		end
	end
	return setmetatable({ native = native }, Link)
end

-- The hook's table in this state, if the hook gave it one. The hook gives
-- it to a state that has printed, so this prints first. Read through pcall:
-- a state that refuses undeclared globals raises on a missing one.
function bridge.find()
	pcall(print, "[tpf3mp] looking for the hook")
	local ok, value = pcall(function() return tpf3mp_native end)
	if ok then return value end
	return nil
end

-- Hands an action table to the room. Returns true and the action's ticket,
-- which results() names when this game applies the action or never will; or
-- nil and why not: an action that was not handed over must not be applied
-- locally either.
function Link:command(action)
	if type(action) ~= "table" then return nil, "an action is a table" end
	local ok, result, reason = pcall(self.native.command, action)
	if not ok then return nil, "the hook refused: " .. tostring(result) end
	if result ~= true then
		return nil, "the hook refused the action: " .. tostring(reason or "no reason given")
	end
	return true, reason
end

-- In a game script's postUpdate: what became of the batch's action `index`
-- (from 1), and the entity it made, if any.
function Link:applied(index, ok, entity, why)
	pcall(self.native.applied, index, ok == true, entity, why and tostring(why) or nil)
end

-- In the GUI: what became of the player's own actions since the last call,
-- a list of { ticket =, ok =, entity =, why = }, oldest first.
function Link:results()
	local ok, results = pcall(self.native.results)
	if not ok or type(results) ~= "table" then return {} end
	return results
end

-- The room, for the Multiplayer window: { room =, speed =, diverged =,
-- players = { { name =, connected =, owner =, me = } } }, or nil before its
-- game.
function Link:status()
	local ok, status = pcall(self.native.status)
	if not ok or type(status) ~= "table" then return nil end
	return status
end

-- What the room's members said since the last call, oldest first:
-- { { from =, text = } }.
function Link:chat()
	local ok, heard = pcall(self.native.chat)
	if not ok or type(heard) ~= "table" then return {} end
	return heard
end

-- Says `text` to the room for the player: true, or nil and why not.
function Link:say(text)
	local ok, said, why = pcall(self.native.say, tostring(text))
	if not ok then return nil, tostring(said) end
	if said ~= true then return nil, tostring(why or "the hook did not take it") end
	return true
end

-- Reports where the player points over the world plane, or nil when the
-- pointer is lifted. True for building while dragging a build tool.
function Link:cursor(x, y, building, label)
	if self.native.cursor then
		pcall(self.native.cursor, x, y, building == true, label and tostring(label) or nil)
	end
end

-- What other players' pointers are showing:
-- { [player_id] = { x =, y =, building =, label = } }
function Link:cursors()
	if not self.native.cursors then return {} end
	local ok, cursors = pcall(self.native.cursors)
	if not ok or type(cursors) ~= "table" then return {} end
	return cursors
end

-- The actions the room ordered for this update, as a list, or nil; and who
-- sent each, a list of player ids (64 hex digits) beside it.
function Link:take()
	local ok, actions, origins = pcall(self.native.take)
	if not ok or type(actions) ~= "table" then return nil end
	if type(origins) ~= "table" then origins = {} end
	return actions, origins
end

function Link:log(line)
	pcall(self.native.log, tostring(line))
end

-- What the hook asks of the game, once: { save = name }, { load = name },
-- or nil.
function Link:poll()
	local ok, request = pcall(self.native.poll)
	if not ok or type(request) ~= "table" then return nil end
	return request
end

-- Answers a save the hook asked for.
function Link:saved(name, ok, why)
	pcall(self.native.saved, tostring(name), ok == true, why and tostring(why) or nil)
end

-- A world's GUI started: after a load the hook asked for, the world loaded.
function Link:world()
	pcall(self.native.world)
end

-- The seed for math.randomseed in this update: the room step's, or nil
-- outside the room's steps (or from a hook without it).
function Link:seed()
	if type(self.native.seed) ~= "function" then return nil end
	local ok, seed = pcall(self.native.seed)
	if ok and type(seed) == "number" then return seed end
	return nil
end

-- Whether this update is the last of a batch that ends at a checkpoint:
-- the world's lanes are read now, after it.
function Link:checkpoint()
	local ok, due = pcall(self.native.checkpoint)
	return ok and due == true
end

-- Hands the lanes read at a checkpoint to the hook. Returns true, or nil
-- and why not; lanes not handed over hold the world.
function Link:lanes(lanes)
	local ok, taken, why = pcall(self.native.lanes, lanes)
	if not ok then return nil, "the hook refused: " .. tostring(taken) end
	if taken ~= true then return nil, tostring(why or "the hook refused the lanes") end
	return true
end

-- The player's builds queued in the room's game so far, or nil where the
-- hook cannot take them to the room (the tools then stay refused).
function Link:clicks()
	local ok, clicks = pcall(self.native.clicks)
	if ok and type(clicks) == "number" then return clicks end
	return nil
end

-- In the GUI: the build the module editor queued at click `click` (the
-- count before it), read by the hook, as game scripts see a proposal; nil
-- and why when it did not read; nil when that click was not the module
-- editor's, or the hook has no `built` (it is optional: the module editor
-- then stays refused).
function Link:built(click)
	if type(self.native.built) ~= "function" then return nil end
	local ok, proposal, why = pcall(self.native.built, click)
	if not ok then return nil, "the hook refused: " .. tostring(proposal) end
	if type(proposal) == "table" then return proposal end
	if why ~= nil then return nil, tostring(why) end
	return nil
end

-- In a game script's postUpdate at a checkpoint: the lanes the hook wants
-- dumped entry by entry (docs/HOOKS.md, "Lane dumps"), { step =, lanes = {
-- n, ... } }, once; or nil, and nil from a hook without dumps (`dump` is
-- optional).
function Link:dump()
	if type(self.native.dump) ~= "function" then return nil end
	local ok, order = pcall(self.native.dump)
	if not ok or type(order) ~= "table" or type(order.lanes) ~= "table" then return nil end
	return order
end

-- Hands the hook one entry of a lane dumped. Returns whether it was taken:
-- false once the checkpoint has written its most.
function Link:dumped(lane, entry)
	if type(self.native.dumped) ~= "function" then return false end
	local ok, taken = pcall(self.native.dumped, lane, tostring(entry))
	return ok and taken == true
end

-- The game script begins (true) or ends applying the room's actions.
function Link:replaying(on)
	pcall(self.native.replaying, on == true)
end

-- Whether the room's game runs. A hook that cannot say is taken to say yes:
-- the guard then refuses rather than lets a command through unchecked.
function Link:room()
	local ok, inRoom = pcall(self.native.room)
	if not ok then return true end
	return inRoom == true
end

return bridge
