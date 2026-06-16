//! rustchm — a fast, dependency-free CHM (HTML Help) compiler in Rust.
//! A port of FastChm (github.com/yeroo/FastChm).

mod builder;
mod bytebuf;
mod chmreader;
mod chmwriter;
mod codepage_tables;
mod fifti;
mod lzx;
mod lzxdecode;
mod objinst_data;
mod sitemap;
mod textenc;

use std::time::Instant;

fn usage() -> i32 {
    println!(
        "rustchm {} — dependency-free CHM compiler/reader\n\
         usage: rustchm <project.hhp> [-o output.chm]\n\
         \x20      rustchm --list <file.chm>\n\
         \x20      rustchm --extract <file.chm> <dir>\n\
         \x20      rustchm --version",
        env!("CARGO_PKG_VERSION")
    );
    2
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut hhp = String::new();
    let mut out = String::new();
    let mut collection = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" if i + 1 < args.len() => {
                out = args[i + 1].clone();
                i += 1;
            }
            "-c" | "--collection" => collection = true,
            "-l" | "--list" if i + 1 < args.len() => {
                std::process::exit(chmreader::chm_list(&args[i + 1]));
            }
            "-x" | "--extract" if i + 2 < args.len() => {
                std::process::exit(chmreader::chm_extract(&args[i + 1], &args[i + 2]));
            }
            "-v" | "--version" => {
                println!("rustchm {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "-h" | "--help" | "/?" => {
                std::process::exit(usage());
            }
            a if hhp.is_empty() && !a.starts_with('-') => hhp = a.to_string(),
            a => {
                eprintln!("rustchm: unexpected argument: {a}");
                std::process::exit(2);
            }
        }
        i += 1;
    }
    if hhp.is_empty() {
        std::process::exit(usage());
    }

    let t0 = Instant::now();

    if collection {
        match builder::compile_collection(&hhp) {
            Ok(members) => {
                let mut failures = 0;
                for m in &members {
                    let tag = if m.is_master { "master" } else if m.reused { "child*" } else { "child " };
                    if m.ok && m.reused {
                        println!("[{tag}] {}: reused prebuilt CHM", m.chm);
                    } else if m.ok {
                        let s = m.stats.as_ref().unwrap();
                        println!("[{tag}] {}: {} files, {} -> {} bytes", m.chm, s.file_count, s.uncompressed, s.compressed);
                    } else {
                        eprintln!("[{tag}] {}: FAILED: {}", if m.hhp.is_empty() { &m.chm } else { &m.hhp }, m.err);
                        failures += 1;
                    }
                }
                println!("collection: {} members, {} failed, {:.1} ms", members.len(), failures, t0.elapsed().as_secs_f64() * 1000.0);
                std::process::exit(if failures > 0 { 1 } else { 0 });
            }
            Err(e) => {
                eprintln!("rustchm: error: {e}");
                std::process::exit(1);
            }
        }
    }

    match builder::compile_project(&hhp, &out) {
        Ok((s, path)) => {
            let pct = if s.uncompressed > 0 {
                100.0 * s.compressed as f64 / s.uncompressed as f64
            } else {
                0.0
            };
            println!(
                "{}: {} files, {} -> {} bytes ({:.1}%), output {} bytes ({:.1} ms)",
                path,
                s.file_count,
                s.uncompressed,
                s.compressed,
                pct,
                s.output,
                t0.elapsed().as_secs_f64() * 1000.0
            );
        }
        Err(e) => {
            eprintln!("rustchm: error: {e}");
            std::process::exit(1);
        }
    }
}
