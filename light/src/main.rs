use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use light::{extract, fuse, gather, libcp, validate_rt};

#[derive(Parser)]
#[command(
	name = "light",
	about = "Luminat — illuminate the 16→1 ritual",
	long_about = "\
Luminat — a not-so-secret society for Light L16.\n\n\
Sixteen modules witness; one image emerges. We decode .lri, undistort, warp, and blend — \
the fusion rite Lumen kept behind closed doors.\n\n\
All seeing is computational. isamarin × IGRS",
	version
)]
struct Cli {
	#[command(subcommand)]
	command: Command,
}

#[derive(Subcommand)]
enum Command {
	/// Print metadata for all .lri files in a directory
	Gather {
		/// Directory containing .lri / .jpg / .lris files
		path: camino::Utf8PathBuf,
	},
	/// Validate R/t by warping module previews and comparing to Lumen fused JPG
	Validate {
		/// Input .lri file
		#[arg(long)]
		lri: camino::Utf8PathBuf,
		/// Lumen fused output .jpg
		#[arg(long)]
		lumen: camino::Utf8PathBuf,
		/// Output directory for overlays and metrics
		#[arg(short, long)]
		output: camino::Utf8PathBuf,
		/// Longest preview side in pixels (default 1024)
		#[arg(long, default_value_t = 1024)]
		max_side: u32,
	},
	/// Luminat fuse: undistort, depth warp, blend; optional full-res TIFF/DNG + crop
	Fuse {
		#[arg(long)]
		lri: camino::Utf8PathBuf,
		#[arg(short, long)]
		output: camino::Utf8PathBuf,
		#[arg(long)]
		lumen: Option<camino::Utf8PathBuf>,
		/// Preview longest side (ignored when --full-res)
		#[arg(long, default_value_t = 1024)]
		max_side: u32,
		/// Fuse at Lumen canvas (10432×7824) and export 16-bit TIFF/DNG with crop
		#[arg(long)]
		full_res: bool,
		#[arg(long, default_value_t = true)]
		export_tiff: bool,
		#[arg(long, default_value_t = true)]
		export_dng: bool,
		#[arg(long, default_value_t = 1500.0)]
		depth_min_mm: f64,
		#[arg(long, default_value_t = 8000.0)]
		depth_max_mm: f64,
		#[arg(long, default_value_t = 25)]
		depth_steps: usize,
	},
	/// Extract per-camera DNGs from one LRI file or a directory of .lri files
	Extract {
		/// Input .lri file **or** directory of .lri files
		input: camino::Utf8PathBuf,
		/// Output directory (per-file subdirs when input is a directory)
		output: camino::Utf8PathBuf,
		/// Parallel export jobs (default: logical CPU count)
		#[arg(short, long)]
		jobs: Option<usize>,
		/// Only export panchromatic modules (A2 / C6 mono)
		#[arg(long)]
		only_mono: bool,
		/// Skip mono/ PNG previews (default: write previews for mono modules)
		#[arg(long)]
		no_mono_previews: bool,
	},
	/// Fuse via Light libcp (CIAPI) — x86_64 helper under Rosetta
	///
	/// Requires Lumen's libcp.dylib + libceres.dylib (not shipped). Build helper:
	/// `make libcp-export`. See tools/libcp-export/README.md.
	Libcp {
		/// Input .lri file (mutually exclusive with --dir)
		#[arg(long, conflicts_with = "dir")]
		lri: Option<camino::Utf8PathBuf>,
		/// Directory of .lri files — batch mode (B4)
		#[arg(long, conflicts_with = "lri")]
		dir: Option<camino::Utf8PathBuf>,
		/// Output directory (writes <stem>.libcp.jpg; batch uses output/<stem>/)
		#[arg(short, long)]
		output: camino::Utf8PathBuf,
		/// CIAPI RendererProfile (1 = proven fast path from A1)
		#[arg(long, default_value_t = 1)]
		profile: i32,
		/// Output format: ppm | jpg | both (default jpg)
		#[arg(long, default_value = "jpg")]
		format: String,
		/// Synthetic aperture f-number (2–15); omit for engine default
		#[arg(long)]
		fnumber: Option<f32>,
		/// Focus plane depth in millimetres
		#[arg(long)]
		focus_depth: Option<f32>,
		/// Click-to-focus X in [0,1] (needs --focus-y)
		#[arg(long)]
		focus_x: Option<f32>,
		/// Click-to-focus Y in [0,1] (needs --focus-x)
		#[arg(long)]
		focus_y: Option<f32>,
		/// Write low-res depth colormap `<stem>.depth.ppm` (+ .jpg)
		#[arg(long)]
		depth_map: bool,
	},
}

fn main() -> Result<()> {
	let cli = Cli::parse();

	match cli.command {
		Command::Gather { path } => gather::run(&path),
		Command::Validate {
			lri,
			lumen,
			output,
			max_side,
		} => validate_rt::run(&lri, &lumen, &output, max_side),
		Command::Fuse {
			lri,
			output,
			lumen,
			max_side,
			full_res,
			export_tiff,
			export_dng,
			depth_min_mm,
			depth_max_mm,
			depth_steps,
		} => fuse::run(
			&lri,
			&output,
			lumen.as_deref(),
			max_side,
			full_res,
			export_tiff,
			export_dng,
			depth_min_mm,
			depth_max_mm,
			depth_steps,
		)
		.map(|_| ()),
		Command::Extract {
			input,
			output,
			jobs,
			only_mono,
			no_mono_previews,
		} => {
			let opts = extract::ExtractOptions {
				jobs,
				only_mono,
				mono_previews: !no_mono_previews,
			};
			if input.is_dir() {
				extract::run_dir(&input, &output, opts).map(|_| ())
			} else {
				extract::run_with_options(&input, &output, opts).map(|_| ())
			}
		}
		Command::Libcp {
			lri,
			dir,
			output,
			profile,
			format,
			fnumber,
			focus_depth,
			focus_x,
			focus_y,
			depth_map,
		} => {
			let format = libcp::OutputFormat::parse(&format)?;
			let focus_xy = match (focus_x, focus_y) {
				(Some(x), Some(y)) => Some((x, y)),
				(None, None) => None,
				_ => bail!("--focus-x and --focus-y must be set together"),
			};
			let dof = libcp::DofOpts {
				fnumber,
				focus_depth_mm: focus_depth,
				focus_xy,
				depth_map,
			};
			match (lri, dir) {
				(Some(lri), None) => libcp::run_with_opts(&lri, &output, profile, format, &dof)
					.map(|_| ()),
				(None, Some(dir)) => {
					libcp::run_dir_with_opts(&dir, &output, profile, format, &dof).map(|_| ())
				}
				_ => bail!("libcp requires --lri <file> or --dir <directory>"),
			}
		}
	}
}
