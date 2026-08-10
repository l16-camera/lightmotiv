use std::collections::HashMap;

use crate::{
	mirror_pose::{extrinsics_from_movable_mirror, movable_mirror_bundle},
	types::CameraId,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FusionMeta {
	pub tof_range_m: Option<f32>,
	pub tof_calibration: Option<TofCalibration>,
	pub imu: Option<ImuSummary>,
	pub gps: Option<GpsFix>,
	/// Per-shot mirror actuator hall code (`CameraModule.mirror_position`).
	pub mirror_hall_codes: HashMap<CameraId, i32>,
	pub module_geometry: Vec<ModuleGeometry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TofCalibration {
	pub offset_distance: f32,
	pub offset_measurement: f32,
	pub xtalk_distance: f32,
	pub xtalk_measurement: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImuSummary {
	pub frames: usize,
	pub accel_samples: usize,
	pub gyro_samples: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpsFix {
	pub latitude: f64,
	pub longitude: f64,
	pub altitude_m: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModuleGeometry {
	pub camera: CameraId,
	pub mirror_type: Option<MirrorType>,
	pub focus_calibrations: Vec<FocusCalibration>,
	pub distortion: crate::distortion::ModuleDistortion,
	pub has_vignetting: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MirrorType {
	None,
	Glued,
	Movable,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Range2F {
	pub min: f32,
	pub max: f32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ActuatorTransformType {
	MeanStdNormalize,
	TanHalfTheta,
	Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActuatorAnglePair {
	pub hall_code: i32,
	pub angle_rad: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuadraticModel {
	pub use_rplus_for_left_segment: bool,
	pub use_rplus_for_right_segment: bool,
	pub inflection_value: f32,
	pub coeffs: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MirrorSystemData {
	pub real_camera_location: [f32; 3],
	pub real_camera_orientation: [f32; 9],
	pub rotation_axis: [f32; 3],
	pub point_on_rotation_axis: [f32; 3],
	pub distance_mirror_plane_to_point_on_rotation_axis: f32,
	pub mirror_normal_at_zero_degrees: [f32; 3],
	pub flip_img_around_x: bool,
	pub mirror_angle_range: Range2F,
	pub reprojection_error: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MirrorActuatorMappingData {
	pub transformation_type: ActuatorTransformType,
	pub actuator_length_offset: f32,
	pub actuator_length_scale: f32,
	pub mirror_angle_offset: f32,
	pub mirror_angle_scale: f32,
	pub actuator_angle_pairs: Vec<ActuatorAnglePair>,
	pub quadratic_model: Option<QuadraticModel>,
	pub angle_to_hall_code_error: Option<f32>,
	pub hall_code_to_angle_error: Option<f32>,
	pub hall_code_range: Option<Range2F>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MovableMirrorData {
	pub mirror_system: Option<MirrorSystemData>,
	pub actuator_mapping: Option<MirrorActuatorMappingData>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FocusCalibration {
	pub focus_distance: f32,
	pub k_matrix: Option<[f32; 9]>,
	pub rotation: Option<[f32; 9]>,
	pub translation: Option<[f32; 3]>,
	pub reprojection_error: Option<f32>,
	pub focus_hall_code: Option<f32>,
	pub movable_mirror: Option<MovableMirrorData>,
	/// Deprecated alias: `movable_mirror.is_some()`.
	pub has_movable_mirror: bool,
}

/// Factory calibration plane for intrinsics (mm diopter-scale distance in `.lri` dumps).
pub const CAL_FOCUS_NEAR: f32 = 818.0;
pub const CAL_FOCUS_FAR: f32 = 1500.0;

/// 35 mm-equivalent focal length threshold between wide (28 mm) and tele (70/150 mm) primes.
pub const FOCAL_WIDE_TELE_THRESHOLD: i32 = 70;

#[derive(Clone, Debug, PartialEq)]
pub struct SelectedFocusBundle {
	pub intrinsics_index: usize,
	/// `None` when only factory intrinsics exist (common for movable-mirror modules until mirror pose is parsed).
	pub extrinsics_index: Option<usize>,
	pub intrinsics_focus_distance: f32,
	pub extrinsics_focus_distance: Option<f32>,
	pub focus_hall_code: Option<f32>,
	pub k_matrix: Option<[f32; 9]>,
	pub rotation: Option<[f32; 9]>,
	pub translation: Option<[f32; 3]>,
	pub reprojection_error: Option<f32>,
	pub has_movable_mirror: bool,
	pub has_extrinsics: bool,
}

/// Map shot `image_focal_length` (35 mm equiv, from `LightHeader`) to factory intrinsics plane.
pub fn target_intrinsics_focus_distance(shot_focal_mm: i32) -> f32 {
	if shot_focal_mm < FOCAL_WIDE_TELE_THRESHOLD {
		CAL_FOCUS_NEAR
	} else {
		CAL_FOCUS_FAR
	}
}

pub fn pick_intrinsics_bundle<'a>(
	bundles: &'a [FocusCalibration],
	shot_focal_mm: i32,
) -> Option<(usize, &'a FocusCalibration)> {
	let target = target_intrinsics_focus_distance(shot_focal_mm);
	let mut candidates: Vec<(usize, &FocusCalibration)> = bundles
		.iter()
		.enumerate()
		.filter(|(_, b)| b.k_matrix.is_some())
		.collect();
	if candidates.is_empty() {
		return None;
	}
	candidates.sort_by(|a, b| {
		a.1.focus_distance
			.partial_cmp(&b.1.focus_distance)
			.unwrap_or(std::cmp::Ordering::Equal)
	});
	let exact = candidates
		.iter()
		.find(|(_, b)| (b.focus_distance - target).abs() < 0.5)
		.copied();
	if let Some(hit) = exact {
		return Some(hit);
	}
	candidates
		.iter()
		.min_by(|a, b| {
			(a.1.focus_distance - target)
				.abs()
				.partial_cmp(&(b.1.focus_distance - target).abs())
				.unwrap_or(std::cmp::Ordering::Equal)
		})
		.copied()
}

pub fn pick_extrinsics_bundle<'a>(
	bundles: &'a [FocusCalibration],
) -> Option<(usize, &'a FocusCalibration)> {
	bundles
		.iter()
		.enumerate()
		.find(|(_, b)| b.rotation.is_some() || b.translation.is_some())
}

pub fn pick_focus_bundle_with_mirror(
	module: &ModuleGeometry,
	shot_focal_mm: i32,
	mirror_hall_code: Option<i32>,
) -> Option<SelectedFocusBundle> {
	let (intrinsics_index, intrinsics) =
		pick_intrinsics_bundle(&module.focus_calibrations, shot_focal_mm)?;

	let canonical = pick_extrinsics_bundle(&module.focus_calibrations);
	let mirror = movable_mirror_bundle(&module.focus_calibrations).and_then(|(idx, mm)| {
		let hall = mirror_hall_code.unwrap_or(0);
		let ext = extrinsics_from_movable_mirror(mm, hall)?;
		Some((idx, ext))
	});

	let (
		extrinsics_index,
		extrinsics_focus_distance,
		rotation,
		translation,
		reprojection_error,
		has_extrinsics,
	) = if let Some((idx, ext)) = mirror {
		(
			Some(idx),
			Some(module.focus_calibrations[idx].focus_distance),
			Some(ext.rotation),
			Some(ext.translation),
			module.focus_calibrations[idx].reprojection_error,
			true,
		)
	} else if let Some((idx, ext)) = canonical {
		(
			Some(idx),
			Some(ext.focus_distance),
			ext.rotation,
			ext.translation,
			ext.reprojection_error,
			true,
		)
	} else {
		(None, None, None, None, None, false)
	};
	Some(SelectedFocusBundle {
		intrinsics_index,
		extrinsics_index,
		intrinsics_focus_distance: intrinsics.focus_distance,
		extrinsics_focus_distance,
		focus_hall_code: intrinsics.focus_hall_code,
		k_matrix: intrinsics.k_matrix,
		rotation,
		translation,
		reprojection_error,
		has_movable_mirror: intrinsics.has_movable_mirror
			|| movable_mirror_bundle(&module.focus_calibrations).is_some()
			|| canonical
				.map(|(_, e)| e.has_movable_mirror)
				.unwrap_or(false),
		has_extrinsics,
	})
}

pub fn pick_focus_bundle(
	module: &ModuleGeometry,
	shot_focal_mm: i32,
) -> Option<SelectedFocusBundle> {
	pick_focus_bundle_with_mirror(module, shot_focal_mm, None)
}

impl FusionMeta {
	pub fn geometry_module_count(&self) -> usize {
		self.module_geometry.len()
	}

	pub fn modules_with_intrinsics(&self) -> usize {
		self.module_geometry
			.iter()
			.filter(|m| m.focus_calibrations.iter().any(|f| f.k_matrix.is_some()))
			.count()
	}

	pub fn modules_with_mirror_system(&self) -> usize {
		self.module_geometry
			.iter()
			.filter(|m| {
				m.focus_calibrations.iter().any(|f| {
					f.movable_mirror
						.as_ref()
						.and_then(|mm| mm.mirror_system.as_ref())
						.is_some()
				})
			})
			.count()
	}

	pub fn pick_all_focus_bundles(
		&self,
		shot_focal_mm: i32,
	) -> Vec<(CameraId, SelectedFocusBundle)> {
		self.module_geometry
			.iter()
			.filter_map(|m| {
				let hall = self.mirror_hall_codes.get(&m.camera).copied();
				pick_focus_bundle_with_mirror(m, shot_focal_mm, hall).map(|sel| (m.camera, sel))
			})
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn bundle(fd: f32, hall: Option<f32>, k: bool, rt: bool) -> FocusCalibration {
		FocusCalibration {
			focus_distance: fd,
			k_matrix: k.then(|| [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]),
			rotation: rt.then(|| [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]),
			translation: rt.then(|| [0.0, 0.0, 0.0]),
			reprojection_error: rt.then_some(0.0),
			focus_hall_code: hall,
			movable_mirror: None,
			has_movable_mirror: false,
		}
	}

	fn l16_module(halls: (f32, f32)) -> ModuleGeometry {
		ModuleGeometry {
			camera: CameraId::A1,
			mirror_type: Some(MirrorType::None),
			focus_calibrations: vec![
				bundle(CAL_FOCUS_NEAR, Some(halls.0), true, false),
				bundle(CAL_FOCUS_FAR, Some(halls.1), true, false),
				bundle(CAL_FOCUS_NEAR, None, false, true),
			],
			distortion: crate::distortion::ModuleDistortion::default(),
			has_vignetting: false,
		}
	}

	#[test]
	fn tele_focal_picks_far_intrinsics() {
		let m = l16_module((9570.0, 10447.0));
		let sel = pick_focus_bundle(&m, 87).unwrap();
		assert_eq!(sel.intrinsics_index, 1);
		assert_eq!(sel.intrinsics_focus_distance, CAL_FOCUS_FAR);
		assert_eq!(sel.extrinsics_index, Some(2));
		assert!(sel.k_matrix.is_some());
		assert!(sel.rotation.is_some());
		assert!(sel.has_extrinsics);
	}

	#[test]
	fn movable_without_canonical_extrinsics_still_picks_intrinsics() {
		let m = ModuleGeometry {
			camera: CameraId::B1,
			mirror_type: Some(MirrorType::Movable),
			focus_calibrations: vec![
				bundle(CAL_FOCUS_NEAR, Some(1659.0), true, false),
				bundle(CAL_FOCUS_FAR, Some(1556.0), true, false),
				bundle(CAL_FOCUS_NEAR, None, false, false),
			],
			distortion: crate::distortion::ModuleDistortion::default(),
			has_vignetting: false,
		};
		let sel = pick_focus_bundle(&m, 87).unwrap();
		assert_eq!(sel.intrinsics_focus_distance, CAL_FOCUS_FAR);
		assert!(sel.k_matrix.is_some());
		assert!(!sel.has_extrinsics);
	}

	#[test]
	fn wide_focal_picks_near_intrinsics() {
		let m = l16_module((9570.0, 10447.0));
		let sel = pick_focus_bundle(&m, 28).unwrap();
		assert_eq!(sel.intrinsics_index, 0);
		assert_eq!(sel.intrinsics_focus_distance, CAL_FOCUS_NEAR);
	}

	#[test]
	fn target_intrinsics_focus_distance_threshold() {
		assert_eq!(target_intrinsics_focus_distance(28), CAL_FOCUS_NEAR);
		assert_eq!(target_intrinsics_focus_distance(69), CAL_FOCUS_NEAR);
		assert_eq!(target_intrinsics_focus_distance(70), CAL_FOCUS_FAR);
		assert_eq!(target_intrinsics_focus_distance(87), CAL_FOCUS_FAR);
		assert_eq!(target_intrinsics_focus_distance(150), CAL_FOCUS_FAR);
	}

	#[test]
	fn pick_intrinsics_nearest_when_exact_missing() {
		let bundles = vec![
			bundle(800.0, None, true, false),
			bundle(1200.0, None, true, false),
		];
		let (idx, picked) = pick_intrinsics_bundle(&bundles, 87).unwrap();
		assert_eq!(idx, 1);
		assert_eq!(picked.focus_distance, 1200.0);
	}

	#[test]
	fn pick_all_focus_bundles_uses_mirror_hall_codes() {
		let Some(bytes) = crate::fixtures::l16_00078_bytes() else {
			return;
		};
		let lri = crate::LriFile::decode(&bytes).expect("decode fixture");
		let picks = lri
			.fusion
			.pick_all_focus_bundles(lri.focal_length.unwrap_or(87));
		assert_eq!(picks.len(), 16);
		let b1 = picks
			.iter()
			.find(|(c, _)| *c == CameraId::B1)
			.map(|(_, s)| s)
			.expect("B1 pick");
		assert!(b1.has_extrinsics);
		assert!(b1.has_movable_mirror);
		assert!(b1.rotation.is_some());
	}

	#[test]
	fn mirror_extrinsics_override_canonical_when_hall_present() {
		let mm = crate::mirror_pose::tests_support::b1_mirror_fixture();
		let m = ModuleGeometry {
			camera: CameraId::B1,
			mirror_type: Some(MirrorType::Movable),
			focus_calibrations: vec![
				bundle(CAL_FOCUS_FAR, Some(1556.0), true, false),
				FocusCalibration {
					focus_distance: CAL_FOCUS_NEAR,
					k_matrix: None,
					rotation: None,
					translation: None,
					reprojection_error: None,
					focus_hall_code: None,
					movable_mirror: Some(mm),
					has_movable_mirror: true,
				},
				bundle(CAL_FOCUS_NEAR, None, false, true),
			],
			distortion: crate::distortion::ModuleDistortion::default(),
			has_vignetting: false,
		};
		let sel = pick_focus_bundle_with_mirror(&m, 87, Some(769)).unwrap();
		assert!(sel.has_extrinsics);
		assert!(sel.has_movable_mirror);
		assert_ne!(sel.translation, Some([0.0, 0.0, 0.0]));
	}

	#[test]
	fn l16_00078_movable_mirror_on_tele_modules() {
		let Some(data) = crate::fixtures::l16_00078_bytes() else {
			return;
		};
		let lri = crate::LriFile::decode(&data).expect("decode fixture");
		assert_eq!(lri.fusion.geometry_module_count(), 16);
		assert!(
			lri.fusion.modules_with_mirror_system() >= 8,
			"expected mirror_system on movable modules"
		);

		let b1 = lri
			.fusion
			.module_geometry
			.iter()
			.find(|m| m.camera == CameraId::B1)
			.expect("B1 geometry");
		let with_mirror: Vec<_> = b1
			.focus_calibrations
			.iter()
			.filter(|b| b.movable_mirror.is_some())
			.collect();
		assert!(
			!with_mirror.is_empty(),
			"B1 should carry moveable_mirror extrinsics"
		);
		let mm = with_mirror[0].movable_mirror.as_ref().unwrap();
		assert!(mm.mirror_system.is_some());
		assert!(mm
			.actuator_mapping
			.as_ref()
			.is_some_and(|a| !a.actuator_angle_pairs.is_empty()));
	}
}

pub(crate) fn mirror_type_from_proto(
	mt: lri_proto::geometric_calibration::geometric_calibration::MirrorType,
) -> MirrorType {
	use lri_proto::geometric_calibration::geometric_calibration::MirrorType as Mt;
	match mt {
		Mt::NONE => MirrorType::None,
		Mt::GLUED => MirrorType::Glued,
		Mt::MOVABLE => MirrorType::Movable,
	}
}

pub(crate) fn mat3(mat: lri_proto::matrix3x3f::Matrix3x3F) -> [f32; 9] {
	[
		mat.x00(),
		mat.x01(),
		mat.x02(),
		mat.x10(),
		mat.x11(),
		mat.x12(),
		mat.x20(),
		mat.x21(),
		mat.x22(),
	]
}

pub(crate) fn point3(p: lri_proto::point3f::Point3F) -> [f32; 3] {
	[p.x(), p.y(), p.z()]
}

pub(crate) fn point2f(p: lri_proto::point2f::Point2F) -> [f32; 2] {
	[p.x(), p.y()]
}

pub(crate) fn range2f(r: lri_proto::range2f::Range2F) -> Range2F {
	Range2F {
		min: r.min_val(),
		max: r.max_val(),
	}
}

pub(crate) fn actuator_transform_from_proto(
	tt: lri_proto::mirror_system::mirror_actuator_mapping::TransformationType,
) -> ActuatorTransformType {
	use lri_proto::mirror_system::mirror_actuator_mapping::TransformationType as Tt;
	match tt {
		Tt::MEAN_STD_NORMALIZE => ActuatorTransformType::MeanStdNormalize,
		Tt::TAN_HALF_THETA => ActuatorTransformType::TanHalfTheta,
	}
}

pub(crate) fn extract_mirror_system(
	ms: lri_proto::mirror_system::MirrorSystem,
) -> MirrorSystemData {
	MirrorSystemData {
		real_camera_location: ms
			.real_camera_location
			.as_ref()
			.map(|p| point3(p.clone()))
			.unwrap_or([0.0; 3]),
		real_camera_orientation: ms
			.real_camera_orientation
			.as_ref()
			.map(|m| mat3(m.clone()))
			.unwrap_or([0.0; 9]),
		rotation_axis: ms
			.rotation_axis
			.as_ref()
			.map(|p| point3(p.clone()))
			.unwrap_or([0.0; 3]),
		point_on_rotation_axis: ms
			.point_on_rotation_axis
			.as_ref()
			.map(|p| point3(p.clone()))
			.unwrap_or([0.0; 3]),
		distance_mirror_plane_to_point_on_rotation_axis: ms
			.distance_mirror_plane_to_point_on_rotation_axis(),
		mirror_normal_at_zero_degrees: ms
			.mirror_normal_at_zero_degrees
			.as_ref()
			.map(|p| point3(p.clone()))
			.unwrap_or([0.0; 3]),
		flip_img_around_x: ms.flip_img_around_x(),
		mirror_angle_range: ms
			.mirror_angle_range
			.as_ref()
			.map(|r| range2f(r.clone()))
			.unwrap_or(Range2F { min: 0.0, max: 0.0 }),
		reprojection_error: ms.reprojection_error,
	}
}

pub(crate) fn extract_actuator_mapping(
	m: lri_proto::mirror_system::MirrorActuatorMapping,
) -> MirrorActuatorMappingData {
	let quadratic_model = m.quadratic_model.as_ref().map(|q| QuadraticModel {
		use_rplus_for_left_segment: q.use_rplus_for_left_segment(),
		use_rplus_for_right_segment: q.use_rplus_for_right_segment(),
		inflection_value: q.inflection_value(),
		coeffs: q.model_coeffs.clone(),
	});
	let actuator_angle_pairs = m
		.actuator_angle_pair_vec
		.iter()
		.map(|p| ActuatorAnglePair {
			hall_code: p.hall_code(),
			angle_rad: p.angle(),
		})
		.collect();
	MirrorActuatorMappingData {
		transformation_type: actuator_transform_from_proto(m.transformation_type()),
		actuator_length_offset: m.actuator_length_offset(),
		actuator_length_scale: m.actuator_length_scale(),
		mirror_angle_offset: m.mirror_angle_offset(),
		mirror_angle_scale: m.mirror_angle_scale(),
		actuator_angle_pairs,
		quadratic_model,
		angle_to_hall_code_error: m.angle_to_hall_code_error,
		hall_code_to_angle_error: m.hall_code_to_angle_error,
		hall_code_range: m.hall_code_range.as_ref().map(|r| range2f(r.clone())),
	}
}

pub(crate) fn extract_movable_mirror(
	mm: lri_proto::geometric_calibration::geometric_calibration::extrinsics::MovableMirrorFormat,
) -> MovableMirrorData {
	MovableMirrorData {
		mirror_system: mm
			.mirror_system
			.as_ref()
			.map(|ms| extract_mirror_system(ms.clone())),
		actuator_mapping: mm
			.mirror_actuator_mapping
			.as_ref()
			.map(|am| extract_actuator_mapping(am.clone())),
	}
}

pub(crate) fn extract_module_geometry(
	mcal: &lri_proto::lightheader::FactoryModuleCalibration,
) -> Option<ModuleGeometry> {
	let geometry = mcal.geometry.as_ref()?;
	let camera = mcal.camera_id().into();

	let mirror_type = Some(mirror_type_from_proto(geometry.mirror_type()));

	let mut focus_calibrations = Vec::new();
	for bundle in &geometry.per_focus_calibration {
		let k_matrix = bundle
			.intrinsics
			.as_ref()
			.and_then(|i| i.k_mat.as_ref())
			.map(|m| mat3(m.clone()));

		let mut rotation = None;
		let mut translation = None;
		let mut reprojection_error = None;
		let mut movable_mirror = None;

		if let Some(extr) = bundle.extrinsics.as_ref() {
			if let Some(canonical) = extr.canonical.as_ref() {
				if let Some(r) = canonical.rotation.as_ref() {
					rotation = Some(mat3(r.clone()));
				}
				if let Some(t) = canonical.translation.as_ref() {
					translation = Some(point3(t.clone()));
				}
				reprojection_error = canonical.reprojection_error;
			}
			if let Some(mm) = extr.moveable_mirror.as_ref() {
				movable_mirror = Some(extract_movable_mirror(mm.clone()));
			}
		}

		let has_movable_mirror = movable_mirror.is_some();

		focus_calibrations.push(FocusCalibration {
			focus_distance: bundle.focus_distance(),
			k_matrix,
			rotation,
			translation,
			reprojection_error,
			focus_hall_code: bundle.focus_hall_code,
			movable_mirror,
			has_movable_mirror,
		});
	}

	let mut distortion = crate::distortion::ModuleDistortion::default();
	if let Some(dist) = geometry.distortion.as_ref() {
		if let Some(poly) = dist.polynomial.as_ref() {
			if let (Some(c), Some(n)) =
				(poly.distortion_center.as_ref(), poly.normalization.as_ref())
			{
				distortion.polynomial = Some(crate::distortion::PolynomialDistortion {
					center: point2f(c.clone()),
					normalization: point2f(n.clone()),
					coeffs: poly.coeffs.clone(),
				});
			}
		}
		if let Some(cra) = dist.cra.as_ref() {
			if let Some(center) = cra.distortion_center.as_ref() {
				distortion.cra = Some(crate::distortion::CraDistortion {
					center: point2f(center.clone()),
					sensor_distance: cra.sensor_distance(),
					exit_pupil_distance: cra.exit_pupil_distance(),
					pixel_size: cra.pixel_size(),
					cra_profile: cra.cra.iter().map(|p| point2f(p.clone())).collect(),
					corr_lut: cra.coeffs.iter().map(|p| point2f(p.clone())).collect(),
					lens_hall_code: cra.lens_hall_code,
					distance_hall_ratio: cra.distance_hall_ratio,
				});
			}
		}
	}

	Some(ModuleGeometry {
		camera,
		mirror_type,
		focus_calibrations,
		distortion,
		has_vignetting: mcal.vignetting.is_some(),
	})
}

pub(crate) fn imu_summary(imu_frames: &[lri_proto::imu_data::IMUData]) -> ImuSummary {
	let mut accel_samples = 0usize;
	let mut gyro_samples = 0usize;
	for frame in imu_frames {
		accel_samples += frame.accelerometer.len();
		gyro_samples += frame.gyroscope.len();
	}
	ImuSummary {
		frames: imu_frames.len(),
		accel_samples,
		gyro_samples,
	}
}

pub(crate) fn gps_from_proto(gps: lri_proto::gps_data::GPSData) -> Option<GpsFix> {
	let latitude = gps.latitude?;
	let longitude = gps.longitude?;
	let altitude_m = gps.altitude.into_option().map(|a| a.value());
	Some(GpsFix {
		latitude,
		longitude,
		altitude_m,
	})
}

pub(crate) fn tof_from_device(
	dc: lri_proto::lightheader::FactoryDeviceCalibration,
) -> Option<TofCalibration> {
	let tof = dc.tof.into_option()?;
	Some(TofCalibration {
		offset_distance: tof.offset_distance(),
		offset_measurement: tof.offset_measurement(),
		xtalk_distance: tof.xtalk_distance(),
		xtalk_measurement: tof.xtalk_measurement(),
	})
}
