//! Windows: the console window and the log of a game started from Explorer or a Start menu
//! shortcut. The game is a console program, so Windows gives it a console of its own then,
//! which only it is attached to (or none at all on Windows 11 24H2 and later, through the
//! manifest's consoleAllocationPolicy, packaging/windows/lasertag.manifest). It closes that
//! console and writes its log to `lasertag.log` in `%APPDATA%\lasertag` (beside the settings)
//! instead, `lasertag-vr.log` for `--vr`. Started from a terminal, it shares the terminal's
//! console and keeps writing to it.

use std::os::windows::io::IntoRawHandle;
use std::path::PathBuf;

use windows_sys::Win32::System::Console::{
    FreeConsole, GetConsoleProcessList, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
};

/// Call first in `run`, before the log is set up.
pub fn leave_explorer_console() {
    let mut pids = [0u32; 2];
    // SAFETY: the list has room for the two entries it is given.
    let attached = unsafe { GetConsoleProcessList(pids.as_mut_ptr(), pids.len() as u32) };
    if attached > 1 {
        return;
    }
    if attached == 1 {
        // SAFETY: nothing holds on to the console's handles yet.
        unsafe { FreeConsole() };
    }
    let Some(dir) = std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("lasertag")) else {
        return;
    };
    let name = if std::env::args().any(|a| a == "--vr") {
        "lasertag-vr.log"
    } else {
        "lasertag.log"
    };
    let Ok(file) =
        std::fs::create_dir_all(&dir).and_then(|_| std::fs::File::create(dir.join(name)))
    else {
        return;
    };
    // Rust's stdout and stderr look the handles up on every write, so the log and panic
    // messages go to the file. The handle stays open for the life of the process.
    let handle = file.into_raw_handle();
    // SAFETY: `handle` is an open file handle that is never closed.
    unsafe {
        SetStdHandle(STD_OUTPUT_HANDLE, handle);
        SetStdHandle(STD_ERROR_HANDLE, handle);
        // No colour codes in the file (bevy's log reads NO_COLOR); no other threads yet.
        std::env::set_var("NO_COLOR", "1");
    }
}
