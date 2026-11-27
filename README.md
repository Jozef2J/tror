# Tror

**English** · [Русский](README.ru.md)

[![build](https://github.com/Jozef2J/tror/actions/workflows/build.yml/badge.svg)](https://github.com/Jozef2J/tror/actions/workflows/build.yml)
[![release](https://img.shields.io/github/v/release/Jozef2J/tror)](https://github.com/Jozef2J/tror/releases/latest)
[![downloads](https://img.shields.io/github/downloads/Jozef2J/tror/total)](https://github.com/Jozef2J/tror/releases)
[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

Tror is a set of tools for people who build and maintain websites. Its first app is **TrorPad**: a fast, compact code editor written in Rust. It is a single executable that opens huge files without lag and keeps everything you need for working on a live site close at hand.

- **Editor:** files of 50 MB and more, syntax highlighting for about 200 languages, completion, search and replace with regular expressions in a file or across a project, named macros, recovery of unsaved work after a crash, file comparison, code folding, bookmarks.
- **Visual HTML editor:** edit HTML, PHP and Twig pages as formatted text, with headings, lists, links, tables and images. PHP and Twig blocks stay untouched. Pasting from Word and Google Docs keeps the structure and drops colors and fonts; `.docx` files can be imported.
- **Servers:** SFTP, FTP and FTPS with saved profiles, several connections at once and transfers between them, drag and drop between the server, the project and the desktop, a download window with progress, pause and cancel, server-side search, folder sync, archives, and a raw protocol log.
- **Safe saving to servers:** before saving, TrorPad checks whether the file was changed on the server after you opened it and offers to compare or overwrite. The previous server version is kept as a local backup.
- **Console:** SSH, Telnet and Mosh sessions and a local terminal (PowerShell, bash, zsh) in tabs, with your own color schemes and fonts. OpenSSH and PuTTY (`.ppk`) keys are supported.
- **Databases:** MySQL/MariaDB, PostgreSQL and SQLite, including through an SSH tunnel and over TLS; search for a value across all tables without writing SQL; table structure (columns, indexes, foreign keys, CREATE statement) with adding, editing and deleting columns; query history; import of SQL dumps and CSV; export to CSV, JSON and SQL.
- **Interface:** every panel can be dragged, docked, detached or moved to its own window; colors, fonts and shortcuts are configurable. English and Russian interface.
- **Plugins:** WebAssembly plugins with an explicit permission model and a Rust SDK (see [`sdk/`](sdk/)).

## Download

Ready-made files are on the releases page: https://github.com/Jozef2J/tror/releases/latest

How to use TrorPad, with screenshots: https://jozef2j.github.io/tror/

- **Windows:** `trorpad-setup.exe` is the installer (shortcut and an "Open in TrorPad" entry in the file menu); `trorpad.exe` runs without installation. The files are not code-signed yet, so SmartScreen will ask: "More info" → "Run anyway".
- **macOS:** unpack `TrorPad-macos.zip` and move `TrorPad.app` to Applications. On the first launch, right-click → "Open", or run `xattr -dr com.apple.quarantine /Applications/TrorPad.app`.
- **Linux:** `chmod +x trorpad-linux-x86_64 && ./trorpad-linux-x86_64`.

With a package manager:

- **Scoop (Windows):** `scoop bucket add trorpad https://github.com/Jozef2J/tror` and `scoop install trorpad`.
- **Homebrew (macOS):** `brew tap jozef2j/tror https://github.com/Jozef2J/tror` and `brew install --cask trorpad`.
- **Homebrew (Linux):** after the same `brew tap`, `brew install trorpad`.

TrorPad tells you when a new version is out and installs it with one click. The check can be turned off in the settings. If you installed it with Scoop or Homebrew, update it there instead (`scoop update trorpad`, `brew upgrade trorpad`).

## Speed

Every build in CI runs `trorpad --bench` and publishes the numbers in the run summary. A build fails if an operation gets slower than its limit.

Version 0.14.0, Linux, 2 cores, software rendering without a GPU, release build, file already in the disk cache:

| What | Result |
| --- | --- |
| Start to first frame | 57–74 ms |
| Opening a 64 MB SQL dump (403k lines) | 133–180 ms |
| Opening a 78 MB single-line JSON | 142 ms |
| Frame while typing, scrolling and undoing in the 64 MB dump | average 1.7–2.1 ms, worst 15–19 ms |
| "Replace all", 403k replacements in 64 MB | 0.34 s |
| Saving 64 MB | 0.1 s |

Your own measurement: `TROR_PERF=1 trorpad file` prints the open time, time to first frame and frame statistics once a second.

## Building

Rust 1.95+ and a C compiler are required; on Linux and macOS also `perl` and `make` (OpenSSL is built from source).

```
cargo run --release -- path/to/file.php
cargo test
```

## Good to know

- Profile passwords are stored in the operating system's credential store (Windows Credential Manager, macOS Keychain, Secret Service on Linux). The portable mode can keep them in `connections.json` instead; there they are masked, not encrypted.
- A file is kept in memory in full; in files larger than 2 MB highlighting is approximate.
- Plugins only get the permissions you grant them. Plugins never receive passwords.
- TrorPad collects no telemetry. See the [privacy policy](PRIVACY.md).

## Contributing

Bug reports, ideas and pull requests are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md). Please report security problems privately as described in [SECURITY.md](SECURITY.md). Everyone in the project follows the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

TrorPad and the plugin SDK are licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Unless you explicitly state otherwise, any contribution you submit for inclusion in this project, as defined in the Apache-2.0 license, is dual-licensed as above, without any additional terms or conditions.

The name "TrorPad" and the logo are not covered by these licenses.

## Code signing policy

Windows releases are being prepared for free code signing by [SignPath.io](https://about.signpath.io/) with a certificate by [SignPath Foundation](https://signpath.org/). Team roles and rules: [CODE_SIGNING.md](CODE_SIGNING.md).

## Privacy

TrorPad is free software that works on your computer. It does not collect, send or sell any personal data. The full text is in [PRIVACY.md](PRIVACY.md).
