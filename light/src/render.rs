use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use lri_rs::{LriFile, RawImage, Whitepoint};

use crate::dng::{self, cfa_pattern};
use crate::mono;

pub fn export_dng(img: &RawImage<'_>, lri: &LriFile<'_>, path: Utf8PathBuf) -> Result<()> {
	let (black, white) = lri.levels_for(img.sensor);
	let label = mono::dng_camera_label(img);

	eprintln!(
		"{} {:?} [{}:{}] {}x{} {} (levels {black}/{white}){}",
		img.camera,
		img.sensor,
		img.sbro.0,
		img.sbro.1,
		img.width,
		img.height,
		img.format,
		if mono::is_mono_image(img) {
			" [MONO]"
		} else {
			""
		}
	);

	let mut bayer = img.decode_pixels().context("decode sensor pixels")?;
	rotate_180(&mut bayer, 1);

	// Mono: no CFA — DNG photometric LinearRaw / CFA-less
	let cfa = if mono::is_mono_image(img) {
		None
	} else {
		img.cfa_string().and_then(cfa_pattern)
	};
	let color_matrix = if mono::is_mono_image(img) {
		None
	} else {
		img.color_info(Whitepoint::D65).map(|c| c.forward_matrix)
	};

	eprintln!("  write {path}");
	dng::write_dng(
		&path,
		img.width as u32,
		img.height as u32,
		&bayer,
		cfa,
		black,
		white,
		color_matrix,
		&label,
	)
}

pub fn rotate_180<T: Copy>(data: &mut [T], channels: usize) {
	if channels == 1 {
		data.reverse();
		return;
	}

	let pixels = data.len() / channels;
	let mut tmp = vec![data[0]; data.len()];
	for (dst, src) in (0..pixels)
		.map(|i| i * channels)
		.zip((0..pixels).rev().map(|i| i * channels))
	{
		for c in 0..channels {
			tmp[dst + c] = data[src + c];
		}
	}
	data.copy_from_slice(&tmp);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rotate_180_single_channel() {
		let mut data = vec![1u8, 2, 3, 4];
		rotate_180(&mut data, 1);
		assert_eq!(data, vec![4, 3, 2, 1]);
	}

	#[test]
	fn rotate_180_rgb_pixels() {
		let mut data = vec![
			1, 10, 100, //
			2, 20, 200, //
			3, 30, 300, //
		];
		rotate_180(&mut data, 3);
		assert_eq!(data, vec![3, 30, 300, 2, 20, 200, 1, 10, 100]);
	}
}
