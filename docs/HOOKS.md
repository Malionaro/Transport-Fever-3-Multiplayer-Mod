# The native hook

The native hook is the small library that runs *inside* the game process. It
captures and cancels player commands, gates the simulation step, controls speed
and save/load, and talks to the [agent](ARCHITECTURE.md#components) over
shared memory. This document specifies the parts built at milestone M0: the
build-signature engine, the per-build profile format, the detour engine, the
shared-memory ABI, and the release-day procedure. Locating and detouring the
actual TPF3 functions comes with the release-day profile (see
[DAY_ONE.md](DAY_ONE.md)); everything the hook needs to do it is here and
tested.

Crates:

| crate | contents |
|---|---|
| `tpf3mp-hookcore` | pattern scanning, per-build profiles + resolution, the x86-64 inline detour engine, a small read-only PE reader |
| `tpf3mp-ipc` | the shared-memory link (this document's ABI) |
| `tpf3mp-hook` | the `cdylib` loaded into the game: platform entry points, profile loading, agent connection |
| `tpf3mp-launch` | starts the game with the hook in that one process, and nowhere else |

## How the hook gets into the game

Only the TPF3-MP launcher puts the hook into the game, into the game it
starts for a room, and for as long as that game runs (D11 in
[DECISIONS.md](DECISIONS.md)). Nothing is installed into the game's folder
and no launch option is set: the game started from Steam is the plain game.

- **Windows.** `tpf3mp-launch` starts the game suspended, writes the hook's
  path into it, and runs `LoadLibraryW` on a thread it creates in the game.
  It checks the thread's result and the game's module list, and only then
  lets the game run. If the hook is not there, it ends the game rather than
  let it run without it. This is how TPF2MP's injector started Transport
  Fever 2 (`--launch`).
- **Linux.** The game starts with `LD_PRELOAD` naming the hook, in its own
  environment only, ahead of anything already preloaded (Steam's overlay).
- **macOS.** Not yet. The game's hardened runtime refuses libraries it did
  not load itself; how to get the hook in is a release-day question
  ([DAY_ONE.md](DAY_ONE.md)).

Before starting the game, the launcher checks four things, and refuses if
any fails:

- the player is in a room;
- the game and the hook are where they should be;
- Steam is running;
- no game it started is still running.

It gives the game `SteamAppId` and `SteamGameId` (3493540), so that the
game does not restart itself through Steam, which would start it without
the hook.

The hook runs only when the launcher started the game. The launcher names
two things in the game's environment:

- its link, `TPF3MP_GAME_LINK`;
- its own process, `TPF3MP_LAUNCHER_PID`.

Without the link the hook returns at once: it writes, hashes and opens
nothing. On Linux and macOS it also returns when its process's parent is
not that launcher. Programs the game starts, a browser opened from the
game for example, inherit `LD_PRELOAD` and the variables; the hook stays
out of them. On Windows nothing the game starts loads the hook.

## Design and the fail-closed rules

TPF3 will be patched often after launch, so the hook never pins raw addresses.
It carries one **profile** per game build. A profile binds a build identity
(executable SHA-256, and optionally file size and PE timestamp) to a set of
named **targets**, each located by a byte **signature** rather than an address.

Resolution is **fail-closed**. `tpf3mp_hookcore::profile::resolve` returns either
a complete, verified target table or a precise refusal, and it installs nothing
on the way to a refusal:

- **Unknown build** - the running executable's identity does not match the
  profile. The hook does not scan at all; multiplayer is disabled.
- **Missing** - a *required* target's signature is not found. Resolution fails
  as a whole, so required hooks are all-or-nothing; a partial install never
  happens.
- **Ambiguous** - a signature matches more than once. Refused, even for an
  optional target: a second, unexpected match is a corruption signal, not
  something to skip.
- **Prologue mismatch** - the signature matched but the exact bytes at the
  target are not the ones the profile expects. Refused.

An *optional* target that is simply absent is recorded and does not fail the
profile. Everything else fails closed. The hook logs the precise reason and
leaves the game untouched.

### Where the resolver scans (production vs. this repo's test)

`resolve` takes a byte slice plus the address its first byte corresponds to, so
it does not care whether those bytes come from a file or from memory.

- **Production**: the hook scans the running process's **mapped, unpacked module
  image** - the bytes the loader (and any DRM stub) produced in memory - passing
  the module's base address as the region base. This is the only correct source
  when a build's code section is packed or encrypted on disk.
- **This repo's static proof** (`tpf3mp-hookcore/tests/tpf2_static_proof.rs`)
  scans the executable **on disk**. That is a development convenience, valid
  only when the build's `.text` is readable on disk (see
  [the TPF2 verification](#what-was-verified-on-the-tpf2-binary)). The call is
  identical; only the byte source differs.

## Signatures

A signature is an IDA-style pattern: two hex digits per fixed byte and `??` (or
`?`) for a byte that may be anything, for example `48 8B ?? ?? E8`. Wildcards
exist so a signature skips the bytes that move between builds - RIP-relative
displacements, call targets, absolute addresses - and matches only the opcodes
and operands that identify the code. A signature must be **unique** across the
scanned region; the scanner reports zero, one, or many matches, and the resolver
treats "many" as a refusal.

Rules of thumb, applied to the TPF2 profile below:

- Prefer register/immediate operands; wildcard every relative or absolute
  displacement.
- Extend the pattern only as far as needed to make it unique. Two functions can
  share a prologue (the two TPF2 menu functions share a seven-`push` opening);
  run the signature to the first distinguishing bytes.
- Keep the **prologue** field free of wildcards: it is the exact code the detour
  engine relocates, and it is re-checked byte-for-byte after the scan.

## The profile format

A profile is TOML. `tpf3mp_hookcore::profile::Profile::from_toml` parses and
validates it.

```toml
name = "Transport Fever 2 Build 35924 (Windows x64)"
image_base = 0x140000000   # informational: what RVAs are relative to
region = ".text"           # informational: the section the resolver scans

[build]
sha256 = "782b904a8f7bbdac1f7a18528f1a5c778691e5aa3087c37c351bf6912585175c"
size = 72843280            # optional; checked when present
pe_timestamp = 0x675ABCC6  # optional; checked when present

[[target]]
name = "GameSim::Step"
signature = "40 53 41 56 48 83 EC 68 48 8B DA 4C 8B F1 48 81 FA E8 03 00 00"
offset = 0                 # bytes from the match to the target (default 0)
prologue = "40 53 41 56 48 83 EC 68 48 8B DA 4C 8B F1 48 81 FA E8 03 00 00"
required = true            # default true
```

- **`signature`** locates the target. **`offset`** (signed, default 0) is added
  to the match position to reach the target address, for the case where a
  signature must begin before or after the function it names.
- **`prologue`** is the exact, wildcard-free bytes expected at the target; the
  resolver verifies them and the detour engine relocates them.
- **`required`** (default `true`): a required target that does not resolve
  refuses the whole profile.

A resolved target's address is `region_base + match_index + offset`.

## The detour engine

Trampolines are allocated within 1 GiB of their target, so a relocated
RIP-relative operand keeps a 32-bit displacement to the data it addresses.
On Windows the free regions around the target are walked; on Unix, mmap
hints step away from it. A trampoline is written while read-write, then
turned read-execute; it is never writable and executable at once. On
Windows, the prologue read stops at the end of the target's memory region.
A relocation that still cannot reach fails the install cleanly
(`DetourError::Encode`); it never produces wrong code.

`tpf3mp_hookcore::detour::InlineDetour` is an x86-64 inline hook. Installing it
overwrites a function's first instructions with a jump to a replacement, after
copying those instructions into a **trampoline** that ends by jumping back into
the function; calling the trampoline therefore runs the original.

- **Relocation.** The stolen prologue is decoded and re-encoded at the
  trampoline's address with iced-x86's block encoder, so a RIP-relative operand
  keeps addressing the same absolute memory from its new home. A prologue that
  cannot be relocated - it branches, returns, or does not decode - is **refused**
  (`DetourError::UnsupportedPrologue`) rather than patched wrong. Only
  straight-line instructions are stolen.
- **Patch form.** A near replacement (within 2 GiB) is reached with a 5-byte
  `jmp rel32`; otherwise a 14-byte `jmp [rip+0]` absolute jump. The trampoline
  always returns with an absolute jump, so it works at any distance.
- **iced-x86, not `retour`.** `retour`'s stable line is 0.3.1 (0.4 is alpha) and
  it owns trampoline allocation and instruction relocation internally - exactly
  the part that must be inspectable and testable on a binary that shifts every
  patch. iced-x86 is a pure-Rust decoder/encoder with no build script; the
  engine drives decode/relocate directly and keeps the trampoline and patch
  bytes in this crate, where tests read them.
- **Architecture.** The engine is x86-64 only. On any other architecture it
  compiles to a stub that returns `DetourError::UnsupportedArchitecture`, so the
  workspace still builds and the caller fails closed (see
  [macOS arm64](#macos-arm64)).

### Thread-safety assumptions

Installing overwrites up to fourteen live code bytes with a non-atomic copy. The
caller must guarantee the target cannot execute during install or uninstall:
**install before the target's first run**, or **park every thread that could
reach it first**. The launcher loads the hook before the game's entry point
runs (into the suspended game on Windows, by `LD_PRELOAD` on Linux). The hook
installs from its bootstrap thread while the game starts, as TPF2MP's injector
let its worker do: the targets run only once a world is loaded, long after.
The engine does not stop threads itself. Detours are removed by
dropping the handle (or `detach`), under the same quiescence rule. The engine's
own tests only ever hook functions inside the test binary, never another
process.

## The `tpf3mp-ipc` ABI

The hook and the agent share one memory mapping: a fixed 64-byte header followed
by two single-producer/single-consumer ring buffers. This section is
byte-exact, because the agent is written separately.

### Object naming and security

The logical link name is mapped to a per-user OS object:

- **Windows**: `CreateFileMappingW`/`MapViewOfFile` in the per-session `Local\`
  namespace, object name `Local\tpf3mp.<hash>` where `<hash>` includes the user
  name. No explicit security descriptor is passed, so the mapping gets the
  process token's default DACL - access for the creating user and SYSTEM only.
- **Linux/macOS**: `shm_open`/`mmap` with mode `0600` (owner only). The name is
  `/tpf3mp.<hash>`, where `<hash>` includes the uid; it is kept within macOS's
  31-character `shm_open` limit.
- **Other users** cannot reach the link. **The same user's processes can.**
  Creation is not exclusive, because the agent re-creates the mapping after
  a restart while the game still holds it (see "Restart"). A process of the
  same user could therefore create the object first, or write into it. That
  process can already debug or inject into the game, so this opens no new
  boundary. Each side still treats the other's bytes as hostile:
  - `open` refuses sizes `create` would refuse;
  - no frame is read beyond its ring;
  - a producer refuses a consumer index claiming more than the ring holds;
  - `next_len` never exceeds `max_message`.

  A hostile peer can garble or stall the link, but cannot make this side
  read or write out of bounds (`tpf3mp-ipc/tests/poc_hostile_peer.rs`).
  Nor can it make the agent take in, and so delete, a file other than a
  save in the directory the agent named (`tests/hook_save_path.rs` in
  `tpf3mp-agent`). A game that stops reading makes the agent stop taking
  the room's turns once about 16 MiB wait for it, rather than hold them
  all.

### Header layout (little-endian)

Total mapping size is `64 + 2 * ring_capacity` bytes.

| offset | size | field | notes |
|---|---|---|---|
| 0  | 4 | `magic` | `T3MP` (bytes `54 33 4D 50`), written **last** as a readiness flag |
| 4  | 4 | `abi_version` | currently `1` |
| 8  | 4 | `header_size` | `64` |
| 12 | 4 | `ring_capacity` | bytes per ring; power of two, `<= 2^31` |
| 16 | 4 | `max_message` | largest payload per message |
| 20 | 4 | `session` | non-zero link generation; changes on re-create |
| 24 | 4 | `hook_pid` | 0 until the hook attaches |
| 28 | 4 | `agent_pid` | 0 until the agent attaches |
| 32 | 8 | `hook_heartbeat` | `u64`, bumped by the hook |
| 40 | 8 | `agent_heartbeat` | `u64`, bumped by the agent |
| 48 | 4 | `h2a_head` | hook->agent read index (consumer: agent) |
| 52 | 4 | `h2a_tail` | hook->agent write index (producer: hook) |
| 56 | 4 | `a2h_head` | agent->hook read index (consumer: hook) |
| 60 | 4 | `a2h_tail` | agent->hook write index (producer: agent) |

Then the data areas: hook->agent at `[64, 64 + ring_capacity)`, agent->hook at
`[64 + ring_capacity, 64 + 2 * ring_capacity)`.

### Rings

Each ring is a byte stream carrying length-prefixed messages: a little-endian
`u32` payload length followed by that many payload bytes. Both the length and
the payload may wrap around the end of the buffer.

- `head` and `tail` are **free-running** `u32` counters (they wrap at `2^32`,
  not at the capacity). Bytes in the ring = `tail - head` with wrapping
  subtraction; this is correct because capacity is a power of two `<= 2^31`. The
  index into the data area is `counter & (capacity - 1)`.
- **Producer**: writes the payload bytes, then stores `tail` with **Release**.
  It reads `head` with **Acquire** to compute free space; it only writes `tail`.
- **Consumer**: reads `tail` with **Acquire**, reads the bytes, then stores
  `head` with **Release**. It only writes `head`.
- A message larger than `max_message` is rejected by the producer; a full ring
  returns "full". Nothing is allocated on either side of a send or receive.

### Startup, heartbeat and restart

- **Startup.** One side (in TPF3-MP, the agent) is the owner: it creates the
  mapping, zeroes the header, writes the ABI version, ring sizes and a fresh
  non-zero `session`, sets its pid and heartbeat, and **publishes `magic` last**
  with a Release store. The other side opens the mapping and reads `magic` with
  an Acquire load; until it appears the open returns "not ready". The opener
  then checks `abi_version` and `header_size`, reads the ring sizes, and sets
  its own pid and heartbeat. The hook fails closed (runs solo) if no mapping is
  present.
- **Heartbeat.** Each side bumps its own counter and reads the peer's. A counter
  that stops advancing means the peer is gone.
- **Restart.** The owner re-creates the mapping with a new `session`. A peer
  that sees `session` change knows the rings were reset and drops anything in
  flight, then re-syncs from the new generation. The launcher does this for
  every room session (`begin_session`): leaving one room and creating or
  joining another re-creates the link while the game runs on. On Windows
  the new owner re-initialises the very mapping the game still holds; on
  Linux and macOS the old owner unlinks the name when it lets go, and the
  new link is another object, so a peer finds the next generation by
  opening the link again by name. What the hook does about a new
  generation is in "Following the launcher from room to room" below.

### Several games on one PC

The hook opens the link its launcher names, and keeps its log (`hook.log`)
and build profiles (`profiles/*.toml`) in the per-user `TPF3-MP` data
folder. It also carries the release's own profiles, built in from the
repository's `profiles/` folder (Transport Fever 3 Steam build 40408 on
Windows, so far); a profile in the data folder for the same build comes
first, so one can be tried there without a release. The game's environment says which, so several games on one PC each
reach their own agent:

| variable | effect |
|---|---|
| `TPF3MP_GAME_LINK` | the link name to open, the launcher's `--game-link` (`tpf3mp.default` unless given); without it the hook does nothing |
| `TPF3MP_LAUNCHER_PID` | the process that started the game; on Linux and macOS, the hook does nothing in a process whose parent is another |
| `TPF3MP_DATA_DIR` | the folder for the hook's log and profiles; unset or empty, the per-user one |

`tpf3mp-fakegame` reads `TPF3MP_GAME_LINK` too, when no link is given on its
command line. The multiplayer rig (`tpf3mp-rig`, in
[DEVELOPMENT.md](DEVELOPMENT.md)) sets all three for every game it starts, and starts a real
game with the hook in it as the launcher does.

## The bridge: what travels over the link

`tpf3mp-bridge` defines the messages, postcard-encoded, one per ring frame,
at most 60 KiB each. It has no async runtime or network code, so the hook can
link it. The agent's side is `tpf3mp_agent::bridge`.

- **From the agent (`ToHook`):**
  - `Hello`: always first.
  - `Begin`: a game starts, and saves go in this directory. It also names the
    room's rules: with `native`, the game's own economy runs untouched;
    with canonical rules, the Lua mod shows the server's values instead. And
    the local player, as the room's events name the actor: the hook knows
    the player's own commands by it when the room orders them (bridge
    version 5). And the mods the room's worlds load with in this game:
    the room's shared mods and this player's personal ones, or none when
    the agent does not know this player's mods ([MODS.md](MODS.md);
    bridge version 12).
  - `Load { file, next_step }`: load a world, then run `next_step`.
    Without a file, the game loads the world the player chose to start
    from: the owner's, or everyone's on a server that keeps no snapshots.
    With one, a save the room agreed on: the owner's world at the start
    (the save the owner named for the room, or the one the owner's game
    saved), or the room's latest for a player who joins a running game,
    could no longer resume, or is rebased after diverging. Everything sent before
    a load is void.
  - `Apply(event)`: apply this event before its step. A `Save` event is not
    applied: the session saves the world there (see below).
  - `Release { through }`: steps up to and including this one may run.
  - `Speed`: the room's speed, for display only: sent with the game's
    first turn, whatever the speed, and whenever it changes.
  - `Diverged`, `Refused`: tell the player.
  - `Chat { from, text }`: a member of the room said something. Sent only
    once the game has begun; talk in the lobby stays in the launcher.
  - `Room(RoomInfo)`: the room as it stands (its name, owner, and members
    with their names and whether they are connected), for the game's
    Multiplayer window: sent when the game begins and whenever the room
    changes (bridge version 6).
  - `Lobby(LobbyView)`: the launcher's lobby as it stands (its
    connection, server, the player's name, the room with its invite,
    members, ready marks and owner, the newest 40 chat lines, the last
    error and notice), for the Multiplayer window on the game's main menu
    (D17): sent whenever it changes, before, during and after a room's
    game; only the newest counts (bridge version 9). Since bridge version
    10 it also carries the rules the server offers, the player's saves
    (newest 40, by name) and the one offered first (`start_save`), where
    the room's world is in this game (`world`: none, fetching with its
    bytes, loading, playing) and how the game differs from the room's.
    Since bridge version 14 it carries the page of the server's public
    rooms last asked for (`rooms`). Since version 16 each member carries the banner they
    picked, and the view the player's own (`banner`); `SetBanner` sets it
    (`set_banner` from the window, empty for the default). Since version 17 the room, the
    room list and create carry the play style (`competitive`). Since bridge version 15 it carries the
    server's address (`server_address`) and the launcher's default server
    (`server_default`), for the server setting.
  - `End`: the session is over. Sent only once the room's game has begun:
    a room left before that ends nothing in the game, which keeps its link
    for the player's next room.
- **From the hook (`ToAgent`):**
  - `Hello`: always first, with the game build.
  - `Loaded { next_step }`: the ordered world is loaded.
  - `Command { payload, secret }`: the player acted; the room orders it.
    `secret` is a company's password the action needs (joining or locking
    a company), which the agent sends beside the intent and the room seals
    (PROTOCOL.md, "Secrets"); the hook never logs it. Bridge version 11,
    which also carries each ordered `Command`'s seal to the hook.
  - `Ran { step }`: the game ran this step.
  - `Checkpoint { step, lanes }`: digests at a checkpoint.
  - `Saved { event, lanes, file }`: the world as saved at a save event, and
    its digests there; no file if saving failed. The file must be in the
    directory `Begin` named; the agent takes in no other.
  - `Chat { text }`: the player says something to the room
    (`Session::chat`).
  - `Speed { speed }`: the player picked this speed in the game's speed
    row (`Session::request_speed`); the agent asks the room, which takes it
    from the owner only. Bridge version 4.
  - `WorldUp { world }`: before the room begins a game, the game has a
    world up with the mod linked, and steps it (`Session::world_up`).
    `world` counts the worlds whose GUI started (`tpf3mp_native.world`)
    since the hook began, so each is told once. In the room's lobby the
    agent marks the player ready (`Request::SetReady(true)`, as the Ready
    button does), once a world: a player who then presses Not ready stays
    so until another world is up. Nothing is marked once the room's game
    began, while the game's world is being replaced, or before the agent
    knows the room is in its lobby; the step gate sends nothing after
    `Begin`. Bridge version 7.
  - `MenuUp { menu }`: before the room begins a game, the game is at its
    main menu with no world up, and will load the room's world from there
    (`Session::menu_up`; "Loading from the main menu" below). `menu`
    counts the game's arrivals at its menu since the hook began; each is
    told once per room session, so again to the launcher's next room. In
    the room's lobby the agent marks a player other than the room's owner
    ready, once per arrival, if it keeps worlds. The owner only once the
    room has the save the owner named for it to start from
    (`BridgeOptions::start_world`, the launcher's `--start-save`); without
    one never, as the owner's game must have the world everyone plays up
    to save it for the room. Otherwise as `WorldUp`. Bridge version 8.
  - `Lobby(LobbyAction)`: the player pressed a button of the main menu's
    Multiplayer window: connect (a name; the server is the launcher's,
    D12), disconnect, create, join, ready, start, kick, chat or leave. The
    launcher carries it out as if its own window had asked (bridge
    version 9). Since version 10, create also names the rules and the save
    the room starts from: one of the saves the window was offered, by name
    only (never a path); absent for the launcher's own `--start-save`,
    empty for none. Since version 14, create also carries a room's
    listing (its climate and year, for a public room) and `ListRooms {
    page }` asks for a page of the server's public rooms. Since version 15,
    `SetServer { server }` is the player's server setting: a `host:port`,
    or empty for the launcher's default; the launcher checks, remembers
    and reconnects, and refuses it in a room (D12, proposed amendment).
    Connect still names no server, and an invite never switches it.
  - `Log`: a line for the agent's log.
- **The step gate.** The game asks the hook's `Gate` before every step. Until
  the step is released, the hook reads messages and applies each event the
  gate hands over, so an event for step `s` is applied after step `s - 1`
  and before step `s`, never mid-step.
- **Ordering.** The agent sends every event for step `s` after the release of
  step `s - 1` and before the release of step `s`. It only merges releases
  of consecutive steps with no event between them. The hook stops reading
  once its next step is released. The gate refuses anything that breaks
  this: an event for another step, an event after its step's release, or a
  release that goes back. The hook must then stop following and say so.
- **Pacing.** The agent releases steps on its jitter-buffered schedule
  (`Playout`). The game runs a released step at its own speed and waits at
  the gate for the next one. It reports each step it ran; the agent reports
  progress to the server from that, at most every 20 ms.
- **Liveness.** The hook must beat its heartbeat from a thread of its own,
  since the game thread blocks while loading. The agent gives up on a hook
  whose heartbeat stands still for 60 s, or 10 minutes while the world
  loads (`BridgeOptions::hook_timeout`, `load_timeout`), and ends the
  session with "the hook stopped responding". A game the launcher started
  is also watched as a process: once its hook attached, the launcher tells
  the bridge within half a second of the process exiting
  (`Control::GameClosed`), and the session ends the same way with
  "Transport Fever 3 closed", without waiting out those limits, so the
  player can start the game again at once. The heartbeat limits remain for
  games the launcher did not start and for a hook whose game still runs
  but hangs.

### The hook's session

`tpf3mp_bridge::Session` is the hook's whole side of the link, run on the
game thread. The game-specific part of the hook implements the `Game` trait
and calls the session from its detours:

- **Startup.** `Session::attach(DEFAULT_LINK, build, patience)`, then
  `wait_for_begin()`. The gate's first answer is `StepGate::Load`.
- **Loading.** Whenever `before_step` or `poll_step` answers
  `StepGate::Load(load)`, replace the world: with the save `load.file`, or
  without one with the world every player starts from. Then call
  `loaded(load.next_step)`. Call `heartbeat()` while loading.
- **Before each simulation step.**
  - `before_step(&mut game)` blocks until the room releases the step,
    calling `Game::apply` for each event on the way. A pause can hold it
    there for as long as the pause lasts.
  - A game whose simulation shares a thread with its rendering, which must
    never block, calls `poll_step` instead. It returns `Wait` until the
    step is released, and the detour skips the step for that frame.
- **After each step.** `after_step(&mut game)` reports it, and at
  checkpoint steps sends `Game::lanes()`.
- **Saving.** At a save event the session calls `Game::save(file)`, then
  `Game::lanes()`, and reports both. The save must hold everything needed
  to continue from that point, because it is what other players load. The
  agent cuts it into its chunk store and deletes the file.
- **When the player acts.** Capture the action before the game applies it
  locally and call `command(payload)`. For a build the payload is the
  action the Lua mod handed over as a table, converted by
  `tpf3mp_proto::lua` and encoded with `Action::to_payload` ("The action
  schema" in [BUILDING.md](BUILDING.md)). The action happens only when the
  room's event comes back through `Game::apply`, on every replica alike.
- **Notices.** `Game::notice` receives speed changes, refusals,
  divergences and the end of the session, for the game's UI.

The session gives up (`AgentGone`) only when the agent's heartbeat stands
still for its patience, never merely because a step is withheld.

#### Following the launcher from room to room

A game started from the launcher outlives the room it was started in: the
player may leave the room and create or join another while the game runs
on. Until the room's game begins (`Begin`), the session follows:

- `End` before `Begin` means the room session is over, not the game: the
  session stops reading that link and does not check that agent's
  heartbeat any more. A link whose `session` changes before `Begin` means
  the same, whether or not the `End` was read first.
- It then opens the link again by name, at most every 100 ms, until it
  finds a new `session`. There it exchanges hellos as `attach` does: its
  own `Hello` first (then a `Log` line saying it followed), and the new
  agent's `Hello` must be the first thing it reads; anything else is
  refused. Then it waits for that room's `Begin`. The agent needs nothing
  new for this: each room session is a fresh `Bridge`, which expects the
  hook's hello first, exactly once.
- Until the next room is created the game runs on its own, as before any
  room. `try_begin`, which the game's step polls, waits for as long as that
  takes; the blocking `wait_for_begin` gives up after the session's
  patience (`AgentGone`).

Everything else still fails closed. A second `Hello` on the same link
generation is refused, and once a game has begun a new generation is
never followed: every read then checks the link's `session`, and a change
is `SessionError::LinkReset`, on which the hook holds the world. The
bridge's messages did not change for this, so `BRIDGE_VERSION` stays.
(`session::tests` in `tpf3mp-bridge` play these through over a real link.)

`tpf3mp_testkit::fake_hook` implements `Game` for the toy game and runs it
through `Session`: the exact code the real hook will run, over the real
link. The `games_behind_the_bridge_and_gate_agree` scenario runs three of
them in one room end to end; others have a player join a running game,
rebase a replica that drifted, and hand a world on across a server restart.
`tpf3mp-fakegame` does the same as a separate process, for trying the stack
by hand (see [DEVELOPMENT.md](DEVELOPMENT.md)). On release day, what remains for TPF3 is:

- the build profile with its signatures;
- the detours that call the session;
- `Game` for the real world: applying an event means executing the player
  command it carries, the lanes are digests of the game state, and saving
  and loading use the game's own save format;
- checking that a save is complete and loads on every platform
  (DAY_ONE.md);
- checking a received save's script data before the game loads it
  (`tpf3mp_agent::save_check`, DAY_ONE.md section 7);
- on the server, a `Ruleset` that validates TPF3's command format and
  applies the canonical economy, with `save` and `restore` so its rooms'
  logs compact (`crates/tpf3mp-server/src/ruleset.rs`). It is added to
  the server's `RulesMenu` next to `native`, which stays offered.

### The main menu's Multiplayer window

The room's lobby is in the game (D17, as amended on 2026-09-30): the
Multiplayer entry on the game's main menu (docs/LOBBY.md) opens a window
that connects, creates or joins a room, shows its players and their ready
marks, chats, gets ready and, for the owner, starts the room's game. It
drives the launcher that started the game, which still holds the
connection (D11): the window is only another front end of the launcher's
[`Action`s](../crates/tpf3mp-agent/src/launcher/api.rs).

- **One link for the game's life.** The launcher opens the game's link
  when it starts, not when a room begins, so the game can start before a
  room is chosen. While no room session holds the link, the launcher
  serves it itself (`launcher::lobby::IdleLink`): it answers the hook's
  hello, sends the lobby whenever it changes and carries out the window's
  actions. A room session's bridge takes the link over already greeted
  (`Bridge::greeted`), passes the lobby both ways (`BridgeOptions::lobby`)
  and gives the link back when it ends (`Bridge::into_link`).
- **Reading the link at the menu.** At the main menu no step of the game
  runs, so nothing else reads the link. The window asks the hook for the
  lobby a few times a second; each request exchanges it through the step
  driver (`StepDriver::lobby`): the window's actions go out as
  `ToAgent::Lobby`, and `Session::poll_lobby` reads what came in, keeping
  the lobby, and stops at the first message the game must see at its gate
  (the room's `Begin`), which `try_begin` takes when a world steps. In the
  room's game the gate reads the link and keeps the lobby it finds
  (`Gated::Lobby`); the lobby never holds the world or reaches the
  in-game Multiplayer window's notices.
- **Fail closed.** A hook whose step gate did not install has no driver:
  the window says the game has no link to the launcher and takes no action.
  A menu the hook cannot reach (its targets missing) stays the game's own,
  and the launcher's window still does everything.
- **One room's game per game.** A room left before its game began ends
  nothing: the game follows the player into the next room. After a room's
  game has ended, the window still shows the lobby, but the next room's
  game needs the game started again from the launcher.

### The step gate in the game

`tpf3mp-hook` detours the game's `GameSim::Step` (TF3 build 40408:
`0x159390`, from the built-in profile) and hands every call to
`step::StepDriver` (`crates/tpf3mp-hook/src/step.rs`, `install.rs`).

What one call of `GameSim::Step` does decides the design. In TPF2 and TF3
alike it is one batch of the game's own pacing: the main thread
(`CGame::Step`/`CGame::Sync`) calls it on its own schedule, 5 times a second
at 1x, and it runs as many simulation updates as its one call of the speed
getter answers: none while paused (the game's own paused path), one at 1x,
four at 4x. TF3 also adds a pending count, fed by the debug command
`makeGamePerformSimulationStepsCmd` and capped at 64 a call (the global at
`0x403b8e0`). The renderer interpolates from each batch, so **every call
must run**: a call skipped looks like a batch that ran without the world
moving on, the render clock goes back, and TF3 fails
`data.emitCount >= .0f && data.emitCount < 1.0f` in
`UI::particle_manager_util::UpdateParticleSystemInstance` (a negative
particle time step) within seconds of starting a game. The hook therefore
runs the game's step exactly once per call and chooses the answer to its
call of the speed getter, as TPF2MP's speed hook
(`tpf2-multiplayer/native/src/speedhook.cpp`) patched TPF2's step:

- **Before the room begins a game**, and **after it ends**, the game's own
  speed.
- **In the room's game**, the steps the room has released
  (`Session::poll_step`, never blocking the game's thread, then
  `Session::batch`, which reads on through the agent's one-step releases),
  at most 16 a call and never past a checkpoint step, whose lanes must be
  the world's right after it; the call then reports each (`after_step`).
  A room faster than the game's own pace catches up that way, up to 16x.
  When the room withholds the next step (paused, or a player behind), 0:
  the game's paused path, and the world stands still.
- **The room's world.** A `Load` without a file (the world every player
  starts from) takes the world the game has loaded. A `Load` with a save
  file, and a `Save` the room orders, go through the mod's GUI ("The
  room's world" below), every call answering 0 until they are done. Any
  error (the agent gone, a malformed message, a failed report, a world
  that did not load) **holds** the world for good: every call answers 0,
  rather than run apart from the room's (fail closed).
- **Only the step's own call is changed.** The profile's target
  `GameSim::Step/GetSpeed call` (`0x1593ee`) is the step's one call of
  `CGameTime::GetSpeed` (`0x2a95a0`); the hook redirects that call
  (`hookcore::detour::CallRedirect`, which checks the call targets the
  getter) to answer the chosen count. The getter's six other callers
  (`CGame::Sync`, the UI, the camera, the particles) read the game's own
  speed: telling them 1 when the speed row says otherwise also trips the
  particle assertion. The game's own speed and pause therefore change
  nothing in the room's game but the display, until the room follows them.
  Nothing may send the debug step command during a room: its pending count
  would add updates to a call.
- **The speed row follows the room.** The Lua replacement
  `gui/tpf3mp/speed_control.script.lua` highlights the accepted speed for guests from
  the hook's room status, including pause. Guests see disabled buttons
  with "Host controls speed" help; their keyboard speed shortcuts are
  disabled through both `game.tl`'s internal `game_react_globals` table
  and the public module that copies its functions. It copies feature
  flags instead of changing the game's table, preserving mission/mod
  restrictions. Becoming host or leaving the room restores the stock
  recipe and shortcuts, including the host's keyboard hints. Updating the display
  never sends `makeGameSetSpeedCmd`: feeding the room's speed back into
  the game's local value would be mistaken for a player's request.
- **The host's speed row asks the room.** The getter's detour only reads: the
  game's own speed, the speed row's value, every time the game asks
  (`CGame::Sync` and the game UI ask every frame, paused or not). When it
  changes in the room's game, the hook sends `ToAgent::Speed` and the agent
  asks the room (`Request::SetSpeed`): the owner's choice sets the room's
  speed for everyone, and anyone else's is refused, shown as a notice. The
  value found on entering the room's game is not sent, so joining never
  resets a room's speed. What the room says back (its speed, a refusal, the
  end) goes to the hook's log.

Measured in TF3 build 40408 on release day (the rig with one player,
`app.startGame()` from the console, the game's speed set with
`makeGameSetSpeedCmd` as the speed row does, `GameTime.updateCount` read
before and after): 4.97 updates a second at 1x, 20.34 at 4x, 10.02 at 2x,
none paused, 4.98 back at 1x, the room confirming each change within a
second, and no assertion in several minutes of play.

The hook installs the detours from its bootstrap thread while the game
starts, before any world is loaded, so no thread is inside the step when
it is patched. It resolves the profile in the game's own mapped image
(not the file on disk), attaches the `Session` to the launcher's link, and
calls the game's step through the detour's trampoline with all four
register arguments passed through unchanged. A panic in the detour sets it
to hold. Tests: `step::tests` drive the driver against a scripted room;
`install::tests` detours a stand-in step in the test binary and checks it
runs exactly once per call with the count chosen and its arguments
untouched; `session::tests` read releases ahead over a real link;
`detour::x86_64::tests` redirect one call in hand-written code.

### The Lua side

The mod runs in two kinds of Lua state: the game's GUI state, started by a
game bar plugin on the first frame of a game
(`mod/tpf3mp_1/content/gui/tpf3mp/`), and the engine states the game runs
game scripts in (`mod/tpf3mp_1/content/tpf3mp_sim/`, "Actions in the
game" below). The hook and the mod meet through one global table,
`tpf3mp_native`, which the hook gives every Lua state that calls `print`:
it detours Lua's `luaB_print` (profile target `luaB_print`) and, after the
game's print, adds the table to that state's globals, once, set raw past
any metatable a strict state gives them. The mod prints before it looks
for the table (`bridge.find`). Its contract is in
`mod/tpf3mp_1/content/scripts/tpf3mp/bridge.lua`; the hook's half is
`crates/tpf3mp-hook/src/lua.rs`:

- `tpf3mp_native.version`: 12. The mod refuses any other.
- `tpf3mp_native.note(key[, value])`: a short string one of the game's Lua
  states notes for the others (at most 16 keys, 512 bytes each; "" forgets
  it); with only a key, what was noted, or nil. The GUI runs in more than
  one Lua state, and a game script's GUI half reads there what the
  construction menu knows (the stop tool's stop, "The build tools").
- `tpf3mp_native.command(action, password)`: an action table, in the game's units.
  The hook reads it into a `tpf3mp_proto::lua::LuaValue`, within
  `MAX_DEPTH` and `MAX_NODES` (a function, userdata or a table as a key is
  refused), converts it with `action_from_lua` and queues
  `Action::to_payload`; the step gate hands it to `Session::command`, in
  the room's game only. It returns `true` and a ticket, or `false` and why.
  `false` or an error means refused, and the mod does not apply the action
  locally either: every game applies it when the room orders it. The
  ticket comes back in `results()`. `password`, optional, is a company's
  password, 1 to 64 bytes of text: only joining or locking a company takes
  one, scoped to that company (`tpf3mp_proto::Secret`); it goes to the room
  with the action and is never logged, and no refusal quotes it (version
  12).
- `tpf3mp_native.take()`: the actions the room ordered for this simulation
  update, as `action_to_lua` tables, or `nil` (below), and second, who
  sent each, a list of player ids (64 hex digits) beside it ("Companies"
  below), and third, each one's seal, `{ scope =, tag = }` (the tag as 64
  hex digits), or `false` (version 12). A list's items are
  in its table's array part, so `next` walks them in order. The game
  copies a list it is handed (a stop's loading flags, a consist's groups)
  into its own vector in the order `next` gives, and what a game script's
  `update` returns reaches `postUpdate` as the game's own copy, whose
  lists `next` walks in hash order all the same (build 40408: a bus line's
  stops set to load grain, one cargo over from passengers). So `apply.lua`
  hands the game every such list afresh, filled in order (`seq`).
- `tpf3mp_native.log(line)`: a line for `hook.log`, marked `mod:`.
- `tpf3mp_native.poll()`: in the GUI, every frame: what the hook asks of
  it, once, `{ save = name }` or `{ load = name }`, or `nil` ("The room's
  world" below).
- `tpf3mp_native.saved(name, ok, why)`: the GUI's answer to a save.
- `tpf3mp_native.world()`: a world's GUI started. Before the room begins
  a game, the step gate's next call tells the agent the latest such world
  (`lua::take_world_up`, `ToAgent::WorldUp`), which marks the player ready.
- `tpf3mp_native.room()`: whether the room's game runs (the step gate's
  phase, held included), for the guard ("The player's commands" below).
- `tpf3mp_native.checkpoint()`: in a game script's update, whether it is
  the last of a batch that ends at a checkpoint step ("The world's lanes"
  below).
- `tpf3mp_native.lanes(t)`: the lanes read there, a table from lane
  numbers to strings. Returns `true`, or `false` and why.
- `tpf3mp_native.clicks()`: the player's builds queued in the room's game
  so far, or `nil` where the hook cannot take them to the room ("The build
  tools" below).
- `tpf3mp_native.built(n)`: in the GUI: the build the module editor
  queued at click `n`, read by the hook, as game scripts see a proposal,
  once; `nil` and why when it did not read; `nil` when click `n` was not
  the module editor's ("The module editor" below). An optional function:
  the bridge's version stays 9, and a mod or hook without it keeps the
  module editor refused.
- `tpf3mp_native.replaying(on)`: the game script begins or ends applying
  the room's actions, whose builds the hook lets through.
- `tpf3mp_native.applied(index, ok, entity, why)`: in the game script's
  `postUpdate`, after the batch's action `index` (from 1): whether it went,
  the entity it made, if any, and why not.
- `tpf3mp_native.status()`: in the GUI: the room for the Multiplayer
  window, `{ room =, speed =, diverged =, players = { { name =,
  connected =, owner =, me = } } }`, or nil before the room's game. The
  hook keeps what the room tells it (`Room`, `Speed`, `Diverged`), and
  forgets the divergence when a world loads (bridge version 9).
- `tpf3mp_native.chat()`: in the GUI: what the room's members said since
  the last call, `{ { from =, text =, old = } }`, oldest first, 64 lines
  at most. A world's GUI starts with none of the chat so far, so after
  `world()` the next call first gives the last 50 lines taken before
  again, with `old` set.
- `tpf3mp_native.say(text)`: in the GUI: says `text` (280 bytes at most,
  trimmed) to the room for the player, `true` or `false` and why; the step
  driver sends it (`Session::chat`) in the room's game only.
- `tpf3mp_native.results()`: in the GUI: what became of the player's own
  actions since the last call, `{ { ticket =, ok =, entity =, why = } }`,
  oldest first. The step driver knows the player's own actions when the
  room orders them back: the room's event names the player (`Begin`'s
  `player`) and the client sequence number, which `Session::command`
  returned when the driver handed the action over, and which it keeps with
  the action's ticket. An action the room refuses (`Notice::Refused`), or
  one handed over when no room's game runs, answers `ok = false` with why.
- `tpf3mp_native.dump()` and `tpf3mp_native.dumped(lane, entry)`: in a
  game script's `postUpdate` at a checkpoint: the lanes the hook wants
  written to its log entry by entry, and each entry ("Lane dumps" below).
  Optional, as `built`.
- `tpf3mp_native.cursor(x, y, building, label, curves)` and `tpf3mp_native.cursors()`:
  in the GUI: reports the player's world-plane pointer, active build preview
  position, and optional Hermite curve geometry for linear builds, and reads
  other room members' active cursors, carried as advisory datagrams. Optional
  in the contract.
- `tpf3mp_native.mods(list)`: the mods to load a save with, given the
  save's (names, one a line): that list, then those left out and those
  added, the same way, from the room's `Begin` (`tpf3mp_bridge::mods::plan`);
  `nil` without the room's lists. `mods()` alone: `true` when the room gave
  them. Said in the hook's log ("the room's world loads with N mods: ...").
  Optional, as `built` ([MODS.md](MODS.md)).
- `tpf3mp_native.personal()`: this player's personal mods, names one a
  line, or `nil`: the guards tell a personal mod's commands by it
  ("The player's commands" below). Optional.

The table's functions run on whichever thread runs their state (the GUI's
the main thread, the game scripts' a pool of simulation threads) and share
nothing but the hook's queues. They reach Lua through Lua 5.2's C API as
the build profile names it: 18 functions besides `luaB_print`, found from
`luaB_print`'s own calls and their places in lapi.c, which the linker laid
out alphabetically (TPF2's were too; several prologues match TPF2's byte
for byte). They are only called, never detoured. Everything that crosses
into Lua is `C-unwind`: a Lua error, which TF3 raises as a C++ exception,
passes through as the game raised it.

Without the table, the mod logs "no hook in this game" and does nothing.
That is every game Steam started (D11).

### Actions in the game

A player's action happens in no game until the room orders it, and then in
every game in the same simulation update:

1. The mod hands the action to `tpf3mp_native.command`. The step gate
   sends it to the room.
2. The room orders it as an event for a step `s`. The session ends a
   batch before every step with events (`Session::batch`), so `s` is
   always the first update of a batch; the driver hands that batch its
   actions (`lua::begin_batch`), and runs it.
3. The mod's game script asks the hook in every `update`
   (`tpf3mp_native.take`); the first update of the batch gets the actions
   and returns them, and its `postUpdate` applies them through `api.cmd`
   (`mod/tpf3mp_1/content/scripts/tpf3mp/apply.lua`).
4. After the batch, the driver checks the actions were taken
   (`lua::end_batch`). If they were not, the world ran step `s` without
   them: none of those steps is reported and the world stands still
   (fail closed).

Measured on build 40408:

- A mod's game script (`*.gs.lua`, discovered by the game) has its
  `update` called once per simulation update: `GameTime.updateCount` goes
  up by one from call to call, and `dt` is 0.2. Its script file defines
  `data()` returning the functions, as the GUI's do.
- The game runs game scripts on a pool of Lua states ("Sim Pool" threads),
  so a script's locals are kept per state, not per game.
- The game's own scripts decide in `update` and change the world in
  `postUpdate`, which the game calls with what `update` returned, and not
  when that is nil: the company script reads its argument unchecked, and
  a lane read from a `postUpdate` after an update that returned nothing
  never reached the hook. The mod's game script does the same, so the
  world changes only in `postUpdate`, not while other scripts' updates may
  run beside it.
- In an engine state a command runs at once (the game's
  `api/tealdef/api/cmd.d.tl`). The game takes no callback in `update`
  ("Callbacks are currently disallowed"), but in `postUpdate` it calls one
  at once, with the command's result: the game's mission scripts read the
  line they made from it right after `sendCommand`
  (`mission_vehicle_util.tl`). `apply.lua` sends every command with one
  there: a command the game answers as failed fails the action in every
  game, and the entity a command made (`resultEntity`,
  `resultVehicleEntity`, else the first of its result entities) is what
  the action made. A state where the game refuses the callback sends
  without one, logs so once, and leaves finding what an action made to
  the registry. A refused command raises. In the GUI and console states
  commands run "in the next simulation step", which is a different update
  on each game: the reason the game script applies them, not the GUI.
- The console has a Lua state of its own. For tests, its
  `api.cmd.makeScriptingSendEventCmd("", "tpf3mp", "command", action)`
  reaches the game script's `handleEvent` in that game only, which hands
  the action to the room.

Seen in two hooked games in one room (the rig, the fixture save): a road
depot sent from the first game's console was handed to the room, and both
games applied it in the same update; the determinism probe's construction
lane changed between steps 500 and 600 in both, to the same value, and
matched at every sample after.

Seen on build 40408, two games through the deployed server: a loan
taken in the guest's finance window went to the room, and both games
booked it in the same update, the account and the loan script's list of
loans alike in both; the loan script's own callback, which books the
money, ran in the game script's `postUpdate`.

`apply.lua` applies, among others:

- `BuildConstruction`: a `SimpleProposal` with one `ConstructionEntity`
  (the file, the matrix from the transform, the parameters from their
  flattened paths, the name, the player) and, in its street proposal, the
  construction's connection, resolved as a road build's polyline, less
  the construction's own entrance (a new vertex at the end of a single
  link), sent with a `Context` naming the
  player, who pays, and gathering the town buildings and fields in its
  way, and `playerInitiated` true, as the player's own build. The game's
  verdict comes first (`makeProposalData`): a critical error refuses the
  build, with its reasons, in every game; warnings (town buildings to
  remove, reputation lost) are logged and built through, as the tool
  builds once the player clicks (`ignoreErrors` true: with it false the
  game dropped such a build without a word, seen on build 40408);
- `Loan`: the loan script's own event, `makeScriptingSendEventCmd("",
  "Loan", "Obtain", { next, offer })` or `"Repay", { nil, loan }`, with the
  tables the finance window sends;
- `Prospect`: the company script's own event,
  `makeScriptingSendEventCmd("", "Companies", "spawnIndustry", {
  companyEntity, townEntity, types, permitKey, cargoType })`, with the
  player's company, the town the registry names, and the industry types in
  the order the action carries them ("Prospecting" below).

Every other action is refused with a line in `hook.log`, the same on every
game, so the worlds stay alike. The native build tools come next.

### The room's world

A room plays its owner's world. On a server that keeps worlds, the owner
names a save before the room starts (the launcher's `--start-save`), which
the owner's agent reads from the game's save folder and hands to the room
in its lobby: no game is involved, every game waits at its main menu, and
all of them, the owner's too, load that save from there when the room
starts. Without one, the room has the owner's game, with its world up,
save it before step 1, and every player's game loads that save. A player
who joins later loads the room's latest one ("The first world" in
[PROTOCOL.md](PROTOCOL.md)). Transport Fever 3 saves and loads only through its GUI's
script API (`app.saveGame`, `app.loadGame`) and only in its own save
folder, so the hook asks the mod's GUI for both
(`crates/tpf3mp-hook/src/worlds.rs`,
`mod/tpf3mp_1/content/gui/tpf3mp/tpf3mp.script.lua`), and the world
stands still meanwhile:

- **Saving.** The session answers `StepGate::Save(order)` until the save
  is reported (`Session::saved`). The driver asks the GUI to save under a
  name of this game's own, `tpf3mp_<pid>_<event>`, as two games on one PC
  share the folder. The GUI calls `app.saveGame(name, callback, false,
  true)` (the last argument leaves the player's own save name alone) and
  answers through `tpf3mp_native.saved` from the callback, once the file
  is written. The hook finds `<name>.sav`, removes the picture the game
  writes beside it, and moves the save to `order.file`, which the agent
  cuts into its store. A save the game refuses, or does not make within
  `SAVE_PATIENCE` (120 s), is reported failed, and the game goes on.
- **Loading.** A `Load` with a file (the room's save, fetched by the
  agent) is copied into the folder as `tpf3mp_room_<pid>.sav`, and the GUI
  asked to load it (`app.loadGame`, with a `SavegameId` in the game's save
  namespace). With the room's mod lists (`Begin`), the GUI first reads the
  save's details (`app.getSavegameInfo`, a few frames) and loads it with
  their `mods` replaced: the save's shared mods, TPF3-MP, and this
  player's personal mods, leaving out another player's
  (`mod/tpf3mp_1/content/scripts/tpf3mp/worldload.lua`; [MODS.md](MODS.md)).
  A shared mod not installed here fails the load, and says which. The GUI tells the hook each time a world's GUI starts
  (`tpf3mp_native.world`); the room's world is the first to start after
  the GUI took the request, never the one it was asked in, whose GUI may
  well report itself in between. Then `Session::loaded(next_step)`, and
  the room's steps run on. A world not up within `LOAD_PATIENCE` (600 s)
  is held.
- **The folder** is Steam's for the account playing,
  `<Steam>/userdata/<account>/3493540/local/save`, from the registry
  (Steam's `SteamPath` and `ActiveProcess\ActiveUser`), or else the one
  account with a save folder for the game. Without one, a load holds the
  world and a save is reported failed.

#### Loading from the main menu

The GUI runs only in a world, and at the game's main menu no world steps,
so neither the GUI nor the step's detour is there. A game at its menu
(`crates/tpf3mp-hook/src/menu.rs`; the static findings and the in-game
checks are in `investigation/TPF3_MENU_JOIN_2026-09-30.md`):

- **Gets a Lua state that can load.** The game gives a Lua state its `app`
  table in one function, `RegisterAppUsertypes` (profile target, build
  40408 `0xdc5fa0`), for the menu's states and the in-game GUI's. The
  hook detours it and, after the game's registration, runs a short chunk
  there (`lua_load`, `lua_pcallk`): it hands the hook a function that
  loads a save by name, kept in the state's registry (`luaL_ref`), and a
  sentinel whose `__gc` tells the hook when the state closes (Lua 5.2 runs
  every finalizer at `lua_close`), so the hook never calls into a closed
  state. hook.log: `menu: Lua state 0x... has app; the main menu can load
  the room's world from it`.
- **Knows whether a world is loaded.** `DoStep` runs at the menu and in
  a world alike, so the hook reads the engine's own answer:
  `CMenuUI::m_game`, the loaded world, which `CMenuUI::StartGame` asserts
  clear and sets and `CMenuUI::StopGame` clears. `DoStep` tests it before
  it hands its frame to the world's UI; that test is the optional profile
  target `UI::CMenuUI::DoStep/m_game test` (`0x6a01c0`, `cmp [rsi+0x6b0],
  r13`), and its displacement is the field's offset, which the menu's
  frame reads in the `CMenuUI` it is handed. A state given `app` while
  `m_game` is set is the world's GUI's: the menu never loads from it, and
  forgets it when the world closes. Loading is the menu's own sign, the
  progress monitor's task, asked through the chunk's `busy()`.
- **Follows the room from the menu's frame.** `UI::CMenuUI::DoStep`
  (`0x6a0160`, the menu's per-frame update on the main thread) is
  detoured. After the game's own frame, the driver runs
  `StepDriver::on_menu` only while the game is at its main menu with no
  world, and a state of the menu's adopted on that thread is open
  (`crates/tpf3mp-hook/src/at_menu.rs`):
  - a game that has had no world up yet (never stepped, no world's GUI
    started, `m_game` never set) is at its menu, loading or not;
  - a world loaded blocks, before its first step and while it stops
    stepping (saving the room's world, held for another player): an
    earlier rule of "no step for 2 s" took the room's session inside the
    owner's world and hung it, and the owner's menu frame once took it
    while saving the room's world before that world's first step (both
    measured 2026-09-30);
  - after a world, the menu is back once `m_game` is clear and the
    progress monitor has no task for 2 s with no step between. A load
    blocks: the GUI's load stops the world first and loads after, and a
    moment without a task restarts the 2 s;
  - after a world, a game whose `m_game` the hook cannot read (the target
    missing) or whose menu cannot say whether it loads is never taken for
    the menu (fail closed): as before this rule, only a fresh game follows
    the room from its menu.

  hook.log says where the menu sees the game on each change:
  `menu: a world is loaded (CMenuUI::m_game set)`, `menu: the world closed
  (CMenuUI::m_game cleared); ...`, `menu: no world loaded, but the game is
  loading one`, `menu: back at the main menu after a world (no world loaded
  or loading for 2 s)`; and when the room begins there, `the room began at
  the main menu; the menu sees: <where>`. Before the room begins, `on_menu` reads `Begin` and tells
  the agent `MenuUp` once per arrival, which marks a guest ready (and the
  room's owner, once the room has the save the owner named for it to start
  from: PROTOCOL.md, "The first world") and keeps the hook's heartbeat
  going at the menu; in the room's game it answers a
  `Load` with a file by copying the save into the folder as above and
  loading it from the menu, as the menu's own Load Game page does:
  `api.type.SavegameId.new()` with the name and the `savegame` namespace
  (`app.SaveGameNamespace.getSavegame()`), then `app.loadGame(id, false,
  nil)`. A load the game is busy with already (the progress monitor has a
  task, the menu's own sign of it) is tried again on the next frame. Any
  other failure holds the world.
- **Starts the world without Start Game.** The menu's pages call
  `app.setWaitForStartReadyGame()` before they load, which is what makes
  the loading screen wait for the player's Start Game
  (`app.startReadyGame()`); the hook does not, and the game then starts
  the loaded world by itself. Confirmed in a two-game playtest on build
  40408: a guest at the main menu loaded the room's world by itself, with
  no Start Game click.
- **Loads it with TPF3-MP active.** Without the room's mod lists `info`
  is nil, so the save keeps its own mod list, and the room's save was
  written by a game whose GUI had TPF3-MP linked (only the mod saves for
  the room). With them (the room's `Begin`, [MODS.md](MODS.md)), the chunk,
  handed the hook's `mods` as a third function, first reads the save's
  details (`app.getSavegameInfo`, answered "busy" until the game has them)
  and passes `info` as the menu's own Load Game page does when a player
  changes a save's mods: `api.type.SaveGameDetails.new(data.info)` with
  `mods` set to the save's shared mods, TPF3-MP and this player's
  personal ones, each `api.type.ModId` checked with the user profile's
  `ModRep:exists` (a shared mod not installed here fails the load and
  holds the world). A world whose mod does not start never says it is up,
  and is held after `LOAD_PATIENCE`.

The loaded world's GUI says it started (`tpf3mp_native.world`), the step's
detour takes the world at its first call (`Session::loaded`), and the
room's steps run on, as for a load from the GUI.

What the menu leaves to a world up: a `Load` without a file (the owner's
world, or everyone's on a server that keeps no worlds), and a `Save` the
room orders. Without a save named to start from, the owner's game
therefore still needs its world up to start the room: the menu logs `at the main menu: the room plays the world this
game starts from ...` once, and takes nothing. Without the menu's five
function targets (all optional in the profile) or an adopted state,
nothing of this runs: hook.log says `the main menu cannot load the room's
world (fail closed): ...`, and a game needs a world up, any one, before it
can load the room's. Without the `m_game` test, the install line says so
and only a game that has had no world up follows the room from its menu.

Tried on build 40408 through the deployed server, with two games on one PC
(the rig, the fixture save): the owner's game saved its world for the room
in 215 ms and reported it within a second. The guest fetched the save,
its GUI took the load in the world it was in, and the room's world was up
4 s later and played from step 1. A road depot sent from the owner's
console was applied by both games, and the determinism probe's 15 samples
from step 500 to 1900 matched in every lane it reads, the construction and
money lanes changing alike after the build
(`investigation/dayone-2026-09-29/6-determinism.md`; the edge lane is
still unread there).

### The player's commands

In the room's game a player's command runs in every game at the same
update, or in none. Transport Fever 3's GUI sends most of what a player
does from its own Lua state, through `api.cmd.sendCommand` (build 40408's
scripts): buying, selling, replacing and assigning vehicles and making and
editing lines (`gui/line_vehicle_mgmt/`, `gui/entity_window/`), a
construction's parameters and bridges and tunnels
(`makeWorldBuildProposalCmd`, from `gui/construction/construction.tl` and
the entity windows), loans and other game mechanics as script events
(`makeScriptingSendEventCmd`), and the speed row (`makeGameSetSpeedCmd`).
The street and track tools, and placing a construction, are native
builders that reach the command queue (`CommandList::Add`) without Lua.

So the stock windows do send through `api.cmd.sendCommand`, as the mod's
replays do (the question in PLAN.md, Part 2). What tells them apart is the
Lua state, not a caller's address: the player's commands come from the
GUI's state, the room's replays from the game script's states, whose
`api.cmd` the mod leaves alone, but for one thing: what a player's
personal mods send there (below, and [MODS.md](MODS.md)).

A GUI mod only one player runs (a personal mod, [MODS.md](MODS.md)) sends
through the same guard as the player's clicks: carried or refused alike.
A refusal names the mod it came from, the nearest function on the stack
whose source is a mod's file (`<modId>::/...`, `guard.callers`), so
hook.log says `refused the player's makeTownCreateCmd in the room's game
(1 so far), from the mod gw_big_city_1`. A personal mod's event to its own
game script (`makeScriptingSendEventCmd` with an id that names the mod and
neither an id nor a name the game's scripts or TPF3-MP listen to,
`guard.ownEvent`) is sent here as it is: it reaches this game's game scripts
only, where the mod's own game script runs. Any other of its events is
carried or refused as the player's own would be.

A personal mod's game script runs in its player's game only, in the
simulation's states, where a command runs at once. There
`tpf3mp/modguard.lua`, installed by the mod's game script in each state it
links in, sits in front of `sendCommand` in the room's game: a command with
a personal mod anywhere on its stack is not run; a vehicle's manual
departure (`VehicleChange::ManualDeparture`), departure or stop, or a
line's rename or update, for the player's own company's, goes to the room
as an action, once per change within 5 s of game time; an event between
game scripts is dropped; anything else is refused, each once in hook.log
(`handed makeVehicleSetManualDepartureCmd from the personal mod
celmi_timetables to the room (1 so far)`). Commands from the game's own
scripts, TPF3-MP's and shared mods run as before. A state without
`debug.getinfo` cannot tell them apart: it notes `personal-mods-unguarded`,
after which the hook loads the room's worlds without the player's personal
mods (docs/MODS.md).

The mod guards the build tools in its game script, whose `guiHandleEvent`
runs in the GUI's state. The street, track, station and depot, stop and
bulldozer tools tell game scripts of every proposal they make
(`builder.proposalCreate`), and they honour an error a script returns, as
the game's company script refuses constructions without a permit
(`game_mechanics/company/company.script.tl`). In the room's game the mod's
script returns "Not in multiplayer yet: building with this tool" for every
proposal of a tool the room does not carry yet, and the tool shows it in
red and builds nothing (seen on build 40408, with the street tool). A tool
the room carries builds through it instead ("The build tools" below). The
script subscribes to those events by name, as a save may carry an older
version's subscriptions.

The mod guards the GUI's commands
(`mod/tpf3mp_1/content/scripts/tpf3mp/guard.lua`). `api.cmd` is a plain
table whose factories and `sendCommand` (a callable table) can be replaced,
as the console's state showed, and none of the game's 6,041 scripts keeps a
reference of its own to either. Once linked, the GUI wraps every
`make*Cmd` factory, to note which one made each command, and
`sendCommand`. While the room's game runs (`tpf3mp_native.room()`):

- a command of a kind in `guard.PASS` is sent, its arguments untouched. So
  far that is the speed row's `makeGameSetSpeedCmd`, which the step gate
  reads as the player's request to the room;
- a command `guard.CARRY` makes an action of goes to the room instead
  (`tpf3mp_native.command`), which orders it for every game, this one
  included ("Actions in the game"). Its callback hears what became of it
  once this game has applied it (`results()`, `guard.deliver`), as the
  game's own command would answer: `(data, ok, {{ entity, 0 }})`, with the
  entity the action made. The game's windows chain on that: the store
  puts the vehicle it bought on a line by the entity its callback hears
  (`resultVehicleEntity`), the line manager opens the new line
  (`resultEntities[1][1]`); told nothing, or told before its world had the
  line, the line manager made a second line at the next stop clicked (seen
  on build 40408). So an answer waits until the GUI's world has the entity
  it names (`api.engine.entityExists`; the game script made it in the
  simulation), a few seconds at most, and answers keep their order; a
  command that should have made something and made nothing the game could
  name is answered as failed, which the windows handle. With both, a new
  line took its stops one by one as in single player (build 40408). So
  far:
  - loans, the finance window's `makeScriptingSendEventCmd("", "Loan",
    "Obtain" | "Repay", …)`, as a `Loan` action carrying the loans' terms,
    which every game's game script replays through the loan script's own
    event;
  - prospecting, the construction menu's `makeScriptingSendEventCmd("",
    "Companies", "spawnIndustry", …)`, as a `Prospect` action ("Prospecting"
    below);
  - taking a rank, the company window's `makeScriptingSendEventCmd("",
    "Companies", "applyLevel", { level })`, as an `ApplyRank` action
    ("Company ranks" below). The company's other events (greening an
    industry, `MakeGreen`; a marketing campaign) stay refused;
  - vehicles: buying (`makeVehicleBuyCmd`: the depot by its construction's
    file and position, the consist part by part, as the store configured
    it), selling, putting on a line, and the vehicle window's stop, start,
    to the depot (sold there or not), reverse and depart; replacing
    (`makeVehicleReplaceCmd`, the vehicle window's "modify" and the store's
    "replace", `ReplaceVehicle`). The store sends one command per vehicle,
    a group's vehicles one by one, and none with a callback
    (`vehicle_react_util.tl`, `HandleVehicleChanges`, build 40408); a
    vehicle whose new consist is empty it sells instead. A part the player
    left in the consist is the vehicle's own, its purchase time kept; the
    store bought the rest (purchase time 0, set to the GUI's game time
    before it sends). So the capture marks a part kept when it is one of
    the vehicle's own of the same model and purchase time, each matched
    once, and every game gives a kept part the purchase time and wear it
    has there, a new one the game time of the update it applies in. The
    vehicle keeps its canonical id: the game script binds the id to the
    entity the vehicle is after the command (the command's result
    entities, its data's `vehicleEntity`, else the vehicle itself; TF3's
    API says the vehicle is replaced and its command data has no result
    field, so the same entity is INFERRED), before the registry's sync
    would retire it;
  - lines: creating, changing (the line whole, as the line manager built
    it: stops, terminals, loading rules), deleting, renaming and
    recolouring;
  - a construction's edit sent from its window
    (`makeWorldBuildProposalCmd` with the game's replacement proposal), as
    a `BuildConstruction` that replaces it ("The build tools" below).

  Vehicles, lines, station groups, towns and industries have no place to
  name them by, so actions name them by canonical id
  (`tpf3mp/registry.lua`). Towns and the industries standing when the room
  began get theirs at the room's first update (or, in a room begun by an
  older mod, at its next update); an industry is named by its construction. Every game
  gives them the same ids without telling another: every game runs the
  same world, so after the same action the same things exist, and the
  mod's game script binds each new one to the next id of its kind, lowest
  entity first, after every action the room orders and at the room's
  first update, and retires the ids of those gone; an id never comes back.
  What an action made, as the game answered its command, is bound at
  once, whatever the game's lists say, and an id is retired only when its
  entity no longer exists, never because a list left it out.
  It keeps the registry in its state, which the game saves with the world,
  so a player who joins or reloads from the room's save has it as the
  others do. The GUI reads it from the script's state
  (`gameScriptSystem.getEntityForGameScript`, the `GAME_SCRIPT`
  component), as the loan window reads the loan script's. A command
  naming something the registry has no id for is refused, and says so;
- any other kind, and a command no wrapped factory made, is refused, as
  PLAN.md (Part 3) says of every action whose strict flag is off. It is
  not sent. Its callback, if it has one, is called on the next frame with
  `(command, false, {})`, as the game answers a command that failed. The
  game bar shows "Not in multiplayer yet: …" for a few seconds, and
  `hook.log` gets a line for the first refusal of each kind and every
  hundredth after.

Before the room begins, and after it ends, every command is sent as it
would be, and every tool builds. A kind the room comes to carry is
captured into an action instead of refused, and applied by every game
("Actions in the game").

### Companies

A room holds up to eight companies (DECISIONS.md, D21). It starts as one,
the save's own player, which every player plays for; a player founds a
company of their own, joins another, renames or recolours theirs, or
dissolves it as its last player once it owns nothing, through the room
(`CompanyOp`), and every game applies it at the same update, as any
action. `tpf3mp/companies.lua` keeps the roster in the mod's game script's
state, which the game saves with the world:

- *A company is a TF3 player entity*, `makeGameAddPlayerCmd(name,
  colour)`: every game makes it at the same update of the same world, so it
  is the same entity in every game (seen on build 40408: entity 7595 in all
  three games of a room). Its money is that entity's `ACCOUNT`, and what it
  builds and buys is owned by it (`PLAYER_OWNED`), as the game keeps
  ownership.
- *Who acted.* The hook hands each ordered action to the game script with
  the player who sent it (the Lua link's version 10, `tpf3mp_native.version`:
  `take()` answers the actions
  and, second, each one's sender as 64 hex digits, and since version 12
  third, each one's seal; `status()` names each
  player's `id` and the local one's `me_id`). The game script books the
  action to that player's company: `apply.lua` puts the company's player
  entity where it put the save's player before (a build's `Context.player`
  and its constructions' and stops' `playerEntity`, `makeVehicleBuyCmd`'s
  and `makeLineCreateCmd`'s player, prospecting's `companyEntity`).
- *What another company owns* is refused, the same in every game, naming
  its owner: an edited, bulldozed or removed construction, road or track
  edge, or stop, and the vehicles and lines an action names, when their
  `PLAYER_OWNED` player is another company's. What no company owns (the
  towns' roads) stays everyone's.
- *Loans.* The game's loan script (`::/game_mechanics/finance/loan.gs`)
  keeps the save's own player's loans only. Another company borrows on the
  terms the loan script offers (its `availableLoans`), and the room keeps
  that loan: booked to the company as the game books one (a `LOAN` journal
  entry, `makeJournalBookAssetCmd`, which raises the account's balance and
  loan alike, seen on build 40408), and paid back each month of the game's
  calendar as an annuity, the interest as `INTEREST` and the rest as
  `LOAN`, or all at once. The game script books the months since the last
  on the first update of a new month, in every game alike.
- *Colours.* With more than one company, a vehicle bought is painted in its
  company's colour (`makeEntitySetColorCmd`), and a new colour repaints the
  company's vehicles, in the engine's own order. With one company the
  game's colours stay, as in single player.
- *Markers.* Painting colours a vehicle's body and its pictures in the
  game's windows, not its marker on the map: build 40408 draws every
  vehicle's marker alike, a white glyph on a dark tile
  (`HudIconManager.cpp`, `gui/main/internal_hud.css.lua`), where TPF2's
  followed the vehicle's paint. `gui/tpf3mp/company_markers.res.lua`, a
  `react-replacement-config`, replaces the game's marker recipe
  (`hud_icon_toolbox.HudIconMasterGame`) with one that calls it and, while
  the room has more than one company, puts the marker of a vehicle painted
  in a company colour in that colour's class; `gui/tpf3mp/tpf3mp.css.lua`
  colours the tile of each class. Seen on build 40408: the HUD renders
  markers on its worker threads ("Main Pool" in the log), each with a Lua
  state of its own where the Multiplayer plugin does not run and a
  vehicle's `PLAYER_OWNED` reads empty; so each reads the roster from the
  game script's state (every 2 seconds) and the vehicle's first part's
  colour, and the HUD takes only a layout from the recipe ("Recipe child
  must be a layout"). The game's log says what became of the markers
  (`[tpf3mp] company markers: ...`).
- *The GUI's company.* TF3's windows ask `api.engine.util.getPlayer()`
  whose money to show and what is the player's own ("Foreign" otherwise).
  In the GUI state the mod replaces it (a callable table on build 40408,
  which takes the assignment) with one that answers the player entity of
  the company this player plays for, or the game's own answer for the
  room's first; `hook.log` says `the GUI's company follows the player's`.
  The game scripts' states keep the game's own answer, so the simulation
  is the same in every game. Seen on build 40408: the game bar's account
  showed the new company's money, and another company's depot opened
  without its vehicle management. The GUI runs in more than one Lua state:
  the line manager's HUD, drawn in another, showed the room's first
  company's depots to a Rival player and not Rival's own. So
  `gui/tpf3mp/gui_state.res.lua`, a `react-replacement-config` that
  replaces no recipe, has the game run `gui_state.script.lua` in the state
  it renders its recipes in, before any renders; there
  `tpf3mp/follow.lua` gives getPlayer the same answer, read from the hook
  (`status().me_id`) and the game script's roster every 2 seconds
  (`hook.log`: `the GUI's company follows the player's in the HUD's
  state`).
- *The Multiplayer window* lists the companies with their money and
  players, the one the player plays for first with its colour to choose
  (the game's colour chooser, `ColorChooserButton`, with the companies'
  colours first and then the game's line colours and greys, as its vehicle
  window offers them), a name to change, loans to take and pay back, and
  each other company to join, with a password field (`passwordMode`, as
  the game's own login form) for one that has a password. Each company
  shows its head, and whether it has a password or has closed its
  stations. The head of the player's company also sees its password to
  set, change or remove, its stations to open or close, and a button to
  send each other player out.
- *The game's company window* renames the company by its title
  (`game_mechanics/company/company.tl` sends `makeEntitySetNameCmd` on the
  player entity `getPlayer()` answers, the player's company): the guard
  captures a name or colour set on a company's player entity as that
  company's `Rename` or `Recolor` (`capture.setName`, `setColor`, which
  know companies by the GUI's roster), and every game checks it is the
  player's own. The window has no extension point for more; the rest
  stays in the Multiplayer window.
- *Who may do what* (DECISIONS.md, D22, proposed), checked by every game
  alike when the room orders it (`companies.run`): a company's players
  build, buy, run lines, borrow, rename and recolour it (a colour of
  fractions from 0 to 1 that no other company wears); its head, the
  founder while they play for it and else the player who has played for
  it longest (the roster's members are kept in the order they joined),
  alone gives it a password (`CompanyOp::Lock`), takes it away
  (`Unlock`), sends a player out (`Dismiss`: they play for the room's
  first company again) and opens or closes its stations
  (`ShareStations`). The room's first company is everyone's: no head, no
  password, and its stations stay open. `hook.log` names why a refused
  action was refused.
- *Passwords.* The window hands the password to `command` beside the
  action (`Join` or `Lock`), the hook sends it to the room beside the
  intent (`tpf3mp_proto::Secret`, scoped to the company), and the server
  orders the action with the password's seal, never the password
  (PROTOCOL.md, "Secrets"). `take()` hands each action's seal third; a
  `Lock` keeps it as the company's `lock`, and a `Join` of a company with a
  lock needs a seal for that company equal to it ("joining X needs its
  password", "the password for X is not right"). The roster keeps the seal
  only, which is safe in a save: it cannot be checked against a guess
  without the server's key. The window reads only whether a company has
  one.
- *Using another company's stations.* The game stops a line at any
  station: build 40408's line and stop commands check that a stop names a
  station and terminal the station group has, not who owns it (no owner
  refusal among the exe's line errors; a stop is a station group, a
  station index and a terminal, `api/tealdef/api/engine.d.tl`). Its line
  manager offers only the player's own stations and those no one owns
  (`gui/line_vehicle_mgmt/manager_window.tl`, at each click that adds or
  moves a stop, asks `scripts/entity_util.tl`'s
  `isOwnedByPlayerOrNotOwned`). In the GUI state the mod wraps that test so
  it also takes a station group or station construction of a company that
  keeps its stations open; `hook.log` says `the line manager offers other
  companies' open stations`. That the line manager and the mod share one
  `entity_util` table (one `ug_require` cache) is INFERRED. Every game
  checks each stop of a new or changed line (`apply.lua`, `lineComponent`,
  `companies.mayUse`) and refuses one at a closed company's station, naming
  it. TPF2 had to patch a native station filter for this (TPF2MP's shared
  stations); on TF3 the filter is Lua, and no native patch looks needed
  (INFERRED: not seen in a game). A company's vehicles still use its own
  depots, as TPF2MP left `FindPathToDepot`'s owner check alone.

### Prospecting

Prospecting for an industry near a town
([investigation/TPF3_PROSPECTING_2026-09-30.md](../investigation/TPF3_PROSPECTING_2026-09-30.md))
is one command from the player's GUI, and everything after it happens in
the simulation:

1. The construction menu's prospection, on the town the player picks,
   sends the company script `Companies` `spawnIndustry`. In the room's
   game the guard captures it (`capture.prospect`) as a `Prospect` action:
   the town by its canonical id, the cargo, the industry types in the
   menu's order, the permit. It is refused, and says why in `hook.log`,
   for a town the registry cannot name, another company, or no industry
   type. The menu keeps its permit reserved until this game has applied
   the action, as it does until the game answers its own command.
2. Every game applies it at the same update, through the company script's
   own event (`apply.lua`). The company script checks that the company
   does not prospect there already, keeps the prospection with the game
   time as its start, uses the permit, and tells the prospecting plane to
   fly.
3. Months later (six at most, trying each month with a growing chance),
   every game's company script draws the outcome, from the game time
   (`math.randomseed(gameTime * 1000 + index)`), shuffles the industry
   types by those draws in the order the action carried them, and asks the
   game for a place (`makeIndustrySpawnProposal`, seeded from the game
   time too). It builds the industry, with the construction's seed the
   game chose, itself: not player-initiated, so the hook's build stop lets
   it through, and in the engine state, where the guard is not. A
   prospection that finds nothing gives the permit back.
4. The mod's game script hears the company script's `startProspection`
   and `endProspection` and says each in `hook.log`; an industry found is
   bound in the registry at once, so every game names it by the same id.

What the room carries is the request, not the outcome: the outcome is a
function of the world and the game time, which every game has alike
(INFERRED from the scripts and the binary; the investigation says what is
seen and what is not). The permit check is the menu's alone, as in single
player: two players prospecting in the same moment could each pass their
own menu's check and use one permit more than the company has, the same in
every game.

In `hook.log`, prospecting for coal near a town shows first, in the game
of the player who picked the town:

```
handed the player's action 42 to the room
```

then in every game of the room, at the same step:

```
prospecting for ::/cargos/coal/coal.cargo near town-3 (1234): coal_mine
prospecting began: ::/cargos/coal/coal.cargo near town-3 at game time 5400000
the game applied 1 action(s) the room ordered
```

and, one to six game months later, again in every game at the same step:

```
prospecting ended: ::/cargos/coal/coal.cargo near town-3, begun at game time 5400000, found industry-17 <the coal mine's construction file> at (1234.5, -250.3)
```

or `..., found nothing`. The entity in brackets on the first line is each
game's own; the rest, the industry's id, file and place included, is the
same in every game.

The game's company script runs the prospections of the save's own player
alone (`company.script.tl`, its update looks at `getPlayer()` only, in the
engine state): a prospection of another company is kept and its permit
used, but its outcome is never drawn (seen in the scripts; not carried yet,
docs/PLAN.md).

### Company ranks

A company's rank gives it the game's permits: headquarters, marketing,
prospecting and the rest
([investigation/TPF3_PROGRESSION_2026-09-30.md](../investigation/TPF3_PROGRESSION_2026-09-30.md)).
The game's growth script keeps one company, the save's player: its
experience is the highest world population it has seen, the rank that
reaches is its potential, and the company window takes a rank reached with
`Companies` `applyLevel`.

- **One company in the room**: the game's own score. The company window's
  `applyLevel` goes to the room as an `ApplyRank` action, and every game
  applies it at the same update through the growth script's own event. A
  rank not reached, or taken already, is refused with why in `hook.log`.
- **More than one** (D23, proposed): the mod's game script scores every
  company four times a game month, at the same game time in every game
  (`tpf3mp/progression.lua`): for each town, the town's population times
  the company's share of what was carried for it (cargo delivered to it in
  the last half year, passengers travelling to and from it on lines, from
  the game's statistics per line and the lines' owners) times its rating
  there over 100 (the town's rating, with the happiness of its own
  passengers and the punctuality of its own cargo, by the game's formulas).
  Experience is the highest score, the rank it reaches the game's own
  thresholds. The records are the mod's game script's state, saved with
  the world. `ApplyRank` takes a rank reached in that record; the room's
  first company also takes it in the game's own state. In the GUI the
  mod answers the game's windows' `getCompanyProgressionState` from the
  record, so the company window and the permits show the player's
  company's own rank (`the company window shows each company's own rank`
  in `hook.log`; INFERRED that the game's windows share the module the mod
  changes, to check in the game).

Nothing is guessed: a sample whose statistics or game modules do not read
is left out (`the companies' scores were not sampled: ...`), and a town
with no rating counts for nobody.

In `hook.log`, every game of a room of two companies writes at each
sample, at the same game time and with the same numbers:

```
progression at game time 5400000: 12 towns, weights cargo 1 passengers 1
progression at game time 5400000: town-3 population 1240: company-0 share 0.5500 rating 80.0000 part 545.6000, company-1 share 0.4500 rating 64.2859 part 358.7153
progression at game time 5400000: company-0 score 3120.4000, experience 20514, rank 3 reached, 2 taken
progression at game time 5400000: company-1 score 812.9000, experience 812, rank 0 reached, 1 taken
```

one line per town someone carried for, then one per company. Compare the
lines of the same game time across the games: any difference is a
divergence.

### The build tools

The street, track and construction tools are native: a click queues a
`WorldBuildProposal` command, which the simulation applies at its next
step, in that game alone. Nothing a script does stops one on build 40408:
the street tool sends no `builder.proposalPrepareForApply` (a click is
`proposalCreate` twice, then the simulation's `onPreBuildProposal` and
`onPostBuildProposal`, then the GUI's `proposalApply`), an error raised in
`onPreBuildProposal` is logged and the build goes on, and emptying the
proposal's lists there crashes the game (found with
`tools/probe/tf3/tpf3mp_buildprobe_1`). So the hook stops the player's
builds natively (`crates/tpf3mp-hook/src/builds.rs`, profile targets
`CommandList::Add` and `WorldBuildProposal apply`):

- **At the click**, `CommandList::Add`, on the main thread: a command whose
  payload is a `WorldBuildProposal` (its variant index, at payload +
  0x9b8, is 52) with `playerInitiated` set (payload + 0x3d2) is counted, in
  the room's game only, and added as the game would.
- **At the apply**, the simulation's apply of a `WorldBuildProposal` (the
  command dispatcher's case 53, `0x9e1160`): a player-initiated build in
  the room's game answers false, as a build the game refused, and the game
  tells the tool so through its own path. The room's own builds go
  through: the mod's game script applies them with
  `tpf3mp_native.replaying(true)`. Towns' growth and the game's scripts,
  not player-initiated, go through as ever.

The mod's game script, in the GUI (`guiHandleEvent`), keeps the action each
proposal of a tool the room carries makes
(`mod/tpf3mp_1/content/scripts/tpf3mp/capture.lua`), marked with the clicks
counted when it saw it (`tpf3mp_native.clicks()`). Its `guiUpdate` hands
the room the one each click saw last: the proposal and the click both run
on the main thread, in order, so that is the proposal clicked. The room
orders it for every game, this one included, and each game's `postUpdate`
builds it, paid by the player (`Context.player`) and clearing town
buildings in its way (`gatherBuildings`), as the tool builds; without a
context the game builds for free.

Five tools build through the room so far, the module editor through the
hook, and a construction's window its edits:

- **The construction tool** (`constructionBuilder`): a proposal of one
  construction (a station, a depot, anything the tool places) becomes a
  `BuildConstruction`, with the file, the transform, the parameters
  flattened and the game's name for it. The streets in its proposal are
  what the tool built around the construction, and travel with it as its
  connection: a bus station placed by a road rebuilds the road through a
  new junction and adds an entrance edge from the junction to the
  station's own street node (seen on build 40408, the construction's
  frozen nodes and edges empty in the proposal). Built without them, the
  station stood beside the road, its entrance a dead end, and the line
  manager could not connect it ("Could Not Connect Stations"). The
  entrance edge in the tool's proposal is the construction's own, snapped
  onto the road: every station and depot has one frozen node and one
  frozen edge, its entrance, and a scripted build makes that edge again
  unsnapped, ending about 2 m short of the road (build 40408, read from
  the console). The game's refresh of the construction
  (`api.engine.util.proposal.refreshConstruction`) snaps it as the tool
  does: its proposal's entrance edge ends at the road's node. So the
  replay builds, in one action, the construction with the rest of the
  connection (the road rebuilt through the junction), then the refresh of
  the new construction, free (no context) and not as a click of the
  player's; the refresh finds the junction the tool chose. The town
  buildings in its way the replay clears again (`gatherBuildings`, and
  `gatherFields` for fields). Several
  constructions at once, or one replacing more than one construction that
  is not a town building, is refused. Before sending, the replay asks the
  game's verdict (`makeProposalData`) and refuses what it calls critical,
  with its reasons.

  A proposal that replaces one construction of the player's with a new
  one (an edit of its modules or parameters, an upgrade) becomes a
  `BuildConstruction` with `replaces`: the old construction by its file and
  where it stands (a `ConstructionRef`, as a depot is named; entity ids are
  no name, docs/BUILDING.md), the new one's file, transform, parameters and
  name (the old one's, where the proposal leaves it out). Its street part
  is the construction's own entrance, made again with it, and is not
  carried; an edit that removes a street or track the old construction
  does not own (its `frozenEdges`, `frozenNodes`) is refused, as is one
  replacing a construction the room cannot name. Every game finds the old
  construction by file and place (within 2 m), then asks the game's
  verdict and builds, as the player's own build (`ignoreErrors`,
  `playerInitiated`), paid by the player and clearing town buildings in
  its way, one `SimpleProposal` that removes it (`constructionsToRemove`,
  this game's own entity) and adds the new one, mapped old to new
  (`old2new = { [old] = 0 }`), as the game's own upgrade makes one
  (`mission_framework_util_entity.tl`, `upgradeConstruction`). The new
  construction stands where the old one stood, so the next edit, a depot
  bought at it or a line finds it by the same reference; what stood on it
  passes to it through `old2new`, and the registry binds, after the
  action as after every other, any station group the game made anew, the
  same in every game. An old construction not there fails the action in
  every game, and nothing is sent. INFERRED, not yet seen in the game:
  that an edit's proposal names the old construction in `toRemove` and
  the new one in `toAdd` (TPF2's shape), that its street part removes
  only the construction's own entrance, and that `old2new` keeps its
  stations' station groups.

  Where edits come from on build 40408 (read from the binary and the
  game's Lua, not yet seen in the game):
  - the game tells game scripts of the proposals of six tools only, under
    the ids `UI::CGameUI`'s constructor names them by:
    `constructionBuilder`, `streetTerminalBuilder`, `streetBuilder`,
    `trackBuilder`, `streetTrackModifier` (the road and track modifiers,
    "The road and track modifiers" below) and `bulldozer`. The **module editor**
    (`UI::ModuleBuilder`, opened from a station's window) is not among
    them: it queues its `WorldBuildProposal` itself and game scripts hear
    nothing of it (its click was stopped with "no proposal seen" in the
    game, 2026-09-30). So the hook reads it natively ("The module editor"
    below), and the GUI takes that for the click. The mod's `CAPTURE` also
    names `moduleBuilder` and `moduleBulldozer` (the construction menu's
    names for those tools, `ConstructionActionParam`) in case a later
    build sends their proposals; INFERRED;
  - a construction's parameters changed in the construction menu, and the
    cargo buttons of a station's window, send the game's replacement
    proposal from Lua (`api.engine.util.proposal
    .createProposalReplaceConstruction`, then `makeWorldBuildProposalCmd(proposal,
    nil, false, true)`, `gui/construction/construction.tl`,
    `gui/entity_window/entity_window_util.tl`). The guard carries such a
    command as the same edit (`guard.CARRY.makeWorldBuildProposalCmd`,
    `capture.windowBuild`); any other build a window sends stays refused
    ("building from this window");
  - a bulldozer proposal that removes a construction of the player's and
    adds one (a module removed, if the module bulldozer reaches game
    scripts as the bulldozer) is carried as the same edit; INFERRED.

  In the room's game `guiHandleEvent` also logs each event it does not
  handle by id and name, once each and 40 at most (`an event the mod does
  not handle: id …, name …`), so a test in the game shows what a tool
  that builds "with no proposal seen" sends, if anything.
- **The module editor** (`UI::ModuleBuilder`), read natively
  (`crates/tpf3mp-hook/src/modules.rs`). It queues its build with its own
  call of `CommandList::Add` (profile target `ModuleBuilder::MousePressed/Add
  call`, 0x543b25 in `UI::ModuleBuilder::MousePressed`; `Add` returns to
  0x543b2a; the factory before it sets `playerInitiated`). The add's
  detour is entered through a thunk that notes where `Add` returns to; a
  player's build counted there whose call returns into the module editor
  has its `Proposal` read before `Add` runs: `toRemove`, the first
  `toAdd` entity's file (`ResName`, printed `mod::/path`), parameters (an
  abseil b-tree of variants, walked whole), matrix and name, and the
  street part's removed nodes and segments by entity (the layouts of
  `feat/capture-all`'s `conscap.rs` and
  `investigation/TPF3_CONSTRUCTION_CAPTURE_2026-09-29.md` there, re-checked
  with `tools/tpfre` on build 40408). Every read is checked readable,
  every count and text capped, every table walked to exactly its size. It
  is kept, as a Lua table of the shape game scripts see a proposal in,
  under the click count before it (the 16 newest kept), and logged
  (`module editor: click N queued …`, or `module editor: click N does not
  read: …`). The GUI's `guiUpdate`, handing on click N, asks
  `tpf3mp_native.built(N)` first, ahead of any preview another tool
  showed: the proposal is made an action by `capture.construction`, as
  the construction tool's, and must replace a construction (else
  refused: `an edit that replaces no construction`); one that did not
  read is refused with why. The click's apply is stopped as every
  player's build is, and the room orders the edit for every game, which
  replaces the construction as above. `built` is optional in the bridge:
  a mod or hook without it keeps the module editor refused. Without the
  profile target the hook logs so at install and the module editor stays
  refused. INFERRED, not yet seen in the game: every layout read (static
  only), that the proposal's parameters are the ones the replay needs
  (`seed` among them), and that the module bulldozer (a
  `UI::Bulldozer` action) is not this call.
- **The street and track tools** (`streetBuilder`, `trackBuilder`): the
  proposal becomes a `BuildRoad` or `BuildTrack`, as the tool made it, by
  positions (docs/BUILDING.md, "The action schema"): the nodes and edges it
  adds, each edge in its own kind (the street it joins is rebuilt through
  the new junction in that street's template), and the edges and nodes it
  removes. The replay builds it as the game's own scripted track builder
  does, `nodesToRemove` included. A build that moves or removes an edge
  with a stop or signal on it, or that places stops, signals or
  constructions, is refused.
- **The bulldozer** (`bulldozer`): its proposal removes one construction
  (with the construction's own entrance edge and node) or edges of one
  network (with the nodes they leave on their own). It becomes a
  `Bulldoze`: the construction by its file and position, or the edges by
  their ends. The replay removes them as the game makes such a removal
  itself, `createProposalRemove` for the construction and
  `makeSegmentsRemoveProposal` for the edges (on build 40408 the first
  gave exactly the bulldozer's proposal), and the player pays. A stop it
  removes is carried as the stop tool's builds are (below): its edge
  rebuilt without it, the stop named by its edge, where it stands and its
  construction (the `EDGE_OBJECT` component's `transf` and
  `edgeObjectConstruction`), a `Bulldoze::EdgeObject`; the replay puts it
  in `edgeObjectsToRemove`. Removing a signal, or an edge with a stop or
  signal on it, is refused.
- **The stop tool** (`streetTerminalBuilder`): a stop on a street. The
  tool queues a `WorldBuildProposal` (command 52 from
  `UI::StreetTerminalBuilder`, found statically), which the hook's gate
  stops like the others. Its proposal has the shape the game's own mission
  scripts check a stop by (`checkStop`,
  `mission_task_build_construction_util.tl`): one edge removed and the same
  edge added again between the same nodes, whose `objects` list its stops
  as `{ entity, EdgeObjectType }`, as many as `edgeObjectsToAdd`. The new
  stop is the one entity the old edge did not list. It becomes a
  `PlaceStop`: the edge by its ends, the point of its centreline where the
  stop stands, the engine's `left`, the edge's direction there, and the
  stop's construction. Seen on build 40408 (a room, 2026-09-30): the
  proposal game scripts get has no model and no place on its edge objects
  (`+o{resultEntity=-1 category=0 left=false playerEntity=3869}`), and a
  stop is a construction (`stations/street/small_stops/small_new.con`,
  whose update script places `small_new.mdl` on the edge). So the GUI
  notes the construction the construction menu gives the tool (its
  `EdgeObjectBuilder.resName`, from `construction_react_util
  .getActionParams`, which `capture.watchStopTool` wraps in each GUI Lua
  state) through the hook (`tpf3mp_native.note`), and the capture reads it;
  a proposal whose edge object has a model
  (`api.res.modelRep.getName(modelInstance.modelId)`) names it itself. The
  place is the proposal's parameter or model position where it has one,
  else the point of the centreline nearest the ground under the cursor
  (`api.gui.mouse.getTerrainPosition`), which every game then uses. A
  two-sided stop is one click that adds an object on each side: a
  `PlaceStop` with `two_sided`, which every game builds on both sides in
  one proposal (objects `-1` and `-2`). Every game's
  `postUpdate` rebuilds the edge as the game's electrify task rebuilds one
  (`electrify.tl`: the edge's own component read afresh, entity -1), its
  other stops kept under their own entities, the new stop
  `edgeObjectsToAdd[1]` (edge -1, the parameter where it stands, `left`,
  the model, the player), named in the edge's objects as `{ -1, side }`,
  the lane configurations at the edge's ends removed as for any edge a
  replay removes; then the game's verdict, and the build as the player's
  own (`ignoreErrors`, `playerInitiated`), paid by the player. A receiver
  whose edge runs the other way flips `left`; a side already taken is
  refused (two stops on one side is a fatal assert in the game's lane
  creation on TPF2). Refused: a stop dropped where one stood (the game
  moves its lines to the new one, which a replay cannot say), more than
  two new objects, or two on one side, signals and waypoints, a stop whose
  engine side (`STOP_LEFT`, `STOP_RIGHT`) is not what its `left` says, and
  a stop whose construction no GUI state noted.
  INFERRED, not yet seen in the game: that the tool's proposal lists
  `objects` in the order of `edgeObjectsToAdd`, that a kept stop keeps its
  entity there, that `STOP_LEFT` goes with `left`, that a script proposal
  names a new object `-1` in the edge's objects (TPF2's convention), that
  the model name `modelRep` gives is the one `EdgeObject.model` takes, and
  that the tool's click is `playerInitiated`.

A refusal shows its reason in the tool, and the log has each new reason
with the proposal's shape (`the room cannot carry this ... build`); every
build handed to the room is logged with its shape too. The upgrade, bus
lane and tram track tools and the signal tools stay refused until their
builds are captured. Where the profile lacks the
two targets, `clicks()` is nil and every tool stays refused.

Seen on build 40408, through the deployed server with two games on one PC:
a maintenance building placed with the construction tool in the guest's
game was stopped there, handed to the room, and built in both games in the
same update; both accounts paid its $180,336, and the room found no
divergence. The same for a street across open ground ($71,820), a street
onto an existing junction ($94,231), a street onto another's middle, a
track across open ground ($22,054), a track across a street, and a bus
depot snapped onto a town street, clearing three town buildings
($825,816): identical in both games, towns included.


### The road and track modifiers

The tools of the road menu's tools tab and the track menu's (tram tracks,
bus lanes, noise barriers, alleys, the towns' lock, electrification, a
track type) tell game scripts their builds as `streetTrackModifier`. Seen
on build 40408 (a room, 2026-09-30; `hook.log` names the tool, e.g.
`ACTION_TRAM_TRACK_TOOL ::/gui/construction/tools/tram_track_tool.res`,
and what it changes): each rebuilds the stretch of road it is used on,
edge by edge, between the same places (a node between two edges may be
removed and added again at the same place), with its new template (a
tram track or bus lane is the road's template on TF3), decorations
(`edgeDecorations`: `barrier_b.edge` as `{ 3, false }`, `alley.edge` as `{
0, false }`), `roadDevelopmentLocked` and owner (the player-owned tool:
`false>true`, `nil>` the player), new node configurations at its ends, and
the town buildings along it cleared and put back.

The mod carries such a build as the `BuildRoad` or `BuildTrack` of the
network of its first edge (`capture.modify`, `engine.captureModify`): each
link names its template (`kind`), its decorations by name
(`edgeDecorationRep.getName`; every game finds its own id with `find`), and
whether it is locked and owned by the acting company. The town buildings it
clears every game's build clears again (the build is the player's own,
`ignoreErrors`). Junction changes in the tool's proposal now travel with
the polyline; otherwise the replay preserves the endpoint configurations
and remaps their edge references (BUILDING.md, "Junction edits").
Stops and signals on the stretch stay: a new edge between the same
places in the same direction as a removed one, listing exactly its objects,
is the edge rebuilt in place, and every game's build gives it the objects
of the edge it replaces under their own entities (`engine.keptInPlace`,
`networkInto`); a stop moved onto another edge is refused. The same rule
lets the road and track tools build through an edge with a stop on it.
### Junction tools

Build 40408's native lane/crosswalk tools do not emit a Lua proposal preview.
At the existing `CommandList::Add` hook, `junctions.rs` reads junction-only
WorldBuildProposals and stores them against the click number through the
same `tpf3mp_native.built(n)` queue as the module editor. That snapshot
overrides an older road/construction preview. The Lua command guard also
captures a window's `createTrafficLightProposal` via `capture.windowBuild`.
The original player command remains cancelled; each replica uses the
ordinary ordered replay path. Mixed native geometry proposals keep their
existing capture path, never a partial junction replacement.
The reader classifies mixed geometry/construction proposals before imposing
the standalone junction limit. A live eight-track station attempt generated
392 node configs (another attempt generated 200); checking the 64-junction
limit first incorrectly rejected the station before construction capture.
The regression test covers both sizes and every mixed-proposal vector,
while oversized standalone junction edits remain refused.
The corrected reader was tested in two real games on the local server on
2026-10-01: an eight-track, 320 m station placed and applied on both replicas.
The same run exercised two module edits after correcting the ownership
check for unfrozen station track ends (BUILDING.md, "Module edits and upgrades").

Read-only binary evidence (Steam Windows 40408): StreetProposal config
vectors at +0x60/+0x78; BaseNodeLaneConnectionAndEntity has the component
at +0 and node at +0x78, stride 0x80. BaseNodeConfig connections/crosswalks
are at +0/+0x18, double-slip at +0x48, preference at +0x4c, light states at
+0x50, light type at +0x68 and custom phases at +0x70. Connections have
stride 0x14; phases have stride 0x28. Binding-registration signatures
at RVAs 0x1768337 and 0x22c395f gate this reader in the profile; the
static proof resolves both against the installed executable. Crosswalks
are a `phmap::flat_hash_set<int>` occupying +0x18 through +0x47: control
bytes and slot pointers, size, capacity and internal bookkeeping. The
constructor at RVA 0xa4990d is a third profile anchor; the move/copy at
0x1eda20/0x1fe1e0 and iteration at 0xa49f1a establish this layout. The
reader checks the sentinel, occupied-slot count, bounds and unique edge
IDs, skipping empty/deleted slots. The first real crosswalk click on
2026-10-01 exposed and now regression-covers the earlier incorrect vector
assumption. Every read, vector count and boolean is checked.
The API declaration's `userModifiedLaneConnections` is absent from this
build's binding registration; the adapter does not invent an offset.

Two-game test on 2026-10-01 (Steam Windows 40408, local server, disposable
copy of `tpf3mp_fixture2.sav`): the host removed a crosswalk, the guest
restored it, then the guest changed a lane connection and enabled traffic
lights at the same crossing. Both replicas logged each ordered replay;
network checkpoint hashes agreed at steps 500, 750, 1000 and 1350. The
crosswalk and traffic lights also changed visibly in the other game.
Evidence is in the local `runtime/junction-live-20261001-01/` logs. This
does not yet cover custom phases/reset, adjacent geometry changes,
proposal field comparison or rejoining after an edit. The initial load
also hit a native crash/Lua UI error; a later host-then-guest restart
worked. That startup failure has not been diagnosed. A rejoined game's
speed row showed 1x while the room was paused; pause then play resumed it.

Full acceptance is still pending. Keep `junctions.strict_junctions` off in
the normal mod. In matching disposable test mod copies, turn it on and:

1. Start two games through the launcher into one room and the same save.
   Use the actual tools to toggle crosswalks, lane turns and traffic lights;
   edit phase timings and reset settings. Repeat from the other player.
2. Confirm the native capture log names the clicked junction edit, and
   the emitted action has positions/resource names, with no engine IDs.
   Compare the engine's proposal against the replay's proposal field by
   field before treating matching synthetic tests as gameplay evidence.
3. Check both games' visible crossings, arrows and light settings, then
   upgrade/split an adjacent road and check that the settings survive.
   A change that removes a referenced lane must refuse without changing
   either game, rather than silently replace the configuration.
4. Compare network checkpoint dumps through subsequent updates and after
   save/load or rejoin. One-sided edits to a crosswalk, turn or phase in a
   disposable diagnostic test must cause a network-lane mismatch.

`lua_mod.rs` runs portable capture → wire → replay against two different
ID spaces and checks the native-click precedence, refusal cases and
curved-road preservation. `hook/tests/junctions.rs` checks native layouts
and malformed memory. These tests do not launch the game and do not
complete the playtest above. AGENTS.md currently prohibits automated game
launches/modifications; a human must run this check or explicitly override
that restriction before an agent runs it.

### The world's lanes

A room finds a game that drifted from the others by comparing the world's
lanes at every checkpoint step (`checkpoint_interval`, 50 steps by
default): digests of parts of the world, which the session reports with
the step (`Session::after_step`, `Game::lanes`). The lanes are read by the
mod's game script, which sees the world between updates:

- The session ends every batch at a checkpoint step, so a checkpoint is
  always a batch's last update. The driver knows the step a batch starts
  at (`RoomGate::next_step`) and so whether it ends at one, and tells the
  hook's Lua side (`lua::begin_batch`), which counts the batch's updates by
  their `take()`.
- In that last update, `tpf3mp_native.checkpoint()` answers true; the game
  script's `update` returns that, and its `postUpdate` reads the lanes
  (`mod/tpf3mp_1/content/scripts/tpf3mp/lanes.lua`) and hands them over
  (`tpf3mp_native.lanes`).
- After the batch the driver takes them (`lua::end_batch`), makes a
  SHA-256 digest of each lane's text (`step::lane_digests`), and the
  session reports them for the checkpoint step. A batch that ended at a
  checkpoint without them holds the world: the room could not tell whether
  it is still its own (fail closed).

The lanes, numbered as the regression harness's model numbers its own
(`crates/tpf3mp-testkit/src/regress/model.rs`, `lane`), each a count and a
hash of sorted rows, so the order the engine lists things in does not
matter:

| lane | reads |
|---|---|
| 0 network | street/track endpoints (0.1 m), template and per-lane modes/dimensions/direction; junction positions (1 mm), portable turn/crosswalk references, light preference/resource/phases and flags |
| 1 constructions | every construction by its file and position (0.1 m) |
| 2 lines | every line's number of stops |
| 3 vehicles | each vehicle's state, stop and place on its path: the path edge, the distance along it (1 cm) and the speed (1 cm/s), the simulation's own (`MOVE_PATH.dyn`) |
| 4 economy | the player's balance |
| 5 towns | each town's number of buildings |
| 6 people | the number of people |

Nothing is read by an entity id that two games agreeing on the world could
number differently, except where the save carries it (towns, the player).
A lane the engine cannot read is `err` on every game alike and says why in
`hook.log`, once per Lua state. On build 40408 `getEntitiesWithComponent`
refuses `BASE_EDGE`, `LINE` and `PLAYER` ("Cannot loop over this component
type"), hence the street and line systems.

Vehicles are compared by their place on their paths, not in the world. On
build 40408, with a bus running a line in two games in one room, the bus's
world position (`api.engine.util.vehicle.getPosition`) and its path state
as the frame began (`MOVE_PATH.dyn0`) differed between the games in the
same simulation update, by millimetres to metres: they follow each game's
own frames. Its path state (`MOVE_PATH.dyn`) was the same in every sample.
Read to 1 m, the world position flipped at a rounding edge now and then,
and the room resynced a game whose simulation had not diverged.

Seen on build 40408, two games through the deployed server: every lane
read, and the room found no divergence over several checkpoints.

#### Lane dumps

A digest says *that* a lane differs, not which vehicle, line or edge, nor
how. So after a divergence every game in the room writes the full text of
the diverged lanes to its `hook.log`, entry by entry, at the same two
checkpoints, and `tools/lane_diff.py` diffs two or three games' logs
(`crates/tpf3mp-hook/src/lanedump.rs`). Seen in a three-player playtest on
build 40408: the room said `Diverged { step: 250, lanes: [3] }` to two games
and nothing more.

- **Who dumps.** The room tells only the games whose world differed from
  its verdict (`Notice::Diverged`), and the others are needed to diff
  against. So a game told it diverged says so in the room's chat, a line
  every member's hook reads (the room passes chat to its sender too):

  ```
  [tpf3mp] lane dump lanes=3 steps=300,350 diverged=250: this game's world differed from the room's; every game writes these lanes to its hook.log
  ```

  Its steps are the first two checkpoints at least 20 steps
  (`lanedump::MARGIN_STEPS`) past the step the game runs next, time for the
  line to reach every game. Every game that hears it, that one included,
  dumps those lanes at those steps; the line shows in the Multiplayer
  window like any other. No protocol change: the chat carries it.
- **Bounds.** One dump asked for a minute at most, here or heard
  (`lanedump::GAP`); a line naming steps already planned only adds its
  lanes (both diverged games of a room of three say one). A line heard is
  checked: at most 16 lanes and two steps, each a checkpoint step, not past,
  and no more than 20 checkpoints ahead. One checkpoint writes at most
  `lua::MAX_DUMP_LINES` (5000) entries, all its lanes together, each cut to
  2000 bytes; the rest are counted.
- **By hand.** `TPF3MP_HOOK_LANE_DUMP` in the game's environment dumps
  lanes at every checkpoint, for chasing a desync on purpose: `all`, or
  lane numbers (`3`, `0,3`). `off` dumps nothing, not even after a
  divergence. The hook says in its log what it read.
- **In the game.** The driver passes the dump with the batch that ends at
  the checkpoint (`step::Batch::dump`, `lua::begin_batch`). In that
  batch's last update `tpf3mp_native.dump()` answers `{ step =, lanes = {
  ... } }` once; the game script, right after handing over the lanes, reads
  each lane asked for again with `lanes.dump`, and hands each entry to
  `tpf3mp_native.dumped(lane, entry)`, which the hook writes to its log
  after the batch as `lane <n> step <step> <entry>`, then a line
  `lane dump at step <step>: ... entries written`. Both functions are
  optional in the bridge contract (a hook or mod without them dumps
  nothing, as `built`), so `VERSION` stays 9.
- **The entries.** `lanes.dump` runs the very reader that sums the lane
  up, so it reads the same values the digest hashes: each entry is
  `<key> <field>=<value> ... entity=<e> row=<row>`, the fields the raw
  values the row was made from at full precision (`%.17g`), `row` the text
  hashed. The key is the registry's canonical id where there is one
  (`vehicle-N`, `line-N`, `town-N`, `industry-N`), else `row:<row>`
  (edges, other constructions) or `player` and `people`. Entries are sorted
  by key, ids first, so the same world dumps the same lines in the same
  order whatever order the engine lists it in. Each lane ends with
  `summary <text>`, the lane's text as hashed; a lane that cannot be read
  is one line `err <why>`.

  | lane | fields |
  |---|---|
  | 0 network | `p0`, `p1` (the edge's ends), `template` |
  | 1 constructions | `file`, `x`, `y`, `z` |
  | 2 lines | `stops`, then `stop<i>=<group>/<station>/<terminal>` |
  | 3 vehicles | `state`, `stop` (index), `line`, `edge`, `pos`, `speed` (`MOVE_PATH.dyn`) |
  | 4 economy | `balance` |
  | 5 towns | `buildings` |
  | 6 people | `count` |

  A vehicle's line, a line's stops and their station groups are read for
  the dump only; the lanes hash what the table above says.

After a divergence, gather each game's `hook.log` (a second player in a
Sandboxie box has its own under the box's copy of the data folder) and run:

```
grep 'lane 3 step 300' hook.log          # one game's vehicles at step 300
python tools/lane_diff.py james=james/hook.log bob=bob/hook.log cat=cat/hook.log
python tools/lane_diff.py --lane 3 --step 300 --max 50 --ignore entity a.log b.log
```

For each lane and step dumped by two or more games it pairs the entries
by key and prints the ones that differ, grouping the games that agree and
showing only the fields that differ, and whether the lane's text itself
differed (a difference below the lane's rounding leaves it alike). It exits
1 when an entry differs. `python tools/test_lane_diff.py` tests it.

### Seeds, as built

Ported onto dev from `feat/steam-hook-on-dev` (971c48c, as merged in
87f6b05). The targets are resolved by dev's profile resolution and handed
over at their addresses in the process. Piece 1 has run in the game
(2026-09-30, three games of one room, below) and was rebuilt from what that
showed; what follows marks what is tested only off the game.

**The game scripts' `math.random` is never reseeded through a Lua state the
hook remembers** (changed twice on 2026-09-30). The first cut called
`math.randomseed` from `ecs::Engine::Update` in every state its registrar
detour had marked, assuming two per world. A world load makes and frees
many, one registration group per loader thread; after a rebase the roster
held two freed states, and the next reseed crashed the game in
`lua_getfield`. The second had the mod's game script seed its own state at
the start of its `update` (`tpf3mp_native.seed`, `seeds::current_seed`):
safe, but the game runs its game scripts on a pool of states (piece 1), so
that seeded only the state the mod's script ran in that update, not the one
`reforestation.script.tl` ran in. Now the hook reseeds each script call in
the state the engine hands that call, on the thread about to run it (piece
1). The registrar detour is gone; the mod's own call stays and adds
nothing.

`crates/tpf3mp-hook/src/seeds.rs` is TF3's counterpart for the seeds the
survey (`investigation/TPF3_RNG_2026-09-29.md`, items 5 to 7) found are
not functions of the room's state. Three independent pieces, each failing
closed on its own with its reason in `hook.log` (every line starts with
`seeds:`), installed by `install.rs::install_inner` after the step gate,
the build tools and the main menu's load. A fourth, `ticks.rs`, keeps the
frame counter one of the engine's own seeds reads equal in every game
(piece 4 below).
All three are Windows x64 only, like the step gate.

**1. The game scripts' `math.random`, reseeded per call (live).** The
engine's `math.random` is a `boost::mt19937` per `lua::State`, seeded 5489
at creation and never saved (`reforestation.script.tl:54` draws its
proposal seed from it every update, `exploration_plane.script.tl` its
targets). The game runs its game scripts on a **pool** of such states, one
per worker thread of its job pool: `GameScriptSystem` takes the engine's
shared pool when the machine has eight or more hardware threads, and makes
its own "GameScriptSystem Pool" of half of them only below eight
(`0xaaeb80`). So which state, and so which stream, a script's `update`
draws from depends on how that update's jobs were scheduled. Measured on
2026-09-30, three games of one room that loaded one save (`real67`, the
entity trace): every allocation matched for the first 392, then
`reforestation` planted its first trees at steps 360, 360 and 364, and
every build after that at other steps in each game. The first cut of this
piece reseeded the states `CGame::CGame::lambda_3` registers, from the
simulation thread before each update; in the game that registrar runs 16
to 38 times per load, from the loader's and the pool's threads, and again
while the world runs, so the two states it kept were a different pair in
each game, and the reseed called into states other threads could be
running. It is gone. The hook now reseeds per **call**, in the state that
runs it:

- *Where.* The engine calls a script's callback through a functor that is
  handed the state it runs in, on the thread that runs it, right before
  the Lua function is called in that state
  (`game_script_util::UseFunctionAndCallScriptCreateParamFn`: the functor
  builds the `GameScriptStateNotificationHelper` the script gets as its
  `state`). Three profile targets:
  `game_script_util::Update/lambda_1::_Do_call` (`0xf45450`, `(this,
  lua::State*& rdx, GameScriptData& r8)`, the entity at `this+0x20`),
  `game_script_util::PostUpdate/lambda_1::_Do_call` (`0xf449b0`, the same
  shape, the entity at `this+0x18`; the same code serves
  `HandleApplyCommandBuildProposal`'s functor) and
  `game_script_util::HandleEvent/lambda_1/lambda_1::operator()`
  (`0xf41770`, `(captures, lua::State* rdx, GameScriptData& r8)`, the
  entity at `captures+0x20`). Each offset is where the engine's own code
  reads the entity in that function. The detour reads the `lua_State*` at
  `lua::State+0` and the entity, both through readable checks, and calls
  `math.randomseed(seed)` in that state, then the original runs.
- *Events with a seed.* `HandleEvent` takes an `int const*` seed; when it
  is set (`captures+0x28`), the engine itself calls `lua::State::
  RandomSeed` (`0x2fb17c0`: `math.randomseed(n)` in the state) before the
  script's `handleEvent`. Such an event keeps the engine's seed.
- *The step.* `GameSim::Step`'s update loop applies the update's commands
  and then calls `ecs::Engine::Update(engine, float dt)` (`0x2bb8a50`, its
  only caller, at `0x15955c`), which runs every system's `Update`,
  `ecs::GameScriptSystem::Update` among them. The hook detours
  `ecs::Engine::Update` and makes that update's room step the current one
  (`seeds::current_step`), which the worker threads read when the
  update's scripts run. It is **per update, not per call of the step**:
  one call runs a batch of released steps, batch sizes differ per
  machine, and a seed per batch would itself diverge the replicas. The
  driver arms the batch (`step::StepDriver::on_step` calls
  `seeds::before_updates(next_step, updates)` right before running the
  game's step, which also clears the current step), and the detour takes
  one step per call while the batch lasts. An update beyond the batch, or
  one before any batch is armed (before the room's world, at the game's own
  speed, on the paused path), has no current step, and its script calls
  are not reseeded: the game's own randomness, as outside a room. The next
  arm logs a mismatch between the calls seen and the updates released,
  once, as the sign the "one `Engine::Update` per update" reading is wrong.
- *The seed.* `seeds::script_seed(step, call, entity)` =
  `seed_for(step, script_salt(call, entity))`; `seed_for(step, salt)` is
  `splitmix64(step ^ (salt << 32))` folded to `1..=0x7fff_ffff`, and the
  salt mixes the call's kind (`"updt"`, `"post"`, `"evnt"`) and the
  script's entity, which every replica shares (one save, one numbering);
  never the state or the thread, which differ per machine. So a script
  draws the same numbers at the same step on every game, whichever state
  runs it and whatever ran in that state before. Pinned in tests: step 1
  gives 1216681719 for salt 0, and the update of entity 5023 at step 360
  gets 1108749812.
- *The call.* `math.randomseed(seed)` through Lua C API pointers the
  seeds module resolves from the profile itself (`seeds::SeedApi`: the
  link to the mod calls no Lua code, so its API has no `pcall`):
  `lua_rawgeti(REGISTRY, LUA_RIDX_GLOBALS)` for the globals table in 5.2,
  `lua_getfield` for `math` and `randomseed`, `lua_pushnumber`,
  `lua_pcallk`, stack restored after; a profile without one of them leaves
  the reseed off, and `hook.log` names it. Not the engine's `lua::State::
  RandomSeed`, which calls without protection. A call where
  `math.randomseed` is missing or raises runs unreseeded, said once in the
  log (the same failure on every replica, from the same game code).
- *The detours* are assembly thunks (`#[unsafe(naked)]`): they save the
  four argument registers and `xmm0`-`xmm3`, call the Rust side, restore
  and jump to the trampoline with the stack untouched, so no target's
  signature is assumed; before the trampoline is published a thunk returns
  to the caller without running the original.

**2. `TownDevelopAt` (gated off).** The applier (`TownDevelopAt::Apply`,
`0x9dedf0`) seeds its town developer's `minstd_rand` from the CRT `rand()`,
which the game seeds from the wall clock. The player's
`makeTownDevelopAtCmd` is refused in a room already (the mod's `guard.lua`,
"The player's commands"). The alternative for
later is a thunk on the applier that calls the CRT's `srand` (resolved
with `GetProcAddress` from `api-ms-win-crt-utility-l1-1-0.dll`, then
`ucrtbase.dll`: the UCRT keeps `rand()`'s state per thread, and the
applier's thread is the caller's) with `seed_for(step, "town" + n)`, `n`
the command's number within the step, right before the original. It is
behind `seeds::TOWN_DEVELOP_RESEED = false`: flip it once a room has
shown in the `t` lane that a reseeded `TownDevelopAt` develops the same
town everywhere. Whether `srand` resolved is logged either way.

**3. The CRT math dispatch (measurement).** The UCRT picks FMA3 or SSE2
bodies for `sinf`, `cosf`, `tanf`, `asinf`, `acosf`, `atan2f`, `expf`,
`logf`, `powf`, `fmodf` and the double `sin`, `cos`, `tan`, `exp`, `log`,
`pow`, `atan2`, `fmod` at start-up from `__isa_available` (`sqrtf`,
`floorf`, `ceilf` are exact either way). At bootstrap the hook logs one
`cpu:` line: vendor, family/model/stepping, the `cpuid` bits the CRT's rule
reads (SSE4.2, FMA, MOVBE, AVX, F16C, BMI1, AVX2, BMI2, AVX-512 F/CD/BW/DQ/
VL), `XCR0`, the `__isa_available` level those bits imply by the published
rule (an estimate, not a read of the variable: it is not exported and the
survey named no target for it), and whether the FMA3 bodies are expected
(level 4, AVX2, and up). **The plan:** run the two-replica determinism
probe (DAY_ONE section 4, `tools/probe/tf3`) once on an Intel and an AMD
machine whose `cpu:` lines differ in the FMA3 answer, and once on two
machines whose lines agree. A split that appears only in the first pair,
in the `p` (vehicle positions) and `e` (edge geometry) lanes first (the
movement and path code is where `sinf`/`cosf`/`atan2f` run every step)
and later in `n` and `t`, is the CRT; the same split in the second pair is
not. If it is the CRT, the fix is a detour of those imports to one body
(the survey's known technique, not built).

**4. The paused frames' `tickCount` (`paused-tick`, live; not run in the
game yet).** `crates/tpf3mp-hook/src/ticks.rs`; the findings are in
[investigation/TPF3_TRAIN_PRIORITY_2026-09-30.md](../investigation/TPF3_TRAIN_PRIORITY_2026-09-30.md).
`GameTime.tickCount` (`+0x3c`) counts every update and also every call of
`GameSim::Step` on its paused path; `updateCount` (`+0x40`) counts updates
only (the API's own words, `engine.d.tl:434`). The paused path (the speed
call answered 0) calls the GameTime advance (`0xbace10`, `CGameTime::Advance`,
name ours) with `r8b = 0` at `0x159412`; the advance runs `inc [+0x3c]`
always and `inc [+0x40]` only when `r8b` is set (`0xbace99`). The room's
game takes that path whenever the room holds the world, and during every
room `Load` and `Save`, a machine-dependent number of times, so without
the fix every hold leaves the games' `tickCount` apart for good. The
simulation reads it in the land-vehicle reservation shuffle's seed
(`0xac1b23`: which train or road vehicle gets contested track first), in
`AccountSystem::Update2` (`tickCount % n`), in the town developer's and
street proposals' stamps, and in the base game's notifications game
script (`tickCount % 30` picks which notifications' sim scripts update).
TPF2 Multiplayer made the same call a NOP (`156824d`, `pausedtick`).

- *The patch.* The profile target `GameSim::Step/paused GameTime advance`
  (the call's `E8`, found by the eleven instructions around it, one match)
  is redirected with `CallRedirect`, which refuses unless the call reaches
  the target `CGameTime::Advance`; the fix also refuses unless
  `CGameTime::Advance/tick` (the two increments' eleven bytes) sits `0x89`
  into the advance and reads back as expected. Any miss leaves the call
  alone and says why (`paused-tick fix: off, ...`). The redirect's
  function holds the advance only when the step detour said this call is
  the room's game's (`step::Batch::room`: the driver follows a room's game
  or holds it) and the call is a paused one (`r8b` clear); otherwise it
  calls the game's own advance with the registers the step passed. So a
  game outside a room pauses exactly as before, and the running loop's
  call (`0x15954b`, `r8b = 1`) is never touched. A redirect rather than
  TPF2's NOP, for that reason: the same process can play alone and in a
  room.
- *What skipping drops* (read in the binary): the advance opens an engine
  modification scope (`0x2bbbba0`/`0x2bbc060`) around the increments and
  records a `ComponentChanged` for `GameTime` into each observer's change
  log (`0x2bb6b50`); a held frame records none, which is true. The UI
  readers of `tickCount` stand still while the room holds: the
  notifications script refreshes one of its four type groups per paused
  frame by `tickCount % 4`, and the industry window rate-limits its
  expansion preview by `tickCount`. The town and street builder tools stamp
  their proposals with it; those proposals travel to the room in the
  action's bytes. The horn-sound choice (`0x26948f0`) is presentation.
- *Kill switch.* `TPF3MP_HOOK_PAUSED_TICK=0` (or `off`) in the launcher's
  environment leaves the call alone.
- *The counters in hook.log.* At every checkpoint, and at the first batch
  after a world is loaded, the step detour logs
  `ticks: step <room step>: tickCount=<n> updateCount=<n>`, read through the
  game's getters (`CGameTime::GetTickCount` `0x2a95c0`,
  `CGameTime::GetUpdateCount` `0x2a9680`, names ours) with the `CGameTime`
  the step's own speed call was made on. Two games of one room must log
  equal lines at equal steps. The redirect itself logs
  `paused-tick: holding tickCount on the room's paused frames (held n,
  passed m)` at the first hold and every 16384th: those counts differ per
  machine by design.

**Tested without the game** (`ticks::tests`, `step::tests`): the decision
(only the room's paused frames), the kill switch, the layout check, the
checkpoint line, that nothing installs without the targets, that the
profile states the site's bytes, the driver's `room` and `first_step` on
each batch, and the redirect through a real `call` on a hand-written
function (the advance runs outside a room, is held in it, runs again
after). The static proof (`tf3_static_proof.rs`) checks on the installed
game that both calls reach the advance, with `r8b` 0 and 1, and the
increments' bytes.

**Tested without the game** (`seeds::tests`): the
seed derivation (range, purity, the pinned values, a script call's seed
by step, kind and entity), the batch arithmetic (one step per update, the
mismatch report, disarming), the current step (the released update
running, none between batches, beyond a batch or on the paused path),
which calls are reseeded (none without a step, none over an event's own
seed), the CRT level rule on synthetic CPUs and the running CPU's report,
the driver's step counter, and the reseed's call sequence against the
embedded Lua 5.1: the exact seed reaches `math.randomseed`, and a missing
`math` or a raising `randomseed` refuses with the stack restored. The
static proof checks that the three call sites resolve uniquely at their
addresses in the installed game. **Not tested without the game:** the
thunks against the real targets (the functors' layouts, that one
`ecs::Engine::Update` is one released update), the 5.2 globals path, and
whether `srand` resolves in the game's process. A room's `hook.log` says:
`seeds: detour installed on ...` for the three call sites and
`ecs::Engine::Update`, `seeds: step n: math.randomseed(...) before the
update of script entity e` for the first three calls of each kind and at
every thousandth step, and no mismatch line.

### The order fixes, as built

Ported with the seeds, from the same commits; **not run in the game yet**
either.

`crates/tpf3mp-hook/src/order.rs` ports the four order dependencies to
TF3 Steam build 40408, from the static survey
[investigation/TPF3_RNG_2026-09-29.md](../investigation/TPF3_RNG_2026-09-29.md)
("What must change for lockstep"). `install.rs::install_inner` calls
`order::install` once the step gate is in; each site is its own fix,
installed on its own and failing closed on its own: its profile targets
must resolve (all `required = false`, so a build that lacks one loses the
fix, not the profile), the bytes at the site must be exactly what the fix
expects (the resolver checks the target's `prologue`, and the splice
reads and compares them again before it writes), and every read the hook
makes on the game's thread goes through `image::readable`. A shape the
hook does not recognise is refused for that step, said once per reason in
hook.log, and the engine's own order stands; a panic on the game's thread
switches that fix off for the rest of the game. hook.log carries one
`order fix <name>: installed (...)` or `order fix <name>: off, <why>`
line per fix. Nothing here has run in the game yet: the sites were read
statically with `tools/tpfre`, the sort and stub logic is unit-tested, and
the two mid-function hooks are exercised through their real stolen bytes
on hand-written functions in the test binary.

| survey item | fix | site (RVA) | what it does |
|---|---|---|---|
| 1, land-vehicle reservation order | `land-vehicle-order` | `ecs::LandVehicleMoveSystem::Update2/shuffle` (`0xac1b70`) | sorts the vector of vehicles that want track by entity id before the engine's seeded shuffle (kill switch `TPF3MP_HOOK_LAND_VEHICLE_ORDER=0`) |
| 2, ship and aircraft claim order | `order-measure` | `EdgeReservationManager::Reserve` (`0x255c2e0`), `Reserve_simple` (`0x255c160`) | measured only, when `TPF3MP_HOOK_MEASURE_ORDER` is set |
| 3, road edge entries | `road-entry-order` | `EdgeUseManager::Add` (`0x255e940`), `AddRange` (`0x255cc70`) | keeps each edge's entries in entity order after every append (kill switch `TPF3MP_HOOK_ROAD_ENTRY_ORDER=0`) |
| 4, vehicles at a stop | `vehicles-at-stop-order` | `ecs::SimEntityAtTerminalSystem::Update/vehicles at stop` (`0xb0e35c`) | sorts the vehicles at a line stop by entity id before the boarding loop (kill switch `TPF3MP_HOOK_VEHICLES_AT_STOP_ORDER=0`) |
| 5, platform choice | `platform-order` | `ecs::TransportVehicleSystem::Update2/visit` (`0xb8bccb`), `FindNextFreeTerminal/candidate sort` (`0xb85430`) | asks the vehicles for a free platform in entity order, and puts the candidate terminals in one order before their cost sort (kill switch `TPF3MP_HOOK_PLATFORM_ORDER=0`) |

**The mid-function splice** (`tpf3mp_hookcore::detour::Splice`) is what
the two fixes hook with. A whole-function detour cannot reach a point in
the middle of `Update2`, so the splice replaces `steal` bytes at the site
(whole instructions, at least five) with a `jmp rel32` into a stub
allocated near it (`alloc_near`, as `CallRedirect`'s). The stub is
hand-assembled and pure (`splice_stub`, checked byte for byte in a test):
it pushes every general-purpose register and the flags, which form a
`SavedRegs` block on the game's stack, aligns the stack, saves the
volatile `xmm0`-`xmm5`, calls the fix's `extern "system" fn(*mut
SavedRegs)` with the block, restores everything, runs the stolen bytes
verbatim and jumps back to the instruction after them with an absolute
jump. The register contract is therefore: the hook sees the site's every
register, may change one through the block (the tests do), and the
function sees nothing else changed but memory. The stolen bytes are
decoded with iced-x86 and refused if they branch, call, return, end inside
an instruction or address memory relative to `rip` (they run at another
address). The caller states the bytes it expects at the site, and nothing
may branch into the stolen bytes past their first (established with
`tpfre q xrefs` and noted in the profile). `xmm6`-`xmm15` are callee-saved,
so the hook keeps them as any function would; the upper `ymm` halves are
volatile at every call, and both sites follow a `call` with no vector
instruction between (the disassembly), so nothing lives in them there.

**Land-vehicle reservation order** (the survey's item 1, CONFIRMED). In
`ecs::LandVehicleMoveSystem::Update2` (`0xac0f90`) the engine walks its
family's node list (20-byte records: the entity id, then four component
indices) and pushes an 8-byte `{int32 nodeIndex, float priority}` entry
for each vehicle that wants track into a vector at `[rbp-0x20]..[rbp-0x18]`
(`0xac1a50..0xac1abe`), reads `GameTime+0x3c` and folds it into a
`minstd_rand` seed (`0xac1b15..0xac1b4c`), Fisher-Yates-shuffles the
vector (`0xac1b70..0xac1c63`), **stable-sorts it by the float**
(`0xac1c6c..0xac1d62`: an insertion sort under 33 entries, otherwise
`std::stable_sort` with a temporary buffer; the survey had not seen this
step), and then reserves track in that order (`Reserve` at `0xac1f20`).
So the shuffle only decides the order among equal priorities, and it is
applied to positions in node-list order, which two replicas can hold
differently. The fix splices in at `0xac1b70`, the first instruction of
the shuffle (`mov r13, [rbp-0x20]; mov rsi, [rbp-0x18]; cmp r13, rsi`;
the first two, 8 bytes, are stolen), reads the vector through `rbp`,
reads each entry's entity id the way the engine's own reservation loop
does at `0xac1d72` (`this` at `[rbp-0x80]`, the node-list holder at
`this+8`, the records at `[holder]..[holder+8]`, the record at
`records + nodeIndex*20`, the id at its first dword; that walk is the
profile target `.../records`, and the fix refuses unless it resolves a
few hundred bytes after the site), and sorts the entries in place by that
id, each entry whole. The engine's seed, shuffle and priority sort then
run unchanged on a vector whose order is a pure function of the entity set,
so every replica shuffles the same sequence with the same seed. TPF2's
name key and seeded jitter (`trainorder.h`) are left out: the engine's own
shuffle already keeps a fixed priority from starving anyone, and TF3's ids
are lockstep state (the free-id queue is saved, HOTJOIN_ORDER.md). A
duplicate id, an index past the records, or bounds that are not whole
entries is a refusal. With `TPF3MP_HOOK_MEASURE_ORDER` set the ids in the
engine's order are hashed into the `land` lane, so two replicas' logs
show whether their node lists agreed before the sort (`reordered` counts
how often they were out of order), each call's seed into `seeds`, and
the family's whole node list, entity by entity, into `nodes` (on updates
where at least two vehicles want track): the survey's item 3 for this
family.

The sort makes the shuffle's input a function of the entity set; the
shuffle's **seed** is `GameTime.tickCount` (`0xac1b23`, then `% (2^31-1)`,
0 becomes 1), which the `paused-tick` fix above keeps equal in every game.
The splice reads it from `r8d` at the site (the seed's fix-up leaves it
there, `cmove r8d, r12d`, and the mask loop after it does not touch `r8`).
Whatever the measurement, for one seed value in 256 (about one update in
256, since the seed is the frame counter) the fix logs

```
order fix land-vehicle-order: sample seed=<seed> n=<vehicles that want track> ids=<fnv64 of their ids, sorted>
```

sampled by the seed's value, not by a count of calls, so two games that
agree write the same lines. The engine's shuffle and its priority sort
(each vehicle's line's `reservationPriority`, descending, stable) are a
function of exactly the seed, the ids and the priorities, so equal lines
at equal seeds mean an equal claim order for vehicles of equal priority.
`n` is 0 when fewer than two vehicles want track.

**Vehicles at a stop** (item 4, TPF2's `vehstop`, TERMINAL_WAIT_ORDER.md).
`ecs::SimEntityAtTerminalSystem::Update` (`0xb0db00`) asks
`TransportVehicleSystem` for the vector of vehicles standing at each
`(line, stopIndex)` (`0xb86510`, name ours: it hashes the pair into the
system data's phmap at `+0x90` and returns `&slot.vector`, or a static
empty vector) and hands the waiting cargo and people to those vehicles in
the vector's order with one running index, exactly TPF2's shape. The
vector is append order while the game runs (its owner's `EntityAdded`
adds after a `std::find`, the assert string at `0xb8433a`) and load order
after a load, so a retained and a loaded replica load two trucks at one
stop differently. The fix splices in at `0xb0e35c`, the `mov
[rsp+0x248], rax` right after the getter's call (the profile target
resolves the site by the call and the two instructions after it; the fix
also checks that the `call rel32` before the site reaches the getter
target), and sorts the `std::vector<Entity>` `rax` names by id in place.
Its `vehstop` lane hashes the order found. What is **not** ported: the
unload deques (TPF2's `unload` in `SimEntityAtVehicleSystem`), which were
not located in TF3 in this pass; measure the `vehstop` lane first.

**Ship and aircraft claim order** (item 2) is measured, not changed, for
the reason TPF2's `moveorder.inl` gives: `ShipMoveSystem::Update2`
(`0xaf6120`) and `AircraftMoveSystem::Update2` (`0xa83a40`) index the
family's node vector directly, and permuting an engine-owned list blind is
not safe. With `TPF3MP_HOOK_MEASURE_ORDER` set, both overloads of
`transport::EdgeReservationManager::Reserve(this, engine, typeIndex,
entity, &path, from, to)` (assert-named; every land, ship and aircraft
claim goes through them) are detoured whole, and each claim's entity and
the twelve bytes of each edge of `path[from..to)` go into the `claims`
lane. That lane is the direct proof for items 1 and 2 together: two
replicas whose `claims` hashes agree at every line let the same vehicles
through in the same order. If it splits with the land-vehicle fix on,
the ships and aircraft are next, and the fix is the node-list canon
(TPF2's `step` site, `family_canon.h`: every family's node list in entity
order at each `ecs::Engine::Update`, `0x2bb8a50`).

**Road edge entries** (item 3, `road-entry-order`; TPF2's `roadentries`).
TF3's `EdgeUseManager` keeps 20-byte entries per edge (`{int32 entity,
int32 component, float back, float front, bool forward}`), consumed by
the nearest-occupant searches (`0x255f340`, `0x255ef60`: `vcomiss; jbe`,
the first entry wins an exact tie), `GetNext` (`0x255f760`) and the claim
loop's first-blocked search (`0x255afd0`). The fix is **sort on add**,
because every writer of an edge's list was read and none reorders it:

| writer | what it does to the list | label |
|---|---|---|
| `Add` `0x255e940` (persons; from `PersonMoveSystem`'s node-added callback, `0xaec8e6`) | asserts the entity is not there, then `push_back` (`0x255ea6d`, or the grow path `0x1cf220`) | SEEN |
| `AddRange` `0x255cc70` (vehicles; from `LandVehicleMoveSystem`'s node-added callback via `0x255edc0`) | per path edge `from..=to`: updates the vehicle's entry in place if it has one, else `push_back` (`0x255cde9`, `0x255cedb`, `0x255cffe`) | SEEN |
| `Remove` `0x2561440` (`PersonMoveSystem` node-removed) | find, `memmove` the tail down one entry | SEEN |
| `RemoveRange` `0x2561690` (`LandVehicleMoveSystem` node-removed) | the same per path edge | SEEN |
| `RemoveEntity` `0x2561510` (`TransportNetworkSystem` node-removed) | drops a deleted edge's data whole | SEEN |
| `GetOrAddEdgeData` `0x255d5a0`/`0x255d740`, `VectorMap::Add` `0x255e860` | grow the slot and edge vectors, moving each entries vector whole | SEEN |
| the copy-on-write copy (`0x255f0b0` -> `0x255e2d0`) | copies the data, order kept | INFERRED |
| the readers (`0x255ef60`, `0x255f340`, `0x255f760`, `0x255ff70`, `0x2560270`, `0x2560ef0`, `0x2561160`, `GetPos01sDEBUG`) | read only (their callees: position getters) | SEEN |

So a list is append order (history while running, registration order
after a load, since the lists are rebuilt through the node-added
callbacks) and nothing else moves an entry; a list sorted after each
append stays sorted, with no cost per update, which a per-update sort
could not beat. The fix detours `Add` and `AddRange` whole (both profile
targets already), runs the engine's, and then sorts, by entity id, the
entries of the edges it touched: `Add`'s one edge, `AddRange`'s path
edges `from..=to` (its seventh and eighth arguments). An edge's list is
found as `GetOrAddEdgeData` finds it: from the manager's data (an
`EdgeUseManagerData`), its entity-to-slot index `[+0]..[+8]` (`int32`s),
its slots `[+0x18]..[+0x20]` (72 bytes each), the slot's edges
`[slot]..[slot+8]` (32 bytes each), the entries vector at `edge+8`; every
bound is checked and a shape that does not fit is a refusal for that edge.
The two appenders are handed the data differently: `Add`'s `this` is the
manager, whose data is at `[this+0x18]` (`Add` reads it through the
copy-on-write getter `0x255f0b0`, which answers `[this+0x18]`), while
`AddRange`'s `this` *is* the data (its one caller, `0x255edc0`, calls the
getter and passes the answer on; the manager is its ninth argument, and
the fix checks that `[manager+0x18]` names the data it was given).
Until 2026-09-30 the fix read `[this+0x18]` in `AddRange` too, which is
the data's slot vector, so every vehicle append was refused with "the
edge's entity has no slot" (the only refusal in the three-player
playtest's hook.log, about 70% of the appends) and the vehicles' lists
were never sorted; the persons' (`Add`) were.

Every read on the hot paths (this walk, the order fixes' vectors, the
game scripts' reseed) is checked through `image::Readable`: a per-thread
cache of the regions `VirtualQuery` found committed and readable, dropped
(`image::invalidate`) before every simulation update, before every call
of the game's step, when a world's GUI starts and when a load is asked
for, so a region is asked of the system about once per update, not once
per word (13 times per edge before). The system call costs about 1.7 µs on
the development PC and tens of microseconds inside Sandboxie, which hooks
system calls: in the three-player playtest (`f622599`) the boxed games
spent 37 to 42% of their step time in the hook, most of it in these
checks. Every address the hook reads comes from a structure the engine
keeps live; the checks guard against a layout the hook misreads, and
dropping the cache wherever the engine frees keeps a freed region from
answering. The sort is `road::place`, in place: one scan
finds how far the list is strictly ascending; a list kept sorted is then
either whole (nothing written) or out of order only in the entry just
appended, which is moved into place by binary search; anything else is
sorted whole through a reused buffer. It gives exactly the order of the
reference `road::entry_order` (checked on random lists in the tests),
refuses the same lists, and writes nothing when it refuses. On the
development PC one append to an edge of 2 to 32 entries went from about
5 to 8 µs to about 0.1 µs (`order::splice_tests::road_append_bench`; a
check from the cache is about 7 ns against 1.7 µs asking the system,
`image::tests::readable_bench`), the sort alone
from 94 to 31 ns at 2 entries and 508 to 117 ns at 128
(`order::tests::road_sort_bench`). Both appenders must be detoured, or none sorts. The callbacks run
serially at the end of a modification (INFERRED from their callers, the
engine's node-added dispatch), so the sort writes nothing a reader is
walking. The `AddRange` detour takes all nine arguments (the ninth at the
caller's `[rsp+0x48]`); the measurement's earlier detour forwarded eight.

**Platform choice** (`platform-order`; investigation/TPF3_TRAIN_PRIORITY_2026-09-30.md,
"Platforms"). `ecs::TransportVehicleSystem::Update2` (`0xb8bae0`) walks its
node list (8-byte `{entity, TransportVehicle index}` records at
`[[this+8]]`, as many as the update's `int` argument, spilled at
`[rbp+0x5b0]`) and, for each vehicle en route with its decision flag set,
calls `FindNextFreeTerminal` (`0xb84e20`, its one caller, `0xb8bea1`); a
choice is stored for the rest of the update in a copy of the allocation
map (`StoreTerminalAllocation` `0xb8b960`), so a vehicle visited later
sees what earlier ones took, and the path changes are applied later in
visit order (deferred lambdas). Two sites, each on its own:

- *visit* (`0xb8bccb`): the loop reloads the list's begin into `rdi`
  every iteration (`mov rax,[r13+8]; mov rdi,[rax]`) and reads only
  `[rsi+rdi]` and `[rsi+rdi+4]`; `rdi` is set anew after the loop
  (`0xb8c22f`). At the first iteration (`rsi` 0) the hook copies the
  list, checks that its length is the count, and sorts the copy by
  entity id; at every iteration it points `rdi` at the copy. The engine's
  list is never written (its index into it stays valid), and the same
  vehicles are visited, in entity order. A list already sorted is left
  alone; a list that does not match the count, or that moves during the
  loop, is refused and the engine's order stands.
- *candidates* (`0xb85430`): `FindNextFreeTerminal` gathers 12-byte
  candidates from `LineSystem`'s per-stop table and `std::sort`s them by a
  cost it looks up per candidate (`0xb85453`, a float-only comparator), so
  equal costs keep an introsort order that depends on the input's. The
  hook puts `[r13, r14)` in one canonical order first (by the
  station and terminal words, then the first), so the sort, and the
  search that starts at the current terminal and wraps around, break
  equal costs the same way in every game.

Both sites allocate nothing in the common case: the visit site scans the
list in place and copies it only when it is out of entity order, into a
buffer kept from update to update (`platform::sort_records`); the
candidates are checked and, if need be, sorted in place
(`platform::sort_candidates_in_place`). Each gives exactly the order of
the reference functions (`visit_order`, `candidate_order`), checked on
random inputs in the tests.

With `TPF3MP_HOOK_MEASURE_ORDER` set, the `visits` lane hashes the
engine's visit order before the fix each update, `candidates` counts the
candidate sorts that changed something, and `road` hashes each checked
edge's id and its entities in the order kept.

**The measurement** (`order::measure`). Off, nothing is hooked. With
`TPF3MP_HOOK_MEASURE_ORDER=1` in the launcher's environment (the game
inherits it; a number above 1 is the interval, default 100 updates), three
whole-function detours install: `ecs::Engine::Update` (`0x2bb8a50`, the
per-step engine advance, one caller: `GameSim::Step`'s iteration loop;
its `dt` rides in `xmm1`, which the detour's float parameter forwards)
counts updates and closes each one's lanes, and the two `Reserve`
overloads feed the `claims` lane. `EdgeUseManager::Add` and `AddRange`
feed `appends` from the road entry fix's detours, which install while
measuring even with that fix switched off. The fixes' sites feed the
other lanes. Every `interval` updates one line goes to hook.log:

```
order measure: updates 201..=300: claims=<fnv64>/<n> appends=<fnv64>/<n> land=<fnv64>/<calls> reordered <n> seeds=<fnv64> nodes=<fnv64>/<n> vehstop=<fnv64>/<calls> reordered <n> visits=<fnv64>/<updates> reordered <n> candidates=<reordered>/<sorts> road=<fnv64>/<edges> reordered <n>
```

Updates are numbered from the room's step once the step driver has
loaded the room's world (`step.rs` tells `measure::room_step` the next
step; before that, from the hook's start), on the survey's reading that
one `Engine::Update` call is one update (INFERRED, to be confirmed by
the numbers agreeing with the room's). Two replicas' lines can be diffed
directly; a lane that differs names the container. The hashes are FNV-1a
64 over the raw values, so an entity id that legitimately differs shows
too; TF3's ids are expected equal (the survey), and the `reordered`
counts say whether the sorts changed anything.

### What the hook costs: the `perf:` lines

`crates/tpf3mp-hook/src/perf.rs` times the hook's per-update work where
it runs, and the game's own `GameSim::Step` around the step detour's call
of the original, so the one can be set against the other. Each timed call
reads the clock twice (`Instant`, which is `QueryPerformanceCounter` on
Windows) and adds its nanoseconds and one call to two atomics; no call is
sampled. A timed call costs about 56 ns with the timing on and under 1 ns
off (`perf::tests::a_timed_call_costs_two_clock_reads`, release build, on
the development PC); at a few thousand timed calls a second that is a few
tenths of a millisecond a second, so the timing is **on by default**.
`TPF3MP_HOOK_PERF=0` (or `off`) in the game's environment turns it and its
lines off; hook.log says which at install (`perf: timing the hook's
work, ...` or `perf: timing off (...)`).

Every 10 seconds of wall time, after a call of the step, two lines go to
hook.log (nothing while no step runs, at the main menu):

```
perf: 10.0s: game step 2000.0 ms (200.0 ms/s) in 600 batches, 600 updates (3.333 ms/update); hook 42.5 ms (4.25 ms/s, 2.12% of the game's step); readable cache 90000 hits, 1200 misses
perf: road-entry 19000/9.50ms/0.50us, platform-visit 0/0.00ms/0.00us, platform-candidates 0/0.00ms/0.00us, land-vehicle 0/0.00ms/0.00us, vehicles-at-stop 0/0.00ms/0.00us, reseed 6000/30.00ms/5.00us, paused-tick 0/0.00ms/0.00us, lanes 0/0.00ms/0.00us, lane-dump 0/0.00ms/0.00us, gate 600/3.00ms/5.00us; road-entry refused 12 (12 the edge's entity has no slot)
```

The first line: the window's length; the game's step, its total time,
that time per second of wall time, its calls (batches) and the
simulation updates run (`ecs::Engine::Update` calls, counted by the seeds'
per-update detour, so `0` without it), and the step's time per update;
then the hook: the sum of every piece below, per second, and as a share
of the game's step time; then the readability checks answered from
`image::Readable`'s cache and those that asked the system (each miss is
one `VirtualQuery` or more). Each piece of the second line is
`<name> <calls>/<total ms>/<mean µs>` over the window:

| piece | what is timed | calls are |
|---|---|---|
| `road-entry` | `road-entry-order`'s sort after `EdgeUseManager::Add` or `AddRange` (the engine's append is not counted) | appends |
| `platform-visit` | `platform-order`'s visit site: the copy and sort at the loop's first iteration, the redirect at every one | iterations of the chooser's loop (one per vehicle per update) |
| `platform-candidates` | `platform-order`'s candidate sort | `FindNextFreeTerminal` sorts |
| `land-vehicle` | `land-vehicle-order`'s sort | reservation updates |
| `vehicles-at-stop` | `vehicles-at-stop-order`'s sort | stop lookups |
| `reseed` | the per-call reseed of the game scripts (update, postUpdate, handleEvent), its checks and `math.randomseed` | script calls |
| `paused-tick` | the paused-tick redirect's decision (the game's own advance, when passed on, is not counted) | paused frames |
| `lanes` | `tpf3mp_native.lanes(t)`: the lanes' text read off the Lua stack at a checkpoint (the mod's own reading of the world is Lua, inside the game's step) | checkpoints |
| `lane-dump` | `dump()` and `dumped(lane, entry)` | calls |
| `gate` | the step detour's own work around the game's step: the room's session, the driver, the lane digests, the log | step calls |

The line ends with the road fix's refusals in the window, by reason.

Reading them: every piece but `gate` runs inside the game's step, so the
game's step time includes it; `gate` runs around it. `reseed` runs on the
threads that run the game scripts, which may run side by side, so its
total is time spent on those threads, not wall time; the share is then an
upper bound of what the hook adds to a frame. A share of a few percent is
the hook's; a game step that takes more per update with the same world is
the game's.

**A/B the fixes.** Every piece has a kill switch in the game's
environment (the launcher's environment reaches the game). Run the same
save with and without one, a minute or more each, and compare the first
lines' `ms/update` and the piece's total:

| switch (`0`, `off`, `false` or `no`) | turns off |
|---|---|
| `TPF3MP_HOOK_ROAD_ENTRY_ORDER` | `road-entry-order` |
| `TPF3MP_HOOK_PLATFORM_ORDER` | `platform-order`, both sites |
| `TPF3MP_HOOK_LAND_VEHICLE_ORDER` | `land-vehicle-order` |
| `TPF3MP_HOOK_VEHICLES_AT_STOP_ORDER` | `vehicles-at-stop-order` |
| `TPF3MP_HOOK_PAUSED_TICK` | the paused-tick redirect |
| `TPF3MP_HOOK_SCRIPT_RESEED` | the game scripts' per-call reseed (the per-update detour stays, so the mod's own `tpf3mp_native.seed` still works) |
| `TPF3MP_HOOK_LANE_DUMP=off` | lane dumps, even after a divergence |
| `TPF3MP_HOOK_MEASURE_ORDER` | (unset by default) the order measurement, which adds its own detours and hashing when set |
| `TPF3MP_HOOK_PERF` | the timing and these lines |

Each switch changes what the game computes, so a game with one off
diverges from a room whose other games have it on: A/B in a room where
every game has the same switches, or alone.

## Release-day procedure: adding a target for a new build

The first TF3 build's targets are already located (RVAs, RTTI/source
names) in
[investigation/TPF3_RECON_2026-09-29.md](../investigation/TPF3_RECON_2026-09-29.md):
the command queue (`CommandList::Add`), the sim step (`GameSim::Step`,
`CGame::RunGameSimLoop`), `CGameTime`, the two-`GameState` swap, the
player/company commands, and a lockstep step-budget global. This procedure
turns each into a verified profile target; the recon page also lists the
reconciliations to settle in-game first (e.g. `CommandList` vs
`DeferredCommandBuffer`).

1. **Archive the build.** Record the executable SHA-256, file size and PE
   timestamp (`BuildIdentity::of_file`), plus the Steam build/manifest ids. Keep
   a private copy (see [DAY_ONE.md](DAY_ONE.md)).
2. **Find the function** with the RE pipeline, and note its RVA and the bytes at
   its start. `tools/tpfre` indexes the executable in seconds and answers
   `func`, `callers`, `xrefs`, `str`, `dis` and `whois` queries on it
   ([its README](../tools/tpfre/README.md)).
3. **Write a signature.** Take the opening bytes; replace every relative or
   absolute displacement with `??`; extend only until the pattern is unique
   across the scanned section. Record the exact, wildcard-free `prologue` (at
   least the number of bytes the detour must steal - 5 for a near hook, 14 for a
   far one, on an instruction boundary).
4. **Add a `[[target]]`** to the build's profile with `name`, `signature`,
   `offset`, `prologue` and `required`.

   `tools/re/make_profile.py` does steps 3 and 4 for x86-64 builds, from the
   binary and the symbol map `name_functions.py` wrote:

   ```
   python tools/re/make_profile.py TransportFever3.exe out/TransportFever3.symbols.json \
       GameSim::Step CGame::Step -o profile.toml
   ```

   It writes the `[build]` identity (SHA-256, size, and the PE timestamp on a
   PE) and one target per function, with `offset = 0`. Displacements it
   wildcards: branch and call targets (rel8 and rel32), RIP-relative operands,
   and immediates or absolute displacements that point into the image. The
   signature starts as the prologue's instructions and grows one instruction at
   a time until it matches once in the function's on-disk section, never past
   the function's end or `--max-length` (128) bytes. The prologue covers
   `--steal` bytes, 14 by default: a far jump, since how far the detour lands is
   only known at install. Like the engine, it refuses a prologue holding a
   branch, call, return or interrupt, and keeps RIP-relative data operands,
   which the engine relocates. Every refusal names the target and the reason: a
   name shared by several functions (pick one with `NAME@0xRVA`), a function
   byte-identical to another (never unique), a branch too early to steal around.
   `tpfre q <db> sig NAME --toml` applies the same rules to one function and
   prints its `[[target]]` block (identical to make_profile's on TPF2's
   targets), to try a target before writing the profile.
   `tools/re/test_make_profile.py` checks the tool on a synthetic PE and keeps
   `tpf3mp-hookcore/tests/data/make_profile_fixture.{pe,toml}` current, which
   `tests/make_profile_fixture.rs` resolves with hookcore itself.
5. **Verify.** Resolve the profile against the **in-memory module image** of the
   running build and confirm the target resolves uniquely to the expected
   address and that the prologue matches. Keep a static check against an
   archived copy where the code section is readable on disk.
6. **Never widen a signature to force a match** on a build you have not archived.
   An unknown build must stay unknown, so the hook fails closed.

## What was verified on the TPF2 binary

Against `TransportFever2.exe`, Steam build 35924 (SHA-256
`782b904a...585175c`, size 72,843,280, PE timestamp `0x675ABCC6`, image base
`0x140000000`), the profile in `tpf3mp-hookcore/tests/data/tpf2_build35924.toml`
resolves all five targets, each **matching exactly once** across `.text`:

| target | RVA |
|---|---|
| `GameSim::Step` | `0x15aa00` |
| `CGame::Step` | `0x118e90` |
| `CGameTime::GetSpeed` | `0x2877a0` |
| `UI::CMenuUI::StartSavegame` | `0x6785c0` |
| `UI::CMenuUI::CreatePage` | `0x663370` |

`CGameTime::GetSpeed` sits next to two near-identical siblings, so its signature
runs past the (wildcarded) call to the distinguishing `mov eax,[rax+4]`;
`StartSavegame` and `CreatePage` share a seven-`push` prologue, so each signature
runs to its distinct `lea`/frame bytes (and `CreatePage` to the `mov
[rsp+0x330],rbx` store that separates it from a twin at `0x215c480`). The test
also confirms the resolver refuses a modified copy (corrupting one target's
bytes yields a `Missing` refusal) and refuses a mismatched build identity.

**DRM note.** This build carries a SteamStub section (`.bind`, high entropy),
which can decrypt code at load time. For build 35924 the code section is
nonetheless **readable on disk**: all five prologues match the on-disk `.text`
exactly, consistent with the RE survey's ~88,000 assert-string references found
in the same on-disk section. On-disk verification is therefore valid *for this
build*. It is not guaranteed in general - a future build could encrypt `.text` -
which is why the production resolver scans the in-memory, unpacked module image,
and why on-disk scanning is documented as a development convenience only.

## What a shipped mod hooks on the same build

Build 35924 has a shipped lockstep mod hooking it, [TpF2 Multiplayer](https://github.com/silver2127/tpf2-multiplayer)
(0.6.1.12, 2026-09-20), so every target in the profile and everything in this
section runs in players' games rather than in a test. RVAs are from image base
`0x140000000`. Names are the ones the binary carries in its `__FUNCSIG__` assert
strings where it has one (`tools/re/name_functions.py` recovers those); the
rest are the mod's own names for functions it identified by decompiling or by
differential capture. Steal sizes are the bytes that mod's detour engine
overwrites; its engine refuses RIP-relative instructions in a prologue rather
than relocating them, so its steals are a conservative bound for one that does.

### The five profile targets, as the mod uses them

| target | RVA | how the mod uses it |
|---|---|---|
| `GameSim::Step` | `0x15aa00` | Not detoured whole. Two sites inside it are patched: the calls to `CGameTime::GetSpeed` at `0x15aa30` and `0x15aae4` (a fractional speed scales the batch interval) and the paused branch's `call 0xaea970` (GameTime advance) at `0x15aa4a`, so a paused game advances `GameTime+0x30` per simulation step and not per render batch. Both siblings of `GetSpeed` are real: the profile's extended signature is the right call. |
| `CGame::Step` | `0x118e90` | Detoured, 16-byte steal, for pacing (the leader is the clock; joiners pace to it). |
| `CGameTime::GetSpeed` | `0x2877a0` | Read through its call sites rather than hooked; `GameTime::get` at `0x2877c0` returns the counter at `+0x30`. |
| `UI::CMenuUI::StartSavegame` | `0x6785c0` | Detoured (the share observer: a host that loads another world pushes it), and called directly to load a shared save in-process: build a `SaveGameId` `{wstring path; string name; string namespace}`, get its `SavegameInfo` from the save manager (`0x2e6ca0`; the manager is `+200` on the app object from `0xbb23c0`), default-construct `LoadGameParams` (`0x553b70`, 0x138 bytes) and call from `CMenuUI`'s own per-frame update, vftable `0x301dc38` slot 33 (`0x672b10`), on the main thread, where the game starts its own queued loads. Guards on the menu object: `+0x4e8` non-zero while a game runs, `+0x1988` "initialization already active", `+0x19a0` a queued load. |
| `UI::CMenuUI::CreatePage` | `0x663370` | Detoured, 20-byte steal (the Multiplayer panel on the title menu). Two more menu entries go with it: the list-add at `0x22d99e0` (15) and the main-page builder at `0x667bc0` (14). |

### The command pipeline: two hooks, not one

TF3's command surface is now documented, not guessed: Urban Games'
reference lists **61 `api.cmd.make*Cmd` factories** with their argument
types, recorded in
[investigation/TF3_OFFICIAL_API_2026-09-29.md](../investigation/TF3_OFFICIAL_API_2026-09-29.md).
A TF3 profile's factory targets are found for that list, not ported name
for name from TPF2's. Two entries change the design directly:
`makeWorldBuildProposalCmd` takes a fifth `playerInitiated` argument (a
possible player-vs-replay signal, see below), and companies are commands
(`makeGameAddPlayerCmd`, `makeEntitySetPlayerCmd`), so ownership changes go
through this same pipeline rather than the native, assert-bypassed
`setPlayer` binding TPF2 patched.

Every player action becomes a `Command` built by a `make_cmd::*` factory and
handed to `CommandList::Add(list, OUT handle, cmd, ..., callback)`. The mod
hooks both, and the reason is worth carrying into a TPF3 profile:

- The **factory** hook sees *what* the command is, while its arguments are still
  the caller's typed structures (a proposal, a `component::Line`, a vehicle
  configuration), which is the only moment they are cheap to decode.
- The **`Add`** hook is the only place a command can be *cancelled*: it zeroes
  the result handle and returns without queueing. The factory cannot cancel;
  its caller still holds the command.
- The mod's own replays go through the same factories (the script's
  `api.cmd.*` path), so the **return address of the factory call** is the only
  thing that tells a player's command from the mod's replay of one. That
  caller-RVA filter is load-bearing, not tidiness: without it every replay is
  captured again. On TF3 it may not hold: the GUI is script, and if the
  stock tools build their commands through `api.cmd` as our replays do,
  both arrive from the same caller. Check this before porting the filter
  ([investigation/TF3_MODS_2026-09-27.md](../investigation/TF3_MODS_2026-09-27.md)).

| factory | RVA | steal | |
|---|---|---|---|
| `BuildProposal` | `0x9dc750` | 19 | roads, track, constructions, terrain, assets, the bulldozer: one command, told apart by the proposal's shape ([BUILDING.md](BUILDING.md)) |
| `CommandList::Add` | `0x9d2a00` | 18 | the cancel point |
| `BuyVehicle` | `0x9dca00` | 15 | its UI waits on the result entity |
| `SellVehicle` | `0x9de380` | 20 | |
| `ReplaceVehicle` | `0x9dddb0` | 15 | its UI waits on the result entity |
| `SendToDepot` | `0x9de6f0` | 20 | |
| `SetLine` | `0x9dea10` | 18 | |
| `CreateLine` | `0x9dcde0` | 19 | its UI asserts on an empty result |
| `UpdateLine` | `0x9df4e0` | 19 | |
| `DeleteLine` | `0x9dd190` | 20 | |
| `Reverse` | `0x9ddfe0` | 20 | a toggle: replaying an uncancelled one applies it twice |
| `SetColor` | `0x9de8a0` | 20 | `r9 -> CVec3f*` |
| `SetName` | `0x9deb70` | 15 | `r9 -> std::string*` (MSVC SSO) |
| `SetGameSpeed` | `0x9de9e0` | 21 | the clock buttons |
| `SetDate`, `SetCalendarSpeed` | `0x9de9b0`, `0x9de870` | 21 | the editor's date controls |

Three rules the cancel point taught, each after a crash or a wedged tool:

1. **A cancelled command's completion callback is a contract.** Commands whose
   UI waits on the result (the build tools, `BuyVehicle`, `ReplaceVehicle`)
   must have their callback fired with a zeroed result at `Add`, or the tool
   hangs for the rest of the session. The callback is a `std::function` whose
   impl the game builds on the stack (`{vftable, captured this}`; `_Do_call`
   is vftable slot 2) or on the heap (impl pointer at `r9+0x38`).
2. **Fire-and-forget commands must not have it fired.** `SetLine` and
   `Reverse` fired with the success byte still 0 make the UI take its failure
   branch ("unable to find a path to a stop"). Suppress without firing.
3. **A callback that asserts on an empty result is moved, not fired.**
   `CreateLine`'s callers (`UI::LineList` `0x610490`, `UI::LineManager`
   `0x6154a0`) assert `resultEntity != ecs::Entity()`. The mod moves the
   callback object into a stash and fires it later, from a later `Add` on the
   same thread, with a stand-in result naming the entity the replay created
   (a 16-byte entry `{int32 entity; double gen; int32}` whose generation must
   match the registry's, `[reg+0xb8]+id*12`).

And one that holds everywhere: **never cancel when the decode failed.** A
command the mod cannot ship in full runs natively and is read back afterwards;
cancelling it would lose the player's action.

### Layouts the capture depends on

Every vector is read at the game's own length (`{begin, end, cap}`), with a
sanity bound on the span and a readability check on every page it touches,
under SEH: a misread pointer fails the decode loudly, and a failed decode is
never cancelled.

- `component::Line`: `vector<Stop>` at `+0x00`, `int waitingTime` `+0x18`,
  `VehicleInfo` `+0x1c` (8 bytes: a `std::bitset<16>` of transport modes plus
  4). `VehicleInfo` is **engine-maintained**: the sim-side `UpdateLine`
  handler (`0x9d9fd0`) restores its own copy, and a command cannot set it.
- `Line::Stop`, 0xa8 bytes: `Entity stationGroup` `+0x00`, `int station`
  `+0x04`, `int terminal` `+0x08`, `vector<StationTerminal{int,int}>
  alternativeTerminals` `+0x10`, `int loadMode` `+0x28` (0..3), two `float`
  waits `+0x2c`/`+0x30`, `vector waypoints` `+0x38`.
- A proposal's edge record, 120 bytes: node ids `+0x00`/`+0x04`, tangents
  `+0x10`/`+0x1c` (3 floats each), `BaseEdge` type and type index
  `+0x28`/`+0x2c` (1 bridge, 2 tunnel), and an optional `PlayerOwned` as
  `{int32 player +0x70; uint8 present +0x74}`.
- `TransportNetwork` and the other components are reached through the engine's
  type index: `GetComponentDataIndex` (`0xd0920`) with the component's
  `RTTI_Type_Descriptor`, then `engine+0x88[typeIndex]`, entries of 0x48
  bytes, data at `+0x68` (indices below `0x40000000`) or paged at `+0x80`.

### The game has two engines

`CGame::RunGameSimLoop` (`0x1184d0`) keeps two `GameState` objects at
`CGame+0x168 -> { GameState*[2], ..., int current at +0x20 }` and copies one
into the other every frame with `GameState::Replicate` (`0x241630`);
`CGame+0x158` is whichever is current this frame, and that is what the UI's
`GameStateProvider` returns (`0x8badf0`: `mov rax,[rcx+8]; mov rax,[rax+0x158]`).
`GameState+0x28` is that state's `ecs::Engine`, and each engine owns its own
system objects. A command carries a specific engine pointer, so anything
computed on its behalf (the mod re-runs the line editor's platform assignment
at the replay) has to take the state whose `+0x28` is that engine, never
"this frame's".

### What lockstep needed beyond command capture

Identical commands at identical steps were not enough; the mod patches four
places where the engine's order depended on memory layout or on a seed:

| | RVA | steal | |
|---|---|---|---|
| train reservation order | `0xabe02d` | 16 | the engine shuffles the order trains claim track with a `minstd_rand` seeded from `GameTime+0x30` over node-list positions, which differ per machine; the detour orders by train name with a seeded jitter |
| free space on a road edge | `0x2117350`, `0x2117140` | 5 | the sum is taken in ascending order in `double`, so every peer gets the same float |
| road edge use entries | `0xa64473` | | kept in name order after `EdgeUseManager::Add` |
| ship and aircraft claim order | `0xa6c1e0`, `0xa2bc60` | 5 | measured only: the family node vector is engine-owned |

The world comparison that finds the remaining divergences hashes geometry and
state, never entity ids: ids, seeds and town growth differ legitimately
between machines that agree on the world.

### UI patches for companies mode

Small in-place patches, each verified against the exact bytes at the site
before it is applied: the line editor's station owner gate (`0x609631`, a
5-byte `cmp eax,[rbx+0x28]; je` with accept `0x609605` and reject `0x609636`),
three owner gates that hide other players' icons, the icon draw call
(`0x80b613`), the station label background (`0x80a0ee`), a foreign entity's
window opening read-only (`jne` at `0x8b3060`), the window bind (`0x8b2390`),
and the HUD station and depot icons (`0x5e38e1`, `0x5e45d0`, depot ctor
`0x5e2b70`). The `setPlayer` binding's ownership assert is bypassed at the
`je` `0x11677a1`. Each is a separate patch with its own byte check, so a build
change disables one feature rather than the mod.

### Detour rules that held up

- **Verify, then steal.** Every target's expected bytes are compared before
  the patch; a mismatch logs and leaves that feature off. Steals stop on an
  instruction boundary at or past 14 bytes and cover only plain,
  position-independent instructions; a `call`, a jump or a RIP-relative
  operand in the prologue is a refusal. Where only five bytes are safe to
  take, a page within ±2 GiB of the site is allocated for the detour and the
  five bytes become a `jmp rel32` into it.
- **Install before the target's first run.** The mod's proxy `alut.dll` loads
  every DLL from `DllMain`, before the game's entry point, so no thread can
  be inside a target when it is patched.
- **The cancel is gated on evidence.** Cancelling is only safe because
  something replays, so it is switched on by fresh evidence from the script
  half on disk (its per-tick status file). With the mod's Lua side absent, the
  hooks capture nothing and cancel nothing, and the base game is unchanged.
