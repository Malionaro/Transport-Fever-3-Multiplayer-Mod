//! Where Transport Fever 3 keeps the mods a player can activate (build
//! 40408, Windows; the other platforms' places are to confirm):
//!
//! - Mod Hub (mod.io) downloads: `%LOCALAPPDATA%\mod.io\10640\mods\<mod.io
//!   id>`, whose `mod.json` names the mod (`celmi_timetables` in
//!   `...\6037864`);
//! - local mods: `<Steam>\userdata\<account>\3493540\local\staging_area\<modId>`
//!   (investigation/TF3_MODS_2026-09-27.md), and `...\local\mods`;
//! - the game's own: `<game>\mods` and `<game>\dlcs`.
//!
//! A mod is found by the id its `mod.json` gives, or else by its folder's
//! name; which of the two a save lists for a Mod Hub mod is to confirm in
//! the game (docs/MODS.md).

use std::{
    fs,
    path::{Path, PathBuf},
};

/// Transport Fever 3's game id on mod.io.
pub const MODIO_GAME: u32 = 10640;
/// Transport Fever 3's Steam app id.
pub const STEAM_APP: u32 = 3_493_540;

/// The folders mods are kept in, those that exist: the game's own (in
/// `game`, its install folder), each Steam account's local ones (under
/// each of `steam_roots`), and Mod Hub's downloads (under `local_data`,
/// the per-user local data folder).
pub fn roots(
    game: Option<&Path>,
    steam_roots: &[PathBuf],
    local_data: Option<&Path>,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(data) = local_data {
        out.push(
            data.join("mod.io")
                .join(MODIO_GAME.to_string())
                .join("mods"),
        );
    }
    for steam in steam_roots {
        let Ok(accounts) = fs::read_dir(steam.join("userdata")) else {
            continue;
        };
        let mut accounts: Vec<PathBuf> =
            accounts.filter_map(Result::ok).map(|e| e.path()).collect();
        accounts.sort();
        for account in accounts {
            let local = account.join(STEAM_APP.to_string()).join("local");
            out.push(local.join("staging_area"));
            out.push(local.join("mods"));
        }
    }
    if let Some(game) = game {
        out.push(game.join("mods"));
        out.push(game.join("dlcs"));
    }
    out.retain(|root| root.is_dir());
    out
}

/// This player's roots, as [`roots`] with the per-user local data folder.
pub fn default_roots(game: Option<&Path>, steam_roots: &[PathBuf]) -> Vec<PathBuf> {
    roots(game, steam_roots, dirs_local().as_deref())
}

fn dirs_local() -> Option<PathBuf> {
    // %LOCALAPPDATA% on Windows; $XDG_DATA_HOME or ~/.local/share on Linux;
    // ~/Library/Application Support on macOS.
    #[cfg(windows)]
    let found = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let found = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| {
                let home = PathBuf::from(home);
                if cfg!(target_os = "macos") {
                    home.join("Library/Application Support")
                } else {
                    home.join(".local/share")
                }
            })
        });
    found
}

/// One mod found in a root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Its id, from `mod.json`, else the folder's name.
    pub id: String,
    /// Its folder's name.
    pub folder: String,
    pub path: PathBuf,
}

/// Every mod in `roots`: each folder with a `mod.json`, in the roots'
/// order, then by folder name.
pub fn installed(roots: &[PathBuf]) -> Vec<Found> {
    let mut out = Vec::new();
    for root in roots {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.join("mod.json").is_file())
            .collect();
        dirs.sort();
        for path in dirs {
            let folder = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let id = fs::read_to_string(path.join("mod.json"))
                .ok()
                .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                .and_then(|v| v.get("modId").and_then(|id| id.as_str()).map(str::to_owned))
                .unwrap_or_else(|| folder.clone());
            out.push(Found { id, folder, path });
        }
    }
    out
}

/// The folder of the mod named `name`, by id first, then by folder name.
pub fn find<'a>(found: &'a [Found], name: &str) -> Option<&'a Found> {
    found
        .iter()
        .find(|f| f.id == name)
        .or_else(|| found.iter().find(|f| f.folder == name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn mods_are_found_in_mod_hub_steam_and_the_game_folders() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let steam = dir.path().join("steam");
        let game = dir.path().join("game");
        write(
            &data.join("mod.io/10640/mods/6037864/mod.json"),
            r#"{"modId": "celmi_timetables"}"#,
        );
        write(
            &steam.join("userdata/42/3493540/local/staging_area/gw_big_city_1/mod.json"),
            "{}",
        );
        write(
            &game.join("dlcs/urbangames_preorder_pack/mod.json"),
            r#"{"modId": "urbangames_preorder_pack"}"#,
        );
        // Not a mod: no mod.json.
        fs::create_dir_all(game.join("mods/release")).unwrap();

        let roots = roots(Some(&game), std::slice::from_ref(&steam), Some(&data));
        assert_eq!(roots.len(), 4, "{roots:?}");
        let found = installed(&roots);
        let ids: Vec<&str> = found.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "celmi_timetables",
                "gw_big_city_1",
                "urbangames_preorder_pack"
            ]
        );
        assert_eq!(find(&found, "celmi_timetables").unwrap().folder, "6037864");
        assert_eq!(find(&found, "6037864").unwrap().id, "celmi_timetables");
        assert!(find(&found, "missing").is_none());
    }
}
