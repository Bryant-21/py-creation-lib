/// Binary-tree guillotine bin packer — port of `TwbBinPacker` (wbLOD.pas:461-558).
///
/// Algorithm: Jake Gordon's binary-tree packer. Each placed block splits the
/// remaining free space into a `right` node (to the right of the block, same row
/// height) and a `down` node (below the block, full width).  Blocks are sorted
/// descending by `max(w,h)` before packing (MaxSideSort, wbLOD.pas:517-535).
///
/// Object atlas uses padding (0, 0); tree atlas uses (2, 2) — caller sets at construction.

/// One block to be placed in the atlas.
#[derive(Clone, Debug, PartialEq)]
pub struct BinBlock {
    /// Caller-assigned index (for tracking back to source after sort).
    pub index: usize,
    /// Block width in pixels.
    pub w: u32,
    /// Block height in pixels.
    pub h: u32,
    /// X position in the atlas (set by `BinPacker::fit`).
    pub x: u32,
    /// Y position in the atlas (set by `BinPacker::fit`).
    pub y: u32,
    /// True if the block was successfully placed.
    pub fit: bool,
}

// Internal arena tree node (wbLOD.pas:43-50).
struct Node {
    used: bool,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    right: Option<Box<Node>>,
    down: Option<Box<Node>>,
}

impl Node {
    fn new(x: u32, y: u32, w: u32, h: u32) -> Box<Self> {
        Box::new(Node {
            used: false,
            x,
            y,
            w,
            h,
            right: None,
            down: None,
        })
    }
}

/// Guillotine bin packer — port of `TwbBinPacker` (wbLOD.pas:461-558).
pub struct BinPacker {
    pub width: u32,
    pub height: u32,
    pub padding_x: u32,
    pub padding_y: u32,
}

impl BinPacker {
    /// Create a packer with zero padding (object atlas).
    /// Tree atlas: call `BinPacker { padding_x: 2, padding_y: 2, .. }` directly.
    pub fn new(width: u32, height: u32) -> Self {
        BinPacker {
            width,
            height,
            padding_x: 0,
            padding_y: 0,
        }
    }

    /// Sort blocks descending by `max(w,h)`, then fit them into the atlas.
    /// Returns `true` iff every block was placed (fit=true).
    ///
    /// Port: wbLOD.pas:515-558 `TwbBinPacker.Fit` + `MaxSideSort`.
    pub fn fit(&self, blocks: &mut Vec<BinBlock>) -> bool {
        // MaxSideSort — stable bubble-sort descending by max(w,h).
        // port: wbLOD.pas:517-535
        let mut changed = true;
        while changed {
            changed = false;
            for i in 0..blocks.len().saturating_sub(1) {
                let ai = blocks[i].w.max(blocks[i].h);
                let bi = blocks[i + 1].w.max(blocks[i + 1].h);
                if ai < bi {
                    blocks.swap(i, i + 1);
                    changed = true;
                }
            }
        }

        // Build root node covering the full atlas.
        let mut root = Node::new(0, 0, self.width, self.height);
        let mut all_fit = true;

        for block in blocks.iter_mut() {
            if let Some(node) = find_node(&mut root, block.w, block.h) {
                // SAFETY: pointer is derived from a live &mut Node in the tree.
                let (nx, ny) = unsafe { ((*node).x, (*node).y) };
                split_node(node, block.w, block.h, self.padding_x, self.padding_y);
                block.x = nx;
                block.y = ny;
                block.fit = true;
            } else {
                // block.fit stays false
                all_fit = false;
            }
        }

        all_fit
    }
}

/// Find a node in the tree that can fit a block of size (w, h).
/// port: wbLOD.pas:486-497 FindNode — if used, try right then down;
/// else if the block fits, return this node.
fn find_node(node: &mut Node, w: u32, h: u32) -> Option<*mut Node> {
    if node.used {
        if let Some(ref mut r) = node.right {
            let res = find_node(r, w, h);
            if res.is_some() {
                return res;
            }
        }
        if let Some(ref mut d) = node.down {
            return find_node(d, w, h);
        }
        None
    } else if w <= node.w && h <= node.h {
        Some(node as *mut Node)
    } else {
        None
    }
}

/// Mark the node used and create right + down children.
/// port: wbLOD.pas:499-513 SplitNode
///
/// # Safety
/// The pointer is always derived from a `&mut Node` that is alive for the
/// duration of the call — we never outlive the tree.
fn split_node(node_ptr: *mut Node, w: u32, h: u32, pad_x: u32, pad_y: u32) {
    // SAFETY: pointer comes from a live `&mut Node` in `fit`.
    let node = unsafe { &mut *node_ptr };
    node.used = true;
    // right: to the right of the block, same row height as the block
    node.right = Some(Node::new(
        node.x + w + pad_x,
        node.y,
        node.w.saturating_sub(w + pad_x),
        h,
    ));
    // down: below the block, full width of parent, reduced height
    node.down = Some(Node::new(
        node.x,
        node.y + h + pad_y,
        node.w,
        node.h.saturating_sub(h + pad_y),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binpacker_padding_object_atlas_is_zero() {
        let p = BinPacker::new(4096, 4096);
        assert_eq!(p.padding_x, 0);
        assert_eq!(p.padding_y, 0);
    }
}
