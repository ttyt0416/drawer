//! Bounds hierarchy keeps long text from scanning every outline at every pixel.
use super::{Contour, Point, Rect, winding};
#[derive(Default)]
pub(super) struct Set {
    paths: Vec<Contour>,
    order: Vec<usize>,
    nodes: Vec<Node>,
}
struct Node {
    bounds: Rect,
    start: usize,
    end: usize,
    children: Option<[usize; 2]>,
}
impl Set {
    pub fn new(paths: Vec<Contour>) -> Self {
        let mut order: Vec<_> = (0..paths.len()).collect();
        let mut nodes = vec![];
        if !order.is_empty() {
            build(&paths, &mut order, &mut nodes, 0);
        }
        Self {
            paths,
            order,
            nodes,
        }
    }
    pub fn winding(&self, p: Point) -> i32 {
        if self.nodes.is_empty() {
            0
        } else {
            self.at(0, p)
        }
    }
    fn at(&self, n: usize, p: Point) -> i32 {
        let node = &self.nodes[n];
        if !node.bounds.contains(p) {
            return 0;
        }
        if let Some([left, right]) = node.children {
            self.at(left, p) + self.at(right, p)
        } else {
            self.order[node.start..node.end]
                .iter()
                .map(|i| {
                    let path = &self.paths[*i];
                    if path.bounds.contains(p) {
                        winding(&path.points, p)
                    } else {
                        0
                    }
                })
                .sum()
        }
    }
}
fn build(paths: &[Contour], order: &mut [usize], nodes: &mut Vec<Node>, start: usize) -> usize {
    let mut bounds = paths[order[0]].bounds;
    for i in order.iter() {
        let b = paths[*i].bounds;
        for k in 0..2 {
            bounds.min[k] = bounds.min[k].min(b.min[k]);
            bounds.max[k] = bounds.max[k].max(b.max[k]);
        }
    }
    let n = nodes.len();
    nodes.push(Node {
        bounds,
        start,
        end: start + order.len(),
        children: None,
    });
    if order.len() > 8 {
        let k = usize::from(bounds.max[1] - bounds.min[1] > bounds.max[0] - bounds.min[0]);
        order.sort_unstable_by(|a, b| {
            let a = paths[*a].bounds;
            let b = paths[*b].bounds;
            (a.min[k] + a.max[k]).total_cmp(&(b.min[k] + b.max[k]))
        });
        let middle = order.len() / 2;
        let (left, right) = order.split_at_mut(middle);
        let a = build(paths, left, nodes, start);
        let b = build(paths, right, nodes, start + middle);
        nodes[n].children = Some([a, b]);
    }
    n
}
