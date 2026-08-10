//! Lens distortion from `ltpb.Distortion` (polynomial + CRA / `lt::LensUndistortCRA`).

#[derive(Clone, Debug, PartialEq)]
pub struct PolynomialDistortion {
	pub center: [f32; 2],
	pub normalization: [f32; 2],
	pub coeffs: Vec<f32>,
}

impl PolynomialDistortion {
	/// Normalized coords from pixel `(u, v)`.
	pub fn pixel_to_normalized(&self, u: f32, v: f32) -> (f32, f32) {
		let nx = self.normalization[0].max(1e-6);
		let ny = self.normalization[1].max(1e-6);
		((u - self.center[0]) / nx, (v - self.center[1]) / ny)
	}

	/// Pixel coords from normalized (distorted) coords.
	pub fn normalized_to_pixel(&self, xn: f32, yn: f32) -> (f32, f32) {
		(
			xn * self.normalization[0] + self.center[0],
			yn * self.normalization[1] + self.center[1],
		)
	}

	/// Forward: undistorted normalized → distorted normalized.
	pub fn distort_normalized(&self, xn: f32, yn: f32) -> (f32, f32) {
		let r2 = xn * xn + yn * yn;
		let mut scale = 1.0f32;
		let mut rp = r2;
		for &k in &self.coeffs {
			scale += k * rp;
			rp *= r2;
		}
		(xn * scale, yn * scale)
	}

	/// Inverse: distorted normalized → undistorted normalized (fixed-point).
	pub fn undistort_normalized(&self, xd: f32, yd: f32) -> (f32, f32) {
		let mut xn = xd;
		let mut yn = yd;
		for _ in 0..12 {
			let (fx, fy) = self.distort_normalized(xn, yn);
			xn += xd - fx;
			yn += yd - fy;
		}
		(xn, yn)
	}

	/// Undistorted source pixel for output `(u, v)` (backward map).
	pub fn undistort_pixel(&self, u: f32, v: f32) -> (f32, f32) {
		let (xd, yd) = self.pixel_to_normalized(u, v);
		let (xn, yn) = self.undistort_normalized(xd, yd);
		self.normalized_to_pixel(xn, yn)
	}
}

/// Chief-ray-angle undistortion (`ltpb.Distortion.CRA` → `lt::LensUndistortCRA`).
///
/// Calibration layout (L16):
/// - `cra_profile[101]`: measured θ vs field radius (reference)
/// - `corr_lut[30]`: runtime radius(mm) → additive θ correction (rad)
#[derive(Clone, Debug, PartialEq)]
pub struct CraDistortion {
	pub center: [f32; 2],
	pub sensor_distance: f32,
	pub exit_pupil_distance: f32,
	pub pixel_size: f32,
	pub cra_profile: Vec<[f32; 2]>,
	pub corr_lut: Vec<[f32; 2]>,
	pub lens_hall_code: Option<f32>,
	pub distance_hall_ratio: Option<f32>,
}

impl CraDistortion {
	/// Interpolate correction angle (radians) at radius `r_mm`.
	pub fn corr_angle_at(&self, r_mm: f32) -> f32 {
		interp_lut_y(&self.corr_lut, r_mm)
	}

	/// Backward CRA map: ideal pixel → distorted source pixel.
	///
	/// `corr_lut` stores radius(mm) → radial correction δr(mm) from factory fit.
	pub fn undistort_pixel(&self, u: f32, v: f32) -> (f32, f32) {
		let dx = u - self.center[0];
		let dy = v - self.center[1];
		let r_px = (dx * dx + dy * dy).sqrt();
		if r_px < 1e-3 {
			return (u, v);
		}

		let r_mm = r_px * self.pixel_size;
		let delta_mm = self.corr_angle_at(r_mm);
		let r_src_mm = (r_mm + delta_mm).max(0.0);
		let scale = (r_src_mm / r_mm.max(1e-6)).clamp(0.5, 2.0);

		(self.center[0] + dx * scale, self.center[1] + dy * scale)
	}
}

fn interp_lut_y(lut: &[[f32; 2]], x: f32) -> f32 {
	if lut.is_empty() {
		return 0.0;
	}
	if x <= lut[0][0] {
		return lut[0][1];
	}
	if let Some(last) = lut.last() {
		if x >= last[0] {
			return last[1];
		}
	}
	for w in lut.windows(2) {
		let (x0, y0) = (w[0][0], w[0][1]);
		let (x1, y1) = (w[1][0], w[1][1]);
		if x >= x0 && x <= x1 {
			let t = (x - x0) / (x1 - x0).max(1e-6);
			return y0 * (1.0 - t) + y1 * t;
		}
	}
	lut.last().map(|p| p[1]).unwrap_or(0.0)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModuleDistortion {
	pub polynomial: Option<PolynomialDistortion>,
	pub cra: Option<CraDistortion>,
}

impl ModuleDistortion {
	pub fn has_polynomial(&self) -> bool {
		self.polynomial.is_some()
	}

	pub fn has_cra(&self) -> bool {
		self.cra.is_some()
	}

	pub fn poly_coeffs(&self) -> usize {
		self.polynomial
			.as_ref()
			.map(|p| p.coeffs.len())
			.unwrap_or(0)
	}

	/// Lumen order: CRA (`LensUndistortCRA`) then polynomial.
	pub fn undistort_pixel(&self, u: f32, v: f32) -> (f32, f32) {
		let (mut x, mut y) = if let Some(cra) = self.cra.as_ref() {
			cra.undistort_pixel(u, v)
		} else {
			(u, v)
		};
		if let Some(poly) = self.polynomial.as_ref() {
			(x, y) = poly.undistort_pixel(x, y);
		}
		(x, y)
	}
}

/// Remap with full module distortion (CRA + polynomial).
pub fn undistort_module_gray(
	src: &[u8],
	width: u32,
	height: u32,
	model: &ModuleDistortion,
) -> Vec<u8> {
	let w = width as i32;
	let h = height as i32;
	let mut out = vec![0u8; src.len()];
	for y in 0..height {
		for x in 0..width {
			let (su, sv) = model.undistort_pixel(x as f32, y as f32);
			let i = (y * width + x) as usize;
			out[i] = sample_bilinear_u8(src, w, h, su, sv);
		}
	}
	out
}

/// Remap with polynomial only (legacy helper).
pub fn undistort_gray(
	src: &[u8],
	width: u32,
	height: u32,
	model: &PolynomialDistortion,
) -> Vec<u8> {
	undistort_module_gray(
		src,
		width,
		height,
		&ModuleDistortion {
			polynomial: Some(model.clone()),
			cra: None,
		},
	)
}

fn sample_bilinear_u8(src: &[u8], w: i32, h: i32, x: f32, y: f32) -> u8 {
	if x < 0.0 || y < 0.0 || x >= (w - 1) as f32 || y >= (h - 1) as f32 {
		return 0;
	}
	let x0 = x.floor() as i32;
	let y0 = y.floor() as i32;
	let fx = x - x0 as f32;
	let fy = y - y0 as f32;
	let idx = |px: i32, py: i32| (py * w + px) as usize;
	let p00 = src[idx(x0, y0)] as f32;
	let p10 = src[idx(x0 + 1, y0)] as f32;
	let p01 = src[idx(x0, y0 + 1)] as f32;
	let p11 = src[idx(x0 + 1, y0 + 1)] as f32;
	let top = p00 * (1.0 - fx) + p10 * fx;
	let bot = p01 * (1.0 - fx) + p11 * fx;
	(top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn roundtrip_distort_undistort_normalized() {
		let m = PolynomialDistortion {
			center: [100.0, 100.0],
			normalization: [2000.0, 2000.0],
			coeffs: vec![0.01, -0.002, 0.0, 0.0, 0.0],
		};
		let (xd, yd) = m.distort_normalized(0.1, -0.05);
		let (xn, yn) = m.undistort_normalized(xd, yd);
		assert!((xn - 0.1).abs() < 1e-3);
		assert!((yn + 0.05).abs() < 1e-3);
	}

	#[test]
	fn cra_corr_lut_interpolates() {
		let lut = vec![[0.0, 0.0], [1.0, 0.1], [2.0, 0.2]];
		assert!((interp_lut_y(&lut, 0.5) - 0.05).abs() < 1e-6);
		assert!((interp_lut_y(&lut, 2.5) - 0.2).abs() < 1e-6);
	}

	#[test]
	fn cra_center_pixel_is_fixed_point() {
		let cra = CraDistortion {
			center: [100.0, 200.0],
			sensor_distance: 3.7,
			exit_pupil_distance: 2.5,
			pixel_size: 0.0011,
			cra_profile: vec![],
			corr_lut: vec![[0.0, 0.0], [1.0, -0.01]],
			lens_hall_code: None,
			distance_hall_ratio: None,
		};
		let (u, v) = cra.undistort_pixel(100.0, 200.0);
		assert!((u - 100.0).abs() < 1e-3);
		assert!((v - 200.0).abs() < 1e-3);
	}

	#[test]
	fn l16_a1_cra_extracted() {
		let Some(bytes) = crate::fixtures::l16_00078_bytes() else {
			return;
		};
		let lri = crate::LriFile::decode(&bytes).expect("decode");
		let a1 = lri
			.fusion
			.module_geometry
			.iter()
			.find(|m| m.camera == crate::CameraId::A1)
			.expect("A1");
		let cra = a1.distortion.cra.as_ref().expect("A1 CRA");
		assert_eq!(cra.cra_profile.len(), 101);
		assert_eq!(cra.corr_lut.len(), 30);
		assert!(cra.sensor_distance > 1.0);
		assert!(cra.exit_pupil_distance > 1.0);
		assert!(cra.pixel_size > 0.0);
		// LUT radius keys are monotonic 0..~3 mm.
		for w in cra.corr_lut.windows(2) {
			assert!(w[1][0] > w[0][0]);
		}
	}

	#[test]
	fn l16_a1_polynomial_extracted() {
		let Some(bytes) = crate::fixtures::l16_00078_bytes() else {
			return;
		};
		let lri = crate::LriFile::decode(&bytes).expect("decode");
		let a1 = lri
			.fusion
			.module_geometry
			.iter()
			.find(|m| m.camera == crate::CameraId::A1)
			.expect("A1");
		let poly = a1.distortion.polynomial.as_ref().expect("A1 polynomial");
		assert!(
			poly.coeffs.len() >= 3,
			"expected >=3 coeffs, got {}",
			poly.coeffs.len()
		);
		assert!(poly.normalization[0] > 100.0);
	}
}
