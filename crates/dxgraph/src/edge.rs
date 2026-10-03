//! SVG edge anchoring and stable path geometry.
use crate::geometry::{Point, Rect};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeSide {
    Top,
    Right,
    Bottom,
    Left,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeAnchor {
    #[default]
    Floating,
    Side(EdgeSide),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgePathStyle {
    Straight,
    #[default]
    Bezier,
    Step,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeMarker {
    #[default]
    None,
    Arrow,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeStyle {
    pub path: EdgePathStyle,
    pub source_anchor: EdgeAnchor,
    pub target_anchor: EdgeAnchor,
    pub from_end: EdgeMarker,
    pub to_end: EdgeMarker,
    pub dashed: bool,
    pub class: Option<String>,
    pub label: Option<String>,
}
impl Default for EdgeStyle {
    fn default() -> Self {
        Self {
            path: EdgePathStyle::Bezier,
            source_anchor: EdgeAnchor::Floating,
            target_anchor: EdgeAnchor::Floating,
            from_end: EdgeMarker::None,
            to_end: EdgeMarker::Arrow,
            dashed: false,
            class: None,
            label: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeGeometry {
    pub path_d: String,
    pub label_pos: Point,
    pub start: Point,
    pub end: Point,
}
/// Intersect a center-to-point ray with the border of a rectangle.
pub fn floating_anchor(rect: Rect, toward: Point) -> Point {
    let center = rect.center();
    let dx = toward.x - center.x;
    let dy = toward.y - center.y;
    if (dx == 0.0 && dy == 0.0) || rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return center;
    }
    let tx = if dx == 0.0 {
        f64::INFINITY
    } else {
        rect.size.width / 2.0 / dx.abs()
    };
    let ty = if dy == 0.0 {
        f64::INFINITY
    } else {
        rect.size.height / 2.0 / dy.abs()
    };
    let t = tx.min(ty);
    Point::new(center.x + dx * t, center.y + dy * t)
}
fn side_anchor(rect: Rect, side: EdgeSide) -> Point {
    let center = rect.center();
    match side {
        EdgeSide::Top => Point::new(center.x, rect.origin.y),
        EdgeSide::Right => Point::new(rect.right(), center.y),
        EdgeSide::Bottom => Point::new(center.x, rect.bottom()),
        EdgeSide::Left => Point::new(rect.origin.x, center.y),
    }
}
fn anchor(rect: Rect, toward: Rect, kind: EdgeAnchor) -> Point {
    match kind {
        EdgeAnchor::Floating => {
            if rect.intersects(toward) {
                rect.center()
            } else {
                floating_anchor(rect, toward.center())
            }
        }
        EdgeAnchor::Side(side) => side_anchor(rect, side),
    }
}
fn normal(anchor: EdgeAnchor, from: Point, to: Point) -> Point {
    match anchor {
        EdgeAnchor::Side(EdgeSide::Top) => Point::new(0.0, -1.0),
        EdgeAnchor::Side(EdgeSide::Right) => Point::new(1.0, 0.0),
        EdgeAnchor::Side(EdgeSide::Bottom) => Point::new(0.0, 1.0),
        EdgeAnchor::Side(EdgeSide::Left) => Point::new(-1.0, 0.0),
        EdgeAnchor::Floating => {
            let len = from.distance(to).max(1.0);
            Point::new((to.x - from.x) / len, (to.y - from.y) / len)
        }
    }
}
/// Round to tenths without trailing decimal zeros or negative zero.
fn number(value: f64) -> String {
    let value = (value * 10.0).round() / 10.0;
    if value == 0.0 {
        "0".into()
    } else {
        value.to_string()
    }
}
fn xy(p: Point) -> String {
    format!("{} {}", number(p.x), number(p.y))
}
pub fn edge_geometry(source: Rect, target: Rect, style: &EdgeStyle) -> EdgeGeometry {
    edge_geometry_with_offset(source, target, style, 0)
}
/// `offset_index` separates parallel edges. Self-loops are determined by coincident rectangles.
pub fn edge_geometry_with_offset(
    source: Rect,
    target: Rect,
    style: &EdgeStyle,
    offset_index: i32,
) -> EdgeGeometry {
    if source == target {
        let start = Point::new(source.right(), source.origin.y + source.size.height * 0.25);
        let end = Point::new(source.origin.x + source.size.width * 0.75, source.origin.y);
        let distance = 40.0 + f64::from(offset_index.unsigned_abs()) * 12.0;
        let a = Point::new(start.x + distance, start.y - distance);
        let b = Point::new(end.x + distance, end.y - distance);
        return EdgeGeometry {
            path_d: format!("M {} C {} {} {}", xy(start), xy(a), xy(b), xy(end)),
            label_pos: cubic_midpoint(start, a, b, end),
            start,
            end,
        };
    }
    let start = anchor(source, target, style.source_anchor);
    let end = anchor(target, source, style.target_anchor);
    let midpoint = Point::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0);
    let (path_d, label_pos) = match style.path {
        EdgePathStyle::Straight => (format!("M {} L {}", xy(start), xy(end)), midpoint),
        EdgePathStyle::Bezier => {
            let distance = start.distance(end);
            let offset = distance.max(40.0) * 0.4;
            let n1 = normal(style.source_anchor, start, end);
            let n2 = normal(style.target_anchor, end, start);
            let bend = f64::from(offset_index) * 24.0;
            let denominator = distance.max(1.0);
            let perpendicular = Point::new(
                -(end.y - start.y) / denominator * bend,
                (end.x - start.x) / denominator * bend,
            );
            let a = Point::new(
                start.x + n1.x * offset + perpendicular.x,
                start.y + n1.y * offset + perpendicular.y,
            );
            let b = Point::new(
                end.x + n2.x * offset + perpendicular.x,
                end.y + n2.y * offset + perpendicular.y,
            );
            (
                format!("M {} C {} {} {}", xy(start), xy(a), xy(b), xy(end)),
                cubic_midpoint(start, a, b, end),
            )
        }
        EdgePathStyle::Step => {
            let horizontal = match style.source_anchor {
                EdgeAnchor::Side(EdgeSide::Left | EdgeSide::Right) => true,
                EdgeAnchor::Side(EdgeSide::Top | EdgeSide::Bottom) => false,
                EdgeAnchor::Floating => (end.x - start.x).abs() >= (end.y - start.y).abs(),
            };
            let (a, b) = if horizontal {
                (
                    Point::new(midpoint.x, start.y),
                    Point::new(midpoint.x, end.y),
                )
            } else {
                (
                    Point::new(start.x, midpoint.y),
                    Point::new(end.x, midpoint.y),
                )
            };
            (
                format!("M {} L {} L {} L {}", xy(start), xy(a), xy(b), xy(end)),
                midpoint,
            )
        }
    };
    EdgeGeometry {
        path_d,
        label_pos,
        start,
        end,
    }
}
fn cubic_midpoint(p: Point, a: Point, b: Point, q: Point) -> Point {
    Point::new(
        (p.x + 3.0 * a.x + 3.0 * b.x + q.x) / 8.0,
        (p.y + 3.0 * a.y + 3.0 * b.y + q.y) / 8.0,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Size;
    #[test]
    fn quadrants_and_sides() {
        let r = Rect::new(Point::default(), Size::new(100.0, 100.0));
        for (to, expected) in [
            (Point::new(200.0, 200.0), Point::new(100.0, 100.0)),
            (Point::new(-100.0, 200.0), Point::new(0.0, 100.0)),
            (Point::new(-100.0, -100.0), Point::new(0.0, 0.0)),
            (Point::new(200.0, -100.0), Point::new(100.0, 0.0)),
        ] {
            assert_eq!(floating_anchor(r, to), expected);
        }
        for (side, expected) in [
            (EdgeSide::Top, Point::new(50.0, 0.0)),
            (EdgeSide::Right, Point::new(100.0, 50.0)),
            (EdgeSide::Bottom, Point::new(50.0, 100.0)),
            (EdgeSide::Left, Point::new(0.0, 50.0)),
        ] {
            assert_eq!(side_anchor(r, side), expected);
        }
    }
    #[test]
    fn exact_paths_degenerate_and_parallel() {
        let a = Rect::new(Point::default(), Size::new(100.0, 100.0));
        let b = Rect::new(Point::new(200.0, 0.0), a.size);
        let mut style = EdgeStyle {
            path: EdgePathStyle::Straight,
            ..Default::default()
        };
        assert_eq!(edge_geometry(a, b, &style).path_d, "M 100 50 L 200 50");
        style.path = EdgePathStyle::Step;
        assert_eq!(
            edge_geometry(a, b, &style).path_d,
            "M 100 50 L 150 50 L 150 50 L 200 50"
        );
        style.path = EdgePathStyle::Bezier;
        assert_eq!(
            edge_geometry(a, b, &style).path_d,
            "M 100 50 C 140 50 160 50 200 50"
        );
        assert_ne!(
            edge_geometry_with_offset(a, b, &style, 1).path_d,
            edge_geometry(a, b, &style).path_d
        );
        assert_eq!(
            edge_geometry(a, a, &style).path_d,
            "M 100 25 C 140 -15 115 -40 75 0"
        );
        let overlap = Rect::new(Point::new(10.0, 10.0), a.size);
        assert_eq!(edge_geometry(a, overlap, &style).start, a.center());
        assert_eq!(number(-0.01), "0");
        assert_eq!(number(1.26), "1.3");
    }
}
