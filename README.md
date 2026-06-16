# rustchm

[![CI](https://github.com/yeroo/RustChm/actions/workflows/ci.yml/badge.svg)](https://github.com/yeroo/RustChm/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/yeroo/RustChm)](https://github.com/yeroo/RustChm/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![OpenSSF Scorecard](https://api.securityscorecards.dev/projects/github.com/yeroo/RustChm/badge)](https://scorecard.dev/viewer/?uri=github.com/yeroo/RustChm)

A fast, dependency-free CHM (Microsoft HTML Help) compiler **and reader** in Rust —
a complete port of [FastChm](https://github.com/yeroo/FastChm) (C++). No runtime
dependencies (only a build-time crate that embeds Windows version metadata); the
shipped binary is a single self-contained executable. Builds and runs on Windows
and Linux.

```
rustchm <project.hhp> [-o output.chm]
rustchm --collection <master.hhp>      # build a master + its [MERGE FILES] children
rustchm --list <file.chm>              # list internal files
rustchm --extract <file.chm> <dir>     # extract user files (built-in LZX decoder)
rustchm --version
```

## Features (full parity with FastChm)

- From-scratch **LZX encoder** (lazy matching) and **decoder**.
- ITSF/ITSP container; the core internal files (`#SYSTEM`, `#TOPICS`, `#URLSTR`,
  `#URLTBL`, `#STRINGS`, `::DataSpace/*`).
- Auto-inclusion of files reached via HTML `href`/`src` and sitemap `Local` entries.
- `[WINDOWS]` definitions (`#WINDOWS`), `[ALIAS]`/`[MAP]` context IDs (`#IVB`).
- Binary TOC (`#TOCIDX`) and binary index (`$WWKeywordLinks`); KLinks/ALinks from
  in-topic `<object>` controls (`$WWAssociativeLinks`).
- Full-text search (`$FIftiMain` + `$OBJINST`).
- `[MERGE FILES]`, `[SUBSETS]`, `[INFOTYPES]`, TOC site properties.
- Codepage/Unicode handling (codepage + LCID from `Language`/`Charset`; UTF-8 /
  UTF-16 / codepage source decoded and metadata re-encoded into the project codepage).
- `--collection` to build a master plus every child CHM it references.

## Verification

rustchm's output is cross-validated three ways: by its own reader, by FastChm's
independent reader, and by Windows' `hh.exe -decompile` — all byte-identical. It
also passes FastChm's full 49-check test suite unchanged.

## Building & testing

```
cargo build --release
python test/run_tests.py
```

The harness compiles every project under `test/`, asserts the expected internal
files, and round-trips through rustchm's own `--extract` (no OS dependency).

## Why a Rust port?

The C++ FastChm binary is sometimes flagged by Microsoft Defender as a false
positive (a common fate for small, unsigned native tools). This Rust rewrite is a
clean-room re-implementation of the same format logic. (Note: a language change does
not by itself prevent heuristic false positives — code signing and submitting the
binary to Microsoft are the durable fixes.)
