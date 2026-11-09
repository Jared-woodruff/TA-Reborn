//! TA Reborn's visual identity, shared by the launcher and the game, after Total
//! Annihilation's own screens: grimy metal plates in steel pipe frames, bevelled gunmetal
//! buttons, olive-black wells, brass for the main action and focus, green lamps.
//!
//! [`theme`] holds the palette, the fonts and the egui style; [`metal`] paints the materials
//! (plates, pipes, bolts, wells, slabs, lamps, embossed titles); [`widgets`] the
//! custom-painted pieces (buttons, cards, switches, segmented selectors...);
//! [`panel`] the window chrome for pages and modal dialogs, with their transitions;
//! [`scroll`] scroll areas with a thumb in sight and their contents fading at a cut edge;
//! [`settings`] the settings screen the game and the launcher share; [`sound`] the
//! interface sounds widgets ask for; [`motion`] whether idle animations run or hold still.

pub mod badges;
pub mod benchmark;
pub mod credits;
pub mod faction;
pub mod i18n;
pub mod icon;
pub mod loading;
pub mod metal;
pub mod motion;
pub mod panel;
pub mod portrait;
pub mod profiles;
pub mod progress;
pub mod record_mods;
pub mod recycle;
pub mod scroll;
pub mod seasonal;
pub mod settings;
pub mod sound;
pub mod theme;
pub mod tooltip;
pub mod widgets;
