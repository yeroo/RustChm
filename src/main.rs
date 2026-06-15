//! rustchm — a fast, dependency-free CHM (HTML Help) compiler in Rust.
//! A port of FastChm (github.com/yeroo/FastChm).

mod builder;
mod bytebuf;
mod chmwriter;
mod lzx;

use std::time::Instant;

fn usage() -> i32 {
    println!(
        "rustchm {} — dependency-free CHM compiler\n\
         usage: rustchm <project.hhp> [-o output.chm]\n\
         \x20      rustchm --version",
        env!("CARGO_PKG_VERSION")
    );
    2
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut hhp = String::new();
    let mut out = String::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" if i + 1 < args.len() => {
                out = args[i + 1].clone();
                i += 1;
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
