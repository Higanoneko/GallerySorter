# Gallery Sorter 相册整理工具

**[English](README.md)** | 中文文档

Gallery Sorter 是一个结合 CLI 与 TUI 的照片/视频整理工具，基于创建时间自动归档。程序会依次尝试 EXIF、视频元数据（FFprobe）、文件名和文件系统时间，并将文件整理为清晰的目录结构。

## 功能亮点

- 多来源时间提取（EXIF → FFprobe → 文件名 → 文件系统时间）
- 使用 xxHash (xxh3) 的高速去重
- 灵活的分类方式：无分类/按年/按年月，月份支持嵌套或组合格式
- 处理模式：增量（默认）、补充、完整
- 并行处理、可配置线程数与试运行模式
- 目录排除（`exclude_dirs`），TUI 向导与 CLI 共用
- 文件名统一化：基于 EXIF / FFprobe 元数据，把文件名统一为标准相机命名
  （`MVIMG_`/`IMG_`/`VID_` + `YYYYMMDD_HHMMSS`），并随归档流程一起执行；
  无元数据的文件保持原名并输出未修改列表
- Ratatui 交互向导 + 完整 CLI 自动化
- 中英文双语界面

## 安装

### 从 Releases 下载

从 [GitHub Releases](https://github.com/PianCat/GallerySorter/releases) 下载最新二进制文件。

### 可选：安装 FFprobe（视频元数据）

如需提取视频元数据，请安装 FFprobe（FFmpeg 自带）：

- Windows：从 https://ffmpeg.org/download.html 下载并加入 PATH
- macOS：`brew install ffmpeg`
- Linux：`apt install ffmpeg` 或对应包管理器命令

### 从源码编译

需要 Rust 2024 版本。

```bash
git clone https://github.com/PianCat/GallerySorter.git
cd GallerySorter/GallerySorter_RS
cargo build --release
```

可执行文件位于 `target/release/gallery-sorter`（Windows 为 `gallery-sorter.exe`）。

## 使用方法

### TUI 模式

不带参数运行即可启动 Ratatui 向导：

```bash
gallery-sorter
```

即使携带其他参数，也可以用 `-i` / `--interactive` 强制进入 TUI 模式：

```bash
gallery-sorter -i
```

### CLI 模式

```bash
# 基本用法
gallery-sorter -I /path/to/photos -o /path/to/sorted

# 使用配置文件（解析自 Config/Name.toml）
gallery-sorter -C Name

# 完整示例
gallery-sorter \
  -I /path/to/photos \
  -I /path/to/more/photos \
  -o /path/to/sorted \
  -M incremental \
  --classify year-month \
  --month-format nested \
  --operation copy \
  --dry-run

# 文件名统一化（重命名 + 移动/复制到输出目录）
gallery-sorter --unify-filenames --operation move -I /path/to/photos -o /path/to/sorted

# 同时保留标准相机名称（MVIMG_/IMG_/VID_ + 时间戳格式）
gallery-sorter --unify-filenames --preserve-standard-names -I /path/to/photos
```

注意：大写 `-I` 才是输入目录参数；小写 `-i` 表示 `--interactive`，会进入
TUI 模式。

去重默认开启，可用 `--no-deduplicate` 关闭。

不使用配置文件且未指定 `--classify` 时，分类默认是 `year-month`；使用
`-C` 加载配置文件时，分类沿用文件中的值，除非显式传入 `--classify`。

### 命令行参数

| 参数 | 简写 | 说明 |
|------|------|------|
| `--config` | `-C` | 配置文件路径或名称（TOML） |
| `--input` | `-I` | 输入目录（可多次指定） |
| `--output` | `-o` | 输出目录 |
| `--mode` | `-M` | `full`、`supplement`、`incremental` |
| `--classify` | `-c` | `none`、`year`、`year-month` |
| `--month-format` | `-m` | `nested`、`combined` |
| `--classify-by-type` |  | 添加 `Photos/Videos/Raw` 子目录 |
| `--operation` | `-O` | `copy`、`move`、`hardlink`、`symlink`（Windows 上 symlink 需要相应权限） |
| `--no-deduplicate` |  | 禁用去重（默认开启） |
| `--state-file` |  | 增量模式状态文件路径 |
| `--threads` | `-t` | 线程数（0 = 自动） |
| `--large-file-mb` |  | 大文件阈值（MB） |
| `--dry-run` | `-n` | 试运行，仅预览 |
| `--unify-filenames` |  | 基于 EXIF / FFprobe 元数据，在归档时按标准相机命名重命名文件 |
| `--preserve-standard-names` |  | 保留标准相机名称（`MVIMG_`/`IMG_`/`VID_` + `_YYYYMMDD_HHMMSS` 格式）不修改 |
| `--unmodified-list` |  | 未修改文件列表路径（默认 `<output_dir>/unmodified_files.txt`） |
| `--verbose` | `-v` | 详细输出 |
| `--json-log` |  | JSON 日志 |
| `--interactive` | `-i` | 即使携带其他参数也强制进入 TUI 模式 |

**CLI 没有目录排除参数**——排除项通过配置文件中的 `exclude_dirs` 设置
（见[排除目录](#排除目录)）。

CLI 参数只有传入时才覆盖配置值，且多数布尔参数只能*开启*某设置
（`--dry-run`、`--unify-filenames`、`--preserve-standard-names`、
`--classify-by-type`、`--verbose`），只有 `--no-deduplicate` 用于*关闭*
去重。若想把配置文件中的值改成相反方向（例如 `dry_run = true` 或
`unify_filenames = true`），请直接编辑 TOML 文件。

### 排除目录

TUI 向导与 CLI 使用同一套排除机制：配置文件中的 `exclude_dirs`。扫描器
会递归遍历输入目录，并跳过列表中每个目录及其整个子树。

每个条目可以是：

- 绝对路径——只匹配该目录及其下级，例如 `"D:/Photos/.thumbnails"`。
- 文件夹名——匹配任意输入目录下任意层级的同名目录，例如 `".sync"` 会排除
  所有名为 `.sync` 的文件夹。

示例：

```toml
exclude_dirs = [
    ".sync",            # 任意名为 .sync 的文件夹
    ".thumbnails",      # 任意名为 .thumbnails 的文件夹
    "D:/Photos/@eaDir", # 仅这一个具体目录
]
```

**TUI：** 配置向导中有“排除目录”输入项。用 `;` 分隔多个文件夹名或路径
（留空表示不排除），保存配置时会写入 `exclude_dirs`。

**CLI：** 没有 `--exclude` 参数。CLI 模式要排除目录，需在配置文件中填写
`exclude_dirs` 并用 `-C Name` 运行。`-C` 配合其他 CLI 参数时，CLI 参数只覆盖
其他配置项，无法添加或修改排除项。

另一种过滤方式：修改配置文件中的 `image_extensions`、`video_extensions`
和 `raw_extensions`，可以完全控制扫描哪些文件（见下文）。

### 文件名统一化

`--unify-filenames` 会扫描输入目录中的媒体文件，仅从 EXIF（图片）或
FFprobe（视频）解析创建时间，并把输出目录中的文件命名为标准相机格式：
照片与 RAW 使用 `IMG_YYYYMMDD_HHMMSS.ext`，动态照片（Motion Photo）使用
`MVIMG_YYYYMMDD_HHMMSS.ext`，视频使用 `VID_YYYYMMDD_HHMMSS.ext`。配置的
操作（复制/移动/硬链接/符号链接）照常生效，因此文件会先重命名再放入输出
目录。当 EXIF/FFprobe 元数据含有毫秒时，文件名保留毫秒段
（`IMG_YYYYMMDD_HHMMSSfff.ext`）；没有毫秒时，同一秒的重名文件自动追加
`_1`、`_2` 后缀。所有目标名都会在复制/移动/链接之前确定并预占，同名文件
不会互相覆盖。

启用 `--preserve-standard-names` 后，只要文件名符合标准相机命名（以
`MVIMG_`、`IMG_` 或 `VID_` 开头，后接 `_年月日_时分秒`，时间之后无论接
什么都算标准名称）就不会重命名（但仍会移动/复制到输出目录），也不会出现在
未修改列表中。

无法从元数据解析出时间的文件会保持原文件名（仍会按配置移动/复制到输出
目录），其源路径会写入未修改列表文件（默认
`<output_dir>/unmodified_files.txt`，可用 `--unmodified-list` 指定）。
试运行模式（`--dry-run`）不会写入列表文件，也不会执行任何操作。

## 配置文件

配置文件会从可执行文件同级的 `Config/` 目录读取。仓库中的 `Template.toml`
可作为模板，保存为 `Config/Name.toml`。

`-C Name` 按以下顺序解析：直接路径 → `Name.toml` → 可执行文件旁的
`Config/Name.toml`，也支持直接给文件路径。

CLI 参数会覆盖配置文件中对应的键；而 `exclude_dirs` 与支持的扩展名列表
只能通过配置文件设置。

### 配置键

| 键 | 类型 | TOML 中必填 | 默认值 | 说明 |
|----|------|-------------|--------|------|
| `input_dirs` | 路径数组 | 是 | — | 递归扫描的输入目录 |
| `output_dir` | 路径 | 是 | `output` | 输出目录 |
| `exclude_dirs` | 路径数组 | 否 | `[]` | 跳过的目录（绝对路径或文件夹名） |
| `processing_mode` | 字符串 | 是 | `incremental` | `incremental`、`full`、`supplement` |
| `classification` | 字符串 | 是 | `none`（纯 CLI 为 `year-month`） | `none`、`year`、`year-month` |
| `month_format` | 字符串 | 否 | `nested` | `nested`（`YYYY/MM/`）、`combined`（`YYYY-MM/`） |
| `classify_by_type` | 布尔 | 否 | `false` | 添加 `Photos/Videos/Raw` 子目录（RAW 在 `Photos/Raw` 下） |
| `operation` | 字符串 | 是 | `copy` | `copy`、`move`、`hardlink`、`symlink` |
| `deduplicate` | 布尔 | 是 | `true` | 基于哈希的去重 |
| `state_file` | 路径 | 否 | `<output_dir>/.gallery_sorter_state.json` | 增量模式状态文件 |
| `threads` | 整数 | 是 | `0`（自动） | 并行线程数 |
| `large_file_threshold` | 整数（字节） | 是 | `104857600`（100 MiB） | 超过此大小的文件使用采样哈希 |
| `dry_run` | 布尔 | 是 | `false` | 试运行，不实际写入 |
| `unify_filenames` | 布尔 | 否 | `false` | 归档时统一为标准相机命名 |
| `preserve_standard_names` | 布尔 | 否 | `false` | 重命名模式下保留标准相机名称 |
| `unmodified_list_file` | 路径 | 否 | `<output_dir>/unmodified_files.txt` | 自定义未修改文件列表路径 |
| `verbose` | 布尔 | 是 | `false` | 详细输出 |
| `image_extensions` | 字符串数组 | 是 | 见下文 | 支持的图片扩展名 |
| `video_extensions` | 字符串数组 | 是 | 见下文 | 支持的视频扩展名 |
| `raw_extensions` | 字符串数组 | 是 | 见下文 | 支持的 RAW 扩展名 |

默认扩展名列表：

```toml
image_extensions = ["jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "heif", "avif", "tiff", "tif"]
video_extensions = ["mp4", "mov", "avi", "mkv", "wmv", "flv", "m4v", "3gp"]
raw_extensions = ["raw", "arw", "cr2", "cr3", "nef", "orf", "rw2", "dng", "raf", "srw", "pef"]
```

完整示例：

```toml
input_dirs = ["D:/Photos", "D:/Videos"]
output_dir = "D:/Sorted"

# 扫描时排除的目录。
# 绝对路径匹配该目录；纯文件夹名匹配任意层级的同名目录。
exclude_dirs = [".sync", ".thumbnails", "D:/Photos/@eaDir"]

processing_mode = "incremental"
classification = "year-month"
month_format = "nested"
classify_by_type = false
operation = "copy"
deduplicate = true

# 增量模式状态文件路径（默认：<output_dir>/.gallery_sorter_state.json）
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

运行方式：

```bash
gallery-sorter -C Name
```

TUI 向导目前只管理部分配置键：配置名称、输入目录、输出目录、排除目录、
处理模式、分类规则、月份格式、按类型分类、文件操作、去重、试运行、统一
文件名和保留标准名称。高级键（`threads`、`large_file_threshold`、
`state_file`、`unmodified_list_file`、`verbose` 及扩展名列表）不会在向导中
显示；经向导运行或保存的配置会使用这些键的默认值，如需自定义请直接编辑
TOML 文件。

## 输出结构

默认（按年月、嵌套）：

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

启用文件类型分类：

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

## 日志

日志保存在可执行文件同级的 `Log/` 目录：

- TUI：`Log/Interactive_YYYYMMDD_HHMMSS.log`
- CLI（配置文件）：`Log/ConfigName/ConfigName_YYYYMMDD_HHMMSS.log`
- CLI（无配置）：`Log/CLIRun_YYYYMMDD_HHMMSS.log`

## 许可证

GPL-3.0，详见 `LICENSE`。
