//! The car: arcade driving over voxel ground.
//!
//! A car is not a block and not a player — it is the one thing in the game that a player
//! *rides*, and [`Ride`] is the whole of its life cycle: in your pocket, standing where you
//! left it, or under you. Nothing else can be written down, so "driving a car that isn't
//! there" is not a state anybody has to handle.
//!
//! The physics are deliberately not a physics engine. There are no wheels, no suspension
//! and no sideways slip: the car hovers a hair above whatever the ground under its four
//! corners is, drives along its own facing, and climbs anything up to [`MAX_CLIMB`]. That
//! is the whole model — a step up is drivable, a wall is not, and the difference is one
//! constant. Everything here is a pure function of `(&World, Car)`, so all of it is tested
//! without a window.
//!
//! Cars are **not** host-authoritative state, and deliberately so. Where a car is, is the
//! same *kind* of claim as where its owner's feet are: the driver simulates it and the host
//! relays it, exactly as it relays a pose, and the driver's own feet stay under the host's
//! travel budget the whole time — [`TOP_SPEED`] is well inside it. What the host *does* own
//! is the [`crate::registry::Item::Car`] that entitles you to one, so a car nobody built
//! cannot be seen (`game::net_receive` strips it).

use bevy::math::{Quat, Vec2, Vec3};

use crate::player;
use crate::world::{BlockPos, WORLD_HEIGHT, World};

/// Flat out, in blocks per second. Faster than a sprint is the whole point of a car —
/// `a_car_beats_running` is what keeps it true — and it stays well under the host's
/// `TRAVEL_RATE`, because a driver's feet ride the same pose budget as a runner's.
pub const TOP_SPEED: f32 = 14.0;
/// Backwards is for getting off a wall, not for travelling.
pub const REVERSE_SPEED: f32 = 5.0;
// A car slower than the boots you already own is eight nails wasted, and a car that
// reverses as fast as it drives has no front. Both fail the build rather than a test: they
// are facts about two numbers, and nothing has to run to find out.
const _: () = assert!(TOP_SPEED > player::SPRINT_SPEED && REVERSE_SPEED < TOP_SPEED);
/// Blocks per second per second, and also the brake: with no throttle the target speed is
/// zero, so coasting, braking and pulling away are one line of arithmetic.
pub const ACCEL: f32 = 11.0;
/// Radians per second at full stick. Independent of speed: turning on the spot is what a
/// six-year-old does to point the thing at a hill, and an arcade car that refuses to is
/// just a car you have to reverse out of a corner.
pub const TURN_RATE: f32 = 2.0;

/// Clearance under the chassis. Small enough to read as sitting on the ground, big enough
/// that the car is never *in* the block it is standing on.
pub const HOVER: f32 = 0.04;
/// The tallest rise the car will drive up onto. One block and a little: a stair you built
/// is drivable, and the wall above it stops you.
pub const MAX_CLIMB: f32 = 1.1;

/// Half the car's footprint, along its own facing and across it. The wheels, not the
/// bumpers: a bumper overhanging a ledge should not hold the car up.
pub const HALF_LENGTH: f32 = 0.90;
pub const HALF_WIDTH: f32 = 0.76;

/// The driver's feet, relative to the car's underside: `+X` right, `+Y` up, `-Z` forward —
/// the same model space [`crate::avatar`] draws in. On the deck and behind the bonnet, so
/// there is something in shot from the seat. `avatar::the_driver_stands_on_the_deck` pins
/// it to the model, rather than to a number that drifts away from it.
pub const SEAT: Vec3 = Vec3::new(0.0, 0.28, 0.32);
/// How far from the car you may still get into it.
pub const BOARDING_RANGE: f32 = 3.5;
/// How far in front of its owner a car is put down. Clear of the player's own box, and
/// inside [`BOARDING_RANGE`] so putting one down and getting in is two presses.
const PARK_AHEAD: f32 = 2.6;
const _: () = assert!(PARK_AHEAD < BOARDING_RANGE);

/// Longest hop between ground tests. Under the footprint's half-length, so a wall can
/// never be crossed between two samples — the same tunnelling defence
/// [`player::move_and_slide`] makes, for the same reason.
const MAX_STEP: f32 = 0.4;
const _: () = assert!(MAX_STEP < HALF_LENGTH);

/// One car. `pos` is the middle of its underside — the car's feet, exactly as a player's
/// `pos` is theirs, so "put it on the ground at y" means one thing for both.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Car {
    pub pos: Vec3,
    /// Radians, left-handed about +Y, as everywhere else: 0 faces -Z.
    pub yaw: f32,
    /// Along its own facing. Negative is reverse.
    pub speed: f32,
    /// How fast it is dropping while it is off the ground. Zero the moment it settles.
    pub fall: f32,
}

impl Car {
    /// Where the driver's feet go. The one place the seat is, so the camera and the body
    /// everyone else sees sitting in the car cannot end up in different places.
    pub fn seat(self) -> Vec3 {
        self.pos + Quat::from_rotation_y(self.yaw) * SEAT
    }

    /// Beside the driver's door, where somebody getting out is put down. The ordinary
    /// walking collision takes it from there — including pushing them out of a wall they
    /// parked against.
    pub fn step_out(self) -> Vec3 {
        // Level with the car's own underside, which is already clear of the ground by
        // [`HOVER`] — the walking fall settles the last hair of it.
        let out = Vec3::new(HALF_WIDTH + player::HALF_WIDTH + 0.2, 0.0, 0.0);
        self.pos + Quat::from_rotation_y(self.yaw) * out
    }
}

/// Where a player's car is in their life, and the only three places it can be.
///
/// [`Ride::Driving`] carries the car itself rather than pointing at one, so a player who is
/// driving always has something to be driving — the state that would otherwise need a
/// runtime check does not exist.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ride {
    /// No car of theirs in the world: it is in their pocket, or they have not built one.
    Pocketed,
    /// Standing where they left it.
    Parked(Car),
    /// They are in it, and it is the thing that moves.
    Driving(Car),
}

impl Ride {
    /// Their car, if it is out in the world at all — which is exactly what has to be drawn
    /// and what rides the pose stream.
    pub fn car(self) -> Option<Car> {
        match self {
            Ride::Pocketed => None,
            Ride::Parked(car) | Ride::Driving(car) => Some(car),
        }
    }

    pub fn is_driving(self) -> bool {
        matches!(self, Ride::Driving(_))
    }
}

/// What is under a car.
///
/// [`Ground::Unknown`] is terrain nobody has generated, and it is not a hole: an unloaded
/// chunk reads as air, so a car that trusted it would drive off the edge of the generated
/// world and keep going. It stops the car, exactly as it stops a walking player.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Ground {
    /// The top face of the highest solid block under here.
    Top(f32),
    /// Nothing solid the whole way down.
    Void,
    Unknown,
}

/// The surface of one column, searching down from `ceiling`.
fn column(world: &World, x: f32, z: f32, ceiling: f32) -> Ground {
    let (bx, bz) = (x.floor() as i32, z.floor() as i32);
    if !world.is_loaded(BlockPos::new(bx, 0, bz).chunk()) {
        return Ground::Unknown;
    }
    let top = (ceiling.floor() as i32).min(WORLD_HEIGHT - 1);
    for y in (0..=top).rev() {
        if world.solid(BlockPos::new(bx, y, bz)) {
            return Ground::Top((y + 1) as f32);
        }
    }
    Ground::Void
}

/// What is under the car's four corners, taking the highest — so a car straddling a step
/// rides up onto it instead of sinking a corner into it.
///
/// The search starts [`MAX_CLIMB`] above the car, not at the sky: a block higher than the
/// car can climb is a wall, and a wall is not something to be stood on top of.
fn ground_under(world: &World, pos: Vec3, yaw: f32) -> Ground {
    let rot = Quat::from_rotation_y(yaw);
    let mut top: Option<f32> = None;
    let mut unknown = false;
    for (x, z) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        let at = pos + rot * Vec3::new(HALF_WIDTH * x, 0.0, HALF_LENGTH * z);
        match column(world, at.x, at.z, pos.y + MAX_CLIMB) {
            Ground::Unknown => unknown = true,
            Ground::Top(y) => top = Some(top.map_or(y, |best: f32| best.max(y))),
            Ground::Void => {}
        }
    }
    // One unknown corner is enough: the car is at the edge of what has been generated, and
    // there is no answer for where it would end up.
    match (unknown, top) {
        (true, _) => Ground::Unknown,
        (false, Some(y)) => Ground::Top(y),
        (false, None) => Ground::Void,
    }
}

/// One frame of driving: `control.x` steers, `control.y` is the throttle. Both come
/// straight off the left stick, so a car is driven with the hand that walks.
pub fn drive(world: &World, mut car: Car, control: Vec2, dt: f32) -> Car {
    // Steering right *lowers* the yaw: yaw is left-handed about +Y, so it climbs to the
    // left. `a_car_and_a_player_face_the_same_way` pins the convention.
    car.yaw -= control.x.clamp(-1.0, 1.0) * TURN_RATE * dt;

    let throttle = control.y.clamp(-1.0, 1.0);
    let want = throttle
        * if throttle >= 0.0 {
            TOP_SPEED
        } else {
            REVERSE_SPEED
        };
    // One rate for pulling away, braking and coasting: with no throttle `want` is zero, so
    // letting go of the stick *is* the brake and there is no second number to tune.
    car.speed += (want - car.speed).clamp(-ACCEL * dt, ACCEL * dt);

    let steps = ((car.speed.abs() * dt) / MAX_STEP).ceil().max(1.0) as u32;
    for _ in 0..steps {
        car = step(world, car, dt / steps as f32);
    }
    car
}

/// One sub-step: forward if there is somewhere to go, then down onto whatever is there.
fn step(world: &World, mut car: Car, dt: f32) -> Car {
    let ahead = car.pos + Quat::from_rotation_y(car.yaw) * Vec3::NEG_Z * (car.speed * dt);
    match ground_under(world, ahead, car.yaw) {
        Ground::Unknown => car.speed = 0.0,
        // The whole of collision: a rise you can climb is a hill, and one you cannot is a
        // wall. Hitting it stops the car dead rather than grinding along it — a car that
        // slid down walls would need a second collision model to say how.
        Ground::Top(top) if top + HOVER - car.pos.y > MAX_CLIMB => car.speed = 0.0,
        _ => {
            car.pos.x = ahead.x;
            car.pos.z = ahead.z;
        }
    }
    settle(world, car, dt)
}

/// Puts the car on the ground, or drops it towards one. Rising is instant — that is what
/// makes driving up a hill feel like driving rather than climbing — and falling is gravity,
/// so a drive off a cliff is a fall and not a glide.
fn settle(world: &World, mut car: Car, dt: f32) -> Car {
    let rest = match ground_under(world, car.pos, car.yaw) {
        Ground::Unknown => return car,
        Ground::Top(top) => top + HOVER,
        // Nothing under it at all. Keep falling; there is nothing to land on.
        Ground::Void => f32::NEG_INFINITY,
    };
    if car.pos.y <= rest {
        car.pos.y = rest;
        car.fall = 0.0;
    } else {
        car.fall = (car.fall + player::GRAVITY * dt).min(player::MAX_FALL_SPEED);
        car.pos.y = (car.pos.y - car.fall * dt).max(rest);
    }
    car
}

/// A car standing still on the ground under `at`, or nothing if there is no generated
/// ground to stand it on. `at.y` is where the search for that ground starts.
fn on_ground(world: &World, at: Vec3, yaw: f32) -> Option<Car> {
    match ground_under(world, at, yaw) {
        Ground::Top(top) => Some(Car {
            pos: Vec3::new(at.x, top + HOVER, at.z),
            yaw,
            speed: 0.0,
            fall: 0.0,
        }),
        Ground::Unknown | Ground::Void => None,
    }
}

/// A car put down in front of a player standing at `feet` and facing `yaw`, or nothing if
/// there is nowhere generated to stand it — better in the pocket than dropped into a chunk
/// that has not arrived.
pub fn park_in_front(world: &World, feet: Vec3, yaw: f32) -> Option<Car> {
    on_ground(
        world,
        feet + Quat::from_rotation_y(yaw) * Vec3::NEG_Z * PARK_AHEAD,
        yaw,
    )
}

/// What the ride button does, and where its presser ends up.
///
/// Pure in `(ride, feet)`, so the whole enter/exit state machine is exercised without a
/// window: the caller writes both halves back onto the player.
pub fn toggle_ride(ride: Ride, feet: Vec3) -> (Ride, Vec3) {
    match ride {
        // Nothing to get into — put one down first.
        Ride::Pocketed => (ride, feet),
        Ride::Parked(car) if feet.distance(car.pos) <= BOARDING_RANGE => {
            (Ride::Driving(car), car.seat())
        }
        // Too far away to reach the door. Walk to it.
        Ride::Parked(_) => (ride, feet),
        // Out of the driver's side, and the car stops: one left with its speed still on
        // would drive itself away with nobody in it.
        Ride::Driving(car) => {
            let parked = Car { speed: 0.0, ..car };
            (Ride::Parked(parked), parked.step_out())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Block;
    use crate::world::{CHUNK_SIZE, ChunkPos};

    /// Well above anything worldgen makes, so the test floor is the only thing a car can
    /// ever find under it and no hill or tree has to be cleared out of the way.
    const FLOOR: i32 = 60;
    /// Chunks of floor in every direction from the origin: enough road either way to get a
    /// car to [`TOP_SPEED`] and still be inside the generated world.
    const CHUNKS: i32 = 4;
    /// The floor's near edge, in blocks. It runs from `-EDGE` to `EDGE + CHUNK_SIZE`.
    const EDGE: i32 = CHUNKS * CHUNK_SIZE;

    /// A wide flat stone floor at [`FLOOR`], with clear air over it.
    fn flat_world() -> World {
        let mut w = World::new(1, []);
        for cz in -CHUNKS..=CHUNKS {
            for cx in -CHUNKS..=CHUNKS {
                w.load_chunk(ChunkPos::new(cx, cz));
            }
        }
        for z in -EDGE..(EDGE + CHUNK_SIZE) {
            for x in -EDGE..(EDGE + CHUNK_SIZE) {
                w.set_block(BlockPos::new(x, FLOOR, z), Block::Stone);
                for y in FLOOR + 1..WORLD_HEIGHT.min(FLOOR + 6) {
                    w.set_block(BlockPos::new(x, y, z), Block::Air);
                }
            }
        }
        w
    }

    /// A car standing on whatever is under `(x, z)`, put there the way the game puts one.
    fn parked_at(world: &World, x: f32, z: f32, yaw: f32) -> Car {
        on_ground(world, Vec3::new(x, (WORLD_HEIGHT - 1) as f32, z), yaw)
            .expect("the test world has ground here")
    }

    /// Facing +X. Yaw is left-handed about +Y and 0 faces -Z, so a quarter turn the other
    /// way points a car down the long axis these tests build walls across.
    const EAST: f32 = -std::f32::consts::FRAC_PI_2;

    /// One second of driving, at the frame rate the game clamps to.
    fn drive_for(world: &World, mut car: Car, control: Vec2, seconds: f32) -> Car {
        let dt = 1.0 / 60.0;
        for _ in 0..(seconds / dt) as u32 {
            car = drive(world, car, control, dt);
        }
        car
    }

    /// The reason a car exists. A child who can outrun the thing they spent eight nails on
    /// has been sold a worse pair of boots.
    #[test]
    fn a_car_beats_running() {
        let w = flat_world();
        let car = parked_at(&w, 0.5, 0.5, 0.0);
        let driven = drive_for(&w, car, Vec2::new(0.0, 1.0), 3.0);
        let covered = driven.pos.distance(car.pos);
        assert!(
            covered > player::SPRINT_SPEED * 3.0,
            "three seconds of car went {covered} blocks, a sprint would go {}",
            player::SPRINT_SPEED * 3.0
        );
        assert!(
            (driven.speed - TOP_SPEED).abs() < 0.1,
            "never reached top speed: {}",
            driven.speed
        );
    }

    /// A car and its driver must mean the same thing by `yaw`, or the body everyone sees
    /// sitting in it faces one way while it drives another.
    #[test]
    fn a_car_and_a_player_face_the_same_way() {
        for yaw in [0.0, 0.7, -2.1, 3.0] {
            let mut p = player::Player::spawn_at(Vec3::ZERO);
            p.yaw = yaw;
            let (forward, right) = p.move_basis();
            let rot = Quat::from_rotation_y(yaw);
            assert!(
                (rot * Vec3::NEG_Z - forward).length() < 1e-5,
                "forward @ {yaw}"
            );
            assert!((rot * Vec3::X - right).length() < 1e-5, "right @ {yaw}");
        }
    }

    /// Steering right must turn right. Getting this backwards is the sort of thing that is
    /// obvious in the chair and invisible in the diff.
    #[test]
    fn the_stick_steers_the_way_it_points() {
        let w = flat_world();
        let car = parked_at(&w, 0.5, 0.5, 0.0);
        let right = drive_for(&w, car, Vec2::new(1.0, 0.3), 0.5);
        let forward = Quat::from_rotation_y(right.yaw) * Vec3::NEG_Z;
        assert!(right.yaw < car.yaw, "steering right raised the yaw");
        assert!(
            forward.x > 0.1,
            "turned right and did not head +X: {forward}"
        );
    }

    /// The whole of the car's collision, in one test: a step is a hill and a wall is a
    /// wall, and [`MAX_CLIMB`] is the only thing that tells them apart.
    #[test]
    fn a_step_is_drivable_and_a_wall_is_not() {
        let step_up = |height: i32| {
            let mut w = flat_world();
            // Raised ground from x=10 to the far edge, so the car ends up on top of it
            // rather than driving over it and off the other side.
            for y in FLOOR + 1..=(FLOOR + height) {
                for z in -EDGE..(EDGE + CHUNK_SIZE) {
                    for x in 10..(EDGE + CHUNK_SIZE) {
                        w.set_block(BlockPos::new(x, y, z), Block::Stone);
                    }
                }
            }
            let car = Car {
                yaw: EAST,
                ..parked_at(&w, 0.5, 0.5, 0.0)
            };
            drive_for(&w, car, Vec2::new(0.0, 1.0), 3.0)
        };

        let onto = step_up(1);
        assert!(
            onto.pos.x > 11.0,
            "a one-block step stopped the car: {onto:?}"
        );
        assert!(
            (onto.pos.y - (FLOOR + 2) as f32 - HOVER).abs() < 0.01,
            "did not end up on top of the step: y={}",
            onto.pos.y
        );

        let into = step_up(3);
        assert!(
            into.pos.x < 10.0,
            "drove up a three-block wall to x={}",
            into.pos.x
        );
        assert_eq!(into.speed, 0.0, "a wall stops the car dead");
    }

    /// Driving off the edge is a fall, not a glide: the car has to reach a real falling
    /// speed and land on what is actually below.
    #[test]
    fn driving_off_a_ledge_falls() {
        let top = FLOOR + 6;
        let mut w = flat_world();
        for y in FLOOR + 1..=top {
            for z in -EDGE..(EDGE + CHUNK_SIZE) {
                for x in -EDGE..8 {
                    w.set_block(BlockPos::new(x, y, z), Block::Stone);
                }
            }
        }
        // On the plateau at x<8, facing +X, driving off its east edge onto the floor.
        let mut car = Car {
            yaw: EAST,
            ..parked_at(&w, 0.5, 0.5, 0.0)
        };
        assert!(
            (car.pos.y - (top + 1) as f32 - HOVER).abs() < 0.01,
            "start: {car:?}"
        );

        let mut fastest_fall = 0.0f32;
        for _ in 0..4 * 60 {
            car = drive(&w, car, Vec2::new(0.0, 1.0), 1.0 / 60.0);
            fastest_fall = fastest_fall.max(car.fall);
        }
        assert!(
            fastest_fall > 5.0,
            "drifted off the edge at {fastest_fall} blocks a second rather than falling"
        );
        assert!(
            (car.pos.y - (FLOOR + 1) as f32 - HOVER).abs() < 0.01,
            "did not land on the floor: {car:?}"
        );
        assert_eq!(car.fall, 0.0, "landed and kept falling");
    }

    /// Ungenerated terrain reads as air. A car that believed it would drive off the edge of
    /// the world and never stop — the same trap `player::move_and_slide` guards against.
    #[test]
    fn a_car_stops_at_the_edge_of_the_generated_world() {
        let w = flat_world();
        let edge = (EDGE + CHUNK_SIZE) as f32;
        let car = Car {
            yaw: EAST,
            ..parked_at(&w, 0.5, 0.5, 0.0)
        };
        let driven = drive_for(&w, car, Vec2::new(0.0, 1.0), 10.0);
        assert!(
            driven.pos.x + HALF_LENGTH <= edge,
            "left the generated world at x={}",
            driven.pos.x
        );
        assert!(driven.pos.x > edge - 3.0, "stopped well short: {driven:?}");
        assert_eq!(driven.speed, 0.0);
        assert!(driven.pos.y > FLOOR as f32, "sank through unloaded ground");
    }

    /// Letting go of the stick has to stop the car — coasting for ever is a car nobody can
    /// park, and `ACCEL` doubling as the brake is the reason there is no second constant.
    #[test]
    fn letting_go_stops_the_car() {
        let w = flat_world();
        let car = parked_at(&w, 0.5, 0.5, 0.0);
        let rolling = drive_for(&w, car, Vec2::new(0.0, 1.0), 2.0);
        assert!(rolling.speed > 1.0);
        let stopped = drive_for(&w, rolling, Vec2::ZERO, 3.0);
        assert!(
            stopped.speed.abs() < 0.01,
            "still rolling: {}",
            stopped.speed
        );
    }

    #[test]
    fn reverse_is_slower_than_forward() {
        let w = flat_world();
        let car = parked_at(&w, 0.5, 0.5, 0.0);
        let back = drive_for(&w, car, Vec2::new(0.0, -1.0), 3.0);
        assert!(
            (back.speed + REVERSE_SPEED).abs() < 0.1,
            "reverse settled at {}",
            back.speed
        );
    }

    /// Put it down, get in, get out — and the car you get out of is the car you were in.
    #[test]
    fn the_ride_button_goes_in_and_out() {
        let w = flat_world();
        let feet = Vec3::new(0.5, (FLOOR + 1) as f32, 0.5);
        let car = park_in_front(&w, feet, 0.0).expect("flat ground in a loaded chunk");
        assert!(
            (car.pos.y - (FLOOR + 1) as f32 - HOVER).abs() < 0.01,
            "parked off the ground: {car:?}"
        );
        assert!(
            feet.distance(car.pos) <= BOARDING_RANGE,
            "parked out of reach"
        );

        let (driving, seated) = toggle_ride(Ride::Parked(car), feet);
        assert_eq!(driving, Ride::Driving(car));
        assert_eq!(seated, car.seat());
        assert!(seated.y > car.pos.y, "the seat is inside the floor");

        let rolling = Car { speed: 9.0, ..car };
        let (out, standing) = toggle_ride(Ride::Driving(rolling), seated);
        assert_eq!(
            out,
            Ride::Parked(car),
            "the car drove off without its driver"
        );
        assert!(
            standing.distance(car.pos) > HALF_WIDTH,
            "stepped out into the car: {standing}"
        );
    }

    /// The whole feature, in the order a child does it: put the car down, get in, drive up
    /// over a step and away, get out, and be standing on the ground next to it a long way
    /// from where you started.
    #[test]
    fn park_board_drive_and_step_out() {
        let mut w = flat_world();
        for z in -EDGE..(EDGE + CHUNK_SIZE) {
            for x in 12..(EDGE + CHUNK_SIZE) {
                w.set_block(BlockPos::new(x, FLOOR + 1, z), Block::Stone);
            }
        }
        let start = Vec3::new(0.5, (FLOOR + 1) as f32, 0.5);

        let car = park_in_front(&w, start, EAST).expect("standing on the test floor");
        let (mut ride, feet) = toggle_ride(Ride::Parked(car), start);
        assert!(
            ride.is_driving(),
            "could not get into a car parked in reach"
        );

        let mut feet = feet;
        for _ in 0..4 * 60 {
            let Ride::Driving(car) = ride else {
                unreachable!("nothing gets a driver out but the button")
            };
            let driven = drive(&w, car, Vec2::new(0.0, 1.0), 1.0 / 60.0);
            feet = driven.seat();
            ride = Ride::Driving(driven);
        }

        let (parked, standing) = toggle_ride(ride, feet);
        let Ride::Parked(car) = parked else {
            panic!("stepping out left them {parked:?}")
        };
        assert!(
            standing.x > 30.0,
            "four seconds of driving got them to x={}",
            standing.x
        );
        assert!(
            (standing.y - (FLOOR + 2) as f32 - HOVER).abs() < 0.01,
            "stepped out onto nothing: {standing}"
        );
        assert!(
            standing.distance(car.pos) > HALF_WIDTH,
            "stepped out into their own car"
        );
    }

    /// A car you have not put down is not a car, and one across the valley is not one you
    /// can climb into from here.
    #[test]
    fn you_cannot_board_what_is_not_there() {
        let feet = Vec3::new(8.0, 21.0, 8.0);
        assert_eq!(toggle_ride(Ride::Pocketed, feet), (Ride::Pocketed, feet));

        let far = Car {
            pos: feet + Vec3::X * (BOARDING_RANGE + 1.0),
            yaw: 0.0,
            speed: 0.0,
            fall: 0.0,
        };
        assert_eq!(
            toggle_ride(Ride::Parked(far), feet),
            (Ride::Parked(far), feet),
            "boarded a car from across the field"
        );
    }

    /// There is nowhere to stand a car in a chunk nobody has generated, and guessing is how
    /// one ends up buried in the hill that arrives a frame later.
    #[test]
    fn a_car_is_not_parked_on_terrain_that_does_not_exist() {
        let w = World::new(1, []);
        assert!(park_in_front(&w, Vec3::new(8.0, 40.0, 8.0), 0.0).is_none());
    }

    /// [`Ride`] is the answer to "is there a car to draw", and the two variants that carry
    /// one are exactly the two that are out in the world.
    #[test]
    fn only_a_car_in_the_world_is_drawn() {
        let car = Car {
            pos: Vec3::new(1.0, 2.0, 3.0),
            yaw: 0.5,
            speed: 0.0,
            fall: 0.0,
        };
        assert_eq!(Ride::Pocketed.car(), None);
        assert_eq!(Ride::Parked(car).car(), Some(car));
        assert_eq!(Ride::Driving(car).car(), Some(car));
        assert!(Ride::Driving(car).is_driving());
        assert!(!Ride::Parked(car).is_driving());
    }
}
