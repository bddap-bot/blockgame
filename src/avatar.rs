//! The player model — **the one place a body is built.**
//!
//! Drawn from `design/spaceman-avatar.jpg`: a white suit with teal trim, a bubble helmet
//! whose dark visor is ringed in teal, a backpack, a chest panel with a rocket on it,
//! mitten hands, knee patches and boots. Every part is the same unit cube scaled and
//! placed, so the whole model is the [`SPACEMAN`] table and swapping it for a different
//! body is an edit to that table — no other file changes.
//!
//! Coordinates are relative to the player's feet, in blocks: `+Y` is up, `-Z` is the way
//! they are facing.

use bevy::prelude::*;

/// Which material a part is painted with. Add a colour here and to [`Palette`] together.
#[derive(Clone, Copy)]
pub enum Skin {
    /// The white suit.
    Suit,
    /// Teal: trim, boots, mittens, the visor ring.
    Trim,
    /// Grey hardware — the backpack.
    Gear,
    /// Near-black gloss: the visor, and the rocket inked on the chest panel.
    Dark,
}

pub struct Part {
    pub skin: Skin,
    /// Width, height, depth in blocks.
    pub size: [f32; 3],
    /// Centre of the part, relative to the feet.
    pub at: [f32; 3],
}

const fn part(skin: Skin, size: [f32; 3], at: [f32; 3]) -> Part {
    Part { skin, size, at }
}

/// Front face of the torso. Anything decorating the chest sits just proud of it, so the
/// panel and the rocket on it are stacked in `z` from this one number.
const CHEST_Z: f32 = -0.13;

/// The spaceman, one row per box.
///
/// The whole table lives inside the player's collision box — `±`[`crate::player::HALF_WIDTH`]
/// in X and Z, [`crate::player::HEIGHT`] tall — because an avatar is drawn where its owner's
/// box is, so anything sticking out clips through walls its owner is standing clear of.
/// `the_model_fits_the_player_box` holds the table to it.
pub const SPACEMAN: &[Part] = &[
    // legs, knee patches, boots
    part(Skin::Suit, [0.14, 0.62, 0.20], [-0.09, 0.46, 0.0]),
    part(Skin::Suit, [0.14, 0.62, 0.20], [0.09, 0.46, 0.0]),
    part(Skin::Trim, [0.15, 0.15, 0.22], [-0.09, 0.50, 0.0]),
    part(Skin::Trim, [0.15, 0.15, 0.22], [0.09, 0.50, 0.0]),
    part(Skin::Trim, [0.16, 0.16, 0.30], [-0.09, 0.08, -0.03]),
    part(Skin::Trim, [0.16, 0.16, 0.30], [0.09, 0.08, -0.03]),
    // torso, belt, and the collar ring the helmet sits in
    part(Skin::Suit, [0.28, 0.58, 0.26], [0.0, 1.06, 0.0]),
    part(Skin::Trim, [0.30, 0.09, 0.28], [0.0, 0.815, 0.0]),
    part(Skin::Trim, [0.24, 0.08, 0.28], [0.0, 1.36, 0.0]),
    // backpack, with a tank strapped either side of it
    part(Skin::Gear, [0.24, 0.40, 0.14], [0.0, 1.10, 0.19]),
    part(Skin::Trim, [0.07, 0.30, 0.07], [-0.08, 1.12, 0.26]),
    part(Skin::Trim, [0.07, 0.30, 0.07], [0.08, 1.12, 0.26]),
    // arms, held clear of the body like the drawing, with shoulder pads and mittens. The
    // gap to the torso is what makes them read as arms and not as a wider chest, and both
    // arms plus that gap plus the chest between them share 0.6 blocks — so the arm is
    // narrow and the pad and mitten are what give the shoulder and hand their bulk.
    part(Skin::Suit, [0.10, 0.60, 0.15], [-0.2375, 1.02, 0.0]),
    part(Skin::Suit, [0.10, 0.60, 0.15], [0.2375, 1.02, 0.0]),
    part(Skin::Trim, [0.13, 0.10, 0.20], [-0.2325, 1.33, 0.0]),
    part(Skin::Trim, [0.13, 0.10, 0.20], [0.2325, 1.33, 0.0]),
    // mittens hang below the belt, or the two teal bands merge into one from a distance
    part(Skin::Trim, [0.12, 0.16, 0.19], [-0.235, 0.68, 0.0]),
    part(Skin::Trim, [0.12, 0.16, 0.19], [0.235, 0.68, 0.0]),
    // chest panel: a teal frame around a white plate, with a rocket on it
    part(Skin::Trim, [0.20, 0.26, 0.02], [0.0, 1.12, CHEST_Z - 0.01]),
    part(Skin::Suit, [0.15, 0.20, 0.02], [0.0, 1.12, CHEST_Z - 0.02]),
    part(Skin::Dark, [0.06, 0.11, 0.02], [0.0, 1.13, CHEST_Z - 0.03]), // rocket body
    part(Skin::Dark, [0.03, 0.04, 0.02], [0.0, 1.20, CHEST_Z - 0.03]), // nose
    part(
        Skin::Dark,
        [0.03, 0.05, 0.02],
        [-0.05, 1.06, CHEST_Z - 0.03],
    ), // fins
    part(Skin::Dark, [0.03, 0.05, 0.02], [0.05, 1.06, CHEST_Z - 0.03]),
    part(Skin::Trim, [0.03, 0.03, 0.02], [0.0, 1.05, CHEST_Z - 0.04]), // exhaust
    part(Skin::Trim, [0.03, 0.03, 0.02], [0.0, 1.15, CHEST_Z - 0.04]), // porthole
    // helmet: a white bubble, a dark visor across the front, teal ringing it
    part(Skin::Suit, [0.36, 0.40, 0.42], [0.0, 1.57, 0.0]),
    part(Skin::Trim, [0.38, 0.06, 0.44], [0.0, 1.74, 0.0]), // crown band
    part(Skin::Dark, [0.22, 0.22, 0.04], [0.0, 1.58, -0.21]),
    part(Skin::Trim, [0.28, 0.04, 0.03], [0.0, 1.71, -0.215]), // visor ring
    part(Skin::Trim, [0.28, 0.04, 0.03], [0.0, 1.45, -0.215]),
    part(Skin::Trim, [0.04, 0.30, 0.03], [-0.12, 1.58, -0.215]),
    part(Skin::Trim, [0.04, 0.30, 0.03], [0.12, 1.58, -0.215]),
];

/// The materials [`SPACEMAN`] paints with. One per [`Skin`].
#[derive(Resource)]
pub struct Palette {
    cube: Handle<Mesh>,
    suit: Handle<StandardMaterial>,
    trim: Handle<StandardMaterial>,
    gear: Handle<StandardMaterial>,
    dark: Handle<StandardMaterial>,
}

impl Palette {
    pub fn new(meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) -> Self {
        let mut paint = |r, g, b, rough| {
            materials.add(StandardMaterial {
                base_color: Color::srgb(r, g, b),
                perceptual_roughness: rough,
                ..default()
            })
        };
        Self {
            cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
            suit: paint(0.93, 0.94, 0.95, 0.85),
            trim: paint(0.10, 0.60, 0.60, 0.7),
            gear: paint(0.55, 0.58, 0.62, 0.8),
            dark: paint(0.06, 0.09, 0.12, 0.25),
        }
    }

    fn material(&self, skin: Skin) -> Handle<StandardMaterial> {
        match skin {
            Skin::Suit => self.suit.clone(),
            Skin::Trim => self.trim.clone(),
            Skin::Gear => self.gear.clone(),
            Skin::Dark => self.dark.clone(),
        }
    }
}

/// Spawns one player body at `transform` (its feet) and returns the root entity. The ONE
/// site a player model is created — remote players today, a visible local body tomorrow.
pub fn spawn(commands: &mut Commands, palette: &Palette, transform: Transform) -> Entity {
    commands
        .spawn((transform, Visibility::Visible))
        .with_children(|body| {
            for p in SPACEMAN {
                body.spawn((
                    Mesh3d(palette.cube.clone()),
                    MeshMaterial3d(palette.material(p.skin)),
                    Transform {
                        translation: Vec3::from(p.at),
                        scale: Vec3::from(p.size),
                        ..default()
                    },
                ));
            }
        })
        .id()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The model has to stand on the ground and fit inside the player's collision box on
    /// all three axes, or avatars sink into the floor and clip through walls their owner
    /// is clear of. Every part is checked, because one row of the table is all it takes.
    #[test]
    fn the_model_fits_the_player_box() {
        let mut lowest = f32::MAX;
        let mut highest = f32::MIN;
        let mut widest = 0.0f32;
        let mut deepest = 0.0f32;
        for p in SPACEMAN {
            lowest = lowest.min(p.at[1] - p.size[1] / 2.0);
            highest = highest.max(p.at[1] + p.size[1] / 2.0);
            widest = widest.max(p.at[0].abs() + p.size[0] / 2.0);
            deepest = deepest.max(p.at[2].abs() + p.size[2] / 2.0);
        }
        assert!(lowest >= -0.01, "the model starts below the feet: {lowest}");
        assert!(
            highest <= crate::player::HEIGHT,
            "the model is taller than the player box: {highest}"
        );
        assert!(
            widest <= crate::player::HALF_WIDTH,
            "the model is wider than the player box: {widest}"
        );
        assert!(
            deepest <= crate::player::HALF_WIDTH,
            "the model is deeper than the player box: {deepest}"
        );
        assert!(highest > 1.5, "the model is suspiciously short: {highest}");
        assert!(
            widest > 0.2 && deepest > 0.2,
            "the model is suspiciously thin: {widest} wide, {deepest} deep"
        );
    }

    #[test]
    fn every_part_has_volume() {
        for (i, p) in SPACEMAN.iter().enumerate() {
            assert!(
                p.size.iter().all(|d| *d > 0.0),
                "part {i} has a zero dimension"
            );
        }
    }

    /// The chest decoration is a stack of thin plates on the torso's front face: frame,
    /// then plate, then rocket. A plate level with the layer under it z-fights into a
    /// flicker — the sort of thing you only ever see on someone else's screen.
    #[test]
    fn the_chest_stack_is_layered() {
        let mut layers: Vec<f32> = SPACEMAN
            .iter()
            .filter(|p| p.at[2] < CHEST_Z && p.at[1] < 1.3)
            .map(|p| p.at[2] - p.size[2] / 2.0)
            .collect();
        assert!(layers.len() >= 6, "the chest panel lost its plates");
        assert!(
            layers.iter().all(|z| *z < CHEST_Z),
            "a chest plate is buried in the torso"
        );
        layers.sort_by(f32::total_cmp);
        layers.dedup();
        assert!(
            layers.len() >= 3,
            "frame, plate and rocket must sit at three different depths"
        );
    }
}
