//! Optional on-disk fixtures for development and integration tests.

use std::path::{Path, PathBuf};

const L16_LRI_CANDIDATES: &[&str] = &[
	concat!(
		env!("CARGO_MANIFEST_DIR"),
		"/../.data-from-camera/from-lumen/L16_00078.lri"
	),
	concat!(
		env!("CARGO_MANIFEST_DIR"),
		"/../.data-from-camera/L16_00078.lri"
	),
];

const L16_LUMEN_JPG_CANDIDATES: &[&str] = &[concat!(
	env!("CARGO_MANIFEST_DIR"),
	"/../.data-from-camera/from-lumen/Light/Export/2026-07-14/L16_00078.jpg"
)];

/// Resolved path to `L16_00078.lri` when present in `.data-from-camera/`.
pub fn l16_00078_path() -> Option<PathBuf> {
	first_existing(L16_LRI_CANDIDATES)
}

/// Raw bytes of `L16_00078.lri` when the fixture file exists.
pub fn l16_00078_bytes() -> Option<Vec<u8>> {
	l16_00078_path().and_then(|p| std::fs::read(p).ok())
}

/// Lumen fused JPG for `L16_00078` when exported locally.
pub fn l16_00078_lumen_jpg_path() -> Option<PathBuf> {
	first_existing(L16_LUMEN_JPG_CANDIDATES)
}

fn first_existing(candidates: &[&str]) -> Option<PathBuf> {
	candidates
		.iter()
		.map(Path::new)
		.find(|p| p.exists())
		.map(|p| p.to_path_buf())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn fixture_helper_returns_none_when_missing() {
		// When no fixture on disk, helpers must not panic.
		let _ = l16_00078_path();
		let _ = l16_00078_bytes();
		let _ = l16_00078_lumen_jpg_path();
	}
}
