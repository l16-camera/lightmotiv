use std::collections::HashMap;

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use lri_rs::{CameraId, LriFile, RawImage};
use png::{BitDepth, ColorType, Encoder};
use rayon::prelude::*;

use crate::render::rotate_180;
use crate::threads;

const MAX_SIDE: u32 = 160;

/// Grayscale preview pixels plus subsample step (for scaling intrinsics).
pub fn render_preview_gray(
	lri: &LriFile<'_>,
	camera: CameraId,
	max_side: u32,
) -> Result<(Vec<u8>, u32, u32, usize)> {
	let img = lri
		.images
		.iter()
		.find(|i| i.camera == camera)
		.context("camera not in file")?;
	let (black, white) = lri.levels_for(img.sensor);
	let range = (white - black).max(1) as f32;

	let preview = img.decode_preview().context("decode preview")?;
	let step = subsample_step(preview.width, preview.height, max_side);
	let (mut bayer, sw, sh) = subsample(&preview.data, preview.width, preview.height, step);
	rotate_180(&mut bayer, 1);

	let bytes: Vec<u8> = bayer
		.iter()
		.map(|p| {
			let n = (*p).saturating_sub(black) as f32 / range;
			(n * 255.0).clamp(0.0, 255.0) as u8
		})
		.collect();

	Ok((bytes, sw as u32, sh as u32, step))
}

pub fn render_camera_png(lri: &LriFile<'_>, camera: CameraId) -> Result<Vec<u8>> {
	let img = lri
		.images
		.iter()
		.find(|i| i.camera == camera)
		.context("camera not in file")?;
	render_thumbnail_fast(img, lri)
}

pub fn thumbnail_base64(lri: &LriFile<'_>, camera: CameraId) -> Result<String> {
	let png = render_camera_png(lri, camera)?;
	Ok(format!("data:image/png;base64,{}", STANDARD.encode(png)))
}

pub fn thumbnails_batch(
	lri: &LriFile<'_>,
	cameras: &[CameraId],
	jobs: Option<usize>,
) -> Result<HashMap<String, String>> {
	let n = threads::export_jobs(jobs);
	let pool = rayon::ThreadPoolBuilder::new()
		.num_threads(n)
		.build()
		.context("thumbnail thread pool")?;

	pool.install(|| {
		cameras
			.par_iter()
			.map(|&camera| {
				let data_url = thumbnail_base64(lri, camera)?;
				Ok((camera.to_string(), data_url))
			})
			.collect::<Result<HashMap<_, _>>>()
	})
}

pub fn parse_camera_id(name: &str) -> Option<CameraId> {
	use CameraId::*;
	match name {
		"A1" => Some(A1),
		"A2" => Some(A2),
		"A3" => Some(A3),
		"A4" => Some(A4),
		"A5" => Some(A5),
		"B1" => Some(B1),
		"B2" => Some(B2),
		"B3" => Some(B3),
		"B4" => Some(B4),
		"B5" => Some(B5),
		"C1" => Some(C1),
		"C2" => Some(C2),
		"C3" => Some(C3),
		"C4" => Some(C4),
		"C5" => Some(C5),
		"C6" => Some(C6),
		_ => None,
	}
}

/// Grid preview: subsampled Bayer as grayscale — no debayer.
pub(crate) fn render_thumbnail_fast(img: &RawImage<'_>, lri: &LriFile<'_>) -> Result<Vec<u8>> {
	let (bytes, w, h, _) = render_preview_gray(lri, img.camera, MAX_SIDE)?;
	encode_png(w, h, &bytes, ColorType::Grayscale)
}

fn subsample_step(width: usize, height: usize, max_side: u32) -> usize {
	let max_dim = width.max(height) as u32;
	((max_dim + max_side - 1) / max_side).max(1) as usize
}

fn subsample(data: &[u16], width: usize, height: usize, step: usize) -> (Vec<u16>, usize, usize) {
	let sw = width.div_ceil(step);
	let sh = height.div_ceil(step);
	let mut out = Vec::with_capacity(sw * sh);
	for y in (0..height).step_by(step) {
		for x in (0..width).step_by(step) {
			out.push(data[y * width + x]);
		}
	}
	(out, sw, sh)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parse_camera_id_covers_all_modules() {
		for (name, cam) in [
			("A1", CameraId::A1),
			("A5", CameraId::A5),
			("B3", CameraId::B3),
			("C6", CameraId::C6),
		] {
			assert_eq!(parse_camera_id(name), Some(cam));
		}
		assert_eq!(parse_camera_id("Z9"), None);
		assert_eq!(parse_camera_id(""), None);
	}

	#[test]
	fn subsample_step_never_zero() {
		assert_eq!(subsample_step(4160, 3120, 160), 26);
		assert_eq!(subsample_step(100, 100, 1024), 1);
		assert_eq!(subsample_step(2000, 1000, 1024), 2);
	}

	#[test]
	fn subsample_picks_top_left_of_each_cell() {
		let data: Vec<u16> = (0..36).map(|i| i as u16).collect();
		let (out, w, h) = subsample(&data, 6, 6, 2);
		assert_eq!((w, h), (3, 3));
		assert_eq!(out, vec![0, 2, 4, 12, 14, 16, 24, 26, 28]);
	}

	#[test]
	fn l16_preview_dimensions_match_step() {
		let Some(bytes) = lri_rs::fixtures::l16_00078_bytes() else {
			return;
		};
		let lri = LriFile::decode(&bytes).expect("decode");
		let (pixels, w, h, step) = render_preview_gray(&lri, CameraId::A1, 1024).unwrap();
		assert_eq!(pixels.len(), (w * h) as usize);
		assert!(w <= 1024);
		assert!(h <= 1024);
		assert!(step >= 1);
	}
}

fn encode_png(width: u32, height: u32, data: &[u8], color: ColorType) -> Result<Vec<u8>> {
	let mut buf = Vec::new();
	{
		let mut enc = Encoder::new(&mut buf, width, height);
		enc.set_color(color);
		enc.set_depth(BitDepth::Eight);
		let mut writer = enc.write_header().context("png header")?;
		writer.write_image_data(data).context("png data")?;
	}
	Ok(buf)
}
