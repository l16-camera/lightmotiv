//! Coarse depth estimation (plane sweep MVP before full SGM).

use crate::warp::{homography_at_depth, CameraPose};

/// Scan frontal plane depths; `score(depth_mm)` should return higher for better alignment.
pub fn plane_sweep<F>(
	depth_min_mm: f64,
	depth_max_mm: f64,
	steps: usize,
	mut score: F,
) -> (f64, f64)
where
	F: FnMut(f64) -> f64,
{
	assert!(steps >= 2);
	let mut best_z = depth_min_mm;
	let mut best = f64::NEG_INFINITY;
	for i in 0..steps {
		let t = i as f64 / (steps - 1) as f64;
		let z = depth_min_mm + t * (depth_max_mm - depth_min_mm);
		let s = score(z);
		if s > best {
			best = s;
			best_z = z;
		}
	}
	(best_z, best)
}

/// Homography for warping `src` into `dst` at plane depth `depth_mm`.
pub fn warp_homography(
	src: &CameraPose,
	dst: &CameraPose,
	depth_mm: f64,
) -> nalgebra::Matrix3<f64> {
	homography_at_depth(src, dst, depth_mm)
}

/// Zero-mean normalized cross-correlation on overlapping non-zero pixels.
pub fn ncc_overlap(a: &[u8], b: &[u8]) -> f64 {
	assert_eq!(a.len(), b.len());
	let mut n = 0u32;
	let mut sum_a = 0f64;
	let mut sum_b = 0f64;
	let mut sum_aa = 0f64;
	let mut sum_bb = 0f64;
	let mut sum_ab = 0f64;
	for (pa, pb) in a.iter().zip(b.iter()) {
		if *pa == 0 || *pb == 0 {
			continue;
		}
		let av = *pa as f64;
		let bv = *pb as f64;
		n += 1;
		sum_a += av;
		sum_b += bv;
		sum_aa += av * av;
		sum_bb += bv * bv;
		sum_ab += av * bv;
	}
	if n == 0 {
		return f64::NAN;
	}
	let nf = n as f64;
	let cov = sum_ab - sum_a * sum_b / nf;
	let var_a = sum_aa - sum_a * sum_a / nf;
	let var_b = sum_bb - sum_b * sum_b / nf;
	if var_a < 1e-6 || var_b < 1e-6 {
		return f64::NAN;
	}
	cov / (var_a.sqrt() * var_b.sqrt())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn plane_sweep_picks_peak() {
		let (z, s) = plane_sweep(0.0, 100.0, 101, |d| -(d - 42.0).powi(2) as f64);
		assert!((z - 42.0).abs() < 1.0);
		assert!(s >= -1.0);
	}

	#[test]
	fn ncc_identical_vectors() {
		let v = vec![10u8, 20, 30, 40];
		let n = ncc_overlap(&v, &v);
		assert!((n - 1.0).abs() < 1e-9);
	}
}
