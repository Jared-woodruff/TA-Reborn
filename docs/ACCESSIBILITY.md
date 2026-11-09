# Accessibility

This audit covers the launcher, shared settings, modern and classic battle interfaces,
controller input, loading screens and visual effects. It is a source review with automated
regression checks and local visual checks, not a claim of accessibility certification or a
substitute for testing with disabled players.

Local Windows UI Automation checks exercised native checkbox focus/toggling in both
applications, the history summary/filter, and starting a battle through its loading
transition on the same window. Native keyboard checks covered selection, movement,
building and cancellation with both order-button schemes. These checks do not establish
Narrator/NVDA speech behaviour or complete keyboard-only play.

## Available controls

Open **Settings → Accessibility** in the launcher or **Options → Accessibility** in the
game. These preferences are saved with the other settings and can be searched by name.

| Control | Behaviour |
| --- | --- |
| Interface scale | Applies 75–200% zoom in the launcher and game Options. Settings reflow into a narrow layout with scrolling. Battlefield HUD and other game menus still cap enlargement to fit; some launcher pages use horizontal scrolling. |
| Colour-blind colours | Alternative team palette; can be combined with the colour-independent player markers below. |
| Player markers | Optional black-and-white player-slot numbers and relation symbols on visible units and the minimap: `=1` yours, `+2` allied, `×3` enemy. Existing unit-class shapes remain. Unidentified radar contacts stay unidentified; unrestricted spectators use neutral labels. Nearby units of the same player may share a label. |
| Keyboard pointer | Optional pointer operation using arrows, Shift for precision, Enter to select/place/activate, End to order, Home to centre, and PageUp/PageDown to zoom. Tab/Shift+Tab cycle your available units; Backspace/Shift+Backspace aim at visible enemies while preserving your selection. Alt+K toggles it by default. |
| Screen-reader announcements | Enables rate-limited polite announcements for selection changes and battle notices when native assistive technology connects. Resource/selection summaries remain available without automatic announcements. |
| Controller dead zone / pointer speed | Adjust the stick dead zone from 0–50% and pointer speed from 25–300%. Controls are released on disconnect or focus loss; sticks/buttons must return to neutral before resuming after focus loss. |
| Reduced motion | Holds decorative UI animation and the menu camera still, removes blast shake and optional cockpit sway, recoil, roll and entrance movement, and suppresses heat distortion. Loading progress still updates. |
| Reduced flashes | Softens luminous combat effects and lights, removes broad explosion flashes, storm flashes and unit damage tint, suppresses bloom and heat distortion, and holds cockpit warning pulses steady. Projectile paths, health information and directional damage cues remain. |
| TA cursors | Turn off to use the operating system pointer, including Windows pointer accessibility settings. |
| Cursor size | Enlarge TA's battle cursor from 100% to 250% without moving its click hotspot. |
| High-contrast cursor | Adds a static black-and-white locator around the TA cursor's click position. |
| Message duration | Keep battle notices visible for 1–4 times their normal duration. Pausing the battle also pauses their expiry. Long notices wrap, repeats refresh and the queue is limited to six notices. |

Cursor size and the locator apply to TA cursors. Reduced motion does not stop unit movement,
player-controlled cameras, the benchmark's required camera path or original video playback.
Reduced flashes is a reduction, **not a photosensitivity safety guarantee**; original videos
and other bright scene changes remain. Skip introductory video using its existing setting.

These presentation settings do not change simulation rules, multiplayer compatibility or
saved battles. Reducing effects changes the rendering workload, so benchmark reports retain
both reduction settings and identify them in the launcher and copied results. Compare runs
using the same accessibility settings; the reference machine's baseline uses both off.

## Functional improvements

- Keyboard focus has a consistent visible outline on shared controls. Tabbing to a shared
  control in a scrollable panel brings it into view. Decorative scroll blockers no longer
  create empty Tab stops.
- Native AccessKit backends are connected in the launcher and game, including loading
  transitions. Loading stages, battle resource/selection summaries and standard controls
  enter the native accessibility tree. Battle live announcements coalesce to at most one
  update per 1.5 seconds. This is foundational support, not complete screen-reader gameplay.
- Overflowing scroll areas accept arrow keys, PageUp/PageDown and Home/End when focused,
  including virtualized lists. Settings remain reachable at 200% UI scale.
- Keyboard pointer mode avoids holds/drags for its primary clicks and supports the build
  menu and minimap. Its reserved navigation keys replace their normal battle shortcuts
  while enabled; text fields and dialogs retain normal navigation. This is a virtual
  pointer, not semantic traversal of every battlefield object or build location.
- Battle dialogs block camera movement and gameplay input, including in multiplayer where
  the simulation continues. The controller reference follows the selected order button.
- **Alt+H** (remappable), or **Pause → Battle event history**, opens a filterable, timestamped
  record of the latest 200 battle notices. Rapid consecutive duplicates are grouped.
  Clearing transient messages does not erase it. History is session-only: it is not saved
  with battles, and rewinding clears the prior timeline. Opening the history directly does
  not pause a live battle; single-player pause remains available before opening it.
- Disconnecting a controller releases its held pointer buttons, preventing a stuck drag.
- Resource exhaustion has a written notice in both battle interfaces, alongside the
  existing sound. Notices remain useful with sound muted or unavailable.

## Existing support retained

- Remappable gameplay keys, a controls reference, a choice of order mouse button, adjustable
  camera scrolling, and controller operation using a virtual pointer.
- Pause and slower single-player game speeds; these are not promises of a local pause in
  a live multiplayer match.
- Separate master, music, effects and voice volumes, and written campaign briefings.
- Text feedback for attacks, nuclear launches and failed orders, health bar lengths and
  numeric health information on inspected units.
- Searchable settings shared between game and launcher, with persistent preferences and
  translations in all supported interface languages.

## Follow-up priorities

The launcher QA pass on 31 October 2026 added whole-page scrolling to short
library viewports, so enlarged toolbars no longer make the cards unreachable.
It also prevents compact header controls from overlapping and keeps page
shortcuts and file drops inactive behind dialogs. These fixes do not complete
the gameplay reflow and assistive-technology checks below.

| Priority | Remaining barrier | Suggested implementation and acceptance check |
| --- | --- | --- |
| High | Reflow remains incomplete outside settings | Reflow the battlefield HUD, pause/menu panels and fixed-width launcher pages at larger sizes. Verify long translations, high-DPI displays and the smallest usable viewport. Settings now honour the full scale; this does not make every screen scalable. |
| High | Keyboard pointer still depends on spatial aiming | Add semantic traversal of targets, build locations and minimap destinations; alternatives to remaining modifier/drag gestures. Validate a complete tutorial and skirmish with keyboard-only players, both order-button modes and both HUDs. |
| High | Screen-reader gameplay is incomplete | Validate Narrator and NVDA focus order, loading transitions and speech rates. Add semantic actions and names to remaining custom-painted HUD controls, plus controlled order confirmation. Native text replacement via UIA SetValue is supported by the history filter; other egui text fields still need handling for that action. Native UIA exposure is necessary but not a substitute for reader testing. |
| Medium | Controller mapping is fixed | Add controller remapping and alternatives to remaining holds/modifiers; validate hot-plugging and limited-dexterity play with physical controllers. Dead zone, pointer speed, focus loss and reference labels are now addressed. |
| Medium | Busy scenes can crowd player markers | Validate greyscale identification with disabled players at strategic zoom, all palettes and maximum army density. Clustering preserves distinct owners, but extreme density can still overlap labels. |
| Medium | History is limited to the current session | Consider an optional saved/exportable event log and structured categories. The new history retains notices, not every simulation event or cutscene line. |
| Medium | Original cutscene speech lacks captions | Provide timed captions and speaker labels where legally distributable, plus readable transcripts and skip controls. Written mission briefings do not caption all original video. |
| Medium | Contrast varies across custom HUD elements | Audit text, focus, selection and disabled controls over bright terrain, dark terrain and translated layouts; offer a consistent high-contrast HUD theme and adjustable opaque backgrounds. |

Next validation should include players with low vision, colour-vision differences, limited
dexterity, motion sensitivity and hearing loss. Cover first launch, settings recovery,
tutorials, a full skirmish, save/load, classic HUD, controller disconnect and multiplayer.

## Review references

The scope was informed by the [Xbox Accessibility Guidelines](https://learn.microsoft.com/gaming/accessibility/guidelines),
especially [visual distractions and motion](https://learn.microsoft.com/en-us/gaming/accessibility/xbox-accessibility-guidelines/117),
and the [Game Accessibility Guidelines](https://gameaccessibilityguidelines.com/basic/).
The priorities above are findings about this codebase, not a standards compliance assessment.
