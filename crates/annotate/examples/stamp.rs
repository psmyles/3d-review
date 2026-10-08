//! Stamp a sample review comment on every `Model` of an FBX — the files the
//! storage spike opens in other applications.
//!
//!   cargo run -p review-annotate --example stamp -- <in.fbx> <out.fbx> [--hidden]

use review_annotate::fbx::{self, Edit};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output, rest @ ..] = args.as_slice() else {
        return Err("usage: stamp <in.fbx> <out.fbx> [--hidden]".into());
    };
    let hidden = rest.iter().any(|arg| arg == "--hidden");
    let bytes = std::fs::read(input)?;
    let scan = fbx::scan(&bytes, "ReviewComments")?;
    let edits: Vec<Edit> = scan
        .models
        .iter()
        .map(|model| Edit {
            model: model.id,
            value: Some(format!(
                r#"{{"v":1,"threads":[{{"status":"open","messages":[{{"author":"spike","text":"Comment on {} — \"quoted\" ✓"}}]}}]}}"#,
                model.name.replace('"', "'")
            )),
            hidden,
        })
        .collect();
    std::fs::write(output, fbx::patch(&bytes, "ReviewComments", &edits)?)?;
    println!(
        "{} models stamped ({})",
        edits.len(),
        if hidden { "UH" } else { "U" }
    );
    Ok(())
}
