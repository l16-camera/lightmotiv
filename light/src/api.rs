use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use lri_rs::{AwbMode, DataFormat, HdrMode, LriFile, MirrorType, SceneMode, SensorModel};
use rayon::prelude::*;
use serde::Serialize;

use crate::mono::{self, MonoInfo};
use crate::session::LriSession;
use crate::threads;

#[derive(Debug, Clone, Serialize)]
pub struct LriSummary {
	pub path: String,
	pub name: String,
	pub firmware: Option<String>,
	pub focal_length: Option<i32>,
	pub integration_ms: Option<u64>,
	pub gain: Option<f32>,
	pub hdr: Option<String>,
	pub scene: Option<String>,
	pub on_tripod: Option<bool>,
	pub af_achieved: Option<bool>,
	pub awb: Option<String>,
	pub awb_gain: Option<[f32; 4]>,
	pub reference_camera: Option<String>,
	pub image_count: usize,
	pub cameras: Vec<CameraSummary>,
	pub fusion: FusionSummary,
	/// Panchromatic modules present in this capture (A2 / C6).
	pub mono: MonoInfo,
}

#[derive(Debug, Clone, Serialize)]
pub struct FusionSummary {
	pub geometry_modules: usize,
	pub modules_with_intrinsics: usize,
	pub movable_mirror_modules: usize,
	pub modules_with_mirror_system: usize,
	pub tof_range_m: Option<f32>,
	pub imu_frames: Option<usize>,
	pub has_gps: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CameraSummary {
	pub id: String,
	pub sensor: String,
	pub format: String,
	pub width: usize,
	pub height: usize,
	pub bayer_jpeg: bool,
	/// True when sensor is AR1335 Mono (no CFA).
	pub is_mono: bool,
	/// Optical design hint: 28 for A2 mono, 150 for C6 mono.
	pub mono_focal_mm: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DirScan {
	pub directory: String,
	pub files: Vec<LriSummary>,
}

pub fn inspect_file(path: impl AsRef<Path>) -> Result<LriSummary> {
	LriSession::open(path)?.summary()
}

pub fn summarize(path: String, name: String, lri: &LriFile<'_>) -> LriSummary {
	summarize_inner(path, name, lri)
}

pub fn scan_directory(path: impl AsRef<Path>) -> Result<DirScan> {
	let path = path.as_ref();
	let jobs = threads::export_jobs(None);
	let pool = rayon::ThreadPoolBuilder::new()
		.num_threads(jobs)
		.build()
		.context("scan thread pool")?;

	let entries: Vec<PathBuf> = std::fs::read_dir(path)
		.with_context(|| format!("read {}", path.display()))?
		.filter_map(|entry| entry.ok())
		.filter(|entry| entry.metadata().map(|m| m.is_file()).unwrap_or(false))
		.map(|entry| entry.path())
		.filter(|p| p.extension().and_then(|e| e.to_str()) == Some("lri"))
		.collect();

	let mut files = pool.install(|| {
		entries
			.par_iter()
			.filter_map(|p| match LriSession::open(p) {
				Ok(session) => match session.summary() {
					Ok(summary) => Some(summary),
					Err(e) => {
						eprintln!("{}: {e}", p.display());
						None
					}
				},
				Err(e) => {
					eprintln!("{}: {e}", p.display());
					None
				}
			})
			.collect::<Vec<_>>()
	});

	files.sort_by(|a, b| a.name.cmp(&b.name));

	Ok(DirScan {
		directory: path.to_string_lossy().into_owned(),
		files,
	})
}

fn summarize_inner(path: String, name: String, lri: &LriFile<'_>) -> LriSummary {
	LriSummary {
		path,
		name,
		firmware: lri.firmware_version.clone(),
		focal_length: lri.focal_length,
		integration_ms: lri.image_integration_time.map(|d| d.as_millis() as u64),
		gain: lri.image_gain,
		hdr: lri.hdr.map(hdr_label),
		scene: lri.scene.map(scene_label),
		on_tripod: lri.on_tripod,
		af_achieved: lri.af_achieved,
		awb: lri.awb.map(awb_label),
		awb_gain: lri.awb_gain.map(|g| [g.r, g.gr, g.gb, g.b]),
		reference_camera: lri.image_reference_camera.map(|c| c.to_string()),
		image_count: lri.image_count(),
		cameras: lri.images().map(camera_summary).collect(),
		fusion: fusion_summary(lri),
		mono: mono::mono_info(lri),
	}
}

fn fusion_summary(lri: &LriFile<'_>) -> FusionSummary {
	let fusion = &lri.fusion;
	FusionSummary {
		geometry_modules: fusion.geometry_module_count(),
		modules_with_intrinsics: fusion.modules_with_intrinsics(),
		movable_mirror_modules: fusion
			.module_geometry
			.iter()
			.filter(|m| m.mirror_type == Some(MirrorType::Movable))
			.count(),
		modules_with_mirror_system: fusion.modules_with_mirror_system(),
		tof_range_m: fusion.tof_range_m,
		imu_frames: fusion.imu.as_ref().map(|i| i.frames),
		has_gps: fusion.gps.is_some(),
	}
}

fn camera_summary(img: &lri_rs::RawImage<'_>) -> CameraSummary {
	let is_mono = mono::is_mono_image(img);
	CameraSummary {
		id: img.camera.to_string(),
		sensor: sensor_label(img.sensor),
		format: img.format.to_string(),
		width: img.width,
		height: img.height,
		bayer_jpeg: matches!(img.format, DataFormat::BayerJpeg),
		is_mono,
		mono_focal_mm: if is_mono {
			mono::mono_focal_mm(img.camera)
		} else {
			None
		},
	}
}

fn sensor_label(sensor: SensorModel) -> String {
	match sensor {
		SensorModel::Ar1335 => "AR1335".into(),
		SensorModel::Ar1335Mono => "AR1335 Mono".into(),
		SensorModel::Unknown => "Unknown".into(),
	}
}

fn hdr_label(mode: HdrMode) -> String {
	match mode {
		HdrMode::None => "none".into(),
		HdrMode::Default => "default".into(),
		HdrMode::Natural => "natural".into(),
		HdrMode::Surreal => "surreal".into(),
	}
}

fn scene_label(mode: SceneMode) -> String {
	match mode {
		SceneMode::Portrait => "portrait".into(),
		SceneMode::Landscape => "landscape".into(),
		SceneMode::Sport => "sport".into(),
		SceneMode::Macro => "macro".into(),
		SceneMode::Night => "night".into(),
		SceneMode::None => "none".into(),
	}
}

fn awb_label(mode: AwbMode) -> String {
	match mode {
		AwbMode::Auto => "auto".into(),
		AwbMode::Daylight => "daylight".into(),
	}
}
