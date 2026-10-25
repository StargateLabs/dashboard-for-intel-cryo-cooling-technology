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
        if d < 20.0 {
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

        let subzero = crate::appearance::active();
        if subzero {
            // Segmented instrument ring; values, scales and warning zones are unchanged.
            for i in 0..48 {
                let from = i as f32 / 48.0;
                let to = (i as f32 + 0.66) / 48.0;
                let color = if from < frac { self.color } else { Color::from_rgb8(25, 46, 28) };
                frame.stroke(&arco(from, to, r), Stroke {
                    style: canvas::Style::Solid(color), width: w * 0.72, ..Default::default()
                });
            }
            for radius in [r - w * 0.8, r + w * 0.95] {
                for i in 0..SEG {
                    frame.stroke(&arco(i as f32 / SEG as f32, (i+1) as f32 / SEG as f32, radius), Stroke {
                        style: canvas::Style::Solid(Color { a: 0.45, ..crate::appearance::LIME }),
                        width: 0.7, ..Default::default()
                    });
                }
            }
        }
        if !subzero {
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

        }

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

        if !subzero {
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

        }

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

        if d < 60.0 { return vec![frame.into_geometry()]; }

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

/// Adaptive circular instrument. It displays the original reading, never a
/// percentage substituted for a physical measurement.
#[derive(Clone)]
pub struct Instrument {
    pub value: Option<f32>,
    pub min: f32,
    pub max: f32,
    pub color: Color,
    pub reading: String,
    pub unit: String,
}

/// Two quiet continuous arcs, with all related readouts inside the dial.
pub struct Cluster {
    pub channels: Vec<Instrument>,
    pub primary_label: &'static str,
    pub details: Vec<(String, String, Color)>,
    pub history: String,
}
impl Program<crate::Message> for Cluster {
    type State = ();
    fn draw(&self, _: &(), renderer: &iced::Renderer, _: &iced::Theme,
        bounds: Rectangle, _: iced::mouse::Cursor) -> Vec<canvas::Geometry> {
        let mut frame=Frame::new(renderer,bounds.size());
        let d=bounds.width.min(bounds.height);
        if d<100.0 || self.channels.is_empty() {return vec![frame.into_geometry()];}
        let center=Point::new(bounds.width*0.5,bounds.height*0.46);
        let start=std::f32::consts::PI*0.70;
        let sweep=std::f32::consts::PI*1.60;
        let polar=|f:f32,r:f32| Point::new(center.x+(start+sweep*f).cos()*r,center.y+(start+sweep*f).sin()*r);
        let primary=&self.channels[0];
        let arc=|radius:f32,from:f32,to:f32| canvas::Path::new(|p|{
            for i in 0..=160 {
                let point=polar(from+(to-from)*i as f32/160.0,radius);
                if i==0 {p.move_to(point);}else{p.line_to(point);}
            }
        });
        // Concentric machined surfaces keep the instrument separate from its panel.
        for (radius,color) in [(0.437,Color::from_rgba8(23,61,45,0.60)),(0.422,Color::from_rgba8(4,20,15,0.48)),
            (0.337,Color::from_rgba8(12,42,30,0.50)),(0.328,Color::from_rgba8(4,20,15,0.42))] {
            frame.fill(&canvas::Path::circle(center,d*radius),color);
        }
        frame.stroke(&canvas::Path::circle(center,d*0.437),Stroke::default()
            .with_width(0.8).with_color(Color::from_rgba8(120,211,167,0.23)));
        for (ring,channel) in self.channels.iter().take(2).enumerate() {
            let radius=d*if ring==0 {0.398}else{0.350};
            let width=d*if ring==0 {0.028}else{0.008};
            frame.stroke(&arc(radius,0.0,1.0),Stroke{width,line_cap:canvas::LineCap::Butt,
                style:canvas::Style::Solid(Color::from_rgba8(115,177,146,0.12)),..Default::default()});
            if let Some(fraction)=channel.fraction().filter(|f|*f>0.0) {
                frame.stroke(&arc(radius,0.0,fraction),Stroke{width:width+3.0,
                    style:canvas::Style::Solid(Color{a:0.09,..channel.color}),..Default::default()});
                frame.stroke(&arc(radius,0.0,fraction),Stroke{width,line_cap:canvas::LineCap::Butt,
                    style:canvas::Style::Solid(channel.color),..Default::default()});
                let marker=canvas::Path::new(|p|{p.move_to(polar(fraction,radius-width));p.line_to(polar(fraction,radius+width));});
                frame.stroke(&marker,Stroke::default().with_width(1.4).with_color(Color::WHITE));
            }
        }
        for i in 0..=50 {
            let f=i as f32/50.0;
            let major=i%10==0;
            let tick=canvas::Path::new(|p|{p.move_to(polar(f,d*0.448));p.line_to(polar(f,d*if major {0.474}else{0.460}));});
            frame.stroke(&tick,Stroke::default().with_width(if major{1.2}else{0.6})
                .with_color(Color::from_rgba8(145,209,173,if major{0.72}else{0.28})));
            if major {
                let value=primary.min+(primary.max-primary.min)*f;
                let point=polar(f,d*0.495);
                frame.fill_text(canvas::Text{content:format!("{value:.0}"),position:point,size:8.0.into(),
                    color:Color::from_rgb8(135,182,155),horizontal_alignment:iced::Alignment::Center.into(),
                    vertical_alignment:iced::Alignment::Center.into(),..Default::default()});
            }
        }
        let bold=iced::Font{weight:iced::font::Weight::Bold,..iced::Font::DEFAULT};
        let reading_size=(d*0.60/(primary.reading.chars().count().max(3) as f32*0.59)).clamp(18.0,43.0);
        let mut labels=vec![
            (self.primary_label.to_owned(),-0.19*d,(d*0.035).clamp(9.0,11.0),Color::from_rgb8(148,196,170),iced::Font::DEFAULT),
            (primary.reading.clone(),-0.055*d,reading_size,Color::from_rgb8(242,249,255),bold),
            (primary.unit.clone(),0.055*d,(d*0.042).clamp(10.0,13.0),primary.color,iced::Font::DEFAULT),
        ];
        let count=self.details.len();
        for (i,(label,value,color)) in self.details.iter().enumerate() {
            let y=if count==1 {0.19*d}else{(0.14+i as f32*0.067)*d};
            labels.push((format!("{label}  {value}"),y,(d*0.037).clamp(9.0,12.0),*color,iced::Font::DEFAULT));
        }
        for (content,y,size,color,font) in labels {
            frame.fill_text(canvas::Text{content,position:Point::new(center.x,center.y+y),size:size.into(),color,font,
                horizontal_alignment:iced::Alignment::Center.into(),vertical_alignment:iced::Alignment::Center.into(),..Default::default()});
        }
        // Lower aperture carries the inner channel's actual unit and scale.
        if let Some(secondary)=self.channels.get(1) {
            frame.fill_text(canvas::Text{content:format!("INT  {:.0}—{:.0} {}",secondary.min,secondary.max,secondary.unit),
                position:Point::new(center.x,center.y+d*0.45),size:8.0.into(),
                color:Color::from_rgb8(117,170,141),horizontal_alignment:iced::Alignment::Center.into(),
                vertical_alignment:iced::Alignment::Center.into(),..Default::default()});
        }
        frame.fill_text(canvas::Text{content:self.history.clone(),
            position:Point::new(center.x,center.y+d*0.52),size:8.0.into(),
            color:Color::from_rgb8(138,186,159),horizontal_alignment:iced::Alignment::Center.into(),
            vertical_alignment:iced::Alignment::Center.into(),..Default::default()});
        vec![frame.into_geometry()]
    }
}
impl Instrument {
    fn fraction(&self) -> Option<f32> {
        self.value.filter(|v| v.is_finite())
            .map(|v| ((v-self.min)/(self.max-self.min).max(0.0001)).clamp(0.0,1.0))
    }
}
impl Program<crate::Message> for Instrument {
    type State = ();
    fn draw(&self, _: &(), renderer: &iced::Renderer, _: &iced::Theme,
        bounds: Rectangle, _: iced::mouse::Cursor) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let d = bounds.width.min(bounds.height);
        if d < 36.0 { return vec![frame.into_geometry()]; }
        let center = frame.center();
        let r = d * 0.38;
        let start = std::f32::consts::PI * 0.75;
        let sweep = std::f32::consts::PI * 1.5;
        let polar = |f: f32, radius: f32| Point {
            x: center.x + (start + sweep*f).cos()*radius,
            y: center.y + (start + sweep*f).sin()*radius,
        };
        frame.fill(&canvas::Path::circle(center, r*0.84), Color::from_rgb8(5, 13, 10));
        let span = (self.max-self.min).max(0.0001);
        let fraction = self.fraction();
        for i in 0..56 {
            let from = i as f32/56.0;
            let to = (i as f32+0.70)/56.0;
            let segment = canvas::Path::new(|p| { p.move_to(polar(from,r)); p.line_to(polar(to,r)); });
            let lit = fraction.is_some_and(|f| from < f);
            frame.stroke(&segment, Stroke::default().with_width(d*0.065)
                .with_color(if lit { self.color } else { Color::from_rgb8(25, 48, 33) }));
            if i%4==0 {
                let tick = canvas::Path::new(|p| {
                    p.move_to(polar(from,r+d*0.055)); p.line_to(polar(from,r+d*0.08));
                });
                frame.stroke(&tick, Stroke::default().with_width(0.8)
                    .with_color(Color::from_rgb8(99, 138, 112)));
            }
        }
        for (radius,color) in [(r*0.83, Color::from_rgba8(95,155,110,0.40)),
            (r+d*0.09, Color::from_rgba8(125,240,60,0.45))] {
            let outline = canvas::Path::new(|p| {
                for i in 0..=96 {
                    let point = polar(i as f32/96.0,radius);
                    if i==0 { p.move_to(point); } else { p.line_to(point); }
                }
            });
            frame.stroke(&outline, Stroke::default().with_width(0.7).with_color(color));
        }
        if let Some(fraction) = fraction {
            let tip = polar(fraction,r+d*0.09);
            frame.fill(&canvas::Path::circle(tip,1.8), self.color);
        }
        let font = iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT };
        let size = (d*0.54/(self.reading.chars().count().max(2) as f32*0.61)).min(d*0.25).max(9.0);
        frame.fill_text(canvas::Text {
            content: self.reading.clone(), position: Point::new(center.x,center.y-d*0.065),
            color: Color::from_rgb8(240,249,242), size:size.into(), font,
            horizontal_alignment: iced::Alignment::Center.into(),
            vertical_alignment: iced::Alignment::Center.into(), ..Default::default()
        });
        frame.fill_text(canvas::Text {
            content:self.unit.clone(), position:Point::new(center.x,center.y+d*0.13),
            color:self.color, size:(d*0.09).clamp(8.0,12.0).into(),
            horizontal_alignment:iced::Alignment::Center.into(),
            vertical_alignment:iced::Alignment::Center.into(), ..Default::default()
        });
        frame.fill_text(canvas::Text {
            content:if span < 1.0 { format!("{:.2}   —   {:.2}",self.min,self.max) } else { format!("{:.1}   —   {:.1}",self.min,self.max) },
            position:Point::new(center.x,center.y+d*0.35),
            color:Color::from_rgb8(130,156,139),size:(d*0.065).clamp(7.0,10.0).into(),
            horizontal_alignment:iced::Alignment::Center.into(),
            vertical_alignment:iced::Alignment::Center.into(), ..Default::default()
        });
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod instrument_tests {
    use super::*;
    #[test]
    fn ring_normalizes_the_physical_scale_without_changing_the_reading() {
        let mut instrument = Instrument { value:Some(-5.0), min:-20.0, max:10.0,
            color:Color::WHITE, reading:"-5.00".into(),unit:"°C".into() };
        assert_eq!(instrument.fraction(),Some(0.5));
        assert_eq!(instrument.reading,"-5.00");
        instrument.value=Some(50.0);
        assert_eq!(instrument.fraction(),Some(1.0));
        assert_eq!(instrument.value,Some(50.0));
    }
    #[test]
    fn missing_or_invalid_sensor_never_lights_a_ring() {
        let mut instrument = Instrument {value:None,min:0.0,max:1.0,color:Color::WHITE,
            reading:"N/D".into(),unit:"bar".into()};
        assert_eq!(instrument.fraction(),None);
        instrument.value=Some(f32::NAN);
        assert_eq!(instrument.fraction(),None);
        assert_eq!(instrument.reading,"N/D");
    }
}
