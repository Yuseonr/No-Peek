//! Direct2D / Direct3D11 / DirectComposition GPU renderer with software DIB fallback.
//! Adapted from `oyenza-studio/crates/poni/src/gfx.rs`.

use nopeek_core::geom::{palette, Color};
use nopeek_core::icons::{self, Icon};
use nopeek_core::island::{Island, ICON_SIZE, ICON_TEXT_GAP, SHOULDER};
use nopeek_core::svgpath::{arc_to_cubics, notch_segments, Seg};
use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::ManuallyDrop;
use windows::core::{w, Interface, Result, GUID, HRESULT, PCWSTR};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{BOOL, COLORREF, E_NOTIMPL, HMODULE, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::{
    IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows::Win32::UI::WindowsAndMessaging::{
    UpdateLayeredWindowIndirect, ULW_ALPHA, UPDATELAYEREDWINDOWINFO,
};

const CLEAR_COLOR: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};

const PREMUL_BGRA: D2D1_PIXEL_FORMAT = D2D1_PIXEL_FORMAT {
    format: DXGI_FORMAT_B8G8R8A8_UNORM,
    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
};

fn d2d_color(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
}

fn pt(x: f32, y: f32) -> D2D_POINT_2F {
    D2D_POINT_2F { x, y }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe fn create_d3d_device() -> Result<(ID3D11Device, ID3D11DeviceContext, bool)> {
    let levels = [
        D3D_FEATURE_LEVEL_11_1,
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_1,
        D3D_FEATURE_LEVEL_10_0,
        D3D_FEATURE_LEVEL_9_3,
        D3D_FEATURE_LEVEL_9_2,
        D3D_FEATURE_LEVEL_9_1,
    ];
    let mut last = None;
    for (driver, warp) in [
        (D3D_DRIVER_TYPE_HARDWARE, false),
        (D3D_DRIVER_TYPE_WARP, true),
    ] {
        for lv in [&levels[..], &levels[1..]] {
            let (mut dev, mut ctx) = (None, None);
            match D3D11CreateDevice(
                None,
                driver,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(lv),
                D3D11_SDK_VERSION,
                Some(&mut dev),
                None,
                Some(&mut ctx),
            ) {
                Ok(()) => {
                    if let (Some(d), Some(c)) = (dev, ctx) {
                        return Ok((d, c, warp));
                    }
                }
                Err(e) => last = Some(e),
            }
        }
    }
    Err(last.unwrap_or_else(|| E_NOTIMPL.into()))
}

unsafe fn dcomp_create_device(dxgi: &IDXGIDevice) -> Result<IDCompositionDevice> {
    type Create = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
    let lib = LoadLibraryExW(w!("dcomp.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)?;
    let Some(f) = GetProcAddress(lib, windows::core::s!("DCompositionCreateDevice")) else {
        return Err(E_NOTIMPL.into());
    };
    let create: Create = std::mem::transmute(f);
    let mut out = std::ptr::null_mut();
    create(dxgi.as_raw(), &IDCompositionDevice::IID, &mut out).ok()?;
    if out.is_null() {
        return Err(E_NOTIMPL.into());
    }
    Ok(IDCompositionDevice::from_raw(out))
}

struct GpuTarget {
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    _dcomp: IDCompositionDevice,
    dc: ID2D1DeviceContext,
    swap: IDXGISwapChain1,
    target_bmp: Option<ID2D1Bitmap1>,
    _w: u32,
    _h: u32,
}

impl GpuTarget {
    unsafe fn new(d2d: &ID2D1Factory1, hwnd: HWND, w: u32, h: u32) -> Result<Self> {
        let (d3d, _ctx, _warp) = create_d3d_device()?;
        let dxgi: IDXGIDevice = d3d.cast()?;
        let dc = d2d
            .CreateDevice(&dxgi)?
            .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
        let dxgi_adapter = dxgi.GetAdapter()?;
        let factory: IDXGIFactory2 = dxgi_adapter.GetParent()?;
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: w,
            Height: h,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: BOOL(0),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 3,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: 0,
        };
        let swap = factory.CreateSwapChainForComposition(&d3d, &desc, None)?;
        let dcomp = dcomp_create_device(&dxgi)?;
        let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
        let visual = dcomp.CreateVisual()?;
        visual.SetContent(&swap)?;
        target.SetRoot(&visual)?;
        dcomp.Commit()?;
        Ok(Self {
            _target: target,
            _visual: visual,
            _dcomp: dcomp,
            dc,
            swap,
            target_bmp: None,
            _w: w,
            _h: h,
        })
    }

    unsafe fn begin(&mut self, dpi: f32) -> Result<()> {
        let surface: IDXGISurface = self.swap.GetBuffer(0)?;
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: PREMUL_BGRA,
            dpiX: dpi,
            dpiY: dpi,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            colorContext: ManuallyDrop::new(None),
        };
        let bmp = self
            .dc
            .CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
        self.dc.SetTarget(&bmp);
        self.dc.SetDpi(dpi, dpi);
        self.dc.BeginDraw();
        self.target_bmp = Some(bmp);
        Ok(())
    }

    unsafe fn end(&mut self) {
        self.dc.SetTarget(None::<&ID2D1Image>);
        self.target_bmp = None;
    }
}

struct SoftTarget {
    rt: ID2D1DCRenderTarget,
    mem_dc: HDC,
    dib: HBITMAP,
    old_bmp: HGDIOBJ,
    _bits: *mut u8,
    _cap_w: i32,
    _cap_h: i32,
}

impl Drop for SoftTarget {
    fn drop(&mut self) {
        unsafe {
            if !self.old_bmp.is_invalid() {
                SelectObject(self.mem_dc, self.old_bmp);
            }
            if !self.dib.is_invalid() {
                let _ = DeleteObject(self.dib);
            }
            let _ = DeleteDC(self.mem_dc);
        }
    }
}

impl SoftTarget {
    unsafe fn new(d2d: &ID2D1Factory, w: i32, h: i32) -> Result<Self> {
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: PREMUL_BGRA,
            dpiX: 0.0,
            dpiY: 0.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let rt = d2d.CreateDCRenderTarget(&props)?;
        rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        rt.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        let mem_dc = CreateCompatibleDC(None);

        let nw = ((w + 63) / 64) * 64;
        let nh = ((h + 63) / 64) * 64;
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: nw,
                biHeight: -nh,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let old_bmp = SelectObject(mem_dc, dib);

        Ok(Self {
            rt,
            mem_dc,
            dib,
            old_bmp,
            _bits: bits as *mut u8,
            _cap_w: nw,
            _cap_h: nh,
        })
    }

    unsafe fn begin(&mut self, w: i32, h: i32, dpi: f32) -> Result<()> {
        let rc = RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: h,
        };
        self.rt.BindDC(self.mem_dc, &rc)?;
        self.rt.SetDpi(dpi, dpi);
        self.rt.BeginDraw();
        Ok(())
    }

    unsafe fn push(&self, hwnd: HWND, w: i32, h: i32) -> Result<()> {
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let mut rect = RECT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut rect);
        let (dst, size, src) = (
            POINT {
                x: rect.left,
                y: rect.top,
            },
            SIZE { cx: w, cy: h },
            POINT { x: 0, y: 0 },
        );
        let info = UPDATELAYEREDWINDOWINFO {
            cbSize: std::mem::size_of::<UPDATELAYEREDWINDOWINFO>() as u32,
            hdcDst: HDC::default(),
            pptDst: &dst,
            psize: &size,
            hdcSrc: self.mem_dc,
            pptSrc: &src,
            crKey: COLORREF(0),
            pblend: &blend,
            dwFlags: ULW_ALPHA,
            prcDirty: std::ptr::null(),
        };
        if UpdateLayeredWindowIndirect(hwnd, &info).as_bool() {
            Ok(())
        } else {
            Err(windows::core::Error::from_win32())
        }
    }
}

fn scale_about(cx: f32, cy: f32, s: f32) -> Matrix3x2 {
    Matrix3x2 {
        M11: s,
        M12: 0.0,
        M21: 0.0,
        M22: s,
        M31: cx * (1.0 - s),
        M32: cy * (1.0 - s),
    }
}

pub struct Gfx {
    hwnd: HWND,
    d2d_factory: ID2D1Factory,
    dwrite_factory: IDWriteFactory,
    format_semibold: IDWriteTextFormat,
    format_bold: IDWriteTextFormat,
    brush: ID2D1SolidColorBrush,
    round_stroke: ID2D1StrokeStyle,
    gpu: Option<GpuTarget>,
    soft: Option<SoftTarget>,
    icon_geoms: HashMap<Icon, Vec<(ID2D1PathGeometry, bool)>>,
    pub width: u32,
    pub height: u32,
}

impl Gfx {
    pub fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self> {
        unsafe {
            let d2d_factory: ID2D1Factory =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_factory1: ID2D1Factory1 = d2d_factory.cast()?;
            let dwrite_factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;

            let font_name = wide("Segoe UI Variable Text");
            let fallback_name = wide("Segoe UI");

            let make_format = |w: DWRITE_FONT_WEIGHT| -> Result<IDWriteTextFormat> {
                dwrite_factory
                    .CreateTextFormat(
                        PCWSTR(font_name.as_ptr()),
                        None,
                        w,
                        DWRITE_FONT_STYLE_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        13.5,
                        w!(""),
                    )
                    .or_else(|_| {
                        dwrite_factory.CreateTextFormat(
                            PCWSTR(fallback_name.as_ptr()),
                            None,
                            w,
                            DWRITE_FONT_STYLE_NORMAL,
                            DWRITE_FONT_STRETCH_NORMAL,
                            13.5,
                            w!(""),
                        )
                    })
            };

            let format_semibold = make_format(DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
            let format_bold = make_format(DWRITE_FONT_WEIGHT_BOLD)?;

            let stroke_props = D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 10.0,
                dashStyle: D2D1_DASH_STYLE_SOLID,
                dashOffset: 0.0,
            };
            let round_stroke = d2d_factory.CreateStrokeStyle(&stroke_props, None)?;

            let gpu = GpuTarget::new(&d2d_factory1, hwnd, width, height).ok();
            let (brush, soft) = if let Some(ref g) = gpu {
                let b =
                    g.dc.CreateSolidColorBrush(&d2d_color(palette::TEXT), None)?;
                (b, None)
            } else {
                let s = SoftTarget::new(&d2d_factory, width as i32, height as i32)?;
                let b =
                    s.rt.CreateSolidColorBrush(&d2d_color(palette::TEXT), None)?;
                (b, Some(s))
            };

            Ok(Self {
                hwnd,
                d2d_factory,
                dwrite_factory,
                format_semibold,
                format_bold,
                brush,
                round_stroke,
                gpu,
                soft,
                icon_geoms: HashMap::new(),
                width,
                height,
            })
        }
    }

    /// Measure text DIP width and height using DirectWrite text layout.
    pub fn measure_text(&self, text: &str, is_bold: bool) -> (f32, f32) {
        if text.is_empty() {
            return (0.0, 0.0);
        }
        unsafe {
            let fmt = if is_bold {
                &self.format_bold
            } else {
                &self.format_semibold
            };
            let w = wide(text);
            if let Ok(layout) = self.dwrite_factory.CreateTextLayout(&w, fmt, 1000.0, 100.0) {
                let mut metrics = DWRITE_TEXT_METRICS::default();
                if layout.GetMetrics(&mut metrics).is_ok() {
                    return (metrics.widthIncludingTrailingWhitespace, metrics.height);
                }
            }
        }
        (text.len() as f32 * 8.0, 18.0)
    }

    pub fn icon_geometry(&mut self, icon: Icon) -> &[(ID2D1PathGeometry, bool)] {
        if !self.icon_geoms.contains_key(&icon) {
            let mut list = Vec::new();
            for part in icons::parts(icon) {
                if let Ok(segs) = nopeek_core::svgpath::parse(part.d) {
                    if let Ok(geo) = self.create_path_geometry(&segs, part.fill) {
                        list.push((geo, part.fill));
                    }
                }
            }
            self.icon_geoms.insert(icon, list);
        }
        self.icon_geoms
            .get(&icon)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    fn create_path_geometry(&self, segs: &[Seg], close_fill: bool) -> Result<ID2D1PathGeometry> {
        unsafe {
            let geo = self.d2d_factory.CreatePathGeometry()?;
            let sink = geo.Open()?;
            sink.SetFillMode(D2D1_FILL_MODE_WINDING);
            let mut open = false;
            let mut cur = (0.0f32, 0.0f32);
            let begin = if close_fill {
                D2D1_FIGURE_BEGIN_FILLED
            } else {
                D2D1_FIGURE_BEGIN_HOLLOW
            };

            for s in segs {
                match *s {
                    Seg::MoveTo(x, y) => {
                        if open {
                            sink.EndFigure(D2D1_FIGURE_END_OPEN);
                        }
                        sink.BeginFigure(pt(x, y), begin);
                        open = true;
                        cur = (x, y);
                    }
                    Seg::LineTo(x, y) => {
                        if !open {
                            sink.BeginFigure(pt(cur.0, cur.1), begin);
                            open = true;
                        }
                        sink.AddLine(pt(x, y));
                        cur = (x, y);
                    }
                    Seg::CubicTo(x1, y1, x2, y2, x, y) => {
                        if !open {
                            sink.BeginFigure(pt(cur.0, cur.1), begin);
                            open = true;
                        }
                        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                            point1: pt(x1, y1),
                            point2: pt(x2, y2),
                            point3: pt(x, y),
                        });
                        cur = (x, y);
                    }
                    Seg::QuadTo(x1, y1, x, y) => {
                        if !open {
                            sink.BeginFigure(pt(cur.0, cur.1), begin);
                            open = true;
                        }
                        sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT {
                            point1: pt(x1, y1),
                            point2: pt(x, y),
                        });
                        cur = (x, y);
                    }
                    Seg::ArcTo(rx, ry, rot, large, sweep, x, y) => {
                        if !open {
                            sink.BeginFigure(pt(cur.0, cur.1), begin);
                            open = true;
                        }
                        for c in arc_to_cubics(nopeek_core::svgpath::ArcParams {
                            from: cur,
                            r: (rx, ry),
                            rot_deg: rot,
                            large,
                            sweep,
                            to: (x, y),
                        }) {
                            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                                point1: pt(c[0], c[1]),
                                point2: pt(c[2], c[3]),
                                point3: pt(c[4], c[5]),
                            });
                        }
                        cur = (x, y);
                    }
                    Seg::Close => {
                        if open {
                            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                            open = false;
                        }
                    }
                }
            }
            if open {
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
            }
            sink.Close()?;
            Ok(geo)
        }
    }

    /// Render a frame of the Dynamic Island.
    pub fn render(&mut self, island: &Island, scale: f32) {
        let dpi = scale * 96.0;
        let cx_dip = (self.width as f32 / scale) / 2.0;
        let body = island.body_rect_at(cx_dip);

        let border_color = if island.level == nopeek_core::ipc::RiskLevel::Escalated {
            Color::rgba(1.0, 69.0 / 255.0, 58.0 / 255.0, 0.35)
        } else {
            palette::BORDER
        };

        let icon = island.icon;
        let icon_color = if island.level == nopeek_core::ipc::RiskLevel::Escalated {
            palette::ESCALATED_RED
        } else {
            palette::ALERT_AMBER
        };

        let content_alpha = island.content_alpha();
        let content_scale = island.content_scale();
        let text = island.text.clone();
        let is_escalated = island.level == nopeek_core::ipc::RiskLevel::Escalated;

        let segs = notch_segments(body, island.current().r, SHOULDER);
        let notch_geo = self.create_path_geometry(&segs, true).ok();

        // Prepare icon geometries
        let geoms = self.icon_geometry(icon).to_vec();

        if let Some(ref mut gpu) = self.gpu {
            unsafe {
                let _ = gpu.begin(dpi);
                gpu.dc.Clear(Some(&CLEAR_COLOR));

                // 1. Draw notch body
                if let Some(ref geo) = notch_geo {
                    self.brush.SetColor(&d2d_color(palette::ISLAND));
                    gpu.dc.FillGeometry(geo, &self.brush, None);
                    self.brush.SetColor(&d2d_color(border_color));
                    gpu.dc.DrawGeometry(geo, &self.brush, 1.0, None);
                }

                // 2. Draw content (icon + text) with pop scale
                if content_alpha > 0.002 && !text.is_empty() {
                    let mut prev_xf = Matrix3x2::identity();
                    gpu.dc.GetTransform(&mut prev_xf);
                    let pop_xf =
                        scale_about(body.x + body.w / 2.0, body.y + body.h / 2.0, content_scale);
                    gpu.dc.SetTransform(&(pop_xf * prev_xf));

                    // Draw SVG Icon
                    let icon_x = body.x + 18.0;
                    let icon_y = body.y + (body.h - ICON_SIZE) / 2.0;
                    let s = ICON_SIZE / 24.0;
                    let mut icon_prev = Matrix3x2::identity();
                    gpu.dc.GetTransform(&mut icon_prev);
                    let m = Matrix3x2 {
                        M11: s,
                        M12: 0.0,
                        M21: 0.0,
                        M22: s,
                        M31: icon_x,
                        M32: icon_y,
                    };
                    gpu.dc.SetTransform(&(m * icon_prev));
                    self.brush
                        .SetColor(&d2d_color(icon_color.mul_alpha(content_alpha)));
                    for (geo, fill) in &geoms {
                        if *fill {
                            gpu.dc.FillGeometry(geo, &self.brush, None);
                        } else {
                            gpu.dc.DrawGeometry(
                                geo,
                                &self.brush,
                                icons::STROKE,
                                &self.round_stroke,
                            );
                        }
                    }
                    gpu.dc.SetTransform(&icon_prev);

                    // Draw text
                    let text_x = icon_x + ICON_SIZE + ICON_TEXT_GAP;
                    let text_y = body.y + (body.h - 18.0) / 2.0;
                    let text_color = if is_escalated {
                        Color::rgba(1.0, 240.0 / 255.0, 239.0 / 255.0, 0.98)
                    } else {
                        palette::TEXT
                    };
                    self.brush
                        .SetColor(&d2d_color(text_color.mul_alpha(content_alpha)));
                    let fmt = if is_escalated {
                        &self.format_bold
                    } else {
                        &self.format_semibold
                    };
                    let w = wide(&text);
                    if let Ok(layout) = self.dwrite_factory.CreateTextLayout(&w, fmt, 1000.0, 100.0)
                    {
                        gpu.dc.DrawTextLayout(
                            pt(text_x, text_y),
                            &layout,
                            &self.brush,
                            D2D1_DRAW_TEXT_OPTIONS_NONE,
                        );
                    }

                    gpu.dc.SetTransform(&prev_xf);
                }

                let _ = gpu.dc.EndDraw(None, None);
                let _ = gpu.swap.Present(1, DXGI_PRESENT(0));
                gpu.end();
            }
        } else if let Some(ref mut soft) = self.soft {
            unsafe {
                let _ = soft.begin(self.width as i32, self.height as i32, dpi);
                soft.rt.Clear(Some(&CLEAR_COLOR));

                if let Some(ref geo) = notch_geo {
                    self.brush.SetColor(&d2d_color(palette::ISLAND));
                    soft.rt.FillGeometry(geo, &self.brush, None);
                    self.brush.SetColor(&d2d_color(border_color));
                    soft.rt.DrawGeometry(geo, &self.brush, 1.0, None);
                }

                if content_alpha > 0.002 && !text.is_empty() {
                    let icon_x = body.x + 18.0;
                    let icon_y = body.y + (body.h - ICON_SIZE) / 2.0;
                    let s = ICON_SIZE / 24.0;
                    let mut icon_prev = Matrix3x2::identity();
                    soft.rt.GetTransform(&mut icon_prev);
                    let m = Matrix3x2 {
                        M11: s,
                        M12: 0.0,
                        M21: 0.0,
                        M22: s,
                        M31: icon_x,
                        M32: icon_y,
                    };
                    soft.rt.SetTransform(&(m * icon_prev));
                    self.brush
                        .SetColor(&d2d_color(icon_color.mul_alpha(content_alpha)));
                    for (geo, fill) in &geoms {
                        if *fill {
                            soft.rt.FillGeometry(geo, &self.brush, None);
                        } else {
                            soft.rt.DrawGeometry(
                                geo,
                                &self.brush,
                                icons::STROKE,
                                &self.round_stroke,
                            );
                        }
                    }
                    soft.rt.SetTransform(&icon_prev);

                    let text_x = icon_x + ICON_SIZE + ICON_TEXT_GAP;
                    let text_y = body.y + (body.h - 18.0) / 2.0;
                    self.brush
                        .SetColor(&d2d_color(palette::TEXT.mul_alpha(content_alpha)));
                    let fmt = if is_escalated {
                        &self.format_bold
                    } else {
                        &self.format_semibold
                    };
                    let w = wide(&text);
                    if let Ok(layout) = self.dwrite_factory.CreateTextLayout(&w, fmt, 1000.0, 100.0)
                    {
                        soft.rt.DrawTextLayout(
                            pt(text_x, text_y),
                            &layout,
                            &self.brush,
                            D2D1_DRAW_TEXT_OPTIONS_NONE,
                        );
                    }
                }

                let _ = soft.rt.EndDraw(None, None);
                let _ = soft.push(self.hwnd, self.width as i32, self.height as i32);
            }
        }
    }
}
