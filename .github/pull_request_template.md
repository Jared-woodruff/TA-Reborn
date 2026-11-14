<!-- Please read the contributing guide first: https://github.com/Jared-woodruff/TA-Reborn/blob/main/CONTRIBUTING.md -->

## What this changes

<!-- What was wrong or missing, and what this does about it. For a bug, how to see it. -->


## How it was checked

<!-- The tests you ran and the tests you added, and what you tried in the game. -->


## Compatibility

<!-- The versions bumped (SIMULATION_VERSION, PROTOCOL, RECORD_VERSION, SCHEMA_VERSION) and what that means for players, or "None". -->


## Checklist

- [ ] `cargo test -p ta_formats -p ta_vfs -p ta_sim -p ta_net -p ta_render` passes, and a test covers this change
- [ ] The simulation stays deterministic (no floats, `HashMap` order or wall clock in `ta_sim` / `ta_ai`)
- [ ] Versions are bumped where the contributing guide asks
- [ ] `CHANGELOG.md` has an entry under `## Unreleased`
- [ ] New interface text goes through `t` / `tf`, and `ta-tools i18n lang` was run
- [ ] No original game data is added to the repository, or attached here

Contributions are accepted under the project's licence: the GNU GPL, version 3 or any later version.
