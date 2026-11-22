//! The little SVG the embedded icons are written in, read into paths: the
//! elements `path`, `circle`, `ellipse`, `rect`, `line`, `polyline` and
//! `polygon`, painted with the root's or their own `fill` and `stroke`
//! (`currentColor` or `none`), `stroke-width`, `stroke-linecap` and
//! `stroke-linejoin`. The XML is read by roxmltree and path data by
//! svgtypes (its arcs come back as cubics); nothing is rasterized here.

use crate::convert::f64_to_f32;
use crate::shape::{Cap, Join, Path, PathBuilder};

/// One element of an icon: its path in the view box's units and whether it
/// is filled and stroked.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Part {
    pub path: Path,
    pub fill: bool,
    pub stroke: bool,
}

/// An icon read from SVG: its parts in painting order, its view box's
/// width, and how its strokes are drawn.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Drawing {
    pub parts: Vec<Part>,
    pub view: f32,
    pub stroke_width: f32,
    pub cap: Cap,
    pub join: Join,
}

/// Why an icon could not be read.
#[derive(Debug)]
pub(crate) struct SvgError(pub String);

impl std::fmt::Display for SvgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn number(node: roxmltree::Node<'_, '_>, name: &str) -> f32 {
    node.attribute(name)
        .and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok())
        .unwrap_or(0.0)
}

fn paints(v: Option<&str>, inherited: bool) -> bool {
    match v.map(str::trim) {
        Some("none") => false,
        Some(_) => true,
        None => inherited,
    }
}

/// Read `svg` (its `currentColor` is the colour an icon is drawn in).
pub(crate) fn parse(svg: &str) -> Result<Drawing, SvgError> {
    let doc = roxmltree::Document::parse(svg).map_err(|e| SvgError(e.to_string()))?;
    let root = doc.root_element();
    let view = root
        .attribute("viewBox")
        .and_then(|v| v.split_whitespace().nth(2).and_then(|w| w.parse::<f32>().ok()))
        .unwrap_or_else(|| number(root, "width"))
        .max(1.0);
    // SVG's initial values: filled black, not stroked.
    let fill = paints(root.attribute("fill"), true);
    let stroke = paints(root.attribute("stroke"), false);
    let stroke_width = root
        .attribute("stroke-width")
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(1.0);
    let cap = match root.attribute("stroke-linecap") {
        Some("round") => Cap::Round,
        Some("square") => Cap::Square,
        _ => Cap::Butt,
    };
    let join = match root.attribute("stroke-linejoin") {
        Some("round") => Join::Round,
        Some("bevel") => Join::Bevel,
        _ => Join::Miter,
    };
    let mut parts = Vec::new();
    for node in root.descendants().filter(roxmltree::Node::is_element) {
        let path = match node.tag_name().name() {
            "path" => path_data(node.attribute("d").unwrap_or(""))?,
            "circle" => {
                let r = number(node, "r");
                Path::circle(number(node, "cx"), number(node, "cy"), r)
            }
            "ellipse" => Path::ellipse(
                number(node, "cx"),
                number(node, "cy"),
                number(node, "rx"),
                number(node, "ry"),
            ),
            "rect" => rect(node),
            "line" => Path::builder()
                .move_to(number(node, "x1"), number(node, "y1"))
                .line_to(number(node, "x2"), number(node, "y2")),
            "polyline" | "polygon" => {
                let mut b = Path::builder();
                for (i, (x, y)) in svgtypes::PointsParser::from(node.attribute("points").unwrap_or("")).enumerate() {
                    let (x, y) = (f64_to_f32(x), f64_to_f32(y));
                    b = if i == 0 { b.move_to(x, y) } else { b.line_to(x, y) };
                }
                if node.tag_name().name() == "polygon" {
                    b = b.close();
                }
                b
            }
            _ => continue,
        };
        parts.push(Part {
            path: path.build(),
            fill: paints(node.attribute("fill"), fill),
            stroke: paints(node.attribute("stroke"), stroke),
        });
    }
    Ok(Drawing {
        parts,
        view,
        stroke_width,
        cap,
        join,
    })
}

/// A `rect`, its corners elliptical when `rx` and `ry` differ.
fn rect(node: roxmltree::Node<'_, '_>) -> PathBuilder {
    let (x, y, w, h) = (
        number(node, "x"),
        number(node, "y"),
        number(node, "width"),
        number(node, "height"),
    );
    let (rx, ry) = match (node.attribute("rx"), node.attribute("ry")) {
        (Some(_), Some(_)) => (number(node, "rx"), number(node, "ry")),
        (Some(_), None) => (number(node, "rx"), number(node, "rx")),
        (None, Some(_)) => (number(node, "ry"), number(node, "ry")),
        (None, None) => (0.0, 0.0),
    };
    let (rx, ry) = (rx.clamp(0.0, w / 2.0), ry.clamp(0.0, h / 2.0));
    if rx <= 0.0 || ry <= 0.0 {
        return Path::rect(x, y, w, h);
    }
    let k = 1.0 - 0.552_284_8;
    let (x1, y1) = (x + w, y + h);
    Path::builder()
        .move_to(x + rx, y)
        .line_to(x1 - rx, y)
        .cubic_to(x1 - rx * k, y, x1, y + ry * k, x1, y + ry)
        .line_to(x1, y1 - ry)
        .cubic_to(x1, y1 - ry * k, x1 - rx * k, y1, x1 - rx, y1)
        .line_to(x + rx, y1)
        .cubic_to(x + rx * k, y1, x, y1 - ry * k, x, y1 - ry)
        .line_to(x, y + ry)
        .cubic_to(x, y + ry * k, x + rx * k, y, x + rx, y)
        .close()
}

/// A path's `d`, made absolute with arcs as cubics (svgtypes' simplifying parser).
fn path_data(d: &str) -> Result<PathBuilder, SvgError> {
    use svgtypes::SimplePathSegment as S;
    let mut b = Path::builder();
    for seg in svgtypes::SimplifyingPathParser::from(d) {
        b = match seg.map_err(|e| SvgError(e.to_string()))? {
            S::MoveTo { x, y } => b.move_to(f64_to_f32(x), f64_to_f32(y)),
            S::LineTo { x, y } => b.line_to(f64_to_f32(x), f64_to_f32(y)),
            S::Quadratic { x1, y1, x, y } => b.quad_to(f64_to_f32(x1), f64_to_f32(y1), f64_to_f32(x), f64_to_f32(y)),
            S::CurveTo { x1, y1, x2, y2, x, y } => b.cubic_to(
                f64_to_f32(x1),
                f64_to_f32(y1),
                f64_to_f32(x2),
                f64_to_f32(y2),
                f64_to_f32(x),
                f64_to_f32(y),
            ),
            S::ClosePath => b.close(),
        };
    }
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lucide_icon_reads_as_stroked_parts() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"
            fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" />
            <path d="M12 8v4" /><line x1="12" y1="16" x2="12.01" y2="16" />
            <circle cx="7.5" cy="7.5" r=".5" fill="currentColor" />
            <path d="M3 3a9 9 0 0 1 18 0" />
            </svg>"#;
        let d = parse(svg).expect("read");
        assert_eq!(d.parts.len(), 5);
        assert!((d.view - 24.0).abs() < f32::EPSILON);
        assert_eq!((d.cap, d.join), (Cap::Round, Join::Round));
        assert!(d.parts.iter().all(|p| p.stroke));
        assert_eq!(d.parts.iter().filter(|p| p.fill).count(), 1);
        // The arc became cubics.
        let arc = d.parts.get(4).expect("the arc");
        assert!(arc.path.verbs().contains(&crate::shape::Verb::Cubic));
    }
}
