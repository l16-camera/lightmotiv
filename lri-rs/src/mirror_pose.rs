//! Movable-mirror extrinsics — port of `libcp.dylib` `0x1c7580` + `0x1c79e0`.
//!
//! Hall→angle uses `MirrorActuatorMapping` (linear for `MEAN_STD_NORMALIZE`, verified
//! against factory LUT on L16_00078). R/t matches the gallery fusion engine layout.

use crate::fusion::{
	ActuatorTransformType, MirrorActuatorMappingData, MirrorSystemData, MovableMirrorData,
};

const DEG2RAD: f64 = std::f64::consts::PI / 180.0;

#[derive(Clone, Copy, Debug)]
pub struct MirrorExtrinsics {
	pub rotation: [f32; 9],
	pub translation: [f32; 3],
}

/// Per-shot mirror hall code from `CameraModule.mirror_position` (0–1023).
pub fn hall_code_to_mirror_angle_deg(
	hall_code: i32,
	mapping: &MirrorActuatorMappingData,
) -> Option<f64> {
	let hall = hall_code as f64;
	let x = (hall - mapping.actuator_length_offset as f64) / mapping.actuator_length_scale as f64;
	match mapping.transformation_type {
		ActuatorTransformType::MeanStdNormalize => {
			Some(mapping.mirror_angle_offset as f64 + mapping.mirror_angle_scale as f64 * x)
		}
		ActuatorTransformType::TanHalfTheta => {
			let theta = (x * 0.5).tan();
			Some(mapping.mirror_angle_offset as f64 + mapping.mirror_angle_scale as f64 * theta)
		}
		ActuatorTransformType::Unknown => None,
	}
}

pub fn compute_mirror_extrinsics(
	mirror_system: &MirrorSystemData,
	angle_deg: f64,
) -> Option<MirrorExtrinsics> {
	let mut r_raw = [0.0f64; 9];
	let mut t_raw = [0.0f64; 3];
	compute_mirror_rt_raw(mirror_system, angle_deg, &mut r_raw, &mut t_raw)?;
	let mut t_out = [0.0f64; 3];
	finalize_mirror_translation(&r_raw, &t_raw, &mut t_out);
	Some(MirrorExtrinsics {
		rotation: f64_mat9_to_f32(r_raw),
		translation: [t_out[0] as f32, t_out[1] as f32, t_out[2] as f32],
	})
}

pub fn extrinsics_from_movable_mirror(
	mm: &MovableMirrorData,
	mirror_hall_code: i32,
) -> Option<MirrorExtrinsics> {
	let mirror_system = mm.mirror_system.as_ref()?;
	let mapping = mm.actuator_mapping.as_ref()?;
	let angle_deg = hall_code_to_mirror_angle_deg(mirror_hall_code, mapping)?;
	compute_mirror_extrinsics(mirror_system, angle_deg)
}

/// Find the focus bundle carrying `moveable_mirror` metadata.
pub fn movable_mirror_bundle<'a>(
	bundles: &'a [crate::fusion::FocusCalibration],
) -> Option<(usize, &'a MovableMirrorData)> {
	bundles
		.iter()
		.enumerate()
		.find_map(|(i, b)| b.movable_mirror.as_ref().map(|mm| (i, mm)))
}

fn f64_mat9_to_f32(m: [f64; 9]) -> [f32; 9] {
	m.map(|v| v as f32)
}

fn normalize3(v: [f64; 3]) -> [f64; 3] {
	let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
	if len < 1e-12 {
		return v;
	}
	[v[0] / len, v[1] / len, v[2] / len]
}

fn rodrigues(axis: [f64; 3], angle_rad: f64) -> [[f64; 3]; 3] {
	let (s, c) = angle_rad.sin_cos();
	let x = axis[0];
	let y = axis[1];
	let z = axis[2];
	[
		[
			c + x * x * (1.0 - c),
			x * y * (1.0 - c) - z * s,
			x * z * (1.0 - c) + y * s,
		],
		[
			y * x * (1.0 - c) + z * s,
			c + y * y * (1.0 - c),
			y * z * (1.0 - c) - x * s,
		],
		[
			z * x * (1.0 - c) - y * s,
			z * y * (1.0 - c) + x * s,
			c + z * z * (1.0 - c),
		],
	]
}

fn mat3_vec(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
	[
		m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
		m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
		m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
	]
}

fn mirror_sys_to_param(ms: &MirrorSystemData, _angle_deg: f64) -> MirrorParam {
	MirrorParam {
		r_cam: f32_mat9_to_f64(ms.real_camera_orientation),
		real_camera_location: ms.real_camera_location.map(f64::from),
		distance: ms.distance_mirror_plane_to_point_on_rotation_axis as f64,
		point_on_axis: ms.point_on_rotation_axis.map(f64::from),
		rotation_axis: ms.rotation_axis.map(f64::from),
		mirror_normal: ms.mirror_normal_at_zero_degrees.map(f64::from),
		flip_img_around_x: ms.flip_img_around_x,
	}
}

fn f32_mat9_to_f64(m: [f32; 9]) -> [[f64; 3]; 3] {
	[
		[m[0].into(), m[1].into(), m[2].into()],
		[m[3].into(), m[4].into(), m[5].into()],
		[m[6].into(), m[7].into(), m[8].into()],
	]
}

struct MirrorParam {
	r_cam: [[f64; 3]; 3],
	real_camera_location: [f64; 3],
	distance: f64,
	point_on_axis: [f64; 3],
	rotation_axis: [f64; 3],
	mirror_normal: [f64; 3],
	flip_img_around_x: bool,
}

fn mat3_mul(a: [[f64; 3]; 3], b: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
	let mut out = [[0.0; 3]; 3];
	for i in 0..3 {
		for j in 0..3 {
			out[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
		}
	}
	out
}

fn reflection_matrix(n: [f64; 3]) -> [[f64; 3]; 3] {
	let n = normalize3(n);
	[
		[
			1.0 - 2.0 * n[0] * n[0],
			-2.0 * n[0] * n[1],
			-2.0 * n[0] * n[2],
		],
		[
			-2.0 * n[1] * n[0],
			1.0 - 2.0 * n[1] * n[1],
			-2.0 * n[1] * n[2],
		],
		[
			-2.0 * n[2] * n[0],
			-2.0 * n[2] * n[1],
			1.0 - 2.0 * n[2] * n[2],
		],
	]
}

/// `libcp` `0x1c78b8`: negate the second row of R (image flip around horizontal axis).
fn flip_x_mat(m: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
	let mut out = m;
	out[1] = [-out[1][0], -out[1][1], -out[1][2]];
	out
}

fn reflect_point(p: [f64; 3], n: [f64; 3], p0: [f64; 3]) -> [f64; 3] {
	let n = normalize3(n);
	let d = (p[0] - p0[0]) * n[0] + (p[1] - p0[1]) * n[1] + (p[2] - p0[2]) * n[2];
	[
		p[0] - 2.0 * d * n[0],
		p[1] - 2.0 * d * n[1],
		p[2] - 2.0 * d * n[2],
	]
}

fn mat3_to_flat(m: [[f64; 3]; 3]) -> [f64; 9] {
	[
		m[0][0], m[0][1], m[0][2], m[1][0], m[1][1], m[1][2], m[2][0], m[2][1], m[2][2],
	]
}

/// Mirror pose from `MirrorSystem` + mirror angle (degrees).
///
/// Model: rotate `mirror_normal_at_zero` about `rotation_axis`, build reflection,
/// compose with `real_camera_orientation`, reflect `real_camera_location`.
/// Matches `libcp` `0x1c7580`/`0x1c79e0` structure (orthogonal R, mm translations).
fn compute_mirror_rt_raw(
	ms: &MirrorSystemData,
	angle_deg: f64,
	r_out: &mut [f64; 9],
	t_out: &mut [f64; 3],
) -> Option<()> {
	let p = mirror_sys_to_param(ms, angle_deg);
	let axis = normalize3(p.rotation_axis);
	let angle_rad = angle_deg * DEG2RAD;
	let r_delta = rodrigues(axis, angle_rad);
	let n = normalize3(mat3_vec(r_delta, p.mirror_normal));
	let p_plane = [
		p.point_on_axis[0] + p.distance * n[0],
		p.point_on_axis[1] + p.distance * n[1],
		p.point_on_axis[2] + p.distance * n[2],
	];

	let mut r = mat3_mul(reflection_matrix(n), p.r_cam);
	if p.flip_img_around_x {
		r = flip_x_mat(r);
	}
	*r_out = mat3_to_flat(r);
	*t_out = reflect_point(p.real_camera_location, n, p_plane);
	Some(())
}

/// Port of `libcp` `0x1c79e0` post-process: `t = -R * t_raw` (f32 extrinsics convention).
fn finalize_mirror_translation(r: &[f64; 9], t_raw: &[f64; 3], t_out: &mut [f64; 3]) {
	let tx = -(r[0] * t_raw[0] + r[1] * t_raw[1] + r[2] * t_raw[2]);
	let ty = -(r[3] * t_raw[0] + r[4] * t_raw[1] + r[5] * t_raw[2]);
	let tz = -(r[6] * t_raw[0] + r[7] * t_raw[1] + r[8] * t_raw[2]);
	t_out[0] = tx;
	t_out[1] = ty;
	t_out[2] = tz;
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::fusion::MirrorType;

	#[test]
	fn hall_to_angle_matches_b1_lut() {
		let mapping = MirrorActuatorMappingData {
			transformation_type: ActuatorTransformType::MeanStdNormalize,
			actuator_length_offset: 547.6,
			actuator_length_scale: 175.66673,
			mirror_angle_offset: 40.150547,
			mirror_angle_scale: 3.6576087,
			actuator_angle_pairs: vec![],
			quadratic_model: None,
			angle_to_hall_code_error: None,
			hall_code_to_angle_error: None,
			hall_code_range: None,
		};
		let angle = hall_code_to_mirror_angle_deg(769, &mapping).unwrap();
		assert!((angle - 44.75521).abs() < 0.02);
		let angle = hall_code_to_mirror_angle_deg(325, &mapping).unwrap();
		assert!((angle - 35.506893).abs() < 0.02);
	}

	#[test]
	fn mirror_extrinsics_produces_orthogonal_rotation() {
		let ms = MirrorSystemData {
			real_camera_location: [18.54517, 7.6582804, -3.4655511],
			real_camera_orientation: [
				-0.38093942,
				0.47482356,
				0.7933648,
				-0.49646932,
				0.61882627,
				-0.60874647,
				-0.7800022,
				-0.6257768,
				1.5680847e-8,
			],
			rotation_axis: [0.60439825, 0.7966334, -0.008826814],
			point_on_rotation_axis: [22.03876, 5.055312, 0.8291366],
			distance_mirror_plane_to_point_on_rotation_axis: 3.9343035,
			mirror_normal_at_zero_degrees: [0.79949564, -0.6006531, -0.0047436645],
			flip_img_around_x: false,
			mirror_angle_range: crate::fusion::Range2F {
				min: 35.5,
				max: 44.75,
			},
			reprojection_error: Some(0.35),
		};
		let ext = compute_mirror_extrinsics(&ms, 44.755).unwrap();
		let r = ext.rotation;
		// R^T R ≈ I
		for row in 0..3 {
			for col in 0..3 {
				let mut dot = 0.0f64;
				for k in 0..3 {
					dot += r[k * 3 + row] as f64 * r[k * 3 + col] as f64;
				}
				let expect = if row == col { 1.0 } else { 0.0 };
				assert!(
					(dot - expect).abs() < 0.05,
					"orthogonal check ({row},{col}) dot={dot}"
				);
			}
		}
		assert!(ext.translation[0].abs() < 80.0);
	}

	#[test]
	fn tan_half_theta_actuator_mapping() {
		let mapping = MirrorActuatorMappingData {
			transformation_type: ActuatorTransformType::TanHalfTheta,
			actuator_length_offset: 0.0,
			actuator_length_scale: 1.0,
			mirror_angle_offset: 10.0,
			mirror_angle_scale: 5.0,
			actuator_angle_pairs: vec![],
			quadratic_model: None,
			angle_to_hall_code_error: None,
			hall_code_to_angle_error: None,
			hall_code_range: None,
		};
		let angle = hall_code_to_mirror_angle_deg(1, &mapping).unwrap();
		let x = 1.0f64;
		let expected = 10.0 + 5.0 * (x * 0.5).tan();
		assert!((angle - expected).abs() < 1e-6);
	}

	#[test]
	fn unknown_transform_returns_none() {
		let mapping = MirrorActuatorMappingData {
			transformation_type: ActuatorTransformType::Unknown,
			actuator_length_offset: 0.0,
			actuator_length_scale: 1.0,
			mirror_angle_offset: 0.0,
			mirror_angle_scale: 1.0,
			actuator_angle_pairs: vec![],
			quadratic_model: None,
			angle_to_hall_code_error: None,
			hall_code_to_angle_error: None,
			hall_code_range: None,
		};
		assert!(hall_code_to_mirror_angle_deg(100, &mapping).is_none());
	}

	#[test]
	fn flip_x_mat_negates_second_row() {
		let m = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]];
		let f = flip_x_mat(m);
		assert_eq!(f[0], [1.0, 2.0, 3.0]);
		assert_eq!(f[1], [-4.0, -5.0, -6.0]);
		assert_eq!(f[2], [7.0, 8.0, 9.0]);
	}

	#[test]
	fn b1_hall_769_matches_near_lut_angle() {
		let mm = tests_support::b1_mirror_fixture();
		let mapping = mm.actuator_mapping.as_ref().unwrap();
		let angle = hall_code_to_mirror_angle_deg(769, mapping).unwrap();
		let ext = extrinsics_from_movable_mirror(&mm, 769).unwrap();
		assert!((angle - 44.75521).abs() < 0.02);
		assert!(ext.rotation.iter().all(|v| v.is_finite()));
		assert!(ext.translation[2].abs() < 50.0);
	}

	#[test]
	fn l16_00078_movable_modules_get_mirror_extrinsics() {
		let Some(data) = crate::fixtures::l16_00078_bytes() else {
			return;
		};
		let lri = crate::LriFile::decode(&data).expect("decode fixture");
		let mut with_rt = 0usize;
		for m in &lri.fusion.module_geometry {
			if m.mirror_type != Some(MirrorType::Movable) {
				continue;
			}
			let Some(hall) = lri.fusion.mirror_hall_codes.get(&m.camera).copied() else {
				continue;
			};
			let (_, mm) = movable_mirror_bundle(&m.focus_calibrations).unwrap();
			let ext = extrinsics_from_movable_mirror(mm, hall).expect("mirror extrinsics");
			assert!(ext.rotation.iter().all(|v| v.is_finite()));
			assert!(ext.translation.iter().all(|v| v.is_finite()));
			let sel = lri
				.fusion
				.pick_all_focus_bundles(lri.focal_length.unwrap_or(87))
				.into_iter()
				.find(|(c, _)| *c == m.camera)
				.map(|(_, s)| s)
				.expect("focus pick");
			assert!(
				sel.has_extrinsics,
				"{:?} should have mirror extrinsics",
				m.camera
			);
			with_rt += 1;
		}
		assert!(with_rt >= 3, "expected mirror extrinsics on shot modules");
	}
}

/// Shared mirror fixture for unit tests in `fusion` and `mirror_pose`.
#[cfg(test)]
pub(crate) mod tests_support {
	use super::*;
	use crate::fusion::{MirrorActuatorMappingData, MirrorSystemData, MovableMirrorData};

	pub fn b1_mirror_fixture() -> MovableMirrorData {
		MovableMirrorData {
			mirror_system: Some(MirrorSystemData {
				real_camera_location: [18.54517, 7.6582804, -3.4655511],
				real_camera_orientation: [
					-0.38093942,
					0.47482356,
					0.7933648,
					-0.49646932,
					0.61882627,
					-0.60874647,
					-0.7800022,
					-0.6257768,
					1.5680847e-8,
				],
				rotation_axis: [0.60439825, 0.7966334, -0.008826814],
				point_on_rotation_axis: [22.03876, 5.055312, 0.8291366],
				distance_mirror_plane_to_point_on_rotation_axis: 3.9343035,
				mirror_normal_at_zero_degrees: [0.79949564, -0.6006531, -0.0047436645],
				flip_img_around_x: false,
				mirror_angle_range: crate::fusion::Range2F {
					min: 35.5,
					max: 44.75,
				},
				reprojection_error: Some(0.35),
			}),
			actuator_mapping: Some(MirrorActuatorMappingData {
				transformation_type: ActuatorTransformType::MeanStdNormalize,
				actuator_length_offset: 547.6,
				actuator_length_scale: 175.66673,
				mirror_angle_offset: 40.150547,
				mirror_angle_scale: 3.6576087,
				actuator_angle_pairs: vec![],
				quadratic_model: None,
				angle_to_hall_code_error: None,
				hall_code_to_angle_error: None,
				hall_code_range: None,
			}),
		}
	}
}
