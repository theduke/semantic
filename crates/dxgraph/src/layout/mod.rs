//! Deterministic, framework-independent layouts. Positions are world top-left corners.
use crate::{NodeId, Point, Rect, Size};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
pub mod force;
pub mod mindmap;
pub mod radial;
mod rng;
mod spanning;
pub mod tree;
pub use force::{ForceLayout, ForceOptions};
pub use mindmap::{MindMapBalance, MindMapLayout, MindMapOptions};
pub use radial::{RadialLayout, RadialOptions};
pub use tree::{TreeDirection, TreeLayout, TreeLayoutOptions};
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutNode {
    pub id: NodeId,
    pub size: Size,
    pub fixed: Option<Point>,
    pub previous: Option<Point>,
    pub layout_parent: Option<NodeId>,
    pub order_key: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutEdge {
    pub source: NodeId,
    pub target: NodeId,
    pub weight: f64,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutInput {
    pub nodes: Vec<LayoutNode>,
    pub edges: Vec<LayoutEdge>,
    pub roots: Vec<NodeId>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutOutput {
    pub positions: IndexMap<NodeId, Point>,
}
pub trait LayoutAlgorithm {
    fn layout(&self, input: &LayoutInput) -> LayoutOutput;
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LayoutConfig {
    Tree(TreeLayoutOptions),
    MindMap(MindMapOptions),
    Force(ForceOptions),
    Radial(RadialOptions),
    Manual,
}
impl Default for LayoutConfig {
    fn default() -> Self {
        Self::Tree(TreeLayoutOptions::default())
    }
}
pub fn run_layout(config: &LayoutConfig, input: &LayoutInput) -> LayoutOutput {
    match config {
        LayoutConfig::Tree(options) => TreeLayout(options.clone()).layout(input),
        LayoutConfig::MindMap(options) => MindMapLayout(options.clone()).layout(input),
        LayoutConfig::Force(options) => ForceLayout(options.clone()).layout(input),
        LayoutConfig::Radial(options) => RadialLayout(options.clone()).layout(input),
        LayoutConfig::Manual => LayoutOutput {
            positions: input
                .nodes
                .iter()
                .map(|n| (n.id.clone(), n.fixed.or(n.previous).unwrap_or_default()))
                .collect(),
        },
    }
}
/// Pack disconnected components in rows. Components containing pins retain their
/// world coordinates; movable components are placed outside their bounds.
pub fn pack_components(
    input: &LayoutInput,
    components: &[Vec<usize>],
    positions: &mut IndexMap<NodeId, Point>,
    gap: f64,
) {
    let mut occupied = Vec::<Rect>::new();
    for component in components
        .iter()
        .filter(|c| c.iter().any(|&i| input.nodes[i].fixed.is_some()))
    {
        if let Some(&anchor) = component.iter().find(|&&i| input.nodes[i].fixed.is_some()) {
            let node = &input.nodes[anchor];
            if let (Some(fixed), Some(current)) = (node.fixed, positions.get(&node.id).copied()) {
                let delta = Point::new(fixed.x - current.x, fixed.y - current.y);
                for &i in component {
                    let node = &input.nodes[i];
                    if let Some(p) = positions.get_mut(&node.id) {
                        *p = node
                            .fixed
                            .unwrap_or(Point::new(p.x + delta.x, p.y + delta.y));
                    }
                }
            }
        }
        if let Some(bounds) = component_bounds(input, component, positions) {
            occupied.push(bounds);
        }
    }
    let max_width = components
        .iter()
        .filter_map(|c| component_bounds(input, c, positions))
        .map(|r| r.size.width)
        .sum::<f64>()
        .sqrt()
        .max(1000.0);
    let mut cursor = Point::default();
    let mut row_height: f64 = 0.0;
    for component in components
        .iter()
        .filter(|c| !c.iter().any(|&i| input.nodes[i].fixed.is_some()))
    {
        let Some(bounds) = component_bounds(input, component, positions) else {
            continue;
        };
        if cursor.x > 0.0 && cursor.x + bounds.size.width > max_width {
            cursor.x = 0.0;
            cursor.y += row_height + gap;
            row_height = 0.0;
        }
        // Jump past pinned components, rather than repeatedly nudging by a pixel.
        loop {
            let candidate = Rect::new(cursor, bounds.size).inflate(gap / 2.0);
            let collision = occupied.iter().find(|r| {
                let other = r.inflate(gap / 2.0);
                candidate.right() > other.origin.x
                    && other.right() > candidate.origin.x
                    && candidate.bottom() > other.origin.y
                    && other.bottom() > candidate.origin.y
            });
            match collision {
                Some(rect) => cursor.x = (rect.right() + gap).max(cursor.x + 1e-6),
                None => break,
            }
        }
        let delta = Point::new(cursor.x - bounds.origin.x, cursor.y - bounds.origin.y);
        for &i in component {
            if let Some(p) = positions.get_mut(&input.nodes[i].id) {
                p.x += delta.x;
                p.y += delta.y;
            }
        }
        occupied.push(Rect::new(cursor, bounds.size));
        cursor.x += bounds.size.width + gap;
        row_height = row_height.max(bounds.size.height);
    }
    for node in &input.nodes {
        if let Some(fixed) = node.fixed {
            positions.insert(node.id.clone(), fixed);
        }
    }
}
fn component_bounds(
    input: &LayoutInput,
    component: &[usize],
    positions: &IndexMap<NodeId, Point>,
) -> Option<Rect> {
    component
        .iter()
        .filter_map(|&i| {
            let node = &input.nodes[i];
            positions.get(&node.id).map(|&p| Rect::new(p, node.size))
        })
        .reduce(Rect::union)
}
/// Separate rectangles along the axis of least overlap; pins are never moved.
/// A final ordered placement guarantees clearance even for a dense initial pile.
pub(super) fn remove_overlaps(
    input: &LayoutInput,
    positions: &mut IndexMap<NodeId, Point>,
    padding: f64,
) {
    // Keep the quadratic collision loop in indexed arrays. Looking up string ids
    // in the ordered map for every pair dominates layout time in debug builds.
    let mut points: Vec<_> = input
        .nodes
        .iter()
        .map(|node| positions.get(&node.id).copied())
        .collect();
    for _ in 0..80 {
        let mut changed = false;
        for i in 0..input.nodes.len() {
            for j in i + 1..input.nodes.len() {
                let (a, b) = (&input.nodes[i], &input.nodes[j]);
                if a.fixed.is_some() && b.fixed.is_some() {
                    continue;
                }
                let (Some(pa), Some(pb)) = (points[i], points[j]) else {
                    continue;
                };
                let ox = (pa.x + a.size.width).min(pb.x + b.size.width) - pa.x.max(pb.x) + padding;
                let oy =
                    (pa.y + a.size.height).min(pb.y + b.size.height) - pa.y.max(pb.y) + padding;
                if ox <= 0.0 || oy <= 0.0 {
                    continue;
                }
                changed = true;
                let delta = if ox < oy {
                    Point::new(
                        if pa.x + a.size.width / 2.0 <= pb.x + b.size.width / 2.0 {
                            -ox - 0.001
                        } else {
                            ox + 0.001
                        },
                        0.0,
                    )
                } else {
                    Point::new(
                        0.0,
                        if pa.y + a.size.height / 2.0 <= pb.y + b.size.height / 2.0 {
                            -oy - 0.001
                        } else {
                            oy + 0.001
                        },
                    )
                };
                let share_a = if a.fixed.is_some() {
                    0.0
                } else if b.fixed.is_some() {
                    1.0
                } else {
                    0.5
                };
                let share_b = 1.0 - share_a;
                points[i] = Some(Point::new(
                    pa.x + delta.x * share_a,
                    pa.y + delta.y * share_a,
                ));
                points[j] = Some(Point::new(
                    pb.x - delta.x * share_b,
                    pb.y - delta.y * share_b,
                ));
            }
        }
        if !changed {
            break;
        }
    }
    for (node, point) in input.nodes.iter().zip(points) {
        if let Some(point) = point {
            positions.insert(node.id.clone(), point);
        }
    }
    let mut placed: Vec<Rect> = input
        .nodes
        .iter()
        .filter_map(|n| n.fixed.map(|p| Rect::new(p, n.size).inflate(padding / 2.0)))
        .collect();
    for node in input.nodes.iter().filter(|n| n.fixed.is_none()) {
        let Some(mut position) = positions.get(&node.id).copied() else {
            continue;
        };
        loop {
            let rect = Rect::new(position, node.size).inflate(padding / 2.0);
            let Some(other) = placed.iter().find(|other| {
                rect.right() > other.origin.x
                    && other.right() > rect.origin.x
                    && rect.bottom() > other.origin.y
                    && other.bottom() > rect.origin.y
            }) else {
                break;
            };
            position.x = other.right() + padding / 2.0 + 0.001;
        }
        placed.push(Rect::new(position, node.size).inflate(padding / 2.0));
        positions.insert(node.id.clone(), position);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn fixture(count: usize) -> LayoutInput {
        LayoutInput {
            nodes: (0..count)
                .map(|i| LayoutNode {
                    id: format!("n{i:03}").into(),
                    size: Size::new(40.0 + (i * 17 % 70) as f64, 25.0 + (i * 29 % 90) as f64),
                    fixed: None,
                    previous: None,
                    layout_parent: (i > 0).then(|| format!("n{:03}", (i - 1) / 2).into()),
                    order_key: None,
                })
                .collect(),
            edges: Vec::new(),
            roots: vec!["n000".into()],
        }
    }
    pub(super) fn assert_clear(input: &LayoutInput, output: &LayoutOutput) {
        for (i, a) in input.nodes.iter().enumerate() {
            for b in &input.nodes[i + 1..] {
                let ra = Rect::new(output.positions[&a.id], a.size);
                let rb = Rect::new(output.positions[&b.id], b.size);
                assert!(
                    ra.right() <= rb.origin.x + 0.0001
                        || rb.right() <= ra.origin.x + 0.0001
                        || ra.bottom() <= rb.origin.y + 0.0001
                        || rb.bottom() <= ra.origin.y + 0.0001,
                    "{} overlaps {}: {ra:?}, {rb:?}",
                    a.id,
                    b.id
                );
            }
        }
    }
    #[test]
    fn algorithms_obey_contract() {
        let algorithms: Vec<Box<dyn LayoutAlgorithm>> = vec![
            Box::new(TreeLayout::default()),
            Box::new(TreeLayout(TreeLayoutOptions {
                direction: TreeDirection::LeftRight,
                ..Default::default()
            })),
            Box::new(MindMapLayout::default()),
            Box::new(ForceLayout(ForceOptions {
                iterations: 40,
                ..Default::default()
            })),
            Box::new(RadialLayout(RadialOptions {
                focus: "n000".into(),
                ..Default::default()
            })),
        ];
        for algorithm in algorithms {
            for count in [0, 1, 2, 7, 31] {
                let input = fixture(count);
                let output = algorithm.layout(&input);
                assert_eq!(output.positions.len(), count);
                assert_eq!(output, algorithm.layout(&input));
                assert_clear(&input, &output);
            }
            let mut input = fixture(12);
            input.nodes[3].fixed = Some(Point::new(-120.0, -80.0));
            input.nodes[7].layout_parent = None;
            input.edges.push(LayoutEdge {
                source: "n011".into(),
                target: "n000".into(),
                weight: 1.0,
            });
            let output = algorithm.layout(&input);
            assert_eq!(
                output.positions[&input.nodes[3].id],
                Point::new(-120.0, -80.0)
            );
            assert_clear(&input, &output);
        }
    }
    #[test]
    fn pack_components_accepts_touching_padding_boundaries() {
        let mut input = fixture(3);
        for node in &mut input.nodes {
            node.layout_parent = None;
            node.size = Size::new(40.0, 20.0);
        }
        let mut positions = input
            .nodes
            .iter()
            .map(|node| (node.id.clone(), Point::default()))
            .collect();
        pack_components(&input, &[vec![0], vec![1], vec![2]], &mut positions, 10.0);
        for (index, node) in input.nodes.iter().enumerate() {
            assert_eq!(positions[&node.id], Point::new(index as f64 * 50.0, 0.0));
        }
        // Fractional measurements may leave a sub-ulp intersection after a
        // jump. Packing must still make progress instead of repeating it.
        for width in [40.1, 40.123456789, 40.99999999] {
            for node in &mut input.nodes {
                node.size.width = width;
            }
            for point in positions.values_mut() {
                *point = Point::new(-0.17, -0.31);
            }
            pack_components(&input, &[vec![0], vec![1], vec![2]], &mut positions, 10.1);
            for (index, node) in input.nodes.iter().enumerate() {
                assert!((positions[&node.id].x - index as f64 * (width + 10.1)).abs() < 1e-4);
            }
        }
    }
    #[test]
    fn manual_preserves_previous_and_fixed() {
        let mut input = fixture(2);
        input.nodes[0].previous = Some(Point::new(3.0, 4.0));
        input.nodes[1].fixed = Some(Point::new(7.0, 8.0));
        let output = run_layout(&LayoutConfig::Manual, &input);
        assert_eq!(output.positions[&input.nodes[0].id], Point::new(3.0, 4.0));
        assert_eq!(output.positions[&input.nodes[1].id], Point::new(7.0, 8.0));
    }
}
