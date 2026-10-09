# LapCasa

A local photo manager for browsing large collections, organizing folders and people, and editing photos on your own computer.

LapCasa is a fork of [Lap by julyx10 and contributors](https://github.com/julyx10/lap). It builds on their work with a folder-focused interface, responsive thumbnail grids, larger adjustable tiles, and additional editing tools.

## Download

[Download LapCasa 0.4.1 for Windows x64](https://github.com/GanizaniSitara/LapCasa/releases/tag/v0.4.1).

1. Download the Windows ZIP and extract the whole folder.
2. Run `lap-casa.exe`. Keep the DLLs, `models`, `ffmpeg`, and `picasa-import` folders beside it.
3. Choose the photo folders you want to browse. Your photos stay in their existing locations.

This release is a portable Windows build, with no installer or automatic updater. It requires Microsoft WebView2 and the Visual C++ runtime. The download is unsigned. The accompanying SHA-256 file identifies the published archive.

No personal photos, library databases, caches, or account settings are included. On a new computer, LapCasa creates its own local settings and library. Existing LapCasa settings are reused.

## In 0.4.1

- Faster grid refills after approving faces: avoid repeatedly copying every person's selection counters.
- Thumbnail decoding and cache writes run outside the async workers used for navigation.
- Cached thumbnails have their own workers, and newly requested previews take priority over the scrolling backlog.
- Faces and photos use the same longest edge at a given slider setting, preserving proportions and column snapping.

On the test library, the post-approval grid pause fell from 1.73 seconds to 0.19 seconds. A one-minute cold scroll stayed responsive, although the final images still needed about 3.7 seconds to finish filling. Large thumbnail caches can also take tens of seconds to reopen at startup; further performance work is ongoing.

### Retained from 0.4.0

- Larger thumbnails at the right end of the size slider, with whole-column snapping.
- Decoded thumbnails appear as they become ready.
- Small cached images fill their selected tiles while retaining their proportions.
- A plain grid scrollbar, without the date stripe.
- A corrected Windows taskbar and Alt-Tab icon.
- EXIF orientation checks across thumbnail, viewer, and editor rendering.

The Windows release passed 15 VM regression checks and a launch check from the packaged folder. Development continues; report reproducible problems through [Issues](https://github.com/GanizaniSitara/LapCasa/issues). Avoid including private photos or library databases in public reports.

Only Windows x64 binaries are provided for this release. Other platform code inherited from Lap is present, but has not been release-tested here.

## Build from source

The app uses Rust and Tauri with a Vue/Vite frontend. On Windows, use an existing Rust toolchain, MSVC C++ build tools, CMake, Node.js 22.12 or newer, and pnpm. The source pins its native libraries as public Git submodules.

```text
git clone --recurse-submodules https://github.com/GanizaniSitara/LapCasa.git
cd LapCasa
pnpm --dir src-vite install --frozen-lockfile
pnpm --dir src-vite build
cd src-tauri
cargo build --release --locked --bin lap-casa --bin lap-cli
```

See `scripts/README.md` for model and FFmpeg resource preparation. Portable packaging places the resource folders beside the executable. Python 3.12 is used by the optional Picasa importer; see `importers/picasa/README.md`.

The public source contains the same application logic as the tested Windows build. Workstation evidence, private development history, and a VM-only relay are excluded; test examples use generic paths. `SOURCE_PROVENANCE.json` maps each release to its build commit.

## License and attribution

LapCasa application code is licensed under GPL-3.0-or-later; see [LICENSE](LICENSE). Original Lap copyright notices and source attribution are retained. Third-party libraries, runtimes, and pretrained model weights have their own terms; see [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt).

See [PRIVACY.md](PRIVACY.md) for local data handling and network use.
