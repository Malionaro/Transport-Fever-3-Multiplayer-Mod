# Playing

How to play Transport Fever 3 together with TPF3-MP. The network side is
ready. The part that runs inside the game waits for the game's release:
this page says so where it applies.

## What you need

- Transport Fever 3, the same build and mods as everyone in your room, in
  the same order. The room compares everyone's before a game starts, and
  tells you exactly which mods to add, remove or update if yours differ.
- The TPF3-MP package for your system, from the project's releases:
  Windows x64, Linux x64 or macOS on Apple silicon. Players on different
  systems can share one room.

You do not need to forward any port or open anything on your router: your
launcher connects out to the server, and everything goes through it.

## Installing

1. Unpack the package anywhere you can write to, such as your Documents
   folder: the launcher updates the files in it (see "Updates").
2. **The mod.** Start Transport Fever 3 once, so Steam makes its folder for
   your mods, and close it again. Then:
   - **Windows:** double-click `INSTALL_TPF3MP.cmd` in the package.
   - **Linux and macOS:** run `./install.sh` from the package's folder.

   The installer is a script, not a program: open `tools\install.ps1`
   (Windows) or `install.sh` (Linux and macOS) to read exactly what it
   changes. It puts the TPF3-MP mod, `tpf3mp_1`, in Steam's folder for
   your Transport Fever 3 mods, `<Steam>/userdata/<account>/3493540/local/staging_area`,
   and notes its version in TPF3-MP's data folder, which the launcher
   shows. Then start the game once, open **Mod Hub**, find TPF3-MP under
   your mods and click **Activate**: a mod that is not activated does
   nothing. To put it in another mods folder, drop that folder onto
   `INSTALL_TPF3MP.cmd`, or run `./install.sh "<the mods folder>"`.

   Nothing goes into the game's own folder, and no launch option is set.
   The installer refuses, and changes nothing, while the game is running.
   A step that fails undoes the ones before it. Nothing is deleted: a
   TPF3-MP mod it replaces or takes out goes to the `backups` folder in
   TPF3-MP's data folder. `UNINSTALL_TPF3MP.cmd` or `./uninstall.sh` takes
   the mod out again.

   Run the installer again after an update of TPF3-MP. Until the game is
   out, packages carry no mod yet, and the installer says so.

The part of TPF3-MP that runs inside the game is not installed at all: the
launcher loads it into the game it starts for your room, into that game
alone, for as long as it runs (see "The launcher"). Started from Steam,
Transport Fever 3 is the plain game, as if TPF3-MP were not there.

## The launcher

Start the launcher from the package:

- **Windows:** `TPF3-MP.exe`. The first time, Windows may say it protected
  your PC from an unknown app: choose **More info**, then **Run anyway**.
- **macOS:** `TPF3-MP.app`. The first time, macOS refuses to open an app
  from an unidentified developer. On macOS 15 and later: try to open it
  once, then in **System Settings**, **Privacy & Security**, choose **Open
  Anyway**. On earlier versions: right-click it, choose **Open**, then
  **Open** again.
- **Linux:** `tpf3mp-launcher`. It needs a desktop with Vulkan or OpenGL
  drivers, as the game does.

It opens the TPF3-MP window. Keep it open while you play: closing it ends
your session, and during a game it asks first. On a system where the
window cannot open, the launcher opens the same launcher as a page in your
browser instead (`--browser` does so on purpose); that page works only on
your own machine, in the tab the launcher opened.

1. **Connect.** Enter the name others will see, then **Connect**; the
   launcher remembers it for next time. There is no server to type:
   TPF3-MP plays on the project's server, which the Server panel names
   (**EU**, in Germany), and on no other. Its dot is green while the
   server is online. Got an invite code? Type it into **Invite** as well:
   you are connected and in the room in one step.
2. **Rooms.** Either create a room, with an optional password, or type
   the invite code someone sent you, such as `K7QM2X`, and **Join
   room**. Upper or lower case, both work. When the server offers more
   than one set of rules, the host picks one when creating the room:
   `native` is the game's own rules and economy, as in single player;
   others are run by the server, which checks everyone's money and
   actions. The room's title shows its rules, and they cannot change once
   the room exists.
3. **Invite.** Your room shows its **invite code**, six letters and
   digits. Send it to your friends, for example on Discord (**Copy
   invite** copies it), or read it out. Anyone with the code (and the
   password, if you set one) can join; keep it within your group.
4. **Start the game.** In your room, press **Start Transport Fever 3** in
   the Game part, with Steam running. The launcher starts the game with
   TPF3-MP in it, for this room; once the game has loaded, the Game part
   says it is connected. Only a game started here joins the room: started
   from Steam, it is the plain game. Press it once; the launcher refuses to
   start a second game while the first still runs. If the game closes or
   crashes once it has connected, the launcher notices within a second:
   the Session log says "the game session failed: Transport Fever 3
   closed", and you are back on the server, out of the room. Join it again
   with its invite and start the game again. A game that closes before it
   connected leaves you in the room; just start it again.
   You can also start the game before you are in a room (**Start
   Transport Fever 3** under the main button) and do the rest from the
   game's main menu: see "The Multiplayer menu in the game".
5. **Wait at the menu, or load your save.** A guest just waits at the
   game's main menu: once the game is there, the launcher marks you
   **ready** by itself, and when the room starts, your game loads the
   room's world from the menu and starts it, with no **Start Game** to
   press. The room's owner loads the save everyone will play; once its
   world is up, the launcher marks the owner ready. (A guest who loads a
   world instead is marked ready too, and the room's world replaces it.)
   Nobody has to press **Ready**: the button stays, to get ready by hand,
   and **Not ready** keeps you not ready until you come back to the menu
   or load another world. When everyone is ready, the room's owner presses
   **Start game**. Everyone's game starts from the owner's world.

The window is tearded's TPF2 multiplayer launcher, for Transport Fever 3.
On the left, under the game's name, a checklist ticks these steps off as
you go: connect to a server, create or join a room, start the game from
here, everyone ready, play together; below it, the release notes. In a
room, the left side shows the room: its players, whose mods differ, the
chat. The step at hand is on the right, on the big button, which also
follows your game: started from here, receiving the room's world,
loading it, and playing. The bar along the bottom, **Your
game**, says where Steam has Transport Fever 3 and whether the TPF3-MP mod
is installed (see "Installing"). **Chat**
reaches everyone in the room. A message **From the server** is its
operator's, such as a restart coming: when the server comes back, the
launcher rejoins by itself. The **Session log** tells you what happened,
such as your world being replaced by the room's, or your connection coming
back.

## The Multiplayer menu in the game

Connecting, rooms, the lobby and chat are in the game too. The launcher
still starts the game and holds the connection, so keep it open; everything
you do in the game shows in the launcher's window as well, and the other
way round.

1. **Start the game from the launcher**, in a room or not: **Start
   Transport Fever 3**. Started from Steam, the game has no Multiplayer
   entry.
2. **Open the Multiplayer window.** On the game's main menu, click
   **Multiplayer**: the card in the top row, or the button in the top bar.
   If the window says the game has no link to the launcher, the game was
   not started from the launcher: close it and start it from there.
3. **Connect.** Enter the name others will see and press **Connect**. The
   server is the launcher's (**EU**); there is none to type.
4. **Create or join.** Create a room (a name, and a password if you want
   one), or type the invite code a friend sent you, such as `K7QM2X`, and
   **Join**.
5. **The room.** The window shows the room's invite code, its players (the
   crown marks the host, the tick who is ready) and, on the right, the
   room's chat, where you can write to everyone. A guest waiting at the
   main menu is marked ready by itself; the owner is marked ready once
   their world is up. **Ready** is still there to press by hand. The owner
   presses **Start** once everyone is ready.
6. **Play.** The room's game starts from the owner's world: the owner
   loads it with **Load Game**, as in single player, and it becomes the
   room's. Everyone else stays at the main menu: the game loads the room's
   world from there by itself and starts it. (A guest who loads a save of
   their own instead is fine too: the room's world replaces it.)

**Leave** gives up your seat; **Disconnect** leaves the server. After a
room's game has ended, start the game again from the launcher to play the
next room's game.

## Updates

The launcher checks for a new version when it starts and every few hours,
and downloads it in the background. When it is ready, the badge at the top
says so: **Settings**, then **Restart and update**, installs it and
restarts the launcher. During a game it waits: the update installs the
next time you start TPF3-MP.

The launcher installs only what the TPF3-MP project signed: a download
whose signature, version or contents do not check out is refused, and an
install that fails or is cut short puts the old files back, at once or at
the next start. The old version's files stay until the new version has
opened its window; if it fails to three times running, the launcher goes
back to the version before and does not install that one again. Updates
go into the package's folder, so unpack it where you can write, not into a
protected folder such as Program Files.

## While you play

- **The Multiplayer window.** In the room's game the game bar shows the
  room in one line: its name, how many players are in the game, its speed,
  and new chat. Click it, or the Multiplayer button among the mods'
  buttons, for the Multiplayer window: the room's players (the host, you,
  anyone away), its speed, whether your world matches the room's, and the
  room's newest chat, where you can write to everyone. Rooms and invites
  are in the launcher and on the main menu's Multiplayer window (see "The
  Multiplayer menu in the game").
- **Speed and pause.** The room's owner sets the room's speed, pause
  included, with the game's own speed buttons, and everyone's game runs at
  it. Anyone else's speed buttons do not change the room's speed: the
  launcher says so, and the game keeps the room's pace whatever the
  buttons show.
- **Joining later.** You can join a game that is already running: the
  room sends you its world, and your game loads it and catches up.
- **Losing the connection.** If your connection or the server drops, the
  launcher rejoins the room by itself, and your game only pauses. If you
  were away too long to catch up, the room sends you its world again.
- **Leaving.** **Leave room** gives up your seat. The owner can also remove
  a player whose game froze; a removed player cannot come back to that
  room. If the room's game had not begun yet, your game keeps running and
  follows you into the next room you create or join: no need to restart
  it.
- **Your world disagrees.** Every few seconds everyone's game compares the
  world with the room's. If yours has drifted, the room sends you its world
  and your game reloads it. A notice says so.
- **Saving.** The room saves everyone's game together from time to time,
  which you notice as a short pause, like an autosave.
- **Loans.** Take and pay back loans in the company window as usual: every
  player's game books them together.
- **Prospecting.** Prospect for resources near a town from the
  construction menu as usual: every player's game starts the prospection
  together, a moment after your click, and uses your company's permit.
  When it ends, months later, every game finds the same industry at the
  same place, or nothing, and says so in the same notification. Taking a
  new company rank, greening an industry and marketing campaigns are not
  in multiplayer yet.
- **Roads, tracks, stations and depots.** Build them with the game's own
  street, track and construction tools: every player's game builds them
  together, a moment after your click, and your company pays as usual. A
  station or depot placed by a road is joined to it, as in single player.
  Street stops, on one side or both, go on with the stop tool; the stop
  goes where your cursor is on the road. Remove them, and roads and
  tracks, with the bulldozer.
- **Vehicles and lines.** Buy vehicles in a depot's store, make and change
  lines in the line manager, and send vehicles out, stop them or sell
  them, as usual: every player's game does it together, and your window
  hears it a moment after your click.
- **Companies.** Everyone starts in the save's own company, together. In
  the Multiplayer window you can found a company of your own, join
  another, rename or recolour yours, and dissolve it once you are its last
  player and it owns nothing; any split works, two players in one company
  and one in another included. What you build and buy is your company's
  and paid by it, and what another company owns (its vehicles, lines,
  depots, stations and roads) is theirs: you cannot change or remove it.
  The game's own windows show your company: its money in the corner, and
  your things as yours. A new company starts with no money: borrow on the
  terms the game offers in the Multiplayer window, which also shows its
  loans and pays them back (the game's finance window keeps the room's
  first company's loans). With more than one company, vehicles and their
  markers on the map wear their company's colour, and a new colour
  repaints them.
- **Achievements.** A game with TPF3-MP active still earns achievements:
  the mod keeps them on, as the game lets a mod do. This holds even when
  the save has other mods that would switch them off.
- **Not in multiplayer yet.** What the room cannot share with everyone yet
  does not happen in your game either. The game bar says "Not in
  multiplayer yet: …" for what the game's windows do that the room does
  not carry yet. The upgrade, bus lane and tram track tools show
  "Not in multiplayer yet: building with this tool" and build nothing, and
  so does a road or track that would move a stop or signal, or a bulldozer
  click on a stop or signal: the tool says why.

## Playtesting before the game is out

Until Transport Fever 3 is released, `tpf3mp-fakegame` in the package
stands in for it: a small toy game behind the same step gate, which builds
track, saves and loads worlds. Everything but TPF3 itself can be tried,
across PCs and systems:

1. Start TPF3-MP as above.
2. Start `tpf3mp-fakegame` from the same folder (from a terminal on Linux
   and macOS). The window's **Game** part says the game is connected.
3. Connect, create or join a room, get ready and start, as in a real game.
   The fake game has no save to load, so it does not mark you ready:
   press **Ready**.
   The fake game plays by itself: watch the **Game** part count steps, and
   try chatting, leaving and rejoining, and joining a game already running.

Run one fake game next to each launcher. It stops when its room's game
ends.

Developers who want a whole room on one PC, several fake games each with
its own agent, use the multiplayer rig instead (`tpf3mp-rig`, see
[DEVELOPMENT.md](DEVELOPMENT.md)).

## Your identity

The launcher creates a key for you on first use, in your user data folder
(`TPF3-MP/identity.key`). It is what makes you the same player next time,
so you can come back to your seat. There are no accounts or passwords. Keep
the file private, and copy it if you move to another computer.

Other players never see your IP address: everything goes through the
server.

## When something does not work

The bottom of the window shows your **support code**, six letters and
digits like an invite's (**Copy** copies it). It names your connection in
the server's log: quote it to the server's operator with your report. It
lets nobody into your room, so it is safe to post. There is nothing to
send: while you are connected, the launcher sends its log to the server
by itself (see "Diagnostics"), so the operator finds what happened to you
from your support code alone. The launcher also keeps its log on your machine
(`TPF3-MP/logs` in your user data folder, one file a day, a week kept).

### Diagnostics

While you are connected, the launcher sends the lines of its log to the
server you play on, so its operator can see what went wrong for you from
your support code, without asking you for files. Before a line leaves your
machine, paths are cut to their last part (so your user name and your
Steam account are not in them), and IP addresses, invites, keys and
passwords, e-mail addresses and Steam IDs are taken out; the server does
the same again. Your game's own log and crash dumps are not sent. The
server keeps the lines for a limited time, 30 days unless its operator
chose otherwise.

Set **Send diagnostics** to **Off**, in **Settings** (or untick it at the
bottom of the browser page), to stop: the
launcher then sends nothing more, forgets the lines it had not sent yet,
and remembers your choice.

### The game's own logs

The game's own log and crash dumps are not sent. When the operator needs
them, `tpf3mp-agent collect-logs`, run from the TPF3-MP folder, puts them
into one zip with TPF3-MP's logs: `tpf3mp-logs-<time>.zip` in your
Downloads folder (in `TPF3-MP` when there is no Downloads folder).
`--out <folder>` puts it elsewhere, `--since 2h` takes a shorter window,
and `--game-log <file>` adds a log kept elsewhere. Attach the zip to your
report, with your support code.

The zip holds:

- `tpf3mp/logs/`: the launcher's logs, which record its crashes too;
- `tpf3mp/hook.log`: the in-game hook's log;
- `game/…`: the game's own log (`stdout.txt`) and crash reports, from
  `<Steam>/userdata/<account>/3493540/local/crash_dump/`, where Transport
  Fever 3 writes them on Windows. It also looks in `local/stdout.txt`,
  where Transport Fever 2 kept its log, marked "TPF2 location, not seen on
  TF3 Windows", until Linux and macOS are checked;
- `manifest.txt`: the versions of TPF3-MP, its protocol and its link to the
  game, your system, your support code when connected, every file with its
  size, and which places were not found.

Only files changed in the last week are taken, newest first, up to 64 MB;
a long text log that does not fit whole keeps its end. What was left out
is listed in the manifest.

The zip never holds your identity key, invite keys, certificates or
tokens: only the logs folders above are read, and any file there whose name
looks like a key, certificate or token is withheld all the same. Your saved
worlds and remembered server are not included, and TPF3-MP's logs hold no
IP addresses. The game's own logs are the game's: look through the zip
before sharing it publicly if you want to be sure.

- **"the server is out of reach over UDP (...) and through wss://..."**:
  your network blocks both routes, or the server is down. Some school,
  office and hotel networks block the UDP the game uses; the launcher then
  tries a WebSocket connection on port 443 by itself. If the server's
  operator gave you a tunnel address, start the launcher with
  `--tunnel <address>`; if your network never passes UDP, add
  `--tunnel-only`.
- **"connected via tunnel"** next to the connection: your network blocks
  UDP, and the game plays through the tunnel. It works, but lost packets
  cost a little more delay.
- **A version mismatch**: your package and the server are different
  versions. The message says which side is older.
- **"Your game differs from the room's"**: the window lists what to change:
  the game build, the mods you lack, the mods the room does not run, and
  the mods you have in another version. Everyone needs the owner's build
  and mods in the same order. In the room, a **differ** pill next to a
  player shows whose game differs from the owner's; each player sees their
  own list.
- **"too many players are connected from this network"**: the server
  limits connections per network. Close another game, or ask the operator.
- **"that invite is for another server"**: TPF3-MP plays on its own
  server alone. Ask for an invite to a room there.
- **"the invite or password is not valid"**: the room closed, the code
  is mistyped, or the password is wrong.
- **"too many requests; try again in a moment"** when joining: too many
  wrong codes or passwords came from your network in the last 10
  minutes. Wait, then check the code.
