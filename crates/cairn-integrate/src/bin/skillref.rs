//! Print the canonical Skill schema, revision and branch name (D29b).
//!
//! A thin wrapper over `cairn_integrate::revision`, so the release workflow,
//! the released binary and `cairn doctor` cannot disagree about what a
//! revision is. The workflow runs this rather than reimplementing the hash in
//! shell — that is the only mechanism by which CI learns a revision.
//!
//! ```console
//! $ cargo run -q -p cairn-integrate --bin skillref -- --json
//! {"skill_branch":"skill-release/1-…","skill_revision":"…","skill_schema":1}
//! ```

use cairn_integrate::revision;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    let from_disk = args
        .iter()
        .position(|a| a == "--dir")
        .and_then(|i| args.get(i + 1))
        .cloned();

    let files = match from_disk {
        Some(dir) => match revision::files_from_disk(std::path::Path::new(&dir)) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("skillref: {dir}: {e}");
                std::process::exit(1);
            }
        },
        None => revision::embedded_files(),
    };

    let value = revision::skillref_json(&files);
    if json {
        println!("{value}");
    } else {
        println!("skill_schema   {}", value["skill_schema"]);
        println!(
            "skill_revision {}",
            value["skill_revision"].as_str().unwrap_or("")
        );
        println!(
            "skill_branch   {}",
            value["skill_branch"].as_str().unwrap_or("")
        );
    }
}
