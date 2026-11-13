//! A map's art in TA's form, shared by TA Reborn's own map generator (`ta-tools mapgen`) and the
//! converter of other games' maps (`ta_supcom`):
//!
//! - [`slant`]: which ground TA's camera saw at each pixel of the art (TA painted its maps from
//!   the south and above);
//! - [`field`]: a grid of values over a map's cells, sampled as the game draws its heights;
//! - [`tiles`]: painted ground cut into TA's 32-pixel tiles, the alike ones shared within a
//!   budget, the seams between them settled, and each tile dithered into TA's palette.

pub mod field;
pub mod slant;
pub mod tiles;
