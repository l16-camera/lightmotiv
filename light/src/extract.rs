use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use camino::Utf8Path;
use lri_rs::LriFile;
use rayon::prelude::*;
use serde::Serialize;

use crate::fusion_sidecar;
use crate::mono;
use crate::render;
use crate::session::LriSession;
use crate::threads;
use crate::thumbnail;

#[derive(Debug, Clone, Copy, Default)]
pub struct ExtractOptions {
	pub jobs: Option<usize>,
	/// Only export AR1335 Mono planes (A2 / C6 when present).
	pub only_mono: bool,
	/// Write 8-bit gray PNG previews under mono/ for mono modules.
	pub mono_previews: bool,
}

#[derive(Debug, Serialize)]
pub struct ExtractReport {
	pub image_count: usize,
	pub mono_count: usize,
	pub files: Vec<String>,
}

pub fn run(input: &Utf8Path, output: &Utf8Path, jobs: Option<usize>) -> Result<()> {
	run_with_options(
		input,
		output,
		ExtractOptions {
			jobs,
			..Default::default()
		},
	)
	.map(|_| ())
}

pub fn run_with_options(
	input: &Utf8Path,
	output: &Utf8Path,
	opts: ExtractOptions,
) -> Result<ExtractReport> {
	run_with_progress_opts(input, output, opts, |_, _, _| {})
}

/// Classic API (stable for external tools / upstream parity): jobs + progress → `()`.
pub fn run_with_progress(
	input: &Utf8Path,
	output: &Utf8Path,
	jobs: Option<usize>,
	on_progress: impl Fn(usize, usize, &str) + Send + Sync + 'static,
) -> Result<()> {
	run_with_progress_opts(
		input,
		output,
		ExtractOptions {
			jobs,
			..Default::default()
		},
		on_progress,
	)
	.map(|_| ())
}

/// Extended extract with mono filters / report.
pub fn run_with_progress_opts(
	input: &Utf8Path,
	output: &Utf8Path,
	opts: ExtractOptions,
	on_progress: impl Fn(usize, usize, &str) + Send + Sync + 'static,
) -> Result<ExtractReport> {
	let session = LriSession::open(input)?;
	run_session_with_progress(&session, output, opts, on_progress)
}

pub fn run_session_with_progress(
	session: &LriSession,
	output: &Utf8Path,
	opts: ExtractOptions,
	on_progress: impl Fn(usize, usize, &str) + Send + Sync + 'static,
) -> Result<ExtractReport> {
	let n = threads::export_jobs(opts.jobs);
	let pool = rayon::ThreadPoolBuilder::new()
		.num_threads(n)
		.build()
		.context("configure thread pool")?;

	session.with_lri(|lri| pool.install(|| run_decoded(lri, output, opts, on_progress)))
}

/// Batch-extract every .lri in a directory into `output/<stem>/`.
pub fn run_dir(
	input_dir: &Utf8Path,
	output: &Utf8Path,
	opts: ExtractOptions,
) -> Result<Vec<(String, ExtractReport)>> {
	let mut lris: Vec<_> = std::fs::read_dir(input_dir.as_std_path())
		.with_context(|| format!("read {input_dir}"))?
		.filter_map(|e| e.ok())
		.map(|e| e.path())
		.filter(|p| p.extension().and_then(|x| x.to_str()) == Some("lri"))
		.collect();
	lris.sort();

	if lris.is_empty() {
		anyhow::bail!("no .lri files in {input_dir}");
	}

	let mut reports = Vec::new();
	for path in lris {
		let path = camino::Utf8PathBuf::try_from(path).context("non-utf8 path")?;
		let stem = path.file_stem().unwrap_or("out");
		let out = output.join(stem);
		eprintln!(
			"=== extract {} → {} ===",
			path.file_name().unwrap_or("?"),
			out
		);
		let report = run_with_options(&path, &out, opts)?;
		reports.push((path.to_string(), report));
	}
	Ok(reports)
}

fn run_decoded(
	lri: &LriFile<'_>,
	output: &Utf8Path,
	opts: ExtractOptions,
	on_progress: impl Fn(usize, usize, &str) + Send + Sync + 'static,
) -> Result<ExtractReport> {
	if !output.exists() {
		std::fs::create_dir_all(output).context("create output directory")?;
	}

	let images: Vec<_> = lri
		.images
		.iter()
		.filter(|img| !opts.only_mono || mono::is_mono_image(img))
		.collect();

	let total = images.len();
	let mono_count = images.iter().filter(|i| mono::is_mono_image(i)).count();
	let done = AtomicUsize::new(0);
	let on_progress = Arc::new(on_progress);
	let written = std::sync::Mutex::new(Vec::<String>::new());

	eprintln!(
		"{total} images{}",
		if opts.only_mono { " (mono only)" } else { "" }
	);

	if let Some(refimg) = lri.reference_image() {
		eprintln!("reference camera: {}", refimg.camera);
	}

	let minfo = mono::mono_info(lri);
	if minfo.present {
		eprintln!(
			"mono: {} ({})",
			minfo.cameras.join(", "),
			minfo
				.cameras
				.iter()
				.filter_map(|id| {
					let cam = match id.as_str() {
						"A2" => lri_rs::CameraId::A2,
						"C6" => lri_rs::CameraId::C6,
						_ => return None,
					};
					mono::mono_focal_mm(cam).map(|mm| format!("{id}≈{mm}mm"))
				})
				.collect::<Vec<_>>()
				.join(", ")
		);
	} else if opts.only_mono {
		eprintln!("warn: no mono modules in this file");
	}

	if opts.mono_previews && mono_count > 0 {
		std::fs::create_dir_all(output.join("mono")).context("create mono/")?;
	}

	images.par_iter().try_for_each(|img| {
		let stem = mono::export_stem(img);
		let path = output.join(format!("{stem}.dng"));
		render::export_dng(img, lri, path.clone())?;

		let mut names = vec![format!("{stem}.dng")];

		if mono::is_mono_image(img) && opts.mono_previews {
			let png_name = format!("mono/{}.png", img.camera);
			let png_path = output.join(&png_name);
			match write_mono_preview_png(lri, img.camera, &png_path) {
				Ok(()) => names.push(png_name),
				Err(e) => eprintln!("  mono preview {}: {e}", img.camera),
			}
		}

		{
			let mut w = written.lock().unwrap();
			w.extend(names);
		}

		let n = done.fetch_add(1, Ordering::SeqCst) + 1;
		on_progress(n, total, &stem);
		Ok::<(), anyhow::Error>(())
	})?;

	fusion_sidecar::write_json(lri, &output.join("fusion.json"))?;
	eprintln!("wrote {}", output.join("fusion.json"));

	// mono sidecar for Lightroom / batch tools
	let mono_path = output.join("mono.json");
	let mono_json = serde_json::to_string_pretty(&minfo)?;
	std::fs::write(mono_path.as_std_path(), mono_json).context("write mono.json")?;
	eprintln!("wrote {mono_path}");

	let files = written.into_inner().unwrap();
	Ok(ExtractReport {
		image_count: total,
		mono_count,
		files,
	})
}

fn write_mono_preview_png(
	lri: &LriFile<'_>,
	camera: lri_rs::CameraId,
	path: &Utf8Path,
) -> Result<()> {
	// Larger preview than UI thumbs — useful for quick mono look
	let (bytes, w, h, _) = thumbnail::render_preview_gray(lri, camera, 2048)?;
	let file =
		std::fs::File::create(path.as_std_path()).with_context(|| format!("create {path}"))?;
	let mut encoder = png::Encoder::new(file, w, h);
	encoder.set_color(png::ColorType::Grayscale);
	encoder.set_depth(png::BitDepth::Eight);
	let mut writer = encoder.write_header().context("png header")?;
	writer.write_image_data(&bytes).context("png data")?;
	Ok(())
}
