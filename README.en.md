# SVNGUITool

SVN commit browser. A cross-platform desktop GUI built with **Rust + Slint** to browse the commit history of SVN repositories/working copies, showing the files changed by each commit and the content diffs.

> Runtime dependencies vary by platform: **macOS uses the libsvn shared library** to access SVN; **Windows/Linux run `log`/`diff`/`info` via the `svn` command line**.

For the Chinese version, see [README.md](./README.md).

## Features

- Connect to an **SVN repository URL** or a **local working copy path**; automatically detects the repository root and HEAD revision.
- **Commit list**: revision / date / author / message in reverse-chronological order; click a row to see the changed files and content diff for that commit.
- **Changed files list**: files added/modified/deleted per commit, with the first file's diff loaded automatically.
- **Diff view**: syntax coloring (added/removed/header lines); view the diff of any revision in a file's full history.
- **Filter panel**: keyword, author, revision range, date range, and page size (entries fetched per batch); apply filters or refresh.
- **Infinite scroll**: automatically loads the next older batch when the list is scrolled to the end (or the last entry becomes visible), keeping the current window and scroll position intact.
- Auto-connect on launch: if the current directory is a working copy (contains `.svn`) use it directly; otherwise use the last remembered target.

## Requirements

- **macOS**: install `subversion` via Homebrew (including its dependencies `apr`, `apr-util`, `utf8proc`, `gettext`, `lz4`, `zlib`). The app accesses SVN through the libsvn shared library and does **not** need the `svn` command line. Before building, run the setup script to generate pkg-config files and set the environment (see below).
- **Linux / Windows**: the `svn` command-line client (`svn --version` must work and support `--xml` output). On Linux install the `subversion` package; on Windows install TortoiseSVN or SlikSVN and add it to PATH.
- **Rust toolchain**: stable (2021 edition) with `cargo`. Slint's compile-time generation requires network access to fetch crates.

## Build & Run

```bash
# macOS: first prepare libsvn's pkg-config and link environment
# (Homebrew's subversion ships no .pc files)
source scripts/macos-libsvn-env.sh

# Development mode
cargo run

# Release mode
cargo run --release
```

On macOS the build depends on the `subversion` crate (`subversion-sys` probes `libsvn_*` through pkg-config). The script generates `target/svn-pc/svn_*-1.pc` and exports `PKG_CONFIG_PATH` / `LIBRARY_PATH`; if a Homebrew dependency is missing, run `brew install subversion apr apr-util utf8proc gettext lz4 zlib` first. Linux/Windows do not need this script — just `cargo build`.

After launching, enter a repository URL or a local working copy path in the top-left box and click "Connect"; alternatively click "Open Local Project…" to pick a folder.

**Authentication**: the macOS backend shares credentials with the `svn` command line — it reads the on-disk cache under `~/.subversion/auth/` and the macOS Keychain (where the svn CLI stores passwords by default). The first time it accesses an existing Keychain item, macOS shows an authorization prompt ("SVNGUITool would like to use a password stored in your keychain"); after clicking Allow, connections work normally.

## Filtering

All filter fields are combined with **AND**; leave a field empty to disable it:

| Field     | Meaning                                                        |
|-----------|----------------------------------------------------------------|
| Keyword   | Text contained in commit messages (case-insensitive)           |
| Author    | Commit author (exact match)                                    |
| Rev range | A revision range such as `100:200`                             |
| Date range| A date range such as `2024-01-01:2024-01-31` or `2024-01-01:` (open-ended) |
| Page size | Number of commits fetched from SVN per batch, default `500`    |

"Refresh" reloads using the current conditions; when the list reaches the bottom and more entries exist, the next batch is appended automatically.

## Configuration

After the first successful connection the app remembers the target in `.svnguitool.json` in the current directory (this file is in `.gitignore`):

```json
{ "last_target": "https://example.com/svn/repo" }
```

## Tests

```bash
cargo test
```

Unit tests cover config read/write, diff coloring, filter parsing, and SVN XML parsing (the command-line backend on Linux/Windows); integration tests create a real SVN test repository in a temporary directory. On macOS the integration tests exercise the libsvn backend (via `file://` repositories), so running tests also needs `svn`/`svnadmin` (to create repositories) plus `source scripts/macos-libsvn-env.sh` first.

## Packaging (CI)

`.github/workflows/build.yml` builds runnable binaries for all three platforms on pushes to `master`/`main` (or `v*` tags, or manual trigger) and uploads them as GitHub Actions artifacts:

| Platform               | Artifact                              |
|------------------------|---------------------------------------|
| macOS (Apple Silicon)  | `svnguitool-macos-arm64.tar.gz`       |
| Linux (Ubuntu x86_64)  | `svnguitool-linux-x86_64.tar.gz`      |
| Windows (x86_64)       | `svnguitool-windows-x86_64.zip`       |

Push the repository to GitHub first (`git push -u origin main`). Runtime dependencies on the target machine: **macOS needs the Homebrew `subversion` shared library** (same dependencies as the build script, `brew install subversion`); **Linux/Windows need the `svn` command line**.

## Project Structure

```
ui/app.slint                  Slint UI definitions (components, layouts, callbacks)
src/main.rs                   Entry point; connect / commit list / load more / selection & diff view
src/svn.rs                    Backend dispatch + svn command-line backend (Linux/Windows, cfg-gated)
src/svn/libsvn.rs             libsvn backend (macOS)
scripts/macos-libsvn-env.sh   macOS build environment setup (generates svn pkg-config)
src/diff.rs                   Diff text coloring / classification
src/state.rs                  Filter condition parsing and matching
src/config.rs                 Config persistence (.svnguitool.json)
build.rs                      slint-build compiles ui/app.slint
.github/workflows             GitHub Actions for 3-platform packaging
```

## License

**GPLv3** — see [LICENSE](./LICENSE).