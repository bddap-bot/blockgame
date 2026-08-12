//! What both rigs are drawn out of — **the wordless alphabet.**
//!
//! There are two surfaces in this game that talk to a player without writing anything
//! down: [`crate::belt`], which is what you are carrying, and [`crate::forge`], which is
//! what things are made of. They are different rooms, and they say different things, but
//! they say them in the same six shapes: a silhouette, a glowing string, an arrowhead, a
//! bead, a notch bar, and a ring. This file owns all six.
//!
//! It also owns the [`Dir`] alphabet and the [`code`] every item answers to, because both
//! rigs are walked by the same two presses and a code that meant one thing on the belt and
//! another in the forge would be the one thing worth never doing here.
//!
//! Nothing in this file is a word or a number.

use bevy::prelude::*;

use crate::avatar::{self, Palette, Part, Skin, part};
use crate::registry::{Class, Item};

// ---------------------------------------------------------------------------
// The code alphabet.
// ---------------------------------------------------------------------------

/// Cells a cluster holds and clusters a body has — the same four, which is what makes the
/// two presses one gesture repeated rather than two things to learn.
pub const WAYS: usize = 4;
/// Codes there are. A d-pad has four directions and no more, so an item table past sixteen
/// needs a third press rather than a squeeze — and it says so at compile time, where a
/// fifteenth cluster cannot be added by accident and found out about on a Deck.
pub const CODES: usize = WAYS * WAYS;
const _: () = assert!(
    Item::COUNT <= CODES,
    "more items than two d-pad presses can name — give codes a third press"
);

/// One of the four directions a d-pad has, and the alphabet a code is spelled in.
///
/// Ordered clockwise from up, which is the order a cluster is fanned out in — so the
/// picture of a cluster and the order of this enum are the same fact.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Up,
    Right,
    Down,
    Left,
}

impl Dir {
    pub const ALL: [Dir; WAYS] = [Dir::Up, Dir::Right, Dir::Down, Dir::Left];

    pub fn index(self) -> usize {
        Dir::ALL.iter().position(|d| *d == self).expect("Dir::ALL")
    }

    pub fn nth(n: usize) -> Dir {
        Dir::ALL[n % WAYS]
    }

    /// Which way this points in the picture. Both rigs are looked at dead-on, so up on the
    /// pad is up on the screen and there is nothing to translate.
    pub fn towards(self) -> Vec3 {
        match self {
            Dir::Up => Vec3::Y,
            Dir::Right => Vec3::X,
            Dir::Down => Vec3::NEG_Y,
            Dir::Left => Vec3::NEG_X,
        }
    }

    /// The turn an arrowhead takes to point this way. Arrowheads are built pointing up and
    /// rotated, so there is one arrowhead in the game and not four.
    pub fn turn(self) -> Quat {
        Quat::from_rotation_z(-(self.index() as f32) * std::f32::consts::FRAC_PI_2)
    }
}

/// The two presses that reach one item.
pub type Code = [Dir; 2];

/// The code an item answers to: its place in [`Item::ALL`], read four at a time.
///
/// Derived rather than tabulated, so the registry stays the one source of it and a new row
/// in the item table arrives already reachable. It falls out nicely as the table stands —
/// the first four you ever dig up are one cluster, the four tools are another — but the
/// reason to keep it is that there is no second table to fall out of step with.
pub fn code(item: Item) -> Code {
    let n = item.index();
    [Dir::nth(n / WAYS), Dir::nth(n % WAYS)]
}

/// The item a completed code names, or nothing where the table has run out. Two of the
/// sixteen stations are empty today; pressing into one is a press that does nothing rather
/// than a press that picks whatever was nearest.
pub fn coded(code: Code) -> Option<Item> {
    Item::ALL
        .get(code[0].index() * WAYS + code[1].index())
        .copied()
}

// ---------------------------------------------------------------------------
// The kit: the meshes and paints both rigs are built from.
// ---------------------------------------------------------------------------

/// How many you own before a notch bar stops growing. Ten notches is already taller than
/// the thing beside it; past that the count stops being a shape and starts being a number,
/// which is what these surfaces do not do.
pub const BAR_CAP: u32 = 10;
const BAR_NOTCH: f32 = 0.11;

/// Meshes and paints the rigs reuse. Built once, when the world is.
#[derive(Resource)]
pub struct Kit {
    cube: Handle<Mesh>,
    ring: Handle<Mesh>,
    bead: Handle<Mesh>,
    /// Per item: the bright paint a lit bead or notch wears.
    lit: [Handle<StandardMaterial>; Item::COUNT],
    dark: Handle<StandardMaterial>,
    string: Handle<StandardMaterial>,
    ready: Handle<StandardMaterial>,
    waiting: Handle<StandardMaterial>,
    backdrop: Handle<StandardMaterial>,
}

fn glow(
    materials: &mut Assets<StandardMaterial>,
    c: Color,
    strength: f32,
) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: c,
        emissive: LinearRgba::from(c) * strength,
        perceptual_roughness: 0.4,
        ..default()
    })
}

impl Kit {
    pub fn new(meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) -> Kit {
        let lit = std::array::from_fn(|i| {
            let [r, g, b] = Item::ALL[i].color();
            glow(materials, Color::linear_rgb(r, g, b), 3.0)
        });
        Kit {
            cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
            ring: meshes.add(Torus::new(0.86, 0.95)),
            bead: meshes.add(Sphere::new(0.5)),
            lit,
            dark: materials.add(StandardMaterial {
                base_color: Color::srgb(0.10, 0.11, 0.14),
                perceptual_roughness: 0.9,
                ..default()
            }),
            string: glow(materials, Color::srgb(0.26, 0.32, 0.42), 0.6),
            ready: glow(materials, Color::srgb(0.45, 1.0, 0.5), 4.0),
            waiting: glow(materials, Color::srgb(1.0, 0.72, 0.28), 2.0),
            backdrop: materials.add(StandardMaterial {
                base_color: Color::srgba(0.02, 0.03, 0.06, 0.86),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                ..default()
            }),
        }
    }

    pub fn cube(&self) -> Handle<Mesh> {
        self.cube.clone()
    }
    pub fn ring(&self) -> Handle<Mesh> {
        self.ring.clone()
    }
    pub fn bead(&self) -> Handle<Mesh> {
        self.bead.clone()
    }
    pub fn lit(&self, item: Item) -> Handle<StandardMaterial> {
        self.lit[item.index()].clone()
    }
    pub fn dark(&self) -> Handle<StandardMaterial> {
        self.dark.clone()
    }
    pub fn string(&self) -> Handle<StandardMaterial> {
        self.string.clone()
    }
    pub fn ready(&self) -> Handle<StandardMaterial> {
        self.ready.clone()
    }
    pub fn waiting(&self) -> Handle<StandardMaterial> {
        self.waiting.clone()
    }
    pub fn backdrop(&self) -> Handle<StandardMaterial> {
        self.backdrop.clone()
    }
}

/// Builds the kit the moment a world exists, so both rigs find it already there.
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let palette = Palette::new(&mut meshes, &mut materials);
    let kit = Kit::new(&mut meshes, &mut materials);
    commands.insert_resource(palette);
    commands.insert_resource(kit);
}

// ---------------------------------------------------------------------------
// The silhouettes — what makes a nail read as a nail with no label under it.
// ---------------------------------------------------------------------------

/// A blank cube, for anything whose whole identity is its colour: the blocks, which the
/// player has already been digging up all afternoon and knows on sight.
const BLOCK: &[Part] = &[part(
    Skin::Paint(Item::Grass),
    [1.0, 1.0, 1.0],
    [0.0, 0.0, 0.0],
)];

const NAIL: &[Part] = &[
    part(Skin::Paint(Item::Nail), [0.5, 0.13, 0.5], [0.0, 0.44, 0.0]),
    part(Skin::Paint(Item::Nail), [0.16, 0.7, 0.16], [0.0, 0.04, 0.0]),
    part(
        Skin::Paint(Item::Nail),
        [0.07, 0.2, 0.07],
        [0.0, -0.38, 0.0],
    ),
];

const HAMMER: &[Part] = &[
    part(
        Skin::Paint(Item::Hammer),
        [0.17, 0.95, 0.17],
        [0.0, -0.13, 0.0],
    ),
    part(Skin::Gear, [0.78, 0.28, 0.30], [0.0, 0.42, 0.0]),
    part(Skin::Gear, [0.22, 0.20, 0.34], [-0.44, 0.30, 0.0]),
];

const DRILL: &[Part] = &[
    part(
        Skin::Paint(Item::Drill),
        [0.56, 0.52, 0.44],
        [0.0, 0.28, 0.0],
    ),
    part(
        Skin::Paint(Item::Drill),
        [0.20, 0.30, 0.20],
        [0.0, 0.62, 0.0],
    ),
    part(Skin::Gear, [0.30, 0.26, 0.26], [0.0, -0.06, 0.0]),
    part(Skin::Gear, [0.19, 0.24, 0.19], [0.0, -0.28, 0.0]),
    part(Skin::Dark, [0.10, 0.22, 0.10], [0.0, -0.48, 0.0]),
];

const HANDGUN: &[Part] = &[
    part(
        Skin::Paint(Item::Handgun),
        [0.92, 0.22, 0.16],
        [0.06, 0.24, 0.0],
    ),
    part(
        Skin::Paint(Item::Handgun),
        [0.24, 0.52, 0.16],
        [-0.26, -0.16, 0.0],
    ),
    part(Skin::Gear, [0.14, 0.12, 0.10], [-0.05, 0.06, 0.0]),
];

const RIFLE: &[Part] = &[
    part(
        Skin::Paint(Item::Rifle),
        [1.30, 0.14, 0.13],
        [0.10, 0.12, 0.0],
    ),
    part(
        Skin::Paint(Item::Rifle),
        [0.40, 0.30, 0.15],
        [-0.48, -0.04, 0.0],
    ),
    part(
        Skin::Paint(Item::Rifle),
        [0.20, 0.34, 0.14],
        [-0.16, -0.18, 0.0],
    ),
    part(Skin::Dark, [0.34, 0.11, 0.11], [0.16, 0.28, 0.0]),
    part(Skin::Gear, [0.08, 0.10, 0.08], [0.30, 0.30, 0.0]),
];

const PARACHUTE: &[Part] = &[
    part(
        Skin::Paint(Item::Parachute),
        [0.92, 0.20, 0.50],
        [0.0, 0.40, 0.0],
    ),
    part(
        Skin::Paint(Item::Parachute),
        [0.60, 0.18, 0.40],
        [0.0, 0.56, 0.0],
    ),
    part(
        Skin::Paint(Item::Parachute),
        [0.24, 0.14, 0.28],
        [0.0, 0.68, 0.0],
    ),
    part(Skin::Dark, [0.04, 0.52, 0.04], [-0.36, 0.02, 0.0]),
    part(Skin::Dark, [0.04, 0.52, 0.04], [0.36, 0.02, 0.0]),
    part(Skin::Gear, [0.32, 0.24, 0.24], [0.0, -0.34, 0.0]),
];

/// What one item looks like on a rig, and how big to draw the table.
///
/// The car is the game's own car — the one the player drives — shrunk onto a shelf,
/// because the strongest possible label for "this is a car" is a car.
fn icon(item: Item) -> (&'static [Part], f32, f32) {
    match item.class() {
        Class::Block(_) => (BLOCK, 1.0, 0.0),
        Class::Component { .. } => (NAIL, 1.0, 0.0),
        Class::Equippable { .. } => (PARACHUTE, 1.0, 0.0),
        // Every part table is written about its own feet, and the car's is the game's:
        // it is lifted by half its height so it hangs like everything else.
        Class::Vehicle { .. } => (avatar::CAR, 0.54, -0.42),
        Class::Tool { .. } => match item {
            Item::Drill => (DRILL, 1.0, 0.0),
            Item::Handgun => (HANDGUN, 1.0, 0.0),
            Item::Rifle => (RIFLE, 1.0, 0.0),
            _ => (HAMMER, 1.0, 0.0),
        },
    }
}

/// The silhouette tables are written for one item each, so the colour in them is that
/// item's. Re-skinning them per node is what lets one table draw every block.
fn repaint(p: &Part, as_item: Item) -> Part {
    Part {
        skin: match p.skin {
            Skin::Paint(_) => Skin::Paint(as_item),
            other => other,
        },
        size: p.size,
        at: p.at,
    }
}

/// Spawns one item's silhouette at `at`, wearing whatever markers the rig it belongs to
/// needs, and hands back the entity so the rig can move it about afterwards.
pub fn silhouette(
    commands: &mut Commands,
    palette: &Palette,
    item: Item,
    at: Vec3,
    marker: impl Bundle,
) -> Entity {
    let (parts, model_scale, lift) = icon(item);
    // `Inherited` and not `Visible`: a rig that hangs its silhouettes under a holder it
    // hides is the whole of how the belt folds up, and `Visible` means *visible anyway* —
    // which is fourteen models stacked in one place, in front of the man wearing them.
    commands
        .spawn((
            marker,
            Transform::from_translation(at),
            Visibility::Inherited,
        ))
        .with_children(|model| {
            for p in parts {
                let p = repaint(p, item);
                model.spawn((
                    Mesh3d(palette.cube()),
                    MeshMaterial3d(palette.paint(p.skin)),
                    Transform {
                        translation: Vec3::from(p.at) * model_scale + Vec3::new(0.0, lift, 0.0),
                        scale: Vec3::from(p.size) * model_scale,
                        ..default()
                    },
                ));
            }
        })
        .id()
}

// ---------------------------------------------------------------------------
// The notch bar: a count as a height.
// ---------------------------------------------------------------------------

/// One notch of a bar: the `nth` one you would own.
#[derive(Component)]
pub struct Notch {
    item: Item,
    nth: u32,
    lit: Handle<StandardMaterial>,
}

/// The bar beside a thing — how many you own, as a height rather than a number. Both rigs
/// spawn it the same way and one system below lights every one of them; the entities come
/// back so a rig that moves its things about can hang the bar off whatever moves.
pub fn notch_bar(
    commands: &mut Commands,
    kit: &Kit,
    item: Item,
    at: Vec3,
    marker: impl Bundle + Clone,
) -> Vec<Entity> {
    (0..BAR_CAP)
        .map(|nth| {
            commands
                .spawn((
                    marker.clone(),
                    Notch {
                        item,
                        nth,
                        lit: kit.lit(item),
                    },
                    Mesh3d(kit.cube()),
                    MeshMaterial3d(kit.dark()),
                    Transform::from_translation(
                        at + Vec3::new(0.86, -0.62 + nth as f32 * BAR_NOTCH, 0.0),
                    )
                    .with_scale(Vec3::new(0.20, BAR_NOTCH * 0.72, 0.20)),
                ))
                .id()
        })
        .collect()
}

/// Every notch on every rig: lit per one owned.
pub fn notches(
    stock: Res<crate::forge::Stock>,
    kit: Res<Kit>,
    mut notches: Query<(&Notch, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    for (notch, mut paint) in &mut notches {
        let lit = stock.0.count(notch.item).min(BAR_CAP) > notch.nth;
        let want = if lit { notch.lit.clone() } else { kit.dark() };
        if paint.0 != want {
            paint.0 = want;
        }
    }
}

// ---------------------------------------------------------------------------
// Arrowheads, and the code drawn as two of them.
// ---------------------------------------------------------------------------

/// One arrowhead, pointing `dir`: two barbs meeting at a point, in the only geometry this
/// game has. Written pointing up and turned, so there is one of these and not four.
///
/// `at` is the *tip*, and each barb is laid back from it along its own axis rather than
/// centred on it — two bars centred on one point cross into an X, which is a mark meaning
/// the opposite of "go this way".
pub fn arrowhead(
    commands: &mut Commands,
    kit: &Kit,
    dir: Dir,
    at: Vec3,
    size: f32,
    paint: Handle<StandardMaterial>,
    marker: impl Bundle + Clone,
) {
    for lean in [BARB_LEAN, -BARB_LEAN] {
        let turn = dir.turn() * Quat::from_rotation_z(lean);
        let along = turn * Vec3::Y;
        commands.spawn((
            marker.clone(),
            Mesh3d(kit.cube()),
            MeshMaterial3d(paint.clone()),
            Transform::from_translation(at - along * size * 0.5)
                .with_rotation(turn)
                .with_scale(Vec3::new(size * 0.17, size, size * 0.17)),
        ));
    }
}

/// How far each barb leans off the way the arrow points. Wide enough to read as an
/// arrowhead from a sofa, narrow enough not to read as a pair of horns.
const BARB_LEAN: f32 = 0.62;

/// An item's code, drawn as the two presses it is: the first arrow big, the second small.
///
/// Size is the order — big first, then small — so the pair reads the same whichever way a
/// child's eye travels, which a left-to-right rule would not. It is the label under every
/// node in the forge, and the thing that makes the two rigs one alphabet.
pub fn code_glyph(
    commands: &mut Commands,
    kit: &Kit,
    item: Item,
    at: Vec3,
    size: f32,
    marker: impl Bundle + Clone,
) {
    let [first, second] = code(item);
    arrowhead(
        commands,
        kit,
        first,
        at + Vec3::new(-0.24 * size, 0.0, 0.0),
        size,
        kit.lit(item),
        marker.clone(),
    );
    arrowhead(
        commands,
        kit,
        second,
        at + Vec3::new(0.26 * size, 0.0, 0.0),
        size * 0.66,
        kit.lit(item),
        marker,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property the whole surface rests on: two presses, one thing, no collisions and
    /// nothing unreachable. That the table *fits* sixteen is asserted at compile time above;
    /// this is that no two rows of it land on the same two presses.
    #[test]
    fn every_item_has_its_own_code() {
        let mut seen: Vec<(usize, usize)> = Item::ALL
            .iter()
            .map(|i| {
                let c = code(*i);
                (c[0].index(), c[1].index())
            })
            .collect();
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), before, "two items share a code");
        for item in Item::ALL {
            assert_eq!(
                coded(code(*item)),
                Some(*item),
                "{item:?} does not round-trip"
            );
        }
    }

    /// Every station the table has not reached yet names nothing, rather than the nearest
    /// thing to it.
    #[test]
    fn an_empty_station_names_nothing() {
        let empty = (0..CODES)
            .map(|n| [Dir::nth(n / WAYS), Dir::nth(n % WAYS)])
            .filter(|c| coded(*c).is_none())
            .count();
        assert_eq!(empty, CODES - Item::COUNT);
    }

    /// Up is up. Both rigs are looked at dead-on and the arrows are drawn in the plane the
    /// pad is pressed in, so a direction that stopped agreeing with its screen vector would
    /// be a picture teaching the wrong press.
    #[test]
    fn the_alphabet_points_where_it_says() {
        assert_eq!(Dir::Up.towards(), Vec3::Y);
        assert_eq!(Dir::Down.towards(), Vec3::NEG_Y);
        assert_eq!(Dir::Right.towards(), Vec3::X);
        assert_eq!(Dir::Left.towards(), Vec3::NEG_X);
        for dir in Dir::ALL {
            let turned = dir.turn() * Vec3::Y;
            assert!(
                turned.dot(dir.towards()) > 0.99,
                "{dir:?} arrowhead points the wrong way"
            );
        }
    }
}
