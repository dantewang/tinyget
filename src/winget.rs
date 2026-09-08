//! Invoking the `winget` CLI and parsing what it prints.
//!
//! winget has no machine-readable output for `upgrade` or `pin list`, so both are
//! read back from the aligned text table it prints. Two things make that awkward:
//! the column headers are localized, and the columns are aligned by *display*
//! width, where a CJK glyph occupies two cells. Both are handled in
//! [`parse_table`]. Output is UTF-8 even when redirected, which is the one thing
//! that makes this tractable at all.

use std::fmt;
use std::io;
use std::process::{Command, Stdio};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use unicode_width::UnicodeWidthChar;

/// Run without flashing up a console window of our own.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Hand the child its own console, which Windows hosts in the user's default
/// terminal application. This is what keeps the upgrade visible instead of silent.
#[cfg(windows)]
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

// ---------------------------------------------------------------- error

#[derive(Debug, Clone)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error(if e.kind() == io::ErrorKind::NotFound {
            "winget was not found. Install the App Installer package from the \
             Microsoft Store, then reopen tinyget."
                .into()
        } else {
            format!("Could not run winget: {e}")
        })
    }
}

pub type Result<T> = std::result::Result<T, Error>;

// ---------------------------------------------------------------- types

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinKind {
    /// Held back from `upgrade --all`, but a direct upgrade still works.
    Pinning,
    /// Refuses every upgrade until the pin is removed.
    Blocking,
    /// Pinned to a version or wildcard range. tinyget displays these but does
    /// not create them.
    Gating,
}

impl PinKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PinKind::Pinning => "Pinning",
            PinKind::Blocking => "Blocking",
            PinKind::Gating => "Gating",
        }
    }

    /// winget prints these values in English regardless of display language.
    fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "Pinning" => Some(PinKind::Pinning),
            "Blocking" => Some(PinKind::Blocking),
            "Gating" => Some(PinKind::Gating),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Upgrade {
    pub name: String,
    pub id: String,
    pub current: String,
    pub available: String,
}

#[derive(Debug, Clone)]
pub struct Pin {
    pub name: String,
    pub id: String,
    pub kind: PinKind,
}

// ---------------------------------------------------------------- process

struct Output {
    stdout: String,
    ok: bool,
}

fn run(args: &[&str]) -> Result<Output> {
    let mut cmd = Command::new("winget");
    cmd.args(args).stdin(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let out = cmd.output()?;
    Ok(Output {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        ok: out.status.success(),
    })
}

/// The last non-empty line of output, which is where winget puts its reason for
/// failing. Falls back to a generic message so the UI is never blank.
fn failure_reason(stdout: &str) -> String {
    stdout
        .lines()
        .map(clean_line)
        .filter(|l| !l.trim().is_empty())
        .next_back()
        .unwrap_or_else(|| "winget reported an error.".into())
}

pub fn version() -> Result<String> {
    Ok(run(&["--version"])?.stdout.trim().to_string())
}

pub fn list_upgrades() -> Result<Vec<Upgrade>> {
    // --include-unknown so packages whose installed version winget cannot read
    // are still offered. --accept-source-agreements only covers the already
    // configured source and keeps this background read from blocking on a prompt
    // it has no console to show.
    let out = run(&[
        "upgrade",
        "--include-unknown",
        "--accept-source-agreements",
        "--disable-interactivity",
    ])?;

    let rows = parse_table(&out.stdout, 5);
    if rows.is_empty() && !out.ok {
        return Err(Error(failure_reason(&out.stdout)));
    }
    Ok(rows
        .into_iter()
        .map(|r| Upgrade {
            name: r[0].clone(),
            id: r[1].clone(),
            current: r[2].clone(),
            available: r[3].clone(),
        })
        .collect())
}

pub fn list_pins() -> Result<Vec<Pin>> {
    let out = run(&["pin", "list", "--accept-source-agreements"])?;

    // Name, Id, Version, Source, Pin type
    let rows = parse_table(&out.stdout, 5);
    if rows.is_empty() && !out.ok {
        return Err(Error(failure_reason(&out.stdout)));
    }
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            Some(Pin {
                name: r[0].clone(),
                id: r[1].clone(),
                kind: PinKind::parse(&r[4])?,
            })
        })
        .collect())
}

pub fn add_pin(id: &str, blocking: bool) -> Result<()> {
    if !is_valid_id(id) {
        return Err(Error(format!("{id:?} is not a usable package identifier.")));
    }
    let mut args = vec!["pin", "add", "--id", id, "--exact", "--accept-source-agreements"];
    if blocking {
        args.push("--blocking");
    }
    let out = run(&args)?;
    if out.ok {
        Ok(())
    } else {
        Err(Error(failure_reason(&out.stdout)))
    }
}

pub fn remove_pin(id: &str) -> Result<()> {
    if !is_valid_id(id) {
        return Err(Error(format!("{id:?} is not a usable package identifier.")));
    }
    let out = run(&[
        "pin",
        "remove",
        "--id",
        id,
        "--exact",
        "--accept-source-agreements",
    ])?;
    if out.ok {
        Ok(())
    } else {
        Err(Error(failure_reason(&out.stdout)))
    }
}

/// The single `winget upgrade` invocation that would upgrade `ids`.
///
/// `upgrade` takes its query positionally and accepts several at once, so the
/// whole selection goes out as one command rather than one per package.
pub fn upgrade_command(ids: &[String]) -> String {
    let mut cmd = String::from("winget upgrade");
    for id in ids {
        cmd.push(' ');
        cmd.push_str(id);
    }
    cmd.push_str(" --exact --include-unknown");
    cmd
}

/// A launched upgrade, still running in its own terminal.
pub struct UpgradeRun {
    terminal: std::process::Child,
    sentinel: std::path::PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub enum UpgradeOutcome {
    /// winget ran to completion and reported this exit code. The terminal is
    /// still open on its "press any key" prompt, so the output stays readable.
    Finished(i32),
    /// The window went away before winget reported: closed by hand, or it never
    /// got that far.
    TerminalClosed,
}

impl UpgradeRun {
    /// Blocks until winget reports, or the terminal disappears.
    ///
    /// The script writes winget's exit code to a sentinel file *before* it
    /// pauses, which keeps "winget finished" and "the user dismissed the window"
    /// as separate events. Watching the process alone could only tell us the
    /// second, and the window deliberately stays open so the output can be read.
    pub fn wait(mut self) -> UpgradeOutcome {
        loop {
            if let Some(code) = self.read_sentinel() {
                return UpgradeOutcome::Finished(code);
            }
            if matches!(self.terminal.try_wait(), Ok(Some(_)) | Err(_)) {
                // The window can go away in the same tick winget finishes, so
                // give the sentinel one last look before calling it abandoned.
                return match self.read_sentinel() {
                    Some(code) => UpgradeOutcome::Finished(code),
                    None => UpgradeOutcome::TerminalClosed,
                };
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    }

    fn read_sentinel(&self) -> Option<i32> {
        let text = std::fs::read_to_string(&self.sentinel).ok()?;
        let code = text.trim().parse().unwrap_or(0);
        let _ = std::fs::remove_file(&self.sentinel);
        Some(code)
    }
}

/// Upgrade `ids` in a terminal window the user can watch.
///
/// Deliberately not silent: no `--silent` and no `--accept-package-agreements`,
/// so installer UI, licence prompts and elevation all appear in front of the
/// user rather than being answered on their behalf. The batch file is written to
/// a fixed path in the temp directory so repeated runs replace it instead of
/// piling up.
pub fn launch_upgrade(ids: &[String]) -> Result<UpgradeRun> {
    if ids.is_empty() {
        return Err(Error("No packages are selected.".into()));
    }
    for id in ids {
        if !is_valid_id(id) {
            return Err(Error(format!("{id:?} is not a usable package identifier.")));
        }
    }

    let temp = std::env::temp_dir();
    let sentinel = temp.join("tinyget-upgrade.done");
    // A leftover from a previous run would be read as this one finishing instantly.
    let _ = std::fs::remove_file(&sentinel);

    // The redirect comes first on the sentinel line on purpose: written the other
    // way round, `echo %ERRORLEVEL%>"file"` with an exit code of 0 parses as
    // `echo 0>"file"`, which cmd reads as redirecting handle 0 and writes nothing.
    let plural = if ids.len() == 1 { "" } else { "s" };
    let script = format!(
        "@echo off\r\n\
         title tinyget - upgrading {n} package{plural}\r\n\
         {cmd}\r\n\
         >\"{sentinel}\" echo %ERRORLEVEL%\r\n\
         echo.\r\n\
         echo Finished. Press any key to close this window.\r\n\
         pause > nul\r\n",
        n = ids.len(),
        cmd = upgrade_command(ids),
        sentinel = sentinel.display(),
    );

    let path = temp.join("tinyget-upgrade.cmd");
    std::fs::write(&path, script)?;

    let mut cmd = Command::new("cmd.exe");
    cmd.arg("/c").arg(&path);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NEW_CONSOLE);
    Ok(UpgradeRun {
        terminal: cmd.spawn()?,
        sentinel,
    })
}

// ---------------------------------------------------------------- parsing

/// winget identifiers are `Publisher.Package` style, or a Store product code.
/// Nothing else is ever passed to a command line, which is also why no quoting
/// is needed when building the upgrade script.
fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

/// Drop ANSI escape sequences and everything before the last carriage return,
/// which is how winget overwrites its progress spinner in place.
fn clean_line(line: &str) -> String {
    let line = match line.rfind('\r') {
        Some(i) => &line[i + 1..],
        None => line,
    };
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // CSI ... final-byte, or a two-character escape.
            match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c) {
                            break;
                        }
                    }
                }
                _ => continue,
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn char_width(c: char) -> usize {
    c.width().unwrap_or(0)
}

/// Display-width offsets at which each header label begins. These are the column
/// starts winget aligned the table to.
fn column_starts(header: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut w = 0usize;
    let mut prev_space = true;
    for c in header.chars() {
        if c != ' ' && prev_space {
            starts.push(w);
        }
        prev_space = c == ' ';
        w += char_width(c);
    }
    starts
}

/// Cut a row at the given display-width offsets. A character belongs to the
/// column its *first* cell falls in.
fn slice_by_columns(line: &str, starts: &[usize]) -> Vec<String> {
    let mut out = vec![String::new(); starts.len()];
    let mut w = 0usize;
    let mut col = 0usize;
    for c in line.chars() {
        while col + 1 < starts.len() && w >= starts[col + 1] {
            col += 1;
        }
        out[col].push(c);
        w += char_width(c);
    }
    out.into_iter().map(|s| s.trim().to_string()).collect()
}

/// Fallback for locales whose header labels contain spaces (English `Pin type`),
/// which would otherwise be read as an extra column. Only the leftmost field —
/// the package name — can contain spaces, so the rest can be taken from the right.
fn split_right(line: &str, cols: usize) -> Option<Vec<String>> {
    let mut rest = line.trim_end();
    let mut tail = Vec::with_capacity(cols - 1);
    for _ in 0..cols - 1 {
        let i = rest.rfind(char::is_whitespace)?;
        tail.push(rest[i..].trim().to_string());
        rest = rest[..i].trim_end();
    }
    if rest.is_empty() {
        return None;
    }
    tail.reverse();
    let mut fields = Vec::with_capacity(cols);
    fields.push(rest.to_string());
    fields.append(&mut tail);
    Some(fields)
}

/// Read a winget table into `cols` fields per row.
///
/// The table is located by its `-----` rule; the line above it is the header and
/// everything below it is data. Trailing summary lines ("7 upgrades available.")
/// sit directly under the rows with no blank line between, so rows are validated
/// by their identifier column rather than by position.
fn parse_table(out: &str, cols: usize) -> Vec<Vec<String>> {
    let lines: Vec<String> = out.lines().map(clean_line).collect();

    let Some(rule) = lines.iter().position(|l| {
        let t = l.trim();
        t.len() >= 3 && t.chars().all(|c| c == '-')
    }) else {
        return Vec::new();
    };

    let starts = if rule > 0 {
        column_starts(&lines[rule - 1])
    } else {
        Vec::new()
    };
    let by_column = starts.len() == cols;

    lines[rule + 1..]
        .iter()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            let fields = if by_column {
                slice_by_columns(l, &starts)
            } else {
                split_right(l, cols)?
            };
            // A real row has a package identifier in the second column. Summary
            // and warning lines never do, which is what keeps them out.
            (fields.len() == cols && !fields[0].is_empty() && is_valid_id(&fields[1]))
                .then_some(fields)
        })
        .collect()
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    /// Real output from a zh-CN machine: localized headers, CJK glyphs in the
    /// header occupying two cells each, and a summary line right under the rows.
    const UPGRADE_ZH: &str = "\
名称                                 ID                       版本         可用         源
----------------------------------------------------------------------------------------------
Comfy Desktop 1.0.39                 Comfy.ComfyUI-Desktop    1.0.39       1.0.46       winget
MicroDicom DICOM Viewer (64-bit)     MicroDicom.DICOMViewer   2026.1       2026.3       winget
Rustup: the Rust toolchain installer Rustlang.Rustup          1.29.0       1.29.1       winget
7 升级可用。";

    const PIN_ZH: &str = "\
名称                                 ID              版本   源     包钉类型
---------------------------------------------------------------------------
Rustup: the Rust toolchain installer Rustlang.Rustup 1.29.0 winget Pinning
WinRAR 7.22 (64-bit)                 RARLab.WinRAR   7.22.0 winget Blocking";

    /// English headers put a space inside "Pin type", which defeats header-based
    /// column detection and must fall through to the right-split path.
    const PIN_EN: &str = "\
Name                                 Id              Version Source Pin type
----------------------------------------------------------------------------
Rustup: the Rust toolchain installer Rustlang.Rustup 1.29.0  winget Pinning";

    #[test]
    fn parses_localized_upgrade_table() {
        let ups = parse_table(UPGRADE_ZH, 5);
        assert_eq!(ups.len(), 3, "summary line must not be read as a row");
        assert_eq!(ups[0][0], "Comfy Desktop 1.0.39");
        assert_eq!(ups[0][1], "Comfy.ComfyUI-Desktop");
        assert_eq!(ups[0][2], "1.0.39");
        assert_eq!(ups[0][3], "1.0.46");
        // The name column is only one space from the identifier on this row.
        assert_eq!(ups[2][0], "Rustup: the Rust toolchain installer");
        assert_eq!(ups[2][1], "Rustlang.Rustup");
    }

    #[test]
    fn parses_pin_table_in_either_language() {
        let zh = parse_table(PIN_ZH, 5);
        assert_eq!(zh.len(), 2);
        assert_eq!(PinKind::parse(&zh[0][4]), Some(PinKind::Pinning));
        assert_eq!(PinKind::parse(&zh[1][4]), Some(PinKind::Blocking));
        assert_eq!(zh[1][0], "WinRAR 7.22 (64-bit)");

        let en = parse_table(PIN_EN, 5);
        assert_eq!(en.len(), 1, "space in \"Pin type\" must fall back to right-split");
        assert_eq!(en[0][0], "Rustup: the Rust toolchain installer");
        assert_eq!(en[0][1], "Rustlang.Rustup");
        assert_eq!(en[0][4], "Pinning");
    }

    #[test]
    fn empty_pin_list_yields_no_rows() {
        assert!(parse_table("没有已配置包钉。", 5).is_empty());
        assert!(parse_table("", 5).is_empty());
    }

    #[test]
    fn strips_progress_spinner_and_ansi() {
        assert_eq!(clean_line("  \\\r  |\rReal text"), "Real text");
        assert_eq!(clean_line("\u{1b}[32mgreen\u{1b}[0m"), "green");
    }

    #[test]
    fn whole_selection_goes_out_as_one_command() {
        let ids = vec!["Rustlang.Rustup".to_string(), "RARLab.WinRAR".to_string()];
        assert_eq!(
            upgrade_command(&ids),
            "winget upgrade Rustlang.Rustup RARLab.WinRAR --exact --include-unknown"
        );
    }

    #[test]
    fn rejects_identifiers_that_are_not_identifiers() {
        assert!(is_valid_id("Microsoft.PowerToys"));
        assert!(is_valid_id("9NKSQGP7F2NH"));
        assert!(!is_valid_id("rm -rf &"));
        assert!(!is_valid_id("包钉"));
        assert!(!is_valid_id(""));
    }
}
