# sheetpack

Pack sprites into texture atlases from the command line, deterministically,
with a `--check` mode that fails CI when the committed atlas no longer matches
the sprites. Offline Rust CLI.

Free atlas tools are mostly GUIs or browser pages: fine for packing once,
useless inside a build pipeline. sheetpack is the other half of the job -- the
part that runs headless on every push. Same sprites, same flags, same bytes
out, so the atlas can live in git and CI can defend it.

```
$ sheetpack sprites/ -o assets/atlas --trim --extrude 1
sheetpack: pack
sprites: 18 frames, 18 unique (0 duplicates share rectangles)
settings: format=json-hash max-size=2048x2048 padding=2 extrude=1 border=0 trim=on pot=off square=off dedup=on strip-ext=off
fingerprint: 221926691fc81ef5

  page  image                        size         frames  occupancy
     1  atlas.png                        114x441      18      55.3%

wrote: atlas.png, atlas.json

$ sheetpack sprites/ -o assets/atlas --trim --extrude 1 --check
sheetpack: check
manifest: assets/atlas.json
stored:  221926691fc81ef5
current: 221926691fc81ef5
status: current -- the atlas matches today's sprites
```

## Features

- MaxRects best-short-side-fit packing (no rotation, by design -- every
  consumer stays simple), with multipage spill when sprites exceed
  `--max-size`.
- Deterministic: inputs sorted by frame name, every packing tie broken by
  position, no timestamps or randomness in any output. Two machines produce
  identical atlases, byte for byte.
- `--check` for CI: each manifest stores an FNV-1a fingerprint of every
  source file and the settings line; `--check` recomputes it and exits 1 when
  sprites were added, removed, edited or the flags changed.
- `--trim` removes transparent margins and records the offsets
  (`spriteSourceSize` / `sourceSize`), so engines restore the original canvas.
- Deduplication on by default: pixel-identical sprites share one rectangle,
  every name keeps its own manifest entry (`--no-dedup` to disable).
- `--extrude` replicates sprite edges outward against texture bleeding;
  `--padding` and `--border` keep sprites apart; `--pot` and `--square`
  shape the page for engines that want it.
- Pages shrink to their content -- five icons do not ship on a 2048 x 2048
  sheet.
- Inputs: PNG, JPEG and WebP files or directories (searched recursively);
  output pages are always PNG (RGBA).

## Install

Prebuilt binaries for Linux and macOS are on the
[releases page](https://github.com/printemps-tokyo/sheetpack/releases), with
SHA-256 checksums.

Or build from source with a stable Rust toolchain:

```
cargo install --git https://github.com/printemps-tokyo/sheetpack
```

## Manifest formats

| `--format` | file(s) | consumed by |
| --- | --- | --- |
| `json-hash` (default) | one JSON per page, `frames` object keyed by name | PixiJS, Phaser 3 (`load.atlas`), and most TexturePacker-JSON readers |
| `json-array` | one JSON per page, `frames` array with `filename` | same readers, array flavour |
| `phaser-multiatlas` | one JSON for all pages, `textures` array | Phaser 3 `load.multiatlas` |
| `godot-tres` | one Godot 4 `AtlasTexture` text resource per sprite | Godot 4 |
| `css` | one stylesheet of `background-position` classes | plain web pages |

Notes, verified against the named sources on 2026-08-19:

- `json-hash` / `json-array` follow the TexturePacker JSON layout
  (`frame`, `rotated`, `trimmed`, `spriteSourceSize`, `sourceSize`, `meta`).
- `phaser-multiatlas` matches what Phaser's `MultiAtlasFile` loader reads:
  a `textures` array whose entries carry `image` and `frames`
  ([source](https://github.com/phaserjs/phaser/blob/master/src/loader/filetypes/MultiAtlasFile.js)).
- `godot-tres` restores trimmed sprites through `AtlasTexture.margin`. Godot
  draws `region.size + margin.size` pixels and offsets the sprite by
  `margin.position`
  ([source](https://github.com/godotengine/godot/blob/master/scene/resources/atlas_texture.cpp)),
  so sheetpack writes `margin = Rect2(trim_x, trim_y, source_w - w,
  source_h - h)`. Image paths are prefixed with `--godot-base`
  (default `res://`); adjust it to where the atlas PNG lives in your project.

Engine snippets:

```js
// Phaser 3
this.load.atlas("sprites", "atlas.png", "atlas.json");        // json-hash
this.load.multiatlas("sprites", "atlas.json", "assets/");     // phaser-multiatlas

// PixiJS
const sheet = await PIXI.Assets.load("atlas.json");
new PIXI.Sprite(sheet.textures["hero/run_0.png"]);
```

## CI recipe

Commit the atlas and let CI refuse drift:

```yaml
- name: Atlas is up to date
  run: sheetpack sprites/ -o assets/atlas --trim --check
```

`--check` compares fingerprints, not pixels: it re-reads the sprite files and
the flags, not the generated PNG. Regenerating with the same command that CI
runs (minus `--check`) and committing the result is always enough to go green.
The flags are part of the fingerprint, so changing `--padding` in CI without
regenerating also fails, as it should.

## Flags

```
sheetpack [INPUT]... [OPTIONS]

  -o, --output <STEM>      output stem: <STEM>.png / <STEM>-N.png + manifest (default: atlas)
      --format <ID>        manifest dialect, see --list-formats (default: json-hash)
      --max-size <N|WxH>   page size ceiling (default: 2048)
      --padding <N>        empty pixels between sprites (default: 2)
      --extrude <N>        edge-replication rings against bleeding (default: 0)
      --border <N>         empty pixels at the page edge (default: 0)
      --trim               trim transparent margins, record offsets
      --pot                round page sizes up to powers of two
      --square             force square pages
      --no-dedup           identical sprites keep separate rectangles
      --strip-ext          drop file extensions from frame names
      --godot-base <P>     image path prefix for godot-tres (default: res://)
      --dry-run            plan and report, write nothing
      --check              verify the committed atlas, exit 1 if stale
      --json               machine-readable report on stdout
  -q, --quiet              print nothing but errors
      --list-formats       list manifest dialects and exit
```

Frame names are the file paths relative to the input directory, with forward
slashes (`hero/run_0.png`). Two inputs producing the same frame name is an
error, not a silent overwrite.

## Exit codes

| code | meaning |
| --- | --- |
| 0 | the run did what it was asked to do |
| 1 | `--check` found the atlas stale or incomplete, or a sprite is larger than one page can hold |
| 2 | usage error, unreadable input, or duplicate frame names |

## Limitations

- No rotation and no polygon packing -- rectangles only. This costs some
  density and keeps every manifest consumer simple.
- Pages are packed against the `--max-size` bin and then shrunk to content,
  which favours tall or wide pages over near-square ones for small sprite
  sets. `--pot --square` when the shape matters.
- `--check` works for the JSON formats, which carry the fingerprint;
  `godot-tres` and `css` outputs have nowhere standard to store it. Run a
  JSON format alongside if you want the CI gate.
- Dedup matches exact RGBA bytes only; visually identical sprites saved with
  different color profiles do not merge.

## Development

```
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

The test suite builds every sprite programmatically and asserts on decoded
atlas pixels, exit codes and manifest text; no fixtures, no network.

## License

MIT
