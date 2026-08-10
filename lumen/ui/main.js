const invoke = window.__TAURI__.core.invoke;
const Channel = window.__TAURI__.core.Channel;
const listen = window.__TAURI__.event.listen;
const getCurrentWindow = window.__TAURI__.webviewWindow.getCurrentWindow;

const ROWS = [
	["A1", "A2", "A3", "A4", "A5"],
	["B1", "B2", "B3", "B4", "B5"],
	["C1", "C2", "C3", "C4", "C5", "C6"],
];

const $ = (sel) => document.querySelector(sel);

let current = null;
let files = [];
let thumbGen = 0;
let fuseBusy = false;
let libcpBusy = false;
let lastFusePreview = null;
let lastLibcp = null; // { jpg_path, preview_data_url, profile, width, height, … }
let fuseDropActive = false;
let libcpReady = false;
/** M4 DOF state */
let dofFocus = null; // { x, y } in [0,1] or null
let showingDepth = false;
let lastRgbPreview = null;

const FUSE_STAGES = {
	prepare: "Preparing",
	depth: "Depth sweep",
	warp: "Warping modules",
	blend: "Blending",
	export: "Exporting",
};

function showDetail(show) {
	$("#empty-state").classList.toggle("hidden", show);
	$("#detail").classList.toggle("hidden", !show);
}

function metaCard(label, value) {
	return `<div class="meta-card"><div class="label">${label}</div><div class="value">${value ?? "—"}</div></div>`;
}

function monoLabel(summary) {
	const m = summary.mono;
	if (!m?.present) return "none in this capture";
	return m.cameras
		.map((id) => {
			const cam = summary.cameras.find((c) => c.id === id);
			const mm = cam?.mono_focal_mm;
			return mm != null ? `${id} ≈ ${mm} mm` : id;
		})
		.join(" · ");
}

function renderMono(summary) {
	const panel = $("#mono-panel");
	const badge = $("#mono-badge");
	const detail = $("#mono-detail");
	const m = summary.mono;
	const has = !!m?.present;
	panel?.classList.toggle("mono-empty", !has);
	badge.textContent = has ? m.cameras.join(" · ") : "absent";
	badge.classList.toggle("ok", has);
	detail.textContent = has
		? `${monoLabel(summary)} · AR1335 Mono (no CFA)`
		: "No panchromatic planes in this .lri (A2/C6 only when capture included them).";
	$("#btn-export-mono").disabled = !has;
	const btn2 = $("#btn-export-mono-2");
	if (btn2) btn2.disabled = !has;
}

function renderMeta(summary) {
	const awb = summary.awb_gain
		? summary.awb_gain.map((v) => v.toFixed(2)).join(", ")
		: null;

	$("#meta-grid").innerHTML = [
		metaCard("File", summary.name),
		metaCard("Firmware", summary.firmware),
		metaCard("Focal", summary.focal_length),
		metaCard("Exposure", summary.integration_ms != null ? `${summary.integration_ms} ms` : null),
		metaCard("Gain", summary.gain != null ? summary.gain.toFixed(2) : null),
		metaCard("HDR", summary.hdr),
		metaCard("Scene", summary.scene),
		metaCard("Tripod", summary.on_tripod),
		metaCard("AF", summary.af_achieved),
		metaCard("AWB", summary.awb),
		metaCard("WB gains", awb),
		metaCard("Reference", summary.reference_camera),
		metaCard("Mono", monoLabel(summary)),
		metaCard(
			"Fusion data",
			summary.fusion
				? `geo ${summary.fusion.modules_with_intrinsics}/${summary.fusion.geometry_modules}` +
					(summary.fusion.tof_range_m != null ? ` · tof ${summary.fusion.tof_range_m.toFixed(2)}m` : "") +
					(summary.fusion.imu_frames != null ? ` · imu ${summary.fusion.imu_frames}` : "") +
					(summary.fusion.has_gps ? " · gps" : "")
				: null
		),
	].join("");
	renderMono(summary);
}

function camClass(cam) {
	if (!cam) return "missing";
	if (cam.is_mono || cam.sensor?.includes("Mono")) return "color-mono";
	if (cam.bayer_jpeg) return "color-bayer";
	return "color-packed";
}

function renderCameras(summary) {
	const byId = Object.fromEntries(summary.cameras.map((c) => [c.id, c]));
	$("#camera-count").textContent = String(summary.image_count);

	$("#camera-grid").innerHTML = ROWS.map((row) => {
		const cells = row.map((id) => {
			const cam = byId[id];
			const ref = summary.reference_camera === id ? " ref" : "";
			if (!cam) {
				return `<div class="cam missing${ref}" data-cam="${id}">
					<div class="id">${id}</div>
					<div class="thumb empty-thumb"></div>
					<div class="info">—</div>
				</div>`;
			}
			const monoTag = cam.is_mono
				? `<span class="mono-tag">MONO${cam.mono_focal_mm != null ? ` ${cam.mono_focal_mm}` : ""}</span>`
				: "";
			return `<div class="cam ${camClass(cam)}${ref}" data-cam="${id}">
				<div class="id">${id}${monoTag}</div>
				<div class="thumb"><img alt="${id}" /></div>
				<div class="info">${cam.sensor}<br>${cam.format}</div>
			</div>`;
		}).join("");
		return `<div class="camera-row">${cells}</div>`;
	}).join("");

	loadThumbnails(summary);
}

async function loadThumbnails(summary) {
	const gen = ++thumbGen;
	const path = summary.path;
	const cameras = summary.cameras.map((c) => c.id);

	for (const id of cameras) {
		const cell = document.querySelector(`[data-cam="${id}"] img`);
		if (cell) cell.classList.add("loading");
	}

	try {
		const thumbs = await invoke("camera_thumbnails_batch", { path, cameras, jobs: null });
		if (gen !== thumbGen) return;
		for (const [id, dataUrl] of Object.entries(thumbs)) {
			const cell = document.querySelector(`[data-cam="${id}"] img`);
			if (!cell) continue;
			cell.src = dataUrl;
			cell.classList.remove("loading");
		}
	} catch (e) {
		for (const id of cameras) {
			const cell = document.querySelector(`[data-cam="${id}"] img`);
			if (!cell) continue;
			cell.classList.remove("loading");
			cell.alt = "err";
		}
	}
}

function renderFileList() {
	const list = $("#file-list");
	list.innerHTML = files.map((f, i) => {
		const mono = f.mono?.present
			? ` · mono ${f.mono.cameras.join("+")}`
			: "";
		return `
		<li>
			<button type="button" data-idx="${i}" class="${current === f.path ? "active" : ""}">
				${f.name}
				<span class="sub">${f.image_count} modules${mono} · ${f.firmware ?? "?"}</span>
			</button>
		</li>`;
	}).join("");

	list.querySelectorAll("button").forEach((btn) => {
		btn.addEventListener("click", async () => {
			await selectFile(files[Number(btn.dataset.idx)].path);
		});
	});
}

async function selectFile(path) {
	const summary = await invoke("inspect_lri", { path });
	current = path;
	lastLibcp = null;
	dofFocus = null;
	showingDepth = false;
	lastRgbPreview = null;
	updateDofLabels();
	setFocusMarker(null, null);
	$("#libcp-result")?.classList.add("hidden");
	$("#btn-libcp-export") && ($("#btn-libcp-export").disabled = true);
	$("#btn-libcp-reveal") && ($("#btn-libcp-reveal").disabled = true);
	$("#libcp-depth-thumb")?.classList.add("hidden");
	$("#btn-show-depth")?.classList.add("hidden");
	const st = $("#libcp-status");
	if (st) {
		st.className = "status";
		st.textContent = "";
	}
	files = files.map((f) => (f.path === path ? summary : f));
	showDetail(true);
	renderMeta(summary);
	renderCameras(summary);
	renderFileList();
}

async function loadDirectory(path) {
	const scan = await invoke("scan_directory", { path });
	files = scan.files;
	current = files[0]?.path ?? null;
	renderFileList();
	if (current) {
		await selectFile(current);
	} else {
		showDetail(false);
	}
}

async function openFile() {
	const path = await invoke("pick_lri_file");
	if (!path) return;
	await selectFile(path);
	files = [await invoke("inspect_lri", { path })];
	renderFileList();
}

async function openDir() {
	const path = await invoke("pick_directory");
	if (!path) return;
	await loadDirectory(path);
}

function setProgress(done, total, camera) {
	const wrap = $("#export-progress-wrap");
	const bar = $("#progress-bar");
	const label = $("#progress-label");
	wrap.classList.remove("hidden");
	const pct = total > 0 ? Math.round((done / total) * 100) : 0;
	bar.style.width = `${pct}%`;
	label.textContent = `${done}/${total} · ${camera}`;
}

function fuseMode() {
	const checked = document.querySelector('input[name="fuse-mode"]:checked');
	return checked?.value === "full-res" ? "full-res" : "preview";
}

function updateFuseOptions() {
	const fullRes = fuseMode() === "full-res";
	const opts = $("#fuse-export-options");
	opts.classList.toggle("disabled", !fullRes);
	$("#btn-fuse-export").classList.toggle("hidden", !fullRes);
}

function setFuseProgress(stage, done, total) {
	const wrap = $("#fuse-progress-wrap");
	const bar = $("#fuse-progress-bar");
	const label = $("#fuse-progress-label");
	wrap.classList.remove("hidden");
	const pct = total > 0 ? Math.round((done / total) * 100) : 0;
	bar.style.width = `${pct}%`;
	const name = FUSE_STAGES[stage] ?? stage;
	label.textContent = total > 1 ? `${name} · ${done}/${total}` : name;
}

function classifyDropPaths(paths) {
	const lris = [];
	const dirs = [];
	for (const p of paths ?? []) {
		if (p.toLowerCase().endsWith(".lri")) lris.push(p);
		else dirs.push(p);
	}
	return { lris, dirs };
}

async function hitFuseZone(position) {
	const zone = $("#fuse-drop-zone");
	if (!zone || zone.offsetParent === null || !position) return false;
	const rect = zone.getBoundingClientRect();
	const scale = await getCurrentWindow().scaleFactor();
	const x = position.x / scale;
	const y = position.y / scale;
	return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
}

function setFuseDropActive(active) {
	fuseDropActive = active;
	$("#fuse-drop-zone")?.classList.toggle("active", active);
	const text = $("#drop-overlay-text");
	if (!text) return;
	text.textContent = active
		? "Drop to fuse & export"
		: "Drop .lri file";
}

async function startFileDrag(filePath, iconDataUrl) {
	const onEvent = new Channel();
	await invoke("plugin:drag|start_drag", {
		item: [filePath],
		image: iconDataUrl,
		options: { mode: "Copy" },
		onEvent,
	});
}

function renderFuseExports(exportPaths, previewDataUrl) {
	const wrap = $("#fuse-exports");
	const chips = $("#fuse-export-chips");
	if (!exportPaths?.length) {
		wrap.classList.add("hidden");
		chips.innerHTML = "";
		return;
	}

	wrap.classList.remove("hidden");
	chips.innerHTML = exportPaths.map((path) => {
		const name = path.split(/[/\\]/).pop();
		return `<button type="button" class="export-chip" data-path="${path}">
			<span class="chip-icon">⇱</span>${name}
		</button>`;
	}).join("");

	chips.querySelectorAll(".export-chip").forEach((chip) => {
		chip.addEventListener("mousedown", (e) => {
			e.preventDefault();
			startFileDrag(chip.dataset.path, previewDataUrl).catch(console.error);
		});
	});
}

function bindFusePreviewDrag(previewDataUrl, exportPaths) {
	const img = $("#fuse-preview");
	if (!img) return;
	const primary = exportPaths?.[0];
	img.onmousedown = (e) => {
		if (!primary) return;
		e.preventDefault();
		startFileDrag(primary, previewDataUrl).catch(console.error);
	};
}

function renderFuseStats(summary, outputDir) {
	const ncc = summary.depth_ncc_vs_lumen != null
		? summary.depth_ncc_vs_lumen.toFixed(4)
		: "—";
	const depth = `${summary.depth_plane_mm.toFixed(0)} mm`;
	const modules = String(summary.modules_warped);
	const size = summary.full_res
		? `${summary.canvas[0]}×${summary.canvas[1]}`
		: `${summary.preview_max_side}px preview`;

	$("#fuse-stats").innerHTML = [
		["Depth plane", depth],
		["Modules", modules],
		["NCC vs Lumen", ncc],
		["Output", size],
		["Folder", outputDir],
	].map(([label, value]) => `
		<div class="fuse-stat">
			<div class="label">${label}</div>
			<div class="value">${value}</div>
		</div>
	`).join("");
}

async function runFuse(outputDir) {
	if (!current || fuseBusy) return;

	const fullRes = fuseMode() === "full-res";
	const status = $("#fuse-status");
	const wrap = $("#fuse-progress-wrap");
	const result = $("#fuse-result");

	status.className = "status";
	status.textContent = "";
	wrap.classList.remove("hidden");
	setFuseProgress("prepare", 0, 1);
	result.classList.add("hidden");
	fuseBusy = true;
	$("#btn-fuse").disabled = true;
	$("#btn-fuse-export").disabled = true;
	// do not disable libcp — separate pipeline

	const unlisten = await listen("fuse-progress", (event) => {
		const { stage, done, total } = event.payload;
		setFuseProgress(stage, done, total);
	});

	try {
		const res = await invoke("fuse_lri", {
			input: current,
			output: outputDir,
			maxSide: 1024,
			fullRes,
			exportTiff: $("#opt-tiff").checked,
			exportDng: $("#opt-dng").checked,
			lumenJpg: null,
		});

		lastFusePreview = res.preview_data_url;
		$("#fuse-preview").src = res.preview_data_url;
		renderFuseStats(res.summary, res.output_dir);
		renderFuseExports(res.export_paths, res.preview_data_url);
		bindFusePreviewDrag(res.preview_data_url, res.export_paths);
		result.classList.remove("hidden");

		const exports = res.summary.exports.join(", ");
		status.textContent = fullRes
			? `Done — ${exports}`
			: `Preview ready (temp: ${res.output_dir})`;
		status.classList.add("ok");
		setFuseProgress("export", 1, 1);
	} catch (e) {
		status.textContent = String(e);
		status.classList.add("err");
	} finally {
		fuseBusy = false;
		$("#btn-fuse").disabled = false;
		$("#btn-fuse-export").disabled = false;
		unlisten();
	}
}

async function fusePreview() {
	if (fuseMode() === "full-res") {
		const output = await invoke("pick_output_dir");
		if (!output) return;
		await runFuse(output);
		return;
	}
	await runFuse(null);
}

async function fuseToFolder() {
	const output = await invoke("pick_output_dir");
	if (!output) return;
	await runFuse(output);
}

async function exportDngs(onlyMono = false) {
	if (!current) return;
	const output = await invoke("pick_output_dir");
	if (!output) return;

	const status = $("#export-status");
	const wrap = $("#export-progress-wrap");
	status.className = "status";
	status.textContent = "";
	wrap.classList.remove("hidden");
	setProgress(0, 1, "…");

	const unlisten = await listen("export-progress", (event) => {
		const { done, total, camera } = event.payload;
		setProgress(done, total, camera);
	});

	try {
		const count = await invoke("extract_lri", {
			input: current,
			output,
			jobs: null,
			onlyMono,
			monoPreviews: true,
		});
		status.textContent = onlyMono
			? `Done — ${count} mono DNG(s) → ${output}`
			: `Done — ${count} DNGs → ${output}`;
		status.classList.add("ok");
		setProgress(count, count, "done");
	} catch (e) {
		status.textContent = String(e);
		status.classList.add("err");
	} finally {
		unlisten();
	}
}

function libcpProfile() {
	const checked = document.querySelector('input[name="libcp-profile"]:checked');
	return Number(checked?.value ?? 1);
}

function dofFnumber() {
	const el = $("#dof-fnumber");
	if (!el) return null;
	const v = Number(el.value);
	// 0 means "default" — we use a dual-range trick: min 0, 0 = leave engine default
	if (!v || v < 2) return null;
	return v;
}

function updateDofLabels() {
	const f = dofFnumber();
	const val = $("#dof-fnumber-val");
	if (val) val.textContent = f == null ? "default" : `f/${f.toFixed(1)}`;
	const fl = $("#dof-focus-label");
	if (fl) {
		fl.textContent = dofFocus
			? `focus: (${dofFocus.x.toFixed(2)}, ${dofFocus.y.toFixed(2)})`
			: "focus: default";
	}
}

function libcpDofArgs() {
	const fnumber = dofFnumber();
	const depthMap = !!$("#dof-depth-map")?.checked;
	return {
		fnumber: fnumber ?? null,
		focusDepthMm: null,
		focusX: dofFocus?.x ?? null,
		focusY: dofFocus?.y ?? null,
		depthMap: depthMap || false,
	};
}

function setFocusMarker(nx, ny) {
	const marker = $("#focus-marker");
	const stage = $("#viewer-stage");
	const img = $("#libcp-preview");
	if (!marker || !stage || !img?.naturalWidth) return;
	if (nx == null || ny == null) {
		marker.classList.add("hidden");
		return;
	}
	// marker in image pixel space inside viewer-stage
	const x = nx * img.naturalWidth;
	const y = ny * img.naturalHeight;
	marker.style.left = `${x}px`;
	marker.style.top = `${y}px`;
	marker.classList.remove("hidden");
}

/** Map pointer event → normalized image coords [0,1], or null if outside image */
function pointerToImageNorm(e) {
	const img = $("#libcp-preview");
	const stage = $("#viewer-stage");
	if (!img?.naturalWidth || !stage) return null;
	const rect = img.getBoundingClientRect();
	if (rect.width <= 0 || rect.height <= 0) return null;
	const x = (e.clientX - rect.left) / rect.width;
	const y = (e.clientY - rect.top) / rect.height;
	if (x < 0 || x > 1 || y < 0 || y > 1) return null;
	return { x, y };
}

async function refreshLibcpStatus() {
	const pill = $("#libcp-pill");
	try {
		const st = await invoke("libcp_status");
		libcpReady = !!st.ok;
		if (!pill) return;
		if (st.ok) {
			pill.className = "libcp-pill online";
			pill.textContent = "libcp ready";
			pill.title = `${st.libcp}\nhelper: ${st.helper}`;
		} else {
			pill.className = "libcp-pill offline";
			pill.textContent = "libcp missing";
			pill.title = st.error ?? "Install Lumen.app or set LUMINAT_LIBCP_DIR";
		}
	} catch (e) {
		libcpReady = false;
		if (pill) {
			pill.className = "libcp-pill offline";
			pill.textContent = "libcp ?";
			pill.title = String(e);
		}
	}
}

// --- viewer zoom / pan ---
let viewScale = 1;
let viewX = 0;
let viewY = 0;
let panning = false;
let panStart = null;

function applyViewerTransform() {
	const stage = $("#viewer-stage");
	const label = $("#zoom-label");
	if (stage) {
		stage.style.transform = `translate(${viewX}px, ${viewY}px) scale(${viewScale})`;
	}
	if (label) label.textContent = `${Math.round(viewScale * 100)}%`;
}

function fitViewer() {
	const viewer = $("#libcp-viewer");
	const img = $("#libcp-preview");
	if (!viewer || !img || !img.naturalWidth) return;
	const vw = viewer.clientWidth;
	const vh = viewer.clientHeight;
	const sx = vw / img.naturalWidth;
	const sy = vh / img.naturalHeight;
	viewScale = Math.min(sx, sy, 1) * 0.98;
	viewX = (vw - img.naturalWidth * viewScale) / 2;
	viewY = (vh - img.naturalHeight * viewScale) / 2;
	applyViewerTransform();
}

function zoomViewer(factor, cx, cy) {
	const viewer = $("#libcp-viewer");
	if (!viewer) return;
	const rect = viewer.getBoundingClientRect();
	const px = (cx ?? rect.left + rect.width / 2) - rect.left;
	const py = (cy ?? rect.top + rect.height / 2) - rect.top;
	const prev = viewScale;
	viewScale = Math.min(8, Math.max(0.05, viewScale * factor));
	// zoom toward cursor
	viewX = px - (px - viewX) * (viewScale / prev);
	viewY = py - (py - viewY) * (viewScale / prev);
	applyViewerTransform();
}

function setupViewer() {
	const viewer = $("#libcp-viewer");
	const img = $("#libcp-preview");
	if (!viewer || !img || viewer.dataset.bound) return;
	viewer.dataset.bound = "1";

	viewer.addEventListener(
		"wheel",
		(e) => {
			e.preventDefault();
			const factor = e.deltaY < 0 ? 1.12 : 1 / 1.12;
			zoomViewer(factor, e.clientX, e.clientY);
		},
		{ passive: false },
	);

	let panMoved = false;
	viewer.addEventListener("pointerdown", (e) => {
		if (e.button !== 0) return;
		// Alt+click or Click-focus mode → set focus point (no pan)
		const focusMode = !!$("#dof-focus-mode")?.checked;
		if (e.altKey || focusMode) {
			const pt = pointerToImageNorm(e);
			if (pt) {
				dofFocus = pt;
				updateDofLabels();
				setFocusMarker(pt.x, pt.y);
				// re-render with new focus
				runLibcp(true);
			}
			return;
		}
		panMoved = false;
		panning = true;
		panStart = { x: e.clientX, y: e.clientY, vx: viewX, vy: viewY };
		viewer.classList.add("panning");
		viewer.setPointerCapture(e.pointerId);
	});
	viewer.addEventListener("pointermove", (e) => {
		if (!panning || !panStart) return;
		const dx = e.clientX - panStart.x;
		const dy = e.clientY - panStart.y;
		if (Math.abs(dx) + Math.abs(dy) > 3) panMoved = true;
		viewX = panStart.vx + dx;
		viewY = panStart.vy + dy;
		applyViewerTransform();
	});
	const endPan = (e) => {
		if (!panning) return;
		panning = false;
		panStart = null;
		viewer.classList.remove("panning");
		try {
			viewer.releasePointerCapture(e.pointerId);
		} catch {
			/* ignore */
		}
		void panMoved;
	};
	viewer.addEventListener("pointerup", endPan);
	viewer.addEventListener("pointercancel", endPan);

	img.addEventListener("load", () => {
		fitViewer();
		if (dofFocus) setFocusMarker(dofFocus.x, dofFocus.y);
	});
}

function showLibcpResult(res) {
	lastLibcp = res;
	const result = $("#libcp-result");
	const img = $("#libcp-preview");
	setupViewer();
	showingDepth = false;
	if (res.preview_data_url && img) {
		lastRgbPreview = res.preview_data_url;
		img.src = res.preview_data_url;
		if (img.complete) setTimeout(fitViewer, 0);
	}
	const dims =
		res.width && res.height ? `${res.width}×${res.height}` : "—";
	const profileLabel =
		res.profile === 3 ? "DESKTOP (canvas)" : res.profile === 1 ? "MOBILE ~13 MP" : `profile ${res.profile}`;
	const fLabel =
		res.fnumber != null ? `f/${Number(res.fnumber).toFixed(1)}` : "default";
	const focusLabel =
		res.focus_x != null && res.focus_y != null
			? `(${Number(res.focus_x).toFixed(2)}, ${Number(res.focus_y).toFixed(2)})`
			: res.focus_depth_mm != null
				? `${Number(res.focus_depth_mm).toFixed(0)} mm`
				: "default";
	$("#libcp-stats").innerHTML = [
		["Engine", "libcp / CIAPI"],
		["Profile", profileLabel],
		["Size", dims],
		["Aperture", fLabel],
		["Focus", focusLabel],
		["Cache", res.from_cache ? "hit" : "rendered"],
		["JPEG", res.jpg_path ?? "—"],
	]
		.map(
			([label, value]) => `
		<div class="fuse-stat">
			<div class="label">${label}</div>
			<div class="value" title="${value}">${value}</div>
		</div>`,
		)
		.join("");
	const depthThumb = $("#libcp-depth-thumb");
	const depthBtn = $("#btn-show-depth");
	if (res.depth_preview_data_url && depthThumb) {
		depthThumb.src = res.depth_preview_data_url;
		depthThumb.classList.remove("hidden");
		depthBtn?.classList.remove("hidden");
	} else {
		depthThumb?.classList.add("hidden");
		depthBtn?.classList.add("hidden");
	}
	if (res.focus_x != null && res.focus_y != null) {
		dofFocus = { x: res.focus_x, y: res.focus_y };
		updateDofLabels();
		// place marker after image has dimensions
		setTimeout(() => setFocusMarker(res.focus_x, res.focus_y), 50);
	} else {
		setFocusMarker(null, null);
	}
	result.classList.remove("hidden");
	$("#btn-libcp-export").disabled = !res.jpg_path;
	$("#btn-libcp-reveal").disabled = !res.jpg_path;
}

function toggleDepthView() {
	const img = $("#libcp-preview");
	const btn = $("#btn-show-depth");
	if (!img || !lastLibcp?.depth_preview_data_url) return;
	showingDepth = !showingDepth;
	if (showingDepth) {
		img.src = lastLibcp.depth_preview_data_url;
		btn?.classList.add("on");
	} else {
		img.src = lastRgbPreview || lastLibcp.preview_data_url;
		btn?.classList.remove("on");
	}
}

// --- setup wizard ---
function setSetupBadge(el, ok, text) {
	if (!el) return;
	el.className = `setup-badge ${ok ? "ok" : "bad"}`;
	el.textContent = text;
}

async function refreshSetupUi() {
	const st = await invoke("setup_state");
	setSetupBadge(
		$("#setup-libcp-state"),
		st.libcp_ok,
		st.libcp_ok ? "found" : "missing",
	);
	setSetupBadge(
		$("#setup-helper-state"),
		st.helper_ok,
		st.helper_ok ? "found" : "missing",
	);
	if ($("#setup-config-path")) $("#setup-config-path").textContent = st.config_path;
	const done = $("#setup-done");
	if (done) done.disabled = !(st.libcp_ok && st.helper_ok);
	libcpReady = !!(st.libcp && st.helper);
	await refreshLibcpStatus();
	return st;
}

async function maybeShowSetupWizard() {
	try {
		const st = await refreshSetupUi();
		if (st.needs_wizard) {
			$("#setup-modal")?.classList.remove("hidden");
		}
	} catch {
		/* ignore */
	}
}

async function setupPickLibcp() {
	const path = await invoke("pick_libcp_location");
	if (!path) return;
	try {
		await invoke("set_libcp_dir", { path });
		await refreshSetupUi();
	} catch (e) {
		alert(String(e));
	}
}

async function setupPickHelper() {
	const path = await invoke("pick_helper_file");
	if (!path) return;
	try {
		await invoke("set_libcp_export_path", { path });
		await refreshSetupUi();
	} catch (e) {
		alert(String(e));
	}
}

async function setupSkip() {
	await invoke("dismiss_setup_wizard");
	$("#setup-modal")?.classList.add("hidden");
}

async function setupContinue() {
	const st = await refreshSetupUi();
	if (st.libcp_ok && st.helper_ok) {
		$("#setup-modal")?.classList.add("hidden");
	}
}

async function runLibcp(force = false) {
	if (!current || libcpBusy) return;
	if (!libcpReady) {
		await refreshLibcpStatus();
		if (!libcpReady) {
			$("#setup-modal")?.classList.remove("hidden");
			await refreshSetupUi();
			const status = $("#libcp-status");
			status.className = "status err";
			status.textContent =
				"libcp not ready — complete setup (Lumen.app Frameworks + libcp-export)";
			return;
		}
	}

	let profile = libcpProfile();
	const dof = libcpDofArgs();
	// DepthEditor is Desktop-only (matches helper + light::libcp)
	if ((dof.depthMap || dof.focusX != null) && profile < 3) {
		profile = 3;
	}
	const status = $("#libcp-status");
	const wrap = $("#libcp-progress-wrap");
	const label = $("#libcp-progress-label");

	status.className = "status";
	status.textContent = profile === 3 ? "Desktop render (~10–20s)…" : "Preview render…";
	wrap.classList.remove("hidden");
	label.textContent =
		profile === 3
			? "libcp DESKTOP · Rosetta · 10432×7824…"
			: "libcp MOBILE · Rosetta · 4160×3120…";
	$("#libcp-result").classList.add("hidden");
	libcpBusy = true;
	$("#btn-libcp").disabled = true;
	$("#btn-libcp-export").disabled = true;
	$("#btn-libcp-reveal").disabled = true;

	try {
		const res = await invoke("libcp_lri", {
			input: current,
			output: null,
			profile,
			format: "jpg",
			useCache: !force,
			fnumber: dof.fnumber,
			focusDepthMm: dof.focusDepthMm,
			focusX: dof.focusX,
			focusY: dof.focusY,
			depthMap: dof.depthMap,
		});
		showLibcpResult(res);
		const bits = [
			res.from_cache ? "Cached" : "Done",
			`${res.width ?? "?"}×${res.height ?? "?"}`,
			`p${res.profile}`,
		];
		if (res.fnumber != null) bits.push(`f/${res.fnumber}`);
		if (res.focus_x != null) bits.push("refocus");
		if (res.depth_jpg_path) bits.push("depth");
		status.textContent = bits.join(" · ");
		status.classList.add("ok");
	} catch (e) {
		status.textContent = String(e);
		status.classList.add("err");
	} finally {
		libcpBusy = false;
		$("#btn-libcp").disabled = false;
		wrap.classList.add("hidden");
	}
}

async function exportLibcpJpeg() {
	if (!lastLibcp?.jpg_path) return;
	const dir = await invoke("pick_output_dir");
	if (!dir) return;
	try {
		const dest = await invoke("export_jpeg_copy", {
			source: lastLibcp.jpg_path,
			destDir: dir,
		});
		const status = $("#libcp-status");
		status.className = "status ok";
		status.textContent = `Exported → ${dest}`;
	} catch (e) {
		const status = $("#libcp-status");
		status.className = "status err";
		status.textContent = String(e);
	}
}

async function revealLibcp() {
	if (!lastLibcp?.jpg_path) return;
	try {
		await invoke("reveal_path", { path: lastLibcp.jpg_path });
	} catch (e) {
		$("#libcp-status").className = "status err";
		$("#libcp-status").textContent = String(e);
	}
}

function pickLriFromPaths(paths) {
	if (!paths?.length) return null;
	const hit = paths.find((p) => p.toLowerCase().endsWith(".lri"));
	return hit ?? null;
}

async function handleFuseDrop(paths) {
	const { lris, dirs } = classifyDropPaths(paths);
	const outputDir = dirs[0] ?? null;
	let lri = lris[0] ?? null;

	if (!lri && outputDir) {
		const scan = await invoke("scan_directory", { path: outputDir });
		lri = scan.files[0]?.path ?? null;
	}

	if (!lri && current && outputDir) {
		await runFuse(outputDir);
		return;
	}

	if (!lri) return;

	await selectFile(lri);
	if (!files.some((f) => f.path === lri)) {
		files = [await invoke("inspect_lri", { path: lri })];
		renderFileList();
	}

	if (outputDir) {
		await runFuse(outputDir);
		return;
	}

	if (fuseMode() === "full-res") {
		const picked = await invoke("pick_output_dir");
		if (picked) await runFuse(picked);
		return;
	}

	await runFuse(null);
}

async function setupDragDrop() {
	const overlay = $("#drop-overlay");
	const win = getCurrentWindow();

	await win.onDragDropEvent(async (event) => {
		const { type, paths, position } = event.payload;
		if (type === "over" || type === "enter") {
			const onFuse = await hitFuseZone(position);
			setFuseDropActive(onFuse);
			overlay.classList.remove("hidden");
		} else if (type === "leave") {
			setFuseDropActive(false);
			overlay.classList.add("hidden");
		} else if (type === "drop") {
			setFuseDropActive(false);
			overlay.classList.add("hidden");
			const onFuse = await hitFuseZone(position);
			if (onFuse) {
				await handleFuseDrop(paths);
				return;
			}

			const lri = pickLriFromPaths(paths);
			if (lri) {
				await selectFile(lri);
				files = [await invoke("inspect_lri", { path: lri })];
				renderFileList();
				return;
			}

			const { dirs } = classifyDropPaths(paths);
			if (dirs[0]) {
				await loadDirectory(dirs[0]);
			}
		}
	});
}

document.querySelectorAll('input[name="fuse-mode"]').forEach((el) => {
	el.addEventListener("change", updateFuseOptions);
});

// --- M2: camera + batch ---

let camStatus = null;
/** @type {{ name: string, remote_path: string, size: number, mtime?: string }[]} */
let remoteList = [];
/** @type {Set<string>} */
let selectedRemote = new Set();
let camBusy = false;

function fmtSize(n) {
	if (!n) return "—";
	if (n >= 1e9) return `${(n / 1e9).toFixed(2)} GB`;
	if (n >= 1e6) return `${(n / 1e6).toFixed(0)} MB`;
	return `${(n / 1e3).toFixed(0)} KB`;
}

function cameraLabel(dev) {
	if (!dev) return "camera";
	// adb model is already "L16" — don't double "L16 L16"
	const serial = dev.serial || "";
	const short = serial.length > 8 ? serial.slice(-6) : serial;
	return short ? `Light · ${short}` : "Light L16";
}

async function refreshCameraStatus() {
	const pill = $("#cam-pill");
	try {
		camStatus = await invoke("camera_status");
		const online = !!camStatus?.light;
		if (pill) {
			pill.className = `cam-pill ${online ? "online" : "offline"}`;
			pill.textContent = online
				? cameraLabel(camStatus.light)
				: camStatus?.adb_ok
					? "camera offline"
					: "no adb";
			pill.title = online
				? `Light L16 · ${camStatus.light.serial}${camStatus.light.product ? ` · ${camStatus.light.product}` : ""}`
				: camStatus?.error || "Plug Light L16 (USB debugging)";
		}
	} catch (e) {
		camStatus = null;
		if (pill) {
			pill.className = "cam-pill offline";
			pill.textContent = "camera ?";
			pill.title = String(e);
		}
	}
}

function setCamModal(open) {
	$("#cam-modal")?.classList.toggle("hidden", !open);
}

/** @type {Map<string, string>} name → data URL */
const remotePreviewCache = new Map();
let remotePreviewGen = 0;

function renderRemoteList() {
	const body = $("#cam-modal-body");
	const count = $("#cam-modal-count");
	const pullBtn = $("#cam-pull");
	if (!body) return;

	if (!remoteList.length) {
		body.innerHTML = `<p class="modal-empty">No .lri on camera (or list failed)</p>`;
		if (count) count.textContent = "0 captures";
		if (pullBtn) pullBtn.disabled = true;
		return;
	}

	if (count) count.textContent = `${remoteList.length} captures · ${selectedRemote.size} selected`;
	if (pullBtn && !camBusy) {
		pullBtn.disabled = selectedRemote.size === 0;
		const lbl = $("#cam-pull-label");
		if (lbl) lbl.textContent = "Pull selected";
	}

	body.innerHTML = `<ul class="remote-list">${remoteList
		.map((r) => {
			const on = selectedRemote.has(r.name);
			const cached = remotePreviewCache.get(r.name);
			const num = (r.name.match(/(\d+)/) || [])[1] || "";
			const thumb = cached
				? `<img class="r-thumb" src="${cached}" alt="" draggable="false" />`
				: r.has_preview
					? `<div class="r-thumb r-thumb-loading" data-thumb-for="${r.name}"><span class="btn-spinner"></span></div>`
					: `<div class="r-thumb r-thumb-empty" title="No companion JPEG on camera">·</div>`;
			return `<li>
				<label class="remote-row ${on ? "on" : ""}">
					<input type="checkbox" data-name="${r.name}" ${on ? "checked" : ""} />
					${thumb}
					<span class="r-text">
						<span class="r-name">${r.name}${num ? ` <span class="r-num">#${num}</span>` : ""}</span>
						<span class="r-meta">${fmtSize(r.size)}${r.mtime ? ` · ${r.mtime}` : ""}${r.has_preview ? "" : " · no jpg"}</span>
					</span>
				</label>
			</li>`;
		})
		.join("")}</ul>`;

	body.querySelectorAll("input[type=checkbox]").forEach((el) => {
		el.addEventListener("change", () => {
			const name = el.dataset.name;
			if (el.checked) selectedRemote.add(name);
			else selectedRemote.delete(name);
			// update selection chrome without full re-render (keeps thumbs)
			const row = el.closest(".remote-row");
			row?.classList.toggle("on", el.checked);
			if (count) {
				count.textContent = `${remoteList.length} captures · ${selectedRemote.size} selected`;
			}
			if (pullBtn && !camBusy) {
				pullBtn.disabled = selectedRemote.size === 0;
			}
		});
	});

	// click on row (not only checkbox) toggles select
	body.querySelectorAll(".remote-row").forEach((row) => {
		row.addEventListener("click", (e) => {
			if (e.target.matches("input[type=checkbox]")) return;
			const cb = row.querySelector("input[type=checkbox]");
			if (!cb || camBusy) return;
			cb.checked = !cb.checked;
			cb.dispatchEvent(new Event("change"));
		});
	});

	loadRemotePreviews();
}

async function loadRemotePreviews() {
	const gen = ++remotePreviewGen;
	const serial = camStatus?.light?.serial ?? null;
	const need = remoteList.filter(
		(r) => r.has_preview && !remotePreviewCache.has(r.name),
	);
	// sequential-ish with small concurrency — USB is the bottleneck
	const concurrency = 2;
	let i = 0;
	async function worker() {
		while (i < need.length) {
			if (gen !== remotePreviewGen) return;
			const idx = i++;
			const r = need[idx];
			try {
				const url = await invoke("camera_lri_preview", {
					name: r.name,
					serial,
					maxSide: 160,
				});
				if (gen !== remotePreviewGen) return;
				remotePreviewCache.set(r.name, url);
				const slot = document.querySelector(`[data-thumb-for="${r.name}"]`);
				if (slot) {
					const img = document.createElement("img");
					img.className = "r-thumb";
					img.src = url;
					img.alt = "";
					img.draggable = false;
					slot.replaceWith(img);
				}
			} catch {
				if (gen !== remotePreviewGen) return;
				const slot = document.querySelector(`[data-thumb-for="${r.name}"]`);
				if (slot) {
					slot.classList.remove("r-thumb-loading");
					slot.classList.add("r-thumb-empty");
					slot.innerHTML = "";
					slot.title = "preview failed";
				}
			}
		}
	}
	await Promise.all(Array.from({ length: concurrency }, () => worker()));
}

async function openCameraModal() {
	if (camBusy) return;
	setCamModal(true);
	selectedRemote = new Set();
	$("#cam-modal-body").innerHTML = `<p class="modal-empty">Listing…</p>`;
	$("#cam-pull-progress")?.classList.add("hidden");
	$("#cam-pull-progress")?.classList.remove("done-ok", "done-err", "is-busy");
	await refreshCameraStatus();
	const sub = $("#cam-modal-sub");
	if (camStatus?.light) {
		sub.textContent = `Light L16 · ${camStatus.light.serial} · DCIM/Camera`;
	} else {
		sub.textContent = "no device · plug Light L16 with USB debugging";
	}
	try {
		remoteList = await invoke("list_camera_lri", {
			serial: camStatus?.light?.serial ?? null,
		});
		renderRemoteList();
	} catch (e) {
		remoteList = [];
		$("#cam-modal-body").innerHTML = `<p class="modal-empty" style="color:#f0a8a8">${String(e)}</p>`;
		$("#cam-modal-count").textContent = "error";
		$("#cam-pull").disabled = true;
	}
}

function setCamPullBusy(busy, label) {
	camBusy = busy;
	const btn = $("#cam-pull");
	const spinner = $("#cam-pull-spinner");
	const btnLabel = $("#cam-pull-label");
	const progress = $("#cam-pull-progress");
	const bar = $("#cam-pull-bar");
	const modal = $("#cam-modal .modal");

	if (btn) {
		btn.disabled = busy || selectedRemote.size === 0;
		btn.classList.toggle("is-busy", busy);
		btn.setAttribute("aria-busy", busy ? "true" : "false");
	}
	if (spinner) spinner.classList.toggle("hidden", !busy);
	if (btnLabel) btnLabel.textContent = busy ? label || "Pulling…" : "Pull selected";

	// lock secondary controls so double-clicks don't restart work
	for (const id of ["cam-cancel", "cam-refresh", "cam-select-all", "cam-select-none", "cam-modal-close"]) {
		const el = $(`#${id}`);
		if (el) el.disabled = busy;
	}
	const also = $("#cam-also-render");
	if (also) also.disabled = busy;

	modal?.classList.toggle("is-pulling", busy);
	if (progress) {
		if (busy) {
			progress.classList.remove("hidden", "done-ok", "done-err");
			progress.classList.add("is-busy");
		} else {
			progress.classList.remove("is-busy");
		}
	}
	if (bar) {
		bar.classList.toggle("indeterminate", busy);
		if (!busy) bar.style.width = "100%";
		else bar.style.width = "";
	}
}

function setCamPullStatus(text, kind) {
	const status = $("#cam-pull-status");
	const progress = $("#cam-pull-progress");
	if (status) status.textContent = text;
	if (progress) {
		progress.classList.remove("hidden", "done-ok", "done-err");
		if (kind === "ok") progress.classList.add("done-ok");
		if (kind === "err") progress.classList.add("done-err");
	}
}

async function pullSelectedFromCamera() {
	if (camBusy || !selectedRemote.size) return;

	const picks = remoteList.filter((r) => selectedRemote.has(r.name));
	const alsoRender = $("#cam-also-render")?.checked;
	const serial = camStatus?.light?.serial ?? null;
	const pulledPaths = [];
	let hadError = false;

	setCamPullBusy(true, `Pulling 0/${picks.length}…`);
	setCamPullStatus(
		`Starting pull · ${picks.length} file(s) · USB ~2–3 MB/s — do not unplug`,
	);

	for (let i = 0; i < picks.length; i++) {
		const r = picks[i];
		const mb = r.size ? ` · ~${(r.size / 1e6).toFixed(0)} MB` : "";
		setCamPullBusy(true, `Pulling ${i + 1}/${picks.length}…`);
		setCamPullStatus(`[${i + 1}/${picks.length}] ${r.name}${mb}…`);
		try {
			const res = await invoke("pull_camera_lri", {
				remotePath: r.remote_path,
				name: r.name,
				size: r.size,
				serial,
				destDir: null,
			});
			pulledPaths.push(res.local_path);
			// add to library list
			try {
				const summary = await invoke("inspect_lri", { path: res.local_path });
				if (!files.some((f) => f.path === summary.path)) {
					files = [...files, summary];
				}
			} catch {
				/* ignore inspect fail */
			}
		} catch (e) {
			hadError = true;
			setCamPullStatus(`${r.name}: ${e}`);
		}
	}

	files.sort((a, b) => (a.name || "").localeCompare(b.name || ""));
	renderFileList();

	if (alsoRender && pulledPaths.length) {
		setCamPullBusy(true, "Rendering…");
		setCamPullStatus(`Rendering ${pulledPaths.length} via libcp (Rosetta)…`);
		try {
			const batch = await invoke("batch_libcp", {
				paths: pulledPaths,
				profile: libcpProfile(),
				useCache: true,
			});
			hadError = hadError || !!batch.fail_count;
			setCamPullStatus(
				`Pull+render: ${batch.ok_count} ok, ${batch.fail_count} fail`,
				batch.fail_count ? "err" : "ok",
			);
		} catch (e) {
			hadError = true;
			setCamPullStatus(String(e), "err");
		}
	} else if (!hadError) {
		setCamPullStatus(
			`Pulled ${pulledPaths.length} of ${picks.length} → library cache`,
			pulledPaths.length ? "ok" : "err",
		);
	} else if (pulledPaths.length) {
		setCamPullStatus(
			`Pulled ${pulledPaths.length} of ${picks.length} (with errors) → library cache`,
			"err",
		);
	}

	if (pulledPaths[0]) {
		await selectFile(pulledPaths[0]);
	}

	setCamPullBusy(false);
	// keep progress panel visible with final status
}

async function batchRenderLibrary() {
	if (!files.length) return;
	const paths = files.map((f) => f.path).filter(Boolean);
	if (!paths.length) return;
	const el = $("#batch-status");
	el.classList.remove("hidden", "ok", "err");
	el.textContent = `Batch libcp p${libcpProfile()} · ${paths.length} files…`;
	$("#btn-batch-render").disabled = true;

	const unlisten = await listen("batch-libcp-progress", (ev) => {
		const p = ev.payload;
		el.textContent = `[${p.index}/${p.total}] ${p.file} · ${p.message}`;
	});

	try {
		const batch = await invoke("batch_libcp", {
			paths,
			profile: libcpProfile(),
			useCache: true,
		});
		el.classList.add(batch.fail_count ? "err" : "ok");
		el.textContent = `Batch done: ${batch.ok_count} ok, ${batch.fail_count} fail (p${libcpProfile()})`;
	} catch (e) {
		el.classList.add("err");
		el.textContent = String(e);
	} finally {
		unlisten();
		$("#btn-batch-render").disabled = false;
	}
}

$("#btn-open-file").addEventListener("click", () => openFile().catch(console.error));
$("#btn-open-dir").addEventListener("click", () => openDir().catch(console.error));
$("#btn-export").addEventListener("click", () => exportDngs(false).catch(console.error));
$("#btn-export-mono")?.addEventListener("click", () => exportDngs(true).catch(console.error));
$("#btn-export-mono-2")?.addEventListener("click", () => exportDngs(true).catch(console.error));
$("#btn-fuse").addEventListener("click", () => fusePreview().catch(console.error));
$("#btn-libcp")?.addEventListener("click", () => runLibcp(false).catch(console.error));
$("#btn-libcp-export")?.addEventListener("click", () => exportLibcpJpeg().catch(console.error));
$("#btn-libcp-reveal")?.addEventListener("click", () => revealLibcp().catch(console.error));
$("#btn-show-depth")?.addEventListener("click", () => toggleDepthView());
$("#dof-fnumber")?.addEventListener("input", () => updateDofLabels());
// double-click slider left edge already 0; right-click resets to default
$("#dof-fnumber")?.addEventListener("dblclick", () => {
	const el = $("#dof-fnumber");
	if (el) {
		el.value = "0";
		updateDofLabels();
	}
});
$("#btn-fuse-export").addEventListener("click", () => fuseToFolder().catch(console.error));
$("#libcp-pill")?.addEventListener("click", () => {
	refreshLibcpStatus()
		.then(async () => {
			if (!libcpReady) {
				$("#setup-modal")?.classList.remove("hidden");
				await refreshSetupUi();
			}
		})
		.catch(console.error);
});
$("#zoom-in")?.addEventListener("click", () => zoomViewer(1.2));
$("#zoom-out")?.addEventListener("click", () => zoomViewer(1 / 1.2));
$("#zoom-fit")?.addEventListener("click", () => fitViewer());
$("#zoom-100")?.addEventListener("click", () => {
	viewScale = 1;
	viewX = 0;
	viewY = 0;
	applyViewerTransform();
});
$("#setup-pick-libcp")?.addEventListener("click", () => setupPickLibcp().catch(console.error));
$("#setup-pick-helper")?.addEventListener("click", () => setupPickHelper().catch(console.error));
$("#setup-skip")?.addEventListener("click", () => setupSkip().catch(console.error));
$("#setup-done")?.addEventListener("click", () => setupContinue().catch(console.error));
$("#cam-pill")?.addEventListener("click", () => openCameraModal().catch(console.error));
$("#btn-from-camera")?.addEventListener("click", () => openCameraModal().catch(console.error));
$("#cam-modal-close")?.addEventListener("click", () => setCamModal(false));
$("#cam-cancel")?.addEventListener("click", () => setCamModal(false));
$("#cam-refresh")?.addEventListener("click", () => openCameraModal().catch(console.error));
$("#cam-select-all")?.addEventListener("click", () => {
	selectedRemote = new Set(remoteList.map((r) => r.name));
	renderRemoteList();
});
$("#cam-select-none")?.addEventListener("click", () => {
	selectedRemote = new Set();
	renderRemoteList();
});
$("#cam-pull")?.addEventListener("click", () => pullSelectedFromCamera().catch(console.error));
$("#btn-batch-render")?.addEventListener("click", () => batchRenderLibrary().catch(console.error));
// click backdrop to close
$("#cam-modal")?.addEventListener("click", (e) => {
	if (e.target === $("#cam-modal") && !camBusy) setCamModal(false);
});

updateFuseOptions();
refreshLibcpStatus().catch(console.error);
refreshCameraStatus().catch(console.error);
maybeShowSetupWizard().catch(console.error);
setInterval(() => refreshCameraStatus().catch(() => {}), 5000);
setupDragDrop().catch(console.error);