use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use lri_rs::LriFile;
use memmap2::Mmap;
use ouroboros::self_referencing;

use crate::api::{self, LriSummary};

#[self_referencing]
struct CachedLri {
	bytes: Arc<Mmap>,
	#[borrows(bytes)]
	#[not_covariant]
	file: LriFile<'this>,
}

/// One open LRI: mmap-backed bytes, cached decode.
pub struct LriSession {
	path: PathBuf,
	bytes: Arc<Mmap>,
	cache: Mutex<Option<CachedLri>>,
	summary: Mutex<Option<LriSummary>>,
}

impl LriSession {
	pub fn open(path: impl AsRef<Path>) -> Result<Self> {
		let path = path.as_ref().to_path_buf();
		let file = File::open(&path).with_context(|| format!("open {}", path.display()))?;
		let map = unsafe { Mmap::map(&file) }.context("mmap")?;
		Ok(Self {
			path,
			bytes: Arc::new(map),
			cache: Mutex::new(None),
			summary: Mutex::new(None),
		})
	}

	pub fn path(&self) -> &Path {
		&self.path
	}

	/// Run `f` with a cached [`LriFile`] reference (decode at most once).
	pub fn with_lri<T>(&self, f: impl FnOnce(&LriFile<'_>) -> Result<T>) -> Result<T> {
		let mut guard = self.cache.lock().expect("session cache");
		if guard.is_none() {
			let bytes = Arc::clone(&self.bytes);
			*guard = Some(
				CachedLri::try_new(bytes, |bytes| LriFile::decode(bytes).context("decode LRI"))
					.context("cache LRI")?,
			);
		}
		let cached = guard.as_mut().expect("session cache");
		cached.with_file(|lri| f(lri))
	}

	pub fn summary(&self) -> Result<LriSummary> {
		if let Some(s) = self.summary.lock().expect("summary cache").clone() {
			return Ok(s);
		}

		let summary = self.with_lri(|lri| {
			let name = self
				.path
				.file_stem()
				.map(|s| s.to_string_lossy().into_owned())
				.unwrap_or_default();
			Ok(api::summarize(
				self.path.to_string_lossy().into_owned(),
				name,
				lri,
			))
		})?;
		*self.summary.lock().expect("summary cache") = Some(summary.clone());
		Ok(summary)
	}

	pub fn bytes(&self) -> Arc<Mmap> {
		Arc::clone(&self.bytes)
	}
}
