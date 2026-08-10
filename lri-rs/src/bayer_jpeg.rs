use zune_core::bytestream::ZCursor;

use crate::error::LriError;
use crate::RawData;

/// Decoded preview plane (one JPEG for colour modules).
pub struct PreviewPixels {
	pub data: Vec<u16>,
	pub width: usize,
	pub height: usize,
}

/// Decode Bayer JPEG payload into a full-resolution 10-bit-ish Bayer plane.
pub fn decode(data: &RawData<'_>, width: usize, height: usize) -> Result<Vec<u16>, LriError> {
	let count = width
		.checked_mul(height)
		.ok_or(LriError::PixelCountMismatch {
			expected: 0,
			got: 0,
		})?;

	match data {
		RawData::Packed10bpp { .. } => Err(LriError::UnsupportedFormat),
		RawData::BayerJpeg {
			format,
			jpeg0,
			jpeg1,
			jpeg2,
			jpeg3,
			..
		} => match *format {
			0 => decode_colour(count, width, height, jpeg0, jpeg1, jpeg2, jpeg3),
			1 => decode_mono(count, jpeg0),
			other => Err(LriError::BayerJpegDecode(format!(
				"unknown format_type {other}"
			))),
		},
	}
}

/// Fast grid preview: decode a single JPEG plane (¼ decode work for colour).
pub fn decode_preview(
	data: &RawData<'_>,
	width: usize,
	height: usize,
) -> Result<PreviewPixels, LriError> {
	match data {
		RawData::Packed10bpp { .. } => Err(LriError::UnsupportedFormat),
		RawData::BayerJpeg { format, jpeg0, .. } => match *format {
			1 => {
				let count = width
					.checked_mul(height)
					.ok_or(LriError::PixelCountMismatch {
						expected: 0,
						got: 0,
					})?;
				let mut gray = vec![0u8; count];
				decode_jpeg_into(jpeg0, &mut gray)?;
				Ok(PreviewPixels {
					data: gray.into_iter().map(promote_u8).collect(),
					width,
					height,
				})
			}
			0 => {
				let half_w = width / 2;
				let half_h = height / 2;
				let plane_len = half_w
					.checked_mul(half_h)
					.ok_or(LriError::PixelCountMismatch {
						expected: 0,
						got: 0,
					})?;
				let mut plane = vec![0u8; plane_len];
				decode_jpeg_into(jpeg0, &mut plane)?;
				Ok(PreviewPixels {
					data: plane.into_iter().map(promote_u8).collect(),
					width: half_w,
					height: half_h,
				})
			}
			other => Err(LriError::BayerJpegDecode(format!(
				"unknown format_type {other}"
			))),
		},
	}
}

fn decode_mono(count: usize, jpeg0: &[u8]) -> Result<Vec<u16>, LriError> {
	let mut gray = vec![0u8; count];
	decode_jpeg_into(jpeg0, &mut gray)?;
	Ok(gray.into_iter().map(promote_u8).collect())
}

fn decode_colour(
	count: usize,
	width: usize,
	height: usize,
	jpeg0: &[u8],
	jpeg1: &[u8],
	jpeg2: &[u8],
	jpeg3: &[u8],
) -> Result<Vec<u16>, LriError> {
	let mut bayered = vec![0u16; count];
	let plane_len = count / 4;
	let mut plane = vec![0u8; plane_len];
	let half_w = width / 2;

	let planes = [(jpeg0, 0usize), (jpeg1, 1), (jpeg2, 2), (jpeg3, 3)];
	for (jpeg, offset) in planes {
		decode_jpeg_into(jpeg, &mut plane)?;
		for (idx, sample) in plane.iter().enumerate() {
			let in_x = idx % half_w;
			let in_y = idx / half_w;
			let bayer_x = in_x * 2 + (offset % 2);
			let bayer_y = in_y * 2 + (offset / 2);
			let bayer_idx = bayer_y * width + bayer_x;
			if bayer_idx >= count {
				return Err(LriError::BayerJpegDecode(format!(
					"bayer index {bayer_idx} out of range for {width}x{height}"
				)));
			}
			bayered[bayer_idx] = promote_u8(*sample);
		}
	}

	Ok(bayered)
}

#[inline]
fn promote_u8(sample: u8) -> u16 {
	(sample as u16) << 2
}

fn decode_jpeg_into(jpeg: &[u8], out: &mut [u8]) -> Result<(), LriError> {
	zune_jpeg::JpegDecoder::new(ZCursor::new(jpeg))
		.decode_into(out)
		.map_err(|e| LriError::BayerJpegDecode(e.to_string()))
}
