//! Non-layered variable-size tidy trees using contour threads and deferred shifts.
// Algorithm adapted from d3-flextree by Chris Maloney (WTFPL v2),
// https://github.com/Klortho/d3-flextree/blob/master/src/flextree.js,
// implementing van der Ploeg, "Drawing non-layered tidy trees in linear time".
use super::{LayoutAlgorithm, LayoutInput, LayoutOutput, pack_components, spanning::forest};
use crate::Point;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TreeDirection {
    #[default]
    TopDown,
    LeftRight,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TreeLayoutOptions {
    pub direction: TreeDirection,
    pub sibling_gap: f64,
    pub level_gap: f64,
}
impl Default for TreeLayoutOptions {
    fn default() -> Self {
        Self {
            direction: TreeDirection::TopDown,
            sibling_gap: 24.0,
            level_gap: 64.0,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct TreeLayout(pub TreeLayoutOptions);
#[derive(Clone, Default)]
struct Flex {
    breadth: f64,
    depth: f64,
    y: f64,
    x: f64,
    rel: f64,
    prelim: f64,
    shift: f64,
    change: f64,
    left_extreme: usize,
    right_extreme: usize,
    left_extreme_rel: f64,
    right_extreme_rel: f64,
    left_thread: Option<usize>,
    right_thread: Option<usize>,
}
impl LayoutAlgorithm for TreeLayout {
    fn layout(&self, input: &LayoutInput) -> LayoutOutput {
        let f = forest(input);
        let mut output = tidy_positions(input, &f.children, &f.roots, &self.0);
        pack_components(
            input,
            &f.components,
            &mut output.positions,
            self.0.sibling_gap.max(0.0) + 60.0,
        );
        if input.nodes.iter().any(|n| n.fixed.is_some()) {
            super::remove_overlaps(input, &mut output.positions, 0.0);
        }
        output
    }
}
pub(super) fn tidy_positions(
    input: &LayoutInput,
    children: &[Vec<usize>],
    roots: &[usize],
    options: &TreeLayoutOptions,
) -> LayoutOutput {
    let mut arena: Vec<_> = input
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let (breadth, depth) = match options.direction {
                TreeDirection::TopDown => (n.size.width, n.size.height),
                TreeDirection::LeftRight => (n.size.height, n.size.width),
            };
            Flex {
                breadth: breadth.max(0.0),
                depth: depth.max(0.0) + options.level_gap.max(0.0),
                left_extreme: i,
                right_extreme: i,
                ..Default::default()
            }
        })
        .collect();
    let mut output = LayoutOutput::default();
    for &root in roots {
        let mut order = Vec::new();
        let mut stack = vec![root];
        while let Some(i) = stack.pop() {
            order.push(i);
            for &child in children[i].iter().rev() {
                arena[child].y = arena[i].y + arena[i].depth;
                stack.push(child);
            }
        }
        for &i in order.iter().rev() {
            let mut lows = Vec::<(f64, usize)>::new();
            for (index, &kid) in children[i].iter().enumerate() {
                let extreme = if index == 0 {
                    arena[kid].left_extreme
                } else {
                    arena[kid].right_extreme
                };
                let low_y = bottom(&arena, extreme);
                if index > 0 {
                    separate(
                        &mut arena,
                        children,
                        i,
                        index,
                        &lows,
                        options.sibling_gap.max(0.0),
                    );
                }
                while lows.last().is_some_and(|&(y, _)| low_y >= y) {
                    lows.pop();
                }
                lows.push((low_y, index));
            }
            let mut shift_sum = 0.0;
            let mut change_sum = 0.0;
            for &child in &children[i] {
                shift_sum += arena[child].shift;
                change_sum += shift_sum + arena[child].change;
                arena[child].rel += change_sum;
            }
            if let (Some(&first), Some(&last)) = (children[i].first(), children[i].last()) {
                arena[i].prelim = (arena[first].prelim + arena[first].rel
                    - arena[first].breadth / 2.0
                    + arena[last].rel
                    + arena[last].prelim
                    + arena[last].breadth / 2.0)
                    / 2.0;
                arena[i].left_extreme = arena[first].left_extreme;
                arena[i].left_extreme_rel = arena[first].left_extreme_rel;
                arena[i].right_extreme = arena[last].right_extreme;
                arena[i].right_extreme_rel = arena[last].right_extreme_rel;
            }
        }
        let mut stack = vec![(root, -arena[root].rel - arena[root].prelim)];
        while let Some((i, previous_sum)) = stack.pop() {
            let sum = previous_sum + arena[i].rel;
            arena[i].x = sum + arena[i].prelim;
            let breadth_corner = arena[i].x - arena[i].breadth / 2.0;
            let point = match options.direction {
                TreeDirection::TopDown => Point::new(breadth_corner, arena[i].y),
                TreeDirection::LeftRight => Point::new(arena[i].y, breadth_corner),
            };
            output.positions.insert(input.nodes[i].id.clone(), point);
            for &child in children[i].iter().rev() {
                stack.push((child, sum));
            }
        }
    }
    output
}
fn bottom(arena: &[Flex], i: usize) -> f64 {
    arena[i].y + arena[i].depth
}
fn next_left(arena: &[Flex], children: &[Vec<usize>], i: usize) -> Option<usize> {
    children[i].first().copied().or(arena[i].left_thread)
}
fn next_right(arena: &[Flex], children: &[Vec<usize>], i: usize) -> Option<usize> {
    children[i].last().copied().or(arena[i].right_thread)
}
fn separate(
    arena: &mut [Flex],
    children: &[Vec<usize>],
    parent: usize,
    index: usize,
    lows: &[(f64, usize)],
    gap: f64,
) {
    let sibling = children[parent][index - 1];
    let current = children[parent][index];
    let mut right = Some(sibling);
    let mut left = Some(current);
    let mut rmod = arena[sibling].rel;
    let mut lmod = arena[current].rel;
    let mut first = true;
    let mut low_index = lows.len().saturating_sub(1);
    while let (Some(r), Some(l)) = (right, left) {
        while low_index > 0 && bottom(arena, r) > lows[low_index].0 {
            low_index -= 1;
        }
        let distance = rmod + arena[r].prelim - lmod - arena[l].prelim
            + (arena[r].breadth + arena[l].breadth) / 2.0
            + gap;
        if distance > 0.0 || (distance < 0.0 && first) {
            lmod += distance;
            arena[current].rel += distance;
            arena[current].left_extreme_rel += distance;
            arena[current].right_extreme_rel += distance;
            let left_sibling = lows.get(low_index).map(|&(_, i)| i).unwrap_or(index - 1);
            let count = index - left_sibling;
            if count > 1 {
                let delta = distance / count as f64;
                arena[children[parent][left_sibling + 1]].shift += delta;
                arena[current].shift -= delta;
                arena[current].change -= distance - delta;
            }
        }
        first = false;
        let rb = bottom(arena, r);
        let lb = bottom(arena, l);
        if rb <= lb {
            right = next_right(arena, children, r);
            if let Some(r) = right {
                rmod += arena[r].rel;
            }
        }
        if rb >= lb {
            left = next_left(arena, children, l);
            if let Some(l) = left {
                lmod += arena[l].rel;
            }
        }
    }
    if let (None, Some(l)) = (right, left) {
        let first_child = children[parent][0];
        let extreme = arena[first_child].left_extreme;
        arena[extreme].left_thread = Some(l);
        let difference = lmod - arena[l].rel - arena[first_child].left_extreme_rel;
        arena[extreme].rel += difference;
        arena[extreme].prelim -= difference;
        arena[first_child].left_extreme = arena[current].left_extreme;
        arena[first_child].left_extreme_rel = arena[current].left_extreme_rel;
    } else if let (Some(r), None) = (right, left) {
        let extreme = arena[current].right_extreme;
        arena[extreme].right_thread = Some(r);
        let difference = rmod - arena[r].rel - arena[current].right_extreme_rel;
        arena[extreme].rel += difference;
        arena[extreme].prelim -= difference;
        arena[current].right_extreme = arena[sibling].right_extreme;
        arena[current].right_extreme_rel = arena[sibling].right_extreme_rel;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Size,
        layout::tests::{assert_clear, fixture},
    };
    #[test]
    fn chain_and_star_golden() {
        let mut input = fixture(3);
        for node in &mut input.nodes {
            node.size = Size::new(40.0, 20.0);
        }
        input.nodes[2].layout_parent = Some(input.nodes[1].id.clone());
        let output = TreeLayout::default().layout(&input);
        assert_eq!(output.positions[&input.nodes[0].id], Point::new(0.0, 0.0));
        assert_eq!(output.positions[&input.nodes[1].id], Point::new(0.0, 84.0));
        assert_eq!(output.positions[&input.nodes[2].id], Point::new(0.0, 168.0));
        input.nodes[2].layout_parent = Some(input.nodes[0].id.clone());
        let output = TreeLayout::default().layout(&input);
        assert_eq!(output.positions[&input.nodes[0].id], Point::new(32.0, 0.0));
        assert_eq!(output.positions[&input.nodes[1].id], Point::new(0.0, 84.0));
        assert_eq!(output.positions[&input.nodes[2].id], Point::new(64.0, 84.0));
    }
    #[test]
    fn generated_variable_size_trees() {
        for seed in 0..20 {
            let mut input = fixture(100);
            let mut rng = super::super::rng::Rng::new(seed);
            for i in 1..input.nodes.len() {
                input.nodes[i].size =
                    Size::new(20.0 + rng.next() * 180.0, 20.0 + rng.next() * 120.0);
                input.nodes[i].layout_parent =
                    Some(input.nodes[(rng.next() * i as f64) as usize].id.clone());
            }
            for direction in [TreeDirection::TopDown, TreeDirection::LeftRight] {
                assert_clear(
                    &input,
                    &TreeLayout(TreeLayoutOptions {
                        direction,
                        ..Default::default()
                    })
                    .layout(&input),
                );
            }
        }
    }
    #[test]
    #[ignore = "performance smoke test"]
    fn three_hundred_nodes() {
        let input = fixture(300);
        let start = std::time::Instant::now();
        TreeLayout::default().layout(&input);
        let elapsed = start.elapsed();
        eprintln!("300-node tree: {elapsed:?}");
        assert!(elapsed < std::time::Duration::from_millis(50));
    }
}
