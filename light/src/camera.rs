//! Light L16 over ADB — list / pull `.lri` from `/sdcard/DCIM/Camera/`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;

const REMOTE_DIR: &str = "/sdcard/DCIM/Camera";

#[derive(Debug, Clone, Serialize)]
pub struct CameraDevice {
	pub serial: String,
	pub model: String,
	pub product: String,
	pub online: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CameraStatus {
	pub adb_ok: bool,
	pub adb_path: Option<String>,
	pub devices: Vec<CameraDevice>,
	/// Preferred Light L16 (model L16 / device LFC) if any.
	pub light: Option<CameraDevice>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteLri {
	pub name: String,
	pub remote_path: String,
	pub size: u64,
	pub mtime: Option<String>,
	/// Companion camera JPEG exists (`L16_00026.jpg` next to `.lri`).
	pub has_preview: bool,
	pub preview_remote_path: Option<String>,
}

fn adb_bin() -> Result<PathBuf, String> {
	if let Ok(p) = std::env::var("ADB") {
		let pb = PathBuf::from(p);
		if pb.is_file() {
			return Ok(pb);
		}
	}
	// common locations
	for c in [
		"/opt/homebrew/bin/adb",
		"/usr/local/bin/adb",
		"/Users/igor/Library/Android/sdk/platform-tools/adb",
	] {
		let p = PathBuf::from(c);
		if p.is_file() {
			return Ok(p);
		}
	}
	// PATH
	which("adb").ok_or_else(|| {
		"adb not found — install Android platform-tools or set ADB=/path/to/adb".into()
	})
}

fn which(name: &str) -> Option<PathBuf> {
	let path = std::env::var_os("PATH")?;
	for dir in std::env::split_paths(&path) {
		let p = dir.join(name);
		if p.is_file() {
			return Some(p);
		}
	}
	None
}

fn adb_cmd(serial: Option<&str>) -> Result<Command, String> {
	let bin = adb_bin()?;
	let mut c = Command::new(bin);
	if let Some(s) = serial {
		c.arg("-s").arg(s);
	}
	Ok(c)
}

fn run_adb(serial: Option<&str>, args: &[&str]) -> Result<String, String> {
	let mut c = adb_cmd(serial)?;
	c.args(args);
	let out = c.output().map_err(|e| format!("adb spawn failed: {e}"))?;
	if !out.status.success() {
		let err = String::from_utf8_lossy(&out.stderr);
		let stdout = String::from_utf8_lossy(&out.stdout);
		return Err(format!(
			"adb {} failed: {}{}",
			args.join(" "),
			err.trim(),
			if stdout.trim().is_empty() {
				String::new()
			} else {
				format!(" ({})", stdout.trim())
			}
		));
	}
	Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn status() -> CameraStatus {
	let adb_path = adb_bin().ok().map(|p| p.display().to_string());
	let adb_ok = adb_path.is_some();
	if !adb_ok {
		return CameraStatus {
			adb_ok: false,
			adb_path: None,
			devices: vec![],
			light: None,
		};
	}

	let raw = match run_adb(None, &["devices", "-l"]) {
		Ok(s) => s,
		Err(_) => {
			return CameraStatus {
				adb_ok: true,
				adb_path,
				devices: vec![],
				light: None,
			};
		}
	};

	let mut devices = Vec::new();
	for line in raw.lines().skip(1) {
		let line = line.trim();
		if line.is_empty() {
			continue;
		}
		let mut parts = line.split_whitespace();
		let Some(serial) = parts.next() else { continue };
		let Some(state) = parts.next() else { continue };
		if state != "device" {
			continue;
		}
		let mut model = String::new();
		let mut product = String::new();
		for p in parts {
			if let Some(v) = p.strip_prefix("model:") {
				model = v.replace('_', " ");
			} else if let Some(v) = p.strip_prefix("product:") {
				product = v.to_string();
			}
		}
		devices.push(CameraDevice {
			serial: serial.to_string(),
			model,
			product,
			online: true,
		});
	}

	let light = devices
		.iter()
		.find(|d| {
			d.model.eq_ignore_ascii_case("L16")
				|| d.product.contains("LFC")
				|| d.serial.starts_with("LFCL")
		})
		.cloned()
		.or_else(|| devices.first().cloned());

	CameraStatus {
		adb_ok: true,
		adb_path,
		devices,
		light,
	}
}

fn resolve_serial(serial: Option<&str>) -> Result<Option<String>, String> {
	let st = status();
	if st.light.is_none() && st.devices.is_empty() {
		return Err("no Android device online — plug in the Light L16".into());
	}
	Ok(serial
		.map(|s| s.to_string())
		.or_else(|| st.light.map(|d| d.serial))
		.or_else(|| st.devices.first().map(|d| d.serial.clone())))
}

/// Basenames of `*.jpg` in the camera DCIM folder (e.g. `L16_00026.jpg`).
fn list_remote_jpgs(serial: Option<&str>) -> HashSet<String> {
	let Ok(raw) = run_adb(
		serial,
		&["shell", &format!("ls {REMOTE_DIR}/*.jpg 2>/dev/null")],
	) else {
		return HashSet::new();
	};
	let mut set = HashSet::new();
	for tok in raw.split_whitespace() {
		let name = tok.rsplit('/').next().unwrap_or(tok);
		if name.to_ascii_lowercase().ends_with(".jpg") {
			set.insert(name.to_string());
		}
	}
	set
}

fn stem_of(name: &str) -> &str {
	Path::new(name)
		.file_stem()
		.and_then(|s| s.to_str())
		.unwrap_or(name)
}

pub fn list_lri(serial: Option<&str>) -> Result<Vec<RemoteLri>, String> {
	let serial = resolve_serial(serial)?;
	let jpg_set = list_remote_jpgs(serial.as_deref());

	// ls -l: -rw-rw---- root sdcard_rw SIZE DATE TIME NAME
	let raw = run_adb(
		serial.as_deref(),
		&["shell", &format!("ls -l {REMOTE_DIR}/*.lri 2>/dev/null")],
	)?;

	let mut out = Vec::new();
	for line in raw.lines() {
		let line = line.trim();
		if line.is_empty() || line.starts_with("total ") {
			continue;
		}
		// skip "No such file"
		if line.contains("No such file") {
			continue;
		}
		let parts: Vec<&str> = line.split_whitespace().collect();
		// expect at least: perms owner group size date time name
		if parts.len() < 7 {
			continue;
		}
		let name = parts[parts.len() - 1].to_string();
		if !name.to_ascii_lowercase().ends_with(".lri") {
			continue;
		}
		// size is typically index 3 on Android toolbox ls
		let size = parts
			.iter()
			.find_map(|p| p.parse::<u64>().ok().filter(|&n| n > 1_000_000))
			.unwrap_or(0);
		let mtime = if parts.len() >= 7 {
			Some(format!(
				"{} {}",
				parts[parts.len() - 3],
				parts[parts.len() - 2]
			))
		} else {
			None
		};
		// if name is absolute path, take basename
		let name = name.rsplit('/').next().unwrap_or(&name).to_string();
		let stem = stem_of(&name);
		let jpg_name = format!("{stem}.jpg");
		let has_preview = jpg_set.contains(&jpg_name);
		out.push(RemoteLri {
			remote_path: format!("{REMOTE_DIR}/{name}"),
			name,
			size,
			mtime,
			has_preview,
			preview_remote_path: has_preview.then(|| format!("{REMOTE_DIR}/{jpg_name}")),
		});
	}
	// newest first (zero-padded L16_NNNNN)
	out.sort_by(|a, b| b.name.cmp(&a.name));
	if out.is_empty() {
		return Err(format!("no .lri under {REMOTE_DIR}"));
	}
	Ok(out)
}

/// Pull companion camera JPEG and return a small base64 data-URL for the list UI.
///
/// Cached under `~/Library/Caches/lri-drop/camera/thumbs/<stem>.jpg`.
pub fn preview_thumb_data_url(
	serial: Option<&str>,
	lri_name: &str,
	max_side: u32,
) -> Result<String, String> {
	let serial = resolve_serial(serial)?;
	let stem = stem_of(lri_name);
	let max_side = max_side.clamp(64, 512);

	let cache = default_cache_dir();
	let thumbs = cache.join("thumbs");
	std::fs::create_dir_all(&thumbs).map_err(|e| e.to_string())?;
	let thumb_path = thumbs.join(format!("{stem}.jpg"));

	if thumb_path.is_file() {
		if let Ok(bytes) = std::fs::read(&thumb_path) {
			if !bytes.is_empty() {
				return Ok(format!(
					"data:image/jpeg;base64,{}",
					STANDARD.encode(bytes)
				));
			}
		}
	}

	// Pull full proxy JPEG (a few MB) once, then resize.
	let proxies = cache.join("proxies");
	std::fs::create_dir_all(&proxies).map_err(|e| e.to_string())?;
	let proxy_path = proxies.join(format!("{stem}.jpg"));
	let remote = format!("{REMOTE_DIR}/{stem}.jpg");

	if !proxy_path.is_file() {
		let mut c = adb_cmd(serial.as_deref())?;
		c.arg("pull").arg(&remote).arg(&proxy_path);
		let out = c.output().map_err(|e| format!("adb pull preview: {e}"))?;
		if !out.status.success() {
			let err = String::from_utf8_lossy(&out.stderr);
			let _ = std::fs::remove_file(&proxy_path);
			return Err(format!("no camera JPEG for {stem}: {}", err.trim()));
		}
	}

	let img = image::open(&proxy_path).map_err(|e| format!("open preview: {e}"))?;
	let thumb = img.thumbnail(max_side, max_side);
	thumb
		.save_with_format(&thumb_path, image::ImageFormat::Jpeg)
		.map_err(|e| format!("encode thumb: {e}"))?;
	let encoded = std::fs::read(&thumb_path).map_err(|e| e.to_string())?;

	// Keep proxy for reuse (faster re-open of modal).
	Ok(format!(
		"data:image/jpeg;base64,{}",
		STANDARD.encode(encoded)
	))
}

/// Pull remote `.lri` into `dest_dir`. Returns `(local_path, from_cache)`.
pub fn pull_lri(
	serial: Option<&str>,
	remote_path: &str,
	dest_dir: &Path,
	expected_size: Option<u64>,
	on_note: impl Fn(&str),
) -> Result<(PathBuf, bool), String> {
	std::fs::create_dir_all(dest_dir).map_err(|e| e.to_string())?;
	let name = Path::new(remote_path)
		.file_name()
		.and_then(|s| s.to_str())
		.ok_or_else(|| "bad remote path".to_string())?;
	let local = dest_dir.join(name);

	// skip if already complete (size match)
	if let Ok(meta) = std::fs::metadata(&local) {
		let want = expected_size.or_else(|| {
			list_lri(serial)
				.ok()
				.and_then(|list| list.into_iter().find(|r| r.name == name).map(|r| r.size))
		});
		if let Some(sz) = want {
			if sz > 0 && meta.len() == sz {
				on_note(&format!("cache hit {name}"));
				return Ok((local, true));
			}
		}
	}

	on_note(&format!("adb pull {name}…"));
	let serial = resolve_serial(serial)?;

	// remove partial
	let _ = std::fs::remove_file(&local);

	let mut c = adb_cmd(serial.as_deref())?;
	c.arg("pull").arg(remote_path).arg(&local);
	let out = c.output().map_err(|e| format!("adb pull: {e}"))?;
	if !out.status.success() {
		let err = String::from_utf8_lossy(&out.stderr);
		let _ = std::fs::remove_file(&local);
		return Err(format!("adb pull failed: {}", err.trim()));
	}
	if !local.is_file() {
		return Err("pull finished but file missing".into());
	}
	Ok((local, false))
}

pub fn default_cache_dir() -> PathBuf {
	if let Some(home) = std::env::var_os("HOME") {
		return PathBuf::from(home).join("Library/Caches/lri-drop/camera");
	}
	std::env::temp_dir().join("lri-drop-camera")
}
