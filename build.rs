//! Bundles the shipped synthetic model and its example data into the binary, so
//! `pacioliscube demo` works without a source checkout.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn walk(root: &Path, directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).expect("readable bundle directory") {
        let path = entry.expect("readable directory entry").path();
        if path.is_dir() {
            walk(root, &path, files);
        } else {
            files.push(
                path.strip_prefix(root)
                    .expect("inside the crate")
                    .to_owned(),
            );
        }
    }
}

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by Cargo"));
    let mut files = Vec::new();
    for directory in ["model", "examples"] {
        println!("cargo::rerun-if-changed={directory}");
        walk(&root, &root.join(directory), &mut files);
    }
    files.sort();
    let mut table = String::from("pub const FILES: &[(&str, &[u8])] = &[\n");
    for file in &files {
        let relative = file
            .iter()
            .map(|part| part.to_str().expect("UTF-8 bundle path"))
            .collect::<Vec<_>>()
            .join("/");
        let absolute = root.join(file);
        writeln!(table, "    ({relative:?}, include_bytes!({absolute:?})),").unwrap();
    }
    table.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("set by Cargo"));
    std::fs::write(out.join("demo_files.rs"), table).expect("writable OUT_DIR");
}
