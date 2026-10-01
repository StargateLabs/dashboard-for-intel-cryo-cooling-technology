//! Quadranti (donut) per l'hero.
//!
//! Riferimento visivo: anello spesso a capoli arrotondati, valore numerico
//! grande al centro, unita' piccola e spenta sotto il numero, nome del canale
//! **fuori** dal quadrante. Nessuna lancetta: tagliava il testo in due.
//!
//! Il quadrante non e' solo decorativo: dove serve evidenzia in rosso la zona
//! di pericolo lungo l'anello, cosi' l'utente vede a colpo d'occhio quanto
//! e' vicino al limite.

use iced::widget::canvas::{self, Canvas, Frame, Program, Stroke};
use iced::{Color, Length, Point, Rectangle, Size};

/// Tratto di arco da colorare come pericolo.
#[derive(Debug, Clone, Copy)]
pub struct DangerZone {
    /// Inizio della zona, nella stessa unita' del valore.
    pub from: f32,
    /// Fine della zona.
    pub to: f32,
}

/// Programma di disegno del quadrante.
#[derive(Debug, Clone)]
pub struct Gauge {
    /// Valore corrente.
    pub value: f32,
    /// Fondo scala.
    pub min: f32,
    /// Fondo scala.
    pub max: f32,
    /// Colore del valore (deciso dal chiamante in base allo stato).
    pub color: Color,
    /// Fase animazione 0..1, per il respiro dell'alone.
    pub phase: f32,
    /// Zone di pericolo da evidenziare in rosso lungo l'anello.
    pub danger: Vec<DangerZone>,
    /// Testo sotto il numero (es. "°C"). Dentro il quadrante solo l'unita':
    /// il nome del canale va fuori, vedi `gauge()`.
    pub unit: &'static str,
}

/// L'arco parte da 135 gradi e copre 270: guarda in su e lascia libera la
/// parte bassa, dove il chiamante mette l'etichetta del canale.
const START: f32 = std::f32::consts::FRAC_PI_3 * 0.75; // 135°
const SWEEP: f32 = std::f32::consts::PI * 1.5; // 270°
/// Segmenti per l'arco: 96 e' abbastanza liscio da sembrare curvo a 124 px.
const SEG: usize = 96;

impl Program<crate::Message> for Gauge {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let size: Size = bounds.size();
        let d = size.width.min(size.height);
        // Sotto ~60 px non c'e' spazio per un numero leggibile dentro
        // l'anello: meglio non disegnare che disegnare sporco.
        if d < 60.0 {
            return Vec::new();
        }

        let mut frame = Frame::new(renderer, size);
        let center = frame.center();

        let span = (self.max - self.min).max(0.0001);
        let frac = ((self.value - self.min) / span).clamp(0.0, 1.0);

        // ── Geometria ────────────────────────────────────────────
        // r - w/2 = raggio interno libero: e' la larghezza utile per il testo.
        let r = d * 0.355;
        let w = d * 0.135;
        let r_inner = r - w * 0.5;

        let polar = |f: f32, radius: f32| -> Point {
            let a = START + SWEEP * f;
            Point {
                x: center.x + a.cos() * radius,
                y: center.y + a.sin() * radius,
            }
        };

        let arco = |da: f32, a_db: f32, radius: f32| -> canvas::Path {
            let p0 = polar(da, radius);
            let p1 = polar(a_db, radius);
            canvas::Path::new(|p| {
                p.move_to(p0);
                p.line_to(p1);
            })
        };

        // ── Anello di fondo ──────────────────────────────────────
        let track = canvas::Path::new(|p| {
            for i in 0..=SEG {
                let pt = polar(i as f32 / SEG as f32, r);
                if i == 0 { p.move_to(pt); } else { p.line_to(pt); }
            }
        });
        frame.stroke(
            &track,
            Stroke {
                style: canvas::Style::Solid(Color { r: 0.09, g: 0.14, b: 0.17, a: 1.0 }),
                width: w,
                line_cap: canvas::LineCap::Round,
                ..Default::default()
            },
        );

        // ── Zone di pericolo, dentro l'anello di fondo ───────────
        // Sono contesto, non dato: alone sottile, mai pieno spessore.
        for z in &self.danger {
            let f0 = ((z.from - self.min) / span).clamp(0.0, 1.0);
            let f1 = ((z.to - self.min) / span).clamp(0.0, 1.0);
            if f1 <= f0 {
                continue;
            }
            frame.stroke(
                &arco(f0, f1, r),
                Stroke {
                    style: canvas::Style::Solid(Color {
                        r: 0.85, g: 0.12, b: 0.18, a: 0.50,
                    }),
                    width: w * 0.42,
                    line_cap: canvas::LineCap::Round,
                    ..Default::default()
                },
            );
        }

        // ── Arco di valore con alone ──────────────────────────────
        // Respiro lento (0,16 Hz): abbastanza da sembrare vivo, non da
        // stancare. L'alone nasce da passate successive, non da ombre.
        let pulse = 0.5 + 0.5 * (self.phase * std::f32::consts::TAU).sin();
        let n_val = (frac * SEG as f32).ceil().max(1.0) as usize;
        let val_path = canvas::Path::new(|p| {
            for i in 0..=n_val {
                let pt = polar(i as f32 / SEG as f32, r);
                if i == 0 { p.move_to(pt); } else { p.line_to(pt); }
            }
        });
        for (k, a) in [0.26_f32, 0.13].iter().enumerate() {
            frame.stroke(
                &val_path,
                Stroke {
                    style: canvas::Style::Solid(
                        Color { a: a * (0.75 + 0.25 * pulse), ..self.color },
                    ),
                    width: w * (1.0 + k as f32 * 0.42),
                    line_cap: canvas::LineCap::Round,
                    ..Default::default()
                },
            );
        }
        frame.stroke(
            &val_path,
            Stroke {
                style: canvas::Style::Solid(self.color),
                width: w * 0.72,
                line_cap: canvas::LineCap::Round,
                ..Default::default()
            },
        );

        // Punto di lettura sulla testa dell'arco: dice dove si e' arrivati
        // anche a colpo d'occhio, e sostituisce la lancetta.
        let testa = polar(frac, r);
        frame.fill(
            &canvas::Path::circle(testa, w * 0.36),
            Color { a: 0.95, ..self.color },
        );

        // ── Tacche di scala, fuori dall'anello ────────────────────
        // Solo cinque: la scala si indovina, non si legge un numero per
        // tacca, quindi tacche in piu' aggiungono solo rumore.
        for i in 0..=4 {
            let f = i as f32 / 4.0;
            let p = canvas::Path::new(|pp| {
                pp.move_to(polar(f, r + w * 0.62));
                pp.line_to(polar(f, r + w * 0.92));
            });
            frame.stroke(
                &p,
                Stroke {
                    style: canvas::Style::Solid(Color {
                        r: 0.32, g: 0.44, b: 0.48, a: 0.60,
                    }),
                    width: 1.5,
                    ..Default::default()
                },
            );
        }

        // ── Numero al centro ─────────────────────────────────────
        // Cio' che e' dentro l'anello e' SOLO il dato. Il nome del canale e
        // l'unita' non ci stanno: prima erano dentro anche e il testo
        // finiva sopra l'arco.
        let mut valore = canvas::Text::default();
        valore.content = format!("{:.0}", self.value);
        valore.position = Point {
            x: center.x,
            y: center.y - r_inner * 0.18,
        };
        valore.size = (d * 0.30).clamp(18.0, 40.0).into();
        valore.color = self.color;
        valore.horizontal_alignment = iced::Alignment::Center.into();
        valore.vertical_alignment = iced::Alignment::Center.into();
        frame.fill_text(valore);

        // ── Unita' sotto il numero, piccola e spenta ─────────────
        let mut unita = canvas::Text::default();
        unita.content = self.unit.to_string();
        unita.position = Point {
            x: center.x,
            y: center.y + r_inner * 0.52,
        };
        unita.size = (d * 0.11).clamp(9.0, 15.0).into();
        unita.color = Color { r: 0.46, g: 0.62, b: 0.68, a: 1.0 };
        unita.horizontal_alignment = iced::Alignment::Center.into();
        unita.vertical_alignment = iced::Alignment::Center.into();
        frame.fill_text(unita);

        vec![frame.into_geometry()]
    }
}

/// Crea il widget quadrante.
///
/// `size` deve essere >= 60 px: sotto, il numero dentro l'anello non e'
/// leggibile e il disegno viene saltato.
pub fn gauge(g: Gauge, size: f32) -> Canvas<Gauge, crate::Message> {
    Canvas::new(g)
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
}
