use std::time::Duration;

use lri_proto::{
	gps_data::GPSData, lightheader::LightHeader, matrix3x3f::Matrix3x3F,
	view_preferences::ViewPreferences, Message as PbMessage,
};

use crate::{
	error::LriError, fusion, AwbGain, AwbMode, CameraId, CameraInfo, ColorInfo, DataFormat,
	FusionMeta, HdrMode, RawData, RawImage, SceneMode, SensorData, SensorModel, ViewCrop,
	ViewOutput,
};

pub(crate) struct Block<'lri> {
	pub header: Header,
	/// Includes the 32-byte header.
	pub data: &'lri [u8],
}

impl<'lri> Block<'lri> {
	pub fn message_data(&self) -> &[u8] {
		let end = self.header.message_offset + self.header.message_length;
		&self.data[self.header.message_offset..end]
	}

	pub fn message(&self) -> Result<Message, LriError> {
		match self.header.kind {
			BlockType::LightHeader => LightHeader::parse_from_bytes(self.message_data())
				.map(Message::LightHeader)
				.map_err(|e| LriError::ProtobufParse(e.to_string())),
			BlockType::ViewPreferences => ViewPreferences::parse_from_bytes(self.message_data())
				.map(Message::ViewPreferences)
				.map_err(|e| LriError::ProtobufParse(e.to_string())),
			BlockType::Gps => GPSData::parse_from_bytes(self.message_data())
				.map(|_| Message::Gps(()))
				.map_err(|e| LriError::ProtobufParse(e.to_string())),
		}
	}

	pub fn extract_meaningful_data(
		&self,
		ext: &mut ExtractedData,
		images: &mut Vec<RawImage<'lri>>,
		colors: &mut Vec<ColorInfo>,
		infos: &mut Vec<CameraInfo>,
	) -> Result<(), LriError> {
		match self.message()? {
			Message::ViewPreferences(vp) => {
				self.extract_view(vp, ext);
			}
			Message::Gps(_) => {}
			Message::LightHeader(lh) => {
				self.extract_light_header(lh, ext, images, colors, infos)?;
			}
		}
		Ok(())
	}

	fn extract_light_header(
		&self,
		lh: LightHeader,
		ext: &mut ExtractedData,
		images: &mut Vec<RawImage<'lri>>,
		colors: &mut Vec<ColorInfo>,
		infos: &mut Vec<CameraInfo>,
	) -> Result<(), LriError> {
		let LightHeader {
			mut hw_info,
			module_calibration,
			modules,
			image_reference_camera,
			device_fw_version,
			image_focal_length,
			mut af_info,
			mut view_preferences,
			sensor_data,
			device_calibration,
			tof_range,
			imu_data,
			gps_data,
			..
		} = lh;

		if let Some(hw_info) = hw_info.take() {
			for info in hw_info.camera {
				infos.push(CameraInfo {
					camera: info.id().into(),
					sensor: info.sensor().into(),
				});
			}
		}

		if let Some(vp) = view_preferences.take() {
			self.extract_view(vp, ext);
		}

		for mcal in module_calibration {
			let camera = mcal.camera_id().into();

			if let Some(geo) = fusion::extract_module_geometry(&mcal) {
				ext.fusion.module_geometry.push(geo);
			}

			for mut color in mcal.color {
				let whitepoint = color.type_().into();
				let forward_matrix = match color.forward_matrix.take() {
					Some(fw) => Self::deconstruct_matrix3x3(fw),
					None => continue,
				};
				let color_matrix = match color.color_matrix.take() {
					None => [0.0; 9],
					Some(cm) => Self::deconstruct_matrix3x3(cm),
				};

				colors.push(ColorInfo {
					camera,
					whitepoint,
					forward_matrix,
					color_matrix,
					rg: color.rg_ratio(),
					bg: color.bg_ratio(),
				});
			}
		}

		for mut module in modules {
			let camera = module.id().into();
			if let Some(hall) = mirror_hall_from_module(&module) {
				ext.fusion.mirror_hall_codes.insert(camera, hall);
			}
			let mut surface = match module.sensor_data_surface.take() {
				Some(sur) => sur,
				None => continue,
			};

			let size = surface
				.size
				.take()
				.ok_or_else(|| LriError::ProtobufParse("missing surface size".into()))?;
			let width = size.x() as usize;
			let height = size.y() as usize;

			let offset = surface.data_offset() as usize;
			let data_length = surface.row_stride() as usize * height;

			let format = surface.format().into();
			let image_data = match format {
				DataFormat::BayerJpeg => {
					if offset + 24 > self.data.len() {
						return Err(LriError::TruncatedBlock {
							need: offset + 24,
							have: self.data.len(),
						});
					}

					const BJPG_HEADER_LEN: usize = 1576;
					let mut wrk = &self.data[offset..];

					let format_type = u32::from_le_bytes(wrk[4..8].try_into().unwrap());
					let jpeg0_len = u32::from_le_bytes(wrk[8..12].try_into().unwrap()) as usize;
					let jpeg1_len = u32::from_le_bytes(wrk[12..16].try_into().unwrap()) as usize;
					let jpeg2_len = u32::from_le_bytes(wrk[16..20].try_into().unwrap()) as usize;
					let jpeg3_len = u32::from_le_bytes(wrk[20..24].try_into().unwrap()) as usize;

					let mut advance = |len: usize| -> Result<&[u8], LriError> {
						if len > wrk.len() {
							return Err(LriError::TruncatedBlock {
								need: len,
								have: wrk.len(),
							});
						}
						let data = &wrk[..len];
						wrk = &wrk[len..];
						Ok(data)
					};

					let header = advance(BJPG_HEADER_LEN)?;
					let jpeg0 = advance(jpeg0_len)?;

					match format_type {
						1 => RawData::BayerJpeg {
							header,
							format: format_type,
							jpeg0,
							jpeg1: &[],
							jpeg2: &[],
							jpeg3: &[],
						},
						0 => RawData::BayerJpeg {
							header,
							format: format_type,
							jpeg0,
							jpeg1: advance(jpeg1_len)?,
							jpeg2: advance(jpeg2_len)?,
							jpeg3: advance(jpeg3_len)?,
						},
						other => {
							return Err(LriError::BayerJpegDecode(format!(
								"unknown format_type {other}"
							)));
						}
					}
				}
				DataFormat::Packed10bpp => {
					let end = offset + data_length;
					if end > self.data.len() {
						return Err(LriError::TruncatedBlock {
							need: end,
							have: self.data.len(),
						});
					}
					RawData::Packed10bpp {
						data: &self.data[offset..end],
					}
				}
			};

			let sbro = module
				.sensor_bayer_red_override
				.as_ref()
				.map(|p| (p.x(), p.y()))
				.unwrap_or((-1, -1));

			images.push(RawImage {
				camera,
				sensor: SensorModel::Unknown,
				width,
				height,
				format,
				data: image_data,
				sbro,
				color: vec![],
			});
		}

		if let Some(Ok(irc)) = image_reference_camera.map(|ev| ev.enum_value()) {
			ext.reference_camera = Some(irc.into());
		}

		if let Some(afd) = af_info.take() {
			ext.af_achieved.get_or_insert(afd.focus_achieved());
		}

		if let Some(fwv) = device_fw_version {
			ext.fw_version.get_or_insert(fwv);
		}

		if let Some(x) = image_focal_length {
			ext.focal_length.get_or_insert(x);
		}

		for sd in sensor_data {
			ext.sensor_data.push(sd.into());
		}

		if let Some(range) = tof_range {
			ext.fusion.tof_range_m.get_or_insert(range);
		}

		if let Some(dc) = device_calibration.into_option() {
			if let Some(tof) = fusion::tof_from_device(dc) {
				ext.fusion.tof_calibration = Some(tof);
			}
		}

		if !imu_data.is_empty() {
			ext.fusion.imu = Some(fusion::imu_summary(&imu_data));
		}

		if let Some(gps) = gps_data.into_option() {
			ext.fusion.gps = fusion::gps_from_proto(gps);
		}

		Ok(())
	}

	#[rustfmt::skip]
	fn deconstruct_matrix3x3(mat: Matrix3x3F) -> [f32; 9] {
		[
			mat.x00(), mat.x01(), mat.x02(),
			mat.x10(), mat.x11(), mat.x12(),
			mat.x20(), mat.x21(), mat.x22(),
		]
	}

	fn extract_view(&self, vp: ViewPreferences, ext: &mut ExtractedData) {
		let ViewPreferences {
			image_integration_time_ns,
			image_gain,
			hdr_mode,
			scene_mode,
			is_on_tripod,
			awb_mode,
			awb_gains,
			disable_cropping,
			orientation,
			aspect_ratio,
			crop,
			..
		} = vp;

		if let Some(ns) = image_integration_time_ns {
			ext.image_integration_time = Some(Duration::from_nanos(ns));
		}

		if let Some(g) = image_gain {
			ext.image_gain.get_or_insert(g);
		}

		if let Some(Ok(h)) = hdr_mode.map(|ev| ev.enum_value()) {
			ext.hdr = Some(h.into());
		}

		if let Some(Ok(h)) = scene_mode.map(|ev| ev.enum_value()) {
			ext.scene = Some(h.into());
		}

		if let Some(tri) = is_on_tripod {
			ext.on_tripod = Some(tri);
		}

		if let Some(Ok(awbmode)) = awb_mode.map(|ev| ev.enum_value()) {
			ext.awb = Some(awbmode.into());
		}

		if let Some(gain) = awb_gains.into_option() {
			ext.awb_gain = Some(gain.into());
		}

		if let Some(disable) = disable_cropping {
			ext.view_output.disable_cropping = disable;
		}
		if let Some(Ok(o)) = orientation.map(|v| v.enum_value()) {
			ext.view_output.orientation = Some(o.into());
		}
		if let Some(Ok(ar)) = aspect_ratio.map(|v| v.enum_value()) {
			ext.view_output.aspect_ratio = Some(ar.into());
		}
		if let Some(c) = crop.into_option() {
			if let (Some(start), Some(size)) = (c.start.into_option(), c.size.into_option()) {
				ext.view_output.crop = Some(ViewCrop {
					start: [start.x.unwrap_or(0.0), start.y.unwrap_or(0.0)],
					size: [size.x.unwrap_or(0.0), size.y.unwrap_or(0.0)],
				});
			}
		}
	}
}

#[derive(Debug, Default)]
pub(crate) struct ExtractedData {
	pub reference_camera: Option<CameraId>,
	pub fw_version: Option<String>,
	pub focal_length: Option<i32>,

	pub image_gain: Option<f32>,
	pub image_integration_time: Option<Duration>,
	pub af_achieved: Option<bool>,
	pub hdr: Option<HdrMode>,
	pub scene: Option<SceneMode>,
	pub on_tripod: Option<bool>,

	pub awb: Option<AwbMode>,
	pub awb_gain: Option<AwbGain>,

	pub sensor_data: Vec<SensorData>,

	pub fusion: FusionMeta,
	pub view_output: ViewOutput,
}

pub enum Message {
	LightHeader(LightHeader),
	ViewPreferences(ViewPreferences),
	Gps(()),
}

pub struct Header {
	pub block_length: usize,
	pub message_offset: usize,
	pub message_length: usize,
	pub kind: BlockType,
}

impl Header {
	pub fn ingest(data: &[u8]) -> Result<Self, LriError> {
		if data.len() < 32 {
			return Err(LriError::TruncatedBlock {
				need: 32,
				have: data.len(),
			});
		}

		if &data[0..4] != b"LELR" {
			return Err(LriError::InvalidMagic);
		}

		let combined_length = u64::from_le_bytes(data[4..12].try_into().unwrap()) as usize;
		let message_offset = u64::from_le_bytes(data[12..20].try_into().unwrap()) as usize;
		let message_length = u32::from_le_bytes(data[20..24].try_into().unwrap()) as usize;

		let kind = match data[24] {
			0 => BlockType::LightHeader,
			1 => BlockType::ViewPreferences,
			2 => BlockType::Gps,
			t => return Err(LriError::UnknownBlockType(t)),
		};

		Ok(Header {
			block_length: combined_length,
			message_offset,
			message_length,
			kind,
		})
	}
}

#[derive(PartialEq, Eq)]
pub(crate) enum BlockType {
	LightHeader,
	ViewPreferences,
	Gps,
}

fn mirror_hall_from_module(module: &lri_proto::camera_module::CameraModule) -> Option<i32> {
	module
		.af_info
		.as_ref()
		.and_then(|af| af.mirror_position)
		.or(module.mirror_position)
}

#[cfg(test)]
mod tests {
	use super::*;
	use lri_proto::{lightheader::LightHeader, Message as PbMessage};

	#[test]
	fn l16_view_crop_extracted_when_fixture_present() {
		let Some(bytes) = crate::fixtures::l16_00078_bytes() else {
			return;
		};
		let lri = crate::LriFile::decode(&bytes).expect("decode");
		let vo = &lri.view_output;
		eprintln!(
			"view_output: disable_cropping={} orientation={:?} crop={:?}",
			vo.disable_cropping, vo.orientation, vo.crop
		);
		// L16_00078 Lumen export is 10432×7824; crop should match when present.
		if vo.crop.is_some() {
			let (x, y, w, h) = vo.crop_rect_px(ViewOutput::LUMEN_CANVAS);
			eprintln!("crop px on lumen canvas: {x},{y} {w}x{h}");
			assert_eq!((x, y, w, h), (0, 0, 10432, 7824));
		}
	}

	fn all_light_headers(bytes: &[u8]) -> Vec<LightHeader> {
		let mut out = Vec::new();
		let mut cursor = bytes;
		while !cursor.is_empty() {
			let Ok(header) = Header::ingest(cursor) else {
				break;
			};
			let end = header.block_length.min(cursor.len());
			if header.kind == BlockType::LightHeader {
				let block = Block {
					header,
					data: &cursor[..end],
				};
				if let Ok(lh) = LightHeader::parse_from_bytes(block.message_data()) {
					out.push(lh);
				}
			}
			cursor = &cursor[end..];
		}
		out
	}

	#[test]
	fn l16_mirror_hall_lives_in_af_info_for_tele_modules() {
		let Some(bytes) = crate::fixtures::l16_00078_bytes() else {
			return;
		};
		let headers = all_light_headers(&bytes);
		assert!(!headers.is_empty(), "expected light header block");
		let mut rows = Vec::new();
		for lh in &headers {
			for module in &lh.modules {
				rows.push((
					module.id(),
					module.mirror_position,
					module.af_info.as_ref().and_then(|af| af.mirror_position),
				));
			}
		}
		eprintln!("light headers={} module rows={}", headers.len(), rows.len());
		let b2 = rows
			.iter()
			.find(|(id, _, _)| *id == lri_proto::camera_id::CameraID::B2);
		let b3 = rows
			.iter()
			.find(|(id, _, _)| *id == lri_proto::camera_id::CameraID::B3);
		let b1 = rows
			.iter()
			.find(|(id, _, _)| *id == lri_proto::camera_id::CameraID::B1);
		eprintln!("mirror halls: {rows:?}");
		if let Some(b2) = b2 {
			assert_eq!(b2.2, Some(817), "B2 af_info mirror hall");
		}
		let lri = crate::LriFile::decode(&bytes).expect("decode");
		assert_eq!(lri.fusion.mirror_hall_codes.get(&CameraId::B2), Some(&817));
		assert_eq!(lri.fusion.mirror_hall_codes.get(&CameraId::B3), Some(&824));
		if let Some(b3) = b3 {
			assert!(
				b3.2.is_some(),
				"B3 mirror_position expected in af_info: top={:?} af={:?}",
				b3.1,
				b3.2
			);
		}
		if let Some(b1) = b1 {
			assert!(
				b1.1.is_some() || b1.2.is_some(),
				"B1 should have mirror hall somewhere"
			);
		}
	}
}
