use thiserror::Error;

#[derive(Debug, Error)]
pub enum LriError {
	#[error("invalid block magic (expected LELR)")]
	InvalidMagic,

	#[error("unknown block type {0}")]
	UnknownBlockType(u8),

	#[error("block length {need} exceeds remaining data ({have} bytes)")]
	TruncatedBlock { need: usize, have: usize },

	#[error("protobuf parse failed: {0}")]
	ProtobufParse(String),

	#[error("bayer jpeg decode failed: {0}")]
	BayerJpegDecode(String),

	#[error("unsupported raw format")]
	UnsupportedFormat,

	#[error("pixel buffer length mismatch: expected {expected}, got {got}")]
	PixelCountMismatch { expected: usize, got: usize },
}
