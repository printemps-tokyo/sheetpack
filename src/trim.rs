//! Alpha trimming: find the box of pixels actually worth packing.
//!
//! Transparent margins around a sprite waste atlas space, so trimming finds
//! the tight bounding box of pixels with any alpha at all. The manifest keeps
//! the original size and the offset, so the engine puts the sprite back
//! exactly where the artist drew it.

use image::RgbaImage;

/// The opaque bounding box of an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrimBox {
    /// Left edge of the box.
    pub x: u32,
    /// Top edge of the box.
    pub y: u32,
    /// Box width, always at least 1.
    pub w: u32,
    /// Box height, always at least 1.
    pub h: u32,
    /// True when the image has no visible pixel at all. The box is then a
    /// 1x1 placeholder at the origin, so the frame still exists downstream.
    pub empty: bool,
}

/// Compute the bounding box of all pixels with alpha > 0.
pub fn alpha_bbox(img: &RgbaImage) -> TrimBox {
    let (w, h) = img.dimensions();
    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut seen = false;
    for (x, y, p) in img.enumerate_pixels() {
        if p.0[3] > 0 {
            seen = true;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    if !seen {
        return TrimBox {
            x: 0,
            y: 0,
            w: 1,
            h: 1,
            empty: true,
        };
    }
    TrimBox {
        x: min_x,
        y: min_y,
        w: max_x - min_x + 1,
        h: max_y - min_y + 1,
        empty: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn finds_the_tight_box() {
        let mut img = RgbaImage::new(10, 8);
        img.put_pixel(3, 2, Rgba([255, 0, 0, 255]));
        img.put_pixel(6, 5, Rgba([0, 255, 0, 1]));
        assert_eq!(
            alpha_bbox(&img),
            TrimBox {
                x: 3,
                y: 2,
                w: 4,
                h: 4,
                empty: false
            }
        );
    }

    #[test]
    fn opaque_image_trims_to_itself() {
        let img = RgbaImage::from_pixel(5, 6, Rgba([9, 9, 9, 255]));
        let b = alpha_bbox(&img);
        assert_eq!((b.x, b.y, b.w, b.h, b.empty), (0, 0, 5, 6, false));
    }

    #[test]
    fn fully_transparent_becomes_a_flagged_placeholder() {
        let img = RgbaImage::new(4, 4);
        let b = alpha_bbox(&img);
        assert!(b.empty);
        assert_eq!((b.w, b.h), (1, 1));
    }
}
