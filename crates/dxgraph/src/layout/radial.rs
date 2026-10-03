//! Hop-distance concentric rings, ordered by parent angle and stable node key.
use super::{LayoutAlgorithm, LayoutInput, LayoutOutput, pack_components, spanning::forest};
use crate::{NodeId, Point};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    f64::consts::{PI, TAU},
};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadialOptions {
    pub focus: NodeId,
    pub ring_gap: f64,
}
impl Default for RadialOptions {
    fn default() -> Self {
        Self {
            focus: "".into(),
            ring_gap: 80.0,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct RadialLayout(pub RadialOptions);
impl LayoutAlgorithm for RadialLayout {
    fn layout(&self, input: &LayoutInput) -> LayoutOutput {
        let mut rooted = input.clone();
        if input.nodes.iter().any(|n| n.id == self.0.focus) {
            rooted.roots.retain(|r| r != &self.0.focus);
            rooted.roots.insert(0, self.0.focus.clone());
        }
        let f = forest(&rooted);
        let mut output = LayoutOutput::default();
        let mut depth = vec![0usize; input.nodes.len()];
        let mut parent = vec![None; input.nodes.len()];
        let mut angles = vec![0.0; input.nodes.len()];
        for component in &f.components {
            let mut rings = BTreeMap::<usize, Vec<usize>>::new();
            for &i in component {
                rings.entry(depth[i]).or_default().push(i);
                for &child in &f.children[i] {
                    depth[child] = depth[i] + 1;
                    parent[child] = Some(i);
                }
            }
            let mut radius = 0.0;
            let mut previous_extent = 0.0;
            for (level, mut ring) in rings {
                ring.sort_by(|&a, &b| {
                    angles[parent[a].unwrap_or(a)]
                        .partial_cmp(&angles[parent[b].unwrap_or(b)])
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| input.nodes[a].order_key.cmp(&input.nodes[b].order_key))
                        .then_with(|| input.nodes[a].id.cmp(&input.nodes[b].id))
                });
                let max_extent = ring
                    .iter()
                    .map(|&i| input.nodes[i].size.width.hypot(input.nodes[i].size.height) / 2.0)
                    .fold(0.0, f64::max);
                let circumference =
                    ring.len() as f64 * (max_extent * 2.0 + self.0.ring_gap.max(0.0));
                if level > 0 {
                    radius = (radius + previous_extent + max_extent + self.0.ring_gap.max(0.0))
                        .max(circumference / TAU + max_extent);
                }
                for (index, &i) in ring.iter().enumerate() {
                    let angle = -PI / 2.0 + TAU * index as f64 / ring.len() as f64;
                    angles[i] = angle;
                    let node = &input.nodes[i];
                    output.positions.insert(
                        node.id.clone(),
                        Point::new(
                            radius * angle.cos() - node.size.width / 2.0,
                            radius * angle.sin() - node.size.height / 2.0,
                        ),
                    );
                }
                previous_extent = max_extent;
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
    fn hop_rings_increase() {
        let input = fixture(7);
        let output = RadialLayout(RadialOptions {
            focus: input.nodes[0].id.clone(),
            ..Default::default()
        })
        .layout(&input);
        let center = |i: usize| {
            let p = output.positions[&input.nodes[i].id];
            Point::new(
                p.x + input.nodes[i].size.width / 2.0,
                p.y + input.nodes[i].size.height / 2.0,
            )
        };
        assert!(center(0).distance(Point::default()) < 0.001);
        assert!(center(1).distance(Point::default()) < center(3).distance(Point::default()));
        assert!(
            (center(1).distance(Point::default()) - center(2).distance(Point::default())).abs()
                < 0.001
        );
    }
}
