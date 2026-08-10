//! Optional backend: spawn x86_64 `libcp-export` helper under Rosetta.
//!
//! Light’s fusion engine (`libcp.dylib`) is proprietary and x86_64-only.
//! This module does not link libcp; it finds the helper + dylibs and runs:
//!
//! ```text
//! arch -x86_64 libcp-export libcp.dylib input.lri out.ppm [profile]
//!   [fnumber] [focus_depth_mm] [fx] [fy] [depth.ppm]
//! ```
//!
//! See `tools/libcp-export/README.md`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use image::ImageFormat;
use owo_colors::OwoColorize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
	Ppm,
	Jpg,
	Both,
}

impl OutputFormat {
	pub fn parse(s: &str) -> Result<Self> {
		match s.to_ascii_lowercase().as_str() {
			"ppm" => Ok(Self::Ppm),
			"jpg" | "jpeg" => Ok(Self::Jpg),
			"both" => Ok(Self::Both),
			other => bail!("unknown --format {other:?} (ppm|jpg|both)"),
		}
	}
}

/// DOF / refocus options forwarded to `libcp-export` (M4).
///
/// Maps to CIAPI `ParamFloat`: ViewDofFNumber (3), ViewDofFocusDepth (1).
/// Click focus: normalized image coords in \[0,1\] → DepthEditor sample after
/// first render, then re-render with focus depth.
#[derive(Debug, Clone, Default)]
pub struct DofOpts {
	/// f-number 2.0–15.0; `None` = leave engine default
	pub fnumber: Option<f32>,
	/// Focus plane in mm; `None` = leave default (or sample from focus_xy)
	pub focus_depth_mm: Option<f32>,
	/// Normalized click point (0..1, 0..1) for click-to-focus
	pub focus_xy: Option<(f32, f32)>,
	/// When true, write `<stem>.depth.ppm` beside the RGB output
	pub depth_map: bool,
}

impl DofOpts {
	pub fn is_default(&self) -> bool {
		self.fnumber.is_none()
			&& self.focus_depth_mm.is_none()
			&& self.focus_xy.is_none()
			&& !self.depth_map
	}

	/// Cache / file suffix so DOF variants do not collide with plain renders.
	pub fn cache_suffix(&self) -> String {
		if self.is_default() {
			return String::new();
		}
		let mut s = String::new();
		if let Some(f) = self.fnumber {
			s.push_str(&format!("_f{f:.1}"));
		}
		if let Some(z) = self.focus_depth_mm {
			s.push_str(&format!("_z{z:.0}"));
		}
		if let Some((x, y)) = self.focus_xy {
			s.push_str(&format!("_xy{x:.3}_{y:.3}"));
		}
		if self.depth_map {
			s.push_str("_depth");
		}
		s
	}

	fn validate(&self) -> Result<()> {
		if let Some(f) = self.fnumber {
			if !(2.0..=15.0).contains(&f) {
				bail!("fnumber must be in 2.0..=15.0, got {f}");
			}
		}
		if let Some(z) = self.focus_depth_mm {
			if !(1.0..=1.0e7).contains(&z) {
				bail!("focus_depth_mm out of range: {z}");
			}
		}
		if let Some((x, y)) = self.focus_xy {
			if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
				bail!("focus_xy must be in [0,1]², got ({x}, {y})");
			}
		}
		Ok(())
	}
}

#[derive(Debug)]
pub struct LibcpPaths {
	pub helper: PathBuf,
	pub libcp: PathBuf,
	pub lib_dir: PathBuf,
}

/// Resolve helper binary and libcp.dylib from env + well-known locations.
pub fn resolve_paths() -> Result<LibcpPaths> {
	let helper = resolve_helper()?;
	let libcp = resolve_libcp()?;
	let lib_dir = libcp
		.parent()
		.map(Path::to_path_buf)
		.context("libcp has no parent directory")?;
	// ceres lives next to libcp (Lumen Frameworks or vendor/)
	let ceres = lib_dir.join("libceres.dylib");
	if !ceres.is_file() {
		eprintln!(
			"{} {} (libcp may still load if rpath covers it)",
			"warn:".yellow(),
			format!("libceres.dylib not found next to {}", libcp.display())
		);
	}
	Ok(LibcpPaths {
		helper,
		libcp,
		lib_dir,
	})
}

fn resolve_helper() -> Result<PathBuf> {
	if let Ok(p) = env::var("LUMINAT_LIBCP_EXPORT") {
		let pb = PathBuf::from(p);
		if pb.is_file() {
			return Ok(pb);
		}
		bail!("LUMINAT_LIBCP_EXPORT set but not a file: {}", pb.display());
	}

	// User config (setup wizard)
	let cfg = crate::config::load();
	if let Some(p) = cfg.libcp_export {
		let pb = PathBuf::from(&p);
		if pb.is_file() {
			return Ok(fs::canonicalize(&pb).unwrap_or(pb));
		}
	}

	let mut candidates: Vec<PathBuf> = Vec::new();

	// next to this executable (.app Contents/MacOS or target/release)
	if let Ok(exe) = env::current_exe() {
		if let Some(dir) = exe.parent() {
			candidates.push(dir.join("libcp-export"));
			candidates.push(dir.join("../Resources/libcp-export"));
			// target/release → ../../tools/libcp-export/libcp-export
			candidates.push(dir.join("../../tools/libcp-export/libcp-export"));
			// app support
			if let Ok(home) = env::var("HOME") {
				candidates.push(
					PathBuf::from(home).join("Library/Application Support/Luminat/libcp-export"),
				);
			}
		}
	}

	// cwd-relative (dev)
	candidates.push(PathBuf::from("tools/libcp-export/libcp-export"));
	candidates.push(PathBuf::from("./libcp-export"));

	// PATH
	if let Ok(path) = env::var("PATH") {
		for part in path.split(':') {
			if part.is_empty() {
				continue;
			}
			candidates.push(PathBuf::from(part).join("libcp-export"));
		}
	}

	for c in &candidates {
		if c.is_file() {
			return Ok(fs::canonicalize(c).unwrap_or_else(|_| c.clone()));
		}
	}

	bail!(
		"libcp-export helper not found.\n\
		 Build it:  make libcp-export\n\
		 Or set:    export LUMINAT_LIBCP_EXPORT=/path/to/libcp-export\n\
		 See:       tools/libcp-export/README.md"
	)
}

fn resolve_libcp() -> Result<PathBuf> {
	if let Ok(p) = env::var("LUMINAT_LIBCP") {
		let pb = PathBuf::from(p);
		if pb.is_file() {
			return Ok(pb);
		}
		bail!("LUMINAT_LIBCP set but not a file: {}", pb.display());
	}

	let mut dirs: Vec<PathBuf> = Vec::new();

	if let Ok(d) = env::var("LUMINAT_LIBCP_DIR") {
		dirs.push(PathBuf::from(d));
	}

	// User config (setup wizard)
	let cfg = crate::config::load();
	if let Some(d) = cfg.libcp_dir {
		dirs.push(PathBuf::from(d));
	}
	// App Support vendor drop
	if let Ok(home) = env::var("HOME") {
		dirs.push(PathBuf::from(home).join("Library/Application Support/Luminat/libcp"));
	}

	// Lumen app install locations
	dirs.push(PathBuf::from("/Applications/Lumen.app/Contents/Frameworks"));
	if let Ok(home) = env::var("HOME") {
		dirs.push(PathBuf::from(home).join("Applications/Lumen.app/Contents/Frameworks"));
	}

	// vendor drop next to helper / repo
	dirs.push(PathBuf::from("tools/libcp-export/vendor"));
	if let Ok(exe) = env::current_exe() {
		if let Some(dir) = exe.parent() {
			dirs.push(dir.join("../../tools/libcp-export/vendor"));
			dirs.push(dir.join("libcp"));
			dirs.push(dir.join("../Resources/libcp"));
		}
	}

	// Grok research scratch (dev machine)
	dirs.push(PathBuf::from("/tmp/grok-risk0/libcp"));
	dirs.push(PathBuf::from(
		"/tmp/grok-risk0/Lumen.app/Contents/Frameworks",
	));

	for d in &dirs {
		let lib = d.join("libcp.dylib");
		if lib.is_file() {
			return Ok(fs::canonicalize(&lib).unwrap_or(lib));
		}
	}

	bail!(
		"libcp.dylib not found.\n\
		 Install Lumen.app, or:\n\
		   export LUMINAT_LIBCP_DIR=/path/to/Frameworks\n\
		   # dir must contain libcp.dylib and libceres.dylib\n\
		 See tools/libcp-export/README.md"
	)
}

/// Paths written by a successful `run` / `run_with_opts`.
#[derive(Debug, Clone)]
pub struct LibcpOutput {
	pub jpg: Option<Utf8PathBuf>,
	pub ppm: Option<Utf8PathBuf>,
	pub depth_ppm: Option<Utf8PathBuf>,
	pub profile: i32,
}

/// Batch: every `.lri` in `input_dir` → `output/<stem>/`.
/// LibCP is heavy (Rosetta + full fusion); runs **sequentially** by default.
pub fn run_dir(
	input_dir: &Utf8Path,
	output: &Utf8Path,
	profile: i32,
	format: OutputFormat,
) -> Result<Vec<Utf8PathBuf>> {
	run_dir_with_opts(input_dir, output, profile, format, &DofOpts::default())
}

pub fn run_dir_with_opts(
	input_dir: &Utf8Path,
	output: &Utf8Path,
	profile: i32,
	format: OutputFormat,
	dof: &DofOpts,
) -> Result<Vec<Utf8PathBuf>> {
	if !input_dir.is_dir() {
		bail!("not a directory: {input_dir}");
	}
	let mut lris: Vec<_> = fs::read_dir(input_dir.as_std_path())
		.with_context(|| format!("read {input_dir}"))?
		.filter_map(|e| e.ok())
		.map(|e| e.path())
		.filter(|p| p.extension().and_then(|x| x.to_str()) == Some("lri"))
		.collect();
	lris.sort();
	if lris.is_empty() {
		bail!("no .lri files in {input_dir}");
	}

	// Resolve once so missing libcp fails before first heavy load
	let _ = resolve_paths()?;

	let mut outs = Vec::new();
	let total = lris.len();
	for (i, path) in lris.into_iter().enumerate() {
		let path = Utf8PathBuf::try_from(path).context("non-utf8 path")?;
		let stem = path.file_stem().unwrap_or("out");
		let out = output.join(stem);
		eprintln!(
			"\n{} [{}/{}] {} → {}",
			"batch".cyan(),
			i + 1,
			total,
			path.file_name().unwrap_or("?"),
			out
		);
		run_with_opts(&path, &out, profile, format, dof)?;
		outs.push(out);
	}
	eprintln!(
		"\n{} {} file(s) → {}",
		"batch done".green(),
		outs.len(),
		output
	);
	Ok(outs)
}

pub fn run(lri: &Utf8Path, output: &Utf8Path, profile: i32, format: OutputFormat) -> Result<()> {
	run_with_opts(lri, output, profile, format, &DofOpts::default()).map(|_| ())
}

pub fn run_with_opts(
	lri: &Utf8Path,
	output: &Utf8Path,
	profile: i32,
	format: OutputFormat,
	dof: &DofOpts,
) -> Result<LibcpOutput> {
	if !lri.is_file() {
		bail!("LRI not found: {lri}");
	}
	dof.validate()?;

	// DOF / DepthEditor require DESKTOP (libcp: "does not support depth" / DepthEditor).
	let mut profile = profile;
	if (dof.depth_map || dof.focus_xy.is_some() || dof.fnumber.is_some() || dof.focus_depth_mm.is_some())
		&& profile < 3
	{
		eprintln!(
			"{} DOF/depth features need DESKTOP profile — using 3 (was {profile})",
			"note:".yellow()
		);
		profile = 3;
	}

	let paths = resolve_paths()?;
	eprintln!(
		"{} helper={} libcp={}",
		"libcp".cyan(),
		paths.helper.display(),
		paths.libcp.display()
	);

	fs::create_dir_all(output).with_context(|| format!("create output dir {output}"))?;

	let stem = lri
		.file_stem()
		.map(|s| s.to_string())
		.unwrap_or_else(|| "out".into());
	let ppm_path: Utf8PathBuf = output.join(format!("{stem}.libcp.ppm"));
	let jpg_path: Utf8PathBuf = output.join(format!("{stem}.libcp.jpg"));
	let depth_path: Utf8PathBuf = output.join(format!("{stem}.depth.ppm"));

	// Prefer arch -x86_64 so arm64 host always runs Rosetta for the helper.
	// Important: on macOS, `arch` does not reliably forward DYLD_* from the
	// parent Command env into the x86_64 child. Insert `env VAR=...` so libcp
	// can resolve @rpath/libceres.dylib next to itself.
	let use_arch = cfg!(target_os = "macos");
	let lib_dir_str = paths.lib_dir.display().to_string();

	let mut cmd = if use_arch {
		let mut c = Command::new("arch");
		c.arg("-x86_64");
		c.arg("env");
		c.arg(format!("DYLD_LIBRARY_PATH={lib_dir_str}"));
		c.arg(format!("DYLD_FALLBACK_LIBRARY_PATH={lib_dir_str}"));
		c.arg(&paths.helper);
		c
	} else {
		let mut c = Command::new(&paths.helper);
		c.env("DYLD_LIBRARY_PATH", &paths.lib_dir);
		c.env("DYLD_FALLBACK_LIBRARY_PATH", &paths.lib_dir);
		c
	};

	// argv: libcp lri out.ppm profile fnumber focus_mm fx fy [depth.ppm]
	let fnum = dof.fnumber.unwrap_or(-1.0);
	let focus = dof.focus_depth_mm.unwrap_or(-1.0);
	let (fx, fy) = dof.focus_xy.unwrap_or((-1.0, -1.0));

	cmd.arg(&paths.libcp)
		.arg(lri.as_std_path())
		.arg(ppm_path.as_std_path())
		.arg(profile.to_string())
		.arg(format!("{fnum}"))
		.arg(format!("{focus}"))
		.arg(format!("{fx}"))
		.arg(format!("{fy}"));
	if dof.depth_map {
		cmd.arg(depth_path.as_std_path());
	}
	cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

	eprintln!(
		"{} profile={profile} fnum={fnum} focus={focus} xy=({fx},{fy}) depth_map={} → {}",
		"render".cyan(),
		dof.depth_map,
		ppm_path
	);
	let t0 = Instant::now();
	let out = cmd
		.output()
		.with_context(|| format!("spawn {}", paths.helper.display()))?;

	let stdout = String::from_utf8_lossy(&out.stdout);
	let stderr = String::from_utf8_lossy(&out.stderr);
	if !stdout.is_empty() {
		eprint!("{stdout}");
	}
	if !stderr.is_empty() {
		eprint!("{stderr}");
	}

	if !out.status.success() {
		bail!(
			"libcp-export failed (exit {:?}, {:.1}s). \
			 Ensure Rosetta is installed and libceres sits next to libcp.",
			out.status.code(),
			t0.elapsed().as_secs_f64()
		);
	}

	if !ppm_path.is_file() {
		bail!("helper returned success but PPM missing: {ppm_path}");
	}

	let meta = fs::metadata(ppm_path.as_std_path())?;
	eprintln!(
		"{} {} ({:.1} MB) in {:.1}s",
		"wrote".green(),
		ppm_path,
		meta.len() as f64 / 1e6,
		t0.elapsed().as_secs_f64()
	);

	let mut result = LibcpOutput {
		jpg: None,
		ppm: Some(ppm_path.clone()),
		depth_ppm: None,
		profile,
	};

	if dof.depth_map && depth_path.is_file() {
		result.depth_ppm = Some(depth_path.clone());
		eprintln!("{} {}", "wrote".green(), depth_path);
	}

	match format {
		OutputFormat::Ppm => {}
		OutputFormat::Jpg | OutputFormat::Both => {
			convert_ppm_to_jpg(&ppm_path, &jpg_path)?;
			eprintln!("{} {}", "wrote".green(), jpg_path);
			result.jpg = Some(jpg_path.clone());
			if format == OutputFormat::Jpg {
				let _ = fs::remove_file(ppm_path.as_std_path());
				eprintln!("{} {}", "removed".dimmed(), ppm_path);
				result.ppm = None;
			}
		}
	}

	// Convert depth PPM → JPG for UI if present
	if let Some(ref dppm) = result.depth_ppm {
		let djpg = output.join(format!("{stem}.depth.jpg"));
		if convert_ppm_to_jpg(dppm, &djpg).is_ok() {
			eprintln!("{} {}", "wrote".green(), djpg);
		}
	}

	Ok(result)
}

fn convert_ppm_to_jpg(ppm: &Utf8Path, jpg: &Utf8Path) -> Result<()> {
	let img = image::open(ppm.as_std_path()).with_context(|| format!("open {ppm}"))?;
	img.save_with_format(jpg.as_std_path(), ImageFormat::Jpeg)
		.with_context(|| format!("write {jpg}"))?;
	Ok(())
}
