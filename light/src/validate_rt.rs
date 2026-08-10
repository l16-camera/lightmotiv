use std::fs;

use anyhow::{Context, Result};
use camino::Utf8Path;
use image::{imageops::FilterType, GrayImage, ImageBuffer, Luma, Rgb, RgbImage};
use lri_rs::{CameraId, LriFile, SelectedFocusBundle};
use nalgebra::{Matrix3, Vector3};
use serde::{Deserialize, Serialize};

use crate::session::LriSession;
use crate::thumbnail;

#[derive(Debug, Serialize, Deserialize)]
pub struct ValidateSummary {
	pub reference_camera: String,
	pub preview_max_side: u32,
	pub canvas: [u32; 2],
	pub lumen_size: [u32; 2],
	pub blend_mae_vs_lumen: f64,
	pub blend_ncc_vs_lumen: f64,
	pub modules: Vec<ModuleValidate>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModuleValidate {
	pub camera: String,
	pub has_extrinsics: bool,
	pub has_movable_mirror: bool,
	pub overlap_pixels: u32,
	pub overlay_mae: Option<f64>,
	pub overlay_ncc: Option<f64>,
	pub lumen_mae: Option<f64>,
	pub lumen_ncc: Option<f64>,
}

pub fn run(
	lri_path: &Utf8Path,
	lumen_jpg: &Utf8Path,
	output: &Utf8Path,
	max_side: u32,
) -> Result<()> {
	let session = LriSession::open(lri_path)?;
	session.with_lri(|lri| run_decoded(lri, lumen_jpg, output, max_side))
}

fn run_decoded(
	lri: &LriFile<'_>,
	lumen_jpg: &Utf8Path,
	output: &Utf8Path,
	max_side: u32,
) -> Result<()> {
	if !output.exists() {
		fs::create_dir_all(output).context("create output directory")?;
	}

	let focal = lri.focal_length.context("missing focal length")?;
	let ref_cam = lri
		.image_reference_camera
		.context("missing reference camera")?;

	let present: std::collections::HashSet<CameraId> =
		lri.images.iter().map(|i| i.camera).collect();

	let picks: Vec<(CameraId, SelectedFocusBundle)> = lri
		.fusion
		.pick_all_focus_bundles(focal)
		.into_iter()
		.filter(|(c, s)| present.contains(c) && s.k_matrix.is_some())
		.collect();

	let ref_pick = picks
		.iter()
		.find(|(c, _)| *c == ref_cam)
		.map(|(_, s)| s)
		.context("reference camera missing from focus pick")?;

	let (ref_bytes, ref_w, ref_h, ref_step) =
		thumbnail::render_preview_gray(lri, ref_cam, max_side)?;
	let ref_img = bytes_to_gray(&ref_bytes, ref_w, ref_h);

	let ref_k = scaled_k(ref_pick.k_matrix.unwrap(), ref_step);
	let ref_r = mat3(ref_pick.rotation.unwrap_or(identity9()));
	let ref_t = vec3(ref_pick.translation.unwrap_or([0.0; 3]));

	let mut blend_acc = vec![0f64; (ref_w * ref_h) as usize];
	let mut blend_w = vec![0u32; (ref_w * ref_h) as usize];
	accumulate(&mut blend_acc, &mut blend_w, &ref_img, 1.0);

	let lumen = load_lumen_gray(lumen_jpg)?;
	let lumen_fit = resize_gray(&lumen, ref_w, ref_h);

	let overlay_dir = output.join("overlays");
	fs::create_dir_all(&overlay_dir).context("create overlays directory")?;

	let mut modules = Vec::new();

	for (camera, sel) in &picks {
		if *camera == ref_cam {
			modules.push(ModuleValidate {
				camera: camera.to_string(),
				has_extrinsics: sel.has_extrinsics,
				has_movable_mirror: sel.has_movable_mirror,
				overlap_pixels: ref_w * ref_h,
				overlay_mae: Some(0.0),
				overlay_ncc: Some(1.0),
				lumen_mae: Some(0.0),
				lumen_ncc: Some(1.0),
			});
			continue;
		}

		let Some(k_src) = sel.k_matrix else {
			continue;
		};
		let (src_bytes, src_w, src_h, src_step) =
			thumbnail::render_preview_gray(lri, *camera, max_side)?;
		let src_img = bytes_to_gray(&src_bytes, src_w, src_h);

		let k_src = scaled_k(k_src, src_step);
		let r_src = mat3(sel.rotation.unwrap_or(identity9()));
		let t_src = vec3(sel.translation.unwrap_or([0.0; 3]));

		let h = homography_infinity(&ref_k, &ref_r, &ref_t, &k_src, &r_src, &t_src);
		let warped = warp_inverse(&src_img, &h, ref_w, ref_h);
		accumulate_masked(&mut blend_acc, &mut blend_w, &warped);

		let (mae, ncc, overlap) = compare_overlap(&warped, &ref_img);
		let (lumen_mae, lumen_ncc, _) = compare_overlap(&warped, &lumen_fit);
		let overlay = blend_overlay(&ref_img, &warped, 0.45);
		let overlay_path = overlay_dir.join(format!("{camera}_on_{ref_cam}.png"));
		write_gray_png(&overlay_path, &overlay)?;

		modules.push(ModuleValidate {
			camera: camera.to_string(),
			has_extrinsics: sel.has_extrinsics,
			has_movable_mirror: sel.has_movable_mirror,
			overlap_pixels: overlap,
			overlay_mae: Some(mae),
			overlay_ncc: Some(ncc),
			lumen_mae: Some(lumen_mae),
			lumen_ncc: Some(lumen_ncc),
		});

		eprintln!(
			"  {camera} → {ref_cam}: overlap={overlap} ref_ncc={ncc:.4} lumen_ncc={lumen_ncc:.4} mir={}",
			sel.has_movable_mirror
		);
	}

	let our_blend = normalize_blend(&blend_acc, &blend_w, ref_w, ref_h);
	let diff = abs_diff(&our_blend, &lumen_fit);
	let (blend_mae, blend_ncc, _) = compare_overlap(&our_blend, &lumen_fit);

	write_gray_png(&output.join("our_blend.png"), &our_blend)?;
	write_gray_png(&output.join("lumen_resized.png"), &lumen_fit)?;
	write_gray_png(&output.join("diff.png"), &diff)?;
	write_side_by_side(&output.join("side_by_side.png"), &our_blend, &lumen_fit)?;

	let summary = ValidateSummary {
		reference_camera: ref_cam.to_string(),
		preview_max_side: max_side,
		canvas: [ref_w, ref_h],
		lumen_size: [lumen.0, lumen.1],
		blend_mae_vs_lumen: blend_mae,
		blend_ncc_vs_lumen: blend_ncc,
		modules,
	};

	let summary_path = output.join("validate.json");
	fs::write(&summary_path, serde_json::to_string_pretty(&summary)?)
		.context("write validate.json")?;

	eprintln!(
		"blend vs lumen: mae={blend_mae:.2} ncc={blend_ncc:.4} ({}x{} → {}x{})",
		lumen.0, lumen.1, ref_w, ref_h
	);
	eprintln!("wrote {summary_path}");

	Ok(())
}

fn identity9() -> [f32; 9] {
	[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
}

fn mat3(row_major: [f32; 9]) -> Matrix3<f64> {
	Matrix3::new(
		row_major[0] as f64,
		row_major[1] as f64,
		row_major[2] as f64,
		row_major[3] as f64,
		row_major[4] as f64,
		row_major[5] as f64,
		row_major[6] as f64,
		row_major[7] as f64,
		row_major[8] as f64,
	)
}

fn vec3(v: [f32; 3]) -> Vector3<f64> {
	Vector3::new(v[0] as f64, v[1] as f64, v[2] as f64)
}

fn scaled_k(k: [f32; 9], step: usize) -> Matrix3<f64> {
	let s = 1.0 / step as f64;
	let mut m = mat3(k);
	m[(0, 0)] *= s;
	m[(0, 2)] *= s;
	m[(1, 1)] *= s;
	m[(1, 2)] *= s;
	m
}

/// Planar homography at infinity: x_ref ~ K_ref (R_ref R_src^T) K_src^-1 x_src.
fn homography_infinity(
	k_ref: &Matrix3<f64>,
	r_ref: &Matrix3<f64>,
	_t_ref: &Vector3<f64>,
	k_src: &Matrix3<f64>,
	r_src: &Matrix3<f64>,
	_t_src: &Vector3<f64>,
) -> Matrix3<f64> {
	let _ = (_t_ref, _t_src);
	let r_rel = r_ref * r_src.transpose();
	let k_src_inv = k_src.try_inverse().expect("singular K");
	k_ref * r_rel * k_src_inv
}

fn bytes_to_gray(bytes: &[u8], w: u32, h: u32) -> GrayImage {
	ImageBuffer::from_raw(w, h, bytes.to_vec()).expect("gray buffer")
}

fn warp_inverse(src: &GrayImage, h: &Matrix3<f64>, out_w: u32, out_h: u32) -> GrayImage {
	let h_inv = h.try_inverse().expect("singular homography");
	let (sw, sh) = src.dimensions();
	let sw = sw as i32;
	let sh = sh as i32;

	ImageBuffer::from_fn(out_w, out_h, |x, y| {
		let p = h_inv * Vector3::new(x as f64, y as f64, 1.0);
		if p.z.abs() < 1e-9 {
			return Luma([0]);
		}
		let sx = (p.x / p.z) as f32;
		let sy = (p.y / p.z) as f32;
		Luma([sample_bilinear(src, sx, sy, sw, sh)])
	})
}

fn sample_bilinear(img: &GrayImage, x: f32, y: f32, w: i32, h: i32) -> u8 {
	if x < 0.0 || y < 0.0 || x >= (w - 1) as f32 || y >= (h - 1) as f32 {
		return 0;
	}
	let x0 = x.floor() as i32;
	let y0 = y.floor() as i32;
	let fx = x - x0 as f32;
	let fy = y - y0 as f32;
	let p00 = img.get_pixel(x0 as u32, y0 as u32)[0] as f32;
	let p10 = img.get_pixel((x0 + 1) as u32, y0 as u32)[0] as f32;
	let p01 = img.get_pixel(x0 as u32, (y0 + 1) as u32)[0] as f32;
	let p11 = img.get_pixel((x0 + 1) as u32, (y0 + 1) as u32)[0] as f32;
	let top = p00 * (1.0 - fx) + p10 * fx;
	let bot = p01 * (1.0 - fx) + p11 * fx;
	(top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8
}

fn accumulate(acc: &mut [f64], w: &mut [u32], img: &GrayImage, weight: f64) {
	for (i, px) in img.pixels().enumerate() {
		acc[i] += px[0] as f64 * weight;
		w[i] += 1;
	}
}

fn accumulate_masked(acc: &mut [f64], w: &mut [u32], img: &GrayImage) {
	for (i, px) in img.pixels().enumerate() {
		if px[0] == 0 {
			continue;
		}
		acc[i] += px[0] as f64;
		w[i] += 1;
	}
}

fn normalize_blend(acc: &[f64], weights: &[u32], width: u32, height: u32) -> GrayImage {
	let mut out = GrayImage::new(width, height);
	for (i, weight) in weights.iter().enumerate() {
		let v = if *weight > 0 {
			(acc[i] / *weight as f64).round().clamp(0.0, 255.0) as u8
		} else {
			0
		};
		let x = (i as u32) % width;
		let y = (i as u32) / width;
		out.put_pixel(x, y, Luma([v]));
	}
	out
}

fn compare_overlap(a: &GrayImage, b: &GrayImage) -> (f64, f64, u32) {
	assert_eq!(a.dimensions(), b.dimensions());
	let mut n = 0u32;
	let mut sum_a = 0f64;
	let mut sum_b = 0f64;
	let mut sum_aa = 0f64;
	let mut sum_bb = 0f64;
	let mut sum_ab = 0f64;
	let mut mae = 0f64;

	for (pa, pb) in a.pixels().zip(b.pixels()) {
		if pa[0] == 0 || pb[0] == 0 {
			continue;
		}
		let av = pa[0] as f64;
		let bv = pb[0] as f64;
		n += 1;
		mae += (av - bv).abs();
		sum_a += av;
		sum_b += bv;
		sum_aa += av * av;
		sum_bb += bv * bv;
		sum_ab += av * bv;
	}

	if n == 0 {
		return (f64::NAN, f64::NAN, 0);
	}

	let nf = n as f64;
	mae /= nf;
	let cov = sum_ab - sum_a * sum_b / nf;
	let var_a = sum_aa - sum_a * sum_a / nf;
	let var_b = sum_bb - sum_b * sum_b / nf;
	let ncc = if var_a > 1e-6 && var_b > 1e-6 {
		cov / (var_a.sqrt() * var_b.sqrt())
	} else {
		f64::NAN
	};
	(mae, ncc, n)
}

fn abs_diff(a: &GrayImage, b: &GrayImage) -> GrayImage {
	ImageBuffer::from_fn(a.width(), a.height(), |x, y| {
		let pa = a.get_pixel(x, y)[0];
		let pb = b.get_pixel(x, y)[0];
		if pa == 0 || pb == 0 {
			return Luma([0]);
		}
		Luma([pa.abs_diff(pb)])
	})
}

fn blend_overlay(base: &GrayImage, top: &GrayImage, alpha: f64) -> GrayImage {
	ImageBuffer::from_fn(base.width(), base.height(), |x, y| {
		let b = base.get_pixel(x, y)[0] as f64;
		let t = top.get_pixel(x, y)[0] as f64;
		if t == 0.0 {
			return Luma([b.round() as u8]);
		}
		Luma([(b * (1.0 - alpha) + t * alpha).round().clamp(0.0, 255.0) as u8])
	})
}

fn load_lumen_gray(path: &Utf8Path) -> Result<(u32, u32, Vec<u8>)> {
	let img = image::open(path)
		.with_context(|| format!("open {path}"))?
		.to_rgb8();
	let (w, h) = img.dimensions();
	let gray: Vec<u8> = img
		.pixels()
		.map(|p| {
			let [r, g, b] = p.0;
			(0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b)).round() as u8
		})
		.collect();
	Ok((w, h, gray))
}

fn resize_gray(lumen: &(u32, u32, Vec<u8>), w: u32, h: u32) -> GrayImage {
	let src = bytes_to_gray(&lumen.2, lumen.0, lumen.1);
	image::imageops::resize(&src, w, h, FilterType::Triangle)
}

fn write_gray_png(path: &Utf8Path, img: &GrayImage) -> Result<()> {
	img.save(path).with_context(|| format!("write {path}"))?;
	Ok(())
}

fn write_side_by_side(path: &Utf8Path, left: &GrayImage, right: &GrayImage) -> Result<()> {
	let (w, h) = left.dimensions();
	let mut rgb = RgbImage::new(w * 2, h);
	for y in 0..h {
		for x in 0..w {
			let l = left.get_pixel(x, y)[0];
			rgb.put_pixel(x, y, Rgb([l, l, l]));
			let r = right.get_pixel(x, y)[0];
			rgb.put_pixel(x + w, y, Rgb([r, r, r]));
		}
	}
	rgb.save(path).with_context(|| format!("write {path}"))?;
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn test_k() -> Matrix3<f64> {
		mat3([100.0, 0.0, 50.0, 0.0, 100.0, 40.0, 0.0, 0.0, 1.0])
	}

	fn solid_image(w: u32, h: u32, value: u8) -> GrayImage {
		ImageBuffer::from_fn(w, h, |_, _| Luma([value]))
	}

	#[test]
	fn homography_identity_for_same_camera() {
		let k = test_k();
		let r = Matrix3::identity();
		let t = Vector3::zeros();
		let h = homography_infinity(&k, &r, &t, &k, &r, &t);
		let p = h * Vector3::new(120.0, 80.0, 1.0);
		assert!((p.x / p.z - 120.0).abs() < 1e-6);
		assert!((p.y / p.z - 80.0).abs() < 1e-6);
	}

	#[test]
	fn scaled_k_scales_focal_and_principal_point() {
		let k = scaled_k([200.0, 0.0, 100.0, 0.0, 200.0, 80.0, 0.0, 0.0, 1.0], 4);
		assert!((k[(0, 0)] - 50.0).abs() < 1e-9);
		assert!((k[(0, 2)] - 25.0).abs() < 1e-9);
		assert!((k[(1, 1)] - 50.0).abs() < 1e-9);
		assert!((k[(1, 2)] - 20.0).abs() < 1e-9);
	}

	#[test]
	fn warp_identity_preserves_uniform_image() {
		let src = solid_image(8, 6, 128);
		let h = Matrix3::identity();
		let out = warp_inverse(&src, &h, 8, 6);
		// Bilinear sampling needs a 1px inset at the right/bottom edge.
		for y in 1..5 {
			for x in 1..7 {
				assert_eq!(out.get_pixel(x, y)[0], 128);
			}
		}
	}

	#[test]
	fn sample_bilinear_interpolates() {
		let mut img = GrayImage::new(2, 2);
		img.put_pixel(0, 0, Luma([0]));
		img.put_pixel(1, 0, Luma([100]));
		img.put_pixel(0, 1, Luma([200]));
		img.put_pixel(1, 1, Luma([255]));
		let v = sample_bilinear(&img, 0.5, 0.5, 2, 2);
		assert_eq!(v, 139);
	}

	#[test]
	fn compare_overlap_identical_images() {
		let a = ImageBuffer::from_fn(16, 16, |x, y| Luma([((x + y) % 64) as u8 + 32]));
		let (mae, ncc, n) = compare_overlap(&a, &a);
		assert_eq!(n, 256);
		assert!(mae.abs() < 1e-9);
		assert!((ncc - 1.0).abs() < 1e-9);
	}

	#[test]
	fn compare_overlap_empty_when_masked_out() {
		let a = solid_image(4, 4, 0);
		let b = solid_image(4, 4, 100);
		let (mae, ncc, n) = compare_overlap(&a, &b);
		assert_eq!(n, 0);
		assert!(mae.is_nan());
		assert!(ncc.is_nan());
	}

	#[test]
	fn normalize_blend_averages_accumulator() {
		let acc = vec![100.0, 200.0, 0.0, 400.0];
		let weights = vec![1, 2, 0, 4];
		let img = normalize_blend(&acc, &weights, 2, 2);
		assert_eq!(img.get_pixel(0, 0)[0], 100);
		assert_eq!(img.get_pixel(1, 0)[0], 100);
		assert_eq!(img.get_pixel(0, 1)[0], 0);
		assert_eq!(img.get_pixel(1, 1)[0], 100);
	}

	#[test]
	fn blend_overlay_respects_alpha() {
		let base = solid_image(2, 2, 100);
		let top = solid_image(2, 2, 200);
		let out = blend_overlay(&base, &top, 0.5);
		assert_eq!(out.get_pixel(0, 0)[0], 150);
	}

	#[test]
	fn abs_diff_zero_on_equal_nonzero_pixels() {
		let a = solid_image(3, 3, 77);
		let diff = abs_diff(&a, &a);
		assert_eq!(diff.get_pixel(1, 1)[0], 0);
	}

	#[test]
	fn l16_validate_end_to_end_when_fixtures_present() {
		let Some(lri_path) = lri_rs::fixtures::l16_00078_path() else {
			return;
		};
		let Some(lumen_path) = lri_rs::fixtures::l16_00078_lumen_jpg_path() else {
			return;
		};
		let tmp = std::env::temp_dir().join("light_validate_test");
		let _ = std::fs::remove_dir_all(&tmp);
		run(
			lri_path.as_path().try_into().expect("utf8"),
			lumen_path.as_path().try_into().expect("utf8"),
			tmp.as_path().try_into().expect("utf8"),
			256,
		)
		.expect("validate run");
		let summary_path = tmp.join("validate.json");
		let summary: ValidateSummary =
			serde_json::from_str(&std::fs::read_to_string(summary_path).unwrap()).unwrap();
		assert_eq!(summary.reference_camera, "A1");
		assert!(summary.blend_ncc_vs_lumen > 0.0);
		assert_eq!(summary.modules.len(), 10);
		assert!(summary.modules.iter().any(|m| m.has_movable_mirror));
		let b2 = summary
			.modules
			.iter()
			.find(|m| m.camera == "B2")
			.expect("B2");
		let b3 = summary
			.modules
			.iter()
			.find(|m| m.camera == "B3")
			.expect("B3");
		let b2_ncc = b2.lumen_ncc.expect("B2 lumen ncc");
		let b3_ncc = b3.lumen_ncc.expect("B3 lumen ncc");
		assert!(
			b2_ncc > 0.15,
			"B2 lumen_ncc should align after mirror flip fix, got {b2_ncc}"
		);
		assert!(
			b3_ncc > 0.15,
			"B3 lumen_ncc should align after mirror flip fix, got {b3_ncc}"
		);
	}
}
