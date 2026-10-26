//! Splitting a legacy WhiteBrowser thumbnail (one image holding several
//! frames side by side) into its frames. Pure geometry: the `.wb` setting is
//! parsed here, the image itself is cut by the ffmpeg adapter.

use crate::ports::ffmpeg::CropRect;

/// The frame grid a legacy thumbnail was made with (`thum_size` in the `.wb`
/// `profile` table, written `<width>x<height>x<cols>x<rows>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThumbLayout {
    pub tile_w: u32,
    pub tile_h: u32,
    pub cols: u32,
    pub rows: u32,
}

impl ThumbLayout {
    /// Number of frames, or `None` if the product overflows.
    pub fn tile_count(&self) -> Option<u32> {
        self.cols.checked_mul(self.rows)
    }
}

/// Parses `thum_size`. Exactly four `x`-separated positive integers;
/// anything else is `None` so an unrecognized setting never produces a crop.
pub fn parse_thum_size(value: &str) -> Option<ThumbLayout> {
    let mut parts = value.trim().split('x');
    let mut next = || -> Option<u32> {
        let n = parts.next()?.trim().parse::<u32>().ok()?;
        (n > 0).then_some(n)
    };
    let layout = ThumbLayout {
        tile_w: next()?,
        tile_h: next()?,
        cols: next()?,
        rows: next()?,
    };
    if parts.next().is_some() {
        return None;
    }
    Some(layout)
}

/// True only when the image is exactly `tile_w * cols` by `tile_h * rows`.
/// The setting describes how thumbnails were made, not this particular file,
/// so a differently sized image (hand-edited, other skin) must not be cut by
/// it.
pub fn layout_matches_image(layout: &ThumbLayout, img_w: u32, img_h: u32) -> bool {
    layout.tile_w.checked_mul(layout.cols) == Some(img_w)
        && layout.tile_h.checked_mul(layout.rows) == Some(img_h)
}

/// Crop rectangles of every frame, left to right then top to bottom.
pub fn tile_rects(layout: &ThumbLayout) -> Vec<CropRect> {
    let mut rects = Vec::new();
    for row in 0..layout.rows {
        for col in 0..layout.cols {
            rects.push(CropRect {
                x: col * layout.tile_w,
                y: row * layout.tile_h,
                w: layout.tile_w,
                h: layout.tile_h,
            });
        }
    }
    rects
}

/// Crop rectangles to use for an image of `img_w` x `img_h`, or `None` when
/// the image must be imported whole: the layout does not describe the image,
/// or it does not hold exactly `expected_tiles` frames (one per thumbnail
/// slot, so a different grid cannot fill the slots one-to-one).
pub fn split_plan(
    layout: &ThumbLayout,
    img_w: u32,
    img_h: u32,
    expected_tiles: u32,
) -> Option<Vec<CropRect>> {
    if layout.tile_count() != Some(expected_tiles) || !layout_matches_image(layout, img_w, img_h) {
        return None;
    }
    Some(tile_rects(layout))
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: ThumbLayout = ThumbLayout {
        tile_w: 200,
        tile_h: 150,
        cols: 6,
        rows: 1,
    };

    #[test]
    fn parses_the_known_layout() {
        assert_eq!(parse_thum_size("200x150x6x1"), Some(L));
        assert_eq!(parse_thum_size(" 200x150x6x1 "), Some(L));
    }

    #[test]
    fn rejects_malformed_values() {
        for v in [
            "",
            "200x150x6",
            "200x150x6x1x1",
            "0x150x6x1",
            "200x150x0x1",
            "200x150x6x0",
            "axbxcxd",
            "200x150x6x-1",
            "200x150x6x",
            "200,150,6,1",
            "x150x6x1",
        ] {
            assert_eq!(parse_thum_size(v), None, "{v:?}");
        }
    }

    #[test]
    fn matches_only_the_exact_image_size() {
        assert!(layout_matches_image(&L, 1200, 150));
        assert!(!layout_matches_image(&L, 1200, 151));
        assert!(!layout_matches_image(&L, 1199, 150));
        assert!(!layout_matches_image(&L, 150, 1200));
        assert!(!layout_matches_image(&L, 0, 0));
    }

    #[test]
    fn match_does_not_overflow() {
        let big = ThumbLayout {
            tile_w: u32::MAX,
            tile_h: 1,
            cols: 2,
            rows: 1,
        };
        assert!(!layout_matches_image(&big, 0, 1));
        assert_eq!(big.tile_count(), Some(2));
    }

    #[test]
    fn six_by_one_tiles_are_left_to_right() {
        let rects = tile_rects(&L);
        assert_eq!(rects.len(), 6);
        for (i, r) in rects.iter().enumerate() {
            assert_eq!(
                *r,
                CropRect {
                    x: i as u32 * 200,
                    y: 0,
                    w: 200,
                    h: 150
                }
            );
        }
    }

    #[test]
    fn tiles_are_row_major() {
        let l = ThumbLayout {
            tile_w: 100,
            tile_h: 50,
            cols: 3,
            rows: 2,
        };
        let rects = tile_rects(&l);
        assert_eq!(rects.len(), 6);
        assert_eq!((rects[0].x, rects[0].y), (0, 0));
        assert_eq!((rects[2].x, rects[2].y), (200, 0));
        assert_eq!((rects[3].x, rects[3].y), (0, 50));
        assert_eq!((rects[5].x, rects[5].y), (200, 50));
    }

    #[test]
    fn splits_only_exactly_six_matching_tiles() {
        assert_eq!(split_plan(&L, 1200, 150, 6).map(|r| r.len()), Some(6));
        // Size mismatch.
        assert_eq!(split_plan(&L, 1200, 160, 6), None);
        // A 3x2 grid also has six tiles and is accepted.
        let grid = ThumbLayout {
            tile_w: 100,
            tile_h: 50,
            cols: 3,
            rows: 2,
        };
        assert_eq!(split_plan(&grid, 300, 100, 6).map(|r| r.len()), Some(6));
        // Not six tiles.
        let four = ThumbLayout { cols: 4, ..L };
        assert_eq!(split_plan(&four, 800, 150, 6), None);
        let twelve = ThumbLayout { cols: 12, ..L };
        assert_eq!(split_plan(&twelve, 2400, 150, 6), None);
    }
}
