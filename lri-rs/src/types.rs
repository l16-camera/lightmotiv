/// Responsible for mapping generated protobuf enums to enums defined here. It
/// seemed like a bad idea to rexport from lri-proto.
use std::fmt;

use lri_proto::{
	camera_id::CameraID as PbCameraID, camera_module::camera_module::surface::FormatType,
	color_calibration::color_calibration::IlluminantType,
	view_preferences::view_preferences::HDRMode,
};

#[derive(Copy, Clone, Debug, PartialEq)]
/// The representation of the raw data in the LRI file
pub enum DataFormat {
	BayerJpeg,
	Packed10bpp,
	// Never seen. These are likely from other devices with the Light ASIC
	//Packed12bpp,
	//Packed14bpp,
}

impl fmt::Display for DataFormat {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let str = match self {
			Self::BayerJpeg => "BayerJpeg",
			Self::Packed10bpp => "Packed10bpp",
		};

		write!(f, "{str}")
	}
}

impl From<FormatType> for DataFormat {
	fn from(proto: FormatType) -> Self {
		match proto {
			FormatType::RAW_BAYER_JPEG => Self::BayerJpeg,
			FormatType::RAW_PACKED_10BPP => Self::Packed10bpp,
			FormatType::RAW_PACKED_12BPP => unreachable!(),
			FormatType::RAW_PACKED_14BPP => unreachable!(),
			FormatType::RAW_RESERVED_0
			| FormatType::RAW_RESERVED_1
			| FormatType::RAW_RESERVED_2
			| FormatType::RAW_RESERVED_3
			| FormatType::RAW_RESERVED_4
			| FormatType::RAW_RESERVED_5 => unimplemented!(),
		}
	}
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum CameraId {
	A1,
	A2,
	A3,
	A4,
	A5,
	B1,
	B2,
	B3,
	B4,
	B5,
	C1,
	C2,
	C3,
	C4,
	C5,
	C6,
}

impl From<PbCameraID> for CameraId {
	fn from(pbid: PbCameraID) -> Self {
		match pbid {
			PbCameraID::A1 => Self::A1,
			PbCameraID::A2 => Self::A2,
			PbCameraID::A3 => Self::A3,
			PbCameraID::A4 => Self::A4,
			PbCameraID::A5 => Self::A5,
			PbCameraID::B1 => Self::B1,
			PbCameraID::B2 => Self::B2,
			PbCameraID::B3 => Self::B3,
			PbCameraID::B4 => Self::B4,
			PbCameraID::B5 => Self::B5,
			PbCameraID::C1 => Self::C1,
			PbCameraID::C2 => Self::C2,
			PbCameraID::C3 => Self::C3,
			PbCameraID::C4 => Self::C4,
			PbCameraID::C5 => Self::C5,
			PbCameraID::C6 => Self::C6,
		}
	}
}

impl fmt::Display for CameraId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		// this is good; i write good code
		write!(f, "{self:?}")
	}
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Whitepoint {
	A,
	D50,
	D65,
	D75,
	F2,
	F7,
	F11,
	TL84,
}

impl From<IlluminantType> for Whitepoint {
	fn from(it: IlluminantType) -> Self {
		match it {
			IlluminantType::A => Self::A,
			IlluminantType::D50 => Self::D50,
			IlluminantType::D65 => Self::D65,
			IlluminantType::D75 => Self::D75,
			IlluminantType::F2 => Self::F2,
			IlluminantType::F7 => Self::F7,
			IlluminantType::F11 => Self::F11,
			IlluminantType::TL84 => Self::TL84,
			IlluminantType::UNKNOWN => unimplemented!(),
		}
	}
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum SensorModel {
	Unknown,
	Ar1335,
	Ar1335Mono,
	// Never Seen. Likely from other devices with the Light ASIC
	//Ar835,
	//Imx386,
	//Imx386Mono,
}

impl From<lri_proto::sensor_type::SensorType> for SensorModel {
	fn from(pbst: lri_proto::sensor_type::SensorType) -> Self {
		use lri_proto::sensor_type::SensorType as ProtoSt;

		match pbst {
			ProtoSt::SENSOR_UNKNOWN => Self::Unknown,
			ProtoSt::SENSOR_AR1335 => Self::Ar1335,
			ProtoSt::SENSOR_AR1335_MONO => Self::Ar1335Mono,
			ProtoSt::SENSOR_AR835 | ProtoSt::SENSOR_IMX386 | ProtoSt::SENSOR_IMX386_MONO => {
				unimplemented!()
			}
		}
	}
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HdrMode {
	None,
	Default,
	Natural,
	Surreal,
}

impl From<HDRMode> for HdrMode {
	fn from(h: HDRMode) -> Self {
		match h {
			HDRMode::HDR_MODE_NONE => Self::None,
			HDRMode::HDR_MODE_DEFAULT => Self::Default,
			HDRMode::HDR_MODE_NATURAL => Self::Natural,
			HDRMode::HDR_MODE_SURREAL => Self::Surreal,
		}
	}
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SceneMode {
	Portrait,
	Landscape,
	Sport,
	Macro,
	Night,
	None,
}

impl From<lri_proto::view_preferences::view_preferences::SceneMode> for SceneMode {
	fn from(sm: lri_proto::view_preferences::view_preferences::SceneMode) -> Self {
		use lri_proto::view_preferences::view_preferences::SceneMode as PbSceneMode;

		match sm {
			PbSceneMode::SCENE_MODE_PORTRAIT => Self::Portrait,
			PbSceneMode::SCENE_MODE_LANDSCAPE => Self::Landscape,
			PbSceneMode::SCENE_MODE_SPORT => Self::Sport,
			PbSceneMode::SCENE_MODE_MACRO => Self::Macro,
			PbSceneMode::SCENE_MODE_NIGHT => Self::Night,
			PbSceneMode::SCENE_MODE_NONE => Self::None,
		}
	}
}

/// Auto White Balance Mode
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AwbMode {
	Auto,
	Daylight,
}

impl From<lri_proto::view_preferences::view_preferences::AWBMode> for AwbMode {
	fn from(awb: lri_proto::view_preferences::view_preferences::AWBMode) -> Self {
		use lri_proto::view_preferences::view_preferences::AWBMode as PbAwbMode;

		match awb {
			PbAwbMode::AWB_MODE_AUTO => Self::Auto,
			PbAwbMode::AWB_MODE_DAYLIGHT => Self::Daylight,
			_ => panic!("{awb:?}"),
		}
	}
}

/// EXIF-style orientation from `ViewPreferences.orientation`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Orientation {
	Normal,
	Rot90Cw,
	Rot90Ccw,
	Rot90CwVflip,
	Rot90CcwVflip,
	Vflip,
	Hflip,
	Rot180,
}

impl From<lri_proto::view_preferences::view_preferences::Orientation> for Orientation {
	fn from(o: lri_proto::view_preferences::view_preferences::Orientation) -> Self {
		use lri_proto::view_preferences::view_preferences::Orientation as Pb;
		match o {
			Pb::ORIENTATION_NORMAL => Self::Normal,
			Pb::ORIENTATION_ROT90_CW => Self::Rot90Cw,
			Pb::ORIENTATION_ROT90_CCW => Self::Rot90Ccw,
			Pb::ORIENTATION_ROT90_CW_VFLIP => Self::Rot90CwVflip,
			Pb::ORIENTATION_ROT90_CCW_VFLIP => Self::Rot90CcwVflip,
			Pb::ORIENTATION_VFLIP => Self::Vflip,
			Pb::ORIENTATION_HFLIP => Self::Hflip,
			Pb::ORIENTATION_ROT180 => Self::Rot180,
		}
	}
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AspectRatio {
	Ratio4_3,
	Ratio16_9,
}

impl From<lri_proto::view_preferences::view_preferences::AspectRatio> for AspectRatio {
	fn from(a: lri_proto::view_preferences::view_preferences::AspectRatio) -> Self {
		use lri_proto::view_preferences::view_preferences::AspectRatio as Pb;
		match a {
			Pb::ASPECT_RATIO_4_3 => Self::Ratio4_3,
			Pb::ASPECT_RATIO_16_9 => Self::Ratio16_9,
		}
	}
}

/// Crop rectangle from `ViewPreferences.crop` (pixel coords on Lumen output canvas).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ViewCrop {
	pub start: [f32; 2],
	pub size: [f32; 2],
}

/// Output framing from `ViewPreferences` (crop, orientation, aspect).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewOutput {
	pub crop: Option<ViewCrop>,
	pub orientation: Option<Orientation>,
	pub disable_cropping: bool,
	pub aspect_ratio: Option<AspectRatio>,
}

impl ViewOutput {
	/// Lumen desktop default super-res canvas (verified L16_00078 export).
	pub const LUMEN_CANVAS: (u32, u32) = (10432, 7824);

	pub fn effective_crop(&self) -> Option<ViewCrop> {
		if self.disable_cropping {
			return None;
		}
		self.crop
	}

	fn crop_is_normalized(crop: &ViewCrop) -> bool {
		crop.start[0] <= 1.0 && crop.start[1] <= 1.0 && crop.size[0] <= 1.0 && crop.size[1] <= 1.0
	}

	/// Pixel crop rect clamped to `canvas`, or full frame when cropping disabled.
	pub fn crop_rect_px(&self, canvas: (u32, u32)) -> (u32, u32, u32, u32) {
		let (cw, ch) = canvas;
		let Some(crop) = self.effective_crop() else {
			return (0, 0, cw, ch);
		};
		let (x0f, y0f, wf, hf) = if Self::crop_is_normalized(&crop) {
			(
				crop.start[0] * cw as f32,
				crop.start[1] * ch as f32,
				crop.size[0] * cw as f32,
				crop.size[1] * ch as f32,
			)
		} else {
			(crop.start[0], crop.start[1], crop.size[0], crop.size[1])
		};
		let x0 = x0f.round().max(0.0) as u32;
		let y0 = y0f.round().max(0.0) as u32;
		let w = wf.round().max(1.0) as u32;
		let h = hf.round().max(1.0) as u32;
		let w = w.min(cw.saturating_sub(x0));
		let h = h.min(ch.saturating_sub(y0));
		(x0, y0, w, h)
	}
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AwbGain {
	pub r: f32,
	pub gr: f32,
	pub gb: f32,
	pub b: f32,
}

impl From<lri_proto::view_preferences::view_preferences::ChannelGain> for AwbGain {
	fn from(gain: lri_proto::view_preferences::view_preferences::ChannelGain) -> Self {
		// all fields in ChannelGain are marked as required
		Self {
			r: gain.r.unwrap(),
			gr: gain.g_r.unwrap(),
			gb: gain.g_b.unwrap(),
			b: gain.b.unwrap(),
		}
	}
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SensorData {
	pub sensor_type: SensorModel,
	pub characterization: SensorCharacterization,
}

impl From<lri_proto::lightheader::SensorData> for SensorData {
	fn from(sd: lri_proto::lightheader::SensorData) -> Self {
		Self {
			sensor_type: sd.type_().into(),
			characterization: sd.data.into_option().unwrap().into(),
		}
	}
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SensorCharacterization {
	pub black_level: f32,
	pub white_level: f32,
	pub cliff_slope: Option<f32>,
}

impl From<lri_proto::sensor_characterization::SensorCharacterization> for SensorCharacterization {
	fn from(sc: lri_proto::sensor_characterization::SensorCharacterization) -> Self {
		Self {
			black_level: sc.black_level.unwrap(),
			white_level: sc.white_level.unwrap(),
			cliff_slope: sc.cliff_slope,
		}
	}
}
