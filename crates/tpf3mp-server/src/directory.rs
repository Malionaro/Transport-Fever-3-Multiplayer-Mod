//! The server's rooms: creation, recovery, lookup and removal.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use ring::hmac;
use tokio::{sync::mpsc, task::JoinHandle};
use tpf3mp_proto::{
    BoundedVec, Code, CreateRoom, FixedBytes, Invite, ListedRoom, MAX_ROOM_MEMBERS, ROOMS_PER_PAGE,
    RequestError, RoomId, RoomPage, RoomPhase, RoomView, RulesOffer,
};
use tracing::{info, warn};

use crate::{
    admission::RoomShare,
    metrics,
    room::{
        NewMember, ROOM_QUEUE, Room, RoomEnv, RoomHandle, RoomSecrets, RoomSpec, SharedSummary,
    },
    ruleset::RulesMenu,
};

/// How long a shutdown waits for each room to finish.
const ROOM_SHUTDOWN: Duration = Duration::from_secs(5);

pub(crate) struct Directory {
    rooms: Mutex<Rooms>,
    max_rooms: usize,
    key: hmac::Key,
    rules: RulesMenu,
    env: RoomEnv,
}

/// The rooms, by their IDs and by their invites' tags.
#[derive(Default)]
struct Rooms {
    by_id: HashMap<RoomId, Registered>,
    by_invite: HashMap<Vec<u8>, RoomId>,
}

impl Rooms {
    fn insert(&mut self, id: RoomId, registered: Registered) {
        self.by_invite.insert(registered.invite_tag.clone(), id);
        self.by_id.insert(id, registered);
    }
}

/// A room the directory knows: the way to reach it, its task, the tag of
/// its invite, and what the room list shows of it.
struct Registered {
    handle: RoomHandle,
    task: JoinHandle<()>,
    invite_tag: Vec<u8>,
    summary: SharedSummary,
    /// A public room's invite, which the list gives anyone; `None` for a
    /// private room, whose invite the server keeps only as its tag. Kept in
    /// memory only: a room restored after a restart is private.
    invite: Option<Invite>,
}

pub(crate) struct DirectoryConfig {
    pub(crate) secret: [u8; 32],
    pub(crate) max_rooms: usize,
    pub(crate) rules: RulesMenu,
    pub(crate) env: RoomEnv,
}

impl Directory {
    pub(crate) fn new(config: DirectoryConfig) -> Self {
        Self {
            rooms: Mutex::default(),
            max_rooms: config.max_rooms,
            key: hmac::Key::new(hmac::HMAC_SHA256, &config.secret),
            rules: config.rules,
            env: config.env,
        }
    }

    /// Restores every running room logged in the data directory. A log that
    /// cannot be recovered is renamed to `*.broken`, unmodified, and kept
    /// for diagnosis, never deleted. Returns how many rooms were restored.
    pub(crate) fn recover(self: &Arc<Self>) -> usize {
        let mut held = Vec::new();
        let restored = self.recover_rooms(&mut held);
        // Snapshots of rooms that are gone would otherwise stay forever.
        if let Some(snapshots) = &self.env.snapshots {
            snapshots.release_all_but(&held);
        }
        restored
    }

    /// Restores the logged rooms, noting the snapshots they hold.
    fn recover_rooms(self: &Arc<Self>, held: &mut Vec<tpf3mp_snapshot::ManifestId>) -> usize {
        let Some(dir) = &self.env.data_dir else {
            return 0;
        };
        let Ok(entries) = fs::read_dir(dir) else {
            return 0;
        };
        // Only regular files: a planted link must not lead recovery, which
        // may cut a torn record, to some other file.
        let files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.path())
            .collect();
        let has_extension =
            |path: &PathBuf, wanted: &str| path.extension().is_some_and(|ext| ext == wanted);
        for partial in files
            .iter()
            .filter(|path| has_extension(path, "compacting"))
        {
            // A compaction the last process did not finish; the log it
            // would have replaced is whole.
            let _ = fs::remove_file(partial);
        }
        let mut paths: Vec<PathBuf> = files
            .into_iter()
            .filter(|path| has_extension(path, "log"))
            .collect();
        paths.sort();
        let mut restored = 0;
        for path in paths {
            let recovered = Room::recover(&path, self.key.clone(), &self.rules, self.env.clone());
            match recovered {
                Ok(Some(room)) => {
                    info!(room = %room.id(), "restored a running room from its log");
                    held.extend(room.held_snapshots());
                    self.register(room);
                    restored += 1;
                }
                Ok(None) => {}
                Err(error) => {
                    warn!(path = %path.display(), %error, "cannot restore a room; keeping its log aside");
                    set_aside(&path);
                }
            }
        }
        restored
    }

    /// The rules hosts pick from.
    pub(crate) fn offers(&self) -> Vec<RulesOffer> {
        self.rules.offers()
    }

    /// Creates a room with `owner` as its first member and starts its task.
    /// The room holds `share` until it closes.
    pub(crate) fn create(
        self: &Arc<Self>,
        owner: NewMember,
        request: CreateRoom,
        share: RoomShare,
    ) -> Result<(RoomHandle, Invite, RoomView), RequestError> {
        if !request.settings.is_valid() || !(1..=MAX_ROOM_MEMBERS).contains(&request.max_players) {
            return Err(RequestError::InvalidSettings);
        }
        let rules = self
            .rules
            .find(request.rules.as_ref().map(|name| name.as_str()))
            .ok_or(RequestError::UnknownRules)?;
        let mut rooms = self.rooms.lock().unwrap_or_else(PoisonError::into_inner);
        if rooms.by_id.len() >= self.max_rooms {
            return Err(RequestError::TooManyRooms);
        }
        let id = loop {
            let id = RoomId(FixedBytes(random()));
            if !rooms.by_id.contains_key(&id) {
                break id;
            }
        };
        // No two open rooms share an invite.
        let (invite, invite_tag) = loop {
            let invite = Invite(Code::random());
            let tag = self.invite_tag(&invite);
            if !rooms.by_invite.contains_key(&tag) {
                break (invite, tag);
            }
        };
        let secrets = RoomSecrets {
            key: self.key.clone(),
            invite_tag: invite_tag.clone(),
            password_tag: request.password.as_ref().map(|password| {
                hmac::sign(&self.key, &RoomSecrets::password_input(&id, password))
                    .as_ref()
                    .to_vec()
            }),
        };
        let room = Room::new(
            RoomSpec {
                id,
                name: request.name,
                max_players: request.max_players,
                settings: request.settings,
                secrets,
                rules: rules.name.clone(),
                ruleset: (rules.factory)(),
                env: self.env.clone(),
                share,
                listing: request.listing.clone(),
                competitive: request.competitive,
            },
            owner,
        );
        let view = room.view();
        let summary = room.summary();
        let (commands, receiver) = mpsc::channel(ROOM_QUEUE);
        let handle = RoomHandle::new(commands);
        let task = tokio::spawn(room.run(receiver, Arc::clone(self)));
        rooms.insert(
            id,
            Registered {
                handle: handle.clone(),
                task,
                invite_tag,
                summary,
                invite: request.listing.is_some().then_some(invite),
            },
        );
        drop(rooms);
        metrics::increment(&self.env.metrics.rooms_created);
        Ok((handle, invite, view))
    }

    fn register(self: &Arc<Self>, room: Room) {
        let (commands, receiver) = mpsc::channel(ROOM_QUEUE);
        let id = room.id();
        let invite_tag = room.invite_tag();
        let summary = room.summary();
        let task = tokio::spawn(room.run(receiver, Arc::clone(self)));
        self.rooms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                id,
                Registered {
                    handle: RoomHandle::new(commands),
                    task,
                    invite_tag,
                    summary,
                    invite: None,
                },
            );
    }

    /// The tag an invite's room is found by: an HMAC under the server's
    /// key, so a room's log does not give its invite away.
    fn invite_tag(&self, invite: &Invite) -> Vec<u8> {
        hmac::sign(&self.key, &RoomSecrets::invite_input(invite))
            .as_ref()
            .to_vec()
    }

    /// The room `invite` is for, if it is open.
    pub(crate) fn find(&self, invite: &Invite) -> Option<RoomHandle> {
        let tag = self.invite_tag(invite);
        let rooms = self.rooms.lock().unwrap_or_else(PoisonError::into_inner);
        let id = rooms.by_invite.get(&tag)?;
        rooms
            .by_id
            .get(id)
            .map(|registered| registered.handle.clone())
    }

    /// Stops every room once its connections are gone: without the
    /// directory's handles, a room's queue closes and its task ends, closing
    /// its log and letting go of the snapshot store. Waits for each a while.
    pub(crate) async fn shut_down(&self) {
        let registered: Vec<Registered> = {
            let mut rooms = self.rooms.lock().unwrap_or_else(PoisonError::into_inner);
            rooms.by_invite.clear();
            rooms
                .by_id
                .drain()
                .map(|(_, registered)| registered)
                .collect()
        };
        for Registered { handle, task, .. } in registered {
            drop(handle);
            if tokio::time::timeout(ROOM_SHUTDOWN, task).await.is_err() {
                warn!("a room did not stop in time");
            }
        }
    }

    /// Page `page` of the public rooms, [`ROOMS_PER_PAGE`] a page: those in
    /// their lobby first, then the fuller, then by name. A private room is
    /// never in it.
    pub(crate) fn list(&self, page: u16) -> RoomPage {
        let mut listed: Vec<ListedRoom> = {
            let rooms = self.rooms.lock().unwrap_or_else(PoisonError::into_inner);
            rooms
                .by_id
                .values()
                .filter_map(|registered| {
                    let invite = registered.invite?;
                    let summary = registered
                        .summary
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    let listing = summary.listing.clone()?;
                    Some(ListedRoom {
                        invite,
                        name: summary.name.clone(),
                        rules: summary.rules.clone(),
                        players: summary.players,
                        max_players: summary.max_players,
                        has_password: summary.has_password,
                        phase: summary.phase,
                        listing,
                        competitive: summary.competitive,
                    })
                })
                .collect()
        };
        listed.sort_by(|a, b| {
            (a.phase != RoomPhase::Lobby)
                .cmp(&(b.phase != RoomPhase::Lobby))
                .then(b.players.cmp(&a.players))
                .then_with(|| a.name.as_str().cmp(b.name.as_str()))
                .then_with(|| a.invite.0.cmp(&b.invite.0))
        });
        let start = usize::from(page).saturating_mul(ROOMS_PER_PAGE);
        let more = listed.len() > start.saturating_add(ROOMS_PER_PAGE);
        let rooms: Vec<ListedRoom> = listed
            .into_iter()
            .skip(start)
            .take(ROOMS_PER_PAGE)
            .collect();
        RoomPage {
            page,
            rooms: BoundedVec::new(rooms).unwrap_or_default(),
            more,
        }
    }

    pub(crate) fn remove(&self, id: &RoomId) {
        let mut rooms = self.rooms.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(registered) = rooms.by_id.remove(id) {
            rooms.by_invite.remove(&registered.invite_tag);
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.rooms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .by_id
            .len()
    }
}

/// Renames a log that cannot be restored to `<room>.broken`, or
/// `<room>.<n>.broken` if that exists, so an earlier one is never replaced.
fn set_aside(path: &Path) {
    for attempt in 0..1000 {
        let aside = if attempt == 0 {
            path.with_extension("broken")
        } else {
            path.with_extension(format!("{attempt}.broken"))
        };
        if aside.exists() {
            continue;
        }
        if let Err(error) = fs::rename(path, &aside) {
            warn!(path = %path.display(), %error, "cannot set a broken log aside");
        }
        return;
    }
    warn!(path = %path.display(), "too many broken logs of one room; leaving this one in place");
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).expect("the operating system's random source is available");
    bytes
}
