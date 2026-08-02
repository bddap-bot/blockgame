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

use bevy::prelude::Resource;
use std::collections::HashMap;

use crate::net::PlayerId;
use crate::registry::{Item, Use};

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

    /// Is every ingredient on the shelf? An item with no recipe is never craftable — you
    /// go and find grass, you do not conjure it out of an empty hand.
    pub fn can_craft(&self, item: Item) -> bool {
        let recipe = item.recipe();
        !recipe.is_empty() && recipe.iter().all(|(g, n)| self.count(*g) >= *n)
    }

    /// Spends the recipe and adds one. False, and nothing spent, if anything is missing.
    pub fn craft(&mut self, item: Item) -> bool {
        if !self.can_craft(item) {
            return false;
        }
        for (ingredient, n) in item.recipe() {
            let paid = self.take(*ingredient, *n);
            debug_assert!(paid, "can_craft said this was affordable");
        }
        self.add(item, 1);
        true
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
        assert!(!inv.can_craft(Item::Hammer), "empty-handed");
        assert!(!inv.craft(Item::Hammer));

        for (ingredient, n) in Item::Hammer.recipe() {
            inv.add(*ingredient, *n);
        }
        assert!(inv.can_craft(Item::Hammer));
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
                assert!(!inv.can_craft(*item), "{item:?} is made from nothing");
                assert!(!inv.craft(*item));
            }
        }
        assert_eq!(inv, Inventory::default());
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
