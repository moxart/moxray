//! Child-process helpers.

use std::ffi::OsStr;
use tokio::process::Command;

/// A command that does not flash a console window when started from the GUI on Windows.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut command = Command::new(program);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    command
}
