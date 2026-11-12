//! Per-frame extras some engines read from TexturePacker-style JSON:
//! 9-slice borders and animation groups.
//!
//! PixiJS's `Spritesheet` reads both: a frame's `"borders": {left, top,
//! right, bottom}` becomes the texture's default 9-slice borders, and a
//! top-level `"animations": {name: [frame, ...]}` becomes
//! `sheet.animations`. Source: pixijs/pixijs `src/spritesheet/Spritesheet.ts`
//! at commit 1e70768 (read 2026-11-12).

use std::collections::BTreeMap;

/// 9-slice insets, in pixels of the source image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Borders {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

/// Parse a slices file: one `<frame name> = <left> <top> <right> <bottom>`
/// per line; blank lines and `#` comments are skipped. The name is
/// everything before the last `=`, trimmed, so names may hold spaces.
pub fn parse_slices(text: &str) -> Result<BTreeMap<String, Borders>, String> {
    let mut out = BTreeMap::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = |msg: &str| format!("slices line {}: {msg}", i + 1);
        let (name, values) = line
            .rsplit_once('=')
            .ok_or_else(|| at("expected `<frame> = <left> <top> <right> <bottom>`"))?;
        let name = name.trim();
        if name.is_empty() {
            return Err(at("missing frame name"));
        }
        let nums: Vec<u32> = values
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.parse::<u32>()
                    .map_err(|_| at(&format!("`{s}` is not a pixel count")))
            })
            .collect::<Result<_, _>>()?;
        let [left, top, right, bottom] = nums[..] else {
            return Err(at("expected four numbers: left top right bottom"));
        };
        if out
            .insert(
                name.to_string(),
                Borders {
                    left,
                    top,
                    right,
                    bottom,
                },
            )
            .is_some()
        {
            return Err(at(&format!("`{name}` is listed twice")));
        }
    }
    Ok(out)
}

/// Group frames that differ only by a trailing number into animations:
/// `walk_01.png`, `walk_02.png` -> `walk`. The extension is ignored when
/// matching, a `_`, `-`, `.` or space before the number is dropped from the
/// group name, frames are ordered by their number (then by name), and only
/// groups of two or more become animations.
pub fn group_animations(names: &[String]) -> Vec<(String, Vec<String>)> {
    let mut groups: BTreeMap<String, Vec<(u64, String)>> = BTreeMap::new();
    for name in names {
        let base = strip_image_ext(name);
        let digits = base.len() - base.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 || digits > 18 {
            continue;
        }
        let prefix = &base[..base.len() - digits];
        let group = prefix.trim_end_matches(['_', '-', '.', ' ']);
        if group.is_empty() {
            continue;
        }
        let n: u64 = base[base.len() - digits..].parse().unwrap_or(0);
        groups
            .entry(group.to_string())
            .or_default()
            .push((n, name.clone()));
    }
    groups
        .into_iter()
        .filter(|(_, frames)| frames.len() >= 2)
        .map(|(group, mut frames)| {
            frames.sort();
            (group, frames.into_iter().map(|(_, n)| n).collect())
        })
        .collect()
}

fn strip_image_ext(name: &str) -> &str {
    for ext in [".png", ".jpg", ".jpeg", ".webp"] {
        if name.len() > ext.len() && name[name.len() - ext.len()..].eq_ignore_ascii_case(ext) {
            return &name[..name.len() - ext.len()];
        }
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_parse_and_reject() {
        let s = parse_slices("# ui\nbutton.png = 4 4 4 4\npanel big.png=8,6,8,6\n\n").unwrap();
        assert_eq!(
            s["button.png"],
            Borders {
                left: 4,
                top: 4,
                right: 4,
                bottom: 4
            }
        );
        assert_eq!(s["panel big.png"].top, 6);
        assert!(parse_slices("a.png = 1 2 3")
            .unwrap_err()
            .contains("line 1"));
        assert!(parse_slices("a.png = 1 2 3 x").is_err());
        assert!(parse_slices("a.png 1 2 3 4").is_err());
        assert!(parse_slices("a.png = 1 1 1 1\na.png = 2 2 2 2")
            .unwrap_err()
            .contains("twice"));
    }

    #[test]
    fn animations_group_by_trailing_number() {
        let names: Vec<String> = [
            "walk_10.png",
            "walk_2.png",
            "walk_1.png",
            "idle-0.png",
            "idle-1.png",
            "logo.png",
            "jump3.png",
            "hud/hp_1.png",
            "hud/hp_2.png",
            "single_1.png",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let g = group_animations(&names);
        assert_eq!(
            g,
            vec![
                (
                    "hud/hp".to_string(),
                    vec!["hud/hp_1.png".to_string(), "hud/hp_2.png".to_string()]
                ),
                (
                    "idle".to_string(),
                    vec!["idle-0.png".to_string(), "idle-1.png".to_string()]
                ),
                (
                    "walk".to_string(),
                    vec![
                        "walk_1.png".to_string(),
                        "walk_2.png".to_string(),
                        "walk_10.png".to_string()
                    ]
                ),
            ]
        );
    }
}
