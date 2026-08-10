//! Panchromatic (mono) modules A2 / C6 — first-class helpers for extract / UI / gather.
//!
//! Hardware: AR1335 without CFA. Present in .lri only when capture included them
//! (typically 28 mm → A2, 150 mm → C6). See openlight-camera monochrome.md / GROK R2.

use lri_rs::{CameraId, LriFile, RawImage, SensorModel};
use serde::Serialize;

/// Known panchromatic module ids on L16.
pub const MONO_IDS: &[CameraId] = &[CameraId::A2, CameraId::C6];

#[derive(Debug, Clone, Serialize)]
pub struct MonoInfo {
	pub present: bool,
	pub count: usize,
	/// Camera ids that are mono in this file, e.g. ["A2"] or ["A2","C6"].
	pub cameras: Vec<String>,
	pub a2: bool,
	pub c6: bool,
}

/// Focal-length hint for the two panchromatic modules (optical design, not from EXIF).
pub fn mono_focal_mm(id: CameraId) -> Option<u32> {
	match id {
		CameraId::A2 => Some(28),
		CameraId::C6 => Some(150),
		_ => None,
	}
}

pub fn is_mono_sensor(sensor: SensorModel) -> bool {
	matches!(sensor, SensorModel::Ar1335Mono)
}

pub fn is_mono_image(img: &RawImage<'_>) -> bool {
	is_mono_sensor(img.sensor)
}

pub fn is_known_mono_id(id: CameraId) -> bool {
	matches!(id, CameraId::A2 | CameraId::C6)
}

pub fn mono_info(lri: &LriFile<'_>) -> MonoInfo {
	let mono: Vec<&RawImage<'_>> = lri.images().filter(|i| is_mono_image(i)).collect();
	let cameras: Vec<String> = mono.iter().map(|i| i.camera.to_string()).collect();
	let a2 = mono.iter().any(|i| i.camera == CameraId::A2);
	let c6 = mono.iter().any(|i| i.camera == CameraId::C6);
	MonoInfo {
		present: !cameras.is_empty(),
		count: cameras.len(),
		cameras,
		a2,
		c6,
	}
}

/// Human label for DNG UniqueCameraModel / UI.
pub fn dng_camera_label(img: &RawImage<'_>) -> String {
	let id = img.camera.to_string();
	if is_mono_image(img) {
		if let Some(mm) = mono_focal_mm(img.camera) {
			format!("{id} Mono {mm}mm")
		} else {
			format!("{id} Mono")
		}
	} else {
		id
	}
}

/// File stem for export: `A2_mono` vs `A1`.
pub fn export_stem(img: &RawImage<'_>) -> String {
	let id = img.camera.to_string();
	if is_mono_image(img) {
		format!("{id}_mono")
	} else {
		id
	}
}
