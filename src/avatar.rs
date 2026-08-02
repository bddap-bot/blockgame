//! The player model — **the one place a body is built.**
//!
//! Drawn from `design/spaceman-avatar.jpg`: white suit, teal trim, bubble helmet with a
//! dark visor, backpack, chest panel, mitten hands, knee patches, boots. Every part is
//! the same unit cube scaled and placed, so the whole model is the [`SPACEMAN`] table and
//! swapping it for a different body is an edit to that table — no other file changes.
//!
//! Coordinates are relative to the player's feet, in blocks: `+Y` is up, `-Z` is the way
//! they are facing.

use bevy::prelude::*;

/// Which material a part is painted with. Add a colour here and to [`Palette`] together.
#[derive(Clone, Copy)]
pub enum Skin {
    Suit,
    Trim,
    Gear,
    Visor,
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

/// The spaceman, one row per box.
pub const SPACEMAN: &[Part] = &[
    // legs and boots
    part(Skin::Suit, [0.20, 0.62, 0.20], [-0.13, 0.46, 0.0]),
    part(Skin::Suit, [0.20, 0.62, 0.20], [0.13, 0.46, 0.0]),
    part(Skin::Trim, [0.22, 0.16, 0.22], [-0.13, 0.62, 0.0]), // knee patch
    part(Skin::Trim, [0.22, 0.16, 0.22], [0.13, 0.62, 0.0]),
    part(Skin::Gear, [0.24, 0.15, 0.30], [-0.13, 0.075, -0.03]), // boot
    part(Skin::Gear, [0.24, 0.15, 0.30], [0.13, 0.075, -0.03]),
    // torso
    part(Skin::Suit, [0.46, 0.58, 0.26], [0.0, 1.06, 0.0]),
    part(Skin::Trim, [0.48, 0.09, 0.28], [0.0, 0.80, 0.0]), // belt
    part(Skin::Trim, [0.20, 0.20, 0.03], [0.0, 1.12, -0.14]), // chest panel (rocket motif)
    part(Skin::Gear, [0.30, 0.38, 0.14], [0.0, 1.10, 0.19]), // backpack
    // arms, held a little out from the body like the drawing
    part(Skin::Suit, [0.15, 0.50, 0.15], [-0.31, 1.10, 0.0]),
    part(Skin::Suit, [0.15, 0.50, 0.15], [0.31, 1.10, 0.0]),
    part(Skin::Trim, [0.18, 0.15, 0.18], [-0.31, 0.80, 0.0]), // mitten
    part(Skin::Trim, [0.18, 0.15, 0.18], [0.31, 0.80, 0.0]),
    // helmet: a teal ring with a white bubble inside and a dark visor at the front
    part(Skin::Trim, [0.42, 0.40, 0.42], [0.0, 1.58, 0.0]),
    part(Skin::Suit, [0.36, 0.34, 0.36], [0.0, 1.58, 0.0]),
    part(Skin::Visor, [0.26, 0.22, 0.04], [0.0, 1.60, -0.20]),
];

/// The materials [`SPACEMAN`] paints with. One per [`Skin`].
#[derive(Resource)]
pub struct Palette {
    cube: Handle<Mesh>,
    suit: Handle<StandardMaterial>,
    trim: Handle<StandardMaterial>,
    gear: Handle<StandardMaterial>,
    visor: Handle<StandardMaterial>,
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
            visor: paint(0.06, 0.09, 0.12, 0.25),
        }
    }

    fn material(&self, skin: Skin) -> Handle<StandardMaterial> {
        match skin {
            Skin::Suit => self.suit.clone(),
            Skin::Trim => self.trim.clone(),
            Skin::Gear => self.gear.clone(),
            Skin::Visor => self.visor.clone(),
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

    /// The model has to stand on the ground and fit inside the player's collision box, or
    /// avatars sink into the floor and clip through walls their owner is clear of.
    #[test]
    fn the_model_fits_the_player_box() {
        let mut lowest = f32::MAX;
        let mut highest = f32::MIN;
        for p in SPACEMAN {
            lowest = lowest.min(p.at[1] - p.size[1] / 2.0);
            highest = highest.max(p.at[1] + p.size[1] / 2.0);
        }
        assert!(lowest >= -0.01, "the model starts below the feet: {lowest}");
        assert!(
            highest <= crate::player::HEIGHT,
            "the model is taller than the player box: {highest}"
        );
        assert!(highest > 1.5, "the model is suspiciously short: {highest}");
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
}
