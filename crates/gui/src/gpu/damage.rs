//! What a frame must redraw (ADR-0016): the rectangles that changed,
//! merged while the area a merge wastes stays small, as iced groups its
//! damage (`graphics/src/damage.rs`, 20 000 px²), and into one rectangle
//! past eight.

/// A rectangle of physical pixels, `x0, y0` in, `x1, y1` out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct Rect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

/// How much area a merge may waste before two rectangles stay apart.
const WASTE: i64 = 20_000;
/// The most rectangles a frame redraws; past it, their union.
const MOST: usize = 8;

impl Rect {
    /// The whole pixels `r` (x0, y0, x1, y1) touches.
    pub(crate) fn covering(r: [f32; 4]) -> Option<Self> {
        if !(r[2] > r[0] && r[3] > r[1]) || r.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let v = |x: f32| crate::convert::f32_to_i32(x.clamp(-1e9, 1e9));
        Some(Self {
            x0: v(r[0].floor()),
            y0: v(r[1].floor()),
            x1: v(r[2].ceil()),
            y1: v(r[3].ceil()),
        })
    }

    pub(crate) fn area(&self) -> i64 {
        i64::from(self.x1.saturating_sub(self.x0).max(0))
            .saturating_mul(i64::from(self.y1.saturating_sub(self.y0).max(0)))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    pub(crate) fn union(&self, o: &Self) -> Self {
        Self {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }

    pub(crate) fn intersect(&self, o: &Self) -> Self {
        Self {
            x0: self.x0.max(o.x0),
            y0: self.y0.max(o.y0),
            x1: self.x1.min(o.x1),
            y1: self.y1.min(o.y1),
        }
    }
}

/// `damage` inside `bounds`, merged: sorted by distance from the origin, a
/// rectangle joins the one before while the union wastes at most [`WASTE`]
/// pixels; more than [`MOST`] become one.
pub(crate) fn group(damage: Vec<Rect>, bounds: Rect) -> Vec<Rect> {
    let mut rects: Vec<Rect> = damage
        .into_iter()
        .map(|r| r.intersect(&bounds))
        .filter(|r| !r.is_empty())
        .collect();
    let centre = |r: &Rect| {
        let x = i64::from(r.x0).saturating_add(i64::from(r.x1));
        let y = i64::from(r.y0).saturating_add(i64::from(r.y1));
        x.saturating_mul(x).saturating_add(y.saturating_mul(y))
    };
    rects.sort_by_key(centre);
    let mut out: Vec<Rect> = Vec::new();
    for r in rects {
        match out.last_mut() {
            Some(last) => {
                let u = last.union(&r);
                let waste = u.area().saturating_sub(last.area()).saturating_sub(r.area());
                if waste <= WASTE {
                    *last = u;
                } else {
                    out.push(r);
                }
            }
            None => out.push(r),
        }
    }
    // A merge may now overlap a rectangle merged earlier.
    let mut merged = true;
    while merged {
        merged = false;
        'outer: for i in 0..out.len() {
            for j in i.saturating_add(1)..out.len() {
                let (Some(a), Some(b)) = (out.get(i).copied(), out.get(j).copied()) else {
                    continue;
                };
                if a.intersect(&b).area() > 0 {
                    if let Some(x) = out.get_mut(i) {
                        *x = a.union(&b);
                    }
                    out.remove(j);
                    merged = true;
                    break 'outer;
                }
            }
        }
    }
    if out.len() > MOST
        && let Some(u) = out.iter().copied().reduce(|a, b| a.union(&b))
    {
        return vec![u];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    #[test]
    fn near_rectangles_merge_and_far_ones_do_not() {
        let screen = r(0, 0, 2000, 2000);
        let near = group(vec![r(0, 0, 100, 100), r(110, 0, 200, 100)], screen);
        assert_eq!(near, vec![r(0, 0, 200, 100)]);
        let far = group(vec![r(0, 0, 100, 100), r(1500, 1500, 1600, 1600)], screen);
        assert_eq!(far.len(), 2);
    }

    #[test]
    fn many_become_one_and_none_leave_the_screen() {
        let screen = r(0, 0, 4000, 4000);
        let many: Vec<Rect> = (0..20)
            .map(|i| r(i * 300, i * 300, i * 300 + 10, i * 300 + 10))
            .collect();
        assert_eq!(group(many, screen), vec![r(0, 0, 3910, 3910)]);
        assert_eq!(
            group(vec![r(-50, -50, 10, 10)], r(0, 0, 100, 100)),
            vec![r(0, 0, 10, 10)]
        );
        assert!(group(vec![r(200, 200, 300, 300)], r(0, 0, 100, 100)).is_empty());
    }
}
