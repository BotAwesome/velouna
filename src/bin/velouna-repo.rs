//! Builds a Velouna plugin repo (see velouna-plugins/docs/repos.md).
//!
//!     velouna-repo keygen <key-file>
//!     velouna-repo build <plugins-folder> <repo-folder> --key <key-file>
//!                  [--name <name>] [--maintainer <who>] [--homepage <url>]
//!     velouna-repo verify <repo-folder>
//!
//! The key file holds the secret signing key. Keep it out of the repo and
//! back it up: a repo whose key changes is refused by every Velouna that
//! added it.

use std::path::PathBuf;
use velouna::plugins::repo;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("keygen") => {
            let file = PathBuf::from(args.get(1).ok_or("keygen <key-file>")?);
            if file.exists() {
                return Err(format!("{} exists already; not overwriting a key", file.display()));
            }
            if let Some(d) = file.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            let (secret, public) = repo::keygen()?;
            std::fs::write(&file, format!("{secret}\n")).map_err(|e| e.to_string())?;
            println!("key written to {}", file.display());
            println!("public key: {public}");
            println!("fingerprint: {}", repo::fingerprint(&public));
            Ok(())
        }
        Some("build") => {
            let src = PathBuf::from(args.get(1).ok_or("build <plugins-folder> <repo-folder> --key <key-file>")?);
            let out = PathBuf::from(args.get(2).ok_or("build <plugins-folder> <repo-folder> --key <key-file>")?);
            let key = PathBuf::from(flag(args, "--key").ok_or("--key <key-file> is missing")?);
            let sk = repo::load_key(&key)?;
            // name etc. stay as they are in an existing repo.json unless given
            let old: Option<repo::Index> = std::fs::read(out.join("repo.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
            let pick = |f: &str, o: Option<&String>| flag(args, f).or_else(|| o.cloned()).unwrap_or_default();
            let info = repo::RepoInfo {
                name: pick("--name", old.as_ref().map(|o| &o.name)),
                maintainer: pick("--maintainer", old.as_ref().map(|o| &o.maintainer)),
                homepage: pick("--homepage", old.as_ref().map(|o| &o.homepage)),
            };
            if info.name.is_empty() {
                return Err("--name is missing".into());
            }
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            for line in repo::build(&src, &out, &sk, &info)? {
                println!("  {line}");
            }
            println!("repo.json signed with {}", repo::fingerprint(&repo::public_key(&sk)));
            Ok(())
        }
        Some("verify") => {
            let out = PathBuf::from(args.get(1).ok_or("verify <repo-folder>")?);
            let json = std::fs::read(out.join("repo.json")).map_err(|e| e.to_string())?;
            let sig = std::fs::read(out.join("repo.json.sig")).map_err(|e| e.to_string())?;
            let idx = repo::check(&json, &sig, None)?;
            for e in &idx.plugins {
                for v in &e.versions {
                    let f = out.join(&v.url);
                    let b = std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
                    use sha2::Digest;
                    let sum: String = sha2::Sha256::digest(&b).iter().map(|x| format!("{x:02x}")).collect();
                    if b.len() as u64 != v.size || sum != v.sha256 {
                        return Err(format!("{} does not match repo.json", f.display()));
                    }
                }
            }
            println!("ok: {} ({} plugins), key {}", idx.name, idx.plugins.len(), repo::fingerprint(&idx.key));
            Ok(())
        }
        _ => Err("usage: velouna-repo keygen <key-file> | build <plugins> <repo> --key <key-file> [--name …] | verify <repo>".into()),
    }
}
