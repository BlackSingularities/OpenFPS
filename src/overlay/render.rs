//! Renderowanie nakładki: Direct2D (programowy render target) + DirectWrite
//! do 32-bitowej bitmapy DIB, wyświetlanej przez `UpdateLayeredWindow`.
//! Renderer programowy jest celowo wybrany: przy tak małej powierzchni kosztuje
//! mikrosekundy CPU i w ogóle nie konkuruje z grą o GPU.

use std::collections::HashMap;

use windows::core::{w, HSTRING};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::*;

use crate::config::{Color, Orientation, Style, TextEffect};

// ───────────────────────────── Model treści ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Font {
    Value,
    Label,
    Big,
    Title,
}

pub struct Span {
    pub text: String,
    pub color: Color,
    pub font: Font,
    /// Szablon szerokości (np. „8888”), żeby liczby nie „skakały”.
    pub template: &'static str,
}

impl Span {
    pub fn value(text: impl Into<String>, color: Color, template: &'static str) -> Self {
        Self { text: text.into(), color, font: Font::Value, template }
    }
    pub fn big(text: impl Into<String>, color: Color, template: &'static str) -> Self {
        Self { text: text.into(), color, font: Font::Big, template }
    }
    pub fn unit(text: impl Into<String>, color: Color) -> Self {
        Self { text: text.into(), color, font: Font::Label, template: "" }
    }
    /// Jednostka z zarezerwowaną szerokością (np. „KB/s” i „MB/s” zajmują tyle samo).
    pub fn unit_t(text: impl Into<String>, color: Color, template: &'static str) -> Self {
        Self { text: text.into(), color, font: Font::Label, template }
    }
}

pub enum Block {
    Header { title: String, subtitle: String, color: Color },
    Row { label: String, spans: Vec<Span>, bar: Option<(f32, Color)> },
    Graph { label: String, caption: String, values: Vec<f32>, max: f32, color: Color, guide: Option<f32> },
    Cores { label: String, values: Vec<(f32, Color)> },
    Note { text: String, color: Color },
}

pub struct Frame {
    pub blocks: Vec<Block>,
    pub orientation: Orientation,
    pub show_labels: bool,
    pub edit: bool,
}

// ───────────────────────────── Powierzchnia ─────────────────────────────

/// Bitmapa DIB 32 bpp (premultiplied BGRA) wybrana do pamięciowego DC.
pub struct Surface {
    pub dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    pub w: i32,
    pub h: i32,
}

impl Surface {
    pub fn new() -> Self {
        let dc = unsafe { CreateCompatibleDC(HDC::default()) };
        Self { dc, bmp: HBITMAP::default(), old: HGDIOBJ::default(), w: 0, h: 0 }
    }

    fn ensure(&mut self, w: i32, h: i32) {
        if w <= self.w && h <= self.h && !self.bmp.is_invalid() {
            return;
        }
        // Zapas, by nie realokować przy każdej drobnej zmianie rozmiaru.
        let (nw, nh) = (w.max(self.w) + 32, h.max(self.h) + 32);
        unsafe {
            let bi = BITMAPINFO {
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
            let mut bits = std::ptr::null_mut();
            let Ok(bmp) = CreateDIBSection(self.dc, &bi, DIB_RGB_COLORS, &mut bits, None, 0) else { return };
            let prev = SelectObject(self.dc, bmp);
            if self.bmp.is_invalid() {
                self.old = prev;
            } else {
                let _ = DeleteObject(self.bmp);
            }
            self.bmp = bmp;
            self.w = nw;
            self.h = nh;
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            if !self.bmp.is_invalid() {
                SelectObject(self.dc, self.old);
                let _ = DeleteObject(self.bmp);
            }
            let _ = DeleteDC(self.dc);
        }
    }
}

// ───────────────────────────── Grafika ─────────────────────────────

#[derive(Clone, Copy)]
struct FontMetrics {
    height: f32,
    baseline: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FontKey(Font, u32);

pub struct Gfx {
    factory: ID2D1Factory,
    dw: IDWriteFactory,
    rt: ID2D1DCRenderTarget,
    brush: ID2D1SolidColorBrush,
    dash: Option<ID2D1StrokeStyle>,
    fonts: HashMap<FontKey, (IDWriteTextFormat, FontMetrics)>,
    widths: HashMap<(String, FontKey), f32>,
    font_sig: (String, i32),
    /// Kolumny układu pionowego (etykieta, wartości, pasek) wyliczone w fazie pomiaru.
    cols: (f32, f32, f32),
}

fn col(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c.0 as f32 / 255.0, g: c.1 as f32 / 255.0, b: c.2 as f32 / 255.0, a: c.3 as f32 / 255.0 }
}

fn rect(l: f32, t: f32, r: f32, b: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: l, top: t, right: r, bottom: b }
}

fn pt(x: f32, y: f32) -> D2D_POINT_2F {
    D2D_POINT_2F { x, y }
}

/// Geometria jednego elementu po rozmieszczeniu.
struct Placed {
    x: f32,
    y: f32,
    h: f32,
}

struct Sizes {
    k: f32,
    pad: f32,
    gap: f32,
}

impl Gfx {
    pub fn new() -> windows::core::Result<Self> {
        unsafe {
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let rt = factory.CreateDCRenderTarget(&props)?;
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let brush = rt.CreateSolidColorBrush(&col(Color(255, 255, 255, 255)), None)?;
            let dash = factory
                .CreateStrokeStyle(
                    &D2D1_STROKE_STYLE_PROPERTIES {
                        startCap: D2D1_CAP_STYLE_ROUND,
                        endCap: D2D1_CAP_STYLE_ROUND,
                        dashCap: D2D1_CAP_STYLE_ROUND,
                        lineJoin: D2D1_LINE_JOIN_ROUND,
                        miterLimit: 10.0,
                        dashStyle: D2D1_DASH_STYLE_DASH,
                        dashOffset: 0.0,
                    },
                    None,
                )
                .ok();
            Ok(Self { factory, dw, rt, brush, dash, fonts: HashMap::new(), widths: HashMap::new(), font_sig: (String::new(), 0), cols: (0.0, 0.0, 0.0) })
        }
    }

    fn font_size(style: &Style, f: Font, k: f32) -> f32 {
        let base = style.font_size * k;
        match f {
            Font::Value => base,
            Font::Label => base * style.label_scale,
            Font::Big => base * 1.6,
            Font::Title => base * 1.0,
        }
    }

    fn font(&mut self, style: &Style, f: Font, k: f32) -> (IDWriteTextFormat, FontMetrics) {
        let sig = (style.font_family.clone(), style.font_weight.value());
        if sig != self.font_sig {
            self.fonts.clear();
            self.widths.clear();
            self.font_sig = sig;
        }
        let size = Self::font_size(style, f, k);
        let key = FontKey(f, (size * 4.0).round() as u32);
        if let Some(v) = self.fonts.get(&key) {
            return v.clone();
        }
        let weight = match f {
            Font::Label => style.font_weight.value().min(500).max(400),
            _ => style.font_weight.value(),
        };
        let family = HSTRING::from(style.font_family.as_str());
        let fmt = unsafe {
            self.dw
                .CreateTextFormat(&family, None, DWRITE_FONT_WEIGHT(weight), DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!(""))
                .or_else(|_| {
                    self.dw.CreateTextFormat(w!("Segoe UI"), None, DWRITE_FONT_WEIGHT(weight), DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!(""))
                })
                .expect("Segoe UI")
        };
        unsafe {
            let _ = fmt.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
        }
        let metrics = self.line_metrics(&fmt, size);
        self.fonts.insert(key, (fmt.clone(), metrics));
        (fmt, metrics)
    }

    fn line_metrics(&self, fmt: &IDWriteTextFormat, size: f32) -> FontMetrics {
        unsafe {
            let s: Vec<u16> = "8Ag".encode_utf16().collect();
            if let Ok(layout) = self.dw.CreateTextLayout(&s, fmt, 4096.0, 1024.0) {
                let mut lm = [DWRITE_LINE_METRICS::default(); 1];
                let mut n = 0u32;
                if layout.GetLineMetrics(Some(&mut lm), &mut n).is_ok() && n > 0 {
                    return FontMetrics { height: lm[0].height, baseline: lm[0].baseline };
                }
            }
        }
        FontMetrics { height: size * 1.33, baseline: size * 1.06 }
    }

    fn measure(&mut self, style: &Style, f: Font, k: f32, text: &str) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let (fmt, _) = self.font(style, f, k);
        let key = (text.to_string(), FontKey(f, (Self::font_size(style, f, k) * 4.0).round() as u32));
        if let Some(w) = self.widths.get(&key) {
            return *w;
        }
        let s: Vec<u16> = text.encode_utf16().collect();
        let w = unsafe {
            self.dw
                .CreateTextLayout(&s, &fmt, 8192.0, 1024.0)
                .and_then(|l| {
                    let mut m = DWRITE_TEXT_METRICS::default();
                    l.GetMetrics(&mut m).map(|_| m.widthIncludingTrailingWhitespace)
                })
                .unwrap_or(text.len() as f32 * 7.0)
        };
        if self.widths.len() > 4000 {
            self.widths.clear();
        }
        self.widths.insert(key, w);
        w
    }

    fn span_w(&mut self, style: &Style, k: f32, s: &Span) -> f32 {
        let w = self.measure(style, s.font, k, &s.text);
        if s.template.is_empty() {
            w
        } else {
            w.max(self.measure(style, s.font, k, s.template))
        }
    }

    fn spans_w(&mut self, style: &Style, k: f32, spans: &[Span]) -> f32 {
        let mut total = 0.0;
        for (i, s) in spans.iter().enumerate() {
            if i > 0 {
                total += if s.font == Font::Label { 3.0 * k } else { 6.0 * k };
            }
            total += self.span_w(style, k, s);
        }
        total
    }

    /// (wysokość, linia bazowa) wiersza złożonego z podanych fontów.
    fn row_metrics(&mut self, style: &Style, k: f32, fonts: &[Font]) -> (f32, f32) {
        let mut asc: f32 = 0.0;
        let mut desc: f32 = 0.0;
        for f in fonts {
            let (_, m) = self.font(style, *f, k);
            asc = asc.max(m.baseline);
            desc = desc.max(m.height - m.baseline);
        }
        (asc + desc, asc)
    }

    // ───────────────────────────── Rysowanie ─────────────────────────────

    fn text(&mut self, style: &Style, k: f32, f: Font, s: &str, x: f32, baseline_y: f32, color: Color) {
        if s.is_empty() {
            return;
        }
        let (fmt, m) = self.font(style, f, k);
        let wide: Vec<u16> = s.encode_utf16().collect();
        let x = x.round();
        let y = (baseline_y - m.baseline).round();
        let draw = |dx: f32, dy: f32, c: Color| unsafe {
            self.brush.SetColor(&col(c));
            self.rt.DrawText(
                &wide,
                &fmt,
                &rect(x + dx, y + dy, x + dx + 8192.0, y + dy + m.height * 2.0),
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        };
        let o = (k).max(1.0).round();
        match style.text_effect {
            TextEffect::None => {}
            TextEffect::Shadow => draw(o, o, style.effect_color),
            TextEffect::Outline => {
                for (dx, dy) in [(-o, 0.0), (o, 0.0), (0.0, -o), (0.0, o), (-o, -o), (o, o), (-o, o), (o, -o)] {
                    draw(dx, dy, style.effect_color);
                }
            }
        }
        draw(0.0, 0.0, color);
    }

    fn fill_rr(&self, r: D2D_RECT_F, radius: f32, c: Color) {
        unsafe {
            self.brush.SetColor(&col(c));
            self.rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius }, &self.brush);
        }
    }

    fn draw_spans(&mut self, style: &Style, k: f32, spans: &[Span], x0: f32, baseline: f32) -> f32 {
        let mut x = x0;
        for (i, s) in spans.iter().enumerate() {
            if i > 0 {
                x += if s.font == Font::Label { 3.0 * k } else { 6.0 * k };
            }
            let w = self.measure(style, s.font, k, &s.text);
            let slot = self.span_w(style, k, s);
            // Liczby wyrównane do prawej w swoim slocie, jednostki do lewej.
            let tx = if s.template.is_empty() || s.font == Font::Label { x } else { x + slot - w };
            self.text(style, k, s.font, &s.text, tx, baseline, s.color);
            x += slot;
        }
        x
    }

    fn graph(&mut self, style: &Style, k: f32, r: D2D_RECT_F, values: &[f32], max: f32, color: Color, guide: Option<f32>) {
        self.fill_rr(r, 3.0 * k, style.bar_background.with_alpha((style.bar_background.3 as f32 * 0.7) as u8));
        if values.len() < 2 || max <= 0.0 {
            return;
        }
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let inset = 2.0 * k;
        let n = values.len();
        let pts: Vec<D2D_POINT_2F> = values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let x = r.left + w * i as f32 / (n - 1) as f32;
                let y = r.bottom - inset - (v / max).clamp(0.0, 1.0) * (h - inset * 2.0);
                pt(x, y)
            })
            .collect();
        unsafe {
            if let Some(g) = guide {
                let y = r.bottom - inset - (g / max).clamp(0.0, 1.0) * (h - inset * 2.0);
                self.brush.SetColor(&col(style.label.with_alpha(70)));
                self.rt.DrawLine(pt(r.left + 2.0, y), pt(r.right - 2.0, y), &self.brush, 1.0, self.dash.as_ref());
            }
            // Wypełnienie pod wykresem.
            if let Ok(geo) = self.factory.CreatePathGeometry() {
                if let Ok(sink) = geo.Open() {
                    sink.BeginFigure(pt(r.left, r.bottom), D2D1_FIGURE_BEGIN_FILLED);
                    sink.AddLines(&pts);
                    sink.AddLine(pt(r.right, r.bottom));
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                    let _ = sink.Close();
                    self.brush.SetColor(&col(color.with_alpha(40)));
                    self.rt.FillGeometry(&geo, &self.brush, None);
                }
            }
            if let Ok(geo) = self.factory.CreatePathGeometry() {
                if let Ok(sink) = geo.Open() {
                    sink.BeginFigure(pts[0], D2D1_FIGURE_BEGIN_HOLLOW);
                    sink.AddLines(&pts[1..]);
                    sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    let _ = sink.Close();
                    self.brush.SetColor(&col(color));
                    self.rt.DrawGeometry(&geo, &self.brush, 1.4 * k, None);
                }
            }
        }
    }

    fn cores(&mut self, style: &Style, k: f32, r: D2D_RECT_F, values: &[(f32, Color)]) {
        if values.is_empty() {
            return;
        }
        let n = values.len() as f32;
        let gap = if n > 24.0 { 1.0 * k } else { 2.0 * k };
        let bw = ((r.right - r.left - gap * (n - 1.0)) / n).max(1.0);
        let h = r.bottom - r.top;
        for (i, (v, c)) in values.iter().enumerate() {
            let x = r.left + i as f32 * (bw + gap);
            self.fill_rr(rect(x, r.top, x + bw, r.bottom), 1.0 * k, style.bar_background);
            let fh = (h * v / 100.0).clamp(0.0, h);
            if fh > 0.5 {
                self.fill_rr(rect(x, r.bottom - fh, x + bw, r.bottom), 1.0 * k, *c);
            }
        }
    }

    // ───────────────────────────── Układ + render ─────────────────────────────

    /// Renderuje ramkę do powierzchni; zwraca rozmiar w pikselach.
    pub fn render(&mut self, surface: &mut Surface, style: &Style, k: f32, frame: &Frame, min: &mut (f32, f32)) -> (i32, i32) {
        let sz = Sizes { k, pad: style.padding * k, gap: 8.0 * k };
        let (w, h, placed) = match frame.orientation {
            Orientation::Vertical => self.layout_vertical(style, &sz, frame),
            Orientation::Horizontal => self.layout_horizontal(style, &sz, frame),
        };
        // Rozmiar „lepki”: okno może tylko urosnąć do największego dotąd rozmiaru
        // i nie kurczy się, gdy wartości się zmieniają (reset przy zmianie ustawień).
        let w = w.max(min.0);
        let h = h.max(min.1);
        *min = (w, h);
        let (wi, hi) = (w.ceil().max(1.0) as i32, h.ceil().max(1.0) as i32);
        surface.ensure(wi, hi);
        unsafe {
            let r = RECT { left: 0, top: 0, right: wi, bottom: hi };
            if self.rt.BindDC(surface.dc, &r).is_err() {
                return (wi, hi);
            }
            self.rt.BeginDraw();
            self.rt.Clear(Some(&col(Color(0, 0, 0, 0))));
        }

        // Tło i obramowanie.
        let radius = style.corner_radius * k;
        let mut bg = style.background;
        if frame.edit && bg.3 < 140 {
            bg = Color(20, 22, 28, 170);
        }
        if bg.3 > 0 {
            self.fill_rr(rect(0.0, 0.0, w, h), radius, bg);
        }
        let bw = style.border_width * k;
        unsafe {
            if frame.edit {
                self.brush.SetColor(&col(style.accent));
                self.rt.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT { rect: rect(bw, bw, w - bw, h - bw), radiusX: radius, radiusY: radius },
                    &self.brush,
                    (1.5 * k).max(1.0),
                    self.dash.as_ref(),
                );
            } else if style.border && bg.3 > 0 {
                self.brush.SetColor(&col(style.border_color));
                let i = bw / 2.0;
                self.rt.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT { rect: rect(i, i, w - i, h - i), radiusX: (radius - i).max(0.0), radiusY: (radius - i).max(0.0) },
                    &self.brush,
                    bw,
                    None,
                );
            }
        }

        match frame.orientation {
            Orientation::Vertical => self.draw_vertical(style, &sz, frame, w, &placed),
            Orientation::Horizontal => self.draw_horizontal(style, &sz, frame, &placed),
        }

        unsafe {
            let _ = self.rt.EndDraw(None, None);
        }
        (wi, hi)
    }

    fn label_w(&mut self, style: &Style, k: f32, frame: &Frame) -> f32 {
        if !frame.show_labels {
            return 0.0;
        }
        let mut lw: f32 = 0.0;
        for b in &frame.blocks {
            if let Block::Row { label, .. } | Block::Cores { label, .. } = b {
                lw = lw.max(self.measure(style, Font::Label, k, label));
            }
        }
        lw
    }

    fn layout_vertical(&mut self, style: &Style, s: &Sizes, frame: &Frame) -> (f32, f32, Vec<Placed>) {
        let k = s.k;
        let label_w = self.label_w(style, k, frame);
        let label_col = if label_w > 0.0 { label_w + s.gap * 1.5 } else { 0.0 };
        let mut value_w: f32 = 0.0;
        let mut has_bar = false;
        let mut free_w: f32 = 0.0; // elementy na całą szerokość
        for b in &frame.blocks {
            match b {
                Block::Row { spans, bar, .. } => {
                    value_w = value_w.max(self.spans_w(style, k, spans));
                    has_bar |= bar.is_some() && style.show_bars;
                }
                Block::Header { title, subtitle, .. } => {
                    free_w = free_w.max(self.measure(style, Font::Title, k, title));
                    free_w = free_w.max(self.measure(style, Font::Label, k, subtitle));
                }
                Block::Graph { label, caption, .. } => {
                    let cap_w = self.measure(style, Font::Label, k, caption).max(self.measure(style, Font::Label, k, "max 888.8 ms"));
                    let cap = self.measure(style, Font::Label, k, label) + cap_w + s.gap * 2.0;
                    free_w = free_w.max(cap).max(style.graph_width * k);
                }
                Block::Cores { values, .. } => {
                    value_w = value_w.max((values.len() as f32 * 5.0 * k).min(style.graph_width * k));
                }
                Block::Note { text, .. } => free_w = free_w.max(self.measure(style, Font::Label, k, text)),
            }
        }
        let bar_col = if has_bar { s.gap + style.bar_width * k } else { 0.0 };
        let inner = (label_col + value_w + bar_col).max(free_w);
        let rs = style.row_spacing * k;

        let mut y = s.pad;
        let mut placed = Vec::with_capacity(frame.blocks.len());
        for (i, b) in frame.blocks.iter().enumerate() {
            if i > 0 {
                y += rs;
            }
            let h = match b {
                Block::Header { .. } => {
                    let (h1, _) = self.row_metrics(style, k, &[Font::Title]);
                    let (h2, _) = self.row_metrics(style, k, &[Font::Label]);
                    h1 + h2 + 8.0 * k
                }
                Block::Row { spans, .. } => {
                    let mut fonts: Vec<Font> = spans.iter().map(|s| s.font).collect();
                    fonts.push(Font::Label);
                    self.row_metrics(style, k, &fonts).0
                }
                Block::Graph { .. } => self.row_metrics(style, k, &[Font::Label]).0 + 3.0 * k + style.graph_height * k,
                Block::Cores { .. } => (style.graph_height * k * 0.55).max(14.0 * k),
                Block::Note { .. } => self.row_metrics(style, k, &[Font::Label]).0,
            };
            placed.push(Placed { x: s.pad, y, h });
            y += h;
        }
        // Zapamiętaj kolumny w polu x pierwszego elementu (przekazujemy przez szerokość).
        self.cols = (label_col, value_w, bar_col);
        (inner + s.pad * 2.0, y + s.pad, placed)
    }

    fn draw_vertical(&mut self, style: &Style, s: &Sizes, frame: &Frame, w: f32, placed: &[Placed]) {
        let k = s.k;
        let (label_col, value_w, bar_col) = self.cols;
        let right = w - s.pad;
        for (b, p) in frame.blocks.iter().zip(placed) {
            match b {
                Block::Header { title, subtitle, color } => {
                    let (h1, b1) = self.row_metrics(style, k, &[Font::Title]);
                    let (_, b2) = self.row_metrics(style, k, &[Font::Label]);
                    self.text(style, k, Font::Title, title, p.x, p.y + b1, *color);
                    self.text(style, k, Font::Label, subtitle, p.x, p.y + h1 + b2, style.label);
                    let ly = (p.y + p.h - 3.0 * k).round() + 0.5;
                    unsafe {
                        self.brush.SetColor(&col(style.label.with_alpha(50)));
                        self.rt.DrawLine(pt(p.x, ly), pt(right, ly), &self.brush, 1.0, None);
                    }
                }
                Block::Row { label, spans, bar } => {
                    let mut fonts: Vec<Font> = spans.iter().map(|s| s.font).collect();
                    fonts.push(Font::Label);
                    let (rh, base) = self.row_metrics(style, k, &fonts);
                    let baseline = p.y + base + (p.h - rh) / 2.0;
                    if label_col > 0.0 {
                        self.text(style, k, Font::Label, label, p.x, baseline, style.label);
                    }
                    let vx = p.x + label_col;
                    // Wartości wyrównane do prawej krawędzi kolumny wartości.
                    let sw = self.spans_w(style, k, spans);
                    self.draw_spans(style, k, spans, vx + value_w - sw, baseline);
                    if let (Some((frac, c)), true) = (bar, bar_col > 0.0) {
                        let bh = style.bar_height * k;
                        let bx = vx + value_w + s.gap;
                        let by = (p.y + (p.h - bh) / 2.0).round();
                        let bw = right - bx;
                        self.fill_rr(rect(bx, by, right, by + bh), bh / 2.0, style.bar_background);
                        let fw = (bw * frac.clamp(0.0, 1.0)).max(if *frac > 0.0 { bh } else { 0.0 });
                        if fw > 0.0 {
                            self.fill_rr(rect(bx, by, bx + fw, by + bh), bh / 2.0, *c);
                        }
                    }
                }
                Block::Graph { label, caption, values, max, color, guide } => {
                    let (lh, base) = self.row_metrics(style, k, &[Font::Label]);
                    if frame.show_labels {
                        self.text(style, k, Font::Label, label, p.x, p.y + base, style.label);
                    }
                    let cw = self.measure(style, Font::Label, k, caption);
                    self.text(style, k, Font::Label, caption, right - cw, p.y + base, style.label);
                    let gy = p.y + lh + 3.0 * k;
                    self.graph(style, k, rect(p.x, gy, right, gy + style.graph_height * k), values, *max, *color, *guide);
                }
                Block::Cores { label, values } => {
                    let (rh, base) = self.row_metrics(style, k, &[Font::Label]);
                    if label_col > 0.0 {
                        self.text(style, k, Font::Label, label, p.x, p.y + base + (p.h - rh) / 2.0, style.label);
                    }
                    self.cores(style, k, rect(p.x + label_col, p.y, right, p.y + p.h), values);
                }
                Block::Note { text, color } => {
                    let (_, base) = self.row_metrics(style, k, &[Font::Label]);
                    self.text(style, k, Font::Label, text, p.x, p.y + base, *color);
                }
            }
        }
    }

    fn item_w(&mut self, style: &Style, s: &Sizes, frame: &Frame, b: &Block) -> f32 {
        let k = s.k;
        let lw = |g: &mut Self, l: &str| if frame.show_labels && !l.is_empty() { g.measure(style, Font::Label, k, l) + 4.0 * k } else { 0.0 };
        match b {
            Block::Header { title, .. } => self.measure(style, Font::Title, k, title),
            Block::Row { label, spans, .. } => lw(self, label) + self.spans_w(style, k, spans),
            Block::Graph { label, .. } => lw(self, label) + style.graph_width * k * 0.5,
            Block::Cores { label, values } => lw(self, label) + (values.len() as f32 * 4.0 * k).clamp(24.0 * k, 160.0 * k),
            Block::Note { text, .. } => self.measure(style, Font::Label, k, text),
        }
    }

    fn layout_horizontal(&mut self, style: &Style, s: &Sizes, frame: &Frame) -> (f32, f32, Vec<Placed>) {
        let k = s.k;
        let mut fonts = vec![Font::Value, Font::Label];
        if frame.blocks.iter().any(|b| matches!(b, Block::Header { .. })) {
            fonts.push(Font::Title);
        }
        let (rh, _) = self.row_metrics(style, k, &fonts);
        let gap = style.item_spacing * k;
        let mut x = s.pad;
        let mut placed = Vec::new();
        for (i, b) in frame.blocks.iter().enumerate() {
            if i > 0 {
                x += gap;
            }
            let w = self.item_w(style, s, frame, b);
            placed.push(Placed { x, y: s.pad, h: w }); // h = szerokość elementu
            x += w;
        }
        self.cols = (0.0, rh, 0.0);
        (x + s.pad, rh + s.pad * 2.0, placed)
    }

    fn draw_horizontal(&mut self, style: &Style, s: &Sizes, frame: &Frame, placed: &[Placed]) {
        let k = s.k;
        let rh = self.cols.1;
        let mut fonts = vec![Font::Value, Font::Label];
        if frame.blocks.iter().any(|b| matches!(b, Block::Header { .. })) {
            fonts.push(Font::Title);
        }
        let (_, base) = self.row_metrics(style, k, &fonts);
        let gap = style.item_spacing * k;
        for (i, (b, p)) in frame.blocks.iter().zip(placed).enumerate() {
            let baseline = p.y + base;
            if i > 0 && gap >= 8.0 * k {
                // Dyskretna kropka-separator pomiędzy pozycjami.
                let cx = p.x - gap / 2.0;
                let cy = p.y + rh / 2.0;
                let r = 1.4 * k;
                self.fill_rr(rect(cx - r, cy - r, cx + r, cy + r), r, style.label.with_alpha(110));
            }
            let mut x = p.x;
            let label = match b {
                Block::Row { label, .. } | Block::Graph { label, .. } | Block::Cores { label, .. } => label.as_str(),
                _ => "",
            };
            if frame.show_labels && !label.is_empty() {
                self.text(style, k, Font::Label, label, x, baseline, style.label);
                x += self.measure(style, Font::Label, k, label) + 4.0 * k;
            }
            let right = p.x + p.h;
            match b {
                Block::Header { title, color, .. } => self.text(style, k, Font::Title, title, x, baseline, *color),
                Block::Row { spans, .. } => {
                    self.draw_spans(style, k, spans, x, baseline);
                }
                Block::Graph { values, max, color, guide, .. } => {
                    let gh = rh * 0.8;
                    let gy = p.y + (rh - gh) / 2.0;
                    self.graph(style, k, rect(x, gy, right, gy + gh), values, *max, *color, *guide);
                }
                Block::Cores { values, .. } => {
                    let gh = rh * 0.75;
                    let gy = p.y + (rh - gh) / 2.0;
                    self.cores(style, k, rect(x, gy, right, gy + gh), values);
                }
                Block::Note { text, color } => self.text(style, k, Font::Label, text, x, baseline, *color),
            }
        }
    }
}

// ───────────────────────────── Podpis treści ─────────────────────────────

impl Frame {
    /// Tani skrót treści ramki. Jeśli się nie zmienił, nie ma potrzeby
    /// rasteryzować i wysyłać bitmapy do kompozytora.
    pub fn signature(&self, extra: u64) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        extra.hash(&mut h);
        (self.orientation as u8, self.show_labels, self.edit).hash(&mut h);
        let c = |c: &Color, h: &mut std::collections::hash_map::DefaultHasher| (c.0, c.1, c.2, c.3).hash(h);
        for b in &self.blocks {
            match b {
                Block::Header { title, subtitle, color } => {
                    (0u8, title, subtitle).hash(&mut h);
                    c(color, &mut h);
                }
                Block::Row { label, spans, bar } => {
                    (1u8, label).hash(&mut h);
                    for s in spans {
                        (&s.text, s.font).hash(&mut h);
                        c(&s.color, &mut h);
                    }
                    if let Some((f, col)) = bar {
                        ((f * 1000.0) as i32).hash(&mut h);
                        c(col, &mut h);
                    }
                }
                Block::Graph { label, caption, values, max, color, guide } => {
                    (2u8, label, caption, max.to_bits(), guide.map(f32::to_bits)).hash(&mut h);
                    for v in values {
                        v.to_bits().hash(&mut h);
                    }
                    c(color, &mut h);
                }
                Block::Cores { label, values } => {
                    (3u8, label).hash(&mut h);
                    for (v, col) in values {
                        (*v as i32).hash(&mut h);
                        c(col, &mut h);
                    }
                }
                Block::Note { text, color } => {
                    (4u8, text).hash(&mut h);
                    c(color, &mut h);
                }
            }
        }
        h.finish().max(1)
    }
}
