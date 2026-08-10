use std::fs::File;
use std::io::{Seek, Write};

use anyhow::{Context, Result};
use camino::Utf8Path;

/// Write 16-bit grayscale TIFF (little-endian, single strip).
pub fn write_gray_tiff16(path: &Utf8Path, width: u32, height: u32, pixels: &[u16]) -> Result<()> {
	assert_eq!(pixels.len(), (width as usize) * (height as usize));
	let strip: Vec<u8> = pixels.iter().flat_map(|p| p.to_le_bytes()).collect();
	let strip_len = strip.len() as u32;

	let ifd_start: u32 = 8;
	let ifd_count: u16 = 9;
	let ifd_len = 2 + ifd_count as u32 * 12 + 4;
	let data_off = ifd_start + ifd_len;

	let extra: Vec<u8> = Vec::new();
	let mut records: Vec<[u8; 12]> = Vec::new();

	records.push(encode_short(256, width as u16));
	records.push(encode_short(257, height as u16));
	records.push(encode_short(258, 16)); // bits per sample
	records.push(encode_short(259, 1)); // compression: none
	records.push(encode_short(262, 1)); // photometric: min is black
	records.push(encode_short(277, 1)); // samples per pixel
	records.push(encode_short(278, height as u16)); // rows per strip

	let strip_off = align4(data_off + extra.len() as u32);
	records.push(encode_long(273, strip_off));
	records.push(encode_long(279, strip_len));

	let mut file = File::create(path).with_context(|| format!("create {path}"))?;
	file.write_all(b"II")?;
	file.write_all(&42u16.to_le_bytes())?;
	file.write_all(&ifd_start.to_le_bytes())?;
	file.write_all(&ifd_count.to_le_bytes())?;
	for rec in &records {
		file.write_all(rec)?;
	}
	file.write_all(&0u32.to_le_bytes())?;
	file.write_all(&extra)?;
	file.seek(std::io::SeekFrom::Start(strip_off.into()))?;
	file.write_all(&strip)?;
	Ok(())
}

fn align4(v: u32) -> u32 {
	if v % 4 == 0 {
		v
	} else {
		v + (4 - v % 4)
	}
}

fn encode_short(tag: u16, v: u16) -> [u8; 12] {
	let mut rec = [0u8; 12];
	rec[0..2].copy_from_slice(&tag.to_le_bytes());
	rec[2..4].copy_from_slice(&3u16.to_le_bytes());
	rec[4..8].copy_from_slice(&1u32.to_le_bytes());
	rec[8..10].copy_from_slice(&v.to_le_bytes());
	rec
}

fn encode_long(tag: u16, v: u32) -> [u8; 12] {
	let mut rec = [0u8; 12];
	rec[0..2].copy_from_slice(&tag.to_le_bytes());
	rec[2..4].copy_from_slice(&4u16.to_le_bytes());
	rec[4..8].copy_from_slice(&1u32.to_le_bytes());
	rec[8..12].copy_from_slice(&v.to_le_bytes());
	rec
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn write_minimal_tiff16() {
		let pixels: Vec<u16> = (0..16).map(|i| i * 100).collect();
		let path = std::env::temp_dir().join("luminat_test.tiff");
		write_gray_tiff16(path.as_path().try_into().unwrap(), 4, 4, &pixels).unwrap();
		let data = std::fs::read(&path).unwrap();
		assert_eq!(&data[0..2], b"II");
		assert!(data.len() > 80);
	}
}
