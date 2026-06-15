# rustchm

A fast, dependency-free CHM (Microsoft HTML Help) compiler in **Rust** — a port of
[FastChm](https://github.com/yeroo/FastChm) (C++). No external crates; standard
library only.

```
rustchm <project.hhp> [-o output.chm]
rustchm --version
```

## Status

Core compile path is working and verified: it produces a valid `.chm` from an
`.hhp` project (content files + `#SYSTEM`, `#TOPICS`, `#URLSTR`, `#URLTBL`,
`#STRINGS`, and the `::DataSpace` control files), compressed with a from-scratch
**LZX encoder** (64K window, 64K reset intervals).

Output is cross-validated two ways: FastChm's independent CHM reader and Windows'
own `hh.exe -decompile` both extract rustchm's output byte-for-byte.

## Building

```
cargo build --release
```

Produces `target/release/rustchm.exe`.

## Ported from FastChm so far

- `src/lzx.rs` — LZX encoder (lazy matching, verbatim/aligned blocks, canonical
  Huffman with pretree-delta-coded trees).
- `src/chmwriter.rs` — ITSF/ITSP container (PMGL/PMGI directory + quickref, ENCINTs).
- `src/builder.rs` — `.hhp` parsing, internal metadata files, orchestration.

## Not yet ported (tracked)

Auto-inclusion of linked files, sitemap (`.hhc`/`.hhk`) handling, binary TOC/index,
full-text search, window definitions, codepage/Unicode handling, the CHM reader
(`--list`/`--extract`), and `--collection`. These all exist in FastChm and are
being ported incrementally.
