//! What a player has, and what they have selected.
//!
//! An inventory is a count per [`Item`] and nothing else — no slots to shuffle, no stack
//! size, no order to get wrong. Crafting is the one operation that both spends and makes:
//! [`Inventory::craft`] reads the recipe out of the registry, so a new recipe is a row in
//! that table and no code here changes.
//!
//! The host owns every inventory, its own included, and hands each player a copy of theirs
//! over the wire. A client never adds to its own pile — the same rule as blocks, for the
//! same reason: a local guess has to be rolled back the moment the host disagrees.

use bevy::prelude::{Message, Resource};
use std::collections::HashMap;

use crate::net::PlayerId;
use crate::registry::{Fall, Item, Use};

/// One player's things: a count per item, indexed by [`Item::index`].
///
/// An array rather than a map, so every item has an answer and "not in the map" is not a
/// state anything has to handle.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct Inventory([u32; Item::COUNT]);

impl Default for Inventory {
    /// You start with nothing. The first thing anyone does is break a block, and the
    /// second is find out that the block is now theirs.
    fn default() -> Self {
        Inventory::EMPTY
    }
}

impl Inventory {
    pub const EMPTY: Inventory = Inventory([0; Item::COUNT]);

    pub fn count(&self, item: Item) -> u32 {
        self.0[item.index()]
    }

    /// Saturating, because a count that wraps to nothing after a long afternoon of digging
    /// is a worse bug than a pile that stops growing.
    pub fn add(&mut self, item: Item, n: u32) {
        let slot = &mut self.0[item.index()];
        *slot = slot.saturating_add(n);
    }

    /// Spends `n`, or nothing at all and reports it. All-or-nothing so a half-paid recipe
    /// is not a state that exists.
    pub fn take(&mut self, item: Item, n: u32) -> bool {
        let slot = &mut self.0[item.index()];
        match slot.checked_sub(n) {
            Some(left) => {
                *slot = left;
                true
            }
            None => false,
        }
    }

    /// Spends the recipe and adds one. False, and nothing spent, if anything is missing.
    ///
    /// One step, and not the question a player ever asks: an item with no recipe is never
    /// craftable — you go and find grass, you do not conjure it out of an empty hand — and
    /// making a car out of a hillside is [`Inventory::build`] running a whole run of these.
    fn craft(&mut self, item: Item) -> bool {
        let recipe = item.recipe();
        if recipe.is_empty() || !recipe.iter().all(|(g, n)| self.count(*g) >= *n) {
            return false;
        }
        for (ingredient, n) in recipe {
            let paid = self.take(*ingredient, *n);
            debug_assert!(paid, "the shelf was just counted");
        }
        self.add(item, 1);
        true
    }

    /// Everything that has to happen for this player to end up holding one `want`.
    ///
    /// The whole recipe graph is walked, not just the top row: asking for a car with ten
    /// stone and six wood in the pocket comes back with eight nails and then the car,
    /// because a child who has to be told "make eight nails first" is a child reading a
    /// recipe rather than building a car.
    ///
    /// What is already on the shelf is spent before anything is made — the nail you have
    /// is a nail you do not make — and the requested item itself is always made fresh,
    /// which is what asking to craft one means.
    pub fn plan(&self, want: Item) -> Plan {
        let mut shelf = self.clone();
        let mut plan = Plan::default();
        if want.recipe().is_empty() {
            // Nothing makes grass. The shortfall *is* the request, which is what the
            // screen draws as "go and dig this up".
            plan.short.push((want, 1));
            return plan;
        }
        for (ingredient, n) in want.recipe() {
            for _ in 0..*n {
                obtain(&mut shelf, *ingredient, &mut plan);
            }
        }
        plan.steps.push(want);
        plan
    }

    /// Runs [`Inventory::plan`] to the end, or changes nothing at all.
    ///
    /// All-or-nothing for the same reason [`Inventory::craft`] is: a child who asks for a
    /// car and gets six nails and no car has been charged for something they did not get.
    /// Returns what was made, in the order it was made, which is what the screen replays.
    pub fn build(&mut self, want: Item) -> Vec<Item> {
        let plan = self.plan(want);
        if !plan.short.is_empty() {
            return Vec::new();
        }
        for step in &plan.steps {
            let made = self.craft(*step);
            debug_assert!(made, "the plan said {step:?} was affordable by now");
        }
        plan.steps
    }

    /// What is actually in this player's hand: the item they have selected, or nothing if
    /// they have none of it.
    ///
    /// Selection is a cursor over the whole registry — pointing at a car is how you read
    /// what a car costs — so pointing at one is not holding one. Everything that asks
    /// "what are they holding" asks here: the model in their hand, and the behaviour of
    /// the use button, must agree.
    pub fn in_hand(&self, selected: Item) -> Option<Item> {
        Some(selected).filter(|item| self.count(*item) > 0)
    }

    /// What the use button does for this player. A hand, unless they are holding a tool.
    pub fn using(&self, selected: Item) -> Use {
        self.in_hand(selected).map_or(Use::BARE_HAND, Item::using)
    }

    /// How this player falls. Bare, unless they are actually holding an equippable — a
    /// parachute they have not built yet must not break the fall off the tower they built
    /// instead of building it.
    pub fn falling(&self, selected: Item) -> Fall {
        self.in_hand(selected).map_or(Fall::UNAIDED, Item::falling)
    }

    /// How far this player may reach, whatever they have selected: the furthest any tool
    /// they own works.
    ///
    /// This is the host's rule, and it is deliberately answered from the pile rather than
    /// from what somebody says is in their hand — the pile is the host's own record, and a
    /// selection is a claim it has no way to check. The slack it buys a cheat is the range
    /// of a gun they own and could simply have selected, which is no range at all.
    pub fn reach(&self) -> f32 {
        Item::ALL
            .iter()
            .filter(|item| self.count(**item) > 0)
            .map(|item| item.using().reach)
            .fold(Use::BARE_HAND.reach, f32::max)
    }

    /// Everything held, for the wire. Empty piles are left out: an inventory is mostly
    /// zeroes, and [`Inventory::from_contents`] starts from zero anyway.
    pub fn contents(&self) -> Vec<(Item, u32)> {
        Item::ALL
            .iter()
            .copied()
            .filter(|i| self.count(*i) > 0)
            .map(|i| (i, self.count(i)))
            .collect()
    }

    /// The whole inventory a host just sent. Absolute, never a delta: a lost or reordered
    /// update then cannot leave a client's pile drifting from the host's for ever.
    pub fn from_contents(contents: impl IntoIterator<Item = (Item, u32)>) -> Inventory {
        let mut inv = Inventory::default();
        for (item, n) in contents {
            inv.0[item.index()] = n;
        }
        inv
    }
}

/// What it takes to make one of something, out of what a player already has.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Every craft to run, in an order each of them can be paid for. The thing that was
    /// asked for is the last one.
    pub steps: Vec<Item>,
    /// What ran out on the way: the gathered things to go and dig up, and how many more of
    /// each. Empty means the plan can be run right now.
    pub short: Vec<(Item, u32)>,
}

/// Takes one `item` off the shelf, making it — and whatever *it* is made of — if the shelf
/// has none.
///
/// Recursion terminates because a recipe cannot reach itself, which
/// `registry::no_recipe_depends_on_itself` is what keeps true; a cycle added to the table
/// fails that test long before it reaches a player.
fn obtain(shelf: &mut Inventory, item: Item, plan: &mut Plan) {
    if shelf.take(item, 1) {
        return;
    }
    if item.recipe().is_empty() {
        match plan.short.iter_mut().find(|(short, _)| *short == item) {
            Some((_, n)) => *n += 1,
            None => plan.short.push((item, 1)),
        }
        return;
    }
    for (ingredient, n) in item.recipe() {
        for _ in 0..*n {
            obtain(shelf, *ingredient, plan);
        }
    }
    // Made and spent in the same breath: the caller wanted one in order to use it, so it
    // never reaches the shelf and there is nothing to take back off.
    plan.steps.push(item);
}

/// Every inventory the game knows about, keyed by whose it is.
///
/// On the host that is everybody's, because the host is the one whose word decides what
/// anybody has — its own included, at its own id, so the single-player case is not a
/// second path. On a peer it holds exactly one entry: its own, as the host last stated
/// it. Either way "what do I have" is the same lookup.
#[derive(Resource, Default)]
pub struct Inventories(pub HashMap<PlayerId, Inventory>);

impl Inventories {
    /// One player's things. Nobody has been credited with anything yet is the same answer
    /// as an empty pile, so there is no "not known yet" for a caller to handle.
    pub fn of(&self, who: PlayerId) -> &Inventory {
        self.0.get(&who).unwrap_or(&Inventory::EMPTY)
    }
}

/// Somebody asked to be made one of these, whatever it takes.
///
/// Written by the screen the player pressed the button on and read by whoever is entitled
/// to spend a pile — the host, or a peer's request to one. A message rather than a call, so
/// the screen that asks does not have to know which of those it is talking to.
#[derive(Message, Debug, Clone, Copy)]
pub struct Craft(pub Item);

/// Whose things the screen in front of this player is showing.
///
/// The [`crate::net::Session`] is the authority on it and this is read off the session the
/// moment there is a world, once, because a player's id never changes for the life of a
/// session. It exists so that a screen only wants to know "what do *I* have" does not have
/// to hold the network to ask.
#[derive(Resource, Debug, Clone, Copy)]
pub struct Whoami(pub PlayerId);

/// Which item the local player has selected — the hotbar cursor.
///
/// Separate from the inventory because you may point at something you have none of: that
/// is how you read its recipe and decide to make one.
#[derive(Resource, Debug, Clone, Copy)]
pub struct Held(pub Item);

impl Default for Held {
    fn default() -> Self {
        Held(Item::ALL[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Block;

    #[test]
    fn taking_more_than_you_have_takes_nothing() {
        let mut inv = Inventory::default();
        inv.add(Item::Wood, 2);
        assert!(!inv.take(Item::Wood, 3));
        assert_eq!(inv.count(Item::Wood), 2, "a refused take spends nothing");
        assert!(inv.take(Item::Wood, 2));
        assert_eq!(inv.count(Item::Wood), 0);
    }

    #[test]
    fn a_pile_does_not_wrap_round_to_nothing() {
        let mut inv = Inventory::default();
        inv.add(Item::Dirt, u32::MAX);
        inv.add(Item::Dirt, 10);
        assert_eq!(inv.count(Item::Dirt), u32::MAX);
    }

    /// The whole crafting mechanism: point at a thing, pay for it, have it.
    #[test]
    fn crafting_spends_the_recipe_and_makes_one() {
        let mut inv = Inventory::default();
        assert!(!inv.craft(Item::Hammer));

        for (ingredient, n) in Item::Hammer.recipe() {
            inv.add(*ingredient, *n);
        }

        assert!(inv.craft(Item::Hammer));
        assert_eq!(inv.count(Item::Hammer), 1);
        for (ingredient, _) in Item::Hammer.recipe() {
            assert_eq!(inv.count(*ingredient), 0, "{ingredient:?} was not paid for");
        }
        assert!(
            !inv.craft(Item::Hammer),
            "and you cannot make a second free"
        );
    }

    /// Missing one ingredient must cost nothing at all — the failure mode that eats a
    /// child's whole pile of wood and hands back no hammer.
    #[test]
    fn a_recipe_you_cannot_afford_costs_nothing() {
        let mut inv = Inventory::default();
        inv.add(Item::Wood, 9);
        assert!(!inv.craft(Item::Hammer), "no nails");
        assert_eq!(inv.count(Item::Wood), 9);
    }

    /// Gathered things are found, not made. Without this a player crafts grass out of an
    /// empty inventory, for ever.
    #[test]
    fn what_you_gather_cannot_be_crafted() {
        let mut inv = Inventory::default();
        for item in Item::ALL {
            if item.recipe().is_empty() {
                assert!(!inv.craft(*item), "{item:?} is made from nothing");
                assert!(
                    inv.build(*item).is_empty(),
                    "{item:?} was built out of nothing"
                );
            }
        }
        assert_eq!(inv, Inventory::default());
    }

    /// The whole point of planning: a pocket full of rock and timber is a car, and nobody
    /// has to be told that eight nails come first.
    #[test]
    fn a_plan_makes_what_the_recipe_needs_before_it_needs_it() {
        let mut inv = Inventory::default();
        inv.add(Item::Wood, 6);
        inv.add(Item::Stone, 10);

        let plan = inv.plan(Item::Car);
        assert!(plan.short.is_empty(), "ten stone and six wood is a car");
        assert_eq!(plan.steps, [vec![Item::Nail; 8], vec![Item::Car]].concat());
        for (i, step) in plan.steps.iter().enumerate() {
            assert!(
                inv.craft(*step),
                "step {i} ({step:?}) could not be paid for"
            );
        }
        assert_eq!(inv.count(Item::Car), 1);
        assert_eq!(inv.count(Item::Stone), 0, "every stone went into a nail");
    }

    /// What is already on the shelf is spent before anything is made. A child who spent
    /// the afternoon making nails must not watch the game make eight more.
    #[test]
    fn a_plan_spends_what_you_already_have() {
        let mut inv = Inventory::default();
        inv.add(Item::Wood, 6);
        inv.add(Item::Stone, 2);
        inv.add(Item::Nail, 8);
        assert_eq!(
            inv.plan(Item::Car).steps,
            vec![Item::Car],
            "nothing to make"
        );
    }

    /// Short of the raw stuff, the plan says *what to go and dig up* and how much more of
    /// it — the only thing a player who cannot read the recipe can act on.
    #[test]
    fn a_plan_counts_what_is_missing_right_down_to_the_ground() {
        let inv = Inventory::default();
        let plan = inv.plan(Item::Car);
        assert_eq!(
            plan.short,
            vec![(Item::Wood, 6), (Item::Stone, 10)],
            "six wood, and ten stone: two for the frame and one for each nail"
        );
        assert_eq!(
            Inventory::default().plan(Item::Grass).short,
            vec![(Item::Grass, 1)],
            "nothing makes grass — you go and find it"
        );
    }

    /// All-or-nothing, over the whole chain: a refused build must leave the pile exactly
    /// as it was, half-made nails included.
    #[test]
    fn a_build_you_cannot_afford_spends_nothing() {
        let mut inv = Inventory::default();
        inv.add(Item::Stone, 40);
        let before = inv.clone();
        assert!(inv.build(Item::Car).is_empty(), "no wood");
        assert_eq!(inv, before);
    }

    /// One press, and the thing is in your hand — every intermediate step run, and only
    /// the intermediates that were actually needed left over.
    #[test]
    fn one_build_walks_the_whole_graph() {
        let mut inv = Inventory::default();
        inv.add(Item::Wood, 6);
        inv.add(Item::Stone, 10);
        let made = inv.build(Item::Car);
        assert_eq!(made.last(), Some(&Item::Car));
        assert_eq!(made.len(), 9, "eight nails and the car");
        assert_eq!(inv.count(Item::Car), 1);
        assert_eq!(inv.count(Item::Nail), 0, "every nail went into the car");
    }

    /// The long way round the requirement sheet: dig, make nails, make a car. It is the
    /// progression a child actually plays, and it has to close.
    #[test]
    fn a_car_can_be_built_from_blocks_out_of_the_ground() {
        let mut inv = Inventory::default();
        for (block, n) in [(Block::Wood, 6), (Block::Stone, 10)] {
            let item = Item::placing(block).expect("a gathered block is an item");
            inv.add(item, n);
        }
        for _ in 0..8 {
            assert!(inv.craft(Item::Nail), "a nail is one stone");
        }
        assert!(inv.craft(Item::Car));
        assert_eq!(inv.count(Item::Car), 1);
    }

    /// Selecting a thing you have none of is how you read its recipe, so it must not also
    /// hand you the thing: a rifle you have not built cannot be aimed with.
    #[test]
    fn pointing_at_a_rifle_is_not_holding_one() {
        let mut inv = Inventory::default();
        assert_eq!(inv.in_hand(Item::Rifle), None);
        assert_eq!(inv.using(Item::Rifle), Use::BARE_HAND);

        inv.add(Item::Rifle, 1);
        assert_eq!(inv.in_hand(Item::Rifle), Some(Item::Rifle));
        assert_eq!(inv.using(Item::Rifle), Item::Rifle.using());
    }

    /// The same rule as the rifle's, on the way down: a parachute you have not built is a
    /// hotbar cell you are reading the recipe of, and reading a recipe mid-fall must not
    /// break the fall.
    #[test]
    fn pointing_at_a_parachute_is_not_holding_one() {
        let mut inv = Inventory::default();
        assert_eq!(inv.falling(Item::Parachute), Fall::UNAIDED);

        inv.add(Item::Parachute, 1);
        assert_eq!(inv.falling(Item::Parachute), Item::Parachute.falling());
        assert_eq!(
            inv.falling(Item::Rifle),
            Fall::UNAIDED,
            "owning one is not holding it either"
        );
    }

    /// The host's reach rule. It is answered from the pile because the pile is the one
    /// thing about a peer the host actually knows — and it must not be answered by an
    /// empty-handed player owning nothing.
    #[test]
    fn reach_is_the_best_thing_you_own() {
        let mut inv = Inventory::default();
        assert_eq!(inv.reach(), Use::BARE_HAND.reach, "an arm is the floor");

        inv.add(Item::Hammer, 1);
        assert_eq!(inv.reach(), Use::BARE_HAND.reach, "a hammer is swung");

        inv.add(Item::Handgun, 1);
        assert_eq!(inv.reach(), Item::Handgun.using().reach);

        inv.add(Item::Rifle, 1);
        assert_eq!(inv.reach(), Item::Rifle.using().reach, "the best you own");

        assert!(inv.take(Item::Rifle, 1));
        assert_eq!(inv.reach(), Item::Handgun.using().reach, "and it is gone");
    }

    /// The wire round trip. Absolute state, so what a peer decodes is exactly the pile the
    /// host holds — including the items it has none of.
    #[test]
    fn an_inventory_survives_the_wire() {
        let mut inv = Inventory::default();
        inv.add(Item::Nail, 3);
        inv.add(Item::Car, 1);
        assert_eq!(Inventory::from_contents(inv.contents()), inv);
        assert_eq!(
            inv.contents(),
            vec![(Item::Nail, 3), (Item::Car, 1)],
            "empty piles do not ride the wire"
        );
    }

    /// An update replaces the pile rather than adding to it: a resent or reordered
    /// message must not leave a client believing it has twice what it has.
    #[test]
    fn an_update_replaces_rather_than_accumulates() {
        let mut inv = Inventory::default();
        inv.add(Item::Stone, 40);
        let smaller = Inventory::from_contents([(Item::Stone, 1)]);
        assert_eq!(smaller.count(Item::Stone), 1);
        assert_eq!(
            Inventory::from_contents(smaller.contents()),
            smaller,
            "twice is the same as once"
        );
    }
}
