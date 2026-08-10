use std::fs;

use anyhow::{Context, Result};
use camino::Utf8Path;
use image::{imageops::FilterType, GrayImage, ImageBuffer, Luma};
use lri_rs::{
	distortion::undistort_module_gray,
	stereo::{ncc_overlap, plane_sweep, warp_homography},
	CameraId, CameraPose, LriFile, ModuleDistortion, SelectedFocusBundle,
};
use nalgebra::Matrix3;
use serde::{Deserialize, Serialize};

use crate::dng;
use crate::fuse_export::{apply_view_output, gray8_to_u16};
use crate::session::LriSession;
use crate::thumbnail;
use crate::tiff_out;
use lri_rs::ViewOutput;

#[derive(Debug, Serialize, Deserialize)]
pub struct FuseSummary {
	pub producer: String,
	pub reference_camera: String,
	pub depth_plane_mm: f64,
	pub depth_sweep_score: f64,
	pub depth_min_mm: f64,
	pub depth_max_mm: f64,
	pub tof_range_m: Option<f32>,
	pub infinity_ncc_vs_lumen: Option<f64>,
	pub depth_ncc_vs_lumen: Option<f64>,
	pub modules_warped: usize,
	pub preview_max_side: u32,
	pub full_res: bool,
	pub canvas: [u32; 2],
	pub crop_rect: [u32; 4],
	pub exports: Vec<String>,
}

pub fn run(
	lri_path: &Utf8Path,
	output: &Utf8Path,
	lumen_jpg: Option<&Utf8Path>,
	max_side: u32,
	full_res: bool,
	export_tiff: bool,
	export_dng: bool,
	depth_min_mm: f64,
	depth_max_mm: f64,
	depth_steps: usize,
) -> Result<FuseSummary> {
	run_with_progress(
		lri_path,
		output,
		lumen_jpg,
		max_side,
		full_res,
		export_tiff,
		export_dng,
		depth_min_mm,
		depth_max_mm,
		depth_steps,
		|_, _, _| {},
	)
}

pub fn run_with_progress(
	lri_path: &Utf8Path,
	output: &Utf8Path,
	lumen_jpg: Option<&Utf8Path>,
	max_side: u32,
	full_res: bool,
	export_tiff: bool,
	export_dng: bool,
	depth_min_mm: f64,
	depth_max_mm: f64,
	depth_steps: usize,
	on_progress: impl Fn(&str, usize, usize) + Send + Sync,
) -> Result<FuseSummary> {
	let session = LriSession::open(lri_path)?;
	session.with_lri(|lri| {
		run_decoded(
			lri,
			output,
			lumen_jpg,
			max_side,
			full_res,
			export_tiff,
			export_dng,
			depth_min_mm,
			depth_max_mm,
			depth_steps,
			&on_progress,
		)
	})
}

fn run_decoded(
	lri: &LriFile<'_>,
	output: &Utf8Path,
	lumen_jpg: Option<&Utf8Path>,
	max_side: u32,
	full_res: bool,
	export_tiff: bool,
	export_dng: bool,
	depth_min_mm: f64,
	depth_max_mm: f64,
	depth_steps: usize,
	on_progress: &(impl Fn(&str, usize, usize) + Send + Sync),
) -> Result<FuseSummary> {
	on_progress("prepare", 0, 1);
	let canvas = ViewOutput::LUMEN_CANVAS;
	let fuse_max_side = if full_res {
		canvas.0.max(canvas.1)
	} else {
		max_side
	};
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
		.filter(|(c, s)| present.contains(c) && s.k_matrix.is_some() && s.has_extrinsics)
		.collect();

	let ref_module = lri
		.fusion
		.module_geometry
		.iter()
		.find(|m| m.camera == ref_cam)
		.context("reference geometry")?;

	let ref_pick = picks
		.iter()
		.find(|(c, _)| *c == ref_cam)
		.map(|(_, s)| s)
		.context("reference pick")?;

	let (ref_bytes, ref_w, ref_h, ref_step) =
		thumbnail::render_preview_gray(lri, ref_cam, fuse_max_side)?;
	let ref_pose = pose_from_pick(ref_pick, ref_step);

	let ref_undist = undistort_preview(&ref_bytes, ref_w, ref_h, &ref_module.distortion)?;
	let ref_img = bytes_to_gray(&ref_undist, ref_w, ref_h);

	// Plane sweep on first non-ref module (tele baseline for depth).
	let tele = picks
		.iter()
		.find(|(c, _)| *c != ref_cam)
		.context("need tele module for depth sweep")?;
	let tele_module = lri
		.fusion
		.module_geometry
		.iter()
		.find(|m| m.camera == tele.0)
		.context("tele geometry")?;

	let (tele_bytes, tw, th, tele_step) =
		thumbnail::render_preview_gray(lri, tele.0, fuse_max_side)?;
	let tele_pose = pose_from_pick(&tele.1, tele_step);
	let tele_undist = undistort_preview(&tele_bytes, tw, th, &tele_module.distortion)?;
	let tele_img = bytes_to_gray(&tele_undist, tw, th);

	let tof_range_m = lri.fusion.tof_range_m.filter(|t| *t > 0.0);
	let (sweep_min, sweep_max) = depth_range_from_tof(tof_range_m, depth_min_mm, depth_max_mm);
	if let Some(tof) = tof_range_m {
		eprintln!("tof seed {tof:.2}m → sweep {sweep_min:.0}–{sweep_max:.0}mm");
	}

	on_progress("prepare", 1, 1);
	on_progress("depth", 0, 1);

	let (best_depth, best_score) = plane_sweep(sweep_min, sweep_max, depth_steps, |z| {
		let h = warp_homography(&tele_pose, &ref_pose, z);
		let warped = warp_gray(&tele_img, &h, ref_w, ref_h);
		match ncc_overlap(ref_img.as_raw(), warped.as_raw()) {
			v if v.is_nan() => f64::NEG_INFINITY,
			v => v,
		}
	});

	eprintln!(
		"depth sweep ({sweep_min:.0}→{sweep_max:.0}): best={best_depth:.0}mm score={best_score:.4}"
	);
	on_progress("depth", 1, 1);

	let lumen_fit = lumen_jpg.map(|path| {
		let lumen = load_lumen_gray(path)?;
		Ok::<_, anyhow::Error>(resize_gray(&lumen, ref_w, ref_h))
	});
	let lumen_fit = match lumen_fit {
		Some(Ok(img)) => Some(img),
		Some(Err(e)) => return Err(e),
		None => None,
	};

	let h_inf = warp_homography(&tele_pose, &ref_pose, 1.0e9);
	let tele_warp_inf = warp_gray(&tele_img, &h_inf, ref_w, ref_h);
	let infinity_ncc = lumen_fit
		.as_ref()
		.map(|l| ncc_overlap(tele_warp_inf.as_raw(), l.as_raw()));

	let mut blend_acc = vec![0f64; (ref_w * ref_h) as usize];
	let mut blend_w = vec![0u32; (ref_w * ref_h) as usize];
	accumulate(&mut blend_acc, &mut blend_w, &ref_img, 1.0);

	let warp_total = picks.iter().filter(|(c, _)| *c != ref_cam).count();
	let mut warped_count = 0usize;
	for (camera, sel) in &picks {
		if *camera == ref_cam {
			continue;
		}
		on_progress("warp", warped_count, warp_total);
		let (bytes, sw, sh, step) = thumbnail::render_preview_gray(lri, *camera, fuse_max_side)?;
		let module = lri
			.fusion
			.module_geometry
			.iter()
			.find(|m| m.camera == *camera)
			.context("module geometry")?;
		let src_pose = pose_from_pick(sel, step);
		let undist = undistort_preview(&bytes, sw, sh, &module.distortion)?;
		let src_img = bytes_to_gray(&undist, sw, sh);
		let h = warp_homography(&src_pose, &ref_pose, best_depth);
		let warped = warp_gray(&src_img, &h, ref_w, ref_h);
		accumulate_masked(&mut blend_acc, &mut blend_w, &warped);
		warped_count += 1;
	}
	on_progress("warp", warp_total, warp_total);

	on_progress("blend", 0, 1);
	let fused = normalize_blend(&blend_acc, &blend_w, ref_w, ref_h);
	on_progress("blend", 1, 1);

	let canvas_img = if full_res && (fused.width(), fused.height()) != canvas {
		image::imageops::resize(&fused, canvas.0, canvas.1, FilterType::Triangle)
	} else {
		fused.clone()
	};
	let cropped = apply_view_output(canvas_img, &lri.view_output, canvas);
	let (cx, cy, cw, ch) = lri.view_output.crop_rect_px(canvas);

	on_progress("export", 0, 1);

	let fused_path = output.join("fused.png");
	fused.save(&fused_path).context("write fused.png")?;

	let mut exports = vec!["fused.png".to_string()];
	if full_res {
		let cropped_path = output.join("fused_cropped.png");
		cropped
			.save(&cropped_path)
			.context("write fused_cropped.png")?;
		exports.push("fused_cropped.png".to_string());
	}

	let ref_img_meta = lri
		.images
		.iter()
		.find(|i| i.camera == ref_cam)
		.context("reference image")?;
	let (black, white) = lri.levels_for(ref_img_meta.sensor);
	let u16 = gray8_to_u16(&cropped, black, white);

	if full_res && export_tiff {
		let path = output.join("fused.tiff");
		tiff_out::write_gray_tiff16(&path, cropped.width(), cropped.height(), &u16)?;
		exports.push("fused.tiff".to_string());
		eprintln!("wrote {path} ({}×{})", cropped.width(), cropped.height());
	}
	if full_res && export_dng {
		let path = output.join("fused.dng");
		dng::write_dng(
			&path,
			cropped.width(),
			cropped.height(),
			&u16,
			None,
			black,
			white,
			None,
			"Luminat-fused",
		)?;
		exports.push("fused.dng".to_string());
		eprintln!("wrote {path}");
	}

	let depth_ncc = lumen_fit
		.as_ref()
		.map(|l| ncc_overlap(fused.as_raw(), l.as_raw()));

	if let Some(lumen) = lumen_fit.as_ref() {
		let diff = abs_diff(&fused, lumen);
		diff.save(output.join("diff.png")).ok();
		lumen
			.save(output.join("lumen_resized.png"))
			.context("write lumen_resized.png")?;
	}

	let summary = FuseSummary {
		producer: "Luminat".to_string(),
		reference_camera: ref_cam.to_string(),
		depth_plane_mm: best_depth,
		depth_sweep_score: best_score,
		depth_min_mm: sweep_min,
		depth_max_mm: sweep_max,
		tof_range_m,
		infinity_ncc_vs_lumen: infinity_ncc,
		depth_ncc_vs_lumen: depth_ncc,
		modules_warped: warped_count + 1,
		preview_max_side: fuse_max_side,
		full_res,
		canvas: [canvas.0, canvas.1],
		crop_rect: [cx, cy, cw, ch],
		exports,
	};

	fs::write(
		output.join("fuse.json"),
		serde_json::to_string_pretty(&summary)?,
	)?;
	on_progress("export", 1, 1);

	eprintln!("fused {warped_count}+1 modules @ {best_depth:.0}mm → {fused_path}");
	if let Some(ncc) = depth_ncc {
		eprintln!("fused vs lumen ncc={ncc:.4}");
	}

	Ok(summary)
}

/// When ToF reports a positive range (metres), centre the sweep around it.
fn depth_range_from_tof(
	tof_range_m: Option<f32>,
	depth_min_mm: f64,
	depth_max_mm: f64,
) -> (f64, f64) {
	let Some(tof_m) = tof_range_m.filter(|t| *t > 0.0) else {
		return (depth_min_mm, depth_max_mm);
	};
	let center_mm = f64::from(tof_m) * 1000.0;
	let half_span = (depth_max_mm - depth_min_mm) * 0.5;
	((center_mm - half_span).max(500.0), center_mm + half_span)
}

fn pose_from_pick(sel: &SelectedFocusBundle, step: usize) -> CameraPose {
	let k = sel.k_matrix.expect("k");
	let r = sel
		.rotation
		.unwrap_or([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
	let t = sel.translation.unwrap_or([0.0, 0.0, 0.0]);
	CameraPose::from_row_major(k, r, t).scaled(step)
}

fn undistort_preview(
	bytes: &[u8],
	w: u32,
	h: u32,
	distortion: &ModuleDistortion,
) -> Result<Vec<u8>> {
	if distortion.has_polynomial() || distortion.has_cra() {
		Ok(undistort_module_gray(bytes, w, h, distortion))
	} else {
		Ok(bytes.to_vec())
	}
}

fn bytes_to_gray(bytes: &[u8], w: u32, h: u32) -> GrayImage {
	ImageBuffer::from_raw(w, h, bytes.to_vec()).expect("gray")
}

fn warp_gray(src: &GrayImage, h: &Matrix3<f64>, out_w: u32, out_h: u32) -> GrayImage {
	let h_inv = h.try_inverse().expect("singular H");
	let (sw, sh) = src.dimensions();
	let sw = sw as i32;
	let sh = sh as i32;
	ImageBuffer::from_fn(out_w, out_h, |x, y| {
		let p = h_inv * nalgebra::Vector3::new(x as f64, y as f64, 1.0);
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn depth_range_from_tof_centres_on_metric_hint() {
		let (min, max) = depth_range_from_tof(Some(3.0), 1500.0, 8000.0);
		assert!((min - 500.0).abs() < 1.0);
		assert!((max - 6250.0).abs() < 1.0);
		let (min, max) = depth_range_from_tof(None, 1500.0, 8000.0);
		assert_eq!(min, 1500.0);
		assert_eq!(max, 8000.0);
	}

	#[test]
	fn l16_fuse_end_to_end_when_fixtures_present() {
		let Some(lri_path) = lri_rs::fixtures::l16_00078_path() else {
			return;
		};
		let Some(lumen_path) = lri_rs::fixtures::l16_00078_lumen_jpg_path() else {
			return;
		};
		let tmp = std::env::temp_dir().join("light_fuse_test");
		let _ = std::fs::remove_dir_all(&tmp);
		run(
			lri_path.as_path().try_into().expect("utf8"),
			tmp.as_path().try_into().expect("utf8"),
			Some(lumen_path.as_path().try_into().expect("utf8")),
			256,
			false,
			false,
			false,
			1500.0,
			8000.0,
			11,
		)
		.expect("fuse run");
		let json = std::fs::read_to_string(tmp.join("fuse.json")).unwrap();
		let summary: FuseSummary = serde_json::from_str(&json).unwrap();
		assert_eq!(summary.reference_camera, "A1");
		assert_eq!(summary.modules_warped, 10);
		let ncc = summary.depth_ncc_vs_lumen.expect("depth ncc");
		assert!(
			ncc > 0.45,
			"fused ncc should beat polynomial-only baseline, got {ncc}"
		);
		assert!(tmp.join("fused.png").exists());
	}
}
