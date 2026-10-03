//! Greedily balanced left/right trees around a centered root.
use super::{
    LayoutAlgorithm, LayoutInput, LayoutOutput, TreeDirection, TreeLayoutOptions, pack_components,
    spanning::forest, tree::tidy_positions,
};
use crate::Point;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MindMapBalance {
    #[default]
    Auto,
    AllRight,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MindMapOptions {
    pub h_gap: f64,
    pub v_gap: f64,
    pub balance: MindMapBalance,
}
impl Default for MindMapOptions {
    fn default() -> Self {
        Self {
            h_gap: 80.0,
            v_gap: 24.0,
            balance: MindMapBalance::Auto,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct MindMapLayout(pub MindMapOptions);
impl LayoutAlgorithm for MindMapLayout {
    fn layout(&self, input: &LayoutInput) -> LayoutOutput {
        let f = forest(input);
        let mut output = LayoutOutput::default();
        let mut extents = vec![0.0; input.nodes.len()];
        for component in &f.components {
            for &i in component.iter().rev() {
                extents[i] = input.nodes[i].size.height.max(
                    f.children[i].iter().map(|&j| extents[j]).sum::<f64>()
                        + f.children[i].len().saturating_sub(1) as f64 * self.0.v_gap.max(0.0),
                );
            }
        }
        for &root in &f.roots {
            let mut left = Vec::new();
            let mut right = Vec::new();
            let mut lh = 0.0;
            let mut rh = 0.0;
            for &child in &f.children[root] {
                if self.0.balance == MindMapBalance::Auto && lh < rh {
                    left.push(child);
                    lh += extents[child] + self.0.v_gap.max(0.0);
                } else {
                    right.push(child);
                    rh += extents[child] + self.0.v_gap.max(0.0);
                }
            }
            for (branches, mirror) in [(left, true), (right, false)] {
                let mut children = f.children.clone();
                children[root] = branches;
                let side = tidy_positions(
                    input,
                    &children,
                    &[root],
                    &TreeLayoutOptions {
                        direction: TreeDirection::LeftRight,
                        sibling_gap: self.0.v_gap,
                        level_gap: self.0.h_gap,
                    },
                );
                for (id, p) in side.positions {
                    let Some(node) = input.nodes.iter().find(|node| node.id == id) else {
                        continue;
                    };
                    let x = if mirror {
                        -p.x - node.size.width + input.nodes[root].size.width / 2.0
                    } else {
                        p.x - input.nodes[root].size.width / 2.0
                    };
                    output.positions.insert(id, Point::new(x, p.y));
                }
            }
        }
        if f.components.len() > 1 || input.nodes.iter().any(|n| n.fixed.is_some()) {
            pack_components(input, &f.components, &mut output.positions, 80.0);
        }
        if input.nodes.iter().any(|n| n.fixed.is_some()) {
            super::remove_overlaps(input, &mut output.positions, 0.0);
        }
        output
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::tests::fixture;
    #[test]
    fn center_and_balanced_sides() {
        let input = fixture(7);
        let output = MindMapLayout::default().layout(&input);
        let root = &input.nodes[0];
        let center = output.positions[&root.id];
        assert_eq!(
            center,
            Point::new(-root.size.width / 2.0, -root.size.height / 2.0)
        );
        let left = output.positions[&input.nodes[1].id].x;
        let right = output.positions[&input.nodes[2].id].x;
        assert!(left > 0.0 && right < 0.0);
    }
    #[test]
    fn all_right() {
        let input = fixture(10);
        let output = MindMapLayout(MindMapOptions {
            balance: MindMapBalance::AllRight,
            ..Default::default()
        })
        .layout(&input);
        for node in &input.nodes[1..] {
            assert!(output.positions[&node.id].x > 0.0);
        }
    }
}
