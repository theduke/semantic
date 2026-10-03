//! Culling acceptance at the v1 exploration budget.
use dxgraph::canvas::state::visible_nodes;
use dxgraph::layout::TreeLayout;
use dxgraph::{LayoutAlgorithm, LayoutInput, LayoutNode, NodeId, Point, Size, Viewport};
use indexmap::{IndexMap, IndexSet};

#[test]
fn three_hundred_nodes_keep_the_visible_set_bounded() {
    let input = LayoutInput {
        nodes: (0..300)
            .map(|index| LayoutNode {
                id: NodeId::from(format!("node-{index:03}")),
                size: Size::new(160.0, 64.0),
                fixed: None,
                previous: None,
                layout_parent: (index > 0)
                    .then(|| NodeId::from(format!("node-{:03}", (index - 1) / 2))),
                order_key: None,
            })
            .collect(),
        edges: Vec::new(),
        roots: vec!["node-000".into()],
    };
    let positions = TreeLayout::default().layout(&input).positions;
    let rects: IndexMap<_, _> = input
        .nodes
        .iter()
        .map(|node| {
            (
                node.id.clone(),
                dxgraph::Rect::new(positions[&node.id], node.size),
            )
        })
        .collect();
    let root = positions[&input.nodes[0].id];
    let viewport = Viewport {
        x: -root.x + 450.0,
        y: -root.y + 100.0,
        zoom: 1.0,
    };
    let container = Size::new(1000.0, 700.0);
    let visible = viewport.visible_world_rect(container).inflate(200.0);
    let nodes = visible_nodes(&rects, visible, &IndexSet::new(), None);
    eprintln!(
        "300-node graph at 1x: {} visible node wrappers in a 1000x700 viewport plus 200px margin",
        nodes.len()
    );
    assert!(nodes.contains(&input.nodes[0].id));
    assert!(
        nodes.len() < 60,
        "culling unexpectedly exposes {} nodes",
        nodes.len()
    );
    let remote = input
        .nodes
        .iter()
        .find(|node| !nodes.contains(&node.id))
        .unwrap()
        .id
        .clone();
    let selected = IndexSet::from([remote.clone()]);
    let kept = visible_nodes(&rects, visible, &selected, None);
    assert!(kept.contains(&remote));
    assert_eq!(kept.len(), nodes.len() + 1);
    assert!(visible_nodes(&rects, visible, &IndexSet::new(), Some(&remote)).contains(&remote));
    // Screen-space panning only changes which subset is exposed, not node positions.
    let panned = Viewport {
        x: viewport.x - 600.0,
        ..viewport
    };
    assert_ne!(
        nodes,
        visible_nodes(
            &rects,
            panned.visible_world_rect(container).inflate(200.0),
            &IndexSet::new(),
            None
        )
    );
    assert_eq!(
        viewport.screen_to_world(viewport.world_to_screen(Point::new(50.0, 70.0))),
        Point::new(50.0, 70.0)
    );
}
