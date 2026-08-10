use crate::error::LriError;

const TEN_MASK: u64 = 1023;
const CHUNK_BATCH: usize = 8;

pub fn tenbit(packd: &[u8], count: usize, upack: &mut [u16]) -> Result<(), LriError> {
	let required_len = count.saturating_mul(10).div_ceil(8);

	if count > upack.len() {
		return Err(LriError::PixelCountMismatch {
			expected: count,
			got: upack.len(),
		});
	}

	if required_len > packd.len() {
		return Err(LriError::PixelCountMismatch {
			expected: required_len,
			got: packd.len(),
		});
	}

	tenbit_chunks(packd, required_len, upack);

	if let Some(tail) = tenbit_tail(packd, count, required_len) {
		upack[count - tail.len()..count].copy_from_slice(&tail);
	}

	Ok(())
}

fn tenbit_chunks(packd: &[u8], required_len: usize, upack: &mut [u16]) {
	let full_chunks = required_len / 5;
	let mut idx = 0;

	while idx + CHUNK_BATCH <= full_chunks {
		for i in 0..CHUNK_BATCH {
			let chunk_idx = idx + i;
			let start = required_len - (chunk_idx + 1) * 5;
			write_pixels(upack, chunk_idx * 4, &packd[start..start + 5]);
		}
		idx += CHUNK_BATCH;
	}

	while idx < full_chunks {
		let start = required_len - (idx + 1) * 5;
		write_pixels(upack, idx * 4, &packd[start..start + 5]);
		idx += 1;
	}
}

fn tenbit_tail(packd: &[u8], count: usize, required_len: usize) -> Option<Vec<u16>> {
	let remain = required_len % 5;
	if remain == 0 {
		return None;
	}

	let mut long_bytes = [0u8; 8];
	long_bytes[..remain].copy_from_slice(&packd[..remain]);
	let long = u64::from_le_bytes(long_bytes);

	let count_remain = count % 4;
	if count_remain == 0 {
		return None;
	}

	let mut tail = vec![0u16; count_remain];
	for idx in 0..count_remain {
		tail[idx] = ((long >> (10 * idx)) & TEN_MASK) as u16;
	}
	Some(tail)
}

#[inline(always)]
fn write_pixels(out: &mut [u16], offset: usize, chunk: &[u8]) {
	let long = u64::from_be_bytes([
		0x00, 0x00, 0x00, chunk[0], chunk[1], chunk[2], chunk[3], chunk[4],
	]);
	out[offset] = ((long >> 30) & TEN_MASK) as u16;
	out[offset + 1] = ((long >> 20) & TEN_MASK) as u16;
	out[offset + 2] = ((long >> 10) & TEN_MASK) as u16;
	out[offset + 3] = (long & TEN_MASK) as u16;
}

#[cfg(test)]
mod tests {
	use super::*;

	fn reference_tenbit(packd: &[u8], count: usize, upack: &mut [u16]) {
		let required_len = count.saturating_mul(10).div_ceil(8);
		let full_chunks = required_len / 5;
		let remain = required_len % 5;

		for idx in 0..full_chunks {
			let start = required_len - (idx + 1) * 5;
			write_pixels(upack, idx * 4, &packd[start..start + 5]);
		}

		if remain > 0 {
			let tail = tenbit_tail(packd, count, required_len).unwrap();
			upack[count - tail.len()..count].copy_from_slice(&tail);
		}
	}

	#[test]
	fn matches_reference_various_sizes() {
		for &count in &[4usize, 7, 16, 100, 1024, 4160 * 3120] {
			let required = count.saturating_mul(10).div_ceil(8);
			let packd: Vec<u8> = (0..required).map(|i| (i % 251) as u8).collect();
			let mut fast = vec![0u16; count];
			let mut slow = vec![0u16; count];
			tenbit(&packd, count, &mut fast).unwrap();
			reference_tenbit(&packd, count, &mut slow);
			assert_eq!(fast, slow, "count={count}");
		}
	}
}
