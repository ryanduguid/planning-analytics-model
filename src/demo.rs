//! The synthetic model and data `pacioliscube demo` reads, carried in the binary.
//!
//! The Python wheel ships the same files as package data. The engine reads a
//! model tree from disk, so the demo writes the bundled files to a new private
//! directory, reads them from there and removes the directory afterwards.

use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/demo_files.rs"));

/// The cell the demo prints: July's budgeted gross margin for CivilCo.
pub const CELL: &str = "PnL:FY2026-27,Budget,Jul,CivilCo,All Cost Centres,Gross Margin,Amount";

/// The bundled tree written out, removed again when dropped.
pub struct Unpacked {
    root: PathBuf,
}

impl Unpacked {
    /// Write every bundled file below a directory that did not exist before.
    pub fn new() -> std::io::Result<Unpacked> {
        let base = std::env::temp_dir();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let root = base.join(format!("pacioliscube-demo-{}-{stamp}", std::process::id()));
        // create_dir, not create_dir_all: an existing directory is refused, so
        // nothing else's files can be read in place of the bundle.
        std::fs::create_dir(&root)?;
        let unpacked = Unpacked { root };
        for (relative, bytes) in FILES {
            let mapped = match relative.split_once('/') {
                Some(("examples", rest)) => format!("data/{rest}"),
                _ => relative.to_string(),
            };
            let path = unpacked.root.join(Path::new(&mapped));
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, bytes)?;
        }
        Ok(unpacked)
    }

    pub fn model(&self) -> PathBuf {
        self.root.join("model")
    }

    pub fn data(&self) -> PathBuf {
        self.root.join("data")
    }
}

impl Drop for Unpacked {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_carries_the_whole_model_and_all_five_inputs() {
        assert!(
            FILES
                .iter()
                .any(|(name, _)| *name == "model/tm1project.json")
        );
        let inputs = FILES
            .iter()
            .filter(|(name, _)| name.starts_with("examples/"))
            .count();
        assert_eq!(inputs, 5);
    }

    #[test]
    fn unpacking_writes_the_tree_and_dropping_removes_it() {
        let unpacked = Unpacked::new().unwrap();
        let root = unpacked.root.clone();
        assert!(unpacked.model().join("tm1project.json").is_file());
        assert!(unpacked.data().join("drivers.csv").is_file());
        drop(unpacked);
        assert!(!root.exists());
    }
}
