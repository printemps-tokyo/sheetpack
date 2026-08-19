//! Black-box tests of the binary: modes, flags, exit codes, manifest shape,
//! determinism and the --check contract.
//!
//! Every sprite is built programmatically, so the suite carries no fixtures,
//! needs no network and produces the same bytes on every machine.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use image::{Rgba, RgbaImage};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Path to the binary under test, as cargo built it.
fn bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe path");
    p.pop(); // the deps/ directory
    if p.ends_with("deps") {
        p.pop();
    }
    p.join(format!("sheetpack{}", std::env::consts::EXE_SUFFIX))
}

fn run_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(bin())
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run sheetpack")
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("utf-8 stdout")
}

fn stderr(out: &Output) -> String {
    String::from_utf8(out.stderr.clone()).expect("utf-8 stderr")
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

/// Write a solid-colour sprite.
fn sprite(path: &Path, w: u32, h: u32, rgba: [u8; 4]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    RgbaImage::from_pixel(w, h, Rgba(rgba)).save(path).unwrap();
}

/// Count pixels of an exact colour in an image.
fn count(img: &RgbaImage, rgba: [u8; 4]) -> usize {
    img.pixels().filter(|p| p.0 == rgba).count()
}

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

/// A directory with three solid sprites of distinct colours and sizes.
fn basic_sprites(root: &Path) {
    sprite(&root.join("sprites/a.png"), 10, 12, RED);
    sprite(&root.join("sprites/b.png"), 8, 8, GREEN);
    sprite(&root.join("sprites/sub/c.png"), 6, 4, BLUE);
}

// ---------------------------------------------------------------------------
// Packing
// ---------------------------------------------------------------------------

#[test]
fn packs_sprites_and_writes_manifest() {
    let tmp = TempDir::new().unwrap();
    basic_sprites(tmp.path());
    let out = run_in(tmp.path(), &["sprites", "-o", "out/atlas"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));

    let atlas = image::open(tmp.path().join("out/atlas.png"))
        .unwrap()
        .to_rgba8();
    // Every sprite's pixels are present, exactly once each.
    assert_eq!(count(&atlas, RED), 10 * 12);
    assert_eq!(count(&atlas, GREEN), 8 * 8);
    assert_eq!(count(&atlas, BLUE), 6 * 4);

    let json = fs::read_to_string(tmp.path().join("out/atlas.json")).unwrap();
    assert!(json.contains("\"a.png\""));
    assert!(json.contains("\"sub/c.png\""));
    assert!(json.contains("\"rotated\": false"));
    assert!(json.contains("\"fingerprint\": \""));
    assert!(stdout(&out).contains("wrote:"));
}

#[test]
fn output_is_deterministic_across_runs() {
    let tmp = TempDir::new().unwrap();
    basic_sprites(tmp.path());
    let a = run_in(tmp.path(), &["sprites", "-o", "one/atlas"]);
    let b = run_in(tmp.path(), &["sprites", "-o", "two/atlas"]);
    assert_eq!(code(&a), 0);
    assert_eq!(code(&b), 0);
    let png_a = fs::read(tmp.path().join("one/atlas.png")).unwrap();
    let png_b = fs::read(tmp.path().join("two/atlas.png")).unwrap();
    assert_eq!(png_a, png_b);
    let json_a = fs::read_to_string(tmp.path().join("one/atlas.json")).unwrap();
    let json_b = fs::read_to_string(tmp.path().join("two/atlas.json")).unwrap();
    assert_eq!(json_a, json_b);
}

#[test]
fn trim_records_source_geometry() {
    let tmp = TempDir::new().unwrap();
    // An 8x8 canvas whose visible pixels are a 2x3 block at (3, 2).
    let mut img = RgbaImage::new(8, 8);
    for y in 2..5 {
        for x in 3..5 {
            img.put_pixel(x, y, Rgba(RED));
        }
    }
    fs::create_dir_all(tmp.path().join("sprites")).unwrap();
    img.save(tmp.path().join("sprites/t.png")).unwrap();

    let out = run_in(tmp.path(), &["sprites", "-o", "atlas", "--trim"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let json = fs::read_to_string(tmp.path().join("atlas.json")).unwrap();
    assert!(json.contains("\"trimmed\": true"));
    assert!(json.contains("\"spriteSourceSize\": { \"x\": 3, \"y\": 2, \"w\": 2, \"h\": 3 }"));
    assert!(json.contains("\"sourceSize\": { \"w\": 8, \"h\": 8 }"));
    let atlas = image::open(tmp.path().join("atlas.png"))
        .unwrap()
        .to_rgba8();
    assert_eq!(count(&atlas, RED), 6);
}

#[test]
fn dedup_shares_rectangles_but_keeps_names() {
    let tmp = TempDir::new().unwrap();
    sprite(&tmp.path().join("sprites/one.png"), 9, 9, RED);
    sprite(&tmp.path().join("sprites/two.png"), 9, 9, RED);
    sprite(&tmp.path().join("sprites/other.png"), 9, 9, GREEN);
    let out = run_in(tmp.path(), &["sprites", "-o", "atlas", "--json"]);
    assert_eq!(code(&out), 0);
    let report = stdout(&out);
    assert!(report.contains("\"sprites\": 3,"));
    assert!(report.contains("\"unique\": 2,"));
    assert!(report.contains("\"duplicates\": 1,"));
    // Only two rectangles' worth of red+green pixels on the page.
    let atlas = image::open(tmp.path().join("atlas.png"))
        .unwrap()
        .to_rgba8();
    assert_eq!(count(&atlas, RED), 81);
    assert_eq!(count(&atlas, GREEN), 81);
    // Both names still resolve in the manifest.
    let json = fs::read_to_string(tmp.path().join("atlas.json")).unwrap();
    assert!(json.contains("\"one.png\""));
    assert!(json.contains("\"two.png\""));

    let out = run_in(
        tmp.path(),
        &["sprites", "-o", "nod/atlas", "--no-dedup", "--json"],
    );
    assert!(stdout(&out).contains("\"duplicates\": 0,"));
}

#[test]
fn extrude_replicates_sprite_edges() {
    let tmp = TempDir::new().unwrap();
    sprite(&tmp.path().join("sprites/r.png"), 4, 4, RED);
    let out = run_in(
        tmp.path(),
        &["sprites", "-o", "atlas", "--extrude", "2", "--padding", "0"],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let atlas = image::open(tmp.path().join("atlas.png"))
        .unwrap()
        .to_rgba8();
    // A lone 4x4 sprite with a 2-ring becomes a solid 8x8 page.
    assert_eq!(atlas.dimensions(), (8, 8));
    assert_eq!(count(&atlas, RED), 64);
}

#[test]
fn multipack_spills_to_numbered_pages() {
    let tmp = TempDir::new().unwrap();
    sprite(&tmp.path().join("sprites/a.png"), 50, 50, RED);
    sprite(&tmp.path().join("sprites/b.png"), 50, 50, GREEN);
    sprite(&tmp.path().join("sprites/c.png"), 50, 50, BLUE);
    let out = run_in(tmp.path(), &["sprites", "-o", "atlas", "--max-size", "64"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    for i in 0..3 {
        assert!(tmp.path().join(format!("atlas-{i}.png")).is_file());
        assert!(tmp.path().join(format!("atlas-{i}.json")).is_file());
    }

    // The multiatlas dialect bundles the same pages into one file.
    let out = run_in(
        tmp.path(),
        &[
            "sprites",
            "-o",
            "multi/atlas",
            "--max-size",
            "64",
            "--format",
            "phaser-multiatlas",
        ],
    );
    assert_eq!(code(&out), 0);
    let json = fs::read_to_string(tmp.path().join("multi/atlas.json")).unwrap();
    assert!(json.contains("\"textures\": ["));
    assert!(json.contains("\"image\": \"atlas-0.png\""));
    assert!(json.contains("\"image\": \"atlas-2.png\""));
    assert!(!tmp.path().join("multi/atlas-0.json").exists());
}

// ---------------------------------------------------------------------------
// Other formats
// ---------------------------------------------------------------------------

#[test]
fn css_and_godot_formats_write_their_files() {
    let tmp = TempDir::new().unwrap();
    basic_sprites(tmp.path());

    let out = run_in(
        tmp.path(),
        &["sprites", "-o", "css/atlas", "--format", "css"],
    );
    assert_eq!(code(&out), 0);
    let css = fs::read_to_string(tmp.path().join("css/atlas.css")).unwrap();
    assert!(css.contains(".sp-a-png {"));
    assert!(css.contains("no-repeat"));

    let out = run_in(
        tmp.path(),
        &[
            "sprites",
            "-o",
            "gd/atlas",
            "--format",
            "godot-tres",
            "--strip-ext",
        ],
    );
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let tres = fs::read_to_string(tmp.path().join("gd/atlas-sprites/a.tres")).unwrap();
    assert!(tres.contains("[gd_resource type=\"AtlasTexture\""));
    assert!(tres.contains("path=\"res://atlas.png\""));
    assert!(tmp.path().join("gd/atlas-sprites/sub-c.tres").is_file());
}

#[test]
fn strip_ext_drops_extensions_from_names() {
    let tmp = TempDir::new().unwrap();
    basic_sprites(tmp.path());
    let out = run_in(tmp.path(), &["sprites", "-o", "atlas", "--strip-ext"]);
    assert_eq!(code(&out), 0);
    let json = fs::read_to_string(tmp.path().join("atlas.json")).unwrap();
    assert!(json.contains("\"sub/c\": {"));
    assert!(!json.contains("\"sub/c.png\""));
}

// ---------------------------------------------------------------------------
// --check
// ---------------------------------------------------------------------------

#[test]
fn check_passes_then_fails_after_a_source_changes() {
    let tmp = TempDir::new().unwrap();
    basic_sprites(tmp.path());
    let out = run_in(tmp.path(), &["sprites", "-o", "atlas"]);
    assert_eq!(code(&out), 0);

    let out = run_in(tmp.path(), &["sprites", "-o", "atlas", "--check"]);
    assert_eq!(code(&out), 0, "stdout: {}", stdout(&out));
    assert!(stdout(&out).contains("status: current"));

    // Repaint one sprite; the committed atlas is now stale.
    sprite(&tmp.path().join("sprites/a.png"), 10, 12, BLUE);
    let out = run_in(tmp.path(), &["sprites", "-o", "atlas", "--check"]);
    assert_eq!(code(&out), 1);
    assert!(stdout(&out).contains("status: stale"));

    // Different flags also count as stale: the atlas was not built this way.
    sprite(&tmp.path().join("sprites/a.png"), 10, 12, RED);
    let out = run_in(
        tmp.path(),
        &["sprites", "-o", "atlas", "--check", "--padding", "7"],
    );
    assert_eq!(code(&out), 1);
}

#[test]
fn check_reports_missing_manifest_and_deleted_page() {
    let tmp = TempDir::new().unwrap();
    basic_sprites(tmp.path());
    let out = run_in(tmp.path(), &["sprites", "-o", "atlas", "--check"]);
    assert_eq!(code(&out), 1);
    assert!(stdout(&out).contains("manifest: not found"));

    let out = run_in(tmp.path(), &["sprites", "-o", "atlas"]);
    assert_eq!(code(&out), 0);
    fs::remove_file(tmp.path().join("atlas.png")).unwrap();
    let out = run_in(tmp.path(), &["sprites", "-o", "atlas", "--check", "--json"]);
    assert_eq!(code(&out), 1);
    assert!(stdout(&out).contains("\"missing_images\": [\"atlas.png\"]"));
}

#[test]
fn check_refuses_formats_without_a_manifest() {
    let tmp = TempDir::new().unwrap();
    basic_sprites(tmp.path());
    let out = run_in(
        tmp.path(),
        &[
            "sprites",
            "-o",
            "atlas",
            "--format",
            "godot-tres",
            "--check",
        ],
    );
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("not supported"));
}

// ---------------------------------------------------------------------------
// Failure modes
// ---------------------------------------------------------------------------

#[test]
fn dry_run_writes_nothing() {
    let tmp = TempDir::new().unwrap();
    basic_sprites(tmp.path());
    let out = run_in(tmp.path(), &["sprites", "-o", "out/atlas", "--dry-run"]);
    assert_eq!(code(&out), 0);
    assert!(stdout(&out).contains("nothing written"));
    assert!(!tmp.path().join("out").exists());
}

#[test]
fn duplicate_frame_names_are_a_usage_error() {
    let tmp = TempDir::new().unwrap();
    basic_sprites(tmp.path());
    sprite(&tmp.path().join("elsewhere/a.png"), 5, 5, GREEN);
    let out = run_in(tmp.path(), &["sprites", "elsewhere/a.png", "-o", "atlas"]);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("duplicate frame names"));
}

#[test]
fn oversize_sprite_exits_1_with_the_available_room() {
    let tmp = TempDir::new().unwrap();
    sprite(&tmp.path().join("sprites/huge.png"), 100, 100, RED);
    let out = run_in(tmp.path(), &["sprites", "-o", "atlas", "--max-size", "64"]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("huge.png is 100x100"));
}

#[test]
fn flag_validation_is_a_usage_error() {
    let tmp = TempDir::new().unwrap();
    basic_sprites(tmp.path());
    let out = run_in(tmp.path(), &["sprites", "--pot", "--max-size", "1000"]);
    assert_eq!(code(&out), 2);
    let out = run_in(tmp.path(), &["sprites", "--square", "--max-size", "64x32"]);
    assert_eq!(code(&out), 2);
    let out = run_in(tmp.path(), &["sprites", "--format", "nope"]);
    assert_eq!(code(&out), 2);
    let out = run_in(tmp.path(), &[]);
    assert_eq!(code(&out), 2);
}

#[test]
fn list_formats_names_every_dialect() {
    let tmp = TempDir::new().unwrap();
    let out = run_in(tmp.path(), &["--list-formats"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    for id in [
        "json-hash",
        "json-array",
        "phaser-multiatlas",
        "godot-tres",
        "css",
    ] {
        assert!(text.contains(id), "missing {id}");
    }
}

#[test]
fn pot_and_square_shape_the_page() {
    let tmp = TempDir::new().unwrap();
    basic_sprites(tmp.path());
    let out = run_in(tmp.path(), &["sprites", "-o", "atlas", "--pot", "--square"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let atlas = image::open(tmp.path().join("atlas.png"))
        .unwrap()
        .to_rgba8();
    let (w, h) = atlas.dimensions();
    assert_eq!(w, h);
    assert!(w.is_power_of_two());
}
