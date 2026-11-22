//! A dynamic AABB tree (Box2D's `b2DynamicTree`, `f64` boxes): leaves hold
//! fattened boxes so a small move updates nothing, inserts descend by the
//! area they would add, and AVL rotations keep it balanced. Culling, hit
//! testing and marquee query it in `O(log n + k)` (ADR-0017).

/// A box: `[x0, y0, x1, y1]`.
pub type Aabb = [f64; 4];

const NULL: u32 = u32::MAX;

#[derive(Clone, Debug)]
struct TreeNode {
    aabb: Aabb,
    parent: u32,
    left: u32,
    right: u32,
    /// Leaf 0; else one more than its taller child.
    height: i32,
    /// A leaf's item.
    item: u32,
}

impl TreeNode {
    fn leaf(&self) -> bool {
        self.left == NULL
    }
}

fn union(a: &Aabb, b: &Aabb) -> Aabb {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

fn perimeter(a: &Aabb) -> f64 {
    2.0 * ((a[2] - a[0]) + (a[3] - a[1]))
}

fn contains(outer: &Aabb, inner: &Aabb) -> bool {
    outer[0] <= inner[0] && outer[1] <= inner[1] && outer[2] >= inner[2] && outer[3] >= inner[3]
}

/// Whether two boxes meet.
#[must_use]
pub fn overlaps(a: &Aabb, b: &Aabb) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

/// The tree. Proxies are the ids [`Self::insert`] returns.
#[derive(Clone, Debug, Default)]
pub struct AabbTree {
    nodes: Vec<TreeNode>,
    free: Vec<u32>,
    root: u32,
    /// How far a leaf's box is fattened, as a share of its size.
    margin: f64,
}

impl AabbTree {
    /// A tree whose leaves are fattened by `margin` of their size each side.
    #[must_use]
    pub fn new(margin: f64) -> Self {
        Self {
            nodes: Vec::new(),
            free: Vec::new(),
            root: NULL,
            margin,
        }
    }

    fn node(&self, i: u32) -> Option<&TreeNode> {
        self.nodes.get(usize::try_from(i).ok()?)
    }

    fn node_mut(&mut self, i: u32) -> Option<&mut TreeNode> {
        self.nodes.get_mut(usize::try_from(i).ok()?)
    }

    fn get(&self, i: u32) -> TreeNode {
        self.node(i).cloned().unwrap_or(TreeNode {
            aabb: [0.0; 4],
            parent: NULL,
            left: NULL,
            right: NULL,
            height: -1,
            item: NULL,
        })
    }

    fn alloc(&mut self, n: TreeNode) -> u32 {
        if let Some(i) = self.free.pop() {
            if let Some(slot) = self.node_mut(i) {
                *slot = n;
            }
            return i;
        }
        self.nodes.push(n);
        u32::try_from(self.nodes.len().saturating_sub(1)).unwrap_or(NULL)
    }

    fn fat(&self, a: &Aabb) -> Aabb {
        let mx = (a[2] - a[0]) * self.margin;
        let my = (a[3] - a[1]) * self.margin;
        [a[0] - mx, a[1] - my, a[2] + mx, a[3] + my]
    }

    /// Insert `item` with box `aabb`; its proxy.
    pub fn insert(&mut self, aabb: Aabb, item: u32) -> u32 {
        let fat = self.fat(&aabb);
        let leaf = self.alloc(TreeNode {
            aabb: fat,
            parent: NULL,
            left: NULL,
            right: NULL,
            height: 0,
            item,
        });
        self.insert_leaf(leaf);
        leaf
    }

    /// Take `proxy` out.
    pub fn remove(&mut self, proxy: u32) {
        self.remove_leaf(proxy);
        self.free.push(proxy);
    }

    /// `proxy`'s item now has box `aabb`: moved in the tree only when it
    /// left its fattened box. Whether it moved.
    pub fn update(&mut self, proxy: u32, aabb: Aabb) -> bool {
        if contains(&self.get(proxy).aabb, &aabb) {
            return false;
        }
        self.remove_leaf(proxy);
        let fat = self.fat(&aabb);
        if let Some(n) = self.node_mut(proxy) {
            n.aabb = fat;
        }
        self.insert_leaf(proxy);
        true
    }

    /// Every item whose fattened box meets `area`.
    pub fn query(&self, area: &Aabb, mut f: impl FnMut(u32)) {
        let mut stack = vec![self.root];
        while let Some(i) = stack.pop() {
            let Some(n) = self.node(i) else { continue };
            if !overlaps(&n.aabb, area) {
                continue;
            }
            if n.leaf() {
                f(n.item);
            } else {
                stack.push(n.left);
                stack.push(n.right);
            }
        }
    }

    /// How many items' fattened boxes meet `area`, counting no further
    /// than `limit`.
    #[must_use]
    pub fn count(&self, area: &Aabb, limit: usize) -> usize {
        let mut n = 0_usize;
        let mut stack = vec![self.root];
        while let Some(i) = stack.pop() {
            if n >= limit {
                break;
            }
            let Some(node) = self.node(i) else { continue };
            if !overlaps(&node.aabb, area) {
                continue;
            }
            if node.leaf() {
                n = n.saturating_add(1);
            } else {
                stack.push(node.left);
                stack.push(node.right);
            }
        }
        n
    }

    /// The height of the tree (a leaf alone: 0).
    #[must_use]
    pub fn height(&self) -> i32 {
        self.node(self.root).map_or(0, |n| n.height)
    }

    fn insert_leaf(&mut self, leaf: u32) {
        if self.root == NULL {
            self.root = leaf;
            if let Some(n) = self.node_mut(leaf) {
                n.parent = NULL;
            }
            return;
        }
        let box_ = self.get(leaf).aabb;
        // Descend to the sibling that adds the least perimeter.
        let mut i = self.root;
        loop {
            let n = self.get(i);
            if n.leaf() {
                break;
            }
            let area = perimeter(&n.aabb);
            let combined = perimeter(&union(&n.aabb, &box_));
            let cost = 2.0 * combined;
            let inherit = 2.0 * (combined - area);
            let child_cost = |c: u32| {
                let cn = self.get(c);
                let u = perimeter(&union(&box_, &cn.aabb));
                if cn.leaf() {
                    u + inherit
                } else {
                    u - perimeter(&cn.aabb) + inherit
                }
            };
            let (c1, c2) = (child_cost(n.left), child_cost(n.right));
            if cost < c1 && cost < c2 {
                break;
            }
            i = if c1 < c2 { n.left } else { n.right };
        }
        let sibling = i;
        let old_parent = self.get(sibling).parent;
        let parent = self.alloc(TreeNode {
            aabb: union(&box_, &self.get(sibling).aabb),
            parent: old_parent,
            left: sibling,
            right: leaf,
            height: self.get(sibling).height.saturating_add(1),
            item: NULL,
        });
        if old_parent == NULL {
            self.root = parent;
        } else if let Some(p) = self.node_mut(old_parent) {
            if p.left == sibling {
                p.left = parent;
            } else {
                p.right = parent;
            }
        }
        if let Some(s) = self.node_mut(sibling) {
            s.parent = parent;
        }
        if let Some(l) = self.node_mut(leaf) {
            l.parent = parent;
        }
        self.refit(parent);
    }

    fn remove_leaf(&mut self, leaf: u32) {
        if leaf == self.root {
            self.root = NULL;
            return;
        }
        let parent = self.get(leaf).parent;
        let p = self.get(parent);
        let sibling = if p.left == leaf { p.right } else { p.left };
        let grand = p.parent;
        if grand == NULL {
            self.root = sibling;
            if let Some(s) = self.node_mut(sibling) {
                s.parent = NULL;
            }
        } else {
            if let Some(g) = self.node_mut(grand) {
                if g.left == parent {
                    g.left = sibling;
                } else {
                    g.right = sibling;
                }
            }
            if let Some(s) = self.node_mut(sibling) {
                s.parent = grand;
            }
            self.refit(grand);
        }
        self.free.push(parent);
    }

    /// Boxes and heights from `i` up, balancing as it goes.
    fn refit(&mut self, mut i: u32) {
        while i != NULL {
            i = self.balance(i);
            let n = self.get(i);
            let (l, r) = (self.get(n.left), self.get(n.right));
            if let Some(m) = self.node_mut(i) {
                m.height = 1_i32.saturating_add(l.height.max(r.height));
                m.aabb = union(&l.aabb, &r.aabb);
            }
            i = n.parent;
        }
    }

    /// A rotation at `a` when one child is taller by two; the subtree's root.
    fn balance(&mut self, a: u32) -> u32 {
        let na = self.get(a);
        if na.leaf() || na.height < 2 {
            return a;
        }
        let (b, c) = (na.left, na.right);
        let diff = self.get(c).height.saturating_sub(self.get(b).height);
        if diff > 1 {
            self.rotate(a, c, b)
        } else if diff < -1 {
            self.rotate(a, b, c)
        } else {
            a
        }
    }

    /// Lift `up` (the taller child of `a`) over `a`; `other` is `a`'s other child.
    fn rotate(&mut self, a: u32, up: u32, other: u32) -> u32 {
        let nu = self.get(up);
        let (f, g) = (nu.left, nu.right);
        let parent = self.get(a).parent;
        // `up` takes `a`'s place.
        if let Some(n) = self.node_mut(up) {
            n.parent = parent;
        }
        if parent == NULL {
            self.root = up;
        } else if let Some(p) = self.node_mut(parent) {
            if p.left == a {
                p.left = up;
            } else {
                p.right = up;
            }
        }
        if let Some(n) = self.node_mut(a) {
            n.parent = up;
        }
        // The taller grandchild stays under `up`, the other goes under `a`.
        let (keep, give) = if self.get(f).height > self.get(g).height {
            (f, g)
        } else {
            (g, f)
        };
        if let Some(n) = self.node_mut(up) {
            n.left = a;
            n.right = keep;
        }
        if let Some(n) = self.node_mut(a) {
            if n.left == up {
                n.left = give;
            } else {
                n.right = give;
            }
            let _ = other;
        }
        if let Some(n) = self.node_mut(give) {
            n.parent = a;
        }
        let (na, nk) = (self.get(a), self.get(keep));
        let (l, r) = (self.get(na.left), self.get(na.right));
        let a_box = union(&l.aabb, &r.aabb);
        let a_h = 1_i32.saturating_add(l.height.max(r.height));
        if let Some(n) = self.node_mut(a) {
            n.aabb = a_box;
            n.height = a_h;
        }
        if let Some(n) = self.node_mut(up) {
            n.aabb = union(&a_box, &nk.aabb);
            n.height = 1_i32.saturating_add(a_h.max(nk.height));
        }
        up
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_find_what_a_scan_finds_through_moves_and_removals() {
        let mut rng = crate::testing::Rng::new(5);
        let mut t = AabbTree::new(0.1);
        let mut boxes: Vec<Option<(Aabb, u32)>> = Vec::new();
        let rand_box = |rng: &mut crate::testing::Rng| {
            let x = f64::from(rng.unit()) * 1e4;
            let y = f64::from(rng.unit()) * 1e4;
            let w = f64::from(rng.unit()) * 50.0 + 1.0;
            [x, y, x + w, y + w]
        };
        for i in 0..5000_u32 {
            let b = rand_box(&mut rng);
            boxes.push(Some((b, t.insert(b, i))));
        }
        for _ in 0..3000 {
            let k = usize::try_from(rng.below(5000)).unwrap();
            if let Some((_, p)) = boxes[k] {
                if rng.chance(0.3) {
                    t.remove(p);
                    boxes[k] = None;
                } else {
                    let b = rand_box(&mut rng);
                    t.update(p, b);
                    boxes[k] = Some((b, p));
                }
            }
        }
        assert!(t.height() < 40, "balanced: {}", t.height());
        for _ in 0..100 {
            let q = rand_box(&mut rng);
            let q = [q[0], q[1], q[0] + 500.0, q[1] + 500.0];
            let mut found = Vec::new();
            t.query(&q, |i| found.push(i));
            for (i, b) in boxes.iter().enumerate() {
                if let Some((b, _)) = b
                    && overlaps(b, &q)
                {
                    assert!(found.contains(&u32::try_from(i).unwrap()), "box {i} missed");
                }
            }
        }
    }
}
