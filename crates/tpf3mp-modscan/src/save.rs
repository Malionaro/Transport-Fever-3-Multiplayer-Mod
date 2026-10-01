//! The mods a Transport Fever 3 save lists, read from the file without the
//! game (build 40408).
//!
//! A save is one zstd frame. Near its start (after the `tf**` magic and a
//! few settings) is the list of the game's mods as `GameSaveCommandData`
//! writes them (`api/tealdef/api/cmd.d.tl`, `modDescs : {Mod.GameModDesc}`):
//! a little-endian `u32` count, then per mod five `u32`-length strings and
//! an `i32`:
//!
//! ```text
//! modId.name   "tpf3mp_1"
//! modSource    "StagingArea"            ("DLC", ...)
//! modhubModId  "StagingArea,tpf3mp_1"   (the source, a comma, an id)
//! name         "TPF3-MP"
//! url          "https://github.com/..." (often empty)
//! severityRemove  0, 1 or 2
//! ```
//!
//! SEEN in saves of build 40408 with the two DLCs and TPF3-MP. The fields
//! before the list vary, so the list is found by trying each offset in the
//! first [`HEAD_BYTES`] and taking the first where a whole list parses and
//! every entry holds together (a mod id, a hub id that starts with the
//! source and a comma, a severity of 0 to 2). A save where none does is
//! refused, never guessed at.

use std::{fs::File, io::Read, path::Path};

/// How much of the decompressed save is looked through.
pub const HEAD_BYTES: usize = 64 * 1024;
/// Most mods a list may hold.
const MAX_MODS: u32 = 4096;
/// Longest string of an entry.
const MAX_STRING: u32 = 4096;

/// One mod a save lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveMod {
    /// The mod's id, as `app.loadGame`'s `info.mods` names it.
    pub id: String,
    /// Where the game had it from: `StagingArea`, `DLC`, ...
    pub source: String,
    /// Its name for players.
    pub name: String,
}

/// The mods the save at `path` lists, in its order.
pub fn mods(path: &Path) -> Result<Vec<SaveMod>, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut decoder =
        zstd::stream::read::Decoder::new(file).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut head = Vec::with_capacity(HEAD_BYTES);
    let mut buf = [0u8; 8192];
    while head.len() < HEAD_BYTES {
        match decoder.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => head.extend_from_slice(&buf[..n.min(HEAD_BYTES - head.len())]),
            Err(e) => {
                return Err(format!(
                    "{}: not a save the game wrote: {e}",
                    path.display()
                ));
            }
        }
    }
    mods_in(&head).ok_or_else(|| format!("{}: no list of mods found in the save", path.display()))
}

/// The mods listed in the start of a decompressed save, if a list is found.
pub fn mods_in(head: &[u8]) -> Option<Vec<SaveMod>> {
    if !head.starts_with(b"tf**") {
        return None;
    }
    (4..head.len().saturating_sub(4)).find_map(|at| list_at(head, at))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn string_at(bytes: &[u8], at: &mut usize) -> Option<String> {
    let len = u32_at(bytes, *at)?;
    if len > MAX_STRING {
        return None;
    }
    let start = *at + 4;
    let end = start + len as usize;
    let text = std::str::from_utf8(bytes.get(start..end)?).ok()?;
    if text.chars().any(char::is_control) {
        return None;
    }
    *at = end;
    Some(text.to_owned())
}

fn is_mod_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn list_at(bytes: &[u8], at: usize) -> Option<Vec<SaveMod>> {
    let count = u32_at(bytes, at)?;
    if count == 0 || count > MAX_MODS {
        return None;
    }
    let mut pos = at + 4;
    let mut out = Vec::new();
    for _ in 0..count {
        let id = string_at(bytes, &mut pos)?;
        let source = string_at(bytes, &mut pos)?;
        let hub = string_at(bytes, &mut pos)?;
        let name = string_at(bytes, &mut pos)?;
        let _url = string_at(bytes, &mut pos)?;
        let severity = u32_at(bytes, pos)?;
        pos += 4;
        if !is_mod_id(&id)
            || source.is_empty()
            || !hub.starts_with(&format!("{source},"))
            || severity > 2
        {
            return None;
        }
        out.push(SaveMod { id, source, name });
    }
    Some(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn string(out: &mut Vec<u8>, text: &str) {
        out.extend_from_slice(&u32::try_from(text.len()).unwrap().to_le_bytes());
        out.extend_from_slice(text.as_bytes());
    }

    /// The start of a save as build 40408 writes one: settings, then the
    /// mods, then the rest.
    pub(crate) fn head(mods: &[(&str, &str, &str)]) -> Vec<u8> {
        let mut out = b"tf**\x5c\x02\x00\x00".to_vec();
        // A setting that looks like nothing: "company" = 4.
        string(&mut out, "company");
        out.extend_from_slice(&[4, 0, 0, 0, 1, 1, 0, 0, 0, 3, 0, 0, 0]);
        out.extend_from_slice(&u32::try_from(mods.len()).unwrap().to_le_bytes());
        for (id, source, name) in mods {
            string(&mut out, id);
            string(&mut out, source);
            string(&mut out, &format!("{source},{id}"));
            string(&mut out, name);
            string(&mut out, "");
            out.extend_from_slice(&0u32.to_le_bytes());
        }
        out.extend_from_slice(&[1, 0x80, 2, 0, 0, 0x68, 1, 0, 0]);
        out
    }

    #[test]
    fn a_saves_mods_are_read_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("room.sav");
        let bytes = head(&[
            ("urbangames_deluxe_upgrade_pack", "DLC", "Deluxe Upgrade"),
            ("tpf3mp_1", "StagingArea", "TPF3-MP"),
            ("celmi_timetables", "mod.io", "Timetables"),
        ]);
        std::fs::write(&file, zstd::encode_all(&bytes[..], 3).unwrap()).unwrap();
        let mods = mods(&file).unwrap();
        let ids: Vec<&str> = mods.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "urbangames_deluxe_upgrade_pack",
                "tpf3mp_1",
                "celmi_timetables"
            ]
        );
        assert_eq!(mods[1].source, "StagingArea");
        assert_eq!(mods[2].name, "Timetables");
    }

    #[test]
    fn what_is_not_a_save_is_refused() {
        assert_eq!(mods_in(b"not a save at all"), None);
        assert_eq!(mods_in(b"tf**\x01\x00\x00\x00"), None);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.sav");
        std::fs::write(&file, b"plain bytes").unwrap();
        assert!(mods(&file).is_err());
    }
}
