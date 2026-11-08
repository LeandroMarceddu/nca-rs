# Provenance

## Source

- Repository: `ghs-rs` (grblHAL sender, GPUI desktop application)
- Local path used for the fork: `/home/leandro/Desktop/Dev/Testing/ghs-rs`
- Commit: `4bfbda60553661df677eaab15c323ec428c8690b`
- Fork date: 2026-11-08

The parent repository was treated as read-only reference material. No commits,
tags, pushes, or writes were made there.

## Copied assets

| Parent | This repository | Changes |
|---|---|---|
| `crates/ghs-viewer` | `crates/nc-viewer` | Crate rename and doc-link updates; added `move_kind.rs` with `MoveKind`, `MoveFamily`, `MoveStyle`, `AnnotationKind` |
| `crates/ghs-gcode` | `crates/nc-gcode` | Crate rename; added `motion.rs` with `MotionRecord`, `ArcRecord`, `CannedRecord`, `ModalState`, `LossyProgram`, `SkippedBlock`; added `Program` motion fields and methods; added `ProgramBuilder` under `test-util` |
| `crates/ghs-core/src/{axis,position,units,wcs}.rs` | `crates/nc-core/src/` | Copied as the minimal coordinate domain; controller report models were not copied |
| `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE` | same | Copied verbatim; the parent's MIT/Apache-2.0 licensing is preserved |
| `rustfmt.toml`, `rust-toolchain.toml` | same | Copied; toolchain pinned to Rust 1.98.1 |
| `deny.toml` (policy) | not yet copied | The license/advisory check is part of the Phase 4 CI work |

## New code written for `nca-rs`

- `nc-core`: `math.rs` (libm-backed deterministic math), `tool.rs`
  (`Tool`, `ToolTable`, tool hints), `dialect.rs`, `machine.rs`, `material.rs`,
  `error.rs`, new `lib.rs`.
- `nc-stock`: all modules (stock inference, heightmap, cutter).
- `nc-sim`: all modules (Phase 0 scaffold).
- `nc-analysis`: all modules (types, config, report, rule contract).
- `nc-app`, `nc-cli`, `nc-fixtures`, workspace manifests, docs, fixtures layout.

## Deliberately not copied

Controller and sender code: `ghs-engine`, `ghs-proto`, `ghs-serial`,
`ghs-demo`, `ghs-sim` (controller protocol simulator), `ghs-probe`,
`ghs-config`, `ghs-i18n`, the `ghs-app` shell/session/live/connection/probe/
settings/offsets/tools/utilities modules, and the GPUI application bootstrap.
See `docs/REUSE-MAP.md` for the file-level decisions.

## License obligations

The parent's `NOTICE` is preserved. New files are licensed under the same
`MIT OR Apache-2.0` terms. See `LICENSE-MIT` and `LICENSE-APACHE`.
