# Gallery Sorter

**[中文文档](README_zh.md)** | English

Gallery Sorter is a CLI + TUI tool for organizing photos and videos by creation time. It extracts timestamps from EXIF, video metadata (FFprobe), filenames, and filesystem times, then organizes files into a clean folder structure.

## Highlights

- Multi-source time extraction with automatic fallback (EXIF -> FFprobe -> filename -> mtime)
- Fast deduplication with xxHash (xxh3)
- Flexible classification: none/year/year-month, nested or combined month format
- Processing modes: incremental (default), supplement, full
- Parallel processing with configurable threads and dry-run support
- Filename unification: rename files in place to `YYYYMMDD_HHMMSS.ext` from
  EXIF / FFprobe metadata; files without metadata stay unchanged and are
  written to an unmodified-files list
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

### CLI Mode

```bash
# Basic usage
gallery-sorter -i /path/to/photos -o /path/to/sorted

# With config file (resolved from Config/Name.toml)
gallery-sorter -C Name

# Full example
gallery-sorter \
  -i /path/to/photos \
  -i /path/to/more/photos \
  -o /path/to/sorted \
  -M incremental \
  --classify year-month \
  --month-format nested \
  --operation copy \
  --deduplicate \
  --dry-run

# Filename unification (in-place rename, EXIF / FFprobe metadata only)
gallery-sorter --unify-filenames -i /path/to/photos
```

### Options

| Option | Short | Description |
|--------|-------|-------------|
| `--config` | `-C` | Config file path or name (TOML) |
| `--input` | `-i` | Input directories (repeatable) |
| `--output` | `-o` | Output directory |
| `--mode` | `-M` | `full`, `supplement`, `incremental` |
| `--classify` | `-c` | `none`, `year`, `year-month` |
| `--month-format` | `-m` | `nested`, `combined` |
| `--classify-by-type` |  | Adds `Photos/Videos/Raw` subfolder |
| `--operation` | `-O` | `copy`, `move`, `hardlink`, `symlink` |
| `--no-deduplicate` |  | Disable deduplication |
| `--state-file` |  | State file path for incremental mode |
| `--threads` | `-t` | Thread count (0 = auto) |
| `--large-file-mb` |  | Large-file threshold in MB |
| `--dry-run` | `-n` | Preview without writing |
| `--unify-filenames` |  | Unify filenames from EXIF / FFprobe metadata (in-place rename) |
| `--unmodified-list` |  | Path for the unmodified files list (default `<output_dir>/unmodified_files.txt`) |
| `--verbose` | `-v` | Verbose output |
| `--json-log` |  | JSON formatted logs |

### Filename Unification

`--unify-filenames` scans the input directories and parses the creation time
from EXIF (images) or FFprobe (videos) only, then renames each file in place to
`YYYYMMDD_HHMMSS.ext` (e.g. `IMG_20240115_143022.jpg` →
`20240115_143022.jpg`). Collisions in the same directory get `_1`, `_2`, ...
suffixes automatically.

Files whose creation time cannot be parsed from metadata are left unchanged;
their paths are written to the unmodified files list (default
`<output_dir>/unmodified_files.txt`, overridable with `--unmodified-list`).
Dry-run mode (`--dry-run`) writes neither the list nor any filename change.

## Configuration

Configuration files are loaded from the `Config/` directory next to the executable. Use `Template.toml` in the repo as a starting point and save it as `Config/Name.toml`.

CLI flags always override config file values.

Example:

```toml
input_dirs = ["D:/Photos", "D:/Videos"]
output_dir = "D:/Sorted"
processing_mode = "incremental"
classification = "year-month"
month_format = "nested"
classify_by_type = false
operation = "copy"
deduplicate = true
dry_run = false
unify_filenames = false
verbose = false
```

Run with:

```bash
gallery-sorter -C Name
```

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
