use std::time::Duration;

use block::{Block, BlockType, ExtractedData, Header};

mod bayer_jpeg;
mod block;
pub mod distortion;
mod error;
pub mod fixtures;
mod fusion;
mod mirror_pose;
pub mod stereo;
mod types;
pub mod unpack;
pub mod warp;

pub use distortion::{CraDistortion, ModuleDistortion, PolynomialDistortion};
pub use error::LriError;
pub use fusion::*;
pub use types::*;
pub use warp::CameraPose;

pub struct LriFile<'lri> {
	pub image_reference_camera: Option<CameraId>,
	pub images: Vec<RawImage<'lri>>,
	pub colors: Vec<ColorInfo>,
	pub camera_infos: Vec<CameraInfo>,
	pub sensor_data: Vec<SensorData>,

	pub focal_length: Option<i32>,
	pub firmware_version: Option<String>,
	pub image_integration_time: Option<Duration>,
	pub af_achieved: Option<bool>,
	pub image_gain: Option<f32>,
	pub hdr: Option<HdrMode>,
	pub scene: Option<SceneMode>,
	pub on_tripod: Option<bool>,
	pub awb: Option<AwbMode>,
	pub awb_gain: Option<AwbGain>,

	pub fusion: FusionMeta,
	pub view_output: ViewOutput,
}

impl<'lri> LriFile<'lri> {
	/// Parse an LRI file. Returns an error on truncated or malformed blocks.
	pub fn decode(data: &'lri [u8]) -> Result<Self, LriError> {
		let mut images = vec![];
		let mut colors = vec![];
		let mut camera_infos = vec![];
		let mut ext = ExtractedData::default();
		let mut cursor = data;

		while !cursor.is_empty() {
			let header = Header::ingest(cursor)?;
			let end = header.block_length;

			if end > cursor.len() {
				return Err(LriError::TruncatedBlock {
					need: end,
					have: cursor.len(),
				});
			}

			if header.kind == BlockType::Gps {
				cursor = &cursor[end..];
				continue;
			}

			let block = Block {
				header,
				data: &cursor[..end],
			};
			cursor = &cursor[end..];

			block.extract_meaningful_data(&mut ext, &mut images, &mut colors, &mut camera_infos)?;
		}

		for img in images.iter_mut() {
			if let Some(info) = camera_infos.iter().find(|i| i.camera == img.camera) {
				img.sensor = info.sensor;
			}

			img.color = colors
				.iter()
				.filter(|c| c.camera == img.camera)
				.cloned()
				.collect();
		}

		Ok(LriFile {
			image_reference_camera: ext.reference_camera,
			images,
			colors,
			camera_infos,
			sensor_data: ext.sensor_data,
			firmware_version: ext.fw_version,
			focal_length: ext.focal_length,
			image_integration_time: ext.image_integration_time,
			af_achieved: ext.af_achieved,
			image_gain: ext.image_gain,
			hdr: ext.hdr,
			scene: ext.scene,
			on_tripod: ext.on_tripod,
			awb: ext.awb,
			awb_gain: ext.awb_gain,
			fusion: ext.fusion,
			view_output: ext.view_output,
		})
	}

	pub fn image_count(&self) -> usize {
		self.images.len()
	}

	pub fn images(&self) -> std::slice::Iter<'_, RawImage<'_>> {
		self.images.iter()
	}

	pub fn reference_image(&self) -> Option<&RawImage<'lri>> {
		self.image_reference_camera
			.and_then(|irc| self.images.iter().find(|ri| ri.camera == irc))
	}

	/// Black/white levels for a sensor type, falling back to L16 AR1335 defaults.
	pub fn levels_for(&self, sensor: SensorModel) -> (u16, u16) {
		self.sensor_data
			.iter()
			.find(|sd| sd.sensor_type == sensor)
			.map(|sd| {
				(
					sd.characterization.black_level.round() as u16,
					sd.characterization.white_level.round() as u16,
				)
			})
			.unwrap_or((42, 1023))
	}
}

pub enum RawData<'img> {
	BayerJpeg {
		header: &'img [u8],
		format: u32,
		jpeg0: &'img [u8],
		jpeg1: &'img [u8],
		jpeg2: &'img [u8],
		jpeg3: &'img [u8],
	},
	Packed10bpp {
		data: &'img [u8],
	},
}

pub struct RawImage<'img> {
	pub camera: CameraId,
	pub sensor: SensorModel,
	pub width: usize,
	pub height: usize,
	pub format: DataFormat,
	pub data: RawData<'img>,
	pub sbro: (i32, i32),
	pub color: Vec<ColorInfo>,
}

impl<'img> RawImage<'img> {
	pub fn daylight(&self) -> Option<&ColorInfo> {
		self.color
			.iter()
			.find(|c| c.whitepoint == Whitepoint::F7)
			.or_else(|| self.color.iter().find(|c| c.whitepoint == Whitepoint::D65))
	}

	pub fn color_info(&self, whitepoint: Whitepoint) -> Option<&ColorInfo> {
		self.color.iter().find(|c| c.whitepoint == whitepoint)
	}

	pub fn cfa_string(&self) -> Option<&'static str> {
		match self.sensor {
			SensorModel::Ar1335Mono | SensorModel::Unknown => None,
			SensorModel::Ar1335 => self.cfa_string_ar1335(),
		}
	}

	fn cfa_string_ar1335(&self) -> Option<&'static str> {
		match self.sbro {
			(-1, -1) => None,
			(0, 0) => Some("BGGR"),
			(1, 0) => Some("GRBG"),
			(0, 1) => Some("GBRG"),
			(1, 1) => Some("RGGB"),
			_ => None,
		}
	}

	pub fn color_type(&self) -> ColorType {
		match self.sensor {
			SensorModel::Ar1335 => ColorType::Rgb,
			SensorModel::Ar1335Mono | SensorModel::Unknown => ColorType::Grayscale,
		}
	}

	/// Unpack packed 10 bpp RAW. Returns `None` for Bayer JPEG — use [`decode_pixels`](Self::decode_pixels).
	pub fn unpack(&self) -> Option<Vec<u16>> {
		if let RawData::Packed10bpp { data } = self.data {
			let count = self.width * self.height;
			let mut upack = vec![0; count];
			unpack::tenbit(data, count, &mut upack).ok()?;
			Some(upack)
		} else {
			None
		}
	}

	/// Decode sensor pixels for any supported RAW format.
	pub fn decode_pixels(&self) -> Result<Vec<u16>, LriError> {
		match self.data {
			RawData::Packed10bpp { data } => {
				let count = self.width * self.height;
				let mut upack = vec![0; count];
				unpack::tenbit(data, count, &mut upack)?;
				Ok(upack)
			}
			RawData::BayerJpeg { .. } => bayer_jpeg::decode(&self.data, self.width, self.height),
		}
	}

	/// Fast preview decode (single JPEG plane for Bayer JPEG modules).
	pub fn decode_preview(&self) -> Result<bayer_jpeg::PreviewPixels, LriError> {
		match self.data {
			RawData::Packed10bpp { data } => {
				let count = self.width * self.height;
				let mut upack = vec![0; count];
				unpack::tenbit(data, count, &mut upack)?;
				Ok(bayer_jpeg::PreviewPixels {
					data: upack,
					width: self.width,
					height: self.height,
				})
			}
			RawData::BayerJpeg { .. } => {
				bayer_jpeg::decode_preview(&self.data, self.width, self.height)
			}
		}
	}
}

pub enum ColorType {
	Rgb,
	Grayscale,
}

#[derive(Copy, Clone, Debug)]
pub struct ColorInfo {
	pub camera: CameraId,
	pub whitepoint: Whitepoint,
	pub forward_matrix: [f32; 9],
	pub color_matrix: [f32; 9],
	pub rg: f32,
	pub bg: f32,
}

#[derive(Copy, Clone, Debug)]
pub struct CameraInfo {
	pub camera: CameraId,
	pub sensor: SensorModel,
}
