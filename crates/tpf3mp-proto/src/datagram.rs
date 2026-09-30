//! Advisory traffic on QUIC datagrams: where a player is pointing, and what
//! their build tool shows. See "Advisory traffic" in `docs/PROTOCOL.md`.
//!
//! Nothing here is part of the room's event log. A datagram is not sealed, has
//! no step, and is not replayed: a game that loses one draws the next, and a
//! game that never receives any draws its own. That is what makes this usable
//! while the room is paused, where a turn event would wait for a step that
//! does not come.

use serde::{Deserialize, Serialize};

use crate::{ChatText, PlayerId, action::Pos2};

/// Largest datagram accepted, in bytes: the frame header and a cursor.
pub const DATAGRAM_MAX_FRAME: usize = 1024;

/// One datagram's message. Variants are identified by position: append, never
/// reorder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Datagram {
    /// Where this player's pointer is over the ground plane, and whether a
    /// build tool is showing something there.
    Cursor(Cursor),
}

/// A player's pointer, in millimetres on the ground plane.
///
/// `None` for [`Cursor::at`] is the player lifting their pointer out of the
/// world, or closing a tool's preview: the receiver drops the marker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    /// The player whose pointer this is. The server fills it in, so a client
    /// cannot send a cursor as someone else.
    pub player: PlayerId,
    /// Where the pointer is, or `None` when it is not over the world.
    pub at: Option<Pos2>,
    /// True while the player drags a build tool and the game shows its
    /// preview here. The receiver may draw the marker differently, so a
    /// pointer that is about to become a build is told apart from one that is
    /// only looking.
    pub building: bool,
    /// What the player says on the marker, at most a short word: their
    /// company colour's name, or the tool they hold.
    pub label: Option<ChatText>,
}
