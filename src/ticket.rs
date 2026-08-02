//! Getting this world's join ticket to somebody who is not in the room.
//!
//! The LAN join menu covers everybody on the sofa. A cousin two towns away still needs
//! the ticket itself, and the machine hosting is usually a Deck in gaming mode: no
//! terminal, no way to select text, a controller and nothing else. So sharing writes the
//! ticket to two places at once — the clipboard, for a machine somebody is sitting at
//! with a chat window open, and a file at a fixed path, for a machine somebody can only
//! reach over ssh.
//!
//! Both, always, rather than one with the other as a fallback: which of the two will
//! work is not knowable from in here, and a ticket that went somewhere useless is
//! indistinguishable from one that went nowhere.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Where the ticket is left. Fixed, because its whole job is to be findable by somebody
/// who has an ssh prompt and no idea what this program is called.
pub fn path() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"));
    data.join("blockgame/ticket.txt")
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The path as it is worth showing a player: under `$HOME`, spelled with a `~`, because
/// the full one does not fit on a line somebody is reading off a handheld.
fn short(path: &Path) -> String {
    match path.strip_prefix(home()) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Somewhere to put the join ticket so it can leave this machine.
///
/// The clipboard is opened once and held for the life of the game, and that is
/// load-bearing rather than thrift: on X11 and Wayland the copying process *is* the
/// clipboard until something else takes it over, so a handle dropped after the copy
/// takes the ticket with it and the paste a minute later comes up empty.
#[derive(Default)]
pub struct Share {
    clipboard: Option<arboard::Clipboard>,
}

impl Share {
    /// Puts `ticket` everywhere it can, and returns what to tell the player — including
    /// when it went nowhere, since a silent failure here is a cousin who never gets to
    /// play.
    pub fn put(&mut self, ticket: &str) -> String {
        let path = path();
        let clipboard = self.copy(ticket);
        let file = to_file(ticket, &path);
        let where_ = short(&path);
        match (clipboard, file) {
            (Ok(()), Ok(())) => format!("join ticket copied\nand written to {where_}"),
            (Err(_), Ok(())) => format!("no clipboard here\njoin ticket written to {where_}"),
            (Ok(()), Err(e)) => format!("join ticket copied\n(but not saved: {e})"),
            (Err(c), Err(f)) => format!("could not share the ticket\nclipboard: {c}\nfile: {f}"),
        }
    }

    fn copy(&mut self, ticket: &str) -> Result<()> {
        let clipboard = match &mut self.clipboard {
            Some(clipboard) => clipboard,
            // Opened on first use, not at startup: a machine with no display server
            // still hosts perfectly well, and refusing to start over a clipboard would
            // be absurd.
            none => none.insert(arboard::Clipboard::new().context("opening the clipboard")?),
        };
        clipboard
            .set_text(ticket)
            .context("putting the ticket on the clipboard")
    }
}

fn to_file(ticket: &str, path: &Path) -> Result<()> {
    let dir = path
        .parent()
        .context("the ticket path has no directory to live in")?;
    std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    // A trailing newline so `cat` over ssh leaves a usable prompt, and so the file reads
    // as a line of text rather than a blob.
    std::fs::write(path, format!("{ticket}\n"))
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file half must work on a machine with no display server at all — that is the
    /// case it exists for, and it has to make its own directory on a machine that has
    /// never run this game.
    #[test]
    fn the_ticket_reaches_the_file() {
        let path = std::env::temp_dir()
            .join(format!("blockgame-ticket-{}", std::process::id()))
            .join("blockgame/ticket.txt");
        to_file("abc123", &path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "abc123\n");
        std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).ok();
    }

    /// The path is read off a handheld screen, so it is shown the way a person writes
    /// it.
    #[test]
    fn a_path_under_home_is_shown_short() {
        let path = home().join(".local/share/blockgame/ticket.txt");
        assert_eq!(short(&path), "~/.local/share/blockgame/ticket.txt");
        assert_eq!(short(Path::new("/tmp/x")), "/tmp/x");
    }
}
