# Contributing to TA Reborn

Thank you for helping. TA Reborn is an unofficial, clean-room reimplementation of Total
Annihilation in Rust. It plays a copy of the game the player owns, and its repository holds no
original game data. This guide sets out how changes are made and how pull requests are written,
so that every contribution is reviewed the same way.

## Ground rules

- **No original game data.** Never add files from Total Annihilation, its expansions or Supreme
  Commander to the repository, its tests, issues or pull requests: no textures, models, maps,
  sounds, scripts or pictures taken from them. The game reads them from the player's own copy.
- **Clean room.** Never decompile or disassemble the original executables. Formats are worked
  out from the data files, and what is known is written down in
  [docs/FORMATS.md](docs/FORMATS.md).
- **Licence.** TA Reborn is licensed under the GNU GPL, version 3 or any later version.
  Contributions are accepted under the same licence. New dependencies must be compatible with
  it: permissive licences, MPL-2.0, LGPL, GPL-3.0 or later, and the SIL Open Font License for
  fonts; not GPL-2.0-only, nothing non-commercial or proprietary.
- **The name.** The project is Total Annihilation Reborn, TA Reborn for short: an unofficial
  reimplementation. Please don't call it a remaster or a remake.

## Before you start

- For anything larger than a small fix, open an issue first (a bug report or an idea), so the
  approach is agreed before you spend time on it.
- Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the crates and the order of a tick, and
  [docs/FORMATS.md](docs/FORMATS.md) before changing how formats are read, how things are drawn
  or how the simulation plays.
- Build and run TA Reborn as [Build from source](README.md#build) in the README describes. Most
  integration tests need an installed copy of Total Annihilation; without one they skip.

## Making a change

- Branch from `main`, and keep one topic per pull request.
- Keep the diff to the change: no unrelated reformatting, renames or moves.
- Write as the code around you does: Rust 2024, dependencies declared in the workspace's
  `Cargo.toml`, comments that say why the code does what it does.
- **The simulation is deterministic.** `ta_sim` and `ta_ai` must reach the same result on every
  computer: no floating point, no `HashMap` iteration order, no wall clock in their state. Use
  `Fx`, `FxVec3`, the table trigonometry and `Rng`.
- **Interface text is translatable.** Pass it to the `ta_ui` widgets, or through `t` / `tf`
  (with `{name}` placeholders, never `format!` for a translated sentence), and mark constants
  with `n(..)`. Then run `ta-tools i18n lang` from the repository root.

### Versions

Some changes must bump a version, so that players on different builds are told so rather than
drifting apart in a game:

| What changed | Bump | Where |
|---|---|---|
| How a battle plays: anything `ta_sim` or `ta_ai` decides | `SIMULATION_VERSION` and `PROTOCOL` | `crates/ta_config/src/lib.rs`, `crates/ta_net/src/lib.rs` |
| What goes over the network (`Command` variants, other messages) | `PROTOCOL` | `crates/ta_net/src/lib.rs` |
| The layout of `GameSetup` (saved games and replays) | `RECORD_VERSION` | `crates/ta_config/src/lib.rs` |
| What the mod check reports | `SCHEMA_VERSION` | `crates/ta_data/src/validation.rs` |

Each bump adds a few words beside the constant saying what changed, as the earlier ones do.

### Tests

- Add a test that fails without your change and passes with it.
- Run `cargo test -p ta_formats -p ta_vfs -p ta_sim -p ta_net -p ta_render`, and the tests of
  the crates you changed (the game's: `cargo test --release -p ta_game`).
- A test that needs the original game must skip when no copy is found. The release builders
  have none, so no test may depend on finding one.

### Changelog

Add an entry to [CHANGELOG.md](CHANGELOG.md) under `## Unreleased` (add the heading above the
newest version if it is not there), in `### Added`, `### Changed` or `### Fixed`. Write it for
players: what they will notice, not how the code does it. When several pull requests add
entries at once, whichever is merged later takes `main` in and keeps them all.

## Pull requests

### Title

A short sentence in the imperative saying what the change does: "Fix edge scrolling after a
loading screen", "Add origin tags to the skirmish map list".

### Description

The pull request template has these parts. Please fill in each:

- **What this changes:** what was wrong or missing, and what the change does about it. For a
  bug, how to see it.
- **How it was checked:** the tests you ran and the tests you added, and what you tried in the
  game.
- **Compatibility:** the versions bumped (see [Versions](#versions)) and what that means for
  players (multiplayer between versions, saved games, replays), or "None".
- **The checklist.**

Keep the description to the facts. Pull requests are public: don't attach pictures or
recordings that show Total Annihilation's or Supreme Commander's art. Describe a visual change
in words, or show TA Reborn's own interface.

### Review

- A maintainer reviews every pull request and runs its tests, including those that need an
  installed copy of the game, which automated checks cannot run.
- Please leave **Allow edits by maintainers** on. A maintainer may push small follow-up commits
  to your branch (a version bump, a test, the changelog) rather than ask for another round.
- Once it is reviewed and its tests pass, a pull request is merged with a merge commit.

## Bugs, ideas and translations

- Use the [issue forms](https://github.com/Jared-woodruff/TA-Reborn/issues/new/choose): a bug
  report for something that does not work as it should, an idea for something new. Never attach
  files from the original game.
- The languages are TOML files in `lang/`; [tools/i18n/README.md](tools/i18n/README.md) says how
  to correct or add a translation.
