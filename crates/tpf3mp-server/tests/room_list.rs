//! The server's list of public rooms (`Request::ListRooms`, PROTOCOL.md
//! "The room list"): only rooms their owner made public, a bounded page at
//! a time, kept current, described by their owner alone, and rate-limited.

#![allow(clippy::unwrap_used)]

mod common;

use std::time::{Duration, Instant};

use common::{FAST, RunningServer, join, room};
use tpf3mp_agent::ClientError;
use tpf3mp_proto::{
    CreateRoom, ROOMS_PER_PAGE, Request, RequestError, RoomListing, RoomPage, RoomPhase, Text,
};

fn listing(map: &str, year: u16) -> RoomListing {
    RoomListing {
        map: Text::new(map).unwrap(),
        year,
        companies: 1,
    }
}

fn public(name: &str, map: &str, year: u16) -> CreateRoom {
    CreateRoom {
        listing: Some(listing(map, year)),
        competitive: false,
        ..room(name, FAST)
    }
}

/// Asks for a page until `done` holds for it, as the list follows the rooms
/// a moment later than their members do.
async fn page_until(
    client: &tpf3mp_agent::Client,
    page: u16,
    done: impl Fn(&RoomPage) -> bool,
) -> RoomPage {
    let deadline = Instant::now() + common::WAIT;
    loop {
        match client.list_rooms(page).await {
            Ok(found) if done(&found) => return found,
            Ok(_) | Err(ClientError::Refused(RequestError::RateLimited)) => {}
            Err(error) => panic!("listing failed: {error}"),
        }
        assert!(Instant::now() < deadline, "the list never showed it");
        tokio::time::sleep(Duration::from_millis(1100)).await;
    }
}

#[tokio::test]
async fn a_public_room_is_listed_with_what_its_owner_declared_and_a_private_one_never() {
    let server = RunningServer::start(|_| {}).await;
    let ann = server.client("ann").await;
    let bob = server.client("bob").await;
    let cat = server.client("cat").await;
    let (secret_invite, _) = bob.client.create_room(room("private", FAST)).await.unwrap();
    let (invite, _) = ann
        .client
        .create_room(public("Open alps", "temperate", 1850))
        .await
        .unwrap();

    let page = cat.client.list_rooms(0).await.unwrap();
    assert_eq!(page.rooms.len(), 1, "the private room is not listed");
    let listed = &page.rooms[0];
    assert_eq!(listed.name.as_str(), "Open alps");
    assert_eq!(listed.invite, invite);
    assert_ne!(listed.invite, secret_invite);
    assert_eq!(listed.listing, listing("temperate", 1850));
    assert_eq!(listed.phase, RoomPhase::Lobby);
    assert_eq!(listed.players, 1);
    assert_eq!(listed.max_players, room("x", FAST).max_players);
    assert!(!page.more);

    // The list's invite joins, and the list follows the room.
    cat.client.join_room(join(&listed.invite)).await.unwrap();
    page_until(&bob.client, 0, |page| {
        page.rooms.first().is_some_and(|r| r.players == 2)
    })
    .await;

    // Only the owner describes it; the private room cannot be.
    assert_eq!(
        cat.client
            .request(Request::DescribeRoom(listing("dry", 1900)))
            .await
            .unwrap_err(),
        ClientError::Refused(RequestError::NotOwner)
    );
    assert_eq!(
        bob.client
            .request(Request::DescribeRoom(listing("dry", 1900)))
            .await
            .unwrap_err(),
        ClientError::Refused(RequestError::NotListed)
    );
    let now = RoomListing {
        companies: 3,
        ..listing("temperate", 1873)
    };
    ann.client
        .request(Request::DescribeRoom(now.clone()))
        .await
        .unwrap();
    page_until(&cat.client, 0, |page| {
        page.rooms.first().is_some_and(|r| r.listing == now)
    })
    .await;

    // Gone when its last member leaves.
    cat.client.leave_room().await.unwrap();
    ann.client.leave_room().await.unwrap();
    page_until(&bob.client, 0, |page| page.rooms.is_empty()).await;
    server.shut_down().await;
}

#[tokio::test]
async fn the_list_comes_a_bounded_page_at_a_time_and_is_rate_limited() {
    let server = RunningServer::start(|_| {}).await;
    let rooms = ROOMS_PER_PAGE + 3;
    let mut owners = Vec::new();
    for n in 0..rooms {
        let owner = server.client(&format!("p{n}")).await;
        owner
            .client
            .create_room(public(&format!("room {n:02}"), "dry", 1900))
            .await
            .unwrap();
        owners.push(owner);
    }
    let reader = server.client("reader").await;
    let first = reader.client.list_rooms(0).await.unwrap();
    assert_eq!(first.rooms.len(), ROOMS_PER_PAGE);
    assert!(first.more);
    let second = reader.client.list_rooms(1).await.unwrap();
    assert_eq!(second.rooms.len(), 3);
    assert!(!second.more);
    let far = reader.client.list_rooms(u16::MAX).await.unwrap();
    assert!(far.rooms.is_empty() && !far.more);
    // Every room once, by name within equal counts.
    let mut names: Vec<String> = first
        .rooms
        .iter()
        .chain(second.rooms.iter())
        .map(|room| room.name.as_str().to_owned())
        .collect();
    let listed = names.clone();
    names.sort();
    assert_eq!(listed, names);
    names.dedup();
    assert_eq!(names.len(), rooms);

    // Paging faster than a page a second, past a burst, is refused.
    let mut refused = false;
    for _ in 0..10 {
        if reader.client.list_rooms(0).await.unwrap_err_or_page() {
            refused = true;
            break;
        }
    }
    assert!(refused, "the list is rate-limited");
    server.shut_down().await;
}

trait RateLimited {
    fn unwrap_err_or_page(self) -> bool;
}

impl RateLimited for Result<RoomPage, ClientError> {
    /// Whether it was refused as too fast; any other error fails the test.
    fn unwrap_err_or_page(self) -> bool {
        match self {
            Ok(_) => false,
            Err(ClientError::Refused(RequestError::RateLimited)) => true,
            Err(error) => panic!("{error}"),
        }
    }
}

#[tokio::test]
async fn a_rooms_play_style_is_carried_to_its_members_and_the_list() {
    let server = RunningServer::start(|_| {}).await;
    let ann = server.client("ann").await;
    let bob = server.client("bob").await;
    let (invite, created) = ann
        .client
        .create_room(CreateRoom {
            competitive: true,
            ..public("Race", "dry", 1900)
        })
        .await
        .unwrap();
    assert!(created.competitive);
    assert!(
        bob.client
            .join_room(join(&invite))
            .await
            .unwrap()
            .competitive
    );
    let page = server
        .client("cat")
        .await
        .client
        .list_rooms(0)
        .await
        .unwrap();
    assert!(page.rooms[0].competitive);
    // A room is co-op unless its owner says otherwise.
    let (_, plain) = server
        .client("dan")
        .await
        .client
        .create_room(room("plain", FAST))
        .await
        .unwrap();
    assert!(!plain.competitive);
    server.shut_down().await;
}
