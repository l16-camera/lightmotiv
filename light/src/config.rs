//! Persistent user config for Luminat / light CLI.
//! macOS: `~/Library/Application Support/Luminat/config.json`

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LuminatConfig {
	/// Directory containing libcp.dylib (+ libceres.dylib).
	pub libcp_dir: Option<String>,
	/// Full path to x86_64 libcp-export helper.
	pub libcp_export: Option<String>,
	/// First-run wizard dismissed (even if libcp still missing).
	pub setup_dismissed: bool,
}

pub fn config_dir() -> PathBuf {
	if let Some(home) = std::env::var_os("HOME") {
		return PathBuf::from(home).join("Library/Application Support/Luminat");
	}
	std::env::temp_dir().join("Luminat")
}

pub fn config_path() -> PathBuf {
	config_dir().join("config.json")
}

pub fn load() -> LuminatConfig {
	let path = config_path();
	let Ok(data) = fs::read_to_string(&path) else {
		return LuminatConfig::default();
	};
	serde_json::from_str(&data).unwrap_or_default()
}

pub fn save(cfg: &LuminatConfig) -> Result<()> {
	let dir = config_dir();
	fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
	let path = config_path();
	let data = serde_json::to_string_pretty(cfg)?;
	fs::write(&path, data).with_context(|| format!("write {}", path.display()))?;
	Ok(())
}

pub fn set_libcp_dir(dir: impl Into<String>) -> Result<LuminatConfig> {
	let mut cfg = load();
	cfg.libcp_dir = Some(dir.into());
	save(&cfg)?;
	Ok(cfg)
}

pub fn set_libcp_export(path: impl Into<String>) -> Result<LuminatConfig> {
	let mut cfg = load();
	cfg.libcp_export = Some(path.into());
	save(&cfg)?;
	Ok(cfg)
}

pub fn dismiss_setup() -> Result<LuminatConfig> {
	let mut cfg = load();
	cfg.setup_dismissed = true;
	save(&cfg)?;
	Ok(cfg)
}
