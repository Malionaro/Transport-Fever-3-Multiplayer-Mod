# Playing

How to play Transport Fever 3 together with TPF3-MP. The network side is
ready. The part that runs inside the game waits for the game's release:
this page says so where it applies.

## What you need

- Transport Fever 3, the same build and mods as everyone in your room, in
  the same order. The room compares everyone's before a game starts, and
  tells you exactly which mods to add, remove or update if yours differ.
  Mods that only change what you see (windows, overlays, a minimap: your
  *personal* mods) may differ, when you list your mods with `--mods`: the
  launcher scans each one and leaves those out of the comparison, and the
  room's world loads with your personal mods and without other players'
  ([MODS.md](MODS.md); proposed, D25). `tpf3mp-modscan --installed` says
  which of your mods are personal, and why.
  Without `--mods` the launcher finds your mods itself: choose your
  personal ones in the lobby, and it remembers them; a room you create
  takes its shared mods from its start save, and the lobby shows each
  player which of them they have.
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
launcher loads it into the game it starts, into that game alone, for as
long as it runs (see "The launcher"). Started from Steam,
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

It opens the TPF3-MP window. Keep it open while you play: it holds your
connection to the server, closing it ends your session, and during a game
it asks first. On a system where the window cannot open, the launcher
opens the same launcher as a page in your browser instead (`--browser`
does so on purpose); that page works only on your own machine, in the tab
the launcher opened, and has the whole lobby in it.

The window starts the game and shows where things stand; you play from the
game's own **Multiplayer** button (next section).

- **Start Transport Fever 3**, the big button, starts the game with
  TPF3-MP in it, with Steam running. Only a game started here has the
  Multiplayer button and joins rooms: started from Steam, it is the plain
  game. Press it once; the launcher refuses to start a second game while
  the first still runs. It is also the first step of **How to play: in the
  game** on the left, which ticks the steps off as you go.
- **Server** names the server you play on: by default the project's relay
  (**EU**, in Germany), which nearly everyone uses; its dot is green while
  the server is online. The pill
  at the top of the panel says whether you are connected, in a room's
  lobby, or playing.
- **Your room.** Once you are in a room, the left side shows it: its
  players, whose mods differ, and the **Session log**, which tells you
  what happened, such as your world being replaced by the room's or your
  connection coming back. Its chat and buttons are in the game.
- **While the room's world comes**, the big button follows your game:
  receiving the room's world, loading it, and playing, with the room's
  speed.
- **Your game**, the bar along the bottom, says where Steam has Transport
  Fever 3 and whether the TPF3-MP mod is installed (see "Installing").
  **Settings** has the server, updates and diagnostics (see "Changing the
  server" below); the **support code** at the bottom is what to quote to
  the server's operator.
- If the game closes or crashes once it has connected, the launcher
  notices within a second: the Session log says "the game session failed:
  Transport Fever 3 closed", and you are back on the server, out of the
  room. Join it again with its invite and start the game again.

**Lobby in this window instead**, under the big button, brings the whole
lobby into the launcher, as it was before the game had its Multiplayer
button: connect with your name (and an invite, to join in one step),
create a room with its rules and password, join with an invite, the room's
players with **Remove** for the owner, **Copy invite**, **Ready**,
**Start game**, **Leave room** and the chat. Use it if the game's main menu
has no Multiplayer button, for example after a game update the hook does
not know yet (see "When something does not work"). **Lobby in the game's
menu instead** puts it back.

### Changing the server

TPF3-MP plays on the project's relay (**EU**) unless you choose another
server. *This follows a proposed change to the project's decisions (D12),
which the owner has yet to approve.*

1. Open **Settings** (top right). The **Server** card says which server you
   play on, and whether it is the default.
2. Type the other server's address as `host:port`, such as
   `tpf3mp.example.org:29470`, and press **Use this server** (or Enter).
   Anything else is refused and the card says why.
3. If you were connected, the launcher disconnects and connects to the new
   server under the same name. It remembers the server for next time.
4. **Reset to default** goes back to the relay.

You cannot change the server while in a room: leave it first. An invite
never switches servers: an invite to a room on another server is refused,
so friends who play elsewhere all set the same server here. The browser
page (`--browser`) has the same setting, under **Settings: server**.

## Playing from the game's Multiplayer button

Connecting, rooms, the lobby and chat are in the game. Everything you do
there goes through the launcher, which you keep open, and shows in its
window too.

1. **Start the game from the launcher**: **Start Transport Fever 3**.
2. **Click Multiplayer.** The game's main menu has two TPF3-MP cards to
   the right of its own: **Multiplayer**, which says under its title where
   you are (not connected, online on EU, your room and how many are ready),
   and **Join a friend**. The top bar has a **Multiplayer** button too,
   next to Settings. Each opens the Multiplayer window, one page at a
   time; **Join a friend** opens it on the Join page.
3. **Connect.** The first page: type the name others will see, or keep the
   one the launcher remembers, and press **Connect to EU** (the server is
   the launcher's; there is none to type). Then it offers two big cards:
   **Join a room** and **Host a room**. Each opens its page, and **Back**
   returns to this one. **Server...**, at its bottom, shows the server you play
   on by its name, marked (default) when it is the launcher's own (the
   window never shows a server's address, but in this field); type another
   (`host:port`) and **Use this server**, or **Reset to default**.
   Changing it disconnects you and connects to the new one, and an
   invite only joins rooms on your own server. Not while in a room.
   **Your banner**, next to it, picks the picture the others see on your
   card in a room, from the game's own pictures; **Default** goes back to
   the one chosen for you. The launcher remembers it.
4. **Join a room, or host one.**
   - **Join a room**: the rooms their owners made public, as cards like
     the main menu's, each with the picture of its map's climate, its
     name, players out of its limit, companies and the game's year,
     **Playing** once its game runs, and a lock if it has a password.
     Click one to join it; one with a password asks for it first.
     **Previous**, **Next** and **Refresh** page through the list, which
     also refreshes itself every ten seconds. **Join with code**, at
     the top, opens a small popup for a friend's room: the **invite code**
     they sent you, such as `K7QM2X` (upper or lower case), the room's
     password if it has one, and **Join** or **Cancel**. A private room is
     joined only this way.
   - **Host a room**: a **room name** (your name's room if you leave it
     empty); **Start from this save**, one of your saves, newest first, or
     **None: I load a world myself**; **Players**, 2 to 16; **How you
     play**, two pictures: **Co-op**, everyone for the room's one company,
     or **Competitive**, each player founding a company of their own in
     the game; **Who can find
     it**: **Private**, invite only (the default), or **Public**, in the
     room list, with your save's climate and year; the **Rules**, when the
     server offers more than one (`native` is the game's own rules and
     economy, as in single player; a description says what the others
     are); and an optional **password**. Then **Create room**. You own the
     room: you start its game and can remove players.
   - **Your mods**, at the bottom of both pages and of the room's: the
     mods you have installed. Turn on or off those only you play with
     (only you see them); those every player needs are marked so and stay
     as the room has them. In a room, it also lists the room's own mods,
     from its start save, and whether you have each. You can change your
     choice until the room's game starts.
5. **The room.** On the left, the room's name (a lock if it has a
   password), its **invite code** to send your friends, and its players
   as picture cards of their banners, each marked **Owner**, **You**, **Ready** or **Not ready**, **Away**,
   and **Other mods** when their game differs from the owner's. On the
   right, the room's chat: type and press Enter or **Send**.
6. **Get ready.** At the main menu you are marked ready by yourself: a
   guest at once, the owner once the room has the save picked in step 4.
   **Ready** and **Not ready** set it by hand.
7. **Start.** The owner presses **Start the game** once everyone is
   ready (until then it says it is waiting for everyone). Every player's
   game loads the room's world from the menu and starts it, with no Start
   Game to press. Without a save picked, the owner loads the world to play
   with **Load Game** instead, as in single player, and it becomes the
   room's.
8. **Play.** While the world comes, the window says how far it is
   ("Receiving the room's world: 42% (48.0 MB of 112 MB)", then "Loading
   the room's world..."), and the chat and **Leave room** still work. In
   the game, the Multiplayer window on the game bar has the room (see
   "While you play").

What the window says:

- A line under the page's title says what is under way ("Creating the room...")
  until the launcher answers, then what happened. Anything refused, such
  as a wrong invite, a full room or a name that is too long, shows in red
  there, and the button can be pressed again.
- "Your game differs from the room's" names the mods to add, remove or
  update.
- "This game has no link to the TPF3-MP launcher": the game was not
  started from the launcher. Close it and start it from there.
- **Remove** (the bin, for the owner) and **Leave room** ask first.
  **Disconnect** leaves the server. After a room's game has ended, start
  the game again from the launcher to play the next room's game.

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
  are on the main menu's Multiplayer window (see "Playing from the game's
  Multiplayer button").
- **Reading the room.** Its in-game window keeps room status at the top,
  with Players and Companies side by side. Company cards separate the
  name, balance, debt and members; your company comes first. Scroll the
  company column for its management controls or more companies. Chat has
  its own scrolling history, with the message field always below it.
  Long names and messages wrap rather than widening the window.
- **Speed and pause.** The room's owner sets the room's speed, pause
  included, with the game's own speed buttons, and everyone's game runs at
  it. Guests' speed buttons highlight the room's accepted speed,
  including pause. Their buttons and speed shortcuts are disabled;
  their tooltip and the Multiplayer window say **Host controls speed**.
  Outside a multiplayer game the normal controls return.
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
  same place, or nothing, and says so in the same notification. Greening
  an industry and marketing campaigns are not in multiplayer yet.
- **Company ranks.** Take a new rank in the company window as usual: every
  player's game takes it together, a moment after your click. With one
  company in the room the rank grows as in single player. With more (a
  proposal awaiting the owner, D23), each company's progress is its share
  of each town it serves: the town's people, split by the cargo and
  passengers each company carries for it, times the company's rating
  there. The company window shows your company's own rank and permits.
- **Roads, tracks, stations and depots.** Build them with the game's own
  street, track and construction tools: every player's game builds them
  together, a moment after your click, and your company pays as usual. A
  station or depot placed by a road is joined to it, as in single player.
  Street stops, on one side or both, go on with the stop tool; the stop
  goes where your cursor is on the road. The road tools tab works too:
  tram tracks, bus lanes, noise barriers, trees along the road and the
  lock against the town's changes, and a road built through a stretch
  with stops keeps them. Remove them, and roads and tracks, with the
  bulldozer.
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
  repaints them. The colour button offers the companies' colours first,
  then the game's own.
- **Your company's head, passwords and stations** (proposed, D22). The
  player who founded a company is its head while they play for it; after
  that, whoever has played for it longest. The Multiplayer window shows
  each company's head. The head can give the company a password: then
  others join it only by typing the password next to its Join button.
  The password goes to the server, which keeps it from every game and
  every log; nobody, the head included, can read it back, so share it
  the way you share a room's. The head can also remove or change the
  password, send a player back to the room's first company, and close
  the company's stations to other companies' lines. Stations start open:
  your lines may stop at another company's station, and the line manager
  offers it, until its head closes them. You still cannot change or
  remove another company's station, and your vehicles use your own
  depots. The room's first company is everyone's: it has no head and no
  password. The game's company window renames your company too.
- **Achievements.** A game with TPF3-MP active still earns achievements:
  the mod keeps them on, as the game lets a mod do. This holds even when
  the save has other mods that would switch them off.
- **Not in multiplayer yet.** What the room cannot share with everyone yet
  does not happen in your game either. The game bar says "Not in
  multiplayer yet: …" for what the game's windows do that the room does
  not carry yet. Junction tools (lane arrows, crosswalks and traffic-light
  settings) have an implementation behind `strict_junctions`, off until
  the two-player acceptance check in HOOKS.md passes. With it off, these
  tools, and a tool that would move a stop or signal onto another stretch of road, show "Not in
  multiplayer yet" and build nothing: the tool says why.

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

**No Multiplayer button on the game's main menu?** Only a game started
from the launcher has it, and only when the TPF3-MP mod is installed and
activated (see "Installing") and TPF3-MP knows where the game's menu is
in its build. **Lobby in this window instead** in the launcher does
everything the button's window does, with the game started from the
launcher as before.

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
  and shared mods in the same order; personal mods are not compared. In the room, a **differ** pill next to a
  player shows whose game differs from the owner's; each player sees their
  own list.
- **"too many players are connected from this network"**: the server
  limits connections per network. Close another game, or ask the operator.
- **"that invite is for another server"**: an invite never takes you to
  another server. Ask for an invite to a room on yours, or, if your friends
  play elsewhere, change the server in **Settings** (see "Changing the
  server") and join again.
- **"the invite or password is not valid"**: the room closed, the code
  is mistyped, or the password is wrong.
- **"too many requests; try again in a moment"** when joining: too many
  wrong codes or passwords came from your network in the last 10
  minutes. Wait, then check the code.
