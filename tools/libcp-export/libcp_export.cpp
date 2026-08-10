/* libcp-export — Light L16 .lri → PPM via CIAPI (libcp.dylib), no Lumen UI.
 *
 * Must be built and run as x86_64 (Rosetta on Apple Silicon). libcp is x86_64 only.
 *
 * Build (from repo root):
 *   make libcp-export
 *
 * Run:
 *   arch -x86_64 ./tools/libcp-export/libcp-export \
 *     /path/to/libcp.dylib input.lri out.ppm [profile=1] \
 *     [fnumber=-1] [focus_depth_mm=-1] [fx=-1] [fy=-1] [depth.ppm]
 *
 * DOF (gallery JNI RE, FUSION.md):
 *   ParamFloat(1) = ViewDofFocusDepth  (nativeSetDofDepth)
 *   ParamFloat(3) = ViewDofFNumber     (EditProperty.APERTURE → toParamFloat)
 *   fx,fy in [0,1] image-normalized → sample DepthEditor after first render,
 *   then set focus depth and re-render.
 *
 * DepthEditor is a thin shared_ptr wrapper (16 bytes on x86_64); ctor:
 *   CIAPI::DepthEditor::DepthEditor(CIAPI::Renderer&)
 * getDepthAtPoint(Point<float> const&) — Point is {float x, float y}.
 *
 * Env:
 *   LIBCP_MAX_WAIT_MS  total poll budget (default 90000; DESKTOP 180000)
 *
 * ROI is x0,y0,x1,y1 — not width/height.
 */
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <dlfcn.h>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>

struct Renderer {
  void *vptr;
  void *ptr;
  void *ctrl;
};

struct ShPtr {
  void *ptr;
  void *ctrl;
};

struct ROI {
  int x0, y0, x1, y1;
};

struct PointF {
  float x, y;
};

// DepthEditor shell: shared_ptr-like (ptr + control block). Ctor zeros both and
// stores impl at offset 0. Measured via disasm of C2 (movups zero, store pair).
struct DepthEditorShell {
  void *impl;
  void *ctrl;
};

// CIAPI::ParamFloat ordinals (int).
// Name table from gallery libnative-lib TraceRenderer relocs (R13 research):
//   0 ViewDofFNumber  1 ViewDofFocusDepth  2 ViewExposure
//   3 ViewColorTemperature  4 ViewColorTint  5 ViewShadowBoost
//   6 ViewHighlightBoost  7 ViewContrast  8 ViewSaturation
//   9 ViewVibrance  10 ViewClarity  11 ViewBlacks  12 ViewWhites
//  13 ViewSharpening  14..19 Preferred*/Capture*/MaxInFocusBlur
// nativeSetDofDepth hardcodes ParamFloat(1). Earlier M4 mistakenly used 3 for FNumber
// (that is ColorTemperature) — fixed here.
enum ParamFloatId : int {
  kPfViewDofFNumber = 0,
  kPfViewDofFocusDepth = 1,
  kPfViewExposure = 2,
  kPfViewColorTemperature = 3,
  kPfViewContrast = 7,
  kPfViewSaturation = 8,
  kPfViewVibrance = 9,
  kPfViewClarity = 10,
  kPfViewSharpening = 13,
};

using fn_GetVersion = const char *(*)();
using fn_IsHW = int (*)();
using fn_Create = Renderer *(*)(Renderer *out, int profile);
using fn_setInput = void (*)(Renderer *, const void *, unsigned long);
using fn_setPropFloat = void (*)(Renderer *, int param, float value);
using fn_getPropFloat = float (*)(const Renderer *, int param);
using fn_DepthEditor_ctor = void (*)(DepthEditorShell *self, Renderer *r);
using fn_getDepthAtPoint = float (*)(DepthEditorShell *self, const PointF *pt);
using fn_render = void (*)(Renderer *, int level, const ROI *, int renderType, bool flag);
using fn_outputBuffer = ShPtr *(*)(ShPtr *out, const Renderer *self);
using fn_levelCount = int (*)(const ShPtr *);
using fn_at = ShPtr *(*)(const ShPtr *pyr, int level);
using fn_w = int (*)(const ShPtr *);
using fn_h = int (*)(const ShPtr *);
using fn_stride = int (*)(const ShPtr *);
using fn_data = void *(*)(const ShPtr *);

static void *must(void *h, const char *n) {
  void *s = dlsym(h, n);
  if (!s) fprintf(stderr, "MISSING %s\n", n);
  return s;
}

// Returns true when buffer looks *finished* enough to dump.
// Large DESKTOP canvases fill progressively — early tiles give nz>20 while
// most of the frame is still black; require high coverage for big images.
static bool buffer_has_signal(const unsigned char *p, int w, int h, int stride) {
  if (!p || w <= 0 || h <= 0 || stride <= 0) return false;
  long nz = 0, n = 0;
  int step = (w > 64) ? 8 : 1;
  long center_nz = 0, center_n = 0;
  int cx0 = w / 4, cx1 = 3 * w / 4, cy0 = h / 4, cy1 = 3 * h / 4;
  for (int y = 0; y < h; y += step) {
    const unsigned char *row = p + (size_t)y * stride;
    int bpp = stride / w;
    for (int x = 0; x < w; x += step) {
      n++;
      bool hit = false;
      if (bpp >= 16) {
        const float *fr = reinterpret_cast<const float *>(row);
        float R = fr[x * 4], G = fr[x * 4 + 1], B = fr[x * 4 + 2];
        if ((R == R) && (G == G) && (B == B) && (R + G + B > 1e-4f)) hit = true;
      } else if (bpp == 4) {
        float v = *reinterpret_cast<const float *>(row + (size_t)x * 4);
        if (v == v && v > 1e-4f && v < 1e6f) hit = true;
        else {
          const unsigned char *u = row + (size_t)x * 4;
          if (u[0] | u[1] | u[2]) hit = true;
        }
      }
      if (hit) {
        nz++;
        if (x >= cx0 && x < cx1 && y >= cy0 && y < cy1) center_nz++;
      }
      if (x >= cx0 && x < cx1 && y >= cy0 && y < cy1) center_n++;
    }
  }
  double frac = n ? (double)nz / (double)n : 0;
  double cfrac = center_n ? (double)center_nz / (double)center_n : 0;
  long pixels = (long)w * (long)h;
  double need = 0.15;
  if (pixels > 20'000'000)
    need = 0.55;
  else if (pixels > 2'000'000)
    need = 0.40;
  bool ok = (nz > 20) && (frac >= need) && (pixels <= 20'000'000 || cfrac >= 0.40);
  fprintf(stderr,
          "signal: %ld / %ld (%.1f%%) center=%.1f%% need=%.0f%% → %s\n", nz, n, 100.0 * frac,
          100.0 * cfrac, 100.0 * need, ok ? "ACCEPT" : "wait");
  return ok;
}

static bool write_ppm(const char *path, const unsigned char *p, int w, int h, int stride) {
  FILE *f = fopen(path, "wb");
  if (!f) {
    perror(path);
    return false;
  }
  fprintf(f, "P6\n%d %d\n255\n", w, h);
  int bpp = stride / w;
  for (int y = 0; y < h; y++) {
    const unsigned char *row = p + (size_t)y * stride;
    for (int x = 0; x < w; x++) {
      unsigned char rgb[3] = {0, 0, 0};
      if (bpp >= 16) {
        const float *fr = reinterpret_cast<const float *>(row);
        float R = fr[x * 4], G = fr[x * 4 + 1], B = fr[x * 4 + 2];
        auto clamp = [](float v) -> unsigned char {
          if (!(v == v) || v < 0) v = 0;
          if (v > 1) v = 1;
          return (unsigned char)(v * 255.f + 0.5f);
        };
        rgb[0] = clamp(R);
        rgb[1] = clamp(G);
        rgb[2] = clamp(B);
      } else if (bpp == 4) {
        float v = *reinterpret_cast<const float *>(row + (size_t)x * 4);
        if (v == v && v >= 0.f && v <= 4.f) {
          if (v > 1.f) v = 1.f;
          unsigned char g = (unsigned char)(v * 255.f + 0.5f);
          rgb[0] = rgb[1] = rgb[2] = g;
        } else {
          rgb[0] = row[x * 4];
          rgb[1] = row[x * 4 + 1];
          rgb[2] = row[x * 4 + 2];
        }
      }
      fwrite(rgb, 1, 3, f);
    }
  }
  fclose(f);
  return true;
}

static bool write_depth_ppm(const char *path, const std::vector<float> &z, int w, int h) {
  if (z.empty() || w <= 0 || h <= 0) return false;
  float zmin = 1e30f, zmax = -1e30f;
  long valid = 0;
  for (float v : z) {
    if (!(v == v) || v <= 0.f || v > 1e7f) continue;
    valid++;
    if (v < zmin) zmin = v;
    if (v > zmax) zmax = v;
  }
  if (valid < 4 || !(zmax > zmin)) {
    fprintf(stderr, "depth map: only %ld valid samples (need range)\n", valid);
    return false;
  }
  FILE *f = fopen(path, "wb");
  if (!f) return false;
  fprintf(f, "P6\n%d %d\n255\n", w, h);
  for (int i = 0; i < w * h; i++) {
    float v = z[i];
    unsigned char r = 0, g = 0, b = 0;
    if (v == v && v > 0.f && v <= 1e7f) {
      float t = (v - zmin) / (zmax - zmin);
      if (t < 0) t = 0;
      if (t > 1) t = 1;
      // turbo-ish: near=yellow/white, far=blue/black
      float inv = 1.f - t;
      r = (unsigned char)(inv * 255.f + 0.5f);
      g = (unsigned char)((inv * 0.7f + 0.15f) * 255.f + 0.5f);
      b = (unsigned char)(t * 200.f + 0.5f);
    }
    unsigned char rgb[3] = {r, g, b};
    fwrite(rgb, 1, 3, f);
  }
  fclose(f);
  fprintf(stderr, "depth range mm: %.1f .. %.1f (%ld valid) → %s\n", zmin, zmax, valid, path);
  printf("DEPTH wrote %s (%dx%d) z=%.1f..%.1f\n", path, w, h, zmin, zmax);
  fflush(stdout);
  return true;
}

static int try_dump(fn_outputBuffer outputBuffer, fn_levelCount levelCount, fn_at at, fn_w width,
                    fn_h height, fn_stride stride, fn_data data, Renderer *r, const char *outpath) {
  ShPtr pyr;
  memset(&pyr, 0, sizeof pyr);
  outputBuffer(&pyr, r);
  if (!pyr.ptr) return 0;
  int n = levelCount(&pyr);
  if (n <= 0) return 0;
  for (int li = 0; li < n; li++) {
    ShPtr *img = at(&pyr, li);
    if (!img || !img->ptr) continue;
    int w = width(img), ht = height(img), s = stride(img);
    unsigned char *px = (unsigned char *)data(img);
    if (!px || w <= 0 || ht <= 0 || s <= 0) continue;
    fprintf(stderr, "  L%d: %dx%d stride=%d bpp=%d\n", li, w, ht, s, s / w);
    if (buffer_has_signal(px, w, ht, s)) {
      if (write_ppm(outpath, px, w, ht, s)) {
        printf("SUCCESS wrote %s (%dx%d)\n", outpath, w, ht);
        fflush(stdout);
        return 1;
      }
    }
  }
  return 0;
}

// libcp throws std::runtime_error for illegal DOF ops (no depth yet / wrong profile).
static bool set_float_prop(fn_setPropFloat setPropF, fn_getPropFloat getPropF, Renderer *r, int id,
                           float value, const char *name) {
  if (!setPropF) return false;
  try {
    setPropF(r, id, value);
    float got = getPropF ? getPropF(r, id) : value;
    fprintf(stderr, "set %s(ParamFloat=%d)=%.4g → get %.4g\n", name, id, value, got);
    return true;
  } catch (const std::exception &e) {
    fprintf(stderr, "set %s(ParamFloat=%d)=%.4g FAILED: %s\n", name, id, value, e.what());
    return false;
  } catch (...) {
    fprintf(stderr, "set %s(ParamFloat=%d)=%.4g FAILED: unknown exception\n", name, id, value);
    return false;
  }
}

static void apply_dof(fn_setPropFloat setPropF, fn_getPropFloat getPropF, Renderer *r, float fnumber,
                      float focus_depth_mm) {
  if (!setPropF) return;
  if (fnumber >= 2.f && fnumber <= 15.f) {
    set_float_prop(setPropF, getPropF, r, kPfViewDofFNumber, fnumber, "ViewDofFNumber");
  }
  if (focus_depth_mm > 0.f) {
    set_float_prop(setPropF, getPropF, r, kPfViewDofFocusDepth, focus_depth_mm,
                   "ViewDofFocusDepth");
  }
}

int main(int argc, char **argv) {
  if (argc < 4) {
    fprintf(stderr,
            "usage: %s libcp.dylib input.lri out.ppm [profile=1] "
            "[fnumber=-1] [focus_depth_mm=-1] [fx=-1] [fy=-1] [depth.ppm]\n"
            "  x86_64 only; put libceres.dylib next to libcp or in DYLD_LIBRARY_PATH\n"
            "  fnumber 2..15 (0 or -1 = leave default)\n"
            "  focus_depth_mm >0 sets DOF focus plane\n"
            "  fx,fy in 0..1: after first render sample depth, set focus, re-render\n"
            "  depth.ppm: write low-res depth colormap after render\n",
            argv[0]);
    return 1;
  }
  const char *libpath = argv[1];
  const char *lripath = argv[2];
  const char *outpath = argv[3];
  int profile = argc > 4 ? atoi(argv[4]) : 1;
  float fnumber = argc > 5 ? (float)atof(argv[5]) : -1.f;
  float focus_depth_mm = argc > 6 ? (float)atof(argv[6]) : -1.f;
  float fx = argc > 7 ? (float)atof(argv[7]) : -1.f;
  float fy = argc > 8 ? (float)atof(argv[8]) : -1.f;
  const char *depth_path = argc > 9 ? argv[9] : nullptr;

  bool want_click_focus = (fx >= 0.f && fx <= 1.f && fy >= 0.f && fy <= 1.f);
  bool want_depth_map = (depth_path && depth_path[0]);

  // Depth / DOF: mobile profile throws "This profile does not support depth!" on FNumber;
  // DepthEditor requires Desktop. Upgrade when any DOF feature requested.
  bool want_fnumber = (fnumber >= 2.f && fnumber <= 15.f);
  bool want_focus = (focus_depth_mm > 0.f);
  if ((want_click_focus || want_depth_map || want_fnumber || want_focus) && profile < 3) {
    fprintf(stderr,
            "note: DOF/depth features require DESKTOP profile — upgrading %d → 3\n", profile);
    profile = 3;
  }

  int max_wait_ms = (profile >= 3) ? 180000 : 90000;
  if (const char *e = getenv("LIBCP_MAX_WAIT_MS")) {
    int v = atoi(e);
    if (v > 1000) max_wait_ms = v;
  }

  void *h = dlopen(libpath, RTLD_NOW | RTLD_LOCAL);
  if (!h) {
    fprintf(stderr, "dlopen: %s\n", dlerror());
    return 1;
  }

  auto GetVersion = (fn_GetVersion)must(h, "_ZN5CIAPI10GetVersionEv");
  auto IsHW = (fn_IsHW)must(h, "_ZN5CIAPI8Renderer20IsHardwareCompatibleEv");
  auto Create = (fn_Create)must(h, "_ZN5CIAPI8Renderer6CreateENS_15RendererProfileE");
  auto setInput = (fn_setInput)must(h, "_ZN5CIAPI12RendererBase18setInputDataStreamEPKvm");
  auto render = (fn_render)must(h, "_ZN5CIAPI8Renderer6renderEiRKNS_3ROIENS_10RenderTypeEb");
  auto outputBuffer = (fn_outputBuffer)must(h, "_ZNK5CIAPI8Renderer12outputBufferEv");
  auto levelCount = (fn_levelCount)must(h, "_ZNK5CIAPI12ImagePyramid10levelCountEv");
  auto at = (fn_at)must(h, "_ZNK5CIAPI12ImagePyramidixEi");
  auto width = (fn_w)must(h, "_ZNK5CIAPI5Image5widthEv");
  auto height = (fn_h)must(h, "_ZNK5CIAPI5Image6heightEv");
  auto stride = (fn_stride)must(h, "_ZNK5CIAPI5Image6strideEv");
  auto data = (fn_data)must(h, "_ZNK5CIAPI5Image4dataEv");
  auto setPropF = (fn_setPropFloat)dlsym(
      h, "_ZN5CIAPI12RendererBase11setPropertyENS_10ParamFloatEf");
  auto getPropF = (fn_getPropFloat)dlsym(
      h, "_ZNK5CIAPI12RendererBase11getPropertyENS_10ParamFloatE");
  auto depthCtor = (fn_DepthEditor_ctor)dlsym(
      h, "_ZN5CIAPI11DepthEditorC1ERNS_8RendererE");
  // length prefix is 15 (getDepthAtPoint), not 14 — wrong length → NULL
  auto getDepth = (fn_getDepthAtPoint)dlsym(
      h, "_ZN5CIAPI11DepthEditor15getDepthAtPointERKNS_5PointIfEE");

  if (!GetVersion || !Create || !setInput || !render || !outputBuffer || !levelCount || !at ||
      !width || !height || !stride || !data) {
    fprintf(stderr, "symbol resolve failed\n");
    return 1;
  }

  printf("version=%s IsHW=%d profile=%d fnum=%.2f focus=%.1f fx=%.3f fy=%.3f setPropF=%p "
         "depthCtor=%p getDepth=%p\n",
         GetVersion(), IsHW ? IsHW() : -1, profile, fnumber, focus_depth_mm, fx, fy,
         (void *)setPropF, (void *)depthCtor, (void *)getDepth);
  fflush(stdout);

  if ((want_click_focus || want_depth_map) && (!depthCtor || !getDepth)) {
    fprintf(stderr, "warn: DepthEditor symbols missing — click-focus/depth map disabled\n");
    want_click_focus = false;
    want_depth_map = false;
  }
  if ((fnumber >= 2.f || focus_depth_mm > 0.f) && !setPropF) {
    fprintf(stderr, "warn: setProperty(ParamFloat) missing — DOF props ignored\n");
  }

  FILE *lf = fopen(lripath, "rb");
  if (!lf) {
    perror(lripath);
    return 1;
  }
  fseek(lf, 0, SEEK_END);
  long sz = ftell(lf);
  fseek(lf, 0, SEEK_SET);
  if (sz <= 0) {
    fprintf(stderr, "empty LRI\n");
    return 1;
  }
  std::vector<unsigned char> buf((size_t)sz);
  if (fread(buf.data(), 1, (size_t)sz, lf) != (size_t)sz) {
    fprintf(stderr, "short read\n");
    return 1;
  }
  fclose(lf);
  printf("LRI %ld bytes from %s\n", sz, lripath);
  fflush(stdout);

  Renderer r;
  memset(&r, 0, sizeof r);
  Create(&r, profile);
  printf("Create ptr=%p\n", r.ptr);
  fflush(stdout);
  if (!r.ptr) {
    fprintf(stderr, "Create failed\n");
    return 2;
  }

  // DepthEditor MUST be constructed before setInputDataStream — libcp throws
  // "Cannot set DepthEditor after setInputDataStream!" otherwise.
  DepthEditorShell de{};
  bool de_ready = false;
  if ((want_click_focus || want_depth_map) && depthCtor && getDepth) {
    memset(&de, 0, sizeof de);
    depthCtor(&de, &r);
    de_ready = (de.impl != nullptr);
    fprintf(stderr, "DepthEditor ctor (pre-setInput) → impl=%p ctrl=%p ok=%d\n", de.impl, de.ctrl,
            de_ready);
    if (!de_ready) {
      fprintf(stderr, "warn: DepthEditor empty after ctor — click-focus/depth map disabled\n");
      want_click_focus = false;
      want_depth_map = false;
    }
  }

  printf("setInput...\n");
  fflush(stdout);
  setInput(&r, buf.data(), (unsigned long)sz);
  printf("setInput done\n");
  fflush(stdout);

  // Explicit DOF before first render (gallery sets DOF then save/render).
  // Click-focus needs depth first — applied after initial pass.
  apply_dof(setPropF, getPropF, &r, fnumber, want_click_focus ? -1.f : focus_depth_mm);

  // Preferred ROIs. DESKTOP (profile 3) uses Lumen canvas 10432×7824.
  std::vector<ROI> rois;
  if (profile >= 3) {
    rois = {
        {0, 0, 10432, 7824},
        {0, 0, 8320, 6240},
        {0, 0, 4160, 3120},
        {0, 0, 3328, 2496},
    };
  } else if (profile == 0) {
    rois = {
        {0, 0, 1024, 768},
        {0, 0, 520, 390},
        {0, 0, 2080, 1560},
    };
  } else {
    rois = {
        {0, 0, 4160, 3120},
        {0, 0, 3328, 2496},
        {0, 0, 2080, 1560},
        {0, 0, 1024, 768},
    };
  }

  auto poll_until = [&](int budget_ms) -> bool {
    int waited = 0;
    int step = 400;
    while (waited < budget_ms) {
      std::this_thread::sleep_for(std::chrono::milliseconds(step));
      waited += step;
      if (try_dump(outputBuffer, levelCount, at, width, height, stride, data, &r, outpath)) {
        return true;
      }
      if (step < 2000) step += 200;
    }
    return false;
  };

  auto render_pass = [&](int budget_ms, bool expand) -> bool {
    for (const ROI &roi : rois) {
      printf("render L=0 ROI=%d,%d,%d,%d T=0 F=0\n", roi.x0, roi.y0, roi.x1, roi.y1);
      fflush(stdout);
      render(&r, 0, &roi, 0, false);
      if (poll_until(budget_ms / (expand ? 2 : 1))) return true;
      if (!expand) break; // first ROI only for refocus re-pass
    }
    if (!expand) return false;
    fprintf(stderr, "fast path empty; expanding search\n");
    for (int rt = 0; rt < 4; rt++) {
      for (const ROI &roi : rois) {
        for (int level = 0; level < 2; level++) {
          for (int flag = 0; flag < 2; flag++) {
            if (level == 0 && rt == 0 && flag == 0) continue;
            printf("render L=%d ROI=%d,%d,%d,%d T=%d F=%d\n", level, roi.x0, roi.y0, roi.x1,
                   roi.y1, rt, flag);
            fflush(stdout);
            render(&r, level, &roi, rt, (bool)flag);
            if (poll_until(15000)) return true;
          }
        }
      }
    }
    return false;
  };

  // Pass 1: base image (+ optional explicit focus/fnumber)
  if (!render_pass(max_wait_ms, true)) {
    ShPtr pyr;
    memset(&pyr, 0, sizeof pyr);
    outputBuffer(&pyr, &r);
    if (pyr.ptr && levelCount(&pyr) > 0) {
      ShPtr *img = at(&pyr, 0);
      int w = width(img), ht = height(img), s = stride(img);
      unsigned char *px = (unsigned char *)data(img);
      std::string zpath = std::string(outpath) + ".zerodump.ppm";
      write_ppm(zpath.c_str(), px, w, ht, s);
      printf("no signal; zero-dump %s %dx%d\n", zpath.c_str(), w, ht);
    }
    fprintf(stderr, "libcp-export FAILED: no non-zero output\n");
    return 3;
  }

  auto sample_depth = [&](float nx, float ny) -> float {
    if (!de_ready || !getDepth) return -1.f;
    PointF pt{nx, ny};
    float z = getDepth(&de, &pt);
    fprintf(stderr, "getDepthAtPoint(%.3f,%.3f) → %.3f\n", nx, ny, z);
    printf("DEPTH_AT fx=%.4f fy=%.4f z=%.3f\n", nx, ny, z);
    fflush(stdout);
    return z;
  };

  // Click-to-focus: sample → set ViewDofFocusDepth → re-render
  // (depth values available after first successful render)
  if (want_click_focus && de_ready) {
    float z = sample_depth(fx, fy);
    if (z > 0.f && z < 1e7f) {
      focus_depth_mm = z;
      apply_dof(setPropF, getPropF, &r, -1.f, focus_depth_mm);
      printf("refocus depth=%.1f mm @ (%.3f,%.3f)\n", focus_depth_mm, fx, fy);
      fflush(stdout);
      if (!render_pass(max_wait_ms / 2, false)) {
        fprintf(stderr, "warn: refocus re-render produced no new signal; keeping first dump\n");
      }
    } else {
      fprintf(stderr, "warn: click focus sample invalid (z=%.3f); keeping first render\n", z);
    }
  }

  // Low-res depth colormap
  if (want_depth_map && de_ready) {
    const int dw = 320;
    const int dh = 240;
    std::vector<float> zmap((size_t)dw * dh, 0.f);
    for (int y = 0; y < dh; y++) {
      float ny = (y + 0.5f) / (float)dh;
      for (int x = 0; x < dw; x++) {
        float nx = (x + 0.5f) / (float)dw;
        PointF pt{nx, ny};
        float z = getDepth(&de, &pt);
        if (z == z && z > 0.f && z < 1e7f) zmap[(size_t)y * dw + x] = z;
      }
    }
    if (!write_depth_ppm(depth_path, zmap, dw, dh)) {
      fprintf(stderr, "warn: depth map write failed\n");
    }
  }

  return 0;
}
