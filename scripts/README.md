# Resource preparation for source builds

This source candidate contains no pretrained model weights. Build and use the
application without models; install optional packs only through Settings after
reviewing their individual terms. Do not copy an old model folder into a public
package. See `docs/model-downloads.txt`.

The retained `download_ffmpeg_sidecar.ps1` and `.sh` helpers obtain the pinned
FFmpeg utilities used for video features. Review their source and hashes before
running them. Downloading a utility does not resolve redistribution requirements:
the release still needs complete matching source/build material and notices.

Build with existing Rust/MSVC/CMake tools, Node.js and pnpm:

```text
git submodule update --init --recursive
pnpm --dir src-vite install --frozen-lockfile
pnpm --dir src-vite build
cd src-tauri
cargo build --release --locked --bin lap-casa
```

These are instructions for a builder who chooses to provision prerequisites;
the source preparation does not install or update anything. Cargo's ONNX Runtime
build dependency may retrieve its native runtime when a validated local cache is
absent. That runtime is separate from photo-analysis model weights.

The optional Picasa importer requires an existing Python 3.11+ with Pillow.
It supports an explicitly selected `LAP_PICASA_PYTHON`; it does not install Python.
See `importers/picasa/README.md`.

The existing public application identifier is retained so upgrades find existing
settings and libraries. Before testing an upgrade, back up the application profile
and library and use independent copies. The final production build, prerequisites,
upgrade/recovery checks and release review are still pending at this source checkpoint.
