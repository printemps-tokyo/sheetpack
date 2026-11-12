//! Manifest renderers: turning placements into files an engine loads.
//!
//! Five dialects are supported. The two TexturePacker-style JSON layouts are
//! the lingua franca (PixiJS and Phaser 3 both load them), `phaser-multiatlas`
//! bundles every page into the single file Phaser's multiatlas loader wants
//! (a `textures` array whose entries carry `image` and `frames` -- see
//! Phaser's `MultiAtlasFile`), `godot-tres` writes one Godot 4 `AtlasTexture`
//! text resource per sprite, and `css` writes background-position classes for
//! plain web pages.
//!
//! Godot margin semantics, verified against `atlas_texture.cpp`: the drawn
//! size is `region.size + margin.size` and the sprite is offset by
//! `margin.position`. So for a trimmed sprite, `margin = Rect2(trim_x,
//! trim_y, source_w - w, source_h - h)` restores the original canvas exactly.

use std::fmt::Write as _;

use crate::json::{quote, sep};
use crate::model::{Frame, Meta, Page, APP, VERSION};

/// A manifest dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// TexturePacker-style JSON with a `frames` object keyed by name.
    JsonHash,
    /// TexturePacker-style JSON with a `frames` array (`filename` field).
    JsonArray,
    /// One JSON with a `textures` array, as Phaser 3's multiatlas loader reads.
    PhaserMultiatlas,
    /// One Godot 4 `AtlasTexture` `.tres` per sprite.
    GodotTres,
    /// CSS classes with background offsets.
    Css,
}

impl Format {
    /// Parse a `--format` value.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "json-hash" => Some(Self::JsonHash),
            "json-array" => Some(Self::JsonArray),
            "phaser-multiatlas" => Some(Self::PhaserMultiatlas),
            "godot-tres" => Some(Self::GodotTres),
            "css" => Some(Self::Css),
            _ => None,
        }
    }

    /// The flag spelling.
    pub fn id(self) -> &'static str {
        match self {
            Self::JsonHash => "json-hash",
            Self::JsonArray => "json-array",
            Self::PhaserMultiatlas => "phaser-multiatlas",
            Self::GodotTres => "godot-tres",
            Self::Css => "css",
        }
    }

    /// All formats, for `--list-formats`.
    pub fn all() -> &'static [Format] {
        &[
            Self::JsonHash,
            Self::JsonArray,
            Self::PhaserMultiatlas,
            Self::GodotTres,
            Self::Css,
        ]
    }

    /// One-line description for `--list-formats`.
    pub fn blurb(self) -> &'static str {
        match self {
            Self::JsonHash => "frames object keyed by name; PixiJS, Phaser 3 atlas, many others",
            Self::JsonArray => "frames array with filename fields; same consumers as json-hash",
            Self::PhaserMultiatlas => "all pages in one file for Phaser 3's multiatlas loader",
            Self::GodotTres => {
                "one Godot 4 AtlasTexture .tres per sprite (trim restored via margin)"
            }
            Self::Css => "background-position classes for plain web pages",
        }
    }

    /// Whether `--check` can read a fingerprint back out of this format.
    pub fn supports_check(self) -> bool {
        !matches!(self, Self::GodotTres)
    }
}

/// Keep letters, digits, `_` and `-`; everything else becomes `-`.
///
/// Used for CSS class names and `.tres` file names, where frame names like
/// `hero/run 0.png` are not legal.
pub fn sanitize(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if out.is_empty() {
        out.push('-');
    }
    out
}

/// Render the manifest files for `pages`.
///
/// Returns `(file name, contents)` pairs; file names are relative to the
/// directory the atlas images are written into.
pub fn render(format: Format, stem: &str, pages: &[Page], meta: &Meta) -> Vec<(String, String)> {
    match format {
        Format::JsonHash => pages
            .iter()
            .map(|p| (json_name(&p.image), page_json(p, meta, false)))
            .collect(),
        Format::JsonArray => pages
            .iter()
            .map(|p| (json_name(&p.image), page_json(p, meta, true)))
            .collect(),
        Format::PhaserMultiatlas => vec![(format!("{stem}.json"), multiatlas_json(pages, meta))],
        Format::GodotTres => godot_tres(stem, pages, meta),
        Format::Css => vec![(format!("{stem}.css"), css(pages, meta))],
    }
}

fn json_name(image: &str) -> String {
    match image.strip_suffix(".png") {
        Some(s) => format!("{s}.json"),
        None => format!("{image}.json"),
    }
}

fn rect_obj(x: u32, y: u32, w: u32, h: u32) -> String {
    format!("{{ \"x\": {x}, \"y\": {y}, \"w\": {w}, \"h\": {h} }}")
}

fn size_obj(w: u32, h: u32) -> String {
    format!("{{ \"w\": {w}, \"h\": {h} }}")
}

/// The body shared by every TexturePacker-style frame entry.
fn frame_fields(f: &Frame, indent: &str) -> String {
    let g = &f.geom;
    let mut out = format!(
        "{indent}\"frame\": {},\n\
         {indent}\"rotated\": {},\n\
         {indent}\"trimmed\": {},\n\
         {indent}\"spriteSourceSize\": {},\n\
         {indent}\"sourceSize\": {}",
        rect_obj(f.x, f.y, f.w, f.h),
        f.rotated,
        g.trimmed,
        rect_obj(g.trim_x, g.trim_y, f.w, f.h),
        size_obj(g.source_w, g.source_h),
    );
    if let Some(b) = f.borders {
        let _ = write!(
            out,
            ",\n{indent}\"borders\": {{ \"left\": {}, \"top\": {}, \"right\": {}, \"bottom\": {} }}",
            b.left, b.top, b.right, b.bottom
        );
    }
    out
}

/// The top-level `"animations"` member (with a trailing comma and newline),
/// listing the groups whose frames are all among `on_page`; empty when there
/// are none.
fn animations_member(meta: &Meta, on_page: Option<&Page>) -> String {
    let groups: Vec<&(String, Vec<String>)> = meta
        .animations
        .iter()
        .filter(|(_, frames)| {
            on_page.is_none_or(|p| {
                frames
                    .iter()
                    .all(|f| p.frames.iter().any(|pf| &pf.name == f))
            })
        })
        .collect();
    if groups.is_empty() {
        return String::new();
    }
    let mut out = String::from("  \"animations\": {\n");
    for (i, (name, frames)) in groups.iter().enumerate() {
        let list: Vec<String> = frames.iter().map(|f| quote(f)).collect();
        let _ = write!(
            out,
            "    {}: [{}]{}",
            quote(name),
            list.join(", "),
            sep(i, groups.len())
        );
    }
    out.push_str("  },\n");
    out
}

/// The `"sheetpack": {...}` provenance block inside `meta`.
fn provenance(meta: &Meta, indent: &str) -> String {
    format!(
        "{indent}\"sheetpack\": {{\n\
         {indent}  \"fingerprint\": {},\n\
         {indent}  \"settings\": {}\n\
         {indent}}}",
        quote(&meta.fingerprint),
        quote(&meta.settings),
    )
}

fn page_json(page: &Page, meta: &Meta, array: bool) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    if array {
        out.push_str("  \"frames\": [\n");
        for (i, f) in page.frames.iter().enumerate() {
            let _ = write!(
                out,
                "    {{\n      \"filename\": {},\n{}\n    }}{}",
                quote(&f.name),
                frame_fields(f, "      "),
                sep(i, page.frames.len())
            );
        }
        out.push_str("  ],\n");
    } else {
        out.push_str("  \"frames\": {\n");
        for (i, f) in page.frames.iter().enumerate() {
            let _ = write!(
                out,
                "    {}: {{\n{}\n    }}{}",
                quote(&f.name),
                frame_fields(f, "      "),
                sep(i, page.frames.len())
            );
        }
        out.push_str("  },\n");
    }
    out.push_str(&animations_member(meta, Some(page)));
    let _ = write!(
        out,
        "  \"meta\": {{\n\
         \x20   \"app\": {},\n\
         \x20   \"version\": {},\n\
         \x20   \"image\": {},\n\
         \x20   \"format\": \"RGBA8888\",\n\
         \x20   \"size\": {},\n\
         \x20   \"scale\": \"1\",\n{}\n  }}\n}}\n",
        quote(APP),
        quote(VERSION),
        quote(&page.image),
        size_obj(page.w, page.h),
        provenance(meta, "    "),
    );
    out
}

fn multiatlas_json(pages: &[Page], meta: &Meta) -> String {
    let mut out = String::new();
    out.push_str("{\n  \"textures\": [\n");
    for (pi, page) in pages.iter().enumerate() {
        let _ = write!(
            out,
            "    {{\n\
             \x20     \"image\": {},\n\
             \x20     \"format\": \"RGBA8888\",\n\
             \x20     \"size\": {},\n\
             \x20     \"scale\": 1,\n\
             \x20     \"frames\": [\n",
            quote(&page.image),
            size_obj(page.w, page.h),
        );
        for (i, f) in page.frames.iter().enumerate() {
            let _ = write!(
                out,
                "        {{\n          \"filename\": {},\n{}\n        }}{}",
                quote(&f.name),
                frame_fields(f, "          "),
                sep(i, page.frames.len())
            );
        }
        let _ = write!(out, "      ]\n    }}{}", sep(pi, pages.len()));
    }
    out.push_str("  ],\n");
    out.push_str(&animations_member(meta, None));
    let _ = write!(
        out,
        "  \"meta\": {{\n\
         \x20   \"app\": {},\n\
         \x20   \"version\": {},\n{}\n  }}\n}}\n",
        quote(APP),
        quote(VERSION),
        provenance(meta, "    "),
    );
    out
}

fn godot_tres(stem: &str, pages: &[Page], meta: &Meta) -> Vec<(String, String)> {
    let dir = format!("{stem}-sprites");
    let mut used = std::collections::HashMap::new();
    let mut out = Vec::new();
    for page in pages {
        for f in &page.frames {
            // Sanitizing can collide (`a/b` and `a_b`); suffix a counter so
            // every frame keeps a file, deterministically by name order.
            let base = sanitize(&f.name);
            let n = used.entry(base.clone()).or_insert(0u32);
            *n += 1;
            let file = if *n == 1 {
                format!("{dir}/{base}.tres")
            } else {
                format!("{dir}/{base}-{n}.tres")
            };
            let g = &f.geom;
            let mut body = format!(
                "[gd_resource type=\"AtlasTexture\" load_steps=2 format=3]\n\n\
                 [ext_resource type=\"Texture2D\" path=\"{}{}\" id=\"1\"]\n\n\
                 [resource]\n\
                 atlas = ExtResource(\"1\")\n\
                 region = Rect2({}, {}, {}, {})\n",
                meta.godot_base, page.image, f.x, f.y, f.w, f.h
            );
            if g.trimmed {
                let _ = writeln!(
                    body,
                    "margin = Rect2({}, {}, {}, {})",
                    g.trim_x,
                    g.trim_y,
                    g.source_w - f.w,
                    g.source_h - f.h
                );
            }
            out.push((file, body));
        }
    }
    out
}

fn css(pages: &[Page], meta: &Meta) -> String {
    let mut out = format!(
        "/* Generated by {APP} {VERSION}. Do not edit. */\n\
         /* fingerprint: \"{}\" */\n",
        meta.fingerprint
    );
    for page in pages {
        for f in &page.frames {
            let _ = write!(
                out,
                "\n.sp-{} {{\n\
                 \x20 background: url(\"{}\") -{}px -{}px no-repeat;\n\
                 \x20 width: {}px;\n\
                 \x20 height: {}px;\n\
                 }}\n",
                sanitize(&f.name),
                page.image,
                f.x,
                f.y,
                f.w,
                f.h
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SpriteGeom;

    fn page() -> Page {
        Page {
            image: "atlas.png".into(),
            w: 64,
            h: 32,
            occupancy: 0.5,
            frames: vec![
                Frame {
                    name: "hero/run 0.png".into(),
                    x: 2,
                    y: 2,
                    w: 10,
                    h: 12,
                    geom: SpriteGeom {
                        w: 10,
                        h: 12,
                        source_w: 16,
                        source_h: 16,
                        trim_x: 3,
                        trim_y: 1,
                        trimmed: true,
                    },
                    rotated: false,
                    borders: None,
                },
                Frame {
                    name: "tile.png".into(),
                    x: 14,
                    y: 2,
                    w: 8,
                    h: 8,
                    geom: SpriteGeom {
                        w: 8,
                        h: 8,
                        source_w: 8,
                        source_h: 8,
                        trim_x: 0,
                        trim_y: 0,
                        trimmed: false,
                    },
                    rotated: false,
                    borders: None,
                },
            ],
        }
    }

    fn meta() -> Meta {
        Meta {
            fingerprint: "00000000deadbeef".into(),
            settings: "padding=2".into(),
            godot_base: "res://".into(),
            animations: Vec::new(),
        }
    }

    #[test]
    fn json_hash_has_the_texturepacker_shape() {
        let files = render(Format::JsonHash, "atlas", &[page()], &meta());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "atlas.json");
        let doc = &files[0].1;
        assert!(doc.contains("\"hero/run 0.png\": {"));
        assert!(doc.contains("\"frame\": { \"x\": 2, \"y\": 2, \"w\": 10, \"h\": 12 }"));
        assert!(doc.contains("\"spriteSourceSize\": { \"x\": 3, \"y\": 1, \"w\": 10, \"h\": 12 }"));
        assert!(doc.contains("\"sourceSize\": { \"w\": 16, \"h\": 16 }"));
        assert!(doc.contains("\"trimmed\": true"));
        assert!(doc.contains("\"rotated\": false"));
        assert!(doc.contains("\"image\": \"atlas.png\""));
        assert!(doc.contains("\"fingerprint\": \"00000000deadbeef\""));
    }

    #[test]
    fn json_array_uses_filename_fields() {
        let files = render(Format::JsonArray, "atlas", &[page()], &meta());
        let doc = &files[0].1;
        assert!(doc.contains("\"frames\": ["));
        assert!(doc.contains("\"filename\": \"hero/run 0.png\""));
    }

    #[test]
    fn multiatlas_wraps_pages_in_a_textures_array() {
        let mut p2 = page();
        p2.image = "atlas-1.png".into();
        let files = render(Format::PhaserMultiatlas, "atlas", &[page(), p2], &meta());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "atlas.json");
        let doc = &files[0].1;
        assert!(doc.contains("\"textures\": ["));
        assert!(doc.contains("\"image\": \"atlas.png\""));
        assert!(doc.contains("\"image\": \"atlas-1.png\""));
        assert!(doc.contains("\"scale\": 1,"));
    }

    #[test]
    fn godot_tres_restores_trim_via_margin() {
        let files = render(Format::GodotTres, "atlas", &[page()], &meta());
        assert_eq!(files.len(), 2);
        let (name, body) = &files[0];
        assert_eq!(name, "atlas-sprites/hero-run-0-png.tres");
        assert!(body.contains("path=\"res://atlas.png\""));
        assert!(body.contains("region = Rect2(2, 2, 10, 12)"));
        // margin.size = source - trimmed, margin.position = trim offset.
        assert!(body.contains("margin = Rect2(3, 1, 6, 4)"));
        // The untrimmed sprite carries no margin line.
        assert!(!files[1].1.contains("margin"));
    }

    #[test]
    fn godot_tres_disambiguates_sanitize_collisions() {
        let mut p = page();
        p.frames[0].name = "a/b".into();
        p.frames[1].name = "a_b".into();
        let files = render(Format::GodotTres, "atlas", &[p], &meta());
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["atlas-sprites/a-b.tres", "atlas-sprites/a_b.tres"]);
        let mut p = page();
        p.frames[0].name = "a/b".into();
        p.frames[1].name = "a-b".into();
        let files = render(Format::GodotTres, "atlas", &[p], &meta());
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            ["atlas-sprites/a-b.tres", "atlas-sprites/a-b-2.tres"]
        );
    }

    #[test]
    fn css_offsets_are_negative() {
        let files = render(Format::Css, "atlas", &[page()], &meta());
        assert_eq!(files[0].0, "atlas.css");
        let doc = &files[0].1;
        assert!(doc.contains(".sp-hero-run-0-png {"));
        assert!(doc.contains("url(\"atlas.png\") -2px -2px no-repeat"));
        assert!(doc.contains("width: 10px;"));
    }

    #[test]
    fn sanitize_keeps_safe_chars_only() {
        assert_eq!(sanitize("hero/run 0.png"), "hero-run-0-png");
        assert_eq!(sanitize("ok_name-1"), "ok_name-1");
        assert_eq!(sanitize(""), "-");
    }
}
