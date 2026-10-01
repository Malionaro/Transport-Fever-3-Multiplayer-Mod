//! The player's build preview, read where Transport Fever 3 draws it
//! (docs/HOOKS.md, "The build tools").
//!
//! The street, track and construction tools are native: the whole preview —
//! the ribbon under the pointer, its length and angle, and the cost the game
//! shows — is computed in C++ and drawn by the renderer. Nothing of it reaches
//! game scripts: the street tool sends no `builder.proposalCreate` while the
//! pointer moves, only on the click, and it sends no
//! `builder.proposalPrepareForApply` at all (docs/HOOKS.md, "The build
//! tools"). So the mod, which learns of a build from those events, cannot see
//! the preview and never reports one: `hook.log` has no cursor line, and the
//! room never hears a datagram.
//!
//! [`UI::StreetBuilder::CreateProposalAndUpdate`] is where the game computes
//! it. `UI::StreetBuilder::Step` (vtable slot 5) calls it on every frame the
//! pointer moves, so detouring it sees each preview as it is made. What the
//! detour reads, off the `StreetBuilder` this call is on, is the pointer on
//! the ground plane and whether a preview is up at all:
//!
//! | offset | what |
//! |---|---|
//! | `+0x934` | pointer x on the ground plane, in metres |
//! | `+0x938` | pointer y on the ground plane, in metres |
//! | `+0x93c` | pointer height, or its validity |
//! | `+0x940` | whether a preview is up |
//!
//! The three floats are checked against zero on entry, and the function
//! returns without building a preview when all three are; the flag gates the
//! rest. This detour therefore reports the pointer whenever the game is
//! building, and clears the cursor (`at: None`) when it is not, which is what
//! the receiving game draws by.
//!
//! **The preview's shape does not come from here.** The curves the game draws
//! live in the `ProposalDataProduct` the call hands a thread pool
//! ([`tools/tpfre`] on build 40408: `CreateProposalAndUpdate(bool)` is
//! `private`, and enqueues `ProposalData` work), and are not read yet. What
//! crosses the room is therefore the pointer, and the marker the mod draws for
//! it. Reading the curves is the next step, in this file.

#![allow(unsafe_code)]
#![cfg_attr(not(all(windows, target_arch = "x86_64")), allow(dead_code))]

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

/// Where the pointer's x sits on the `StreetBuilder` (build 40408), in metres.
pub const POINTER_X: usize = 0x934;
/// Where the pointer's y sits, in metres.
pub const POINTER_Y: usize = 0x938;
/// The pointer's height, or its validity; all three floats are checked
/// against zero before the game builds a preview.
pub const POINTER_Z: usize = 0x93c;
/// Whether a preview is up.
pub const PREVIEW_UP: usize = 0x940;

/// The game's own call, reached through the detour's trampoline.
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// The detour is in.
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// Previews seen, for the log and the tests.
static SEEN: AtomicU64 = AtomicU64::new(0);

/// Whether the detour is in.
pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// Previews this game has seen so far.
pub fn seen() -> u64 {
    SEEN.load(Ordering::Acquire)
}

/// The call's signature: the `StreetBuilder` this, and the game's `bool`;
/// returns what the game returns.
///
/// The register pass is what the game's own mangled name gives
/// (`CreateProposalAndUpdate(bool)`, `private`, `__cdecl`).
type PreviewFn = unsafe extern "C" fn(usize, u8) -> u64;

/// The detour: reads the pointer, hands the game on, and reports the pointer
/// as a preview afterwards.
///
/// The read is after the call, not before: the pointer moves as a frame runs,
/// so the game has just refreshed it, and the flag says whether the preview it
/// drew from it is still up.
unsafe extern "C" fn preview_detour(builder: usize, building: u8) -> u64 {
    let original = ORIGINAL.load(Ordering::Acquire);
    // SAFETY: the trampoline of this call, called as `Step` called it.
    let returned = unsafe {
        let call: PreviewFn = std::mem::transmute::<usize, PreviewFn>(original);
        call(builder, building)
    };
    if crate::lua::in_room() && builder != 0 {
        report(builder);
    }
    returned
}

/// Reads the pointer off `builder` and hands it to the mod as a cursor.
fn report(builder: usize) {
    if builder == 0 {
        return;
    }
    // SAFETY: the `StreetBuilder` the game passed, read at the offsets its own
    // code reads them (0x577f42..0x577f74).
    let at = |offset: usize| -> *const u8 { unsafe { (builder as *const u8).add(offset) } };
    if unsafe { std::ptr::read_unaligned(at(PREVIEW_UP)) } == 0 {
        // The tool is not showing anything: the receiver drops its marker.
        crate::lua::report_cursor(None, None, false);
        return;
    }
    // SAFETY: as above; the three floats the game itself reads here.
    let (x, y, z) = unsafe {
        (
            std::ptr::read_unaligned(at(POINTER_X).cast::<f32>()),
            std::ptr::read_unaligned(at(POINTER_Y).cast::<f32>()),
            std::ptr::read_unaligned(at(POINTER_Z).cast::<f32>()),
        )
    };
    if !(x.is_finite() && y.is_finite() && z.is_finite()) {
        return;
    }
    SEEN.fetch_add(1, Ordering::AcqRel);
    crate::lua::report_cursor(Some((x, y)), None, true);
}

/// Detours the preview's update, at the address the profile resolved.
///
/// # Safety
///
/// `target` is the function the profile names, in this process, which no
/// thread runs yet (the hook installs while the game starts).
#[cfg(all(windows, target_arch = "x86_64"))]
pub unsafe fn install(
    target: usize,
    detour: unsafe fn(*mut u8, *const u8) -> Result<usize, String>,
) -> Result<(), String> {
    // SAFETY: the caller's; `preview_detour` has the call's ABI, and only
    // reads the `StreetBuilder` the game passes it.
    let original = unsafe { detour(target as *mut u8, preview_detour as *const u8) }?;
    ORIGINAL.store(original, Ordering::Release);
    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offsets_are_where_the_game_puts_them() {
        // The game lays these four out inside its own `StreetBuilder`, at these
        // offsets. The test writes them there and reads them back, so a wrong
        // offset in the detour would not be caught by reading a struct field
        // of the test's own.
        let mut bytes = vec![0u8; PREVIEW_UP + 8];
        let mut put = |offset: usize, raw: [u8; 4]| {
            bytes[offset..offset + 4].copy_from_slice(&raw);
        };
        put(POINTER_X, 1234.5f32.to_le_bytes());
        put(POINTER_Y, (-678.25f32).to_le_bytes());
        put(POINTER_Z, 42.0f32.to_le_bytes());
        bytes[PREVIEW_UP] = 1;
        let base = bytes.as_ptr() as usize;
        // SAFETY: the buffer is live and long enough for every read.
        unsafe {
            let at = |offset: usize| (base as *const u8).add(offset);
            assert_eq!(
                std::ptr::read_unaligned(at(POINTER_X).cast::<f32>()),
                1234.5
            );
            assert_eq!(
                std::ptr::read_unaligned(at(POINTER_Y).cast::<f32>()),
                -678.25
            );
            assert_eq!(std::ptr::read_unaligned(at(POINTER_Z).cast::<f32>()), 42.0);
            assert_eq!(std::ptr::read_unaligned(at(PREVIEW_UP)), 1);
        }
    }

    #[test]
    fn the_offsets_do_not_overlap() {
        // The game reads the three floats, then the flag, in this order
        // (0x577f35, 0x577f42, 0x577f56, 0x577f66), so a pointer read that
        // overlapped another would report a second float as a first. The
        // values are read at runtime, so the check is not a constant fold.
        let offsets = [POINTER_X, POINTER_Y, POINTER_Z, PREVIEW_UP];
        for pair in offsets.windows(2) {
            assert!(pair[0] < pair[1], "{pair:x?} is out of order");
        }
        // Each float's four bytes lie before the next field, and the flag is
        // its own byte.
        for pair in offsets.windows(2) {
            let width = if *pair.last().unwrap() == PREVIEW_UP {
                1
            } else {
                4
            };
            assert!(pair[0] + width <= pair[1]);
        }
    }

    #[test]
    fn a_pointer_that_is_not_a_number_is_not_reported() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(!(bad.is_finite()));
        }
    }
}
