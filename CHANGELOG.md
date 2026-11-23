# Changelog

## Unreleased

### Added
- Taking control of a unit: riding along in one of your own ground units, hovercraft or ships (the
  perspective key, Q), press E to drive it yourself. W and S drive it ahead and astern (astern at
  half speed), A and D turn it, on the spot too, and the mouse aims its guns, which follow at their
  own turning speed; a click fires them. With the sight on an enemy you can see, the guns lift for
  its range and height as they would for an order to attack it. Its orders wait meanwhile, and E
  again, or leaving, lets it go back to them. Riding along is as it was: the unit goes on with its
  orders until you take control. Multiplayer and replays need everyone on this build.
- A Minimalist First-Person View (Options, INTERFACE): riding in a unit, only a sight with its
  range, a wireframe of the unit with its turret turning, its health in a bar along the top and a
  radar at the bottom, without the cockpit round them.

### Fixed
- The game speed keys (+ and -) could leave a battle unable to get back to normal speed: each
  press multiplied or divided the speed by 1.5, so once it was held at the slowest, 0.25, pressing
  back up gave 0.84 and then 1.27, never 1.0 again. The keys now step through set speeds (0.25,
  0.5, 0.75, 1, 1.5, 2, 3, 4, 6, 8 and 10), and from a speed between them (a battle started at
  the settings' 1.25, say) go to the next one their way. The GAME SPEED setting is the speed
  single-player battles start at; the keys change the battle under way.

## 1.0.9 — 2026-11-23

The game no longer crashes when zoomed all the way out on some maps, and a battle it does crash in
is kept as a replay. The Supreme Commander factions are painted at Forged Alliance's full
resolution, no longer turn white in the rain, and move as they work; their factories roll their
units out down their ramps, missile trucks deploy before they fire, anti-air takes on aircraft
alone, adjacency gives its bonuses, and engineering stations upgrade and work round them by
themselves. The computer players use the whole tech tree, strategic missiles and the Krogoth
included. With TA: Escalation its campaign missions play and its vertically launched missiles fly,
and wrecks no longer have a black square at their base.

### Added
- A battle the game crashes in is kept as a replay, as a battle finished or left is, and the crash
  report (`logs\crash.txt`) names its file: watched, it plays up to the moment of the crash, which
  helps tell what went wrong. Attach both to a report.
- Adjacency: the Supreme Commander factions' structures give the structures of their player beside
  them bonuses, as in Forged Alliance. Power generators cut what the factories, upgrading buildings
  and silos beside them pay to build, the upkeep of radars, fabricators and extractors, the energy
  guns' shots cost, and quicken artillery; mass extractors and fabricators cut what the factories
  beside them pay in metal; storage raises what the generators, extractors and fabricators beside it
  make (four mass storages round an extractor: half as much again). The bonuses are Forged
  Alliance's own, read from your copy of the game, by the size of the building that takes them; two
  buildings touch where their footprints meet along an edge. A link shows across the seam between
  two buildings that help each other (bright for the one under the pointer and the selected, and for
  one being placed, before it is put down), the hover card, the selection's details and the
  information page (F1) say what the buildings beside it give a building and how many it helps, and
  the figures over it what it makes and uses with them. What a building was built or upgraded for
  comes back when it is given up unfinished or taken apart, not its price. The computer players put
  their power generators beside their factories and mass storage beside their upgraded extractors.
  Any mod can use it (`[ADJACENCY]`, `AdjacencySize`: see
  [docs/MODDING.md](docs/MODDING.md#adjacency)); make the factions' mod again (UPDATE) to have it.

### Fixed
- The game could crash ("min > max, or either was NaN") when the modern camera was zoomed all the
  way out, often while zooming in and out quickly. It depended on the map's size and the window's:
  at 1920 × 1080, about one map in fourteen, among them Acid Foursome, Aqua Verdigris, Lipar Pass
  and Emerald Seamount. Zoomed out over most of the map the view keeps to it, and the margin it
  keeps from the map's edges, rounded, could come out a hair past the map's middle, so that the far
  edge's margin came before the near one's. It no longer can.
- Five other places that could crash the same way no longer can: the companion screen between
  about 800 and 935 points wide, a walking unit whose thigh is some eighty times as long as its
  shin, a map's start markers on a picture smaller than they are, the command deck on a narrow
  window, and a mod's units whose storage takes a player's below nothing.
- The Kennel and the Hive (the UEF's and the Cybran's engineering stations) offered no upgrade and
  stood idle unless told what to do. Their next tiers, which Forged Alliance makes only by
  upgrading (the tech 3 Kennel, the Hives of tech 2 and 3), were not made at all, nor were the
  Cybran shield generator's four upgrades: they are now, so UPGRADE is offered as in Forged
  Alliance. Left alone, an engineering station now works round it by itself: it helps build what
  is going up within its reach (a frame, a factory's unit, a building upgrading), then mends the
  damaged units of its side and its allies there, then reclaims the wrecks there; any order you
  give it comes first, and waiting for work it is no idle builder. The Kennel works with its own
  beams, as far round it as its drone would go (its drone is not made yet). Any mod can use it
  (`DefaultMissionType=AssistPatrol`: see
  [docs/MODDING.md](docs/MODDING.md#buildings-that-work-round-them-by-themselves)); make the
  factions' mod again (UPDATE) to have it.
- A building upgrading in place into a larger one (the Cybran tech 1 land factory's tech 2, a row
  longer) grew into whatever stood beside it; it now upgrades only with room for what it becomes.
- The Supreme Commander factions' buildings and units looked pixelated and frosty: their paint was
  halved to 512 texels a side, so at a normal battle zoom a factory was drawn from a quarter of
  Forged Alliance's texels, blown up into soft DXT blocks. Their paint is now as large as Forged
  Alliance paints it (most buildings', the commanders' and the large units' 1024 texels a side,
  held with HD surfaces; at the Low level the graphics card holds what it did), smaller textures
  are enlarged keeping their edges sharp, the colour maps' blocks keep their colours, and up close
  the colour is read sharp rather than blurred between texels. In Reborn's art style the paint is
  finished as TA's is: the pale blue-white the UEF's buildings wore and the white specks along
  every panel line (Forged Alliance's sheen, painted into the paint and laid over it again) are
  gone, the greys are gunmetal, the panels matte with their edges worn bright, the Aeon's and the
  Seraphim's chrome polished, the Seraphim's markings glowing in their player's colour. Enhanced
  Classic keeps Forged Alliance's look, at its full resolution. Make the factions' mod again
  (UPDATE) to have it: it takes about 680 MB (440 MB before).
- The Supreme Commander factions' buildings and units turned a glossy, frosty white in the rain,
  the more once a shower had passed and the sky cleared while they were still wet, and the most
  at dawn and dusk (the UEF's land factory more than its air factory: it goes by how much relief
  a model's normal map has). The film of rain water that mirrors the sky over wet models lay on
  their normal maps' relief, which Forged Alliance bakes from finer models, so it ran white along
  every bevel and panel line, and seen from above their sides mirrored the bright haze. On these
  models it now lies on their faces, mirrors the ground below the horizon and is laid on their
  dark paint about as thinly as it shows on TA's: wet, they are darker with a sheen, as TA's units
  are, in either art style. TA's own units look exactly as they did.
- The Supreme Commander factions' buildings move as they work, as in Forged Alliance: extractors
  pump while they extract, stopping where they are when switched off; power generators, mass
  fabricators, radars and shield generators spin, swing and slide their parts; factories' arms
  work as they build (the Cybran's, and the UEF's air and naval ones), the Seraphim's pods turn;
  hatches, silos and antennae open once a building is up. An upgrade in place plays
  the building's own upgrade animation as it goes, a T1 extractor or factory turning into the next
  as the work is done, until the new building stands in its place. Make the factions' mod again
  (UPDATE) to have it. Any mod can use the same: a model's clips named `active`, `open` and
  `upgrade` play as its unit works, opens and upgrades (see
  [docs/MODDING.md](docs/MODDING.md#models-that-move-by-clips-of-their-own)).
- Campaign missions with TA: Escalation on were lost the moment they began: Escalation's copies of
  TA's missions keep their units under a name of their own engine's (`[unitsE]`), which was not
  read, so nothing was placed and every mission failed to "all your units destroyed" half a second
  in. A mission's units are now read wherever its file keeps them, and its log says what decided
  it. A mission the mods that are on cannot play (a total conversion lacking the units it gives
  you or your Commander, a mod's mission that gives you no units: three of Total Mayhem's own) is
  marked MISSION NOT PLAYABLE on the campaign page with the reason, and is not started.
- Some wrecks had a large black square at their base: the ground plate TA's editor laid under
  23 of TA's wrecks (the Sea Serpents', the EMP launcher's, the Thud's...), unflagged in their
  models, was drawn from above, and darkened the ground round them. It is left out, as TA's
  camera never saw it, in both art styles.
- The Supreme Commander factions' buildings that float on water (the air factories, radars,
  anti-air, missile defences and others) drew their pontoons, which Forged Alliance hides on land,
  as dark slabs under the buildings and their wrecks; they are left out, and no longer widen the
  footprints of four of them. Make the factions' mod again (UPDATE) to have it.
- Computer players use the whole tech tree in long games. With the Supreme Commander factions they
  build strategic missile launchers with their tech 3 engineers, making those engineers when they
  have none, and fire them at the enemy's bases: their tactical launchers had taken the launcher's
  place, and no strategic missile was ever built. The first missile is saved for as a project,
  with the power for it built while the launcher goes up, and a plane looks at the enemy's bases
  when a launcher holds a missile and knows of nothing to strike. The tactical launchers are built
  beside them now and strike the enemy's defences and most valuable buildings in reach, and
  tactical missile defences go up once an enemy launcher is seen. TA's Core fields the Krogoth:
  constructors help build it in the Gantry, which had built it alone in half an hour of work, the
  other factories hold back part of their production meanwhile, and the Gantry goes on at the
  army's size limit. Once tech 3 factories stand, production leans to tech 3, and more factories
  are upgraded to it as the economy grows. The Tech persona does the most of this and the Rush
  persona the least; Easy still builds no silos, long-range guns or experimentals.
- The Supreme Commander factions' factories built their units sinking into the build platform as
  they were done, then drove them out through the factory's front wall. A unit is now built on the
  platform facing the way it will leave by, and rolls off as in Forged Alliance: along the factory's
  roll-off lane nearest its rally point, on the platform and down its ramp (the land factories' to
  either side, the naval factories' along their docks, the quantum gateways' to the front or the
  back); the long transports are built along their air factories' platforms, not through the
  factory. Any mod's factory can name its ways out the same (`[EXITS]`, and a unit the way it faces
  as it is built, `BuildHeading`: see [docs/MODDING.md](docs/MODDING.md#factories-ways-out)); TA's
  own factories send their units out of the front as ever.
- The Supreme Commander factions' missile trucks (the Flapjack, the Spearhead), missile launchers,
  silos and heavy artillery fired without deploying. Each weapon that unpacks in Forged Alliance
  now plays its unpack animation first (the launcher raised, the silo's doors opened, the
  artillery's feet set down) and fires once it is done, packing up again a moment after its target
  is gone; the heavy artillery, the Scathis and the Aeon sniper bot hold still while deployed and
  pack up before they move off. A script can hold its unit still so (`IMMOBILE`, see
  [docs/MODDING.md](docs/MODDING.md#scripts-and-ta-31s-questions)).
- The Supreme Commander factions' anti-air towers (and their other anti-air, ships' and
  interceptors' guns) fired on land and sea units, and on the ground, when told to attack them.
  They now take on aircraft alone, by order too. Any mod's weapon can be kept to its targets the
  same (`OnlyTargetCategory<n>`, see [docs/MODDING.md](docs/MODDING.md#units-weapons-and-features));
  TA's own units keep their rules. Make the factions' mod again (UPDATE) to have these three.
- TA: Escalation's vertically launched missiles were invisible and blew up at their launchers: those
  of its missile batteries, the Raptor, the battleships, mobile fortresses and hover battleships,
  the Deluge, Hammerhead, Arbitrator, Talos and others. They leave their tubes faster than their top
  speed and slow down as they fly, and were held to that top speed (which the mod writes as a
  negative number) from the moment they were launched, so they hung inside their launchers until
  their flight time was up and went off there; the Abyssal's and the Vortex's strategic missiles
  went off on the submarine itself. A missile now leaves at its start speed and gathers or loses
  speed towards its top speed, as its weapon's file describes, so these climb out of their
  launchers, turn over and fly at their targets. The C.O.R.E.'s rockets left from far off the map
  and were gone at once, only their launch flashes to be seen: its script moves its launch points
  away as it aims and brings the one firing back as it fires. A shot now leaves from where the
  weapon's fire script puts its firing piece, so the C.O.R.E.'s rockets fly from its pods. TA's own
  units fire as before, except that the few whose fire script kicks the barrel back at once (the
  Guardian, the Punisher, the Sumo, flak and a few more) fire from the kicked-back barrel, where the
  flash shows.

## 1.0.8 — 2026-11-18

Supreme Commander: Forged Alliance's four factions play in TA Reborn, made from your own copy of
the game, as an experimental feature (Settings, LAUNCHER: ENABLE EXPERIMENTAL FEATURES): their
units and buildings with their own models, paint, weapons, walks and deaths, factories upgrading
in place and commanders taking enhancements. The computer players clear wrecks out of their way,
build anti-nukes, silos, long-range guns and experimentals, and each can be given a difficulty and
a persona in skirmish. Units turn with their factory's pad, the Commander mends itself, wrecks keep
their heading and give metal alone, and the modern build menus stand each picture on the ground
where its unit goes.

### Added
- Upgrading in place: the Supreme Commander factions' factories upgrade into the next tier, their
  extractors into richer ones, their radars and sonars into better ones, as in Forged Alliance.
  Select one and press UPGRADE (Alt+U, or a factory's first build picture, the building it
  becomes; with Shift, the step after the one under way): it builds its upgrade on itself with its
  own build power, and builders guarding it help; it pays as construction does, a factory making
  nothing meanwhile; Stop calls it off and gives its metal back. Done, the new building stands in its place, keeping its queue, route, selection and control
  groups. The computer players climb the tiers by upgrading their factories, and upgrade their
  extractors. Any mod can use it (`UpgradesTo`, see [docs/MODDING.md](docs/MODDING.md#units-that-upgrade-in-place));
  make the factions' mod again (UPDATE) to have it.
- Commander enhancements: the Supreme Commander factions' commanders and support commanders
  install Forged Alliance's enhancements on themselves: the engineering suites (tech 2 and 3
  buildings on their menu, more build power and health), the gun upgrades (farther, faster or
  harder), the missile launchers, more armour and mending, sensors, and the Cybran's stealth and
  cloak. The build menu's ENHANCE tab lists them, a slot each, with their price and what they need
  first (the classic interface: an ENHANCE button); a click installs one, built on the commander
  with its own build power as an upgrade is, a right-click takes one out. The information page
  (F1) shows them, those installed marked, and the range overlays show what an enhanced unit
  reaches. The computer players install engineering suites and a gun upgrade. Any mod can use them (`[ENHANCEMENTS]`, see
  [docs/MODDING.md](docs/MODDING.md#enhancements)); make the factions' mod again (UPDATE) to have
  them. Shields, teleporting, drones, resource allocation and the other enhancements TA Reborn
  does not play yet are left out.
- Models can be painted beyond TA's palette and textures: a 3DO with a surface beside it
  (`objects3d/<name>.surface`) takes texture coordinates and smooth normals for its vertices and
  materials for its polygons, each a colour map, a specular map (gloss, mirroring, lights and where
  the player's colour shows) and a normal map, DDS textures drawn at 512 × 512. A material's paint
  is read the first time a model of it is drawn, and a surface can name a lighter model to draw far
  off. The mod check reads surfaces and DDS textures. See
  [docs/MODDING.md](docs/MODDING.md#models-with-surfaces-of-their-own).
- Groundwork for playing Supreme Commander: Forged Alliance's four factions: their units' meshes are
  made into TA models with surfaces, painted with the game's own textures in each player's colour,
  their build icons into TA's build pictures, and their blueprints into TA units with Forged
  Alliance's own numbers, scripts that aim their turrets and build, and the four factions'
  sides, build menus and wrecks; the computer players build and fight with them
  (docs/SUPCOM_UNITS.md).
- Supreme Commander: Forged Alliance's four factions in TA Reborn. With the game installed (GOG's or
  Steam's, or in a folder you choose), the Mods page's SUPREME COMMANDER… (or the FACTIONS tab of
  the Maps page's) counts the UEF's, the Cybran's, the Aeon's and the Seraphim's units and makes
  those of the factions you choose into the mod Supreme Commander factions, turned on: a side each
  to play in skirmish, against each other, the Arm and the Core. Their own sounds come later.
- A mod adds a side without replacing the game's side data: `gamedata/sidedata_<name>.tdf` adds
  its sides and computer players' build lists to the game's. See
  [docs/MODDING.md](docs/MODDING.md#sides-of-a-mods-own).
- Models can move by animations of their own (`objects3d/<name>.anims`): a walk played in step
  with the ground the unit covers, and deaths played where it falls, its wreck lying as the death
  ends. The Supreme Commander factions' Commanders, support Commanders, bots and walking
  experimentals now walk as in their game, and 93 of their units die as they do there (make the
  factions again in the launcher: it offers UPDATE). See
  [docs/MODDING.md](docs/MODDING.md#models-that-move-by-clips-of-their-own).
- The Supreme Commander factions' nukes fly at TA's nukes' speed at least: they left their silo
  at a crawl and took a minute to cross a big map. Their wrecks, their units' own models, are
  drawn burnt out, sooted and without their player's colour, as TA's wrecks are painted.
- A mod's units carry up to eight weapons, `Weapon1` to `Weapon8`: the fourth to the eighth aim
  and fire through the numbered script functions (`AimWeapon4`, `QueryWeapon4`, `FireWeapon4`...),
  each with its own bad targets (`BadTargetCategory4`...). The Supreme Commander factions' 23 units
  with more than three weapons now fire them all: the Fatboy, the CZAR, the Monkeylord and the
  battleships among them (make the factions again in the launcher: it offers UPDATE). See
  [docs/MODDING.md](docs/MODDING.md#units-weapons-and-features).
- **RANGES** (Interface, in the game's options and the launcher's settings) chooses what the
  selected units, the building being placed and the unit under the pointer show of their reach,
  on the battlefield and the minimap: everything (weapons, sight, radar, sonar, jammers and
  missile defence), their radar, sonar and jammers only, or nothing. Alt+O steps through them in
  battle (the key can be changed under Controls).
- The launcher's Settings have **ENABLE EXPERIMENTAL FEATURES** (under LAUNCHER, off by default).
  Features still being tested ship with it off, so testers can try them before they become a
  normal part of the game. With it on, they show in the launcher, each with an EXPERIMENTAL tag.
  The Supreme Commander import is the first: its SUPREME COMMANDER… buttons on the Maps and Mods
  pages, and the Mods page's UPDATE… for what it made, show only with the setting on. Maps and
  factions imported before stay installed and play as any mod, and an import already under way
  can still be watched to its end.
- The modern interface's build menus tell where each unit goes by the ground behind its picture,
  as Supreme Commander: Forged Alliance's do: grass for land, water for the sea, the two split on
  the diagonal for hovercraft, amphibians and what can stand on land or out in the water, sky for
  aircraft (the plants that make them stand on grass). Each tab, and a factory's list, now puts land first, then
  what goes on both, then the sea's, then aircraft, keeping TA's order within each; so do the
  radial menu's rings of buildings and units. The classic interface keeps TA's pages and pictures,
  and the Supreme Commander factions' pictures keep their own game's frames.
- Computer players have personas, as Forged Alliance's had, and play the whole game. Each
  computer seat of a skirmish or a lobby now shows its difficulty plainly, EASY, NORMAL or HARD
  under the seat, and a persona beside it: Balanced, Rush (attacks early and often), Turtle
  (towers, heavy defences, long-range guns), Tech (up the tech tree fast: its factories' and
  extractors' upgrades and its commander's engineering suites sooner, a Rush's later), Air, Naval
  or Random;
  saved games, replays and multiplayer keep the choice, and the battle names it ("AI (Hard,
  Rush)"). On a long game they now put up anti-nuclear defences once they find a silo, nuclear
  silos, heavy defences, long-range guns that shell the bases they have found (the Big Bertha,
  the Intimidator), experimentals (the Core's Krogoth Gantry, Forged Alliance's) and more of the
  top tier as their economy grows (more advanced factories, upgraded where a factory upgrades,
  as the Supreme Commander factions' do); Easy stays gentle. And they clear wreckage out of their way:
  a pass shut by wrecks (Last Stand's, or a skirmish's) is reclaimed by their constructors, the
  Commander too when it is near and safe, or broken up by their army's guns, and a scenario's
  waves break up the wrecks that shut their way instead of piling up behind them.

### Changed
- The modern interface's DETAILS tell what the selection makes and uses of metal and energy a
  second, as the classic interface's unit bar does: a metal extractor the metal it mines, a power
  plant its energy, the Commander both, a factory at work what it spends; several selected, all of
  them together. A factory with a queue or a constructor with plans shows them under its queue.
- While a building is being placed, the cells of the ground it will take show as in TA, under its
  ghost and under each site of a row or block being dragged out: each cell green where the building
  can stand and red where it cannot (a tree, a rock, another building, water, a slope too steep),
  blue and orange with the colour-blind marks, in both interfaces and both cameras, laid over the
  ground and the sea, and gone once the building is put down. No grid is drawn over the cells or
  the ground round them. The squares round the plans that Shift shows follow the ground the same
  way.
- Over the minimap the cursor says what an order given there would do: over an enemy's mark that
  the selection would attack with a right-click, or that the order waiting for its click would aim
  at, it turns into the attack sight (the air-strike sight for bombers), as over the battlefield,
  and the mark is lit and ringed in red round the sight; the same for a unit of ours to guard,
  repair or load.

### Fixed
- Selecting a unit that reaches far, such as TA: Escalation's Shaper (radar 13,440, jammer
  2,688) or a mod's long-range radar, slowed the game down, froze it for a while or crashed it,
  worse the closer the camera: its rims were laid out far beyond the screen and dashed all along,
  up to hundreds of thousands of dashes a frame. Only the part of a rim in view is drawn
  now, whatever its reach (the Shaper's ranges from 13 ms of a frame to 0.1 ms), and the rims follow
  the ground more closely.
- In TA: Escalation, construction planes landed on a flying carrier (or any unit resting on a
  bigger one) were hard to click: the carrier took the click. A click on the plane now selects
  the plane; anywhere else on the carrier, the carrier.
- TA: Escalation's repair tower upgrades, which its air repair pads make ("Automatically repairs
  all nearby units"), did nothing. They now mend the damaged units of their side and its allies in
  their reach by themselves, the nearest first, aircraft too, whether their pad took them on or
  they stand beside it.
- Turning COMPANION SCREEN off in the options did nothing while the companion screen was open: it
  now closes at once, and turning it back on in a battle opens it again on your second display.
- The Supreme Commander factions play as Forged Alliance's (make the factions again in the
  launcher: it offers UPDATE). Their tech 2 and 3 factories, which the game makes only by
  upgrading the one below, are on their engineers' menus, so every unit can now be built, and the
  computer players climb the tiers to tech 3 (TA's own sides, whose factories stop at tech 2,
  play as before). Nukes, anti-nukes and tactical missiles are built one at a time at their price
  (a nuke 12,000 metal and 1,350,000 energy over five minutes); a nuke flies high and comes down
  on its target, a tactical missile at TA's tactical missiles' height. Missile defences take what
  they do in the game: an anti-nuke the nukes, a tactical missile defence or a ship's point
  defence the tactical missiles, and their tactical missiles no longer set off the nuclear
  launch warnings. Anti-air homes on its aircraft, flak bursts over its area, and aircraft gather
  speed, turn and attack as theirs do (only gunships hover to attack). Every weapon deals the
  game's own damage a second, its racks, muzzles, beams, damage over time and splitting shells
  counted. Upkeep is paid only for what TA Reborn plays (no shields yet: the Fatboy's costs
  nothing), weapons and stealth a unit gets only from an enhancement or a toggle are left out,
  hydrocarbon plants stand on the maps' vents, amphibians reach the deepest sea bed, ships
  stand on footprints of their size and leave wrecks that block nothing, the Fire Beetle blows
  itself up among its enemies, engineering stations help build and air staging repairs
  aircraft. Ground weapons take on aircraft last, Commanders keep their guns still while they
  build, walks loop without a jump and the units mend themselves at the game's rate. For mods:
  TA's `interceptor` and `targetable` are now read as classes, as the Spring engine reads them,
  so a mod's missile defences can take different missiles, and a missile defence fires once a
  missile is within its range (TA's own all say 1 and reach across its maps: its nukes and
  anti-nukes play as before; see [docs/MODDING.md](docs/MODDING.md#units-weapons-and-features)).
- The Targeting Facility (Arm and Core, "Automatic Radar Targeting") did nothing. While one of
  yours is finished and switched on, your units set to fire at will now shoot at what your radar
  shows of the enemy as well as at what they see: a Big Bertha shells a radar contact across the
  map. It works as a radar does: short of the energy it uses (1,250 a second), it stops. Units set
  to hold fire (a Big Bertha as it is built) still hold it, and an ally's facility directs its own
  units only.
- Submarines fired at the floating missile towers, the Defender - NS and the Stinger, without ever
  hitting them: their torpedoes ran under the towers. Torpedoes now hit whatever reaches into the
  water where it meets the water (ships, towers and other buildings on the water, units wading or
  standing on the sea bed), climb to their target's depth as they leave the tube instead of
  running into the sea bed rising to the shallows, and run as far as their weapon's range (most
  gave out after 300 px, the advanced torpedo launchers' long before their 600 and 650).
- Skeeters, Searchers and every other unit whose weapons cannot go under the water shot at
  underwater metal extractors (and other underwater buildings) whose tops stood out of shallow
  water, and an order to attack something wholly under the water had them follow it for nothing.
  What is wholly under the water, underwater buildings wherever they stand, is now left to
  torpedoes and depth charges: other weapons never fire at it, and an attack order on it ends at
  once. Like submarines, an enemy's underwater buildings are seen only where sonar reaches them. A
  destroyer sent at a submarine closes to its depth charges' reach instead of stopping at
  its guns'. Units partly out of the water, such as an amphibious K-bot in the shallows, are shot
  where they stand out of it and hit, where the shot used to go off on the water in front of them.
- On an attack order, the Skeeter's and the Searcher's missiles followed the order on to boats
  they are meant to leave for last. A second or third weapon whose own target list puts the
  ordered target last (these missiles, the Panther's and the missile ships' anti-air missiles, the
  Phoenix's and the Hurricane's anti-air lasers, the Doomsday Machine's light laser, the
  destroyers' and cruisers' depth charges and others) now keeps choosing its own targets, aircraft
  first (submarines for depth charges), else the nearest enemy, while the main weapon fires as
  ordered; a Skeeter sent at a boat closes to its laser's reach to do so.
- Orders given while the battle was paused did not show until it went on. Routes, waypoints and
  targets, a constructor's plans, a factory's queue and the route its units take, standing orders
  and switches now show the moment they are given, in multiplayer too, and the interface goes by
  them (a second building queued after the first, Shift taking one back). The battle still takes
  them at its next tick, so replays, saved games and every player's game play them as before.
- The modern interface's ON/OFF button said ON for a selection of metal makers (or radar towers,
  or solar collectors) some on and some off. It now says ON/OFF, its lamp half lit, as the classic
  interface shows both, and a press turns them all off when the first is on and all on otherwise,
  as the classic button and the X key do.
- A unit being built in a factory stood still while the factory's pad turned under it. As in TA,
  it now turns with the pad (the plants, labs and aircraft plants spin it while they build) and
  is finished facing the way the pad left it.
- The Commander now heals itself over time as in TA: 100 hit points every 27 seconds (its
  `HealTime`), up to full health, at no cost.
- Wrecks all lay lined up north to south, whichever way their units had faced. A wreck now lies
  as its unit faced when it died, in every view.
- Reclaiming a wreck gives metal only, as in TA, even where a mod's wreck says it holds energy;
  rocks and trees give what their files say.
- The Supreme Commander factions, and any mod's models with paint or animations of their own: the
  classic interface showed their sides no metal or energy bars, figures or logo (a side that lays
  out nothing takes the boxes of a side in the same panels, else the first side's); a wreck left
  in its death's last pose dropped into the ground as it appeared, and the dying unit turned into
  its dark wreck in one frame (it now fades into it); a texture claiming endless mip levels froze
  the game for seconds, and paint that is not 512 × 512 block-compressed was decoded on the frame,
  stalling it (it is now made on workers, the model in its colours on average meanwhile); the
  paint grows in steps of 32 materials, is let go when the next battle starts, and where the
  graphics card has no room left the rest is drawn in its colours on average; a surface that does
  not fit its model leaves it drawn as TA draws it, without seams along every triangle; a broken
  animation file is told in the log and a clip that does not fit is left out alone; a far model
  no longer swaps back and forth at the edge of its distance; the cockpit shows every weapon, those
  alike as one ("Gauss Cannon x4"); the map editor draws their wrecks in their paint. The mod check
  now reads DDS textures' mip levels and texels, notes paint that is made again as it is read, and
  checks far models' pieces, that surfaces name DDS textures, and each clip.
- Supreme Commander maps look and play more as in their game (make them again: the launcher
  offers to update maps made before). The ground is lit as the game's own terrain shader lights
  it: slopes facing away from the sun were painted black, and maps under a coloured sun took its
  tint (Seraphim Glaciers came out cream, not lavender); the colours of maps twice as wide as
  they are deep are matched to the game's picture of them. The cities' fences and street lights
  no longer stand as palm trees across Flooded Strip Mine and Haven Reef, nor their buses, cars
  and docks as rocks worth 150 metal, and the Cybran's wall segments are walls. A destroyed rock
  no longer cracks into a larger, richer one. Every mass point now takes an extractor, a script's
  call commented out no longer cuts a map down, and a map with a single start is refused. Shared
  tiles meet more smoothly, and the largest maps are made in about a minute with under 1 GB of
  memory (Crossfire Canal took 4 minutes and 4.4 GB). A game whose folders are reached through a
  junction or a link is found.
- The launcher's Supreme Commander dialog, opened again from the Mods page, showed its MAPS tab's
  maps as dark squares, still marked as they were (IMPORTED after REMOVE MAP PACK). Each tab now
  looks for the game again whenever it is shown after the dialog was closed, the factions' too
  while they are being made; and once the map pack is removed, the game's maps are chosen again
  rather than IMPORT 0 MAPS.
- A Supreme Commander import was out of sight once its dialog was closed or showed the other tab.
  It now has its place in the launcher's footer, how far it has got, a click opening the dialog
  at its tab; the dialog opens at the tab whose import is under way; its CLOSE says the import goes
  on; and the other tab says when it has ended.
- UPDATE NOW and INSTALL NOW did not wait for a Supreme Commander import: Setup closed the launcher
  in the middle of one, losing its work. The update now waits for it, as for a mod being installed.
- The Supreme Commander maps and factions made by an older TA Reborn were marked UPDATE only in
  their dialog. The Mods page marks them too (their card, the UPDATES filter and the count on the
  rail), with UPDATE… to make them again.
- A factions import cut short (the launcher killed, the power gone) left about 400 MB behind for
  good; it is now cleared at the next start, as a maps import's is.
- A maps import that could not write a map (the disk full) threw away every map it had made. It
  now checks there is room on the disk before it starts, says what went wrong against that map,
  and installs the maps made so far; and it keeps the maps already in the pack without copying
  them (up to 1.2 GB), so it needs little more room than the maps it makes. The factions' import
  checks the room first too.
- Importing replaced another mod of the same id as the map pack or the factions' mod, its files
  deleted, without a word: it now asks first, as installing a package over a mod does. The
  imports find the pack and the factions' mod wherever the launcher finds mods (one kept in the
  extra mods folder is made again there, and none is made behind a copy in TA Reborn's program
  folder), and the factions' mod can no longer be removed or replaced while it is being made, nor
  checked while an import replaces it.
- CHANGE FOLDER… with a folder that does not hold Forged Alliance said so in a notice hidden under
  the dialog. The folder is now looked through without holding the window up and what it holds is
  said in the dialog; the original Supreme Commander is told apart from Forged Alliance (taken
  first where a folder holds both); and the folder chosen, or the copy switched to (GOG's or
  Steam's), is kept for the next start.
- In the Supreme Commander dialog: STOP AFTER THIS MAP shows it was pressed, the maps' import says
  what it is doing (reading the games' files, making a map, installing them) and STOP is greyed
  while it installs, as the factions' is, with why; why IMPORT cannot be pressed is written beside
  it; the dialog is as tall on either tab, its tabs stay in place, and the maps' list ends between
  two rows instead of leaving an empty band; Shift+Tab and a gamepad go back up the list; one map
  or one faction is told as one, and none chosen as none; the units that could not be made are
  named; the "Still to come" line is up to date; and an error from the system is said once.
- The Supreme Commander factions look as they do in their game (make the factions again in the
  launcher: it offers UPDATE). Their paint was drawn about half as bright as their game lights it,
  the UEF's and the Cybran's units dark as if unpainted, and a selected Commander was a pale haze
  the ground seemed to show through; their metal now has its game's broad sheen, and the player's
  colour is as bright as on TA's units. Their build pictures are their game's own build icons in
  the modern interface too (a mass extractor's was a dark shape, drawn before its paint was read),
  without the white edge each had; the classic interface shows their build menus (it showed none),
  laid out as the pages of the side whose panels they wear. Their buildings stand on footprints as
  wide as what of them meets the ground (a point defence's was a quarter of its turret's: on a
  hillside it ran into the ground beside it, and neighbours were built into it) and on slopes no
  steeper than TA's buildings take.

## 1.0.7 — 2026-11-16

Mods can present themselves in the launcher with a banner, an animated preview, an icon and
screenshots, theme the battle's interface in colours and pictures of their own, and name the mods
they cannot be played with and the oldest TA Reborn they need. Trees and shores no longer flicker
for a few seconds after the camera moves, and in TA's own view metal extractors stand whole on the
Supreme Commander maps' deposits. A turret turns to an enemy in reach rather than holding fire on
one just out of it, and every page of the launcher had a QA pass.

### Added
- Mods present themselves in the launcher's library with pictures of their own, named in their
  `mod.toml`'s new `[media]`: a banner (their card, and the top of their page), an animated GIF of
  the mod in play (played in the banner on their page, and on their card while the pointer rests
  on it), an icon beside their name, and screenshots on their page that open on the whole window.
  A mod installed from a file or a folder now looks as one from the catalogue does. `tags` find a
  mod in the library and show on its page, and `[links]` (and `source`) are links there. The
  installer keeps a mod's `media` folder and the pictures its manifest names; the mod check tells
  under PRESENTATION what would not show as meant (notes: a mod's badge is how it plays). See
  [docs/MODDING.md](docs/MODDING.md#presenting-a-mod-in-the-launcher) and the
  [example mod](docs/examples/example-mod).
- Mods can theme the battle's interface: a `[theme]` in their `mod.toml` draws the HUD, modern
  and classic, in colours of their own (an accent, text, panels, the metal and energy bars, health
  and warnings, a side's own over the rest), with pictures of their own behind the modern HUD's
  plates and top bar and a font for its titles (fitted to the HUD's lines); the classic HUD keeps
  TA's art (a mod's interface files change it) and takes the theme's lettering. The battle's menus,
  dialogs and result pages, the loading screens and the cockpit keep TA Reborn's own look. The
  last mod in the load order with a theme gives it; MODS' INTERFACE THEMES in the Interface options
  turns them off. The launcher shows a
  mod's theme on its page, and the mod check tells what would not draw as meant. Without a theme
  the HUD is drawn as before, to the pixel. See
  [docs/MODDING.md](docs/MODDING.md#theming-the-interface).
- A mod's `mod.toml` can name the mods it must not be on with (`conflicts`) and the oldest TA
  Reborn it plays in (`requires_reborn`): with a conflicting mod on, or in an older TA Reborn, the
  load order does not resolve, and the launcher and the game say why and what to do (its card
  shows a red lamp when it needs a newer TA Reborn).
- `ta-tools mod-new <folder>` makes a new mod's folder: a `mod.toml` naming it, every other key
  written out and commented, a `media` folder and a read-me of what goes where.
- The mod check reads a mod's `mod.toml` as TA Reborn reads it and tells what is read otherwise
  than written: a key it does not know (with the key likely meant, `depends` for `depend`), a key
  written below a `[table]` line, which TOML takes for that table's (a `requires_reborn` after
  `[media]` is never read), a value of the wrong kind, one text where a list is wanted. A key that
  decides what the game mounts, misplaced or mistyped, is a warning: the mod is mounted as if it
  were not given.
- When the load order does not resolve, LOAD ORDER offers to turn off what stops it: a mod that
  needs one not installed or a newer TA Reborn, or either of two that cannot be on together.

### Changed
- A `mod.toml` is read more forgivingly: a value of the wrong kind in what only presents the mod
  (its version, description, authors, tags, pictures, links or theme) is left out and the rest
  read, where the whole mod was left out; one text where a list is wanted is a list of that one
  (`tags = "units"`, `depends = "ship-hulls"`). What decides whether it is mounted is read as
  strictly as before.
- Installing a mod, the launcher and `ta-tools mod-install` keep its `mod.toml` as written,
  comments and keys this TA Reborn does not know too, changing only what the install sets; the map
  editor, saving a map as a mod again, does the same and keeps the map's other authors.

### Fixed
- Trees standing side by side in a wood traded places for a second or two after every pan or zoom
  (the perspective camera): drawn as far from the eye as each other, which was in front came down to
  rounding while the camera came to rest. Each tree's painting now keeps its place in front of or
  behind its neighbours, and the camera comes to rest a moment after a move instead of creeping on
  for seconds.
- A battle started from the menu (a new skirmish, a saved game or replay opened there, a lesson,
  a multiplayer game) and the map editor kept the lens of the menu's film: its field of view,
  its roll and a near plane a few units in front of the camera. Zoomed out over the battlefield
  that left the depth too coarse to tell things apart, and for a few seconds after every pan or
  zoom the trees in a wood traded places and the water along the shores streaked white (a battle
  started straight away, or a saved game resumed from the launcher, never had it). Every battle now
  starts with the game's own camera.
- In the classic camera, what lies on the ground (metal deposits laid as features, scorch marks,
  painted shadows) was drawn over the lower part of what stood on it: a metal extractor or moho
  mine built on one of the Supreme Commander maps' deposits showed only its arms, and a tank on a
  scorch mark sank under it. There, as in TA, the ground and what lies on it are drawn first.
- Installing a mod failed when its manifest named a file in one of TA's folders in other capitals
  than the disk's (`Fonts/hud.ttf` for `fonts/hud.ttf`), when its content folder was called `Base`
  or `Reborn`, when the package's file name made an id the launcher refuses (longer than 64
  letters, `base.zip`) though the package names its own, and when it held only a `mod.toml` that
  blanks archives of the original game. A manifest that does not read is named by its place in the
  package, with what TOML found.
- The map editor saving a map as a mod: a save cut off between its two steps (by an antivirus, a
  crash) could leave the map in a hidden backup folder; the launcher puts it back at its start,
  and the editor at its next save. A `mod.toml` edited by hand that did not read made the editor
  say the folder was not its own: it now says what does not read.
- The launcher checked a mod again at every start when the check stopped at its own limits (a very
  large mod) or found nothing of it to read: such a report is now kept for the same content, as a
  complete one is (a cancelled check never is). The check of the original game without the mod is
  no longer redone when a mod it depends on changes only its name, description, pictures or
  theme.
- A mod's check that could not mount the mod with what it needs said so in English only, with
  advice that did not fit: mods needing each other in a circle, a mod that cannot be on with
  another it is mounted with, and one that needs a newer TA Reborn each have words of their own,
  in the player's language, and what to do, in the launcher and in `ta-tools validate-mod` alike.
- `ta-tools validate-mod` lists its findings in the launcher's order.
- Names listed together on the Mods page ("by …", "requires …") are joined as the player's
  language joins them.
- A turret whose target had moved just out of its reach kept it, held fire and passed over
  another enemy it could hit; it now turns to one in reach, and fires on the first again the moment
  it comes back. Units on attack-move or patrol kept stopping short of a target their weapons could
  not reach yet: they now close until they can fire. An attack order keeps its target as before.
- The launcher, after a QA pass over every page:
  - RESTORE DEFAULTS under Graphics, and RESET ALL SETTINGS, wiped the unit skins chosen on the
    Skins page and said the graphics card had no ray tracing: they keep them now.
  - A damaged profile of another commander stopped every profile from being edited, created or
    renamed, with no way to remove it: it is listed apart, with OPEN FOLDER and MOVE TO RECYCLE
    BIN, and holds nothing else up. A damaged service record (the career file) kept PLAY from
    starting while the Play page said all was well: the Play page and the profile now say so, and
    START A FRESH CAREER sets the damaged file aside (it is kept) and begins anew.
  - Saving an edited skin could fail every time ("destination already exists"); every visit to
    the Skins page read the whole game again; the pattern tiles were redrawn while a slider was
    dragged, holding up the window; the colour picker's hex field could not be typed into.
    Importing a skin you already have offers REPLACE or KEEP BOTH.
  - A game that restarted itself with a saved battle's mods and then crashed was never
    reported; a game that could not start showed a developer's instruction and was not logged;
    the launcher's music kept it redrawing four times a second; "UPDATED TO" showed on a first
    install when the game had been started first.
  - At large interface scales the page list hid SETTINGS, BENCHMARK and ABOUT, and at window
    heights of 640 to 680 points (a 1080p laptop at 150%) PLAY covered ABOUT: every page is in
    reach now. Turning SEASONAL EFFECTS off in the game's Options was undone when the game closed.
    Saving the settings no longer holds up the window, and a rare crash of the footer's "saved"
    timer is gone.
  - The Supreme Commander import's dialog copied every map picture twice a frame (40 ms a frame
    with 450 maps), chose every map not imported after an import, and its failures went unsaid
    once it was closed; closing the launcher while maps were being looked through was refused. A
    map whose terrain cannot be read is no longer offered to play.
  - Screen readers hear the notices, the Play page's state and headings, and the editors' and
    pickers' controls; the gamepad's first press on the Play page reaches PLAY, not the website;
    shortcuts name the keys as the platform and the language do (Ctrl, Strg, ⌘).
  - Messages that were in English only, or raw technical errors, are in the player's language.

## 1.0.6 — 2026-11-15

NVIDIA's graphics driver has more room on the game's main thread, after crashes on some cards, and
with orders on the right button a click no longer turns the 3D view. Orders on the minimap aim at
the units marked there, Shift and a click on a queued reclaim or move takes it back, and the economy
shows by how much you outspend your income. Reclaiming, the D-gun, fire at will and capturing work
more as they do in TA, and edge scrolling keeps working once a battle has loaded.

### Added
- Shift and a click on an order already queued takes it back, as a building put down again on
  its own plan does: a reclaim of a wreck, rock, tree or unit already queued for reclaiming, or a
  move to within a waypoint's mark of one already set (a group's moves there go together).
- A contributing guide, [CONTRIBUTING.md](CONTRIBUTING.md): the project's ground rules, which
  version to bump for which change, tests, the changelog, and how a pull request is written and
  reviewed. The pull request template follows it.
- Orders on the minimap aim at the units marked on it. A right-click on an enemy's mark, in sight
  or on radar, attacks it, and an armed command takes its own kind of unit there: Attack and
  Capture an enemy, Guard, Repair, Load and Reclaim one of yours. A right-click among your own
  units' marks still moves there.

### Fixed
- With a store run dry, the metal or energy used showed only what came in (+8.4 / -8.4), however
  much more was being asked for. The use shown is now what builders, upkeep, cloaks and weapons
  ask for (+8.4 / -20), so you see by how much you outspend your income.
- Edge scrolling stopped working once a battle had loaded, or the profile or career window had
  been open, until the cursor left the window and came back, which in full screen on a single
  display it never could. Any move of the cursor now brings it back. It no longer scrolls while a
  drag is carried out of the window, or while a slider or a scroll bar is dragged to the edge of
  the screen.
- Units and defences on fire at will fired at enemies in the fog of war that radar or sonar
  alone showed, which made long-range guns stronger than their cost. Fire at will, and return
  fire, now takes on only what your side sees; radar and sonar contacts are fired on when you
  order it. (Iron Rain's guns, which a radar post spots for, still shell what it shows.)
- The Commanders' D-gun passed through almost everything: it harmed only what its shot went
  right through, and a shot aimed down at a unit or at the ground went on under the ground
  beyond it. It now runs on along the ground or the water to the end of its flight, destroying
  every unit within its area of effect of its way (sparing those beside the Commander as it
  fires) and the trees, rocks and wrecks in its path; trees catch fire as often as TA's weapon
  file says.
- With orders on the right button, a right-click in the 3D view gave its order and turned the
  view a little as well, whenever the pointer shook. The view now turns only once a right-drag
  has gone further than a click's few pixels, and such a drag never gives an order.
- Reclaiming paid its metal a little at a time as the work went, so its income came at once and
  reclaiming was worth more than in TA. Now, as in TA, a wreck, rock or tree gives all of its
  metal and energy when it is reclaimed whole, and one of your own units its metal when it is
  gone.
- Capturing the most costly units took far longer than meant: a Commander needed 51 minutes to
  capture the Core's Galactic Gate. Capturing a unit still takes four times its cost in work,
  but never more than two minutes of a Commander's.
- The game could crash in the graphics driver on some NVIDIA cards ("the graphics driver
  (nvoglv64.dll) failed", the main thread out of stack): the driver does its work on the
  game's own threads, and Windows gives a program's main thread little room. The game's main
  thread and the one that loads a battle now have 16 MiB.

## 1.0.5 — 2026-11-14

Supreme Commander: Forged Alliance's maps can now be played in skirmish: the launcher makes them
into TA Reborn maps from your own copy of the game, GOG's or Steam's. Every map now shows where it
comes from, in skirmish, the lobby and the launcher, and the lists can show one origin at a time.
Mods install, update and are removed more safely, their compatibility report opens on an overview,
and Total Mayhem's, TA: Escalation's and TA: The Final Frontier's builders and build menus behave
more as in their own engines.

### Added
- Supreme Commander: Forged Alliance's maps in TA Reborn. With the game installed (GOG's or Steam's,
  or in a folder you choose), the Maps page's SUPREME COMMANDER… lists its skirmish maps, and your
  own downloaded and Forged Alliance Forever maps, with the game's pictures of them, and makes the
  ones you choose into TA Reborn maps on your computer, from your copy of the game. Their land, the
  ground as the game paints it, their cliffs (which still stop tanks however low the land became),
  their forests (passable, as in the game) and rocks, their mass points (metal deposits), hydrocarbon
  points (geothermal vents) and starting places come with them, in a map pack shown in the map
  library and in skirmish. The computer players play them. See
  [docs/SUPCOM_MAPS.md](docs/SUPCOM_MAPS.md).
- `ta-tools supcom` makes Supreme Commander's maps into TA's, with pictures of each beside the
  game's own.
- Where each map comes from, in skirmish and in the multiplayer lobby: every map in the list is
  tagged TA, CC (Core Contingency), BT (Battle Tactics), REBORN, SUPCOM (made from Supreme
  Commander's) or MOD, its origin told in full under the pointer. The list shows the maps of one
  origin, or the expansions together, and puts them in order by name, by origin (each origin under
  its name) or by players; the search finds origins too, and a map at random is one of those shown.
- The launcher's map library names the game's own maps by where they come from (Total
  Annihilation, Core Contingency, Battle Tactics), for its SOURCE filter and order, and marks the
  maps made from Supreme Commander's with a SUPREME COMMANDER ribbon and a note in their details.
- With more than one copy of Supreme Commander: Forged Alliance installed (GOG's and Steam's),
  the import shows each, to choose the one it reads; they make the same maps.
- A mod's compatibility report now opens on an OVERVIEW: what the check found, in a word and a
  sentence, with its errors, warnings and notes, each a way into those findings; what the mod
  holds; the findings to fix first; and what the project saw of it in play. A mod's details in
  the library show the check in brief, under COMPATIBILITY.
- The mod check tells how TA Reborn plays what a mod uses of TA's later engines, and of what TA
  left to its own: the computer players' build lists (TA 3.1's `[CANBUILD]`), side panels laid
  out for a 768-pixel screen, build buttons sharing a place or taking several, TA 3.1's script
  questions about units, weapon IDs past 255, and sides and campaigns of its own, all played;
  TA's own AI profiles and the programs a mod brings, not used. The copied report and `ta-tools
  validate-mod` give it too.
- A guide to mods, [docs/MODDING.md](docs/MODDING.md): for players, getting mods, the load order,
  updates, the compatibility report, saved games and multiplayer, where things live and what to do
  when something goes wrong; for mod makers, the layout TA Reborn reads, the `mod.toml` manifest,
  how mods are layered, what TA Reborn plays of TA's later engines, and checking and testing a mod.

### Changed
- In the report's FINDINGS, three or more findings of one check with one fix are one card: the
  first three, the rest a click away, and the fix they share. Each finding takes less room, its
  severity beside its words and HOW TO FIX beside where it is.

### Fixed
- The mod check said units made for the computer players alone had no build picture, though no
  menu shows them (TA: The Final Frontier's AI units), and called their entries in the computer
  players' lists build options. Those lists are now told as the computer players'.
- On Windows, once the Maps page had been opened, updating, replacing or removing a mod kept in an
  archive failed for the rest of the session ("Access is denied"): the map library kept the mods'
  archives open. The launcher now lets go of them before an install or a removal, and stops a check
  reading a mod before it is removed.
- An update stopped while the version it replaced was being deleted (the launcher closed, the power
  cut) could leave that version half deleted under the name kept for it, and every later install of
  the mod was refused. A version replaced is now moved out of the way before it is deleted, and what
  is left of one is cleared at the next start.
- On Linux and macOS, REMOVE deleted a mod (or a skin, a map pack, a profile) for good, though the
  launcher said it went to the Recycle Bin. It now goes to the trash; where it cannot, it stays
  where it was.
- Without 7-Zip, a package named in letters outside Windows' code page (Cyrillic on a Western
  Windows, say), or unpacked in a Windows user folder named so, did not unpack with Windows' own
  `tar`. It now does.
- A package of more than 100,000 files was refused only after minutes of unpacking. It is now
  refused from its list of contents, before anything is unpacked.
- A package whose own `mod.toml` names a mod installed under another name took that mod's place
  without asking, its files deleted for good. The launcher now asks first, as for a package named
  like the installed mod.
- Files marked read-only, as files from a CD often are, could not be written to the disk before an
  update took the installed copy's place, nor replaced by the next update. The mark is now cleared.
- A mod with the id `base` or `reborn` was taken for the original game's content or TA Reborn's
  own (the map library listed its maps as theirs). Such an id is now refused, with what to do.
- On Linux, a manifest named `Mod.toml` or `MOD.TOML` was not read, so the mod had another id there
  and lost its dependencies. It is now found in any case.
- On Linux and macOS, opening the launcher a second time could delete a mod download the first one
  had been making for more than ten minutes. A working folder in use is now left alone.
- The digest of a mod's files kept by saved games and replays covered its documents too, so an
  edited read-me file made them say the mod's files had changed. It now covers the files the game
  reads; records made before still know the same copy.
- A package's folder of documents (`Docs/`, `Documentation/`) was left out of the installed mod.
  Its documents now go into the mod's `docs/` with the rest, and `.nfo` and `.docx` files count as
  documents.
- A mod added from Total Annihilation's folder, whose version is unknown, had UPDATE in its menu
  but not in its details. The details now offer it too.
- `ta-tools mod-install` installed ids longer than the launcher's 64 letters, and passed over a
  package's `mod.toml` that did not read. It now refuses both, as the launcher does. A manifest's
  `[rename]` entries must map a folder of the mod's own to one of TA's folders, or the manifest
  does not read.
- Total Mayhem's construction units (kbots, vehicles, hovercraft and ships) stood still at a
  building ordered while their nano arm was still out from the job before, and never built it:
  it had to be ordered again once the arm had folded away, four or five seconds after a job.
  They now build it at once. As in Total Annihilation, a builder is ready to build when its
  script says so, also between jobs.
- TA: Escalation's Guardian was pictured on a tall red column in the build menu, on its card and
  on its page of facts: the skirt its model reaches 73 high under the ground with, to close the
  gap under it on a slope. Pictures of units that stand on land now leave out what is under the
  ground, as a battle's ground hides it. The skins page's previews too.
- The building being placed, and those queued, were shown as their models are without their
  scripts, so a mod's building could look like another one until it was built: Total Mayhem's
  metal extractors carried the gun turret and the parts of their upgrade, which their scripts
  hide. A building being placed is now shown as it will stand once built, its script having set
  it up: Total Mayhem's long-range guns (Big Bertha, Vulcan, Dora, Intimidator, Buzzsaw and
  Nator) with their guns turned the way the variant chosen faces.
- Buildings that build only their own upgrade, such as TA: Escalation's Guardian and Total
  Mayhem's geothermal plants, were listed as factories: on the build menu's FACTORY tab and in
  the FACTORIES group of the forces panel, with their idle count. They are now with the defences
  or the economy, as what they are.
- Total Mayhem's constructors listed the same building several times. The mod lays one
  place of its build pages out for several variants of a building: its long-range guns (Big
  Bertha, Vulcan, Dora, Intimidator, Buzzsaw, Nator) come in four, one for each way the gun
  faces, round one picture, the side of the picture clicked being the way it faces; its hover
  plants in two halves, for land and for the sea. Each variant took a place of its own, the
  same picture again and again, and in the classic interface they pushed the buildings after
  them (a metal maker among them) onto other pages. Such a place is now one picture, as in the
  mod's own engine: the zone pointed at stands out, the picture in the modern interface turns
  to show that variant (its gun facing that way), and a click builds it. The classic
  interface also keeps the mod's big buttons over several places (a Krogoth's) as they are.
- Mods' build menus offered units made for their computer players alone, beside the ones they
  stand for. TA: The Final Frontier's command cruiser offered 26 buildings for its 11: an AI
  solar collector, extractor, labs and plants beside each of its own, and ground labs the space
  mod gives only its AI; Total Mayhem's and TA: Escalation's menus had such units too (95 and
  27 in all). These lists (the 3.1 patch's `[CANBUILD]` in `sidedata.tdf`) were TA's computer
  players', never on a player's menu: a menu is now the builder's own pages and the units the
  mod adds to them, as in TA, and the computer players still build their own units. A
  player's constructors on a duty build only what their menus offer.
- TA: Escalation's minelayer submarines and spies did not offer their mines, and its decoy
  commander lacked the floating hover plant: their pages mark those buttons as build buttons
  but not as production, the other way to mark one. A build button marked either way is now
  on the menu.

## 1.0.4 — 2026-11-13

The launcher now works with a gamepad, the Steam Deck's controls included, and the first start on
a Steam Deck sets the game up for it. TA: Escalation, Total Mayhem and the other mods made for
TA's later engines now load however they were installed, show their whole build menus, and their
upgrades work. Damaged aircraft repair at pads by themselves, buildings clear the plants under
them, and the mod check judges mods as the game reads them, in the interface's language.

### Added
- The mod check's findings, and what to do about each, are shown in the interface's language,
  as is the rest of the mod's report. The copied report stays in English, for sharing with a
  mod's author.
- The launcher now works with a gamepad, the Steam Deck's controls included: the d-pad or left
  stick moves between controls, A uses them, B goes back, the bumpers turn the pages, the right
  stick moves the pointer and the triggers scroll. X opens a card's menu, Y the search, and View
  the list of keys and buttons, which now shows the controller's too.
- On Linux, `install.sh` also adds *TA Reborn (game only)* to the applications menu: added to
  Steam in place of the launcher, it starts the game straight away with the launcher's settings.
- The Steam Deck is recognised on the first start (by Steam's `SteamDeck=1`, the Deck's
  firmware, SteamOS or its graphics), and the game is set up for it: full screen at 1280 × 800
  with vertical sync, the interface at 125%, as large as the battle's panels allow on its
  screen, the launcher maximised, and the Medium graphics level, which suits the Deck's
  graphics. Whatever you change afterwards is kept.
- In SteamOS's Game Mode (and anywhere else inside gamescope) the launcher now fills the screen,
  drawn pixel for pixel instead of a window scaled up. On the desktop it stays a window, as you
  left it.
- On Linux the launcher now finds Total Annihilation where Heroic installed it, wherever that is
  (another folder, an SD card), as Heroic's own record says; and where GOG's installer or the CD,
  run through Steam as a non-Steam game, put it in that game's Proton prefix, in any Steam
  library. In Wine prefixes it now also looks in `Program Files`, and finds the CD's folders
  whatever the case of their names.
- The mod check now tells when a mod's scripts ask the engine for values TA Reborn does not
  answer, as mods made for later patches of TA's engine can: what such a script does with them
  may not happen as the mod intends. Total Mayhem and TA: Escalation ask for none it lacks.

### Changed
- On Linux, the Steam Deck and macOS, the interface's text is now set in Sofia Sans, a typeface
  close to Windows' Bahnschrift that comes with TA Reborn, instead of a thin fallback font. It
  has Cyrillic and Greek, so Russian and Ukrainian menus read in one typeface, in the letter
  forms those languages use.
- In the game's menus and dialogs, a control the d-pad moves to is scrolled into view, and the
  d-pad now leaves a text field (such as the options' search) instead of moving its cursor.
- On Linux the first start no longer sets every computer to the Medium graphics level. The
  game now picks the level from the graphics card it opens: High on a dedicated card, Medium on
  recent integrated graphics and the Steam Deck, Low on older integrated graphics.
- Choosing AUTO for the graphics level in the options now picks the level again the next time
  the game starts, and leaves the window as it is. Before, the window went back to full screen
  at the screen's size too.

### Fixed
- A building put over ferns, fronds or small stones (on lush and archipelago maps, among
  others) left them standing in it, their 3D models sticking out of the finished building. A
  building now clears its footprint of the plants, stones and scars lying loose there as its
  construction starts, as a mission's buildings do when they are placed. Wreckage keeps its
  metal under it, and the sea floor's anemones and corals stay.
- The mod check's report counted the whole game as the mod's: "6,128 files · 549 units" and "3
  parse/read failures" were the original game's with the mod's. It now gives the mod's own
  files, units, maps and sounds, and says how many files it was checked with.
- A mod changed by hand (a file edited in one of its folders) kept its old badge until the
  launcher was restarted or the mod was checked again by hand. Visiting the Mods page now
  notices the change and checks the mod again.
- The launcher's messages about a mod check that could not run, and the load order's problems
  (a manifest that does not read, two mods of one name in a folder, mods that need each other
  too deeply), were in English whatever the interface's language.
- With the classic interface, TA: Escalation, Total Mayhem and the other mods made for the
  patched engines of later years showed only four of a builder's buildings, the same two
  repeated down the panel, with no page arrows and no orders: most of the tech tree could not
  be reached. Their menus were made for a taller screen (768 pixels), with twelve buildings to
  a page and the orders under them, and were squeezed onto TA's 480. The side panel is now laid
  out on the screen a mod's menus were made for: every page with all twelve places, the arrows
  to turn them and the orders, as in the mods' own engines. TA's own menus are unchanged.
- TA: Escalation, Total Mayhem and the other mods made for patched engines did not load when
  dropped into a mods folder by hand (an archive alone, or a folder without the `mod.toml` the
  launcher writes): their units, menus and weapons are in folders of their own (`unitse`,
  `guim`, `weaponm`...), which were not read. TA: Escalation played as the original game, Total
  Mayhem without its menus or weapons. Such a mod's own folders are now found from what it holds,
  as the launcher's installer finds them.
- Mods installed into Total Annihilation's own folder, as their own installers had you do,
  were loaded as part of the original game: TA: Escalation's models and scripts took the place
  of TA's own units' while Escalation itself did not load, and Total Mayhem's units came
  without their menus or weapons. They are now left out of the original game, and the
  launcher's Mods page lists each one under IN YOUR TOTAL ANNIHILATION FOLDER: ADD AS A MOD
  adds it to your mods, its files linked from there (copied when the mods folder is on
  another drive) and the game's folder left as it is.
- Units' scripts asking TA 3.1's questions about other units (the range of unit ids, a unit's own id,
  another's player, how much of it is left to build, whether it is an ally) were told 0 to each.
  TA: Escalation asks them in most of its scripts and Total Mayhem for its upgrades (a geothermal
  plant finds its upgrade module by them), so what those scripts do never happened. They are now
  answered as TA 3.1 answered them. TA's own scripts never ask them: the original game plays as
  before.
- The upgrades of Total Mayhem and TA: Escalation can now be built, and they work. Total Mayhem's
  geothermal plants and moho mines, and TA: Escalation's towers, reactors, geothermal plants,
  Aegis and repair pads, offer an upgrade in their menus: some offered nothing to build, and the
  others started the upgrade and never finished it. Each now builds its upgrade and takes it on,
  with what it gives, as in the mods' own engines: Total Mayhem's geothermal plant then makes 400
  energy a second instead of 250 and is armoured, TA: Escalation's fusion reactors make 250 more.
  TA: Escalation's advanced and experimental labs make a Level 2 or Level 3 upgrade that stays on
  the lab's pad, and the units the lab makes after it come out at that level.
- Total Mayhem's moho mines and TA: Escalation's repair pads are switched on when they are
  built, as their files say. TA: Escalation's repair pads were switched off as soon as they were
  built, and repaired nothing.
- Buildings whose menus offer only their own upgrade, such as TA: Escalation's towers and
  reactors, were counted among your idle factories (IDLE FACTORIES on the top bar, and the
  coach's advice). They no longer are.
- Total Mayhem's fusion reactors made twice their energy, 2,100 a second instead of 1,050. A
  unit's energy use now counts only while it is switched on, as in Total Annihilation; TA's own
  units are unchanged.
- Total Mayhem's mines now cloak themselves while they wait, as their scripts ask.
- Core Contingency's mines now start cloaked, as their files say: the enemy sees one only when
  a unit comes within a few paces of it. Before, they could be seen from anywhere.
- On the Skins page, at some window widths and interface sizes, "Use the same skin for both
  sides" was squeezed into a column a letter wide, or the search and the skins below it went
  out of reach. It now goes on a line of its own when it does not fit beside the skins.
- Damaged aircraft never used repair pads unless sent to one. Aircraft on patrol, attack-move
  or an attack now break off when worn below two-thirds of their health, fly to the nearest
  free Air Repair Pad or carrier, however far, land on it and are repaired, then carry on with
  what they were doing. A pad takes one aircraft at a time: the others keep on with their
  mission until it is free, and an idle aircraft parked on a pad makes room for a damaged one.
  A new order calls the trip off. Idle aircraft, and aircraft only moved somewhere, still don't
  go looking for a pad.
- Repair pads also repaired aircraft parked on the ground beside them, and went on working
  switched off. They now repair only the aircraft landed on them, and a pad switched off takes
  no aircraft and repairs none.
- Thunder and Hurricane bombers sent to a repair pad flew on past it, landed on the ground
  beyond and were never repaired. They now come back and land on it.
- The Skins page's turning 3D preview stopped on its second frame; it turns again.
- The skin preview now draws on the same graphics card as the game. On a computer with
  integrated graphics beside its graphics card it drew on the integrated graphics, whose driver
  could crash the launcher.
- When the skin preview cannot draw, the Skins page no longer opens a new graphics device for
  every picture; Retry preview tries again.
- If the launcher crashes in a graphics driver, it now saves `launcher-crash.txt` naming the
  driver and says so, as the game does.
- Opening a skin whose scale has more than two decimals no longer marks it as changed.
- The mod check now judges the mods' upgrades as the game plays them. An upgrade module's
  footprint of none is no longer reported as a footprint the game changes, and a building that
  builds only its own upgrade modules is no longer told it lacks a nano-lathe. Menus count only
  for units that can build (Total Mayhem's and TA: Escalation's silos list missile buttons for
  their computer players, which build nothing), and buildings that build without TA's builder
  flag, such as Total Mayhem's geothermal plants, have their menus checked.
- A map whose terrain is missing is now a warning, not an error: the game and the launcher leave
  such a map out of their lists, and nothing else is affected. Total Mayhem lists 9 maps it does
  not ship, and was shown with errors for them.
- When one file names the same missing thing several times (a menu file naming a unit the mod
  does not ship, a builder at a time), the mod check now tells it once, listing who names it,
  and a model's missing textures are told together.
- The original game's own slips no longer count against a mod. A finding that is there without
  the mod, the same in the same place, is shown as inherited even when the mod uses what it is
  about or carries a copy of its file: Total Mayhem and TA: Escalation carry copies of TA's list
  of unit sounds, and the sounds missing from TA's own counted against them. With this and the
  fixes below, Total Mayhem's counted findings fall from 137 to 91, TA: Escalation's from 395
  to 223, each keeping its 4 real errors (units naming weapons the mod does not have).
- A mod that emptied a file its units still use (a model, a script, a sound) passed the mod check
  without a word. Such a file is now told where it is used: the game reads nothing from it.
- Files the game never reads, such as pictures among a mod's documents, are no longer decoded,
  so a mod is no longer marked with errors for them.
- The mod check no longer warns about data the game never acts on: a unit's faction and a menu
  list for a builder that does not exist are notes, and feature animations and sound keys the
  game never plays are not checked. Sounds are judged as the game's decoder reads them: TA's own
  honk and sing sounds, which play, are no longer reported as broken, and sounds the game cannot
  play (IMA ADPCM other than 4-bit, more than 32 channels, an Ogg file of an Opus stream, a
  Humongous sound block without its samples) are.
- A script instruction pointing past a piece, a variable or the end of its code is now a warning
  rather than an error: the game skips it.
- A mod whose check found nothing said "No findings match the filters or the search" in its
  report; it now says no issues were found. Fixes opened in a report no longer stay open on other
  findings after RUN AGAIN.
- When a folder or archive cannot be read during a mod check, the report now says so, instead of
  "Validation cancelled".
- An empty placeholder named like one of Battle Tactics' or Core Contingency's archives is no
  longer reported as unreadable on an install without them, and a link (a shortcut or a junction)
  inside a mod's or the game's folder no longer stops a mod check: it is skipped, as the game
  skips it.
- Mods are checked again after an update of TA Reborn, whose file readers may read them
  differently, and the report's scope line is now in the interface's language.
- The launcher only knew a game was running when it had started that game itself. With a game it
  had not started (one running when the launcher was opened, or one the game had started again
  with a battle's mods), the launcher would update, add or remove mods under it, start a second
  game, or run an update of TA Reborn over it. The game now marks that it is running, and the
  launcher waits for any game as it waits for its own.
- Installing a mod from a file named like one already installed (a mod folder of your own, or an
  archive of Total Mayhem that is not its tested release) replaced the installed copy without a
  word, its files removed for good. The launcher now asks first. A package that would go behind
  the copy in TA Reborn's own mods folder is refused, as an update of that copy is.
- Mods added from Total Annihilation's folder showed an update waiting that nothing could clear
  (TA: Escalation's "Updates waiting: 1"). An update now counts only where the launcher can
  install it, over a copy of a known version. The mod's details still offer UPDATE.
- WHAT IT CHANGES no longer says a mod added from Total Annihilation's folder takes the place of
  its own archives there.
- PLAY on a mod's card and USE ITS MODS no longer change the load order when the game does not
  start (another game running, an install under way, a mod missing one it needs).
- A drag in the load order let go while the order was out of sight no longer moves a mod the next
  time it is shown, and an install finishing during a drag no longer moves the wrong mod.
- The launcher no longer opens every archive in Total Annihilation's folder each time it visits
  the Mods and Maps pages or starts the game: it looks again only when the folder changes, so a
  folder holding many map packs no longer slows it down.
- The REMOVE question no longer promises a GET button for mods that are downloaded by hand.
- An update of TA Reborn asked for while a profile was being edited, or a skin had unsaved
  changes, said STARTING THE UPDATE… and waited without saying why. It now says what it waits for.
- Dropping a mod's file again while it installs now says to wait. The extra mods folder can no
  longer be set to Total Annihilation's own folder, where every one of TA's archives showed as a
  mod. The usual reasons a mod's package cannot be installed (no program to unpack it, an archive
  only partly unpacked, no TA content in it) are now in the interface's language.
- Updating a mod kept in the extra mods folder installed the new version in your own mods folder,
  leaving the old copy behind it as a second mod of the same name, which a later REMOVE brought
  back. A mod folder kept there is now updated where it is, and replacing it with a file of your
  own does the same (a mod that is a single archive is updated in your mods folder, as before).
- A power cut just after a mod's update could leave the new version half written with the
  previous one already gone. The new files are now on the disk before they take its place.
- REMOVE was offered for mods in TA Reborn's program folder, which only an administrator may
  change, and failed. It now says to remove them there. In the Mods page's folders that folder is
  now called the program folder: "game folder" means Total Annihilation's.
- Removing a mod, or turning it off, gave no warning when other mods need it. The REMOVE
  question now names them, and turning it off says they are not loaded without it. The load
  order offers GET for a missing mod that others need, where the catalogue can download it.
- The launcher no longer reads every mod without a manifest (a map pack dropped into the mods
  folder, an archive or a folder of loose files) again at each visit of the Mods and Maps pages:
  it reads one again only when it changes.

## 1.0.3 — 2026-11-12

Battles now end in dark clouds at the map's edge, and the camera zooms out no further than the
whole map. Units fire at lesser targets when nothing better is in range, bombers bomb on patrol,
ships are built at the surface, construction aircraft sway as in TA, and aircraft circle over
the sea instead of stacking up.

### Changed
- In battles, the world beyond the map's edges is now a sea of dark clouds below the map, so
  the battlefield stands out, instead of land and sea carried on to the horizon. Zoomed out,
  that land filled most of the screen and shimmered. The map editor shows the clouds too; the
  menu's film, the credits and staged scenes keep the landscape.
- The battle camera now zooms out only until the whole map is in view, and zoomed out that far
  it keeps the map in view rather than the dark beyond it. On the largest maps it zooms out as
  far as before, and now the same on every screen: a 4K screen used to see twice as far.
- Fire at will: units now fire at their "bad targets" when nothing better is in range, rather
  than never. Missile towers fire on tanks when no aircraft are about, a destroyer's depth
  charges take on an amphibious tank under the water, and a scout ship's missiles a boat. A
  unit firing at a bad target turns to a better one as soon as one comes into range, and units
  still chase only what they are meant for. An attack order works as before.
- `+shootall` in the chat now does what it did in TA: every weapon of your units fires at every
  enemy in range alike, nearest first. Type it again to turn it off.
- Construction left with nobody working on it now un-builds after ten seconds, giving back the
  metal put into it (not the energy), and disappears when nothing is left. A frame placed and
  left before any work went into it goes at once. A factory's unit inside its yard, and
  unfinished buildings a mission or scene places, are left alone.
- Taking one of your own units apart now takes as long as its metal says, as reclaiming a wreck
  does. A Metal Maker, which costs no metal, is reclaimed at once.
- Planes with nothing to do over the sea, where they cannot land, now circle the spot instead
  of hovering in place. New planes sent to a rally point over the water circle there.
- A group of planes ordered onto one target now spread across a front as they come in, instead
  of following each other in single file.
- Putting a building down again where your selected constructor already plans one takes the
  plan back, and putting a different building down over a plan puts it in the plan's place: a
  wind generator instead of a solar collector, or a metal extractor placed in the fog of war
  taken back. Plans of other constructors still keep their ground.
- Shift with the right button while placing a building now ends the placing and queues the
  order there as well, so a Commander told to build a plant can be sent on after it.
- Bombers sent at something show TA's air-strike sight instead of the attack sight.
- Bombers on patrol or attack-move with fire at will now bomb what they come upon near their
  way, then fly on. Before, their bombs dropped only on an attack order, so patrolling bombers
  flew over the enemy and landed at the end of their route. A bomber on its own sees a building
  too late for its first pass and comes round for it; with radar or an ally's sight it lines up
  on targets further ahead. TA's bombers start on hold fire, so set them to fire at will. A
  plain move still bombs nothing.
- The music, and the battle behind the menu, now fall silent while a campaign briefing is read
  out, and come back once it is over. The tutorial's spoken lines still only lower the music.
- Construction aircraft at work now face what they build and sway smoothly round it to one side
  and back, as in TA, instead of hanging still over it. They turn to face it first, never fly
  backwards, stay within reach, and the building goes up as fast as before.
- Constructors on the ECONOMY duty now keep to a few plain jobs: metal extractors on free metal,
  power plants when power runs short (solar collectors, or wind generators where the wind pays
  better), and metal makers when there is power to spare, finishing buildings of those kinds
  left unfinished near them. They no longer build storage or geothermal plants, replace
  extractors with better ones, reclaim, or help at the factories.

### Fixed
- Scouts, construction aircraft and transports with nothing to do over the sea, such as new
  aircraft sent to a rally point on the water, hung still there on top of each other. They now
  circle it as fighters and bombers do, each round its own circle, so they keep apart; slow
  construction aircraft loiter slowly. Gunships still hover.
- Ships were built under the water in their shipyards (most of all the Core's) and only rose to
  the surface once finished. They are now built at the surface.
- A shipyard's ghost, while placing it or queued, showed on the sea floor; it now shows where
  the shipyard will stand.
- Aircraft plants' runways (and other buildings' flat bases, such as solar collectors') could
  flicker, the ground showing through them in lines, most of all zoomed out: the ground under a
  building was drawn up to a fraction of a pixel too high, level with the runway. It is now
  drawn never above where it belongs, and the runway sits a little higher over it.
- A constructor building a block of buildings that touch (Alt+drag while placing) could be
  walled in by them, unable to move again. Units now step off a new building's site clear of
  the builder's next sites, a unit left standing in a building is put on the nearest free
  ground, and a filled rectangle is built from its far side towards the builders.
- On a Mac in full screen, the system's pointer could show on top of the game's own. The game
  now has macOS apply the pointer it wants again whenever it takes the screen and once a second
  while it has the keyboard.

## 1.0.2 — 2026-11-11

Faster on Macs and other integrated graphics, which now draw the battlefield at about 1080p's
pixels. Also a fix for a crash on AMD's newest graphics cards, and a Mac's full screen without
the Dock and the menu bar coming over the game.

### Changed
- Faster on integrated graphics and high-resolution screens. On integrated graphics (Intel,
  AMD and Apple chips) the graphics levels now draw the battlefield at no more than about
  1080p's pixels, and 1440p's at Ultra; Apple's Pro, Max and Ultra chips draw more. The
  interface keeps the screen's full sharpness. A Mac's Retina screen has 2.7 to 3.7 times
  1080p's pixels, and before this an M5 MacBook Air ran well only on Low. The level is fitted
  when it is picked in the options, and once as the game starts: on its first start, or its
  first after this update, where the render scale is still the level's own (a scale set by
  hand is kept). A render scale below the level's own no longer makes the level Custom.
- A render scale below 100% now scales the battlefield up through a sharper filter (a cubic,
  kept from ringing round bright lights), so it loses less detail than it did.
- Units, trees and the painted features are now drawn before the ground, so the ground hidden
  behind them is no longer shaded. On a forest map this saves about a fifth of the frame.
  The picture is the same.
- The ground reads the map's tile table once per tile rather than once per texel (most of all
  for its HD surfaces), and looks up the art's softened colours once where two effects used
  them. The picture is the same.
- On Apple's own chips, the multisampled picture and its depth are stored only for a pass that
  reads them, as Apple advises for its tiled graphics.
- On a Mac, the launcher and the first start now know the computer's chip (an "Apple M5"), its
  memory and processor, and pick its graphics level from them: Medium on the base and Pro
  chips, High on the Max and Ultra.

### Fixed
- On a Mac in full screen, the menu bar and the Dock no longer come over the game when the
  pointer touches the screen's edges. Scrolling the map by the edges brought them up, and with
  the pointer over them the scrolling stopped. They now stay hidden while the game is in full
  screen; Cmd+Tab still switches apps, and they come back when the game leaves full screen.
- On AMD's newest graphics cards (a Radeon RX 9070 XT), the game could crash on Vulkan while
  preparing its graphics, the crash report naming the sprites' pipelines. One value in its Vulkan
  shaders, the sample mask, was written in a form Vulkan's rules do not allow, and AMD's driver
  expects the allowed form; it is now written as the rules require. The picture is the same as
  before.

## 1.0.1 — 2026-11-11

DirectX 12 works: the game no longer stays on its first loading screen there. Also a watch on
the game's memory, and crash reports for graphics drivers.

### Added
- A watch on the game's memory. Every minute of a battle the log records how much memory the
  game holds. Should it climb far past what the battle needs, the game lets go of its highlight
  copies, saves a single-player battle (the autosave) and says so. Near the computer's limit it
  also pauses the battle, rather than let the computer run out and take the battle with it.
- Crash reports give the game's memory at the time, and a game started without the launcher now
  keeps the last words of a crash for want of memory in its log.
- A crash outside the game's own code (a graphics driver failing as it compiles a shader) now
  writes a crash report too, naming the driver and what the game was preparing, and the
  launcher shows it. The log says how long the graphics card took over each of the game's
  shaders, and which it is still on when one takes long.

### Fixed
- On DirectX 12 the game no longer stays on its first loading screen. Windows' own shader
  compiler took many minutes over the ground's and the water's shaders. The game now ships
  Microsoft's DirectX Shader Compiler (DXC), and the shaders are written to compile several times
  faster with either compiler. On the computer it was measured on, the first battle opens on
  DirectX 12 in about half a minute the first time and in about 20 seconds after that (a minute
  and a half without DXC). The picture is the same as before, and Vulkan draws it in the same
  time.
- In a window smaller than 1024×600 points, a battle's first frame after its loading screen was
  laid out for a window of 10,000 points: the classic interface drew its panel and its letters
  dozens of times too large for one frame, which a graphics card could refuse.
- A row of choices (photo mode's tabs and looks, the options' selectors) no longer slides its
  marker in from the side of the screen when the panel it is in moves or first appears.

## 1.0.0 — 2026-11-09

The first public release of Total Annihilation Reborn: an unofficial, clean-room
reimplementation of the Total Annihilation engine in Rust. It plays your own copy of the game
(the Commander Pack from GOG or Steam), reading every unit, map, campaign and sound in place
from your install; everything around them is new.

### The battlefield
- Every world with its own sky, light and colour grade; physically based rendering, ray-traced
  shadows and reflections where the graphics card traces rays.
- Water with surf, bow waves, wakes and caustics; lava that crusts over and glows through its
  cracks; rain that soaks the ground and dries after; dawn, dusk and moonlit nights.
- Fire, smoke, fireballs and nuclear strikes; 3D trees, bushes and rocks; the land beyond the
  map's edge.
- Twenty thousand units in one battle.
- Two art styles: Reborn, which rebuilds TA's designs with armour plates, seams and bolts, and
  Enhanced Classic. Nineteen skins, and an editor for your own.

### Construction
- Nano-lathe beams and build lattices; buildings rising out of the green and sagging into
  rubble; factories that open and roll their units out; reclaim, wrecks and heaps.

### Perspective
- Ride along in any unit, with eight modelled cockpits, live instruments and rain on the glass.

### Command and control
- The command deck, or TA's own 1997 panel; a radial menu on any unit, and builders on duty.
- Formations that march as armies.
- Strategic zoom, and a companion screen for a second display.
- Photo mode.
- All of TA's keys and commands, and gamepads.
- TA's rules of engagement: units leave alone what their files call bad targets unless
  ordered, the long-range guns come out of the factory on hold fire, and strategic missiles
  strike the place their target stood.

### Ways to play
- Skirmish for ten players, with mutators.
- Every campaign of your install.
- Conquest, a galaxy to take battle by battle.
- Thirteen scenarios of TA Reborn's own, and a tutorial in five lessons with a coach, spoken
  in thirteen languages.
- Lockstep multiplayer, with allies, spectators and a lobby. Players with different map packs
  play together, and a join that can't go ahead says which mods differ.
- Replays with a cast view, highlights, MP4 export and social clips; saved games; a debrief
  after every battle.

### Computer players
- Three difficulties that build economies, defences and armies, raid, send waves, air strikes
  and nukes, fight at sea and ferry armies across the water.
- They play fair: they see what their team sees and scout for your base.
- Personas with names and manners talk in transmissions, and remember you.

### The launcher
- Finds GOG and Steam copies of the game.
- Mods from ModDB in a click, checked by a mod validator. Total conversions that replace TA's
  own archives, as TA: Enhanced does, replace them here too.
- A map library, and a skins library with its editor.
- Commander profiles with XP, 21 ranks and 134 achievements.
- A benchmark, music, and updates.

### TA Reborn's worlds
- Twenty-nine maps of its own: forests, deserts, islands, lava, metal, space, winter villages,
  and Australia, the USA and the whole world.
- A map editor that saves maps as mods.

### The Archives
- Every unit on show in a world of your choosing, with wallpapers up to 8K.
- The intro played on a 1997 CRT in a modelled bedroom.
- The Galactic War: the war's chronicle over a living galaxy, an archive of documents from both
  sides, and every campaign's missions.
- A title screen that films TA Reborn's worlds.
- A jukebox, and credits timed to a song of their own.
- Fourteen languages.

### Platforms
- Windows 10 and 11 (x64; ARM64 on Windows 11).
- macOS 13 or later (Intel and Apple Silicon).
- Ubuntu 22.04 or later and Arch Linux (x64 and ARM64).
