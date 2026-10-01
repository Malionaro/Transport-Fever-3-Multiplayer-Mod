//! `tpf3mp-modscan`: says of each Transport Fever 3 mod whether the players
//! of a room may differ in it (personal) or must all run it (shared), and
//! why (docs/MODS.md).
//!
//! ```text
//! tpf3mp-modscan <mod folder or folder of mods>...   # as text
//! tpf3mp-modscan --installed                         # every mod this player has
//! tpf3mp-modscan --json ...                          # one JSON report per line
//! ```

use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::Parser;
use tpf3mp_modscan::{Class, Report, roots, scan};

#[derive(Parser)]
#[command(
    version,
    about = "Sorts Transport Fever 3 mods into personal and shared"
)]
struct Args {
    /// A mod's folder (with its mod.json), or a folder of mods.
    paths: Vec<PathBuf>,
    /// Scan every mod this player has installed: Mod Hub's, the Steam
    /// account's local ones, and the game's own.
    #[arg(long)]
    installed: bool,
    /// The game's install folder, for --installed.
    #[arg(long)]
    game: Option<PathBuf>,
    /// Steam's folder, for --installed (each account's local mods).
    #[arg(long)]
    steam: Vec<PathBuf>,
    /// One JSON report per line instead of text.
    #[arg(long)]
    json: bool,
    /// Print the mods a save lists, and stop.
    #[arg(long)]
    save: Option<PathBuf>,
}

/// The mods under `path`: itself, if it has a mod.json, else each folder in
/// it that has one, and each folder in those (an archive's folder holding
/// the mod's own).
fn mods_in(path: &Path) -> Vec<PathBuf> {
    if path.join("mod.json").is_file() {
        return vec![path.to_path_buf()];
    }
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(path) else {
        return vec![path.to_path_buf()];
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for dir in dirs {
        if dir.join("mod.json").is_file() {
            out.push(dir);
        } else if let Ok(inner) = std::fs::read_dir(&dir) {
            let mut inner: Vec<PathBuf> = inner
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.join("mod.json").is_file())
                .collect();
            inner.sort();
            out.extend(inner);
        }
    }
    out
}

fn main() -> ExitCode {
    let args = Args::parse();
    if let Some(save) = &args.save {
        return match tpf3mp_modscan::save::mods(save) {
            Ok(mods) => {
                for m in mods {
                    println!("{}\t{}\t{}", m.id, m.source, m.name);
                }
                ExitCode::SUCCESS
            }
            Err(why) => {
                eprintln!("{why}");
                ExitCode::FAILURE
            }
        };
    }
    let mut dirs: Vec<PathBuf> = args.paths.iter().flat_map(|p| mods_in(p)).collect();
    if args.installed {
        let found = roots::installed(&roots::default_roots(args.game.as_deref(), &args.steam));
        dirs.extend(found.into_iter().map(|f| f.path));
    }
    if dirs.is_empty() {
        eprintln!("no mods to scan: name a mod's folder, a folder of mods, or --installed");
        return ExitCode::from(2);
    }
    let reports: Vec<Report> = dirs.iter().map(|d| scan(d)).collect();
    for report in &reports {
        if args.json {
            match serde_json::to_string(report) {
                Ok(line) => println!("{line}"),
                Err(error) => eprintln!("{}: {error}", report.id),
            }
        } else {
            println!("{}", report.path.display());
            print!("{report}");
        }
    }
    if !args.json {
        let count = |class: Class| reports.iter().filter(|r| r.class == class).count();
        println!(
            "{} mods: {} personal, {} carried, {} shared",
            reports.len(),
            count(Class::Personal),
            count(Class::Carried),
            count(Class::Shared)
        );
    }
    ExitCode::SUCCESS
}
