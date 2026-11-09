# Visual quality: the units

TA Reborn draws TA's own unit and building designs from the player's install; nothing of TA's
art is copied into the repository. Two art styles (Options → Graphics → ART STYLE, saved as
`graphics.art_style = "reborn"` or `"classic"`, `TA_ART_STYLE=reborn|classic` for captures)
work with either camera, independently of the quality presets.

## Enhanced Classic

TA's models and TA's paint: textures doubled with Scale2x into 64-texel layers rather than
blown up into blocks, each texel kept crisp when magnified (a sharp bilinear lookup,
`crisp_texels`) as TA drew them, corners where faces meet shallowly sharing a normal (barrels, domes and
rounded hulls shade as curves), every convex edge rounded in the shading as a fine line of
light (`bevelled` in `shaders/unit.wgsl`), edges worn a little and creases darkened. The
painted light and shade TA put into its textures stays. Many of TA's textures have a stray
pinkish-purple row or column at an edge (most often the top), which faces, mapping their
texture edge to edge, would show as a pink line along the edge: where an edge is markedly
pinker than the row inside it, its pink pixels take the colours inside them (`mend_edges`,
for both styles).

## Reborn

The same designs rebuilt when the game loads, from the faces of each installed model:

- **The kit** (`crates/ta_render/src/model/kit.rs`): large flat faces become armour plates,
  raised on their outline moved in, a narrow chamfer sloping down round each in a shade of its
  paint (on a team colour, of the player's colour), lit as it slopes; where an edge is open (no
  other face meets it, as at a fin's edge or round a hole left open, see below) the plate
  reaches it and lies flush. A big four-sided face is laid with several plates, up to 3 x 3,
  a darker groove between them: cut only across sides that run parallel (a kite or an odd
  quadrilateral is one plate: cuts fanning across it would read as cracks), each cut moved to
  where the paint is calmest along it (a painted panel line, between its lamps) and left out
  where it would cross painted detail wherever it went (`PaintMap`: lamps, emblems, stripes,
  doors, masonry). The biggest plates are bolted at the corners with low, domed rivet heads of
  dull steel; a face painted with a picture of its own (`PICTURES` in model.rs: Core's air
  plant and emblem panels) and a flat face in a palette colour get one plate, no bolts; so
  does such a face folding shallowly into smaller faces all round, a panel let into a rounded
  part (the Intimidator's and the Buzzsaw's turret sides), where a barrel's or a dome's
  facets, each like its neighbours, stay smooth. The outer sides of tracks (TA's `Tredside` paint on a face looking left or right, away from the
  model's middle) become a relief of road wheels inside a track loop, standing a little out of
  the side and never into the hull; a side's faces in one plane are taken together (ARMMART's
  are two), and the loop and the wheels are fitted inside its outline, a little smaller where
  its ends slope in (three wheels or more, else two, else none: a short wedge of a pod keeps
  TA's paint). The inner sides of pods, which face the hull, and tread paint facing any other
  way (a Kbot's, a submarine's tail) stay as TA painted them. Effect pieces, untextured
  faces, lights, logos, treads, TA's functional markings (`MARKINGS` in model.rs: landing
  circles, runways, factory and air pads, the Core emblem pads, which plates would cut) and
  curved, small or layered faces are left as they are. Layered: a face lying within 0.4 of
  another of its piece facing the same way, over or under it (a decal and the face under
  it), or with another piece's face lying on it at rest, or nearly (a wing's root on the hull,
  a landing pad on a runway lifted clear of the ground). Under another piece's face a little
  higher (the nested segments of a telescoping arm), a plate keeps 0.03 under it, unbolted.
- **Seen from behind**: TA's faces are one-sided, and from behind one TA shows what lies
  beyond it (into a factory's bay with its doors open, a fin's logo on its other side). So
  does the kit: a plate faces out as its face did, and its seams and rivet heads slope gently
  (rising less than half their width), so from behind every part of them faces away but
  within about 24 degrees of the face's plane. Only a track's side gets a dark back, as its
  road wheels would otherwise show through an open pod; the back lies 0.02 in front of the
  face, so a face of the other side in the same plane stays in front of it.
- **The water's reflection** draws the original shapes with Reborn's paint: the mirror sees
  the models from below, where the tracks' backs and the plates' rims would show, and its
  ripples hide the detail anyway.
- **The paint** (`model/remaster.rs`): each texture redrawn at 128 texels. TA painted its
  patterns lit and in shade for faces turned to and from its fixed light (ArmCam3a to 3d);
  each family is evened out towards its brighter variants (at most 2.5 times), as Reborn
  lights the models itself, and the brightest paint (chrome's painted sheen, white panels)
  rises on a shoulder rather than to white. The palette's noise goes first, its painted detail kept: flecks of
  other hues (pink, green, blue) take the hue round them, lone texels the median round them
  (lines a texel wide and lamps stay), an ordered dither's dots the shade round them; an
  edge-keeping smoothing mixes the rest of the dither as the eye does, hues more than light
  and neutral metal more than coloured camouflage. Then palette snapping: a bicubic
  enlargement gives each texel its rough colour and it is drawn to the nearest colour round
  it, so edges and thin panel lines come out crisp and the colours stay the artists'; the rough
  colour is enlarged along the way the paint's edges run (`along_edges`, a structure tensor),
  so TA's diagonal staircases (a team emblem's diamond, a chevron) come out straight lines; a
  metal's ramp in bands of the palette's greys (TA's painted chrome, `deband`) keeps the smooth
  enlargement, not snapped back into stripes. Grey paint loses the lavender the palette's
  light greys lean to, and half its soft painted sheen (`true_greys`: the Arm ships' hulls
  read as marble). A normal map from the paint's own relief finishes it. Team colours, lights and animated textures keep
  their paint exactly (the players' colours a little darker, so the light shapes them rather
  than every face sitting at full blue); treads (`Tredside`) are drawn in the dark steel of the kit's tracks, so
  the pods the kit leaves (a wedge too short for wheels) match the rest. TA's mottled paints
  (its camouflages and noise, by name: `ArmCam3a`, `CoCam10a`, `descamo3`, `graynoise3`) are
  then refinished (`refinished`): split by size, what spans much of a face (two tones, a
  gradient, a painted panel) and what is a texel or two wide (panel lines, a grille) stay, and
  the blotches between keep a third of their light and less of their hue, a clean factory paint
  of the camouflage's own colours rather than a soft, dirty smear when a unit is close. The
  Heritage skin (`original` in its livery) shows TA's units as Enhanced Classic draws them, its
  own shapes and paint, worn, in either style.
- **TA's palette-colour faces**: their brighter shades come down a little (picked for TA's
  light, they would blow out to white under Reborn's sun: a Moho Mine's dish), they take a
  faint mottle of weather (in Enhanced Classic half as strong), and in the detailed version a
  part TA shaded by hand facet by facet (a fusion reactor's globe) takes one colour, the lit
  half's, so Reborn's light shapes it; a plain grey face among painted ones (a Krogoth's claws'
  sides) takes the paint of the painted face it meets (`paint_plain_greys`).
- **Lights** keep their paint's brightness whatever light falls on them, a little over for the
  bloom, their pattern kept.
- **The finish**: painted plates under a faint satin lacquer, the horizon blurred out of rough
  paint's reflection, the ray-traced reflections only on smooth metal.
- **The edges**: broader rounding than Enhanced Classic's, so plate rims catch the light.
- **Holograms**: a building being placed, planned or built is lit as a shape (its faces by the
  sun, its outline bright, fine scan lines climbing it); plans and cloaked units are drawn
  translucent through the multisampled pixel's samples (no screen door), and nothing casts a
  shadow until it is half built.

The detailed version is drawn where a unit is at least 56 pixels across on screen (always in
the Archives and wallpapers); smaller, the original shape with Reborn's paint, so big battles
cost no more triangles than TA's own.

## Forms made whole (both styles)

TA's modellers built each part for a camera high above, and many parts are only the half of a
form that camera saw (`crates/ta_render/src/model/halves.rs`, when a model loads):

- **Halves.** Most of TA's guns, and many of its missile rails and crane booms, are square tubes:
  a top and two sides painted to look round, the underside left out and both ends open; a few
  round ones are the upper facets of a pipe. From the side or below such a barrel was a
  half-pipe with a lip, from in front a lone plate. Before the holes are closed, each open shell
  of a piece that is half of such a form is made whole by mirroring its faces, paint and all: a
  box missing a side gets its top mirrored onto the opening, across the plane halfway between
  them (tilted with a barrel narrowing towards its muzzle, the top's corners set on the
  opening's), so the underside is painted as the top; a round form (three facets or more, every
  corner about as far from the middle of the opening as its edges) is mirrored across the
  opening, its lower half under its upper. Then the caps close what is left, a channel's ends:
  a muzzle shows a dark bore. Only shells open underneath along one plane and at their ends, no
  wider or deeper than 10 units, and long (a channel half as long again as it is across, one
  closed at its ends twice as long), with nothing lying on the opening or where the other half
  goes. A ridged triangle (the Phalanx's guns, whose fire points lie inside the triangle), a
  knee, a turret or a hull's missing underside keeps its cap.
- **Backs.** Faces are drawn from the front alone, and TA built many a part for its camera
  above alone: a wing or a lid as one face facing up (the Construction Seaplane flew without
  its wings), a fin or a petal as one face (gone from its other side), an orb or a silo's shaft
  painted inside, a box with sides it never saw left out where no cap fits. Once the holes are
  closed, each piece is looked at alone from all round as the 3D camera may (60 directions from
  12 degrees under the level to 75 over it, rasterised at about 0.6 units a pixel:
  `crates/ta_render/src/model/sight.rs`), alone because pieces move (a nanolathe's case swings
  open, a turret turns, a door lifts off what it covered), and every face or cap whose back is
  the nearest thing in sight somewhere is given its back: the face again facing the other way,
  painted as it is, cut along the same diagonals so that a face TA built bent and its back are
  one surface. So is every sheet that is a piece of its own (a seaplane's wings lie folded over
  its hull until it flies). A plate lying flat at the model's foot keeps its one side (the
  ground is under it: a pad, a hover's skirt). The Implosion Device's orb is round, the Fark's
  head and the Advanced Construction Kbot's cases solid, fins show from both sides.
- **Rims.** A shallow shell open underneath is closed as the deeper ones are (see below).
- **Joints.** An opening facing up that another piece hid from TA's camera from every side (the
  Commander's hips under its torso, its knees under its thighs, its shoulders under its chest)
  was no doorway TA showed: as a walker steps or a torso turns it showed as a gap. It is capped
  as other holes are (late, as the forms made whole), unless another piece lies in it (a
  carrier's stalls on its deck) or over it as doors over a doorway.

What these add is drawn after TA's own faces, and TA's faces are shaded and fitted as if it
weren't there: their edges' wear, their plates and rims, their smoothed corners, their hand
shading and their bolts are as they were, to the bit, and so are the model's extent and the
shade it lays on the ground (a test builds every installed model with and without, in both
versions, and compares; but for the 35 pieces where a cap from before gave way to the faces
making a half whole, the Stumpy's middle barrel among them). Nothing added faces TA's camera
above but a muzzle pointing at it, now a dark bore. `TA_FORMS=0` leaves TA's forms as it built
them: barrels half, sheets one-sided, shallow rims open (their other holes capped as before).

## Holes closed (both styles)

TA's models were only seen from TA's camera, high over the battlefield, so many lack faces it
never showed: undersides, the inner sides of track pods, the bottoms of turrets, shoulders and
wings. Faces are one-sided, so in the 3D camera such a hole shows straight through the unit.
When a model loads (`crates/ta_render/src/model/caps.rs`), each piece's open loops (edges only
one face uses, its corners welded and edges split where others' corners lie on them) are closed
with caps wound as the faces round them, before the edge wear, the smoothing and the kit see the
faces, so the original and the detailed version both have them. A loop round several missing
sides of a box is cut along the box's edges, never into parts lying on the faces round it (the
Sentinel's sleeves, two sides and a top open below and at both ends, every corner on the one
loop: closed under them and across their ends), an overhang's missing underside spanned by the
least surface, a frame's underside closed round its middle. A shallow shell open underneath (a
thin hull such as the Flash's or the Crock's, a turret's base, a walker's foot) has its faces as
near the loop's plane as a sheet's would be, but standing up from it for most of its length: it
is closed as a rim (never with caps falling on its own faces, as a fin folded along its length
would get: such a fin is closed under it and across its ends), a late face as the forms made
whole are (see above). Left open: a lone face's or a flat sheet's outline, a hole TA's camera
looked into from above (a Shipyard's basin, an Ambusher's pit; not one another piece hid from
it, see Joints above), one lying on another face of its piece or behind another piece's face (a
doorway behind its doors), and loops whose caps would face into the model, hide its faces or cut
through them (given their backs where they show, see Backs above). Caps are dark metal
(`CAP_METAL` in model.rs), an underside in shade: the paint of the faces round, stretched over
them, read as a stray picture. Faces wound against all their neighbours are turned round only
where TA's camera saw neither side of them (an Ambusher's pit floor, wound so, is as its artist
left it). Long flat sheets standing alone in a palette colour (the Sea Serpent's frills, no face
meeting them but their own back), which seen edge-on were hairlines broken into dashes running
out of the model, become thin lenses of their outline (`thicken_lone_sheets` in model.rs):
edge-on a slim blade, from above as TA drew them. `TA_MEND=0` shows the models as they were.

## Chrome drawn out (both styles)

TA maps a texture's corners onto a quad's corners whatever its shape, so on a long, narrow face
its paint is drawn out along it: the Goliath's track covers (31 units by 6) draw TA's painted
chrome out five times, its reflections smeared into streaks like a blurred photograph. Laid
evenly the paint repeats, laid as a band of it its texels are blown up. A face of TA's chrome
sheet (`BRUSHED` in model.rs: the Core's chrome, its tanks' track covers) whose texels are drawn
out three times or more, its paint changing along the way it is drawn out, is laid instead with
the few lines of its paint across it that draw out best (the most painted along them, the most
alike, nearest the paint's own lightness: `PaintMap::stripe_lines`), running its whole length:
TA's own colours as brushed metal (`brushed_uvs`). TA's painted panels, stone and camouflage
keep their mapping: drawn out, their structure still reads.

## Curves rounded (Reborn)

Where TA drew a curve with a few facets (a dome, a barrel, a rounded hull) its corners take the
curve's normals (smooth.rs), so it is shaded round but its outline stays a polygon. In Reborn's
detail (`model/rounded.rs`) such facets are cut into smaller ones laid on the curve the normals
describe (Phong tessellation, drawn towards the corners' tangent planes 0.55 of the way, which
puts an octagon's sides on its circle), so the outline is as round as the shading: the plasma
battery's head is a dome, the air plants' caps are round. Only real curves: a run of at least
six facets meeting smoothly whose normals turn through 60 degrees all told; a shallow fold
between a few plates, shaded smooth but drawn angular (the Reaper's hull), stays as drawn. A
run is cut alike all over (two to four ways along each edge, by how far it would bow out), and a
point on an edge is reckoned from the edge alone, the same from either side: the facets meet
exactly; hard folds stay straight, the curve easing off towards them. About a fifth of TA's
triangles are drawn curved (`scan_curves`); rounding adds about 5% to the detailed versions'
triangles. `TA_ROUNDED=0` leaves the curves as drawn.

## Recoils kept clear (both styles)

TA's fire scripts kick a gun back in the frame of the piece it hangs from, level whatever the
gun's own pitch. Most guns hang from a sleeve their aim pitches and kick back along it; a few
pitch themselves, and pitched up to fire they slid back level through what was round their root:
the Core Raider's barrel through the top of its sleeve (a box no script turns), the Triton's
through its turret's top. TA's camera above never showed it, and TA's own faces do it (every layer
above off, the same). The scene draws the kick of a gun that points itself only as far back as it
runs clear of the faces round it as drawn (those it backs into head on and its way in aside), along
the gun as it points where that has more room (`apps/ta_game/src/scene/recoil.rs`, for units big
enough on screen to see it): level, the Raider kicks back its whole six units into its sleeve, as
TA has it; pitched up 17 degrees to lob a shell, under a unit, along itself (the Triton's barrel,
in a narrow channel of its turret's front, hardly at all). `TA_RECOIL_ROOM=0` shows the scripts'
kicks as they are.

## Numbers (stock TA with Core Contingency and Battle Tactics)

- 608 models; 168 halves in 77 of them made whole (48 units' and 29 wrecks'), and in 471 of
  them faces seen from behind given their backs: 7,185 faces mirrored, in about a second for
  them all (the looking round most of it, every piece and direction on a core of its own).
- 4,778 holes in 577 models closed with 6,378 caps (16,500 triangles) and 12 faces turned
  round, in about 110 ms for them all. Left open: 2,859 sheets' outlines, 930 loops lying on
  faces of their pieces, 66 behind other pieces' faces or with one lying in them, 74 openings
  TA's camera looked into, 65 frames' insides and 154 loops a cap would not fit (given backs
  where they show).
- What a 3D camera sees of the models from behind (the x-ray below, averaged over 96
  directions, all 353 of the units' and features' models together): about 7,600 square units
  before the sleeves, joints and backs, under 40 after.
- The kit details 594 of them, 116,000 triangles (caps and faces added included) becoming
  about 903,000, in about half a second for them all.
- Reborn's paint and finish: 704 layers at 128 texels (the textures the models name: 496 of the
  GAFs' 546), 117 MiB with mipmaps, made in parallel in about two thirds of a second when the
  game loads.
- Enhanced Classic's paint: 704 layers at 64 texels, 15 MiB.

## Looking at it

- The Archives: `--open units` with `TA_UNIT=ARMSTUMP` (and `TA_ART_STYLE`) frames one unit;
  `--width 3840 --height 2160` for a close look. `TA_REBORN_DETAIL=0` shows Reborn's paint on
  the original shapes.
- A battle: `--map "Coast To Coast" --demo battle --camera x,z,zoom,yaw,pitch`.
- The kit's test, over every installed model: `cargo test --release -p ta_render --lib
  the_kit_keeps` (each detailed piece within its original's extent; the triangle total).
- The forms, over every installed model: `cargo test --release -p ta_render --lib
  scan_halves -- --ignored --nocapture` (the models made whole, what became of every open
  shell; `TA_SCAN_MODELS=armbull,armstump` lists those models' shells, `TA_SCAN_VERDICT=Neither`
  every shell judged so, `TA_SCAN_VERDICT=Backed` every sheet given its back), and
  `TA_SCAN_MODELS=... TA_FORMS_OUT=<dir> cargo test --release -p ta_render --lib dump_halves --
  --ignored` writes them as OBJ, each piece's own faces, the faces mirrored and the caps in
  objects of their own. `cargo test --release -p ta_render --lib making_forms_whole` checks that
  TA's own faces are as they were.
- What is seen from behind, over every installed model: `cargo test --release -p ta_render --lib
  xray -- --ignored --nocapture` (each model looked at from 96 directions as the game makes it
  whole: the models showing the most, by the faces showing it and why the holes round them
  were left open; `TA_XRAY_SHARE=1` ranks by the share of the model, `TA_SCAN_MODELS=a,b` lists
  those models' faces, `TA_XRAY_OUT=<file>` writes every model's area for comparing runs). In
  the game, `TA_SHOW_BACKS=1` draws every face seen from behind in magenta, and
  `--demo gallery` with `TA_GALLERY_TOUR=<dir> TA_GALLERY_SOLO=1` photographs every unit alone
  (`TA_GALLERY_YAW`, `TA_GALLERY_PITCH`; on a map with level ground for all of them, such as
  Cinder Basin).
- The holes, over every installed model: `cargo test --release -p ta_render --lib scan_holes
  -- --ignored --nocapture` (what became of each loop, the models showing a 3D camera the most
  hole; `TA_SCAN_MODELS=armstump,corcv` lists those models' loops, `all` every model's), and
  `TA_SCAN_MODELS=... TA_HOLES_OUT=<dir> cargo test --release -p ta_render --lib dump_holes --
  --ignored` writes them as OBJ, faces, caps and the loops left open in objects of their own.
- A model in a 3D program: `ta-tools model objects3d/armstump.3do --obj <dir>` (the
  original), or `TA_KIT_DUMP=armstump,corraid TA_KIT_OUT=<dir> cargo test --release -p
  ta_render --lib dump_kit_models -- --ignored` (Reborn's detailed version). Both stay on this
  computer.

Flat skeleton joints are handled by `model/joints.rs`: a small untextured horizontal
plate with solid children attached above and below is extended to their nearby pivots.
This connects the Hammer's and Jethro's torsos and legs without changing original faces
or COB transforms. A nearby pivot outside the plate is accepted only when its child's
projected faces overlap the plate; the outline widens just enough to enclose that pivot
with a small attachment margin. The installed-model regression scan affects only
ARMHAM's and ARMJETH's pelvis pieces. Fins, platforms, remote hinges, separate bodies
and already solid joints are excluded.

## Still to do

- Modelled detail for more of TA's parts: barrels with a bore and a muzzle brake, hatches,
  vents, landing gear, ships' rails.
- The paint's functional markings (runways, landing pads, apertures) checked one by one:
  the smoothing can soften a very faint one. Markings are known by their textures' names
  (`MARKINGS`), so a mod's own pads still get plates.
- A lighter Reborn texture set for small graphics cards (compressed, or only the units in play).

## History

An earlier pass bundled a fleet of independently modelled replacement units (211 MB of GLBs),
then rebuilt TA's models with chamfered edges, inset panels and generated 256-texel paint
with a sheet of AI-generated machinery templates. It was replaced by the kit and the remaster
above: TA's own designs, more visible detail where it shows, about as many triangles and a
quarter of the texture memory.
