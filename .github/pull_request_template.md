## What this changes


## Checked

- [ ] `cargo test -p ta_formats -p ta_vfs -p ta_sim -p ta_net -p ta_render` passes
- [ ] The simulation stays deterministic (no floats, `HashMap` order or wall clock in `ta_sim` / `ta_ai`)
- [ ] New interface text goes through `t` / `tf`, and `ta-tools i18n lang` was run
- [ ] No original game data is added to the repository

Contributions are accepted under the project's licence: the GNU GPL, version 3 or any later version.
