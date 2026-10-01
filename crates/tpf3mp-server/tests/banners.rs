//! Players' banners (`Request::SetBanner`, PROTOCOL.md "Rooms"): one of
//! the fixed set or none, kept for the connection, shown in the room's
//! member list to everyone, and an unknown one refused.

#![allow(clippy::unwrap_used)]

mod common;

use common::{FAST, RunningServer, join, room};
use tpf3mp_agent::ClientError;
use tpf3mp_proto::{BANNERS, Request, RequestError, Text};

fn banner(id: &str) -> Request {
    Request::SetBanner(Some(Text::new(id).unwrap()))
}

#[tokio::test]
async fn a_banner_is_one_of_the_set_and_everyone_in_the_room_sees_it() {
    let server = RunningServer::start(|_| {}).await;
    let mut ann = server.client("ann").await;
    let mut bob = server.client("bob").await;
    // Unknown ids are refused, before and in a room.
    assert_eq!(
        ann.client.request(banner("rick-roll")).await.unwrap_err(),
        ClientError::Refused(RequestError::UnknownBanner)
    );
    // Picked outside a room, it goes with the player into one.
    ann.client.request(banner(BANNERS[2])).await.unwrap();
    let (invite, created) = ann.client.create_room(room("banners", FAST)).await.unwrap();
    assert_eq!(
        created.members[0].banner.as_ref().map(Text::as_str),
        Some(BANNERS[2])
    );
    let joined = bob.client.join_room(join(&invite)).await.unwrap();
    let ann_there = joined
        .members
        .iter()
        .find(|member| member.player == ann.client.player())
        .unwrap();
    assert_eq!(
        ann_there.banner.as_ref().map(Text::as_str),
        Some(BANNERS[2])
    );
    assert!(
        joined
            .members
            .iter()
            .find(|member| member.player == bob.client.player())
            .unwrap()
            .banner
            .is_none(),
        "bob keeps his default"
    );
    // Changed in the room: everyone hears it; none puts the default back.
    bob.client.request(banner("dry")).await.unwrap();
    let bob_id = bob.client.player();
    ann.room_where(|room| {
        room.members
            .iter()
            .any(|m| m.player == bob_id && m.banner.as_ref().map(Text::as_str) == Some("dry"))
    })
    .await;
    assert_eq!(
        bob.client.request(banner("nope")).await.unwrap_err(),
        ClientError::Refused(RequestError::UnknownBanner)
    );
    bob.client.request(Request::SetBanner(None)).await.unwrap();
    ann.room_where(|room| {
        room.members
            .iter()
            .any(|m| m.player == bob_id && m.banner.is_none())
    })
    .await;
    let _ = &mut bob;
    server.shut_down().await;
}
