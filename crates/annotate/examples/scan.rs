//! Print the `Model` objects of an FBX and the review-comment property each
//! carries — the storage spike's check on files other applications wrote.
//!
//!   cargo run -p review-annotate --example scan -- <file.fbx>

use review_annotate::fbx;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: scan <file.fbx>")?;
    let bytes = std::fs::read(&path)?;
    let scan = fbx::scan(&bytes, "ReviewComments")?;
    println!(
        "{:?}: {} models, {} carry the property",
        scan.format,
        scan.models.len(),
        scan.strings.len()
    );
    for stored in &scan.strings {
        let model = &scan.models[stored.model];
        println!(
            "  {} [{}] hidden={} {}",
            model.name, model.class, stored.hidden, stored.value
        );
    }
    Ok(())
}
