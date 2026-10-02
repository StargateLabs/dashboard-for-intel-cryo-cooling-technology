//! Export PDF report sessione — usa printpdf.
//! Genera un report A4 con: logo, statistiche sessione, grafici ASCII sparkline,
//! parametri PID, OC Score, e timestamp.

use chrono::Utc;
use printpdf::*;
use std::fs::File;
use std::io::BufWriter;

use crate::session_stats::SessionStats;
use crate::analytics::CopState;

// ── Costanti layout A4 ────────────────────────────────────────────────────────
const A4_W: f32 = 210.0;  // mm
const A4_H: f32 = 297.0;  // mm

// Colori palette Stargate
const GREEN_DARK:  (f32,f32,f32) = (0.01, 0.07, 0.05);
const BLUE_EL:     (f32,f32,f32) = (0.12, 0.45, 1.00);
const NEON_GREEN:  (f32,f32,f32) = (0.00, 0.95, 0.28);
const WHITE:       (f32,f32,f32) = (0.85, 0.97, 0.95);
const TEXT_DIM:    (f32,f32,f32) = (0.40, 0.60, 0.55);

pub struct ReportData<'a> {
    pub stats:       &'a SessionStats,
    pub cop:         &'a CopState,
    pub oc_score:    u32,
    pub oc_best:     u32,
    pub ocp_events:  u32,
    pub p_coef:      f32,
    pub i_coef:      f32,
    pub d_coef:      f32,
    pub set_point:   f32,
    pub max_power:   u8,
    pub session_min: u32,  // minuti
    pub fw_ver:      String,
    pub hw_ver:      u32,
    pub tec_history: Vec<f32>,  // per sparkline
    pub dew_history: Vec<f32>,
}

pub fn export_pdf(data: &ReportData<'_>) -> Result<std::path::PathBuf, String> {
    let (doc, page1, layer1) = PdfDocument::new(
        "StargateLabs CryoCooling Report",
        Mm(A4_W), Mm(A4_H),
        "Page 1"
    );

    let layer = doc.get_page(page1).get_layer(layer1);

    // ── Sfondo header ───────────────────────────────────────────────
    let (r,g,b) = GREEN_DARK;
    layer.set_fill_color(Color::Rgb(Rgb::new(r, g, b, None)));
    layer.add_rect(Rect::new(Mm(0.0), Mm(A4_H - 48.0), Mm(A4_W), Mm(48.0)));

    // ── Titolo ──────────────────────────────────────────────────────
    let font_bold = doc.add_builtin_font(BuiltinFont::HelveticaBold)
        .map_err(|e| e.to_string())?;
    let font = doc.add_builtin_font(BuiltinFont::Helvetica)
        .map_err(|e| e.to_string())?;
    let font_mono = doc.add_builtin_font(BuiltinFont::Courier)
        .map_err(|e| e.to_string())?;

    let (r,g,b) = NEON_GREEN;
    layer.set_fill_color(Color::Rgb(Rgb::new(r,g,b,None)));
    layer.use_text("STARGATE LABS", 22.0, Mm(12.0), Mm(A4_H - 18.0), &font_bold);

    let (r,g,b) = BLUE_EL;
    layer.set_fill_color(Color::Rgb(Rgb::new(r,g,b,None)));
    layer.use_text("CryoCooling Dashboard — Session Report", 13.0, Mm(12.0), Mm(A4_H - 32.0), &font_bold);

    let (r,g,b) = TEXT_DIM;
    layer.set_fill_color(Color::Rgb(Rgb::new(r,g,b,None)));
    let ts = Utc::now().format("%Y-%m-%d %H:%M UTC").to_string();
    layer.use_text(
        format!("Generated: {}   FW {}   HW {}", ts, data.fw_ver, data.hw_ver),
        8.0, Mm(12.0), Mm(A4_H - 44.0), &font,
    );

    // ── Linea separatrice ────────────────────────────────────────────
    let (r,g,b) = BLUE_EL;
    layer.set_outline_color(Color::Rgb(Rgb::new(r,g,b,None)));
    layer.set_outline_thickness(0.5);
    layer.add_line(Line {
        points: vec![
            (Point::new(Mm(12.0), Mm(A4_H - 52.0)), false),
            (Point::new(Mm(A4_W - 12.0), Mm(A4_H - 52.0)), false),
        ],
        is_closed: false,
    });

    // ── OC Score box ─────────────────────────────────────────────────
    let (r,g,b) = (0.005, 0.060, 0.045);
    layer.set_fill_color(Color::Rgb(Rgb::new(r,g,b,None)));
    layer.add_rect(Rect::new(Mm(140.0), Mm(A4_H - 90.0), Mm(58.0), Mm(32.0)));

    let (r,g,b) = BLUE_EL;
    layer.set_fill_color(Color::Rgb(Rgb::new(r,g,b,None)));
    layer.use_text("OC SCORE", 9.0, Mm(155.0), Mm(A4_H - 63.0), &font_bold);

    let score_str = format!("{}", data.oc_score);
    let (r,g,b) = NEON_GREEN;
    layer.set_fill_color(Color::Rgb(Rgb::new(r,g,b,None)));
    layer.use_text(&score_str, 32.0, Mm(152.0), Mm(A4_H - 82.0), &font_bold);

    let (r,g,b) = TEXT_DIM;
    layer.set_fill_color(Color::Rgb(Rgb::new(r,g,b,None)));
    layer.use_text(format!("Best ever: {}", data.oc_best), 8.0, Mm(148.0), Mm(A4_H - 89.0), &font);

    // ── Statistiche sessione ─────────────────────────────────────────
    let mut y: f32 = A4_H - 62.0;
    let col_kv = |layer: &PdfLayerReference, k: &str, v: &str, y: f32| {
        let (r,g,b) = TEXT_DIM;
        layer.set_fill_color(Color::Rgb(Rgb::new(r,g,b,None)));
        layer.use_text(k, 9.0, Mm(12.0), Mm(y), &font);
        let (r,g,b) = WHITE;
        layer.set_fill_color(Color::Rgb(Rgb::new(r,g,b,None)));
        layer.use_text(v, 9.0, Mm(65.0), Mm(y), &font_bold);
    };

    let section_title = |layer: &PdfLayerReference, title: &str, y: f32| {
        let (r,g,b) = BLUE_EL;
        layer.set_fill_color(Color::Rgb(Rgb::new(r,g,b,None)));
        layer.use_text(title, 10.0, Mm(12.0), Mm(y), &font_bold);
        let (r,g,b) = (0.15,0.30,0.60);
        layer.set_outline_color(Color::Rgb(Rgb::new(r,g,b,None)));
        layer.set_outline_thickness(0.3);
        layer.add_line(Line {
            points: vec![
                (Point::new(Mm(12.0), Mm(y-2.0)), false),
                (Point::new(Mm(135.0), Mm(y-2.0)), false),
            ],
            is_closed: false,
        });
    };

    section_title(&layer, "THERMAL", y);
    y -= 10.0;
    let s = &data.stats;
    col_kv(&layer, "TEC Temp (min / avg / max)", &format!("{:.1} / {:.1} / {:.1} °C",
        s.tec_temp.min, s.tec_temp.avg(), s.tec_temp.max), y); y -= 7.0;
    col_kv(&layer, "Condensation Margin (min)",
        &format!("{:.2} °C", s.condensation_margin.min), y); y -= 7.0;
    col_kv(&layer, "COP (Coefficient of Performance)",
        &format!("{:.3}  (eff. {:.0}%)", data.cop.cop, data.cop.efficiency_pct), y); y -= 7.0;
    col_kv(&layer, "Uptime", &format!("{} min", data.session_min), y); y -= 7.0;

    y -= 4.0;
    section_title(&layer, "ELECTRICAL", y);
    y -= 10.0;
    col_kv(&layer, "Power (min / avg / max)", &format!("{:.0} / {:.0} / {:.0} W",
        s.tec_power_watts.min, s.tec_power_watts.avg(), s.tec_power_watts.max), y); y -= 7.0;
    col_kv(&layer, "Voltage (min / avg / max)", &format!("{:.2} / {:.2} / {:.2} V",
        s.tec_voltage.min, s.tec_voltage.avg(), s.tec_voltage.max), y); y -= 7.0;
    col_kv(&layer, "Current (min / avg / max)", &format!("{:.2} / {:.2} / {:.2} A",
        s.tec_current.min, s.tec_current.avg(), s.tec_current.max), y); y -= 7.0;
    col_kv(&layer, "OCP Events", &format!("{}", data.ocp_events), y); y -= 7.0;

    y -= 4.0;
    section_title(&layer, "PID PARAMETERS", y);
    y -= 10.0;
    col_kv(&layer, "P / I / D coefficients",
        &format!("P={:.3}  I={:.3}  D={:.3}", data.p_coef, data.i_coef, data.d_coef), y); y -= 7.0;
    col_kv(&layer, "Setpoint offset / Max power",
        &format!("{:.1}°C  /  {}%", data.set_point, data.max_power), y); y -= 7.0;

    // ── Sparkline TEC + Dew ─────────────────────────────────────────
    y -= 8.0;
    section_title(&layer, "TEMPERATURE SPARKLINE (session)", y);
    y -= 12.0;

    if data.tec_history.len() > 4 {
        let spark = ascii_sparkline(&data.tec_history, 90);
        let (r,g,b) = (0.0, 0.85, 0.55);
        layer.set_fill_color(Color::Rgb(Rgb::new(r,g,b,None)));
        layer.use_text(format!("TEC: {}", spark), 7.5, Mm(12.0), Mm(y), &font_mono);
        y -= 6.0;
    }
    if data.dew_history.len() > 4 {
        let spark = ascii_sparkline(&data.dew_history, 90);
        let (r,g,b) = (1.0, 0.75, 0.0);
        layer.set_fill_color(Color::Rgb(Rgb::new(r,g,b,None)));
        layer.use_text(format!("DEW: {}", spark), 7.5, Mm(12.0), Mm(y), &font_mono);
    }

    // ── Footer ───────────────────────────────────────────────────────
    let (r,g,b) = TEXT_DIM;
    layer.set_fill_color(Color::Rgb(Rgb::new(r,g,b,None)));
    layer.use_text(
        "StargateLabs CryoCooling Dashboard v2.0 — report generato automaticamente",
        7.0, Mm(12.0), Mm(10.0), &font,
    );

    // ── Salvataggio ───────────────────────────────────────────────────
    let path = report_path();
    let file = File::create(&path).map_err(|e| format!("Errore creazione PDF: {}", e))?;
    doc.save(&mut BufWriter::new(file))
        .map_err(|e| format!("Errore salvataggio PDF: {}", e))?;

    Ok(path)
}

/// Genera una sparkline ASCII da un vettore di valori.
fn ascii_sparkline(data: &[f32], width: usize) -> String {
    let chars = ['▁','▂','▃','▄','▅','▆','▇','█'];
    let step = (data.len().max(1) as f64 / width as f64).max(1.0);
    let min = data.iter().cloned().fold(f32::MAX, f32::min);
    let max = data.iter().cloned().fold(f32::MIN, f32::max);
    let range = (max - min).max(0.01);

    (0..width).map(|i| {
        let idx = ((i as f64 * step) as usize).min(data.len() - 1);
        let v = (data[idx] - min) / range;
        chars[(v * 7.0).clamp(0.0, 7.0) as usize]
    }).collect()
}

fn report_path() -> std::path::PathBuf {
    let ts = Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let desktop = dirs::desktop_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    desktop.join(format!("stargate_cryo_report_{}.pdf", ts))
}
