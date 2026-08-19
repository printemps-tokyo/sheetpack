//! Command-line entry point for sheetpack.
//!
//! This layer turns flags into settings, moves bytes on and off the
//! filesystem, and prints. Every decision about where a sprite lands, what a
//! manifest says and whether an atlas is stale lives in the library, over
//! in-memory data, where it is testable without a temp directory.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::Parser;
use image::{ImageReader, RgbaImage};

use sheetpack::check::scan_manifest;
use sheetpack::fingerprint::{add_source, hex, Fnv1a64};
use sheetpack::formats::{render, Format};
use sheetpack::json::{quote, sep};
use sheetpack::model::{Frame, Meta, Page, SpriteGeom};
use sheetpack::pack::{pack_pages, PackError, PackOptions};
use sheetpack::trim::alpha_bbox;

/// File extensions picked up when an input is a directory.
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];

const AFTER_HELP: &str = "\
EXIT CODES:
  0  the run did what it was asked to do
  1  --check found the atlas stale or incomplete, or a sprite is larger
     than one page can hold
  2  usage error, unreadable input, or duplicate frame names

MODES:
  (default)    pack the sprites and write pages + manifest
  --dry-run    plan and report, write nothing
  --check      recompute the input fingerprint and compare it with the one
               stored in the manifest; for CI, after committing an atlas

DETERMINISM:
  Same sprites, same flags, same bytes out. Inputs are sorted by frame name,
  packing breaks every tie by position, and no timestamp or randomness enters
  the output. Two machines produce identical atlases, so both the images and
  the manifest can live in git and --check can defend them.

CI RECIPE:
  sheetpack sprites/ -o assets/atlas --trim          # regenerate, commit
  sheetpack sprites/ -o assets/atlas --trim --check  # fails the build when
                                                     # sprites/ drifted";

/// Pack sprites into texture atlases, deterministically, for CI.
#[derive(Parser, Debug)]
#[command(name = "sheetpack", version, about, after_help = AFTER_HELP)]
struct Cli {
    /// Sprite image files, or directories searched recursively.
    #[arg(value_name = "INPUT")]
    inputs: Vec<PathBuf>,

    /// Output stem: pages become <STEM>.png (or <STEM>-N.png), the manifest
    /// sits next to them.
    #[arg(short, long, default_value = "atlas", value_name = "STEM")]
    output: PathBuf,

    /// Manifest dialect; see --list-formats.
    #[arg(long, default_value = "json-hash", value_name = "ID")]
    format: String,

    /// Page size ceiling: one number, or WxH.
    #[arg(long, default_value = "2048", value_name = "N|WxH")]
    max_size: String,

    /// Empty pixels kept between sprites.
    #[arg(long, default_value_t = 2, value_name = "N")]
    padding: u32,

    /// Edge-replication rings around each sprite, against texture bleeding.
    #[arg(long, default_value_t = 0, value_name = "N")]
    extrude: u32,

    /// Empty pixels kept between sprites and the page edge.
    #[arg(long, default_value_t = 0, value_name = "N")]
    border: u32,

    /// Trim transparent margins; offsets are recorded so engines restore them.
    #[arg(long)]
    trim: bool,

    /// Round page sizes up to powers of two.
    #[arg(long)]
    pot: bool,

    /// Force pages square.
    #[arg(long)]
    square: bool,

    /// Give pixel-identical sprites their own rectangle instead of sharing one.
    #[arg(long)]
    no_dedup: bool,

    /// Drop file extensions from frame names.
    #[arg(long)]
    strip_ext: bool,

    /// Path prefix for the atlas image in godot-tres output.
    #[arg(long, default_value = "res://", value_name = "PREFIX")]
    godot_base: String,

    /// Plan and report without writing anything.
    #[arg(long)]
    dry_run: bool,

    /// Verify the committed atlas still matches the inputs; exit 1 if stale.
    #[arg(long)]
    check: bool,

    /// Machine-readable run report on stdout.
    #[arg(long)]
    json: bool,

    /// Print nothing but errors.
    #[arg(short, long)]
    quiet: bool,

    /// List manifest formats and exit.
    #[arg(long)]
    list_formats: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("sheetpack: error: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: &Cli) -> Result<u8> {
    if cli.list_formats {
        print_formats();
        return Ok(0);
    }

    let format = Format::parse(&cli.format)
        .with_context(|| format!("unknown --format '{}'; try --list-formats", cli.format))?;
    let (max_w, max_h) = parse_max_size(&cli.max_size)?;
    if cli.pot && (!max_w.is_power_of_two() || !max_h.is_power_of_two()) {
        bail!("--pot needs a power-of-two --max-size, got {max_w}x{max_h}");
    }
    if cli.square && max_w != max_h {
        bail!("--square needs a square --max-size, got {max_w}x{max_h}");
    }
    if cli.inputs.is_empty() {
        bail!("no inputs given; pass sprite files or directories");
    }

    let sources = collect_sources(&cli.inputs, cli.strip_ext)?;
    let settings = settings_line(cli, format, max_w, max_h);

    // One read per file feeds both the fingerprint and the decoder.
    let mut fp = Fnv1a64::new();
    fp.write(settings.as_bytes());
    let mut raw: Vec<(String, Vec<u8>)> = Vec::with_capacity(sources.len());
    for (name, path) in &sources {
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        add_source(&mut fp, name, &bytes);
        raw.push((name.clone(), bytes));
    }
    let current = hex(fp.finish());

    let out_dir = cli
        .output
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let stem = cli
        .output
        .file_name()
        .and_then(|s| s.to_str())
        .context("--output needs a file-name component")?
        .to_string();

    if cli.check {
        return check_mode(cli, format, &out_dir, &stem, &current);
    }

    // Decode, trim, dedup.
    let mut sprites: Vec<(String, RgbaImage, SpriteGeom)> = Vec::with_capacity(raw.len());
    let mut warnings: Vec<String> = Vec::new();
    for (name, bytes) in &raw {
        let img = ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .with_context(|| format!("reading {name}"))?
            .decode()
            .with_context(|| format!("decoding {name}"))?
            .to_rgba8();
        let (sw, sh) = img.dimensions();
        let (cropped, geom) = if cli.trim {
            let b = alpha_bbox(&img);
            if b.empty {
                warnings.push(format!(
                    "{name}: fully transparent; kept as a 1x1 placeholder"
                ));
            }
            (
                image::imageops::crop_imm(&img, b.x, b.y, b.w, b.h).to_image(),
                SpriteGeom {
                    w: b.w,
                    h: b.h,
                    source_w: sw,
                    source_h: sh,
                    trim_x: b.x,
                    trim_y: b.y,
                    trimmed: b.w != sw || b.h != sh,
                },
            )
        } else {
            (
                img,
                SpriteGeom {
                    w: sw,
                    h: sh,
                    source_w: sw,
                    source_h: sh,
                    trim_x: 0,
                    trim_y: 0,
                    trimmed: false,
                },
            )
        };
        sprites.push((name.clone(), cropped, geom));
    }

    // Dedup: pixel-identical bitmaps share one rectangle; every name keeps
    // its own manifest entry.
    let mut uniques: Vec<usize> = Vec::new();
    let mut members: Vec<Vec<usize>> = Vec::new();
    if cli.no_dedup {
        uniques = (0..sprites.len()).collect();
        members = (0..sprites.len()).map(|i| vec![i]).collect();
    } else {
        let mut buckets: HashMap<(u32, u32, u64), Vec<usize>> = HashMap::new();
        for (i, (_, img, _)) in sprites.iter().enumerate() {
            let (w, h) = img.dimensions();
            let mut f = Fnv1a64::new();
            f.write(img.as_raw());
            let key = (w, h, f.finish());
            let bucket = buckets.entry(key).or_default();
            let hit = bucket
                .iter()
                .copied()
                .find(|&u| sprites[uniques[u]].1.as_raw() == img.as_raw());
            match hit {
                Some(u) => members[u].push(i),
                None => {
                    bucket.push(uniques.len());
                    uniques.push(i);
                    members.push(vec![i]);
                }
            }
        }
    }
    let duplicates = sprites.len() - uniques.len();

    let opts = PackOptions {
        max_w,
        max_h,
        padding: cli.padding,
        extrude: cli.extrude,
        border: cli.border,
        pot: cli.pot,
        square: cli.square,
    };
    let sizes: Vec<(u32, u32)> = uniques
        .iter()
        .map(|&i| (sprites[i].2.w, sprites[i].2.h))
        .collect();
    let packed = match pack_pages(&sizes, &opts) {
        Ok(p) => p,
        Err(PackError::Oversize {
            index,
            fit_w,
            fit_h,
        }) => {
            let (name, _, g) = &sprites[uniques[index]];
            eprintln!(
                "sheetpack: {name} is {}x{} but one page holds at most {fit_w}x{fit_h} \
                 under these flags (max-size {max_w}x{max_h}, padding {}, extrude {}, border {})",
                g.w, g.h, cli.padding, cli.extrude, cli.border
            );
            return Ok(1);
        }
    };

    // Assemble the manifest view.
    let mut pages: Vec<Page> = Vec::with_capacity(packed.len());
    for (pi, pk) in packed.iter().enumerate() {
        let image = if packed.len() == 1 {
            format!("{stem}.png")
        } else {
            format!("{stem}-{pi}.png")
        };
        let mut frames: Vec<Frame> = Vec::new();
        let mut area = 0u64;
        for pl in &pk.placements {
            let (w, h) = sizes[pl.index];
            area += u64::from(w) * u64::from(h);
            for &si in &members[pl.index] {
                let (name, _, geom) = &sprites[si];
                frames.push(Frame {
                    name: name.clone(),
                    x: pl.x,
                    y: pl.y,
                    w,
                    h,
                    geom: *geom,
                });
            }
        }
        frames.sort_by(|a, b| a.name.cmp(&b.name));
        let occupancy = area as f64 / (f64::from(pk.w) * f64::from(pk.h));
        pages.push(Page {
            image,
            w: pk.w,
            h: pk.h,
            frames,
            occupancy,
        });
    }

    let meta = Meta {
        fingerprint: current.clone(),
        settings: settings.clone(),
        godot_base: cli.godot_base.clone(),
    };
    let manifests = render(format, &stem, &pages, &meta);

    let mut outputs: Vec<String> = Vec::new();
    if !cli.dry_run {
        if !out_dir.as_os_str().is_empty() {
            fs::create_dir_all(&out_dir)
                .with_context(|| format!("creating {}", out_dir.display()))?;
        }
        for (pi, pk) in packed.iter().enumerate() {
            let img = compose_page(pk, &sizes, &uniques, &sprites, cli.extrude);
            let path = out_dir.join(&pages[pi].image);
            img.save(&path)
                .with_context(|| format!("writing {}", path.display()))?;
            outputs.push(pages[pi].image.clone());
        }
        for (name, contents) in &manifests {
            let path = out_dir.join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            fs::write(&path, contents).with_context(|| format!("writing {}", path.display()))?;
            outputs.push(name.clone());
        }
    }

    if cli.json {
        println!(
            "{}",
            pack_report_json(
                cli, &settings, &current, &sprites, duplicates, &pages, &outputs, &warnings
            )
        );
    } else if !cli.quiet {
        print_pack_report(
            cli, &settings, &current, &sprites, duplicates, &pages, &outputs, &warnings,
        );
    }
    Ok(0)
}

// ---------------------------------------------------------------------------
// Input collection
// ---------------------------------------------------------------------------

/// Resolve inputs into `(frame name, path)` pairs, sorted by name.
fn collect_sources(inputs: &[PathBuf], strip_ext: bool) -> Result<Vec<(String, PathBuf)>> {
    let mut named: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for input in inputs {
        if input.is_dir() {
            let mut files = Vec::new();
            walk(input, &mut files)?;
            files.sort();
            for f in files {
                let rel = f.strip_prefix(input).expect("walked under input");
                let name = frame_name(rel, strip_ext);
                named.entry(name).or_default().push(f);
            }
        } else if input.is_file() {
            let name = frame_name(
                Path::new(
                    input
                        .file_name()
                        .with_context(|| format!("{} has no file name", input.display()))?,
                ),
                strip_ext,
            );
            named.entry(name).or_default().push(input.clone());
        } else {
            bail!("{} is neither a file nor a directory", input.display());
        }
    }
    let mut collisions: Vec<String> = Vec::new();
    for (name, paths) in &named {
        if paths.len() > 1 {
            let list: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
            collisions.push(format!("  {name}: {}", list.join(", ")));
        }
    }
    if !collisions.is_empty() {
        bail!(
            "duplicate frame names; rename the files or pass separate --output runs:\n{}",
            collisions.join("\n")
        );
    }
    if named.is_empty() {
        bail!(
            "no sprite images found (looked for {})",
            IMAGE_EXTENSIONS.join(", ")
        );
    }
    Ok(named
        .into_iter()
        .map(|(name, mut paths)| (name, paths.remove(0)))
        .collect())
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            walk(&path, out)?;
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
    Ok(())
}

/// Frame name for a path relative to its input root: forward slashes,
/// extension optionally stripped.
fn frame_name(rel: &Path, strip_ext: bool) -> String {
    let joined: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let mut name = joined.join("/");
    if strip_ext {
        if let Some(dot) = name.rfind('.') {
            if dot > name.rfind('/').map_or(0, |s| s + 1) {
                name.truncate(dot);
            }
        }
    }
    name
}

// ---------------------------------------------------------------------------
// Settings and sizes
// ---------------------------------------------------------------------------

fn parse_max_size(s: &str) -> Result<(u32, u32)> {
    let parse_one = |v: &str| -> Result<u32> {
        let n: u32 = v
            .parse()
            .with_context(|| format!("--max-size: '{v}' is not a number"))?;
        if n == 0 {
            bail!("--max-size cannot be 0");
        }
        Ok(n)
    };
    match s.split_once('x') {
        Some((w, h)) => Ok((parse_one(w)?, parse_one(h)?)),
        None => {
            let n = parse_one(s)?;
            Ok((n, n))
        }
    }
}

fn settings_line(cli: &Cli, format: Format, max_w: u32, max_h: u32) -> String {
    let onoff = |b: bool| if b { "on" } else { "off" };
    format!(
        "format={} max-size={}x{} padding={} extrude={} border={} trim={} pot={} square={} dedup={} strip-ext={}",
        format.id(),
        max_w,
        max_h,
        cli.padding,
        cli.extrude,
        cli.border,
        onoff(cli.trim),
        onoff(cli.pot),
        onoff(cli.square),
        onoff(!cli.no_dedup),
        onoff(cli.strip_ext),
    )
}

// ---------------------------------------------------------------------------
// Page composition
// ---------------------------------------------------------------------------

/// Paint one page: sprites at their placements, extrusion rings around them.
fn compose_page(
    page: &sheetpack::pack::PackedPage,
    sizes: &[(u32, u32)],
    uniques: &[usize],
    sprites: &[(String, RgbaImage, SpriteGeom)],
    extrude: u32,
) -> RgbaImage {
    let mut img = RgbaImage::new(page.w, page.h);
    for pl in &page.placements {
        let s = &sprites[uniques[pl.index]].1;
        let (w, h) = sizes[pl.index];
        image::imageops::replace(&mut img, s, i64::from(pl.x), i64::from(pl.y));
        for ring in 1..=extrude {
            for yy in 0..h {
                img.put_pixel(pl.x - ring, pl.y + yy, *s.get_pixel(0, yy));
                img.put_pixel(pl.x + w - 1 + ring, pl.y + yy, *s.get_pixel(w - 1, yy));
            }
            for xx in 0..w {
                img.put_pixel(pl.x + xx, pl.y - ring, *s.get_pixel(xx, 0));
                img.put_pixel(pl.x + xx, pl.y + h - 1 + ring, *s.get_pixel(xx, h - 1));
            }
        }
        // Corners repeat the corner pixel.
        for dy in 1..=extrude {
            for dx in 1..=extrude {
                img.put_pixel(pl.x - dx, pl.y - dy, *s.get_pixel(0, 0));
                img.put_pixel(pl.x + w - 1 + dx, pl.y - dy, *s.get_pixel(w - 1, 0));
                img.put_pixel(pl.x - dx, pl.y + h - 1 + dy, *s.get_pixel(0, h - 1));
                img.put_pixel(
                    pl.x + w - 1 + dx,
                    pl.y + h - 1 + dy,
                    *s.get_pixel(w - 1, h - 1),
                );
            }
        }
    }
    img
}

// ---------------------------------------------------------------------------
// --check
// ---------------------------------------------------------------------------

fn check_mode(cli: &Cli, format: Format, out_dir: &Path, stem: &str, current: &str) -> Result<u8> {
    if !format.supports_check() {
        bail!(
            "--check is not supported for --format {} (no manifest carries the fingerprint); \
             run a JSON format alongside it",
            format.id()
        );
    }
    let candidates = [
        out_dir.join(format!("{stem}.json")),
        out_dir.join(format!("{stem}-0.json")),
    ];
    let manifest = candidates.iter().find(|p| p.is_file());
    let (status, stored, missing): (&str, Option<String>, Vec<String>) = match manifest {
        None => ("missing", None, Vec::new()),
        Some(path) => {
            let text =
                fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
            let info = scan_manifest(&text);
            let missing: Vec<String> = info
                .images
                .iter()
                .filter(|img| !out_dir.join(img.as_str()).is_file())
                .cloned()
                .collect();
            let status = match (&info.fingerprint, missing.is_empty()) {
                (Some(f), true) if f == current => "current",
                (Some(_), _) => "stale",
                (None, _) => "unreadable",
            };
            (status, info.fingerprint, missing)
        }
    };
    let ok = status == "current";
    if cli.json {
        let mut out = String::from("{\n");
        let _ = writeln!(out, "  \"mode\": \"check\",");
        let _ = writeln!(
            out,
            "  \"manifest\": {},",
            manifest.map_or("null".to_string(), |p| quote(&p.display().to_string()))
        );
        let _ = writeln!(
            out,
            "  \"stored\": {},",
            stored.as_deref().map_or("null".to_string(), quote)
        );
        let _ = writeln!(out, "  \"current\": {},", quote(current));
        let missing_json: Vec<String> = missing.iter().map(|m| quote(m)).collect();
        let _ = writeln!(out, "  \"missing_images\": [{}],", missing_json.join(", "));
        let _ = writeln!(out, "  \"status\": {}", quote(status));
        out.push('}');
        println!("{out}");
    } else if !cli.quiet {
        println!("sheetpack: check");
        match manifest {
            Some(p) => println!("manifest: {}", p.display()),
            None => println!(
                "manifest: not found ({} or {})",
                candidates[0].display(),
                candidates[1].display()
            ),
        }
        println!("stored:  {}", stored.as_deref().unwrap_or("(none)"));
        println!("current: {current}");
        for m in &missing {
            println!("missing image: {m}");
        }
        if ok {
            println!("status: current -- the atlas matches today's sprites");
        } else {
            println!("status: {status} -- regenerate the atlas and commit it");
        }
    }
    Ok(u8::from(!ok))
}

// ---------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn print_pack_report(
    cli: &Cli,
    settings: &str,
    fingerprint: &str,
    sprites: &[(String, RgbaImage, SpriteGeom)],
    duplicates: usize,
    pages: &[Page],
    outputs: &[String],
    warnings: &[String],
) {
    let mode = if cli.dry_run {
        "pack (dry-run)"
    } else {
        "pack"
    };
    println!("sheetpack: {mode}");
    println!(
        "sprites: {} frames, {} unique ({} duplicates share rectangles)",
        sprites.len(),
        sprites.len() - duplicates,
        duplicates
    );
    println!("settings: {settings}");
    println!("fingerprint: {fingerprint}");
    println!();
    println!("  page  image                        size         frames  occupancy");
    for (i, p) in pages.iter().enumerate() {
        println!(
            "  {:>4}  {:<27}  {:>11}  {:>6}  {:>8.1}%",
            i + 1,
            p.image,
            format!("{}x{}", p.w, p.h),
            p.frames.len(),
            p.occupancy * 100.0
        );
    }
    if !warnings.is_empty() {
        println!("\nwarnings:");
        for w in warnings {
            println!("  ! {w}");
        }
    }
    if cli.dry_run {
        println!("\nnothing written (--dry-run)");
    } else {
        println!("\nwrote: {}", outputs.join(", "));
    }
}

#[allow(clippy::too_many_arguments)]
fn pack_report_json(
    cli: &Cli,
    settings: &str,
    fingerprint: &str,
    sprites: &[(String, RgbaImage, SpriteGeom)],
    duplicates: usize,
    pages: &[Page],
    outputs: &[String],
    warnings: &[String],
) -> String {
    let mut out = String::from("{\n");
    let _ = writeln!(out, "  \"mode\": \"pack\",");
    let _ = writeln!(out, "  \"dry_run\": {},", cli.dry_run);
    let _ = writeln!(out, "  \"settings\": {},", quote(settings));
    let _ = writeln!(out, "  \"fingerprint\": {},", quote(fingerprint));
    let _ = writeln!(out, "  \"sprites\": {},", sprites.len());
    let _ = writeln!(out, "  \"unique\": {},", sprites.len() - duplicates);
    let _ = writeln!(out, "  \"duplicates\": {duplicates},");
    out.push_str("  \"pages\": [\n");
    for (i, p) in pages.iter().enumerate() {
        let _ = write!(
            out,
            "    {{ \"image\": {}, \"w\": {}, \"h\": {}, \"frames\": {}, \"occupancy\": {:.4} }}{}",
            quote(&p.image),
            p.w,
            p.h,
            p.frames.len(),
            p.occupancy,
            sep(i, pages.len())
        );
    }
    out.push_str("  ],\n");
    let outs: Vec<String> = outputs.iter().map(|o| quote(o)).collect();
    let _ = writeln!(out, "  \"outputs\": [{}],", outs.join(", "));
    let warns: Vec<String> = warnings.iter().map(|w| quote(w)).collect();
    let _ = writeln!(out, "  \"warnings\": [{}]", warns.join(", "));
    out.push('}');
    out
}

fn print_formats() {
    println!("sheetpack: manifest formats\n");
    println!("  {:<19} {:<6} description", "id", "check");
    for f in Format::all() {
        println!(
            "  {:<19} {:<6} {}",
            f.id(),
            if f.supports_check() { "yes" } else { "no" },
            f.blurb()
        );
    }
}
