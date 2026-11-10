# Vendored C headers (build-time only)

Two single-file C libraries, copied byte for byte from upstream and compiled
only into BRYX's build-time cue renderer (`build.rs` builds it with `cc`).
Neither ships in the game binary, and neither is a Cargo dependency.

| File | Upstream | Commit | Licence | Why |
|---|---|---|---|---|
| `dr_flac.h` | https://github.com/mackron/dr_libs | `4ffbded724cc3e3d2838eebca70858bf5984385a` | Public domain (Unlicense) or MIT No Attribution, at the user's choice (see the end of the file) | Decodes the FLAC orchestral samples in `audio/samples/` |
| `qoa.h` | https://github.com/phoboslab/qoa | `93b0680030d1312e7a2829e53272353cc1de6ece` | MIT (`qoa.LICENSE`) | Encodes rendered music to QOA, which `src/audio/qoa.rs` decodes in the game (docs/design/orchestral-score.md, 4.4) |

Do not edit them: update by copying a newer upstream file and its commit here.
