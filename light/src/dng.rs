use std::fs::File;
use std::io::{Seek, SeekFrom, Write};

use anyhow::{Context, Result};
use camino::Utf8Path;

pub fn write_dng(
	path: &Utf8Path,
	width: u32,
	height: u32,
	pixels: &[u16],
	cfa: Option<[u8; 4]>,
	black: u16,
	white: u16,
	color_matrix: Option<[f32; 9]>,
	camera_label: &str,
) -> Result<()> {
	assert_eq!(pixels.len(), (width as usize) * (height as usize));

	let strip: Vec<u8> = pixels.iter().flat_map(|p| p.to_le_bytes()).collect();

	let mut tags: Vec<Tag> = Vec::new();
	let photometric = if cfa.is_some() { 32_803 } else { 34_892 };

	tags.push(Tag::long(256, width));
	tags.push(Tag::long(257, height));
	tags.push(Tag::short(258, 16));
	tags.push(Tag::short(259, 1));
	tags.push(Tag::short(262, photometric));
	tags.push(Tag::short(277, 1));
	tags.push(Tag::long(278, height));
	tags.push(Tag::short(284, 1));
	tags.push(Tag::bytes(50706, &[1, 4, 0, 0]));
	tags.push(Tag::bytes(50707, &[1, 1, 0, 0]));

	let model = format!("Light L16 {camera_label}");
	tags.push(Tag::ascii(50708, &model));

	tags.push(Tag::short(50714, black));
	tags.push(Tag::long(50717, white as u32));

	if let Some(cfa) = cfa {
		tags.push(Tag::shorts(33421, &[2, 2]));
		tags.push(Tag::bytes(33422, cfa.as_slice()));
	}

	if let Some(matrix) = color_matrix {
		tags.push(Tag::srationals(50721, &matrix));
	}

	// Strip tags appended after layout knows strip offset
	let ifd_start: u32 = 8;
	let ifd_len = 2 + (tags.len() + 2) * 12 + 4; // +2 for strip tags
	let mut data_off = ifd_start + ifd_len as u32;

	let mut extra: Vec<u8> = Vec::new();
	let mut records: Vec<[u8; 12]> = Vec::new();

	for tag in &mut tags {
		let rec = tag.encode(&mut data_off, &mut extra);
		records.push(rec);
	}

	let strip_off = align4(data_off);

	records.push(encode_long(273, strip_off));
	records.push(encode_long(279, strip.len() as u32));

	let mut file = File::create(path).with_context(|| format!("create {path}"))?;
	file.write_all(b"II")?;
	file.write_all(&42u16.to_le_bytes())?;
	file.write_all(&ifd_start.to_le_bytes())?;
	file.write_all(&(records.len() as u16).to_le_bytes())?;
	for rec in &records {
		file.write_all(rec)?;
	}
	file.write_all(&0u32.to_le_bytes())?;
	file.write_all(&extra)?;
	file.seek(SeekFrom::Start(strip_off.into()))?;
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

fn encode_long(tag: u16, v: u32) -> [u8; 12] {
	let mut rec = [0u8; 12];
	rec[0..2].copy_from_slice(&tag.to_le_bytes());
	rec[2..4].copy_from_slice(&4u16.to_le_bytes());
	rec[4..8].copy_from_slice(&1u32.to_le_bytes());
	rec[8..12].copy_from_slice(&v.to_le_bytes());
	rec
}

enum TagData {
	Inline([u8; 4]),
	External(Vec<u8>),
}

struct Tag {
	id: u16,
	typ: u16,
	count: u32,
	data: TagData,
}

impl Tag {
	fn long(id: u16, v: u32) -> Self {
		Self {
			id,
			typ: 4,
			count: 1,
			data: TagData::Inline(v.to_le_bytes()),
		}
	}

	fn short(id: u16, v: u16) -> Self {
		let mut b = [0u8; 4];
		b[..2].copy_from_slice(&v.to_le_bytes());
		Self {
			id,
			typ: 3,
			count: 1,
			data: TagData::Inline(b),
		}
	}

	fn bytes(id: u16, data: &[u8]) -> Self {
		if data.len() <= 4 {
			let mut b = [0u8; 4];
			b[..data.len()].copy_from_slice(data);
			Self {
				id,
				typ: 1,
				count: data.len() as u32,
				data: TagData::Inline(b),
			}
		} else {
			Self {
				id,
				typ: 1,
				count: data.len() as u32,
				data: TagData::External(data.to_vec()),
			}
		}
	}

	fn shorts(id: u16, vals: &[u16]) -> Self {
		let mut v = Vec::new();
		for s in vals {
			v.extend_from_slice(&s.to_le_bytes());
		}
		Self {
			id,
			typ: 3,
			count: vals.len() as u32,
			data: TagData::External(v),
		}
	}

	fn ascii(id: u16, s: &str) -> Self {
		let mut v = s.as_bytes().to_vec();
		v.push(0);
		Self {
			id,
			typ: 2,
			count: v.len() as u32,
			data: TagData::External(v),
		}
	}

	fn srationals(id: u16, matrix: &[f32; 9]) -> Self {
		let mut v = Vec::with_capacity(9 * 8);
		for &f in matrix {
			let num = (f * 65_536.0).round() as i32;
			v.extend_from_slice(&num.to_le_bytes());
			v.extend_from_slice(&65_536i32.to_le_bytes());
		}
		Self {
			id,
			typ: 10,
			count: 9,
			data: TagData::External(v),
		}
	}

	fn encode(&mut self, cursor: &mut u32, extra: &mut Vec<u8>) -> [u8; 12] {
		let mut rec = [0u8; 12];
		rec[0..2].copy_from_slice(&self.id.to_le_bytes());
		rec[2..4].copy_from_slice(&self.typ.to_le_bytes());
		rec[4..8].copy_from_slice(&self.count.to_le_bytes());

		match &self.data {
			TagData::Inline(v) => rec[8..12].copy_from_slice(v),
			TagData::External(blob) => {
				*cursor = align4(*cursor);
				let off = *cursor;
				extra.extend_from_slice(blob);
				*cursor += blob.len() as u32;
				rec[8..12].copy_from_slice(&off.to_le_bytes());
			}
		}
		rec
	}
}

pub fn cfa_pattern(cfa: &str) -> Option<[u8; 4]> {
	match cfa {
		"RGGB" => Some([0, 1, 1, 2]),
		"BGGR" => Some([2, 1, 1, 0]),
		"GRBG" => Some([1, 0, 2, 1]),
		"GBRG" => Some([1, 2, 0, 1]),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn cfa_pattern_maps_bayer_layouts() {
		assert_eq!(cfa_pattern("RGGB"), Some([0, 1, 1, 2]));
		assert_eq!(cfa_pattern("BGGR"), Some([2, 1, 1, 0]));
		assert_eq!(cfa_pattern("UNKNOWN"), None);
	}

	#[test]
	fn align4_rounds_up() {
		assert_eq!(align4(0), 0);
		assert_eq!(align4(1), 4);
		assert_eq!(align4(4), 4);
		assert_eq!(align4(5), 8);
	}
}
