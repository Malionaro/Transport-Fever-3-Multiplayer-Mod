//! Saving and loading whole worlds in the game (docs/HOOKS.md, "The room's
//! world"): the files, and what the hook asks of the mod's GUI for them.
//!
//! The game saves and loads through its own script API, which only the
//! GUI's Lua can call (`app.saveGame`, `app.loadGame`), and only into and
//! from its own save folder: `<Steam>/userdata/<account>/3493540/local/save`.
//! So a save the room orders is made under a name of this game's own
//! (`tpf3mp_<pid>_<event>`, as two games on one PC share the folder), found
//! there once the GUI says it is written, and moved to the room's file; the
//! picture the game writes beside it is removed. A world the room hands
//! over is copied into the folder as `tpf3mp_room_<pid>` and loaded from
//! there, by the GUI of the world the game has up, or by the game's main menu
//! when it has none (`crate::menu`); it stays until the next one replaces
//! it.

use std::{
    fs,
    path::{Path, PathBuf},
};

use tpf3mp_bridge::Notice;
use tpf3mp_proto::PlayerId;

use crate::{
    lua,
    step::{GameControl, LoadFrom},
};

/// Transport Fever 3's Steam app.
pub const STEAM_APP: &str = "3493540";

/// The game's side of saves and loads, through the mod's GUI.
pub struct GuiWorlds {
    folder: Result<PathBuf, String>,
    tag: String,
}

impl GuiWorlds {
    /// For the game's own save folder, as Steam names it.
    pub fn in_steam_folder() -> Self {
        Self::in_folder(save_folder())
    }

    /// For the save folder `folder`, or why there is none.
    pub fn in_folder(folder: Result<PathBuf, String>) -> Self {
        Self {
            folder,
            tag: std::process::id().to_string(),
        }
    }
}

impl GameControl for GuiWorlds {
    fn request_save(&mut self, name: &str) {
        lua::request_save(name);
    }

    fn save_result(&mut self) -> Option<Result<PathBuf, String>> {
        let answer = lua::take_save_answer()?;
        Some(answer.and_then(|name| {
            let folder = self.folder.clone()?;
            let file = folder.join(format!("{name}.sav"));
            // The picture beside it is the player's save menu's, not the
            // room's.
            let _ = fs::remove_file(folder.join(format!("{name}.jpg")));
            if file.is_file() {
                Ok(file)
            } else {
                Err(format!(
                    "the game says it saved {name}, but {} is not there",
                    file.display()
                ))
            }
        }))
    }

    fn room_notice(&mut self, notice: &Notice) {
        lua::notice(notice);
    }

    fn set_me(&mut self, player: PlayerId) {
        lua::set_me(player);
    }

    fn set_mods(&mut self, mods: Option<tpf3mp_bridge::ModLists>) {
        lua::set_mods(mods);
    }

    fn request_load(&mut self, file: &Path, from: LoadFrom) -> Result<(), String> {
        let folder = self.folder.clone()?;
        let name = format!("tpf3mp_room_{}", self.tag);
        let target = folder.join(format!("{name}.sav"));
        fs::copy(file, &target).map_err(|error| {
            format!(
                "copying the room's save {} to {}: {error}",
                file.display(),
                target.display()
            )
        })?;
        match from {
            LoadFrom::Gui => lua::request_load(&name),
            LoadFrom::Menu => lua::request_menu_load(&name),
        }
        Ok(())
    }

    fn load_done(&mut self) -> bool {
        lua::load_done()
    }

    fn load_failed(&mut self) -> Option<String> {
        lua::take_load_failure()
    }

    fn world_up(&mut self) -> Option<u64> {
        lua::take_world_up()
    }
}

/// The save folder of the Steam account playing: Steam's folder and the
/// account it runs as, from the registry; without an account, the one
/// account with a save folder for the game.
fn save_folder() -> Result<PathBuf, String> {
    let root = steam_root().ok_or("cannot find Steam's folder")?;
    let userdata = root.join("userdata");
    if let Some(account) = active_account().filter(|account| *account != 0) {
        let folder = save_folder_of(&userdata.join(account.to_string()));
        if folder.is_dir() {
            return Ok(folder);
        }
    }
    let mut found: Vec<PathBuf> = fs::read_dir(&userdata)
        .map_err(|error| format!("reading {}: {error}", userdata.display()))?
        .flatten()
        .map(|account| save_folder_of(&account.path()))
        .filter(|folder| folder.is_dir())
        .collect();
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(format!(
            "no Steam account under {} has a save folder for the game",
            userdata.display()
        )),
        _ => Err("several Steam accounts have a save folder for the game, and Steam names none as playing".into()),
    }
}

fn save_folder_of(account: &Path) -> PathBuf {
    account.join(STEAM_APP).join("local").join("save")
}

#[cfg(windows)]
fn steam_root() -> Option<PathBuf> {
    registry::string(r"Software\Valve\Steam", "SteamPath")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .or_else(|| {
            let base = std::env::var_os("ProgramFiles(x86)")?;
            Some(PathBuf::from(base).join("Steam")).filter(|path| path.is_dir())
        })
}

#[cfg(windows)]
fn active_account() -> Option<u32> {
    registry::dword(r"Software\Valve\Steam\ActiveProcess", "ActiveUser")
}

#[cfg(not(windows))]
fn steam_root() -> Option<PathBuf> {
    None
}

#[cfg(not(windows))]
fn active_account() -> Option<u32> {
    None
}

/// The current user's registry values Steam writes.
#[cfg(windows)]
#[allow(unsafe_code)]
mod registry {
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    pub fn string(key: &str, value: &str) -> Option<String> {
        let (key, value) = (wide(key), wide(value));
        let mut buffer = vec![0u16; 1024];
        let mut size = u32::try_from(buffer.len() * 2).ok()?;
        // SAFETY: both names end in a NUL, and the buffer holds `size`
        // bytes, which RegGetValueW writes back.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &raw mut size,
            )
        };
        if status != 0 {
            return None;
        }
        let units = (size as usize / 2).min(buffer.len());
        let text = String::from_utf16_lossy(&buffer[..units]);
        let text = text.trim_end_matches('\0');
        (!text.is_empty()).then(|| text.to_owned())
    }

    pub fn dword(key: &str, value: &str) -> Option<u32> {
        let (key, value) = (wide(key), wide(value));
        let mut data = 0u32;
        let mut size = 4u32;
        // SAFETY: as above, with room for one DWORD.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&raw mut data).cast(),
                &raw mut size,
            )
        };
        (status == 0).then_some(data)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::PoisonError;

    use super::*;
    use crate::lua::tests::{Lua, SERIAL};

    fn folder(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tpf3mp-worlds-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_save_is_found_in_the_folder_once_the_gui_says_it_is_written() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let dir = folder("save");
        let lua = Lua::new();
        lua.register();
        let _ = lua::take_save_answer();
        let mut worlds = GuiWorlds::in_folder(Ok(dir.clone()));
        worlds.request_save("tpf3mp_1_5");
        assert_eq!(worlds.save_result(), None, "not answered yet");
        fs::write(dir.join("tpf3mp_1_5.sav"), b"world").unwrap();
        fs::write(dir.join("tpf3mp_1_5.jpg"), b"picture").unwrap();
        lua.run("tpf3mp_native.poll() tpf3mp_native.saved('tpf3mp_1_5', true)")
            .unwrap();
        assert_eq!(worlds.save_result(), Some(Ok(dir.join("tpf3mp_1_5.sav"))));
        assert!(
            !dir.join("tpf3mp_1_5.jpg").exists(),
            "the picture is not kept"
        );
        // A save the GUI claims but that is not there is a failure.
        worlds.request_save("tpf3mp_1_6");
        lua.run("tpf3mp_native.saved('tpf3mp_1_6', true)").unwrap();
        assert!(
            worlds
                .save_result()
                .unwrap()
                .unwrap_err()
                .contains("is not there")
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_rooms_save_is_copied_in_and_the_gui_asked_to_load_it() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let dir = folder("load");
        let room = dir.join("from-the-room.sav");
        fs::write(&room, b"the room's world").unwrap();
        let lua = Lua::new();
        lua.register();
        let mut worlds = GuiWorlds::in_folder(Ok(dir.clone()));
        worlds.request_load(&room, LoadFrom::Gui).unwrap();
        let name = format!("tpf3mp_room_{}", std::process::id());
        assert_eq!(
            fs::read(dir.join(format!("{name}.sav"))).unwrap(),
            b"the room's world"
        );
        assert_eq!(lua.run("return tpf3mp_native.poll().load"), Ok(name));
        // Without a save folder nothing is asked.
        let mut nowhere = GuiWorlds::in_folder(Err("no Steam".into()));
        assert_eq!(
            nowhere.request_load(&room, LoadFrom::Menu),
            Err("no Steam".into())
        );
        assert_eq!(lua.run("return tpf3mp_native.poll()"), Ok("nil".into()));
        assert_eq!(lua::take_menu_load(), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn with_no_world_up_the_main_menu_is_asked_to_load_the_rooms_save() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let dir = folder("menu-load");
        let room = dir.join("from-the-room.sav");
        fs::write(&room, b"the room's world").unwrap();
        let lua = Lua::new();
        lua.register();
        let mut worlds = GuiWorlds::in_folder(Ok(dir.clone()));
        worlds.request_load(&room, LoadFrom::Menu).unwrap();
        let name = format!("tpf3mp_room_{}", std::process::id());
        assert!(dir.join(format!("{name}.sav")).is_file());
        assert_eq!(
            lua.run("return tpf3mp_native.poll()"),
            Ok("nil".into()),
            "not the GUI's"
        );
        assert_eq!(lua::take_menu_load(), Some(name));
        lua::menu_load_failed("no menu".into());
        assert_eq!(worlds.load_failed().as_deref(), Some("no menu"));
        assert_eq!(worlds.load_failed(), None);
        fs::remove_dir_all(&dir).unwrap();
    }
}
