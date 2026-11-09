TA Reborn - mods
====================

TA Reborn looks for mods in three places:

  1. This folder, next to ta-launcher.exe: mods for every user of this PC.
     When TA Reborn is installed in Program Files, adding files here needs
     administrator rights.
  2. %APPDATA%\TA Reborn\mods: your personal mods. The "Open mods folder"
     button on the launcher's Mods page opens (and creates) this folder.
  3. An extra folder of your choice, set on the launcher's Mods page.

A mod is either

  * a folder containing a mod.toml file plus its files and/or TA archives, or
  * a classic Total Annihilation archive (.ufo, .hpi, .ccx, .gp3, .gpf), or a folder
    without a mod.toml - existing community content works unchanged.

Example mod.toml:

  id = "my_mod"
  name = "My Mod"
  version = "1.0"
  authors = ["You"]
  description = "What it changes"
  depends = []      # ids of mods that must be enabled too; they load first
  load_after = []   # ids of mods to load before this one, when enabled

Enable mods and arrange their load order on the launcher's Mods page. Mods lower in
the load order override those above them; dependencies are loaded first automatically.
