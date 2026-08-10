use lri_rs::{
	target_intrinsics_focus_distance, ActuatorAnglePair, ActuatorTransformType, CameraId,
	FocusCalibration, FusionMeta, LriFile, MirrorActuatorMappingData, MirrorSystemData, MirrorType,
	MovableMirrorData, QuadraticModel, Range2F, SelectedFocusBundle,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct FusionSidecar {
	pub reference_camera: Option<String>,
	pub focal_length: Option<i32>,
	pub firmware_version: Option<String>,
	pub af_achieved: Option<bool>,
	pub shot: ShotMeta,
	pub focus_pick: Option<FocusPickMeta>,
	pub fusion: FusionDetail,
}

#[derive(Debug, Serialize)]
pub struct FocusPickMeta {
	pub shot_focal_mm: i32,
	pub target_intrinsics_focus_distance: f32,
	pub modules: Vec<ModuleFocusPickJson>,
}

#[derive(Debug, Serialize)]
pub struct ModuleFocusPickJson {
	pub camera: String,
	pub intrinsics_index: usize,
	pub extrinsics_index: Option<usize>,
	pub intrinsics_focus_distance: f32,
	pub extrinsics_focus_distance: Option<f32>,
	pub focus_hall_code: Option<f32>,
	pub has_extrinsics: bool,
	pub k_matrix: Option<[f32; 9]>,
	pub rotation: Option<[f32; 9]>,
	pub translation: Option<[f32; 3]>,
	pub reprojection_error: Option<f32>,
}

#[derive(Debug, Serialize)]
pub struct ShotMeta {
	pub tof_range_m: Option<f32>,
	pub tof_calibration: Option<TofCalibrationJson>,
	pub imu: Option<ImuJson>,
	pub gps: Option<GpsJson>,
}

#[derive(Debug, Serialize)]
pub struct TofCalibrationJson {
	pub offset_distance: f32,
	pub offset_measurement: f32,
	pub xtalk_distance: f32,
	pub xtalk_measurement: f32,
}

#[derive(Debug, Serialize)]
pub struct ImuJson {
	pub frames: usize,
	pub accel_samples: usize,
	pub gyro_samples: usize,
}

#[derive(Debug, Serialize)]
pub struct GpsJson {
	pub latitude: f64,
	pub longitude: f64,
	pub altitude_m: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct FusionDetail {
	pub geometry_module_count: usize,
	pub modules_with_intrinsics: usize,
	pub modules: Vec<ModuleGeometryJson>,
}

#[derive(Debug, Serialize)]
pub struct ModuleGeometryJson {
	pub camera: String,
	pub mirror_type: Option<String>,
	pub has_vignetting: bool,
	pub distortion: DistortionJson,
	pub focus_calibrations: Vec<FocusCalibrationJson>,
}

#[derive(Debug, Serialize)]
pub struct DistortionJson {
	pub polynomial: bool,
	pub cra: bool,
	pub poly_coeffs: usize,
}

#[derive(Debug, Serialize)]
pub struct FocusCalibrationJson {
	pub focus_distance: f32,
	pub focus_hall_code: Option<f32>,
	pub has_movable_mirror: bool,
	pub movable_mirror: Option<MovableMirrorJson>,
	pub reprojection_error: Option<f32>,
	pub k_matrix: Option<[f32; 9]>,
	pub rotation: Option<[f32; 9]>,
	pub translation: Option<[f32; 3]>,
}

#[derive(Debug, Serialize)]
pub struct MovableMirrorJson {
	pub mirror_system: Option<MirrorSystemJson>,
	pub actuator_mapping: Option<MirrorActuatorMappingJson>,
}

#[derive(Debug, Serialize)]
pub struct MirrorSystemJson {
	pub real_camera_location: [f32; 3],
	pub real_camera_orientation: [f32; 9],
	pub rotation_axis: [f32; 3],
	pub point_on_rotation_axis: [f32; 3],
	pub distance_mirror_plane_to_point_on_rotation_axis: f32,
	pub mirror_normal_at_zero_degrees: [f32; 3],
	pub flip_img_around_x: bool,
	pub mirror_angle_range: Range2FJson,
	pub reprojection_error: Option<f32>,
}

#[derive(Debug, Serialize)]
pub struct MirrorActuatorMappingJson {
	pub transformation_type: String,
	pub actuator_length_offset: f32,
	pub actuator_length_scale: f32,
	pub mirror_angle_offset: f32,
	pub mirror_angle_scale: f32,
	pub actuator_angle_pairs: Vec<ActuatorAnglePairJson>,
	pub quadratic_model: Option<QuadraticModelJson>,
	pub angle_to_hall_code_error: Option<f32>,
	pub hall_code_to_angle_error: Option<f32>,
	pub hall_code_range: Option<Range2FJson>,
}

#[derive(Debug, Serialize)]
pub struct Range2FJson {
	pub min: f32,
	pub max: f32,
}

#[derive(Debug, Serialize)]
pub struct ActuatorAnglePairJson {
	pub hall_code: i32,
	pub angle_rad: f32,
}

#[derive(Debug, Serialize)]
pub struct QuadraticModelJson {
	pub use_rplus_for_left_segment: bool,
	pub use_rplus_for_right_segment: bool,
	pub inflection_value: f32,
	pub coeffs: Vec<f32>,
}

pub fn from_lri(lri: &LriFile<'_>) -> FusionSidecar {
	let fusion = &lri.fusion;
	let focus_pick = lri.focal_length.map(|focal| focus_pick_meta(fusion, focal));
	FusionSidecar {
		reference_camera: lri.image_reference_camera.map(camera_id_str),
		focal_length: lri.focal_length,
		firmware_version: lri.firmware_version.clone(),
		af_achieved: lri.af_achieved,
		shot: shot_meta(fusion),
		focus_pick,
		fusion: fusion_detail(fusion),
	}
}

fn focus_pick_meta(fusion: &FusionMeta, shot_focal_mm: i32) -> FocusPickMeta {
	FocusPickMeta {
		shot_focal_mm,
		target_intrinsics_focus_distance: target_intrinsics_focus_distance(shot_focal_mm),
		modules: fusion
			.pick_all_focus_bundles(shot_focal_mm)
			.into_iter()
			.map(|(camera, sel)| module_focus_pick(camera, sel))
			.collect(),
	}
}

fn module_focus_pick(camera: CameraId, sel: SelectedFocusBundle) -> ModuleFocusPickJson {
	ModuleFocusPickJson {
		camera: camera_id_str(camera),
		intrinsics_index: sel.intrinsics_index,
		extrinsics_index: sel.extrinsics_index,
		intrinsics_focus_distance: sel.intrinsics_focus_distance,
		extrinsics_focus_distance: sel.extrinsics_focus_distance,
		focus_hall_code: sel.focus_hall_code,
		has_extrinsics: sel.has_extrinsics,
		k_matrix: sel.k_matrix,
		rotation: sel.rotation,
		translation: sel.translation,
		reprojection_error: sel.reprojection_error,
	}
}

fn shot_meta(fusion: &FusionMeta) -> ShotMeta {
	ShotMeta {
		tof_range_m: fusion.tof_range_m,
		tof_calibration: fusion.tof_calibration.as_ref().map(|t| TofCalibrationJson {
			offset_distance: t.offset_distance,
			offset_measurement: t.offset_measurement,
			xtalk_distance: t.xtalk_distance,
			xtalk_measurement: t.xtalk_measurement,
		}),
		imu: fusion.imu.as_ref().map(|i| ImuJson {
			frames: i.frames,
			accel_samples: i.accel_samples,
			gyro_samples: i.gyro_samples,
		}),
		gps: fusion.gps.as_ref().map(|g| GpsJson {
			latitude: g.latitude,
			longitude: g.longitude,
			altitude_m: g.altitude_m,
		}),
	}
}

fn fusion_detail(fusion: &FusionMeta) -> FusionDetail {
	FusionDetail {
		geometry_module_count: fusion.geometry_module_count(),
		modules_with_intrinsics: fusion.modules_with_intrinsics(),
		modules: fusion
			.module_geometry
			.iter()
			.map(|m| ModuleGeometryJson {
				camera: camera_id_str(m.camera),
				mirror_type: m.mirror_type.map(mirror_type_str),
				has_vignetting: m.has_vignetting,
				distortion: DistortionJson {
					polynomial: m.distortion.has_polynomial(),
					cra: m.distortion.has_cra(),
					poly_coeffs: m.distortion.poly_coeffs(),
				},
				focus_calibrations: m
					.focus_calibrations
					.iter()
					.map(focus_calibration_json)
					.collect(),
			})
			.collect(),
	}
}

fn focus_calibration_json(f: &FocusCalibration) -> FocusCalibrationJson {
	FocusCalibrationJson {
		focus_distance: f.focus_distance,
		focus_hall_code: f.focus_hall_code,
		has_movable_mirror: f.has_movable_mirror,
		movable_mirror: f.movable_mirror.as_ref().map(movable_mirror_json),
		reprojection_error: f.reprojection_error,
		k_matrix: f.k_matrix,
		rotation: f.rotation,
		translation: f.translation,
	}
}

fn movable_mirror_json(mm: &MovableMirrorData) -> MovableMirrorJson {
	MovableMirrorJson {
		mirror_system: mm.mirror_system.as_ref().map(mirror_system_json),
		actuator_mapping: mm.actuator_mapping.as_ref().map(actuator_mapping_json),
	}
}

fn mirror_system_json(ms: &MirrorSystemData) -> MirrorSystemJson {
	MirrorSystemJson {
		real_camera_location: ms.real_camera_location,
		real_camera_orientation: ms.real_camera_orientation,
		rotation_axis: ms.rotation_axis,
		point_on_rotation_axis: ms.point_on_rotation_axis,
		distance_mirror_plane_to_point_on_rotation_axis: ms
			.distance_mirror_plane_to_point_on_rotation_axis,
		mirror_normal_at_zero_degrees: ms.mirror_normal_at_zero_degrees,
		flip_img_around_x: ms.flip_img_around_x,
		mirror_angle_range: range2f_json(&ms.mirror_angle_range),
		reprojection_error: ms.reprojection_error,
	}
}

fn actuator_mapping_json(am: &MirrorActuatorMappingData) -> MirrorActuatorMappingJson {
	MirrorActuatorMappingJson {
		transformation_type: actuator_transform_str(am.transformation_type),
		actuator_length_offset: am.actuator_length_offset,
		actuator_length_scale: am.actuator_length_scale,
		mirror_angle_offset: am.mirror_angle_offset,
		mirror_angle_scale: am.mirror_angle_scale,
		actuator_angle_pairs: am
			.actuator_angle_pairs
			.iter()
			.map(actuator_angle_pair_json)
			.collect(),
		quadratic_model: am.quadratic_model.as_ref().map(quadratic_model_json),
		angle_to_hall_code_error: am.angle_to_hall_code_error,
		hall_code_to_angle_error: am.hall_code_to_angle_error,
		hall_code_range: am.hall_code_range.as_ref().map(range2f_json),
	}
}

fn range2f_json(r: &Range2F) -> Range2FJson {
	Range2FJson {
		min: r.min,
		max: r.max,
	}
}

fn actuator_angle_pair_json(p: &ActuatorAnglePair) -> ActuatorAnglePairJson {
	ActuatorAnglePairJson {
		hall_code: p.hall_code,
		angle_rad: p.angle_rad,
	}
}

fn quadratic_model_json(q: &QuadraticModel) -> QuadraticModelJson {
	QuadraticModelJson {
		use_rplus_for_left_segment: q.use_rplus_for_left_segment,
		use_rplus_for_right_segment: q.use_rplus_for_right_segment,
		inflection_value: q.inflection_value,
		coeffs: q.coeffs.clone(),
	}
}

fn actuator_transform_str(tt: ActuatorTransformType) -> String {
	match tt {
		ActuatorTransformType::MeanStdNormalize => "mean_std_normalize".into(),
		ActuatorTransformType::TanHalfTheta => "tan_half_theta".into(),
		ActuatorTransformType::Unknown => "unknown".into(),
	}
}

fn camera_id_str(id: CameraId) -> String {
	id.to_string()
}

fn mirror_type_str(mt: MirrorType) -> String {
	match mt {
		MirrorType::None => "none".into(),
		MirrorType::Glued => "glued".into(),
		MirrorType::Movable => "movable".into(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use lri_rs::LriFile;

	#[test]
	fn from_lri_builds_focus_pick_for_tele_shot() {
		let Some(bytes) = lri_rs::fixtures::l16_00078_bytes() else {
			return;
		};
		let lri = LriFile::decode(&bytes).expect("decode");
		let sidecar = from_lri(&lri);
		assert_eq!(sidecar.reference_camera.as_deref(), Some("A1"));
		assert_eq!(sidecar.focal_length, Some(87));
		let pick = sidecar.focus_pick.expect("focus pick");
		assert_eq!(pick.shot_focal_mm, 87);
		assert_eq!(pick.modules.len(), 16);
		assert!(pick.modules.iter().all(|m| m.k_matrix.is_some()));
	}

	#[test]
	fn sidecar_roundtrips_through_json() {
		let Some(bytes) = lri_rs::fixtures::l16_00078_bytes() else {
			return;
		};
		let lri = LriFile::decode(&bytes).expect("decode");
		let sidecar = from_lri(&lri);
		let json = serde_json::to_string(&sidecar).expect("serialize");
		assert!(json.contains("\"movable_mirror\""));
		assert!(json.contains("\"focus_pick\""));
	}
}

pub fn write_json(lri: &LriFile<'_>, path: &camino::Utf8Path) -> anyhow::Result<()> {
	let sidecar = from_lri(lri);
	let json = serde_json::to_string_pretty(&sidecar)?;
	std::fs::write(path, json)?;
	Ok(())
}
