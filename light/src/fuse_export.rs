use image::{imageops, GrayImage, ImageBuffer};
use lri_rs::{Orientation, ViewOutput};

/// Scale 8-bit fused preview to 16-bit range for TIFF/DNG export.
pub fn gray8_to_u16(img: &GrayImage, black: u16, white: u16) -> Vec<u16> {
	let span = (white.saturating_sub(black)).max(1) as f32;
	img.pixels()
		.map(|p| {
			let n = p[0] as f32 / 255.0;
			(black as f32 + n * span).round() as u16
		})
		.collect()
}

pub fn crop_gray(img: &GrayImage, x: u32, y: u32, w: u32, h: u32) -> GrayImage {
	imageops::crop_imm(img, x, y, w, h).to_image()
}

pub fn apply_orientation(img: GrayImage, orientation: Option<Orientation>) -> GrayImage {
	let Some(o) = orientation else {
		return img;
	};
	match o {
		Orientation::Normal => img,
		Orientation::Rot90Cw => imageops::rotate90(&img),
		Orientation::Rot90Ccw => imageops::rotate270(&img),
		Orientation::Rot180 => imageops::rotate180(&img),
		Orientation::Vflip => imageops::flip_vertical(&img),
		Orientation::Hflip => imageops::flip_horizontal(&img),
		Orientation::Rot90CwVflip => imageops::flip_vertical(&imageops::rotate90(&img)),
		Orientation::Rot90CcwVflip => imageops::flip_vertical(&imageops::rotate270(&img)),
	}
}

/// Full Lumen canvas crop from view preferences, applied to fused `img`.
pub fn apply_view_output(img: GrayImage, view: &ViewOutput, canvas: (u32, u32)) -> GrayImage {
	let (x, y, w, h) = view.crop_rect_px(canvas);
	let cropped = if w == img.width() && h == img.height() && x == 0 && y == 0 {
		img
	} else {
		crop_gray(&img, x, y, w, h)
	};
	apply_orientation(cropped, view.orientation)
}

pub fn bytes_to_gray(bytes: &[u8], w: u32, h: u32) -> GrayImage {
	ImageBuffer::from_raw(w, h, bytes.to_vec()).expect("gray buffer")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn normalized_crop_full_frame() {
		let vo = ViewOutput {
			crop: Some(lri_rs::ViewCrop {
				start: [0.0, 0.0],
				size: [1.0, 1.0],
			}),
			..Default::default()
		};
		let (x, y, w, h) = vo.crop_rect_px(ViewOutput::LUMEN_CANVAS);
		assert_eq!((x, y, w, h), (0, 0, 10432, 7824));
	}

	#[test]
	fn orientation_rot90_changes_dimensions() {
		let img = GrayImage::new(4, 2);
		let out = apply_orientation(img, Some(Orientation::Rot90Cw));
		assert_eq!(out.dimensions(), (2, 4));
	}
}
