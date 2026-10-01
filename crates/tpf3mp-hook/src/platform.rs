//! Platform entry points that get [`crate::bootstrap`] running early, off the
//! loader lock.

#![allow(unsafe_code)]

#[cfg(windows)]
mod windows {
    use core::ffi::c_void;

    use windows_sys::Win32::Foundation::{CloseHandle, HMODULE};
    use windows_sys::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
    use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
    use windows_sys::Win32::System::Threading::CreateThread;

    /// `TRUE`, the value `DllMain` returns to let the load proceed.
    const DLL_MAIN_OK: i32 = 1;

    /// The DLL entry point. On attach it starts a worker thread and returns at
    /// once: doing the real work here would run under the loader lock, and
    /// loading a library or blocking there can deadlock the process.
    #[unsafe(no_mangle)]
    pub extern "system" fn DllMain(module: HMODULE, reason: u32, _reserved: *mut c_void) -> i32 {
        if reason == DLL_PROCESS_ATTACH {
            // SAFETY: DisableThreadLibraryCalls just drops thread notifications
            // for our module; the handle is the one the loader passed us.
            unsafe {
                DisableThreadLibraryCalls(module);
            }
            // SAFETY: create a plain worker thread; `bootstrap_thread` has the
            // required signature and does not touch the loader lock.
            let handle = unsafe {
                CreateThread(
                    core::ptr::null(),
                    0,
                    Some(bootstrap_thread),
                    core::ptr::null(),
                    0,
                    core::ptr::null_mut(),
                )
            };
            if !handle.is_null() {
                // SAFETY: the thread runs detached; we only drop our handle.
                unsafe {
                    CloseHandle(handle);
                }
            }
        }
        DLL_MAIN_OK
    }

    extern "system" fn bootstrap_thread(_parameter: *mut c_void) -> u32 {
        crate::bootstrap();
        0
    }
}

/// Tells the launcher, which keeps the game suspended meanwhile, that the
/// hook has armed what must be in place before the game runs (the main
/// menu's entry): it sets the event `tpf3mp_ipc::hook_ready_event` names
/// for this process. Set once, by [`Ready::signal`] or when dropped, so
/// every way out of the bootstrap lets the game run.
pub(crate) struct Ready {
    signalled: bool,
}

impl Ready {
    pub(crate) fn new() -> Self {
        Self { signalled: false }
    }

    /// Lets the game run, and says so in the hook's log.
    pub(crate) fn signal(mut self, log: &mut crate::Logger) {
        let told = self.set();
        log.line(if told {
            "told the launcher the hook is ready: the game runs from here"
        } else {
            "no launcher waits for the hook to be ready (no event)"
        });
    }

    fn set(&mut self) -> bool {
        if std::mem::replace(&mut self.signalled, true) {
            return false;
        }
        set_ready_event()
    }
}

impl Drop for Ready {
    fn drop(&mut self) {
        let _ = self.set();
    }
}

/// Sets this process's ready event, if a launcher made one.
#[cfg(windows)]
fn set_ready_event() -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{EVENT_MODIFY_STATE, OpenEventW, SetEvent};
    let name: Vec<u16> = tpf3mp_ipc::hook_ready_event(std::process::id())
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: a NUL-terminated name; the handle, if any, is closed here.
    unsafe {
        let event = OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr());
        if event.is_null() {
            return false;
        }
        let set = SetEvent(event) != 0;
        CloseHandle(event);
        set
    }
}

/// No launcher waits on other systems: the game is not started suspended.
#[cfg(not(windows))]
fn set_ready_event() -> bool {
    false
}

#[cfg(unix)]
mod unix {
    /// A load-time constructor: on Linux, as `LD_PRELOAD` loads the hook
    /// into the game the launcher starts. It hands off to a thread so the
    /// game's own startup is never blocked by our work. `ctor` runs this
    /// before `main`, which is inherently unsafe, hence `#[ctor(unsafe)]`.
    #[ctor::ctor(unsafe)]
    fn tpf3mp_hook_init() {
        std::thread::spawn(crate::bootstrap);
    }
}
