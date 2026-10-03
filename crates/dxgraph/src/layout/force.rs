//! Seeded force simulation with warm-start anchors and rectangle collision removal.
use super::{
    LayoutAlgorithm, LayoutInput, LayoutOutput, pack_components, remove_overlaps, rng::Rng,
};
use crate::{Point, Rect};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForceOptions {
    pub iterations: usize,
    pub link_distance: f64,
    pub charge: f64,
    pub collision_padding: f64,
    pub gravity: f64,
    pub seed: u64,
}
impl Default for ForceOptions {
    fn default() -> Self {
        Self {
            iterations: 300,
            link_distance: 160.0,
            charge: 1800.0,
            collision_padding: 12.0,
            gravity: 0.015,
            seed: 1,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct ForceLayout(pub ForceOptions);
impl LayoutAlgorithm for ForceLayout {
    fn layout(&self, input: &LayoutInput) -> LayoutOutput {
        let n = input.nodes.len();
        if n == 0 {
            return LayoutOutput::default();
        }
        let by_id: BTreeMap<_, _> = input
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (&n.id, i))
            .collect();
        let mut links: Vec<_> = input
            .edges
            .iter()
            .filter_map(|e| {
                Some((
                    *by_id.get(&e.source)?,
                    *by_id.get(&e.target)?,
                    e.weight.max(0.0),
                ))
            })
            .collect();
        for (i, node) in input.nodes.iter().enumerate() {
            if let Some(&parent) = node.layout_parent.as_ref().and_then(|p| by_id.get(p))
                && !links
                    .iter()
                    .any(|&(a, b, _)| (a == i && b == parent) || (a == parent && b == i))
            {
                links.push((parent, i, 1.0));
            }
        }
        let mut rng = Rng::new(self.0.seed);
        let mut positions = Vec::with_capacity(n);
        for (i, node) in input.nodes.iter().enumerate() {
            let position = node.fixed.or(node.previous).unwrap_or_else(|| {
                let neighbors: Vec<_> = links
                    .iter()
                    .filter_map(|&(a, b, _)| {
                        let j = if a == i {
                            b
                        } else if b == i {
                            a
                        } else {
                            return None;
                        };
                        input.nodes[j].fixed.or(input.nodes[j].previous)
                    })
                    .collect();
                if neighbors.is_empty() {
                    let angle = i as f64 * 2.399963229728653;
                    let radius = 30.0 * (i as f64 + 1.0).sqrt();
                    Point::new(radius * angle.cos(), radius * angle.sin())
                } else {
                    Point::new(
                        neighbors.iter().map(|p| p.x).sum::<f64>() / neighbors.len() as f64
                            + (rng.next() - 0.5) * self.0.link_distance,
                        neighbors.iter().map(|p| p.y).sum::<f64>() / neighbors.len() as f64
                            + (rng.next() - 0.5) * self.0.link_distance,
                    )
                }
            });
            positions.push(position);
        }
        let mut velocity = vec![Point::default(); n];
        let anchored = warm_anchors(input, self.0.collision_padding.max(0.0));
        for iteration in 0..self.0.iterations {
            let alpha = (1.0 - iteration as f64 / self.0.iterations.max(1) as f64).powi(2);
            let mut acceleration = vec![Point::default(); n];
            let centroid = Point::new(
                positions.iter().map(|p| p.x).sum::<f64>() / n as f64,
                positions.iter().map(|p| p.y).sum::<f64>() / n as f64,
            );
            let centers: Vec<_> = positions
                .iter()
                .zip(&input.nodes)
                .map(|(p, node)| {
                    Point::new(p.x + node.size.width / 2.0, p.y + node.size.height / 2.0)
                })
                .collect();
            let charge = self.0.charge.max(0.0) * alpha;
            for i in 0..n {
                let center = centers[i];
                let (before, after) = acceleration.split_at_mut(i + 1);
                let current = &mut before[i];
                for (other, force) in centers[i + 1..].iter().zip(after) {
                    let mut dx = other.x - center.x;
                    let mut dy = other.y - center.y;
                    if dx.abs() + dy.abs() < 0.001 {
                        dx = (rng.next() - 0.5) * 0.01;
                        dy = (rng.next() - 0.5) * 0.01;
                    }
                    let squared = (dx * dx + dy * dy).max(25.0);
                    let strength = charge / squared;
                    let len = squared.sqrt();
                    let fx = dx / len * strength;
                    let fy = dy / len * strength;
                    current.x -= fx;
                    current.y -= fy;
                    force.x += fx;
                    force.y += fy;
                }
            }
            for &(a, b, weight) in &links {
                if a == b {
                    continue;
                }
                let dx = positions[b].x - positions[a].x;
                let dy = positions[b].y - positions[a].y;
                let distance = dx.hypot(dy).max(0.01);
                let desired = self.0.link_distance.max(1.0);
                let force = ((distance - desired) * 0.04 * weight * alpha).clamp(-20.0, 20.0);
                let fx = dx / distance * force;
                let fy = dy / distance * force;
                acceleration[a].x += fx;
                acceleration[a].y += fy;
                acceleration[b].x -= fx;
                acceleration[b].y -= fy;
            }
            for i in 0..n {
                if anchored[i] {
                    continue;
                }
                acceleration[i].x +=
                    (centroid.x - positions[i].x) * self.0.gravity.max(0.0) * alpha;
                acceleration[i].y +=
                    (centroid.y - positions[i].y) * self.0.gravity.max(0.0) * alpha;
                // Damped Verlet-style velocity integration, with a bounded step.
                velocity[i].x = (velocity[i].x * 0.7 + acceleration[i].x).clamp(-30.0, 30.0);
                velocity[i].y = (velocity[i].y * 0.7 + acceleration[i].y).clamp(-30.0, 30.0);
                positions[i].x += velocity[i].x;
                positions[i].y += velocity[i].y;
            }
        }
        let mut output = LayoutOutput {
            positions: input
                .nodes
                .iter()
                .zip(positions)
                .map(|(node, p)| (node.id.clone(), p))
                .collect(),
        };
        // Existing positions act as temporary pins during incremental exploration.
        // New nodes absorb collision displacement, preserving the mental map.
        let mut collision_input = input.clone();
        for (node, anchored) in collision_input.nodes.iter_mut().zip(anchored) {
            if anchored {
                node.fixed = node.fixed.or(node.previous);
            }
        }
        remove_overlaps(
            &collision_input,
            &mut output.positions,
            self.0.collision_padding.max(0.0),
        );
        if input.nodes.iter().all(|n| n.previous.is_none()) {
            pack_components(
                input,
                &components(n, &links),
                &mut output.positions,
                super::COMPONENT_GAP,
            );
        }
        output
    }
}

fn components(count: usize, links: &[(usize, usize, f64)]) -> Vec<Vec<usize>> {
    let mut adjacency = vec![Vec::new(); count];
    for &(a, b, _) in links {
        adjacency[a].push(b);
        adjacency[b].push(a);
    }
    let mut visited = vec![false; count];
    let mut components = Vec::new();
    for root in 0..count {
        if visited[root] {
            continue;
        }
        visited[root] = true;
        let mut component = Vec::new();
        let mut pending = std::collections::VecDeque::from([root]);
        while let Some(node) = pending.pop_front() {
            component.push(node);
            for &next in &adjacency[node] {
                if !visited[next] {
                    visited[next] = true;
                    pending.push_back(next);
                }
            }
        }
        components.push(component);
    }
    components
}
// Measurement can enlarge an existing node. Previous positions are soft anchors:
// release affected nodes when their rectangles already overlap, while retaining
// user pins and the stable positions of unaffected neighbors.
fn warm_anchors(input: &LayoutInput, padding: f64) -> Vec<bool> {
    let mut anchored: Vec<_> = input
        .nodes
        .iter()
        .map(|n| n.fixed.is_some() || n.previous.is_some())
        .collect();
    for (i, a) in input.nodes.iter().enumerate() {
        let Some(pa) = a.fixed.or(a.previous) else {
            continue;
        };
        let ra = Rect::new(pa, a.size).inflate(padding / 2.0);
        for (j, b) in input.nodes.iter().enumerate().skip(i + 1) {
            let Some(pb) = b.fixed.or(b.previous) else {
                continue;
            };
            let rb = Rect::new(pb, b.size).inflate(padding / 2.0);
            if ra.overlaps(rb) {
                if a.fixed.is_none() {
                    anchored[i] = false;
                }
                if b.fixed.is_none() {
                    anchored[j] = false;
                }
            }
        }
    }
    anchored
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::tests::{assert_clear, fixture};
    #[test]
    fn warm_start_preserves_positions() {
        let input = fixture(7);
        let first = ForceLayout::default().layout(&input);
        let mut expanded = fixture(8);
        for node in &mut expanded.nodes {
            node.previous = first.positions.get(&node.id).copied();
        }
        let output = ForceLayout::default().layout(&expanded);
        for node in &input.nodes {
            assert_eq!(first.positions[&node.id], output.positions[&node.id]);
        }
        assert_clear(&expanded, &output);
    }
    #[test]
    fn coincident_nodes_separate() {
        let mut input = fixture(12);
        for node in &mut input.nodes {
            node.layout_parent = None;
        }
        let output = ForceLayout(ForceOptions {
            iterations: 0,
            ..Default::default()
        })
        .layout(&input);
        assert_clear(&input, &output);
    }
    #[test]
    fn overlapping_previous_rectangles_can_move_but_pins_cannot() {
        let mut input = fixture(4);
        for node in &mut input.nodes {
            node.previous = Some(Point::default());
        }
        input.nodes[0].fixed = Some(Point::default());
        let output = ForceLayout::default().layout(&input);
        assert_eq!(output.positions[&input.nodes[0].id], Point::default());
        assert_clear(&input, &output);
    }
}
