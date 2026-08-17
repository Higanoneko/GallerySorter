# Gallery Sorter

**[中文文档](README_zh.md)** | English

Gallery Sorter is a CLI + TUI tool for organizing photos and videos by creation time. It extracts timestamps from EXIF, video metadata (FFprobe), filenames, and filesystem times, then organizes files into a clean folder structure.

## Highlights

- Multi-source time extraction with automatic fallback (EXIF -> FFprobe -> filename -> mtime)
- Fast deduplication with xxHash (xxh3)
- Flexible classification: none/year/year-month, nested or combined month format
- Processing modes: incremental (default), supplement, full
- Parallel processing with configurable threads and dry-run support
- Directory exclusion (`exclude_dirs`) shared by the TUI wizard and CLI runs
- Filename unification: rename files to standard camera names
  (`MVIMG_`/`IMG_`/`VID_` + `YYYYMMDD_HHMMSS`) from EXIF / FFprobe metadata
  while organizing; files without metadata keep their names and are written
  to an unmodified-files list
- Interactive Ratatui wizard and full CLI automation
- Bilingual UI (English and Simplified Chinese)

## Installation

### Download Release

Download the latest binary from [GitHub Releases](https://github.com/PianCat/GallerySorter/releases).

### Optional: FFprobe for Videos

Install FFprobe (from FFmpeg) to extract video timestamps:

- Windows: download from https://ffmpeg.org/download.html and add to PATH
- macOS: `brew install ffmpeg`
- Linux: `apt install ffmpeg` or equivalent

### Build from Source

Rust 2024 edition is required.

```bash
git clone https://github.com/PianCat/GallerySorter.git
cd GallerySorter/GallerySorter_RS
cargo build --release
```

The binary will be at `target/release/gallery-sorter` (`gallery-sorter.exe` on Windows).

## Usage

### TUI Mode

Run without arguments to launch the Ratatui wizard:

```bash
gallery-sorter
```

`-i` / `--interactive` forces TUI mode even when other arguments are present:

```bash
gallery-sorter -i
```

### CLI Mode

```bash
# Basic usage
gallery-sorter -I /path/to/photos -o /path/to/sorted

# With config file (resolved from Config/Name.toml)
gallery-sorter -C Name

# Full example
gallery-sorter \
  -I /path/to/photos \
  -I /path/to/more/photos \
  -o /path/to/sorted \
  -M incremental \
  --classify year-month \
  --month-format nested \
  --operation copy \
  --dry-run

# Filename unification while organizing (rename + copy/move to output)
gallery-sorter --unify-filenames --operation move -I /path/to/photos -o /path/to/sorted

# Same, but keep standard camera names (MVIMG_/IMG_/VID_ + timestamp pattern)
gallery-sorter --unify-filenames --preserve-standard-names -I /path/to/photos
```

Note that `-I` (capital i) is the input flag; `-i` (lowercase) means
`--interactive` and would launch the TUI instead.

Deduplication is enabled by default; use `--no-deduplicate` to disable it.

When running without a config file and without `--classify`, the default
classification is `year-month`. When a config file is loaded (`-C`), its
classification value is kept unless `--classify` is given.

### Options

| Option | Short | Description |
|--------|-------|-------------|
| `--config` | `-C` | Config file path or name (TOML) |
| `--input` | `-I` | Input directories (repeatable) |
| `--output` | `-o` | Output directory |
| `--mode` | `-M` | `full`, `supplement`, `incremental` |
| `--classify` | `-c` | `none`, `year`, `year-month` |
| `--month-format` | `-m` | `nested`, `combined` |
| `--classify-by-type` |  | Adds `Photos/Videos/Raw` subfolder |
| `--operation` | `-O` | `copy`, `move`, `hardlink`, `symlink` (symlink requires privileges on Windows) |
| `--no-deduplicate` |  | Disable deduplication (enabled by default) |
| `--state-file` |  | State file path for incremental mode |
| `--threads` | `-t` | Thread count (0 = auto) |
| `--large-file-mb` |  | Large-file threshold in MB |
| `--dry-run` | `-n` | Preview without writing |
| `--unify-filenames` |  | Rename files to standard camera names from EXIF / FFprobe metadata while organizing |
| `--preserve-standard-names` |  | Keep standard camera names (`MVIMG_`/`IMG_`/`VID_` + `_YYYYMMDD_HHMMSS` pattern) unchanged |
| `--unmodified-list` |  | Path for the unmodified files list (default `<output_dir>/unmodified_files.txt`) |
| `--verbose` | `-v` | Verbose output |
| `--json-log` |  | JSON formatted logs |
| `--interactive` | `-i` | Force TUI mode even when other arguments are present |

There is **no CLI flag for directory exclusion** — exclusions are configured
with `exclude_dirs` in a config file (see [Excluding Directories](#excluding-directories)).

CLI flags only take effect when you pass them, and most boolean flags can only
turn a setting *on* (`--dry-run`, `--unify-filenames`,
`--preserve-standard-names`, `--classify-by-type`, `--verbose`);
`--no-deduplicate` is the exception and turns deduplication *off*. To switch a
config-file value in the opposite direction (for example `dry_run = true` or
`unify_filenames = true`), edit the TOML file.

### Excluding Directories

Both the TUI wizard and CLI runs use the same exclusion mechanism:
`exclude_dirs` in the config file. The scanner walks the input directories
recursively and skips every directory listed there, including its whole
subtree.

Each entry can be:

- An absolute path — matches that specific directory and everything below it,
  e.g. `"D:/Photos/.thumbnails"`.
- A folder name — matches any directory with that name at any depth under any
  input root, e.g. `".sync"` excludes every folder named `.sync`.

Examples:

```toml
exclude_dirs = [
    ".sync",          # any folder named .sync
    ".thumbnails",    # any folder named .thumbnails
    "D:/Photos/@eaDir", # this specific folder
]
```

**TUI:** the config wizard has an "Exclude Directories" field. Enter folder
names or paths separated by `;` (leave empty to keep nothing excluded). The
value is written to `exclude_dirs` when the configuration is saved.

**CLI:** there is no `--exclude` flag. To exclude directories in CLI mode,
create a config file containing `exclude_dirs` and run with `-C Name`. Passing
CLI flags together with `-C` overrides the other config values but cannot add
or change exclusions.

As an additional filter, you can restrict which files are scanned at all by
editing `image_extensions`, `video_extensions`, and `raw_extensions` in the
config file (see below).

### Filename Unification

`--unify-filenames` scans the input directories and parses the creation time
from EXIF (images) or FFprobe (videos) only, then names the files in the
output directory with standard camera names: `IMG_YYYYMMDD_HHMMSS.ext` for
photos and RAW files, `MVIMG_YYYYMMDD_HHMMSS.ext` for dynamic (Motion Photo)
files, and `VID_YYYYMMDD_HHMMSS.ext` for videos. The configured operation
(copy/move/hardlink/symlink) still applies, so files are both renamed and
placed into the output directory. When the EXIF/FFprobe metadata contains
milliseconds, the millisecond part is kept in the name
(`IMG_YYYYMMDD_HHMMSSfff.ext`); otherwise same-second collisions are resolved
automatically with `_1`, `_2`, ... suffixes. All destination names are decided
before any copy/move/link operation, so files with identical base names can
never overwrite each other.

With `--preserve-standard-names`, files whose names already follow the standard
camera pattern — `MVIMG_`, `IMG_` or `VID_` followed by `_YYYYMMDD_HHMMSS`,
with anything after the timestamp allowed — are not renamed (but are still
moved/copied to the output directory) and are excluded from the unmodified
files list.

Files whose creation time cannot be parsed from metadata keep their filenames
(they are still moved/copied per the configured operation); their source paths
are written to the unmodified files list (default
`<output_dir>/unmodified_files.txt`, overridable with `--unmodified-list`).
Dry-run mode (`--dry-run`) writes neither the list nor performs any operation.

## Configuration

Configuration files are loaded from the `Config/` directory next to the
executable. Use `Template.toml` in the repo as a starting point and save it as
`Config/Name.toml`.

`-C Name` resolves in this order: the path as given, `Name.toml`, then
`Config/Name.toml` next to the executable. A direct file path also works.

CLI flags override config file values for the keys they map to. The config
file is the only way to set `exclude_dirs` and the supported extension lists.

### Config Keys

| Key | Type | Required in TOML | Default | Description |
|-----|------|------------------|---------|-------------|
| `input_dirs` | array of paths | yes | — | Input roots scanned recursively |
| `output_dir` | path | yes | `output` | Destination directory |
| `exclude_dirs` | array of paths | no | `[]` | Directories to skip (absolute path or folder name) |
| `processing_mode` | string | yes | `incremental` | `incremental`, `full`, `supplement` |
| `classification` | string | yes | `none` (CLI without config: `year-month`) | `none`, `year`, `year-month` |
| `month_format` | string | no | `nested` | `nested` (`YYYY/MM/`), `combined` (`YYYY-MM/`) |
| `classify_by_type` | bool | no | `false` | Adds `Photos/Videos/Raw` subfolders (RAW under `Photos/Raw`) |
| `operation` | string | yes | `copy` | `copy`, `move`, `hardlink`, `symlink` |
| `deduplicate` | bool | yes | `true` | Hash-based deduplication |
| `state_file` | path | no | `<output_dir>/.gallery_sorter_state.json` | State file for incremental mode |
| `threads` | integer | yes | `0` (auto) | Parallel thread count |
| `large_file_threshold` | integer (bytes) | yes | `104857600` (100 MiB) | Files above this use sampled hashing |
| `dry_run` | bool | yes | `false` | Preview without writing |
| `unify_filenames` | bool | no | `false` | Rename to standard camera names while organizing |
| `preserve_standard_names` | bool | no | `false` | Keep standard camera names unchanged in rename mode |
| `unmodified_list_file` | path | no | `<output_dir>/unmodified_files.txt` | Custom unmodified-files list path |
| `verbose` | bool | yes | `false` | Detailed output |
| `image_extensions` | array of strings | yes | see below | Supported image extensions |
| `video_extensions` | array of strings | yes | see below | Supported video extensions |
| `raw_extensions` | array of strings | yes | see below | Supported RAW extensions |

Default extension lists:

```toml
image_extensions = ["jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "heif", "avif", "tiff", "tif"]
video_extensions = ["mp4", "mov", "avi", "mkv", "wmv", "flv", "m4v", "3gp"]
raw_extensions = ["raw", "arw", "cr2", "cr3", "nef", "orf", "rw2", "dng", "raf", "srw", "pef"]
```

Complete example:

```toml
input_dirs = ["D:/Photos", "D:/Videos"]
output_dir = "D:/Sorted"

# Directories to exclude from scanning.
# Absolute paths match that folder; plain folder names match at any depth.
exclude_dirs = [".sync", ".thumbnails", "D:/Photos/@eaDir"]

processing_mode = "incremental"
classification = "year-month"
month_format = "nested"
classify_by_type = false
operation = "copy"
deduplicate = true

# State file path for incremental mode (default: <output_dir>/.gallery_sorter_state.json)
# state_file = ".gallery_sorter_state.json"

threads = 0
large_file_threshold = 104857600
dry_run = false

unify_filenames = false
preserve_standard_names = false
# unmodified_list_file = "D:/Reports/unmodified_files.txt"

verbose = false

image_extensions = ["jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "heif", "avif", "tiff", "tif"]
video_extensions = ["mp4", "mov", "avi", "mkv", "wmv", "flv", "m4v", "3gp"]
raw_extensions = ["raw", "arw", "cr2", "cr3", "nef", "orf", "rw2", "dng", "raf", "srw", "pef"]
```

Run with:

```bash
gallery-sorter -C Name
```

The TUI wizard currently manages a subset of these keys: config name, input
directories, output directory, exclude directories, processing mode,
classification, month format, classify-by-type, operation, deduplication,
dry-run, unify filenames, and preserve standard names. Advanced keys
(`threads`, `large_file_threshold`, `state_file`, `unmodified_list_file`,
`verbose`, and the extension lists) are not shown in the wizard; a
configuration run or saved from the wizard uses their default values, so edit
those keys in the TOML file directly.

## Output Structure

Default (year-month, nested):

```
Output/
├── 2024/
│   └── 01/
│       ├── IMG_20240115_143022.jpg
│       └── VID_20240120_183045.mp4
└── 2023/
    └── 12/
        └── photo.heic
```

With file-type classification:

```
Output/
└── 2024/
    └── 01/
        ├── Photos/
        │   ├── IMG_20240115_143022.jpg
        │   └── Raw/
        │       └── DSC_0001.arw
        └── Videos/
            └── VID_20240120_183045.mp4
```

## Logs

Log files are saved in `Log/` next to the executable:

- TUI: `Log/Interactive_YYYYMMDD_HHMMSS.log`
- CLI with config: `Log/ConfigName/ConfigName_YYYYMMDD_HHMMSS.log`
- CLI without config: `Log/CLIRun_YYYYMMDD_HHMMSS.log`

## License

GPL-3.0. See `LICENSE` for details.
