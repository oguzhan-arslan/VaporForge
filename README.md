# VaporForge

<p align="center">
  <img src="icon.png" alt="VaporForge logo" width="128" />
</p>

A Windows desktop app for managing your Steam library — add non-Steam games, browse and apply artwork from SteamGridDB, and keep your shortcuts tidy.

## Features

- **Non-Steam Games** — scan install directories, add detected games as Steam shortcuts, edit names/paths/launch options, remove entries
- **Artwork Manager** — browse SteamGridDB covers, wide covers, backgrounds, logos and icons; click to apply directly to Steam's grid directory; works for both non-Steam shortcuts and installed Steam games
- **Auto-Artwork** — automatically fetches and applies artwork when you add new non-Steam games

## Requirements

- Windows 10/11 (64-bit)
- [Steam](https://store.steampowered.com/) installed
- A [SteamGridDB API key](https://www.steamgriddb.com/profile/preferences/api) to browse or apply artwork

## Installation

Download the latest `vaporforge-windows-x86_64.zip` from the [Releases](../../releases) page, extract, and run `vaporforge.exe`. No installer required.

## Usage

1. Launch `vaporforge.exe`
2. Open **Settings** and paste your SteamGridDB API key, then save
3. In **Non-Steam Games**, click **Scan for Games** to detect executables in your configured scan directories, then add them to Steam
4. In **Artwork Manager**, select a game from the list and click any image to apply it

> **Note:** Steam must be restarted for shortcut and artwork changes to take effect.

## Configuration

Config is stored at `%APPDATA%\VaporForge\config.toml` and created with defaults on first run. You can edit it directly or use the Settings tab.

| Key                        | Default               | Description                                |
| -------------------------- | --------------------- | ------------------------------------------ |
| `steam.user_id`            | _(auto)_              | Override the detected Steam user ID        |
| `scanner.scan_dirs`        | `[]`                  | Directories to scan for games              |
| `scanner.blocklist`        | common redist folders | Folder names to skip during scan           |
| `steamgriddb.api_key`      | _(required)_          | Your SteamGridDB API key                   |
| `steamgriddb.auto_artwork` | `true`                | Fetch artwork automatically on game add    |
| `steamgriddb.page_size`    | `25`                  | Images per page in Artwork Manager (10–50) |

## Building from Source

```sh
# Requires Rust stable (https://rustup.rs)
git clone git@github.com:oguzhan-arslan/VaporForge.git
cd VaporForge
cargo build --release
# Binary: target/release/vaporforge.exe
```

## Releasing

Push a version tag to trigger a Windows build and publish to GitHub Releases:

```sh
git tag v0.1.0
git push origin v0.1.0
```

## License

MIT
