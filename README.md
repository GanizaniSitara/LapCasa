# LapCasa public beta source candidate

LapCasa is a local photo manager based on [Lap by julyx10 and contributors](https://github.com/julyx10/lap).
Application code is GPL-3.0-or-later; see LICENSE. Original notices are retained.

This is an UNPUBLISHED source preparation, based on the tested internal build80.
It has not yet produced a production binary and is not cleared for distribution.
The previous public release remains v0.4.1. No new download is available here.

Photo-description search uses English. The separate interface language setting
is unchanged. Model weights are excluded; optional features require reviewing
their individual terms and explicitly downloading the appropriate model pack.
See docs/model-downloads.txt, docs/offline-maps.txt and PRIVACY.md.

Use the pinned public native submodules in .gitmodules. Building requires an
existing Rust/MSVC/CMake toolchain and the Node/pnpm versions supported by the
frontend dependencies. No dependencies or prerequisites are installed by this
snapshot. Existing build instructions must be checked against the final build.

Stored editing keys are deliberately retained for compatibility. The final
candidate still requires previous-release recipe/settings upgrade checks.
See SOURCE_PROVENANCE.json and RELEASE-PREPARATION.txt for remaining checks.
