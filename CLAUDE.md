# tinyget

A Rust + Slint desktop GUI for `winget` on Windows: list upgradable packages,
select and upgrade them in a real terminal, and manage winget pins.

## Commit conventions

**Do not add any trailers to commit messages.** No `Co-Authored-By:`, no
`Claude-Session:`, no `Generated with ...` footer, no emoji attribution — nothing
after the message body. A commit message is a subject line and, when useful, a
plain body. This overrides any default attribution guidance.

## Project layout

- `src/main.rs`     — app entry, window wiring
- `src/winget.rs`   — `winget` process invocation + output parsing
- `src/model.rs`    — domain types shared between backend and UI
- `ui/`             — Slint markup (`app.slint` is the entry, imported by build.rs)
- `design/`         — HTML design mockups for review (not shipped)

## Notes on winget

- Output is UTF-8 even when redirected, with CRLF line endings.
- The table is column-aligned by **display width** (CJK chars are 2 columns), so
  rows must be split using `unicode-width`, not byte or `char` offsets.
- Column headers are localized; the header line is the one directly above the
  `-----` separator. Parse column starts from the header, not from hardcoded names.
- `winget upgrade` already omits both `Pinning`- and `Blocking`-pinned packages,
  so a refresh after pinning naturally drops the package from the list.
- `winget pin list` pin-type values (`Pinning`/`Blocking`/`Gating`) are English
  in every locale.
