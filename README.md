# LapCasa

LapCasa is a local photo manager based on [Lap by julyx10 and contributors](https://github.com/julyx10/lap).
Application code is GPL-3.0-or-later; original attribution and notices are retained.

## Windows public beta

[Download LapCasa 0.5.0-beta.1](https://github.com/GanizaniSitara/LapCasa/releases/tag/v0.5.0-beta.1).
Extract the complete Windows x64 ZIP and run lap-casa.exe, keeping the DLLs and
resource folders together. Existing Microsoft WebView2 and Visual C++ runtimes
are required. This is an unsigned portable beta, without an installer or updater.

- Search words, folders and dates; see approximate visual matches separately.
- Find photos by description in English; select the interface language separately.
- Review individual model terms before optional downloads. No model weights ship.
- Use offline/local maps by default, or explicitly choose an online provider.
- Open email in your default app's compose/review flow, without automatic sending.
- Keep non-destructive editing history and browse with the light default theme.

FFmpeg/ffprobe are optional and are **not included** in the public download.
Video processing and FFmpeg-backed formats need separately obtained tools;
see [optional video tools](docs/optional-video-tools.txt).
Picasa import needs an existing suitable Python interpreter.

Back up your library/settings before upgrading. Previous LapCasa profiles retain
their identifier; original upstream Lap profiles remain separate. See
[privacy](PRIVACY.md), [model downloads](docs/model-downloads.txt),
[offline maps](docs/offline-maps.txt) and [third-party notices](THIRD_PARTY_NOTICES.txt).

## Source and verification

The application binary was compiled from a306de3599f0a52384657ebe5c8cdab659269d24.
The release tag adds documentation, attribution and release records; application/build
source files are unchanged. SOURCE_PROVENANCE.json records that distinction.
The release source ZIP includes all four pinned native source trees; ordinary
Git clones need the public submodules in .gitmodules.

The exact application passed the full browsing/People/visual regression gate,
frontend tests and focused editing/storage tests. See
[regression coverage](docs/regression-suite.txt) for executed scope and remaining
manual/hardware acceptance; beta testing does not establish universal compatibility.
Builds require an already provisioned Rust/MSVC/CMake and Node/pnpm environment.
