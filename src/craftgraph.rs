//! The recipe graph and where its nodes stand — the model behind [`crate::crafttree`].
//!
//! Pure arithmetic over [`crate::registry`]: no Bevy, no rendering, no input. Everything
//! here is a fact about the recipes themselves, so the layout and the way a d-pad walks it
//! can be tested without a window.
//!
//! **It is a graph, not a tree, and already was.** A nail is spent by five different
//! things, and a hammer's two ingredients sit on different rows — the layout is layered by
//! *longest* path so an edge may span more than one row, and a node may have any number of
//! edges in either direction. Nothing here counts parents or assumes one.

use crate::registry::Item;

/// Sideways distance between two neighbours on the same row, in layout units. Multiplied
/// into world space by [`crate::crafttree::COLUMN`].
const MIN_GAP: f32 = 1.0;

/// Passes of the barycentre relaxation. Four is where the current recipe set stops moving;
/// it is cheap and this runs once, when the screen opens.
const RELAX_PASSES: usize = 4;

/// One thing you can make or dig up, and where it stands.
#[derive(Debug, Clone)]
pub struct Node {
    pub item: Item,
    /// Which row: `0` is dug out of the ground, and each step up is one more craft away
    /// from it. The *longest* path, so a thing never floats below something it is made of.
    pub depth: usize,
    /// Where along the row, in layout units. Rows are centred on zero.
    pub x: f32,
    /// Nodes this one is an ingredient of.
    pub feeds: Vec<usize>,
    /// What this one is made of, and how many of each.
    pub needs: Vec<(usize, u32)>,
}

/// An ingredient arrow: `count` of `from` go into one `to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub count: u32,
}

/// Which way the d-pad was pushed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// Towards what this makes.
    Up,
    /// Towards what this is made of.
    Down,
    Left,
    Right,
}

pub struct Graph {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    /// Node indices per row, left to right.
    rows: Vec<Vec<usize>>,
}

impl Graph {
    /// The whole registry, laid out. One node per [`Item`], in declaration order, so a
    /// node index is an [`Item::index`] and nothing has to map between the two.
    pub fn of_registry() -> Graph {
        let nodes: Vec<Node> = Item::ALL
            .iter()
            .map(|item| Node {
                item: *item,
                depth: 0,
                x: 0.0,
                feeds: Vec::new(),
                needs: Vec::new(),
            })
            .collect();
        let mut graph = Graph {
            nodes,
            edges: Vec::new(),
            rows: Vec::new(),
        };
        for item in Item::ALL {
            for (ingredient, count) in item.recipe() {
                graph.edges.push(Edge {
                    from: ingredient.index(),
                    to: item.index(),
                    count: *count,
                });
            }
        }
        for edge in &graph.edges {
            graph.nodes[edge.from].feeds.push(edge.to);
            graph.nodes[edge.to].needs.push((edge.from, edge.count));
        }
        graph.rank();
        graph.place();
        graph
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    pub fn rows(&self) -> &[Vec<usize>] {
        &self.rows
    }

    /// Longest path from anything gathered, by repeated relaxation. The registry already
    /// refuses a recipe that depends on itself, so this settles rather than spinning.
    fn rank(&mut self) {
        for _ in 0..self.nodes.len() {
            let mut moved = false;
            for i in 0..self.nodes.len() {
                let want = self.nodes[i]
                    .needs
                    .iter()
                    .map(|(ingredient, _)| self.nodes[*ingredient].depth + 1)
                    .max()
                    .unwrap_or(0);
                if want > self.nodes[i].depth {
                    self.nodes[i].depth = want;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
        let deepest = self.nodes.iter().map(|n| n.depth).max().unwrap_or(0);
        self.rows = (0..=deepest)
            .map(|d| {
                (0..self.nodes.len())
                    .filter(|i| self.nodes[*i].depth == d)
                    .collect()
            })
            .collect();
    }

    /// Puts each node somewhere along its row: start evenly spread, then repeatedly pull
    /// every node towards the average of what it connects to, alternating which end of the
    /// graph is held still. Crossings drop out of the pull; the spacing pass below is what
    /// keeps two things from standing in the same place.
    fn place(&mut self) {
        for row in &self.rows {
            let n = row.len() as f32;
            for (i, node) in row.iter().enumerate() {
                self.nodes[*node].x = (i as f32 - (n - 1.0) / 2.0) * MIN_GAP;
            }
        }
        for pass in 0..RELAX_PASSES {
            // Down then up: pulling only one way drags every row towards one end of the
            // graph and leaves the other end fanned out.
            let downward = pass % 2 == 0;
            let order: Vec<usize> = if downward {
                (0..self.rows.len()).rev().collect()
            } else {
                (0..self.rows.len()).collect()
            };
            for d in order {
                let row = self.rows[d].clone();
                let targets: Vec<f32> = row
                    .iter()
                    .map(|i| {
                        let node = &self.nodes[*i];
                        let neighbours: Vec<usize> = if downward {
                            node.needs.iter().map(|(g, _)| *g).collect()
                        } else {
                            node.feeds.clone()
                        };
                        if neighbours.is_empty() {
                            node.x
                        } else {
                            neighbours.iter().map(|n| self.nodes[*n].x).sum::<f32>()
                                / neighbours.len() as f32
                        }
                    })
                    .collect();
                for (i, x) in row.iter().zip(spread(&targets)) {
                    self.nodes[*i].x = x;
                }
                self.rows[d].sort_by(|a, b| self.nodes[*a].x.total_cmp(&self.nodes[*b].x));
            }
        }
    }

    /// Everything `at` is made of, everything made from it, and itself: the part of the
    /// graph worth lighting up while the cursor is on it.
    pub fn family(&self, at: usize) -> Vec<bool> {
        let mut in_family = vec![false; self.nodes.len()];
        self.walk(at, &mut in_family, |n| {
            n.needs.iter().map(|(g, _)| *g).collect()
        });
        // Let the walk back out of `at` start again, or it stops on the mark it just made.
        in_family[at] = false;
        self.walk(at, &mut in_family, |n| n.feeds.clone());
        in_family
    }

    fn walk(&self, seed: usize, seen: &mut [bool], step: fn(&Node) -> Vec<usize>) {
        let mut stack = vec![seed];
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut seen[i], true) {
                continue;
            }
            stack.extend(step(&self.nodes[i]));
        }
    }

    /// Where the cursor lands, or `None` if that way is a wall.
    ///
    /// Sideways always works and wraps, so no press can strand a player on a dead end.
    /// Up and down travel an ingredient arrow — the lines really are the roads — taking
    /// the nearest row first and, within it, the node most nearly straight ahead.
    pub fn step(&self, at: usize, dir: Dir) -> Option<usize> {
        let here = &self.nodes[at];
        match dir {
            Dir::Left | Dir::Right => {
                let row = &self.rows[here.depth];
                let i = row.iter().position(|n| *n == at)?;
                let n = row.len();
                if n < 2 {
                    return None;
                }
                let next = match dir {
                    Dir::Right => (i + 1) % n,
                    _ => (i + n - 1) % n,
                };
                Some(row[next])
            }
            Dir::Up => self.nearest(at, here.feeds.iter().copied()),
            Dir::Down => self.nearest(at, here.needs.iter().map(|(g, _)| *g)),
        }
    }

    /// Of the nodes offered, the one on the nearest row and then the least sideways
    /// travel — so a press moves the picture as little as it can while still moving.
    fn nearest(&self, at: usize, among: impl Iterator<Item = usize>) -> Option<usize> {
        let here = &self.nodes[at];
        among.min_by(|a, b| {
            let key = |i: &usize| {
                let n = &self.nodes[*i];
                (n.depth.abs_diff(here.depth), (n.x - here.x).abs())
            };
            let (ra, xa) = key(a);
            let (rb, xb) = key(b);
            ra.cmp(&rb).then(xa.total_cmp(&xb))
        })
    }
}

/// Puts nodes where they want to be without letting two of them stand closer than
/// [`MIN_GAP`], keeping the row's centre of gravity where the pull put it.
fn spread(targets: &[f32]) -> Vec<f32> {
    let mut order: Vec<usize> = (0..targets.len()).collect();
    order.sort_by(|a, b| targets[*a].total_cmp(&targets[*b]));
    let mut out = vec![0.0; targets.len()];
    let mut last = f32::NEG_INFINITY;
    for i in &order {
        let x = targets[*i].max(last + MIN_GAP);
        out[*i] = x;
        last = x;
    }
    if targets.is_empty() {
        return out;
    }
    let drift = out.iter().sum::<f32>() / out.len() as f32
        - targets.iter().sum::<f32>() / targets.len() as f32;
    for x in &mut out {
        *x -= drift;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_item_is_a_node_at_its_own_index() {
        let g = Graph::of_registry();
        assert_eq!(g.nodes().len(), Item::ALL.len());
        for item in Item::ALL {
            assert_eq!(g.nodes()[item.index()].item, *item);
        }
    }

    #[test]
    fn what_you_dig_up_is_the_bottom_row_and_what_you_make_stands_on_it() {
        let g = Graph::of_registry();
        for node in g.nodes() {
            let gathered = node.item.recipe().is_empty();
            assert_eq!(
                node.depth == 0,
                gathered,
                "{:?} is on row {} and {} gathered",
                node.item,
                node.depth,
                if gathered { "is" } else { "is not" }
            );
        }
        // Every ingredient stands strictly below what it goes into, which is the whole
        // reason the rows are ranked by longest path rather than shortest.
        for edge in g.edges() {
            assert!(
                g.nodes()[edge.from].depth < g.nodes()[edge.to].depth,
                "{:?} feeds {:?} without being under it",
                g.nodes()[edge.from].item,
                g.nodes()[edge.to].item
            );
        }
    }

    /// The recipes are not a tree today, whatever the README calls them, and the layout
    /// has to survive both ways they are not: one thing spent by many, and one thing made
    /// of ingredients that are different distances from the ground.
    #[test]
    fn the_recipes_are_already_a_graph() {
        let g = Graph::of_registry();
        assert!(
            g.nodes().iter().any(|n| n.feeds.len() > 1),
            "nothing is an ingredient of more than one thing"
        );
        assert!(
            g.nodes().iter().any(|n| {
                let depths: HashSet<usize> =
                    n.needs.iter().map(|(i, _)| g.nodes()[*i].depth).collect();
                depths.len() > 1
            }),
            "no recipe reaches across more than one row"
        );
    }

    #[test]
    fn nothing_stands_on_top_of_anything_else() {
        let g = Graph::of_registry();
        for row in g.rows() {
            let mut xs: Vec<f32> = row.iter().map(|i| g.nodes()[*i].x).collect();
            xs.sort_by(f32::total_cmp);
            for pair in xs.windows(2) {
                assert!(
                    pair[1] - pair[0] >= MIN_GAP - 1e-3,
                    "two nodes {} apart",
                    pair[1] - pair[0]
                );
            }
        }
    }

    /// A child who starts anywhere can reach everything with the d-pad alone. This is the
    /// one property the navigation exists to have: no item is behind a press that does not
    /// exist, and no press strands you.
    #[test]
    fn the_d_pad_reaches_every_item_from_every_item() {
        let g = Graph::of_registry();
        for start in 0..g.nodes().len() {
            let mut seen = HashSet::from([start]);
            let mut stack = vec![start];
            while let Some(at) = stack.pop() {
                for dir in [Dir::Up, Dir::Down, Dir::Left, Dir::Right] {
                    if let Some(next) = g.step(at, dir)
                        && seen.insert(next)
                    {
                        stack.push(next);
                    }
                }
            }
            assert_eq!(
                seen.len(),
                g.nodes().len(),
                "starting on {:?} you cannot get everywhere",
                g.nodes()[start].item
            );
        }
    }

    #[test]
    fn sideways_wraps_round_its_row_and_comes_home() {
        let g = Graph::of_registry();
        for row in g.rows() {
            if row.len() < 2 {
                continue;
            }
            let start = row[0];
            let mut at = start;
            for _ in 0..row.len() {
                at = g.step(at, Dir::Right).expect("sideways always moves");
            }
            assert_eq!(at, start, "a lap around the row does not come back");
        }
    }

    #[test]
    fn the_lit_family_is_what_it_is_made_of_and_what_it_makes() {
        let g = Graph::of_registry();
        let family = g.family(Item::Nail.index());
        assert!(
            family[Item::Nail.index()],
            "the cursor is in its own family"
        );
        assert!(family[Item::Stone.index()], "a nail is made of stone");
        assert!(family[Item::Car.index()], "a car is made of nails");
        assert!(
            !family[Item::Sand.index()],
            "sand has nothing to do with nails"
        );
    }
}
