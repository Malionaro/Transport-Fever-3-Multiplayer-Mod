# Protocol

This page defines the semantics and invariants of the TPF3-MP protocol. The
exact fields live in `crates/tpf3mp-proto`, which is the source of truth; this
page explains what they mean and which orderings are guaranteed. Design
background is in [ARCHITECTURE.md](ARCHITECTURE.md).

## Connection

One QUIC connection per client (ALPN `tpf3mp`, TLS 1.3 only). It carries
three kinds of stream:

- **The control stream** is bidirectional and opened by the client first. It
  carries the handshake, requests and responses, room updates, notices, and
  the client's game messages (intents, progress, checkpoints, saves).
- **The turn stream** is unidirectional, server to client. The server opens it
  when the client enters a running game. It carries the room's ordered event
  log and nothing else.
- **A bulk stream** is bidirectional and opened by the client, one at a time.
  It moves one world snapshot, in either direction (see "Snapshots").

Every stream starts with the version preamble. Frames and limits are described
in the `tpf3mp-proto` crate docs.

### Tunnels

A client whose network blocks UDP runs the same QUIC connection through a
WebSocket tunnel instead:

- The client opens a WebSocket to a `wss://` URL, by default
  `wss://<server host>/tpf3mp` on port 443, offering the subprotocol
  `tpf3mp-quic-1` and sending no `Origin` header. The server answers with
  that subprotocol, or refuses: `403` for a request with an `Origin`, as
  browsers send (a web page must not be able to open tunnels from its
  visitors' machines), and `429` for an address over its share of
  tunnels. A server serving the tunnel's TLS itself counts the address
  before the handshake, and closes such a connection at once instead.
- Each binary message carries exactly one QUIC datagram of at most 2048
  bytes. Text messages and larger messages end the tunnel. Pings and pongs
  are allowed, carry nothing, and keep nothing open.
- Everything else is unchanged: the QUIC handshake, TLS 1.3 with the
  server's certificate, the streams and all limits. A proxy in front of the
  server sees only QUIC ciphertext. The tunnel's own TLS only gets the
  traffic through the network; the client accepts the tunnel's certificate
  if public authorities vouch for it or if it is the server's pinned one.
- The server merges tunnels into its one QUIC endpoint, where each tunnel
  appears at an address of its own. Per-address limits count the tunnel's
  client: its TCP address, or behind a proxy the last `X-Forwarded-For`
  entry. A tunnel's client counts as having proved its address, so it is
  never asked for a QUIC retry.
- A tunnel is for one QUIC connection. It closes when no connection has
  been admitted through it for 10 s, so within 10 s of opening and 10 s
  after its connection ends, and when no datagram crosses it for 60 s.
  QUIC keep-alives cross an open connection's tunnel every 5 s.

Clients try UDP first. If UDP has not connected after 3 s, the tunnel joins
the race, and whichever connects first is kept. Once a connection needed the
tunnel, reconnecting starts both at once. Over a tunnel, one lost TCP
segment holds up everything behind it, so it is only a fallback.

## Handshake and identity

1. Both sides exchange the preamble. If the versions differ, both close with
   `VERSION_MISMATCH`, and the client tells the player which side is older.
2. The client sends `Hello` with:
   - its version and platform;
   - a display name;
   - its **identity key**, a per-install Ed25519 public key;
   - a **proof**: an Ed25519 signature over `"tpf3mp-auth-v1" || E`. `E` is
     32 bytes of TLS keying material exported for this connection (label
     `EXPORTER-tpf3mp-auth`, empty context). Only a party inside this TLS
     session can compute `E`, so a proof cannot be replayed on another
     connection.
3. The server verifies the proof and answers `Welcome` or `Reject`.

A player *is* their identity key. There are no accounts or passwords. A
player who reconnects with the same key is the same player.

## Rooms

A room has a name, an owner, a player limit, settings, members, and a phase:
**lobby** or **running**.

- **Creating.** Any player can create a room and becomes its owner. One
  network address can have at most 8 rooms open (`TooManyRooms`).
- **Rules.** `Welcome` lists the rules the server offers (`RulesOffer`: a
  name of up to 32 bytes and a description), the default first. `CreateRoom`
  names one, or none for the default; a name the server does not offer is
  refused (`UnknownRules`). `native` is the game's own rules and economy:
  the server orders commands without judging them. The room view and every
  `TurnStart` name the room's rules, and they never change for the life of
  the room.
- **Closing.** A room closes when its last member leaves. A running game
  also closes when nobody has been connected to it for 10 minutes; until
  then, disconnected players keep their seats and can resume.
- **Chat.** Any member can say something to the room (`Chat`, up to 280
  bytes). Every member hears it, the sender too, so everyone sees one
  conversation. A player may send one message a second, with a burst of
  five.
- **The operator's notices.** The server sends `Notice` (up to 280 bytes)
  to every connected client, in a room or not, when its operator announces
  something, such as a restart coming (`POST /announce` on the admin
  endpoint, see OPERATIONS.md). A client whose queue is full misses it
  rather than holding the others up.
- **Kicking.** The owner can remove another player (`Kick`), for example
  one whose game froze. The player receives `Kicked` and leaves as if they
  had chosen to; in a running game every replica sees `PlayerLeft` at one
  step. A kicked player cannot join that room again.
- **Invites.** The server answers with an **invite**, a code of six
  letters and digits such as `K7QM2X` (`tpf3mp_proto::Code`, D13 in
  DECISIONS.md): upper case from `23456789ABCDEFGHJKMNPQRSTUVWXYZ`, with
  at least one letter and one digit, typed in either case, and checked
  when decoded. The server finds the room by an HMAC of the code under a
  server-side pepper, stores only that, and gives no two open rooms the
  same code. Invalid invites, unknown rooms and wrong passwords all fail
  the same way (`BadInvite`), so invites cannot be used to probe which
  rooms exist. An address that sends 20 of those in 10 minutes gets
  `RateLimited` for every join until the 10 minutes are up.
- **Updates.** Members receive the full room view (`RoomUpdate`) whenever it
  changes. Updates and responses are independent messages: a `RoomUpdate`
  caused by a request can arrive before that request's `Response`.
- **Content.** A client declares what its game runs with `DeclareContent`,
  once per connection, before joining a running game: a `ContentManifest`
  of the game's build and its active mods in load order, each a name of up
  to 96 bytes and a version of up to 32. A manifest lists at most 2,048
  mods within 48 KiB; `ContentManifest::new` summarises the mods beyond
  those by their count and a digest (`Unlisted`), and the server refuses
  a larger one (`InvalidContent`). The server derives the **content
  fingerprint** from the manifest, a SHA-256 that is equal only for the
  same build and the same mods in the same order, and rooms compare those.
  A room tells each member whose content differs from its own (the owner's
  in the lobby, the game's once it runs) how, with `ContentDiff`: the
  builds if they differ, the mods the member lacks, the mods the room
  lacks, and the mods both run in other versions, naming up to 32 of each
  and counting the rest. It says so again whenever that changes, and sends
  `ContentDiff(None)` once the member matches. The game's manifest is kept
  in its log, so a restored game can still say how a newcomer differs.
- **Starting.** In the lobby, members declare their content and toggle
  **ready**. The owner can start the game only when every member is ready,
  all fingerprints are equal, and the world the owner handed over, if any,
  has arrived (`StartWorldPending` until then).
- **The first world.** On a server that keeps snapshots, the owner's world
  is everyone's, and every player loads it as a save, the owner too (see
  "Snapshots"). Worlds generated separately on each machine could differ
  between platforms, and a game that kept its world differs from one that
  loaded its save (below). Everyone still holds the clock until loaded.
  The owner provides it one of two ways:
  - **Handed over in the lobby.** The owner's client sends
    `StartWorld(SavedWorld)`, a save it holds (the launcher's
    `--start-save`), before the game starts. The room answers `Upload`
    with event 0 at once, and the owner's client uploads it on a bulk
    stream as it uploads any save the room asks for. When the game
    starts, every member's first turn stream names that world, starting
    from the game's first turn, so every game loads the same file from its
    main menu at the same time and no game saves first. Only the owner
    may send it, only in the lobby, and only to a server that keeps
    snapshots (`WorldsNotKept` otherwise); another replaces it, and it
    goes with the owner if the owner leaves. The room gives up on one
    that does not start arriving within 30 seconds, and the game then
    starts as below.
  - **Saved by the owner's game.** Without one, the owner's game loads
    the world, the room saves it before the first step, and every player
    loads that save.

  Without snapshots, every player loads the same world locally.
- **Joining a running game.** A newcomer's declared content must equal the
  game's; otherwise the room sends it a `ContentDiff` and refuses the join
  with `ContentMismatch`. Every replica sees
  `PlayerJoined` at one step, and the newcomer receives the room's world to
  load, every other player with it (see "Snapshots"). Only a server that
  keeps snapshots allows this; others answer `GameRunning`.

## Turns: the ordered event log

A running room has a **sequencer**. Every tick (100 ms by default) it seals a
**turn**: a sequence number, the step **frontier** `sealed_through`, the
session speed, and the events appended since the previous turn. Each event has
a room-global sequence number and an execution step.

Invariants every client relies on:

1. **Events apply before their step.** An event with step `N` is applied
   after step `N-1` has executed and before step `N` executes. Events for the
   same step apply in sequence order.
2. **Steps run only when sealed.** A client never executes step `N` unless a
   received turn has `sealed_through >= N`.
3. **A sealed step is closed.** Once a turn announces `sealed_through = S`, no
   later event has a step `<= S`. The sequencer gives every new event the step
   `sealed_through + 1`.
4. **Turns are gap-free.** Turn numbers start at the number in the stream's
   `Start` message and increase by one; event sequence numbers increase by
   one across turns. A client that sees a gap closes the connection with
   `PROTOCOL_VIOLATION` and reconnects.

A stream's `Start` also says where the world it continues stands: every
step up to `sealed_through` has run, and every event before `next_event` is
applied. When it names a `world`, the client loads that world first (see
"Snapshots").

Together these make every replica apply the same events at the same point in
simulation time, whatever its latency. Clients enforce them strictly
(`TurnFollower`): an event whose step is not the previous turn's frontier
plus one, a counter at the end of its range, or more than 256 MiB of events
waiting is a protocol violation. A client also holds at most 64 MiB of
turns the game has not taken yet, and its link to the game about 16 MiB of
events the game has not read, then stops reading until the game catches up.
Invariant 1 also makes building work
while paused: a paused client has executed `sealed_through` and can apply the
events for `sealed_through + 1` at once, without running a step.

**Pacing.**
- **The server owns the clock.** It advances an ideal step count at
  `steps_per_second × speed`.
- **Frontier.** It seals up to that count plus a small **lead** (the room's
  input delay setting). Clients buffer on their own (see "Playout" below),
  so the lead does not decide what players feel.
- **Nobody runs ahead.** The frontier never goes further than a bounded
  window past the slowest active member's reported progress. A slow machine
  therefore slows the room instead of forking from it.
- **Speed.** Speed `0` pauses. Speed only changes pacing, never simulation
  results, so it travels in the turn header, not as an event. A speed change
  always produces a turn, even when nothing else changed, so a pause is
  announced.
- **Loading.** The clock holds until every member has reported progress
  `0`, meaning it has loaded the world.
- **Catching up.** A member who reconnects does not hold the room until its
  progress is back within the pacing window.
- **Stalls.** A member stops holding the room when either:
  - it has sealed steps to run but has not advanced for the stall timeout
    (20 s by default: long enough for an autosave);
  - it is still loading after the load timeout (5 min by default).

  Both defaults come from stock-sized TPF2 worlds. On a big map an autosave
  writes about 1.4 GB and pauses the game for 15-20 s, and a world entry
  measured 230-290 s ([BIGMAPS.md](BIGMAPS.md)), so a server for such maps
  raises both (`--stall-timeout-secs`, `--load-timeout-mins`; see "Big
  maps" in [OPERATIONS.md](OPERATIONS.md)).

  This way one frozen game, or a client that stops reporting, cannot stop a
  room for good. The member rejoins the pacing set by catching up; nobody is
  kicked. Pausing restarts every member's stall timer.

**Playout.** A game does not run steps the moment they are sealed. It plays
at the room's pace behind a jitter buffer of its own
(`tpf3mp_agent::Playout`):

- **Schedule.** Each step plays a small margin after the latest arrival of
  the frontier seen recently, carried forward at the room's pace.
- **Poor links.** A late turn, from jitter or a retransmission after loss,
  grows the buffer for a while. Afterwards the client converges back by
  playing 5% faster.
- **Catching up.** A client more than a second behind its schedule plays at
  once. This is how it catches up after reconnecting.
- **Pausing.** A client plays on to the frontier, which the pause freezes.
  Every client therefore pauses at the same step, and events sent while
  paused apply at once.
- **Speed changes.** Steps already sealed keep playing evenly from the last
  one played.

What players feel on their own commands is their round trip, plus their own
buffer, plus the wait for the next turn. It is not the room's input delay,
and not anyone else's link: a poor connection only delays its owner. Paced
bots over 150 ms round trips with jitter feel a median of about 220 ms,
whether the input delay is 60 or 1000 ms.

## Game messages from the client

These travel on the control stream.

- **`Intent`**: a player action, carrying a client sequence number and an
  opaque, size-capped payload. The game's actions are encoded in it by the
  action schema (`tpf3mp_proto::action`, described in "The action schema"
  in [BUILDING.md](BUILDING.md)), which has a version of its own.
  - The server validates it: the room is running, the sender is a member,
    rate and size limits hold, and the ruleset accepts it.
  - Accepted: the intent enters the next turn as a `Command` event. The event
    names the player and the client sequence number, so the sender can match
    it.
  - Refused: only the sender gets `IntentRejected` with a reason.
- **`Progress`**: the last step the client executed. It drives pacing.
- **`Checkpoint`**: per-lane digests at every checkpoint step (a room
  setting). The server compares members' digests, as described in
  [ARCHITECTURE.md](ARCHITECTURE.md) under "Authority and data flow".
  - **Deciding.** A round is decided once every member pacing the room has
    reported, or 30 seconds after the first report. It needs at least two
    reports: one report compares against nothing, so a round that has only
    one at its deadline closes without a verdict.
  - **Verdict.** For each lane, a strict majority wins; otherwise the anchor
    wins (the reporter on the most common platform, earliest in join order).
  - **Divergence.** A member that differs from the verdict receives
    `Diverged` with the step and the lanes. So does a member that reports
    after the decision, while the room still keeps that round (the last 64
    decided rounds).
  - **Closed rounds.** A report for a step whose round was closed and
    dropped is ignored, so nobody can reopen old rounds and crowd out new
    ones.

Game messages that arrive when the sender is not in a running room are
ignored; an intent is answered with `IntentRejected(GameNotRunning)`. Such
messages can be in flight when a player leaves, so they are not violations.

## Resuming

A player who reconnects joins the room again with the same identity and a
`Resume`: the last turn it applied and the history that turn belongs to,
which its stream's `TurnStart` named (`TurnFollower::resume_point`). The
server opens a turn stream that starts right after that turn. The new
connection replaces the old one, which is closed with `REPLACED`. A client
checks that the stream continues its log exactly (`TurnFollower::restart`).

A server that can no longer resume a client there (the turns left its
window, or were lost in a crash) answers `ResumeUnavailable`. The client
then joins without a `Resume`, which says it has no world of this game, and
receives the room's world to load. A server that keeps no snapshots sends
the game from its first turn instead.

**Histories.** A room restored after a crash may have lost the last turns
some clients saw, and from then on it numbers different turns the same way.
Each restore therefore begins a new history, recorded in the log. The server
resumes a client only on turns its history shares with the current one; a
client that saw lost turns is told `ResumeUnavailable`, however many turns
the room has sealed since.

## Snapshots

A snapshot is a native world save, stored and sent as deduplicated chunks
(`tpf3mp-snapshot`, described in [SNAPSHOTS.md](SNAPSHOTS.md)). A player who
joins a running game, one who can no longer resume, and one whose world
diverged all receive the room's latest agreed snapshot, then follow the
turns since it. Only a server configured with a snapshot store does this.

**Everyone loads it.** Whenever the room hands a snapshot to a member,
every game plays from the same loaded save afterwards. A member playing
another world reloads it too. One whose game already plays that very
snapshot (it loaded it and has followed the same turns since) needs nothing,
so a newcomer handed the world the others play leaves them undisturbed. A
member that cannot take it at that moment (away, or not reading its turns,
which disconnects it as a slow consumer) owes it, and is handed it when it
next can, even when it comes back resuming its stream. A restart forgets
which snapshot each game loaded, so after one they all load the next one
handed out. A game that keeps its own world
numbers its entities differently from one that loaded a save (Transport
Fever 3 build 40408: saving and loading renumbers some, and the free ids
go back into a first-in, first-out queue in another order), and the
simulation depends on entity ids: a vehicle leaving a depot starts
`id mod 1000` millimetres back along its first edge
(investigation/TF3_VEHICLE_DETERMINISM_2026-09-30.md). Two games that
loaded the same save number every entity alike.

**The world a room starts from** is a snapshot too: one the owner's
client uploaded in the lobby ("The first world" above), which the room
holds as its current snapshot from the start, standing before the first
turn (save event 0, turn 0, step 0).

**Saving.** The room saves every ten minutes of play, sooner when someone
waits for a world, but never twice within a minute, and not at all while
nothing has changed.
1. It orders a `Save` event, sealed as the last event of its own turn,
   which runs no new steps. The save point is therefore a turn boundary: a
   stream from the save starts right after that turn.
2. Every client that plays through the event saves its world, with every
   earlier event applied and before the event's step runs. It cuts the save
   into its chunk store and reports `Saved` with the world's lane digests
   and the snapshot it holds (or none, if saving failed). Only a member
   whose stream carried the save may report it.
3. Once every member pacing the room whose stream carries the save has
   reported, or two minutes have passed, the room judges the lanes as it
   judges a checkpoint's. A lone report stands: that is how a player alone
   in a room hands its world on. Members whose lanes differ are told
   `Diverged`.
4. The room asks a member whose lanes agreed, earliest in join order first,
   to `Upload` its snapshot; members whose uploads failed before are asked
   last. If it does not start within 30 seconds, fails, or runs longer
   than its size allows (30 s plus the size at 128 KiB/s, at most 90
   minutes), the next one is asked, and a transfer still running is cut
   off. A snapshot the room already holds needs no upload.
5. The received snapshot becomes the room's current world. The one before
   stays for downloads that may still run; older ones are released.

**Receiving a world.** The server opens a turn stream whose `Start` names
the world, starting right after the save's turn. The client fetches the
snapshot on a bulk stream, has its game load it, and only then applies the
stream's turns. Until it has caught up, it does not pace the room. A client
that cannot fetch the world joins again without a `Resume` and is offered
the current one.

**Rebasing.** A member found diverged, at a checkpoint or a save, receives
the first world the room agrees on after the divergence, and the room saves
soon to have one; every other member playing receives it with it. A member
is rebased for its own divergence at most once every five minutes; a
replica that keeps diverging is told each time.

**Learned on TPF2 Multiplayer's transfer path.** Its shared-save flow is the
same shape (the host forces a native autosave at a held step, ships it,
everyone loads it and resumes at the votes), and these are what broke in
the field:

- **The received world must have somewhere to go.** A player who had never
  saved a single-player game had no save folder; the transfer verified, the
  placement failed with a path error, and it was reported twice as "the mod
  transfer didn't work". Create the game's save folder before placing a
  world, and log the destination path on failure, not only the source.
- **A world is useless without its mod set, and the mod set does not
  travel.** One player's Workshop set was 556 mods, 93 GB on disk, 19 GB
  zipped; packing runs at about 250 MB/s of mod data. The working shape is
  register what the joiner already has on disk and fetch only what is
  missing, with the registry published after every batch rather than at the
  end of a round (a 359-batch round never reached its end). A folder the
  joiner's game cannot read counts as absent: a mod skipped at the title
  menu is fatal when a shared save forces it to load, at 78% of every load.
- **The world epoch is the credential.** Freezing the roster by session id
  after a resync refused a player whose game had restarted, forever, and
  every broadcast then waited on the dead session until the send window
  blocked the host for everybody. Admit any sender presenting the current
  world epoch, evict after 10 s of silence, and never gate a later join on a
  recovery token that is not cleared when the recovery ends (gate on the
  phase; one resync locked the roster for the life of the process, into a
  brand-new game).
- **Loading looks like quitting.** TPF2 builds the title-menu page on the
  way from the old world to the loading screen, so a "left the world, leave
  the room" rule fired on every resync joiner and the host's barrier failed
  with "player disconnected" at step loading. Any rule keyed on a menu page
  must ask the engine whether a load is in progress first.
- **A home-made reliable UDP transfer topped out at 10 MB/s** (window over
  RTT, one send per 1,350-byte chunk in Python, a whole-window rewind on a
  lost feedback), 5 MB/s at 15% loss, against 2 GB/s for a plain TCP stream
  on the same link; bulk moved to a TCP side channel the same day. The QUIC
  bulk stream above is the right shape; keep the datagrams for turns only.

**Bulk streams.** The client opens one, sends the version preamble and a
`BulkOpen`, and reads the server's preamble:
- `Fetch { snapshot }`: the client fetches a snapshot the room offered to
  this connection. Any other snapshot is answered `Unavailable`, so nobody
  can probe the server's store for other rooms' worlds.
- `Serve { snapshot }`: the client serves a save the room asked it for. The
  server ends a stream it did not ask for without a request.

Then the fetching side asks for the manifest, which must hash to the
snapshot's name, and for chunks in batches of at most 256, keeping up to
16 MiB outstanding. Every chunk must be listed in the manifest, is one zstd
frame, and is checked against its hash before it is stored; the finished
file is checked against the manifest's file hash. A server stores received
chunks compressed by itself, never an uploader's frames. Requests are
capped at 16 KiB per frame and responses at 11 MiB. A side that sends
nothing for a minute is given up on, and every chunk must move at
64 KiB/s for its size (at least 5 s each), so nobody holds a transfer by
trickling.

**Persistence.** Next to each room's log, a pointer file names its current
snapshot and where it stands. A restored room keeps offering that snapshot
if the store still holds it and the recovered log reaches it. At start, the
server releases snapshots no restored room refers to.

## Diagnostics

A client may send lines of its own log to the server, so the server's
operator can see what went wrong for a player from the support code
alone (the session ID, a code like an invite's that the server gives no
two sessions while their diagnostics are kept). TPF2MP's relay kept its
players' diagnostics the same way.

- **What.** `Request::Diagnostics` carries up to 32 `DiagnosticEvent`s: the
  client's time in milliseconds, a level (`Info`, `Warn`, `Error`), where
  it was logged (up to 48 bytes) and the line (up to 1024 bytes). The
  largest request fits a control frame.
- **Redacted on both sides.** The client passes every line through
  `tpf3mp_proto::redact`, and the server does again before keeping it.
  Absolute paths keep only their last part (and not that, when it is
  digits alone, such as a Steam account's folder); IP addresses, secrets
  after keys such as `token=`, `password:` or `invite=`, e-mail addresses
  and Steam IDs are replaced. An invite's code looks like any word, so
  clients never log one but after such a key.
- **Kept per session.** The server appends the lines, with the player's
  ID, to a file named by the session ID, up to 8 MiB a session, and
  answers `Done`. It answers `DiagnosticsNotKept` when it keeps none, or
  this session has sent all it may: the client then stops sending them on
  this connection.
- **Their own budget.** Diagnostics requests take no share of a
  connection's requests: one a second, with a burst of eight, on their
  own. Beyond it, `RateLimited`, and the client sends the lines later.
- **Never in a game's way.** The server hands the lines to a writer of its
  own and does not wait for it; when the writer is behind, it answers
  `RateLimited` and the lines are sent again.
- **The client.** The launcher records its log's lines from `info` up (and
  other libraries' warnings and errors), sends up to four batches every
  five seconds on each connection, and what is left as the connection
  closes; lines waiting when a connection drops go with the next. At most
  2000 lines wait; past that, the oldest go. The player can switch
  diagnostics off, which also forgets the lines waiting.

## Slow and misbehaving clients

- **Bounded buffers.** Outbound queues are bounded per client. A client that
  cannot keep up with its turn stream is disconnected; it never makes the
  server buffer without limit or stall other players.
- **Streams.** A client opens its control stream and at most one bulk
  stream at a time, and sends no datagrams; QUIC flow control refuses
  anything more. The server's receive windows are small: 256 KiB per
  stream, 512 KiB per connection.
- **Per-address limits.** One address, with an IPv6 /64 counting as one,
  holds at most 8 sessions (more are rejected with `TooManyConnections`)
  and has at most 4 handshakes in progress. Once half of the server's
  handshake capacity is in use, new clients must first prove their address
  with a QUIC retry.
- **Idle sessions.** Clients send keep-alives every 5 s and the server sends
  none, so a connection silent for 30 s ends. A session that stays outside
  any room for 10 minutes is closed with `IDLE`, and one whose control
  stream ends is closed with `NORMAL`.
- **Protocol violations** close the connection with `PROTOCOL_VIOLATION`:
  - malformed or oversized frames;
  - a first message that is not `Hello`, or a second `Hello`;
  - progress beyond the sealed frontier;
  - a checkpoint with too many lanes.
- **Rate limits.**
  - Intents, per player: 20 per second with a burst of 40, and 32 KiB of
    payload per second with a burst of 256 KiB. Excess intents are answered
    with `IntentRejected(RateLimited)`.
  - Requests, per connection: 10 per second with a burst of 20, of which
    joins 1 per second with a burst of 5. Excess requests are answered with
    `RateLimited`.
  - Game messages, per connection and per kind: progress reports 200 per
    second with a burst of 400 (excess ones are dropped), intents 40 per
    second with a burst of 80. Checkpoints are not limited here: the room
    ignores reports for closed rounds.
  - Advisory datagrams, per connection: 120 per second with a burst of 240
    (excess ones are dropped).
  - Requests that change nothing, such as setting ready twice, do not send
    everyone the room again.
- **Password guessing.** A room takes 10 wrong passwords per minute from
  players not already seated. Past that, it refuses every newcomer's
  password, right or wrong, for the rest of the minute. Seated members
  rejoining are never held up.
- **Rejected requests** are answered with a typed error and leave the
  connection open.

## Advisory traffic

Advisory traffic uses unreliable QUIC datagrams outside the turn stream. It is
fire-and-forget: datagrams are not sequenced, not acknowledged, not buffered
when queues fill, and do not stall the turn stream or replay on reconnect.
This keeps pointer movements and build previews responsive and fluid, even
when the game is paused.

Datagrams carry `Datagram::Cursor`:
- `player`: authenticated by the server (overwritten with the member's link
  identity so players cannot impersonate each other);
- `at`: optional `Pos2` (ground-plane coordinates in millimetres, `None` when
  the pointer is lifted);
- `building`: boolean indicating whether an active build tool preview is
  shown;
- `label`: optional short tool name or text.

Servers rate-limit incoming datagrams per connection (120/s with a burst of
240) and immediately relay them to all member connections in the room.
