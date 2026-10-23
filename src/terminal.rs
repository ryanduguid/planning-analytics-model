//! What Python learned about the terminal: the width argparse wraps help to
//! (`shutil.get_terminal_size`) and whether each stream is coloured
//! (`_colorize.can_colorize`).

use crate::py;

#[derive(Clone, Debug)]
pub struct Terminal {
    pub columns: i64,
    pub stdout_tty: bool,
    pub colour_stdout: bool,
    pub colour_stderr: bool,
}

#[cfg(windows)]
mod os {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::Console::{
        CONSOLE_SCREEN_BUFFER_INFO, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode,
        GetConsoleScreenBufferInfo, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE, STD_OUTPUT_HANDLE,
    };

    const FILE_TYPE_CHAR: u32 = 2;

    unsafe extern "system" {
        fn GetFileType(handle: HANDLE) -> u32;
    }

    fn handle(fd: i32) -> HANDLE {
        let which: STD_HANDLE = if fd == 1 {
            STD_OUTPUT_HANDLE
        } else {
            STD_ERROR_HANDLE
        };
        // SAFETY: GetStdHandle has no preconditions.
        unsafe { GetStdHandle(which) }
    }

    /// The C runtime's `_isatty`: a character device, the console or NUL.
    pub fn isatty(fd: i32) -> bool {
        let h = handle(fd);
        if h.is_null() || h as isize == -1 {
            return false;
        }
        // SAFETY: GetFileType accepts any handle value.
        unsafe { GetFileType(h) == FILE_TYPE_CHAR }
    }

    /// `os.get_terminal_size(1)`'s columns.
    pub fn columns() -> Option<i64> {
        let h = handle(1);
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
        // SAFETY: info is a valid, writable CONSOLE_SCREEN_BUFFER_INFO.
        if unsafe { GetConsoleScreenBufferInfo(h, &mut info) } == 0 {
            return None;
        }
        Some(i64::from(info.srWindow.Right) - i64::from(info.srWindow.Left) + 1)
    }

    /// `nt._supports_virtual_terminal()`, which reads the error stream's console.
    pub fn virtual_terminal() -> bool {
        let mut mode = 0u32;
        // SAFETY: mode is a valid, writable u32.
        if unsafe { GetConsoleMode(handle(2), &mut mode) } == 0 {
            return false;
        }
        mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0
    }
}

#[cfg(unix)]
mod os {
    pub fn isatty(fd: i32) -> bool {
        // SAFETY: isatty accepts any descriptor.
        unsafe { libc::isatty(fd) == 1 }
    }

    pub fn columns() -> Option<i64> {
        let mut size: libc::winsize = unsafe { std::mem::zeroed() };
        // SAFETY: size is a valid, writable winsize.
        if unsafe { libc::ioctl(1, libc::TIOCGWINSZ, &mut size) } != 0 {
            return None;
        }
        Some(i64::from(size.ws_col))
    }

    pub fn virtual_terminal() -> bool {
        true
    }
}

fn process_env(name: &str) -> Option<String> {
    std::env::var_os(name).map(|v| v.to_string_lossy().into_owned())
}

/// What the process's own console reports: whether each stream is a
/// terminal, the console's columns, and whether it takes colour sequences.
#[derive(Clone, Copy, Debug)]
pub struct Console {
    pub stdout_tty: bool,
    pub stderr_tty: bool,
    pub columns: Option<i64>,
    pub virtual_terminal: bool,
}

impl Console {
    pub fn detect() -> Console {
        Console {
            stdout_tty: os::isatty(1),
            stderr_tty: os::isatty(2),
            columns: os::columns(),
            virtual_terminal: os::virtual_terminal(),
        }
    }

    /// Pipes, as a test harness or CI sees them.
    pub fn pipes() -> Console {
        Console {
            stdout_tty: false,
            stderr_tty: false,
            columns: None,
            virtual_terminal: false,
        }
    }
}

fn can_colorize(env: &dyn Fn(&str) -> Option<String>, tty: bool, console: &Console) -> bool {
    match env("PYTHON_COLORS").as_deref() {
        Some("0") => return false,
        Some("1") => return true,
        _ => {}
    }
    if env("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        return false;
    }
    if env("FORCE_COLOR").is_some_and(|v| !v.is_empty()) {
        return true;
    }
    if env("TERM").as_deref() == Some("dumb") {
        return false;
    }
    // Only Windows consoles can refuse colour sequences.
    #[cfg(windows)]
    if !console.virtual_terminal {
        return false;
    }
    #[cfg(not(windows))]
    let _ = console;
    tty
}

/// `shutil.get_terminal_size().columns`.
fn columns(env: &dyn Fn(&str) -> Option<String>, console: &Console) -> i64 {
    let from_env = env("COLUMNS")
        .and_then(|v| py::parse_int(&v).ok())
        .map(|n| i64::try_from(n.clone()).unwrap_or(if n > 0.into() { i64::MAX / 4 } else { 0 }))
        .unwrap_or(0)
        .min(i64::MAX / 4);
    if from_env > 0 {
        return from_env;
    }
    match console.columns {
        Some(c) if c > 0 => c,
        _ => 80,
    }
}

impl Terminal {
    pub fn detect() -> Terminal {
        Terminal::from(&process_env, &Console::detect())
    }

    /// What Python would decide from these environment variables and this console.
    pub fn from(env: &dyn Fn(&str) -> Option<String>, console: &Console) -> Terminal {
        Terminal {
            columns: columns(env, console),
            stdout_tty: console.stdout_tty,
            colour_stdout: can_colorize(env, console.stdout_tty, console),
            colour_stderr: can_colorize(env, console.stderr_tty, console),
        }
    }

    /// An 80-column terminal without colour, as tests and pipes see it.
    pub fn plain() -> Terminal {
        Terminal {
            columns: 80,
            stdout_tty: false,
            colour_stdout: false,
            colour_stderr: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_console_width_counts_where_columns_gives_none() {
        // shutil.get_terminal_size: COLUMNS first, then the console, then 80.
        let console = |columns| Console {
            columns,
            ..Console::pipes()
        };
        let zero = |name: &str| (name == "COLUMNS").then(|| "0".to_string());
        assert_eq!(Terminal::from(&zero, &console(Some(100))).columns, 100);
        let unset = |_: &str| None;
        assert_eq!(Terminal::from(&unset, &console(Some(0))).columns, 80);
        assert_eq!(Terminal::from(&unset, &console(None)).columns, 80);
        // int(COLUMNS) raising ValueError counts as 0, however long the text.
        let long = |name: &str| (name == "COLUMNS").then(|| "x".repeat(201));
        assert_eq!(Terminal::from(&long, &console(Some(90))).columns, 90);
    }
}
