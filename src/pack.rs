//! Rectangle packing: MaxRects with the best-short-side-fit rule.
//!
//! The algorithm follows Jukka Jylanki's survey "A Thousand Ways to Pack the
//! Bin" (2010): keep a list of maximal free rectangles, place each sprite in
//! the free rectangle that leaves the smallest short-side leftover, then split
//! every free rectangle the placement overlaps and prune the ones that end up
//! contained in another. Rotation is opt-in (`PackOptions::rotate`): a sprite
//! is then also tried turned 90 degrees, and kept upright on equal scores.
//!
//! Determinism is a feature, not an accident: sprites are sorted by a total
//! order (tallest-or-widest first, ties by area, ties by caller-given index,
//! which the binary assigns in name order) and every tie in free-rectangle
//! scoring breaks on position. Same inputs, same flags, same atlas, byte for
//! byte -- which is what lets `--check` and git diffs mean anything.

/// An axis-aligned rectangle inside a bin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

impl Rect {
    fn overlaps(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }

    fn contains(&self, o: &Rect) -> bool {
        o.x >= self.x
            && o.y >= self.y
            && o.x + o.w <= self.x + self.w
            && o.y + o.h <= self.y + self.h
    }
}

/// Options that shape the pages.
#[derive(Debug, Clone, Copy)]
pub struct PackOptions {
    /// Hard ceiling on page width.
    pub max_w: u32,
    /// Hard ceiling on page height.
    pub max_h: u32,
    /// Empty pixels kept between neighbouring sprites.
    pub padding: u32,
    /// Edge-pixel replication rings drawn around each sprite.
    pub extrude: u32,
    /// Empty pixels kept between sprites and the page border.
    pub border: u32,
    /// Round final page dimensions up to powers of two.
    pub pot: bool,
    /// Force final pages square.
    pub square: bool,
    /// Allow a sprite to be placed rotated 90 degrees when that fits better.
    pub rotate: bool,
}

/// One sprite placed on one page.
///
/// `x`/`y` are the top-left of the sprite's own pixels -- extrusion sits
/// outside at `x - extrude`, `y - extrude`.
#[derive(Debug, Clone, Copy)]
pub struct Placement {
    /// Index into the slice given to [`pack_pages`].
    pub index: usize,
    /// X of the sprite's pixels on the page.
    pub x: u32,
    /// Y of the sprite's pixels on the page.
    pub y: u32,
    /// True when the sprite is placed rotated 90 degrees (its page footprint
    /// is then `h` x `w`).
    pub rotated: bool,
}

/// One finished page: placements plus the trimmed-to-content page size.
#[derive(Debug, Clone)]
pub struct PackedPage {
    /// Sprites on this page.
    pub placements: Vec<Placement>,
    /// Final page width, after shrinking to content and applying pot/square.
    pub w: u32,
    /// Final page height.
    pub h: u32,
}

/// Why packing refused to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackError {
    /// A sprite cannot fit a page even alone. Holds the sprite's index and
    /// the largest sprite size the current flags allow.
    Oversize {
        /// Index into the input slice.
        index: usize,
        /// The room actually available for one sprite's pixels.
        fit_w: u32,
        /// Available height.
        fit_h: u32,
    },
}

/// The free-rectangle store for a single bin.
struct MaxRects {
    free: Vec<Rect>,
}

impl MaxRects {
    fn new(w: u32, h: u32) -> Self {
        Self {
            free: vec![Rect { x: 0, y: 0, w, h }],
        }
    }

    /// Place a `w` x `h` rectangle, best-short-side-fit, also trying it turned to `h` x `w` when
    /// `rotate` is set. Returns its top-left and whether it was turned. On
    /// equal scores the unrotated placement wins, so turning on rotation
    /// changes nothing for sprites that fit as well either way.
    fn insert_any(&mut self, w: u32, h: u32, rotate: bool) -> Option<(u32, u32, bool)> {
        // short, long, y, x, rotated, idx
        let mut best: Option<(u32, u32, u32, u32, bool, usize)> = None;
        for (i, f) in self.free.iter().enumerate() {
            for turned in [false, true] {
                if turned && (!rotate || w == h) {
                    continue;
                }
                let (cw, ch) = if turned { (h, w) } else { (w, h) };
                if f.w < cw || f.h < ch {
                    continue;
                }
                let short = (f.w - cw).min(f.h - ch);
                let long = (f.w - cw).max(f.h - ch);
                let key = (short, long, f.y, f.x, turned, i);
                if best.is_none_or(|b| key < b) {
                    best = Some(key);
                }
            }
        }
        let (.., turned, idx) = best?;
        let spot = self.free[idx];
        let (cw, ch) = if turned { (h, w) } else { (w, h) };
        let used = Rect {
            x: spot.x,
            y: spot.y,
            w: cw,
            h: ch,
        };
        self.split_all(used);
        Some((used.x, used.y, turned))
    }

    /// Split every free rectangle that overlaps `used`, then prune.
    fn split_all(&mut self, used: Rect) {
        let mut next: Vec<Rect> = Vec::with_capacity(self.free.len() + 4);
        for f in self.free.drain(..) {
            if !f.overlaps(&used) {
                next.push(f);
                continue;
            }
            // Up to four maximal children survive around the used area.
            if used.x > f.x {
                next.push(Rect {
                    x: f.x,
                    y: f.y,
                    w: used.x - f.x,
                    h: f.h,
                });
            }
            if used.x + used.w < f.x + f.w {
                next.push(Rect {
                    x: used.x + used.w,
                    y: f.y,
                    w: (f.x + f.w) - (used.x + used.w),
                    h: f.h,
                });
            }
            if used.y > f.y {
                next.push(Rect {
                    x: f.x,
                    y: f.y,
                    w: f.w,
                    h: used.y - f.y,
                });
            }
            if used.y + used.h < f.y + f.h {
                next.push(Rect {
                    x: f.x,
                    y: used.y + used.h,
                    w: f.w,
                    h: (f.y + f.h) - (used.y + used.h),
                });
            }
        }
        // Prune rectangles contained in another (identical ones keep the
        // earliest copy).
        let mut keep = vec![true; next.len()];
        for i in 0..next.len() {
            if !keep[i] {
                continue;
            }
            for j in 0..next.len() {
                if i == j || !keep[j] {
                    continue;
                }
                if next[i].contains(&next[j]) && (next[i] != next[j] || i < j) {
                    keep[j] = false;
                }
            }
        }
        self.free = next
            .into_iter()
            .zip(keep)
            .filter_map(|(r, k)| k.then_some(r))
            .collect();
    }
}

/// Round up to the next power of two, saturating at `u32::MAX`.
fn pot_up(v: u32) -> u32 {
    v.max(1).checked_next_power_of_two().unwrap_or(u32::MAX)
}

/// Pack `sizes` (sprite pixel sizes, trim already applied) into pages.
///
/// Pages are filled greedily: sprites that fit go on the current page, the
/// rest wait for the next. Each page is then shrunk to its content before
/// pot/square rounding, so a five-sprite atlas is not shipped on a 2048 x 2048
/// sheet.
pub fn pack_pages(sizes: &[(u32, u32)], opts: &PackOptions) -> Result<Vec<PackedPage>, PackError> {
    let inflate = 2 * opts.extrude + opts.padding;
    // The bin gets one trailing padding's worth of slack: the last sprite in
    // a row owes no gap to a neighbour that does not exist.
    let bin_w = (opts.max_w - (2 * opts.border).min(opts.max_w)) + opts.padding;
    let bin_h = (opts.max_h - (2 * opts.border).min(opts.max_h)) + opts.padding;

    for (i, &(w, h)) in sizes.iter().enumerate() {
        let fits = |w: u32, h: u32| w + inflate <= bin_w && h + inflate <= bin_h;
        if !(fits(w, h) || (opts.rotate && fits(h, w))) {
            return Err(PackError::Oversize {
                index: i,
                fit_w: bin_w.saturating_sub(inflate),
                fit_h: bin_h.saturating_sub(inflate),
            });
        }
    }

    // Tallest-or-widest first is the classic MaxRects ordering; the final
    // index component makes the sort total, hence deterministic.
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&i| {
        let (w, h) = sizes[i];
        (std::cmp::Reverse(w.max(h)), std::cmp::Reverse(w * h), i)
    });

    let mut pages = Vec::new();
    let mut remaining = order;
    while !remaining.is_empty() {
        let mut bin = MaxRects::new(bin_w, bin_h);
        let mut placements = Vec::new();
        let mut spill = Vec::new();
        let mut content_w = 0u32;
        let mut content_h = 0u32;
        for &i in &remaining {
            let (w, h) = sizes[i];
            match bin.insert_any(w + inflate, h + inflate, opts.rotate) {
                Some((x, y, rotated)) => {
                    // insert() returns the padded cell; the sprite's pixels
                    // sit inside it, past the border and the extrusion ring.
                    let px = x + opts.border + opts.extrude;
                    let py = y + opts.border + opts.extrude;
                    let (pw, ph) = if rotated { (h, w) } else { (w, h) };
                    placements.push(Placement {
                        index: i,
                        x: px,
                        y: py,
                        rotated,
                    });
                    content_w = content_w.max(px + pw + opts.extrude);
                    content_h = content_h.max(py + ph + opts.extrude);
                }
                None => spill.push(i),
            }
        }
        // Every sprite fits an empty bin (checked above), so a page always
        // takes at least one; this cannot loop forever.
        debug_assert!(!placements.is_empty());
        let mut w = (content_w + opts.border).min(opts.max_w).max(1);
        let mut h = (content_h + opts.border).min(opts.max_h).max(1);
        if opts.pot {
            w = pot_up(w);
            h = pot_up(h);
        }
        if opts.square {
            w = w.max(h);
            h = w;
        }
        pages.push(PackedPage { placements, w, h });
        remaining = spill;
    }
    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> PackOptions {
        PackOptions {
            max_w: 256,
            max_h: 256,
            padding: 0,
            extrude: 0,
            border: 0,
            pot: false,
            square: false,
            rotate: false,
        }
    }

    /// Sprite rectangles on a page, padded cell included, for overlap checks.
    fn cells(
        page: &PackedPage,
        sizes: &[(u32, u32)],
        o: &PackOptions,
    ) -> Vec<(u32, u32, u32, u32)> {
        page.placements
            .iter()
            .map(|p| {
                let (w, h) = sizes[p.index];
                (
                    p.x - o.extrude,
                    p.y - o.extrude,
                    w + 2 * o.extrude + o.padding,
                    h + 2 * o.extrude + o.padding,
                )
            })
            .collect()
    }

    fn assert_no_overlap(page: &PackedPage, sizes: &[(u32, u32)], o: &PackOptions) {
        let cs = cells(page, sizes, o);
        for i in 0..cs.len() {
            for j in (i + 1)..cs.len() {
                let (ax, ay, aw, ah) = cs[i];
                let (bx, by, bw, bh) = cs[j];
                let apart = ax + aw <= bx || bx + bw <= ax || ay + ah <= by || by + bh <= ay;
                assert!(apart, "cells {i} and {j} overlap: {:?} {:?}", cs[i], cs[j]);
            }
        }
    }

    #[test]
    fn everything_fits_one_page_without_overlap() {
        let sizes: Vec<(u32, u32)> = (1..=20).map(|i| (i * 3 % 40 + 5, i * 7 % 30 + 5)).collect();
        let o = opts();
        let pages = pack_pages(&sizes, &o).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].placements.len(), sizes.len());
        assert_no_overlap(&pages[0], &sizes, &o);
    }

    #[test]
    fn padding_separates_neighbours() {
        let sizes = vec![(10, 10); 9];
        let o = PackOptions {
            padding: 3,
            ..opts()
        };
        let pages = pack_pages(&sizes, &o).unwrap();
        assert_eq!(pages.len(), 1);
        assert_no_overlap(&pages[0], &sizes, &o);
        // With cells of 13, any two x-adjacent sprites are >= 13 apart.
        let mut xs: Vec<u32> = pages[0].placements.iter().map(|p| p.x).collect();
        xs.sort_unstable();
        xs.dedup();
        for w in xs.windows(2) {
            assert!(w[1] - w[0] >= 13);
        }
    }

    #[test]
    fn spills_to_a_second_page_when_full() {
        // Four 100x100 sprites cannot share a 160x160 page.
        let sizes = vec![(100, 100); 4];
        let o = PackOptions {
            max_w: 160,
            max_h: 160,
            ..opts()
        };
        let pages = pack_pages(&sizes, &o).unwrap();
        assert_eq!(pages.len(), 4);
        let placed: usize = pages.iter().map(|p| p.placements.len()).sum();
        assert_eq!(placed, 4);
    }

    #[test]
    fn page_shrinks_to_content_and_pot_rounds_up() {
        let sizes = vec![(30, 20), (30, 20)];
        let o = opts();
        let pages = pack_pages(&sizes, &o).unwrap();
        assert!(pages[0].w <= 60 && pages[0].h <= 40);

        let o = PackOptions {
            pot: true,
            ..opts()
        };
        let pages = pack_pages(&sizes, &o).unwrap();
        assert!(pages[0].w.is_power_of_two());
        assert!(pages[0].h.is_power_of_two());

        let o = PackOptions {
            square: true,
            ..opts()
        };
        let pages = pack_pages(&sizes, &o).unwrap();
        assert_eq!(pages[0].w, pages[0].h);
    }

    #[test]
    fn oversize_is_reported_with_the_available_room() {
        let sizes = vec![(10, 10), (300, 10)];
        let err = pack_pages(&sizes, &opts()).unwrap_err();
        assert_eq!(
            err,
            PackError::Oversize {
                index: 1,
                fit_w: 256,
                fit_h: 256
            }
        );
        // Extrusion and border eat into the available room.
        let o = PackOptions {
            extrude: 2,
            border: 5,
            padding: 1,
            ..opts()
        };
        let err = pack_pages(&[(250, 10)], &o).unwrap_err();
        assert_eq!(
            err,
            PackError::Oversize {
                index: 0,
                fit_w: 256 - 10 + 1 - 5,
                fit_h: 256 - 10 + 1 - 5
            }
        );
    }

    #[test]
    fn identical_inputs_pack_identically() {
        let sizes: Vec<(u32, u32)> = (1..=30).map(|i| (i % 13 + 4, i % 7 + 4)).collect();
        let o = PackOptions {
            padding: 2,
            extrude: 1,
            ..opts()
        };
        let a = pack_pages(&sizes, &o).unwrap();
        let b = pack_pages(&sizes, &o).unwrap();
        let flat = |pages: &[PackedPage]| -> Vec<(usize, u32, u32)> {
            pages
                .iter()
                .flat_map(|p| p.placements.iter().map(|q| (q.index, q.x, q.y)))
                .collect()
        };
        assert_eq!(flat(&a), flat(&b));
    }

    #[test]
    fn extrude_keeps_the_ring_inside_the_page() {
        let sizes = vec![(10, 10); 4];
        let o = PackOptions {
            extrude: 3,
            ..opts()
        };
        let pages = pack_pages(&sizes, &o).unwrap();
        for p in &pages[0].placements {
            assert!(p.x >= 3 && p.y >= 3);
            assert!(p.x + 10 + 3 <= pages[0].w);
            assert!(p.y + 10 + 3 <= pages[0].h);
        }
    }

    #[test]
    fn rotation_is_used_only_when_it_helps() {
        let o = PackOptions {
            max_w: 64,
            max_h: 8,
            rotate: true,
            ..opts()
        };
        let pages = pack_pages(&[(3, 40), (8, 8)], &o).unwrap();
        assert_eq!(pages.len(), 1);
        let tall = pages[0].placements.iter().find(|p| p.index == 0).unwrap();
        let square = pages[0].placements.iter().find(|p| p.index == 1).unwrap();
        assert!(tall.rotated && !square.rotated);
        let upright = PackOptions { rotate: false, ..o };
        assert!(pack_pages(&[(3, 40)], &upright).is_err());

        // Square sprites are never turned: it would change nothing.
        let squares = pack_pages(&[(8, 8), (8, 8), (5, 5)], &o).unwrap();
        assert!(squares
            .iter()
            .flat_map(|p| &p.placements)
            .all(|p| !p.rotated));
    }
}
