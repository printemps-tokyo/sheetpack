//! Data shapes shared across the pipeline.
//!
//! Geometry lives here, pixels do not: the packer and the manifest renderers
//! only ever see rectangles, so they stay testable without decoding a single
//! image.

/// Where a sprite's opaque pixels sit inside its source image.
///
/// With trimming off, the "trimmed" rectangle is simply the whole image and
/// `trimmed` is false.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpriteGeom {
    /// Packed width -- the trimmed width when trimming is on.
    pub w: u32,
    /// Packed height.
    pub h: u32,
    /// Width of the original image, before trimming.
    pub source_w: u32,
    /// Height of the original image.
    pub source_h: u32,
    /// Left offset of the trimmed rectangle inside the original.
    pub trim_x: u32,
    /// Top offset of the trimmed rectangle inside the original.
    pub trim_y: u32,
    /// True when trimming actually removed something.
    pub trimmed: bool,
}

/// One entry a manifest renderer writes: a named sprite at a fixed place on
/// a page, plus enough source geometry to restore the trimmed padding.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Frame key as the engine will ask for it.
    pub name: String,
    /// X of the sprite's pixels on the page (extrusion excluded).
    pub x: u32,
    /// Y of the sprite's pixels on the page.
    pub y: u32,
    /// Sprite width on the page.
    pub w: u32,
    /// Sprite height on the page.
    pub h: u32,
    /// Source geometry, for `spriteSourceSize` / Godot margins.
    pub geom: SpriteGeom,
}

/// One rendered page: an image file plus the frames that live on it.
#[derive(Debug, Clone)]
pub struct Page {
    /// File name of the page image, without directories.
    pub image: String,
    /// Page width in pixels.
    pub w: u32,
    /// Page height in pixels.
    pub h: u32,
    /// Frames on this page, sorted by name.
    pub frames: Vec<Frame>,
    /// Fraction of the page area covered by sprite pixels, 0..=1.
    pub occupancy: f64,
}

/// Everything a manifest embeds about the run itself.
#[derive(Debug, Clone)]
pub struct Meta {
    /// Fingerprint of inputs + settings; what `--check` compares against.
    pub fingerprint: String,
    /// Human-readable settings line, for provenance.
    pub settings: String,
    /// Prefix for image paths in Godot `.tres` output, e.g. `res://`.
    pub godot_base: String,
}

/// Tool name embedded in manifests.
pub const APP: &str = "sheetpack";
/// Tool version embedded in manifests.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
