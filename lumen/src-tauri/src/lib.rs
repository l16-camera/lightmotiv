use std::collections::HashMap;

use base64::{engine::general_purpose::STANDARD, Engine};
use camino::Utf8PathBuf;
use light::api::{self, DirScan, LriSummary};
use light::fuse::{self, FuseSummary};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

mod state;

use state::AppState;

#[derive(Clone, Serialize)]
struct ExportProgress {
	done: usize,
	total: usize,
	camera: String,
}

#[derive(Clone, Serialize)]
struct FuseProgress {
	stage: String,
	done: usize,
	total: usize,
}

#[derive(Serialize)]
struct FuseResult {
	summary: FuseSummary,
	output_dir: String,
	preview_data_url: String,
	export_paths: Vec<String>,
}

fn png_to_data_url(path: &camino::Utf8Path) -> Result<String, String> {
	let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
	Ok(format!("data:image/png;base64,{}", STANDARD.encode(bytes)))
}

#[tauri::command]
fn inspect_lri(state: State<AppState>, path: String) -> Result<LriSummary, String> {
	state.open(&path)
}

#[tauri::command]
fn scan_directory(path: String) -> Result<DirScan, String> {
	api::scan_directory(&path).map_err(|e| e.to_string())
}

#[tauri::command]
async fn extract_lri(
	app: AppHandle,
	state: State<'_, AppState>,
	input: String,
	output: String,
	jobs: Option<usize>,
	only_mono: Option<bool>,
	mono_previews: Option<bool>,
) -> Result<usize, String> {
	let _summary = state.open(&input)?;
	let only_mono = only_mono.unwrap_or(false);
	let input = Utf8PathBuf::from(input);
	let output = Utf8PathBuf::from(output);
	let handle = state.inner().clone();
	let opts = light::extract::ExtractOptions {
		jobs,
		only_mono,
		mono_previews: mono_previews.unwrap_or(true),
	};

	let app2 = app.clone();
	let n = tauri::async_runtime::spawn_blocking(move || {
		handle.with_session(input.as_str(), |session| {
			light::extract::run_session_with_progress(
				session,
				&output,
				opts,
				move |done, total, camera| {
					let _ = app2.emit(
						"export-progress",
						ExportProgress {
							done,
							total,
							camera: camera.to_string(),
						},
					);
				},
			)
			.map_err(|e| e.to_string())
			.map(|r| r.image_count)
		})
	})
	.await
	.map_err(|e| e.to_string())??;

	Ok(n)
}

#[derive(Serialize)]
struct LibcpResult {
	output_dir: String,
	jpg_path: Option<String>,
	ppm_path: Option<String>,
	depth_jpg_path: Option<String>,
	depth_preview_data_url: Option<String>,
	preview_data_url: Option<String>,
	profile: i32,
	from_cache: bool,
	width: Option<u32>,
	height: Option<u32>,
	fnumber: Option<f32>,
	focus_depth_mm: Option<f32>,
	focus_x: Option<f32>,
	focus_y: Option<f32>,
}

fn parse_dof(
	fnumber: Option<f32>,
	focus_depth_mm: Option<f32>,
	focus_x: Option<f32>,
	focus_y: Option<f32>,
	depth_map: Option<bool>,
) -> Result<light::libcp::DofOpts, String> {
	let focus_xy = match (focus_x, focus_y) {
		(Some(x), Some(y)) => Some((x, y)),
		(None, None) => None,
		_ => return Err("focus_x and focus_y must be set together".into()),
	};
	let dof = light::libcp::DofOpts {
		fnumber,
		focus_depth_mm,
		focus_xy,
		depth_map: depth_map.unwrap_or(false),
	};
	// validate via cache_suffix path — run_with_opts validates too
	Ok(dof)
}

fn cache_jpg_name(stem: &str, profile: i32, dof: &light::libcp::DofOpts) -> String {
	format!("{stem}.libcp.p{profile}{}.jpg", dof.cache_suffix())
}

#[derive(Serialize)]
struct LibcpStatus {
	ok: bool,
	helper: Option<String>,
	libcp: Option<String>,
	lib_dir: Option<String>,
	error: Option<String>,
}

#[tauri::command]
fn libcp_status() -> LibcpStatus {
	match light::libcp::resolve_paths() {
		Ok(p) => LibcpStatus {
			ok: true,
			helper: Some(p.helper.display().to_string()),
			libcp: Some(p.libcp.display().to_string()),
			lib_dir: Some(p.lib_dir.display().to_string()),
			error: None,
		},
		Err(e) => LibcpStatus {
			ok: false,
			helper: None,
			libcp: None,
			lib_dir: None,
			error: Some(e.to_string()),
		},
	}
}

#[derive(Serialize)]
struct SetupState {
	libcp_ok: bool,
	helper_ok: bool,
	libcp: Option<String>,
	helper: Option<String>,
	config_path: String,
	setup_dismissed: bool,
	needs_wizard: bool,
}

#[tauri::command]
fn setup_state() -> SetupState {
	let cfg = light::config::load();
	let fully_ok = light::libcp::resolve_paths().is_ok();
	let (libcp, helper) = match light::libcp::resolve_paths() {
		Ok(p) => (
			Some(p.libcp.display().to_string()),
			Some(p.helper.display().to_string()),
		),
		Err(_) => (None, None),
	};
	// Partial checks for wizard steps
	let libcp_present = libcp.is_some()
		|| std::path::Path::new("/Applications/Lumen.app/Contents/Frameworks/libcp.dylib")
			.is_file()
		|| cfg
			.libcp_dir
			.as_ref()
			.map(|d| std::path::Path::new(d).join("libcp.dylib").is_file())
			.unwrap_or(false);
	let helper_present = helper.is_some()
		|| std::path::Path::new("tools/libcp-export/libcp-export").is_file()
		|| cfg
			.libcp_export
			.as_ref()
			.map(|p| std::path::Path::new(p).is_file())
			.unwrap_or(false);

	SetupState {
		libcp_ok: libcp_present,
		helper_ok: helper_present,
		libcp,
		helper,
		config_path: light::config::config_path().display().to_string(),
		setup_dismissed: cfg.setup_dismissed,
		needs_wizard: !fully_ok && !cfg.setup_dismissed,
	}
}

#[tauri::command]
fn set_libcp_dir(path: String) -> Result<SetupState, String> {
	// accept Frameworks dir or Lumen.app
	let p = std::path::PathBuf::from(&path);
	let dir = if p.join("libcp.dylib").is_file() {
		p
	} else if p
		.join("Contents/Frameworks/libcp.dylib")
		.is_file()
	{
		p.join("Contents/Frameworks")
	} else if p.file_name().and_then(|s| s.to_str()) == Some("libcp.dylib") {
		p.parent()
			.ok_or_else(|| "bad path".to_string())?
			.to_path_buf()
	} else {
		return Err(
			"Need a folder with libcp.dylib (e.g. Lumen.app/Contents/Frameworks)".into(),
		);
	};
	if !dir.join("libcp.dylib").is_file() {
		return Err("libcp.dylib not found in that folder".into());
	}
	light::config::set_libcp_dir(dir.display().to_string()).map_err(|e| e.to_string())?;
	Ok(setup_state())
}

#[tauri::command]
fn set_libcp_export_path(path: String) -> Result<SetupState, String> {
	let p = std::path::PathBuf::from(&path);
	if !p.is_file() {
		return Err("not a file".into());
	}
	// install into Application Support for stability
	let dest_dir = light::config::config_dir();
	std::fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;
	let dest = dest_dir.join("libcp-export");
	std::fs::copy(&p, &dest).map_err(|e| e.to_string())?;
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		let mut perms = std::fs::metadata(&dest).map_err(|e| e.to_string())?.permissions();
		perms.set_mode(0o755);
		std::fs::set_permissions(&dest, perms).map_err(|e| e.to_string())?;
	}
	light::config::set_libcp_export(dest.display().to_string()).map_err(|e| e.to_string())?;
	Ok(setup_state())
}

#[tauri::command]
fn dismiss_setup_wizard() -> Result<SetupState, String> {
	light::config::dismiss_setup().map_err(|e| e.to_string())?;
	Ok(setup_state())
}

#[tauri::command]
async fn pick_libcp_location(app: AppHandle) -> Result<Option<String>, String> {
	// folder picker for Frameworks, or file for dylib
	let path = app.dialog().file().blocking_pick_folder();
	Ok(path.map(|p| p.to_string()))
}

#[tauri::command]
async fn pick_helper_file(app: AppHandle) -> Result<Option<String>, String> {
	let path = app.dialog().file().blocking_pick_file();
	Ok(path.map(|p| p.to_string()))
}

fn jpeg_dims(path: &camino::Utf8Path) -> Option<(u32, u32)> {
	let img = image::open(path.as_std_path()).ok()?;
	Some((img.width(), img.height()))
}

#[tauri::command]
async fn libcp_lri(
	input: String,
	output: Option<String>,
	profile: Option<i32>,
	format: Option<String>,
	use_cache: Option<bool>,
	fnumber: Option<f32>,
	focus_depth_mm: Option<f32>,
	focus_x: Option<f32>,
	focus_y: Option<f32>,
	depth_map: Option<bool>,
) -> Result<LibcpResult, String> {
	let input_path = Utf8PathBuf::from(&input);
	let mut profile = profile.unwrap_or(1);
	let use_cache = use_cache.unwrap_or(true);
	let stem = input_path
		.file_stem()
		.unwrap_or("out")
		.to_string();
	let dof = parse_dof(fnumber, focus_depth_mm, focus_x, focus_y, depth_map)?;
	// DepthEditor = DESKTOP only (same rule as light::libcp / helper)
	if (dof.depth_map || dof.focus_xy.is_some()) && profile < 3 {
		profile = 3;
	}

	// Prefer cache next to the .lri: <dir>/<stem>.libcp.p{N}[_dof].jpg
	let cache_name = cache_jpg_name(&stem, profile, &dof);
	let beside = input_path.parent().map(|p| p.join(&cache_name));

	if use_cache && !dof.depth_map {
		if let Some(ref cached) = beside {
			if cached.is_file() {
				let bytes = std::fs::read(cached.as_std_path()).map_err(|e| e.to_string())?;
				let (w, h) = match jpeg_dims(cached) {
					Some((w, h)) => (Some(w), Some(h)),
					None => (None, None),
				};
				return Ok(LibcpResult {
					output_dir: cached
						.parent()
						.map(|p| p.to_string())
						.unwrap_or_default(),
					jpg_path: Some(cached.to_string()),
					ppm_path: None,
					depth_jpg_path: None,
					depth_preview_data_url: None,
					preview_data_url: Some(format!(
						"data:image/jpeg;base64,{}",
						STANDARD.encode(bytes)
					)),
					profile,
					from_cache: true,
					width: w,
					height: h,
					fnumber: dof.fnumber,
					focus_depth_mm: dof.focus_depth_mm,
					focus_x: dof.focus_xy.map(|p| p.0),
					focus_y: dof.focus_xy.map(|p| p.1),
				});
			}
		}
	}

	let output_path = match output {
		Some(dir) => Utf8PathBuf::from(dir),
		None => {
			// write beside LRI when possible, else temp
			if let Some(parent) = input_path.parent() {
				parent.to_path_buf()
			} else {
				std::env::temp_dir()
					.join(format!("luminat-libcp-{stem}"))
					.try_into()
					.map_err(|_| "temp output path".to_string())?
			}
		}
	};
	let format = light::libcp::OutputFormat::parse(format.as_deref().unwrap_or("jpg"))
		.map_err(|e| e.to_string())?;

	tauri::async_runtime::spawn_blocking(move || {
		let out = light::libcp::run_with_opts(&input_path, &output_path, profile, format, &dof)
			.map_err(|e| e.to_string())?;

		// light writes <stem>.libcp.jpg — copy to profile+dof tagged cache name
		let plain = output_path.join(format!("{stem}.libcp.jpg"));
		let tagged = output_path.join(cache_jpg_name(&stem, profile, &dof));
		if plain.is_file() && plain != tagged {
			let _ = std::fs::copy(plain.as_std_path(), tagged.as_std_path());
		}
		let jpg = if tagged.is_file() {
			tagged
		} else if let Some(j) = out.jpg {
			j
		} else {
			plain
		};
		let ppm = output_path.join(format!("{stem}.libcp.ppm"));
		let jpg_path = jpg.is_file().then(|| jpg.to_string());
		let ppm_path = ppm.is_file().then(|| ppm.to_string());
		let (w, h) = if jpg.is_file() {
			match jpeg_dims(&jpg) {
				Some((w, h)) => (Some(w), Some(h)),
				None => (None, None),
			}
		} else {
			(None, None)
		};

		let preview_data_url = if jpg.is_file() {
			let bytes = std::fs::read(jpg.as_std_path()).map_err(|e| e.to_string())?;
			Some(format!(
				"data:image/jpeg;base64,{}",
				STANDARD.encode(bytes)
			))
		} else {
			None
		};

		let depth_jpg = output_path.join(format!("{stem}.depth.jpg"));
		let (depth_jpg_path, depth_preview_data_url) = if depth_jpg.is_file() {
			let bytes = std::fs::read(depth_jpg.as_std_path()).map_err(|e| e.to_string())?;
			(
				Some(depth_jpg.to_string()),
				Some(format!(
					"data:image/jpeg;base64,{}",
					STANDARD.encode(bytes)
				)),
			)
		} else {
			(None, None)
		};

		Ok(LibcpResult {
			output_dir: output_path.to_string(),
			jpg_path,
			ppm_path,
			depth_jpg_path,
			depth_preview_data_url,
			preview_data_url,
			profile,
			from_cache: false,
			width: w,
			height: h,
			fnumber: dof.fnumber,
			focus_depth_mm: dof.focus_depth_mm,
			focus_x: dof.focus_xy.map(|p| p.0),
			focus_y: dof.focus_xy.map(|p| p.1),
		})
	})
	.await
	.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn export_jpeg_copy(source: String, dest_dir: String) -> Result<String, String> {
	let src = Utf8PathBuf::from(&source);
	if !src.is_file() {
		return Err(format!("source missing: {source}"));
	}
	let dest_dir = Utf8PathBuf::from(dest_dir);
	std::fs::create_dir_all(dest_dir.as_std_path()).map_err(|e| e.to_string())?;
	let name = src.file_name().unwrap_or("export.jpg");
	let dest = dest_dir.join(name);
	std::fs::copy(src.as_std_path(), dest.as_std_path()).map_err(|e| e.to_string())?;
	Ok(dest.to_string())
}

#[tauri::command]
fn reveal_path(path: String) -> Result<(), String> {
	#[cfg(target_os = "macos")]
	{
		std::process::Command::new("open")
			.arg(&path)
			.status()
			.map_err(|e| e.to_string())?;
	}
	#[cfg(target_os = "windows")]
	{
		std::process::Command::new("explorer")
			.arg(&path)
			.status()
			.map_err(|e| e.to_string())?;
	}
	#[cfg(all(unix, not(target_os = "macos")))]
	{
		std::process::Command::new("xdg-open")
			.arg(&path)
			.status()
			.map_err(|e| e.to_string())?;
	}
	Ok(())
}

#[tauri::command]
async fn fuse_lri(
	app: AppHandle,
	input: String,
	output: Option<String>,
	max_side: Option<u32>,
	full_res: bool,
	export_tiff: bool,
	export_dng: bool,
	lumen_jpg: Option<String>,
) -> Result<FuseResult, String> {
	let input_path = Utf8PathBuf::from(&input);
	let output_path = match output {
		Some(dir) => Utf8PathBuf::from(dir),
		None => {
			let stem = input_path
				.file_stem()
				.unwrap_or("fuse");
			std::env::temp_dir()
				.join(format!("luminat-fuse-{stem}"))
				.try_into()
				.map_err(|_| "temp output path".to_string())?
		}
	};

	let lumen = lumen_jpg.map(Utf8PathBuf::from);
	let max_side = max_side.unwrap_or(1024);
	let app2 = app.clone();

	tauri::async_runtime::spawn_blocking(move || {
		let summary = fuse::run_with_progress(
			&input_path,
			&output_path,
			lumen.as_deref(),
			max_side,
			full_res,
			export_tiff,
			export_dng,
			1500.0,
			8000.0,
			25,
			move |stage, done, total| {
				let _ = app2.emit(
					"fuse-progress",
					FuseProgress {
						stage: stage.to_string(),
						done,
						total,
					},
				);
			},
		)
		.map_err(|e| e.to_string())?;

		let preview_file = if full_res {
			output_path.join("fused_cropped.png")
		} else {
			output_path.join("fused.png")
		};
		let preview_data_url = png_to_data_url(&preview_file)?;
		let export_paths: Vec<String> = summary
			.exports
			.iter()
			.map(|name| output_path.join(name).to_string())
			.collect();

		Ok(FuseResult {
			summary,
			output_dir: output_path.to_string(),
			preview_data_url,
			export_paths,
		})
	})
	.await
	.map_err(|e| e.to_string())?
}

#[tauri::command]
fn camera_thumbnails_batch(
	state: State<AppState>,
	path: String,
	cameras: Vec<String>,
	jobs: Option<usize>,
) -> Result<HashMap<String, String>, String> {
	state.with_session(&path, |session| {
		session
			.with_lri(|lri| {
				let ids: Vec<_> = cameras
					.iter()
					.filter_map(|c| light::thumbnail::parse_camera_id(c))
					.collect();
				light::thumbnail::thumbnails_batch(lri, &ids, jobs)
			})
			.map_err(|e| e.to_string())
	})
}

#[tauri::command]
async fn pick_lri_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
	let path = app
		.dialog()
		.file()
		.add_filter("Light RAW", &["lri"])
		.blocking_pick_file();
	Ok(path.map(|p| p.to_string()))
}

#[tauri::command]
async fn pick_directory(app: tauri::AppHandle) -> Result<Option<String>, String> {
	let path = app.dialog().file().blocking_pick_folder();
	Ok(path.map(|p| p.to_string()))
}

#[tauri::command]
async fn pick_output_dir(app: tauri::AppHandle) -> Result<Option<String>, String> {
	let path = app.dialog().file().blocking_pick_folder();
	Ok(path.map(|p| p.to_string()))
}

#[tauri::command]
async fn pick_lumen_jpg(app: tauri::AppHandle) -> Result<Option<String>, String> {
	let path = app
		.dialog()
		.file()
		.add_filter("JPEG", &["jpg", "jpeg"])
		.blocking_pick_file();
	Ok(path.map(|p| p.to_string()))
}

// --- M2: camera (adb) + batch libcp ---

#[derive(Serialize)]
struct PullResult {
	local_path: String,
	name: String,
	from_cache: bool,
}

#[derive(Clone, Serialize)]
struct BatchProgress {
	index: usize,
	total: usize,
	file: String,
	phase: String,
	ok: bool,
	message: String,
}

#[derive(Serialize)]
struct BatchItemResult {
	input: String,
	ok: bool,
	jpg_path: Option<String>,
	error: Option<String>,
	from_cache: bool,
}

#[derive(Serialize)]
struct BatchLibcpResult {
	results: Vec<BatchItemResult>,
	ok_count: usize,
	fail_count: usize,
}

#[tauri::command]
fn camera_status() -> light::camera::CameraStatus {
	light::camera::status()
}

#[tauri::command]
fn list_camera_lri(serial: Option<String>) -> Result<Vec<light::camera::RemoteLri>, String> {
	light::camera::list_lri(serial.as_deref())
}

/// Companion camera JPEG → small data-URL thumb (cached on disk).
#[tauri::command]
async fn camera_lri_preview(
	name: String,
	serial: Option<String>,
	max_side: Option<u32>,
) -> Result<String, String> {
	let max_side = max_side.unwrap_or(160);
	tauri::async_runtime::spawn_blocking(move || {
		light::camera::preview_thumb_data_url(serial.as_deref(), &name, max_side)
	})
	.await
	.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn pull_camera_lri(
	app: AppHandle,
	remote_path: String,
	name: String,
	size: Option<u64>,
	serial: Option<String>,
	dest_dir: Option<String>,
) -> Result<PullResult, String> {
	let cache = dest_dir
		.map(std::path::PathBuf::from)
		.unwrap_or_else(light::camera::default_cache_dir);
	let app2 = app.clone();
	let name2 = name.clone();

	tauri::async_runtime::spawn_blocking(move || {
		let (local, from_cache) = light::camera::pull_lri(
			serial.as_deref(),
			&remote_path,
			&cache,
			size,
			|note| {
				let _ = app2.emit(
					"camera-pull-progress",
					BatchProgress {
						index: 0,
						total: 1,
						file: name2.clone(),
						phase: "pull".into(),
						ok: true,
						message: note.to_string(),
					},
				);
			},
		)?;
		Ok(PullResult {
			local_path: local.display().to_string(),
			name,
			from_cache,
		})
	})
	.await
	.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn batch_libcp(
	app: AppHandle,
	paths: Vec<String>,
	profile: Option<i32>,
	use_cache: Option<bool>,
) -> Result<BatchLibcpResult, String> {
	let profile = profile.unwrap_or(1);
	let use_cache = use_cache.unwrap_or(true);
	let total = paths.len();
	if total == 0 {
		return Err("no paths".into());
	}

	// Sequential: libcp is heavy under Rosetta
	let mut results = Vec::new();
	let mut ok_count = 0usize;
	let mut fail_count = 0usize;

	for (i, input) in paths.into_iter().enumerate() {
		let name = std::path::Path::new(&input)
			.file_name()
			.and_then(|s| s.to_str())
			.unwrap_or(&input)
			.to_string();
		let _ = app.emit(
			"batch-libcp-progress",
			BatchProgress {
				index: i + 1,
				total,
				file: name.clone(),
				phase: "start".into(),
				ok: true,
				message: format!("[{}/{}] {name}", i + 1, total),
			},
		);

		// Reuse libcp_lri logic via invoke-equivalent call (default DOF for batch)
		let one = libcp_lri(
			input.clone(),
			None,
			Some(profile),
			Some("jpg".into()),
			Some(use_cache),
			None,
			None,
			None,
			None,
			None,
		)
		.await;
		match one {
			Ok(res) => {
				ok_count += 1;
				let _ = app.emit(
					"batch-libcp-progress",
					BatchProgress {
						index: i + 1,
						total,
						file: name,
						phase: "done".into(),
						ok: true,
						message: if res.from_cache {
							"cache".into()
						} else {
							"rendered".into()
						},
					},
				);
				results.push(BatchItemResult {
					input,
					ok: true,
					jpg_path: res.jpg_path,
					error: None,
					from_cache: res.from_cache,
				});
			}
			Err(e) => {
				fail_count += 1;
				let _ = app.emit(
					"batch-libcp-progress",
					BatchProgress {
						index: i + 1,
						total,
						file: name,
						phase: "error".into(),
						ok: false,
						message: e.clone(),
					},
				);
				results.push(BatchItemResult {
					input,
					ok: false,
					jpg_path: None,
					error: Some(e),
					from_cache: false,
				});
			}
		}
	}

	Ok(BatchLibcpResult {
		results,
		ok_count,
		fail_count,
	})
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
	tauri::Builder::default()
		.manage(AppState::new())
		.plugin(tauri_plugin_drag::init())
		.plugin(tauri_plugin_dialog::init())
		.invoke_handler(tauri::generate_handler![
			inspect_lri,
			scan_directory,
			extract_lri,
			fuse_lri,
			libcp_lri,
			libcp_status,
			setup_state,
			set_libcp_dir,
			set_libcp_export_path,
			dismiss_setup_wizard,
			pick_libcp_location,
			pick_helper_file,
			export_jpeg_copy,
			reveal_path,
			camera_thumbnails_batch,
			pick_lri_file,
			pick_directory,
			pick_output_dir,
			pick_lumen_jpg,
			camera_status,
			list_camera_lri,
			camera_lri_preview,
			pull_camera_lri,
			batch_libcp,
		])
		.run(tauri::generate_context!())
		.expect("error while running Luminat");
}