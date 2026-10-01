//! Grafici real-time con design moderno — palette neon blue/cyan su sfondo dark.

use std::collections::VecDeque;
use std::time::Duration;

use chrono::{DateTime, Utc};
use iced::{
    widget::{
        canvas::{Cache, Frame, Geometry},
        Column, Container,
    },
    Element, Length, Size,
};
use plotters::{
    prelude::ChartBuilder,
    series::AreaSeries,
    style::{Color, IntoFont, RGBAColor, RGBColor, ShapeStyle},
};
use plotters_backend::{DrawingBackend};
use plotters_iced::{Chart, ChartWidget, Renderer};

use crate::Message;
pub const ANIMATION_MS: u64 = 33;
const PRESENTATION_DELAY_MS: i64 = 1000;

// A one-second presentation buffer allows interpolation between real samples.
// Sensor values and histories used by control, logs and exports stay untouched.
fn presentation_points(points: &VecDeque<(DateTime<Utc>, f32)>, end: DateTime<Utc>) -> Vec<(DateTime<Utc>, f32)> {
    let mut result = Vec::with_capacity(points.len());
    let mut newer: Option<(DateTime<Utc>, f32)> = None;
    for &(t,v) in points {
        if t > end { newer = Some((t,v)); continue; }
        if result.is_empty() {
            if let Some((nt,nv)) = newer {
                let gap = (nt-t).num_milliseconds();
                if gap > 0 && gap <= 2500 && t < end {
                    let alpha=(end-t).num_milliseconds() as f32/gap as f32;
                    result.push((end,v+(nv-v)*alpha));
                }
            }
        }
        result.push((t,v));
    }
    result
}
fn refresh_frame(cache: &Cache, slot: &std::cell::Cell<i64>) {
    let frame=Utc::now().timestamp_millis()/ANIMATION_MS as i64;
    if slot.replace(frame)!=frame { cache.clear(); }
}

#[cfg(test)]
mod presentation_tests {
    use super::*;
    #[test]
    fn interpolation_preserves_samples_and_never_overshoots() {
        let t=Utc::now();
        let points=VecDeque::from([(t,40.0),(t-chrono::Duration::milliseconds(500),20.0)]);
        let before=points.clone();
        let rendered=presentation_points(&points,t-chrono::Duration::milliseconds(250));
        assert_eq!(rendered[0].1,30.0);
        assert_eq!(rendered[1],points[1]);
        assert_eq!(points,before);
        assert!(rendered.iter().all(|p|p.0<=t-chrono::Duration::milliseconds(250)));
    }
    #[test]
    fn no_extrapolation_or_interpolation_across_missing_data() {
        let t=Utc::now();
        let points=VecDeque::from([(t,40.0),(t-chrono::Duration::seconds(10),20.0)]);
        assert_eq!(presentation_points(&points,t-chrono::Duration::milliseconds(250)),vec![points[1]]);
        assert_eq!(presentation_points(&points,t+chrono::Duration::seconds(1)),points.iter().copied().collect::<Vec<_>>());
    }
    #[test]
    fn buffered_curve_advances_between_samples_without_changing_measurements() {
        let t=Utc::now();
        let points=VecDeque::from([(t,60.0),(t-chrono::Duration::milliseconds(500),40.0),
            (t-chrono::Duration::milliseconds(1000),20.0),(t-chrono::Duration::milliseconds(1500),0.0)]);
        let original=points.clone();
        assert_eq!(presentation_points(&points,t-chrono::Duration::milliseconds(1100))[0].1,16.0);
        assert_eq!(presentation_points(&points,t-chrono::Duration::milliseconds(1050))[0].1,18.0);
        assert_eq!(points,original);
    }
    #[test]
    fn hover_uses_time_axis_instead_of_sample_count() {
        let t=Utc::now();
        let points=VecDeque::from([(t,40.0),(t-chrono::Duration::seconds(90),20.0)]);
        let bounds=iced::Rectangle {x:0.0,y:0.0,width:662.0,height:200.0};
        assert_eq!(hover_at(Some(356.0),&points,bounds,t,Duration::from_secs(180)),Some(1));
        assert_eq!(hover_at(Some(10.0),&points,bounds,t,Duration::from_secs(180)),None);
    }
    #[test]
    fn invalid_samples_do_not_damage_chart_or_scale() {
        let mut chart=MonitoringChart::new("T",0.0,50.0,"C",C_TEC_TEMP);
        chart.push(Utc::now(),f32::NAN);
        assert!(chart.points.is_empty());
        let mut overlap=OverlapChart::new();
        overlap.push(Utc::now(),20.0,16.0);
        let range=overlap.y_finestra.get().unwrap();
        assert!(range.0<=16.0 && range.1>=20.0);
        overlap.push(Utc::now(),f32::NAN,16.0);
        assert_eq!(overlap.tec.len(),1);
        assert_eq!(overlap.y_finestra.get(),Some(range));
    }
}

fn hover_at(x: Option<f32>, points: &VecDeque<(DateTime<Utc>, f32)>, bounds: iced::Rectangle, end: DateTime<Utc>, window: Duration) -> Option<usize> {
    let plot_width=bounds.width-62.0;
    let relative=x?-bounds.x-56.0;
    if plot_width<=0.0 || !(0.0..=plot_width).contains(&relative) { return None; }
    let target=end.timestamp_millis()-((1.0-relative/plot_width)*window.as_millis() as f32) as i64;
    points.iter().enumerate().filter(|(_,p)|p.0<=end)
        .min_by_key(|(_,p)|(p.0.timestamp_millis()-target).unsigned_abs())
        .filter(|(_,p)|(p.0.timestamp_millis()-target).unsigned_abs()<=2500)
        .map(|(i,_)|i)
}
// ── Design tokens ─────────────────────────────────────────────────────────────
#[allow(dead_code)]
const BG_CHART:   &str = "#010f1e";   // sfondo interno singolo grafico
const GRID_COLOR:  RGBAColor = RGBAColor(95, 160, 185, 0.12);  // verde tenue per la griglia
const LABEL_CLR:   RGBColor  = RGBColor(125, 166, 184);            // verde medio per label assi

// Palette semantica — un colore per canale
const C_TEC_TEMP:  RGBColor = RGBColor(0,   210, 255);  // cyan  — freddo
/// Punto di rugiada.
///
/// **Blu acqua, non ambra.** Con l'arancio la rugiada si confondeva con la
/// linea CPU: due curve calde, una delle due e' una soglia di sicurezza, e
/// non si capiva quale fosse. Il blu la stacca dalla piastra TEC (ciano) per
/// intensita', ma non la sovrappone: sono due temperature diverse con un
/// significato opposto, e devono restare distinguibili a colpo d'occhio.
const C_DEW_POINT: RGBColor = RGBColor(70,  140, 255);  // blue — soglia umidità
/// CPU nel grafico composito: arancio **chiaro**, tratteggiato.
///
/// Piu' chiaro del vecchio RGBColor(255,160,50) e sottile di spessore 1, cosi'
/// non compete con la rugiada e non sembra una soglia di sicurezza.
const C_CPU_LINE:   RGBColor = RGBColor(255, 190, 110);
const C_VOLTAGE:   RGBColor = RGBColor(80,  180, 255);  // blue  — tensione
const C_CURRENT:   RGBColor = RGBColor(170,  80, 255);  // viola — corrente
const C_POWER_W:   RGBColor = RGBColor(255,  50,  80);  // red   — potenza reale
const C_POWER_PCT: RGBColor = RGBColor(220, 220,   0);  // yellow— livello %
const C_HUMIDITY:  RGBColor = RGBColor(0,   200, 150);  // teal  — umidità
const C_PCB_TEMP:  RGBColor = RGBColor(255, 200,  60);  // gold  — PCB

// ─────────────────────────────────────────────────────────────────────────────

pub struct ChartGroup {
    tec_temp:  MonitoringChart,
    dew_point: MonitoringChart,
    overlap:   OverlapChart,     // grafico composito TEC+CPU+Dew
    voltage:   MonitoringChart,
    current:   MonitoringChart,
    power_w:   MonitoringChart,
    power_pct: MonitoringChart,
    humidity:  MonitoringChart,
    pcb_temp:  MonitoringChart,
}

impl Default for ChartGroup {
    fn default() -> Self {
        Self {
            overlap:   OverlapChart::new(),
            tec_temp:  MonitoringChart::new("Temp. TEC",       -20.0,  10.0, "°C", C_TEC_TEMP),
            dew_point: MonitoringChart::new("Punto di rugiada",-15.0,  25.0, "°C", C_DEW_POINT),
            voltage:   MonitoringChart::new("Tensione TEC",      8.0,  13.0, "V",  C_VOLTAGE),
            current:   MonitoringChart::new("Corrente TEC",      0.0,  15.0, "A",  C_CURRENT),
            power_w:   MonitoringChart::new("Potenza TEC",       0.0, 200.0, "W",  C_POWER_W),
            power_pct: MonitoringChart::new("Livello potenza",   0.0, 100.0, "%",  C_POWER_PCT),
            humidity:  MonitoringChart::new("Umidità",          30.0,  80.0, "%",  C_HUMIDITY),
            pcb_temp:  MonitoringChart::new("Temp. scheda",     20.0,  60.0, "°C", C_PCB_TEMP),
        }
    }
}

impl ChartGroup {
    pub fn update(&mut self, data: cryo_cooler_controller_lib::MonitoringData) {
        self.overlap  .push(data.timestamp, data.tec_temperature, data.dew_point_temperature);
        self.tec_temp .push(data.timestamp, data.tec_temperature);
        self.dew_point.push(data.timestamp, data.dew_point_temperature);
        self.voltage  .push(data.timestamp, data.tec_voltage);
        self.current  .push(data.timestamp, data.tec_current);
        self.power_w  .push(data.timestamp, data.tec_power_watts);
        self.power_pct.push(data.timestamp, data.tec_power_level as f32);
        self.humidity .push(data.timestamp, data.humidity);
        self.pcb_temp .push(data.timestamp, data.pcb_temperature);
    }

    pub fn last_tec_temp(&self) -> f32 {
        self.tec_temp.points.front().map(|p| p.1).unwrap_or(0.0)
    }

    /// C'e' almeno un campione di temperatura della piastra?
    ///
    /// `last_tec_temp` senza campioni restituisce `0.0`, e `0.0` e' un numero
    /// che **non esiste come temperatura**: fa scattare tutto quello che
    /// confronta la piastra con una soglia, quindi all'avvio l'avviso
    /// cryogenic compariva senza che esistesse una misura. Con questo
    /// controllo si distingue "nessun dato" da "zero gradi".
    pub fn ha_campione_tec(&self) -> bool {
        !self.tec_temp.points.is_empty()
    }

    /// Il banner cryogenic e' visibile?
    ///
    /// Condizione: la piastra e' **sotto** la soglia. Il rischio cryogenico
    /// e' la piastra troppo fredda.
    ///
    /// **Nessun campione, nessun banner.** Senza misura `last_tec_temp`
    /// restituisce `0.0`, e `0.0` non e' una temperatura: e' sotto la soglia,
    /// quindi all'avvio l'avviso compariva da solo, prima ancora di leggere
    /// qualcosa. "Nessun dato" non puo' voler dire "piastra fredda".
    pub fn cryo_visibile(&self) -> bool {
        cryo_visibile_a(self.last_tec_temp(), self.ha_campione_tec())
    }

    /// Dati per sparkline PDF — ultimi N punti TEC
    pub fn tec_sparkline(&self) -> Vec<f32> {
        self.tec_temp.points.iter().rev().map(|p| p.1).collect()
    }

    /// Ultimi N punti di potenza, per la mini-sparkline nel tasto TEC.
    pub fn power_sparkline(&self) -> Vec<f32> {
        self.power_w.points.iter().rev().map(|p| p.1).collect()
    }

    /// Dati per sparkline PDF — ultimi N punti dew point
    pub fn dew_sparkline(&self) -> Vec<f32> {
        self.dew_point.points.iter().rev().map(|p| p.1).collect()
    }

    pub fn last_humidity(&self) -> String {
        format!("{:.1}%", self.humidity.points.front().map(|p| p.1).unwrap_or(0.0))
    }

    /// Layout verticale con gruppi semantici — tutti visibili senza scroll.
    pub fn update_cpu_temp(&mut self, cpu_temp: f32) {
        self.overlap.push_cpu(cpu_temp);
    }

    /// Layout a colonna singola — per finestre strette (portrait) o medie.
    /// Tutti i grafici in colonna verticale, nessuno scrolling.
    pub fn view(&self) -> Element<'_, Message> {
        Column::new()
            .width(Length::Fill)
            .height(Length::Fill)
            .spacing(4)
            .padding([0, 0])
            .push(group_label("TERMICA CRITICA — TEC + CPU + Rugiada"))
            .push(overlap_card(&self.overlap))
            .push(chart_fill(&self.tec_temp))
            .push(chart_fill(&self.dew_point))
            .push(group_label("ELETTRICA"))
            .push(chart_fill(&self.voltage))
            .push(chart_fill(&self.current))
            .push(group_label("POTENZA"))
            .push(chart_fill(&self.power_w))
            .push(chart_fill(&self.power_pct))
            .push(group_label("AMBIENTE"))
            .push(chart_fill(&self.humidity))
            .push(chart_fill(&self.pcb_temp))
            .into()
    }

    /// Layout a due colonne — per finestre larghe.
    ///
    /// In colonna singola i 9 grafici diventano altissimi e la finestra
    /// richiede molto scrolling, mentre lo spazio orizzontale resta vuoto.
    /// Qui si divide in due colonne: i gruppi termici a sinistra, elettrici /
    /// potenza / ambiente a destra. Stessa quantità di grafici, altezza dimezzata.
    pub fn view_wide(&self) -> Element<'_, Message> {
        let left = Column::new()
            .width(Length::FillPortion(50))
            .height(Length::Fill)
            .spacing(7)
            .push(group_label("TERMICA CRITICA — TEC + CPU + Rugiada"))
            .push(overlap_card(&self.overlap))
            .push(chart_fill(&self.tec_temp))
            .push(chart_fill(&self.dew_point));

        let right = Column::new()
            .width(Length::FillPortion(50))
            .height(Length::Fill)
            .spacing(7)
            .push(group_label("ELETTRICA"))
            .push(chart_fill(&self.voltage))
            .push(chart_fill(&self.current))
            .push(group_label("POTENZA"))
            .push(chart_fill(&self.power_w))
            .push(chart_fill(&self.power_pct))
            .push(group_label("AMBIENTE"))
            .push(chart_fill(&self.humidity))
            .push(chart_fill(&self.pcb_temp));

        Row::new()
            .spacing(0)
            .width(Length::Fill)
            .height(Length::Fill)
            .push(left)
            .push(right)
            .into()
    }
}

use iced::widget::Row;

// ── Helpers layout ────────────────────────────────────────────────────────────

fn group_label(text: &'static str) -> Element<'static, Message> {
    use iced::widget::{Row as IRow, Text};
    // Solo testo colorato, nessuna linea divisoria
    IRow::new()
        .padding(iced::Padding { top: 8.0, right: 8.0, bottom: 2.0, left: 8.0 })
        .push(
            Text::new(text)
                .size(11)
                .color(
                    // Verde-azzurro che sposa sia il verde petrolio che il blu
                    iced::Color { r: 0.10, g: 0.70, b: 0.55, a: 1.0 }
                ),
        )
        .width(Length::Fill)
        .into()
}

/// Variante Fill — grafico si espande per riempire la quota proporzionale.
/// 8 grafici con FillPortion(1) ciascuno si dividono tutto lo spazio verticale.
fn chart_fill(chart: &MonitoringChart) -> Element<'_, Message> {
    Container::new(
        ChartWidget::new(chart)
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::FillPortion(1))
    .padding(iced::Padding { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 })
    .style(|_: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(iced::Color { r: 0.009, g: 0.065, b: 0.050, a: 0.86 })),
            border: iced::Border { color: iced::Color { r: 0.0, g: 0.55, b: 0.35, a: 0.20 }, width: 1.0, radius: 8.0.into() },
            text_color: None, shadow: iced::Shadow::default(),
        })
    .into()
}

// ─────────────────────────────────────────────────────────────────────────────

struct MonitoringChart {
    cache: Cache,
    frame_slot: std::cell::Cell<i64>,
    title:  String,
    min:    f32,
    max:    f32,
    unit:   String,
    color:  RGBColor,
    pub points: VecDeque<(DateTime<Utc>, f32)>,
    window: Duration,
}

/// Quanto e' passato dall'ultimo campione, come frazione (0..=1) del passo
/// atteso fra due campioni.
///
/// Serve a far "camminare" il punto piu' recente invece di farlo saltare: con
/// ridisegno a ogni frame, il punto avanza piano verso la sua posizione
/// finale e la linea risulta continua.
///
/// Il valore resta quello misurato: cambia solo di quanto viene tracciato
/// sulla linea, e quando arriva il campione vero il punto e' gia' al posto
/// giusto. Fuori da qui nessun dato viene inventato.
#[cfg(test)]
pub fn avanzamento(
    adesso: DateTime<Utc>,
    ultimo: Option<DateTime<Utc>>,
    passo_ms: f32,
) -> f32 {
    match ultimo {
        None => 0.0,
        Some(t) => {
            let trascorso = (adesso - t).num_milliseconds().max(0) as f32;
            (trascorso / passo_ms).clamp(0.0, 1.0)
        }
    }
}

impl MonitoringChart {
    fn new(title: &str, min: f32, max: f32, unit: &str, color: RGBColor) -> Self {
        Self {
            cache: Cache::new(),
            frame_slot: std::cell::Cell::new(-1),
            title:  title.to_owned(),
            min, max,
            unit:   unit.to_owned(),
            color,
            points: VecDeque::new(),
            window: Duration::from_secs(180),
        }
    }

    fn push(&mut self, time: DateTime<Utc>, value: f32) {
        if !value.is_finite() { return; }
        self.cache.clear();
        let now_ms = time.timestamp_millis();
        self.points.push_front((time, value));
        while let Some(&(t, _)) = self.points.back() {
            let age = Duration::from_millis((now_ms - t.timestamp_millis()).unsigned_abs());
            if age > self.window { self.points.pop_back(); } else { break; }
        }
        self.recompute_range();
    }

    /// Quanto e' passato dall'ultimo campione, come frazione (0..=1) del
    /// passo atteso fra due campioni.
    ///
    /// Serve a far "camminare" il punto piu' recente invece di farlo
    /// saltare: con ridisegno a ogni frame, il punto avanza piano verso la
    /// sua posizione finale e la linea risulta continua. Il valore resta
    /// quello misurato: cambia solo di quanto viene tracciato sulla linea.
    /// Ricalcola `min`/`max` sui punti visibili, in modo **stabile**.
    ///
    /// Il difetto che risolve: prima la scala veniva ricalcolata a ogni
    /// campione, quindi quando arrivava un nuovo valore tutta la curva si
    /// riscalava di colpo. Non sembrava un salto nei dati, era la **scala**
    /// che si muoveva sotto gli occhi: da l'i' i gradini sulla piastra.
    ///
    /// La regola e' asimmetrica, e serve perche' i due errori non hanno la
    /// stessa gravita':
    ///
    /// - **si espande subito**: se un dato esce dalla scala, non lo si taglia
    ///   mai. Sbagliare la scala e' peggio che muoversi un attimo.
    /// - **si restringe piano**: il restringimento avviene per gradi, quindi
    ///   l'asse non "scatta" indietro mentre guardi la linea.
    ///
    /// Con `0.02` l'asse si stringe di ~2% per campione: lento abbastanza da
    /// non notarlo, veloce abbastanza da non restare largo per minuti.
    const RESTRINGIMENTO: f32 = 0.02;
    fn recompute_range(&mut self) {
        if self.points.len() < 2 {
            return;
        }
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for &(_, v) in self.points.iter() {
            if v.is_finite() {
                if v < lo { lo = v; }
                if v > hi { hi = v; }
            }
        }
        if lo > hi {
            return;
        }
        // Margine proporzionato all'ampiezza, con un minimo assoluto: su
        // una serie piatta un margine relativo darebbe una scala di zero.
        let ampiezza = (hi - lo).max(0.5);
        let margine = (ampiezza * 0.12).max(0.5);
        let (nuovo_min, nuovo_max) = (lo - margine, hi + margine);

        // Espansione immediata, restringimento graduale.
        self.min = if nuovo_min < self.min {
            nuovo_min
        } else {
            self.min + (nuovo_min - self.min) * Self::RESTRINGIMENTO
        };
        self.max = if nuovo_max > self.max {
            nuovo_max
        } else {
            self.max + (nuovo_max - self.max) * Self::RESTRINGIMENTO
        };
    }
}

#[derive(Default)]
struct ChartState {
    hover_x: Option<f32>,
    bounds:  iced::Rectangle,
    /// Indice del punto attualmente evidenziato.
    ///
    /// Serve per NON invalidare la cache a ogni movimento del mouse: il
    /// ridisegno con plotters e' l'operazione piu' costosa del grafico, e
    /// rifarla mentre il cursore resta sullo stesso punto e' lavoro sprecato.
    hover_idx: Option<usize>,
}

impl Chart<Message> for MonitoringChart {
    type State = ChartState;

    fn update(
        &self, state: &mut Self::State,
        event: iced::widget::canvas::Event,
        bounds: iced::Rectangle,
        cursor: iced::mouse::Cursor,
    ) -> (iced::event::Status, Option<Message>) {
        if let iced::widget::canvas::Event::Mouse(iced::mouse::Event::CursorLeft) = event {
            state.hover_x = None; self.cache.clear();
            return (iced::event::Status::Captured, None);
        }
        if let iced::mouse::Cursor::Available(p) = cursor {
            if cursor.position_in(bounds).is_some() {
                state.hover_x = Some(p.x);
                state.bounds  = bounds;
                // Invalida solo se il punto sotto il cursore e' cambiato
                // davvero. Muovere il mouse di qualche pixel dentro la stessa
                // fascia non cambia nulla da disegnare.
                let idx = hover_idx(state.hover_x, self.points.len(),
                                    bounds.width, bounds.x);
                if idx != state.hover_idx {
                    state.hover_idx = idx;
                    self.cache.clear();

                }
            } else {
                state.hover_x = None; self.cache.clear();
                if state.hover_idx.is_some() {
                    state.hover_idx = None;

                }
            }
        }
        (iced::event::Status::Ignored, None)
    }

    fn mouse_interaction(
        &self, state: &Self::State,
        _: iced::Rectangle, _: iced::mouse::Cursor,
    ) -> iced::mouse::Interaction {
        if state.hover_x.is_some() { iced::mouse::Interaction::Crosshair }
        else                       { iced::mouse::Interaction::Idle }
    }

    /// **Nessuna cache, di proposito.**
    ///
    /// Il default di plotters-iced (`R::draw`) ridisegna a ogni frame; io lo
    /// avevo sovrascritto con `R::draw_cache`, che riusa l'immagine del frame
    /// precedente e la ricalcola solo quando la cache viene svuotata. Per un
    /// grafico che scorre e' l'esatto contrario di quello che serve: tra un
    /// ridisegno e l'altro l'immagine resta **congelata**, e quando si
    /// aggiorna salta. E' quello l'origine dello scatto.
    ///
    /// Il prezzo e' che il grafico si ricalcola a ogni frame invece che a ogni
    /// campione: con 9 grafici e ~60 punti ciascuno resta trascurabile, e il
    /// guadagno di fluidita' e' immediato.
    #[inline]
    fn draw<R: Renderer, F: Fn(&mut Frame)>(
        &self, renderer: &R, size: Size, draw_fn: F,
    ) -> Geometry {
        refresh_frame(&self.cache, &self.frame_slot);
        R::draw_cache(renderer, &self.cache, size, draw_fn)
    }

    fn build_chart<'a, DB: DrawingBackend>(&'a self, state: &Self::State, mut b: ChartBuilder<'a, 'a, DB>) {
        // **L'asse X scorre sull'orologio, non sui campioni.**
        //
        // Prima era `oldest..newest`, cioe' ancorato ai dati. Ogni campione
        // spostava il bordo destro di mezzo secondo e *tutta* la linea
        // scivolava di colpo: la linea stava ferma e poi saltava. E' quello
        // lo scatto, e non dipende dalla frequenza dei campioni.
        //
        // Qui la finestra e' `adesso - finestra .. adesso`: scorre in
        // continuita' a ogni ridisegno, e i campioni ci cadono dentro alle
        // loro posizioni reali. Il rilevamento resta com'e'.
        let adesso = Utc::now() - chrono::Duration::milliseconds(PRESENTATION_DELAY_MS);
        let inizio = adesso - chrono::Duration::from_std(self.window)
            .unwrap_or_else(|_| chrono::Duration::seconds(180));

        let hover = hover_at(state.hover_x, &self.points, state.bounds, adesso, self.window);

        // Caption: titolo + valore corrente (live o hover)
        let caption = if let Some(i) = hover {
            format!("{}  ·  {:.2} {}", self.title, self.points[i].1, self.unit)
        } else if let Some(&(_, v)) = self.points.front() {
            format!("{}  ·  {:.2} {}", self.title, v, self.unit)
        } else {
            self.title.clone()
        };

        // Colore caption = colore del canale
        let caption_color = &self.color;

        let mut chart = match b
            .caption(caption, ("sans-serif", 14, caption_color))
            .x_label_area_size(18)
            .y_label_area_size(50)
            .margin_top(4)
            .margin_bottom(2)
            .margin_left(6)
            .margin_right(6)
            // **L'asse X scorre sull'orologio, non sui campioni.**
            //
            // Prima era ancorato a `oldest..newest`, cioe' ai dati: a ogni
            // campione il bordo destro saltava di mezzo secondo e *tutta* la
            // linea scivolava di colpo. Stava ferma, poi strappava. Non e'
            // un problema di frequenza, e' di ancoraggio.
            //
            // Qui la finestra e' `adesso - finestra .. adesso`: scorre in
            // continuita' e i campioni ci cadono dentro alle loro posizioni
            // reali. Il rilevamento non cambia.
            .build_cartesian_2d(inizio..adesso, self.min..self.max)
        {
            Ok(c) => c,
            Err(_) => return,
        };

        // Mesh grid minimalista
        let axis_labels = if chart.plotting_area().dim_in_pixel().1 < 85 { 3 } else { 4 };
        let _ = chart
            .configure_mesh()
            .bold_line_style(GRID_COLOR)
            .light_line_style(RGBAColor(20, 60, 120, 0.04))
            .axis_style(ShapeStyle::from(C_VOLTAGE.mix(0.08)).stroke_width(0))
            .y_labels(axis_labels)
            .x_labels(4)
            .y_label_offset(12)
            // Niente `Rotate90`: plotters-iced 0.11 non implementa la
            // rotazione del testo (l'intero blocco e' commentato nel suo
            // backend), quindi il transform era un no-op silenzioso e le
            // label restavano orizzontali, identiche a quelle dell'asse X.
            // Con lo spazio ridotto l'asse si legge meglio.
            .y_label_style(("sans-serif", 11).into_font().color(&LABEL_CLR))
            .y_label_formatter(&|y| format!("{:.1}{}", y, self.unit))
            .x_label_style(("sans-serif", 11).into_font().color(&LABEL_CLR))
            .x_label_formatter(&|x| x.time().format("%H:%M:%S").to_string())
            .draw();

        // **Il punto piu' recente scorre, non salta.**
        //
        // Il campione arriva ogni 500 ms, ma il ridisegno avviene a ogni
        // frame. Senza intervento, l'ultimo punto resta fermo nella sua
        // posizione per mezzo secondo e poi, al campione successivo, si
        // sposta di un gradino intero: la linea si vede "strappare".
        //
        // Qui l'ultimo punto viene traslato di una frazione del passo atteso,
        // proporzionata a quanto tempo e' passato dal suo arrivo. Con
        // ridisegno a ogni frame il punto cammina piano verso la sua
        // posizione finale e la linea risulta continua.
        //
        // Non e' un dato inventato: il *valore* resta quello misurato, cambia
        // solo di quanto e' stato tracciato sulla linea, e quando arriva il
        // campione vero il punto e' gia' nella posizione giusta.
        let punti = presentation_points(&self.points, adesso);

        // Area riempita
        let _ = chart.draw_series(
            AreaSeries::new(
                punti,
                self.min,
                self.color.mix(0.07),
            )
            .border_style(ShapeStyle::from(self.color).stroke_width(2)),
        );

        // Dot live sull'ultimo punto
        if let Some(&(t, v)) = presentation_points(&self.points, adesso).first() {
            let _ = chart.draw_series(std::iter::once(
                plotters::prelude::Circle::new((t, v), 3_i32, self.color.filled()),
            ));
        }

        // Dot interattivo hover
        if let Some(i) = hover {
            let _ = chart.draw_series(std::iter::once(
                plotters::prelude::Circle::new(
                    (self.points[i].0, self.points[i].1),
                    5_i32,
                    self.color.mix(0.9).filled(),
                ),
            ));
        }
    }
}

fn hover_idx(x: Option<f32>, len: usize, width: f32, off: f32) -> Option<usize> {
    if len == 0 { return None; }
    x.map(|xp| {
        let idx = len.saturating_sub(((xp - off) / width * len as f32).round() as usize);
        idx.min(len - 1)
    })
}

// ── Overlap Chart: TEC + CPU + Dew Point ────────────────────────────────────

pub struct OverlapChart {
    droplets_cache: Cache,
    cache: Cache,
    frame_slot: std::cell::Cell<i64>,
    tec:    VecDeque<(DateTime<Utc>, f32)>,
    dew:    VecDeque<(DateTime<Utc>, f32)>,
    cpu:    VecDeque<(DateTime<Utc>, f32)>,
    window: Duration,
    /// Finestra verticale corrente: `(min, max)`.
    ///
    /// Vive **qui** e non dentro `view()` perche' `view()` e' chiamata a
    /// ogni frame a 30 Hz, mentre i dati arrivano a 2 Hz. Se la scala si
    /// ricalcolasse dai punti visibili a ogni frame si muoverebbe 14 volte
    /// al secondo per aggiornamento, e ogni tanto la finestra si
    /// espanderebbe di mezzo grado. Conservandola fra un frame e l'altro
    /// la scala resta ferma, e si muove solo quando un dato esce dai
    /// limiti — che e' una volta ogni mezzo secondo, non trenta.
    ///
    /// Si azzera quando la finestra temporale scorre oltre i dati
    /// visibili, altrimenti resterebbe stretta per sempre e i valori
    /// futuri non entrerebbero piu'.
    ///
    /// In un `Cell` perche' `view()` prende `&self`: conservare la
    /// finestra fra un frame e l'altro e' scrivere dentro la UI, e senza
    /// `Cell` servirebbe un `&mut self` che iced non concede. `Cell` e'
    /// l'unico posto dove si puo' scrivere con riferimento immutabile,
    /// e qui la semantica e' esattamente "una cache interna".
    y_finestra: std::cell::Cell<Option<(f32, f32)>>,
}

impl OverlapChart {
    pub fn new() -> Self {
        Self {
            droplets_cache: Cache::new(),
            cache: Cache::new(),
            frame_slot: std::cell::Cell::new(-1),
            tec:    VecDeque::new(),
            dew:    VecDeque::new(),
            cpu:    VecDeque::new(),
            window: Duration::from_secs(180),
            y_finestra: std::cell::Cell::new(None),
        }
    }

    pub fn push(&mut self, t: DateTime<Utc>, tec_temp: f32, dew_temp: f32) {
        if !tec_temp.is_finite() || !dew_temp.is_finite() { return; }
        self.cache.clear();
        let ms = t.timestamp_millis();
        self.tec.push_front((t, tec_temp));
        self.dew.push_front((t, dew_temp));
        for q in [&mut self.tec, &mut self.dew, &mut self.cpu] {
            while let Some(&(bt, _)) = q.back() {
                if Duration::from_millis((ms - bt.timestamp_millis()).unsigned_abs()) > self.window {
                    q.pop_back();
                } else { break; }
            }
        }
        self.recompute_range();
    }

    fn recompute_range(&self) {
        let values = self.tec.iter().chain(self.dew.iter()).chain(self.cpu.iter()).map(|p|p.1);
        let (lo,hi) = values.fold((f32::MAX,f32::MIN), |(lo,hi),v| (lo.min(v),hi.max(v)));
        if lo <= hi { self.y_finestra.set(Some(finestra_verticale(self.y_finestra.get(),lo,hi,3.0))); }
    }
    pub fn push_cpu(&mut self, cpu_temp: f32) {
        if !cpu_temp.is_finite() { return; }
        self.cache.clear();
        if let Some(&(t, _)) = self.tec.front() {
            self.cpu.push_front((t, cpu_temp));
            while self.cpu.len() > self.tec.len() + 2 { self.cpu.pop_back(); }
            self.recompute_range();
        }
    }
}

fn overlap_card(chart: &OverlapChart) -> iced::Element<'_, Message> {
    let label = |name:&str, sample:Option<&(DateTime<Utc>,f32)>| match sample {
        Some((_,value)) => format!("● {name}  {value:.1}°"),
        None => format!("● {name}  —"),
    };
    let mut legend = Row::new().spacing(14).padding([3, 12]).height(36).align_y(iced::Alignment::Center)
        .push(iced::widget::Text::new(label("TEC",chart.tec.front())).size(11).color(iced::Color::from_rgb8(0,210,255)))
        .push(iced::widget::Text::new(label("CPU",chart.cpu.front())).size(11).color(iced::Color::from_rgb8(255,190,110)))
        .push(iced::widget::Text::new(label("Rugiada",chart.dew.front())).size(11).color(iced::Color::from_rgb8(100,170,255)));
    if crate::condensation_badge::active(chart.tec.front().copied(),chart.dew.front().copied(),Utc::now()) {
        legend=legend.push(iced::widget::Space::with_width(Length::Fill)).push(
            iced::widget::tooltip(
                iced::widget::canvas(crate::condensation_badge::Droplets {cache:&chart.droplets_cache})
                    .width(60).height(24),
                iced::widget::Text::new("Rischio condensa: TEC sotto il punto di rugiada").size(12),
                iced::widget::tooltip::Position::Bottom,
            )
        );
    }
    Container::new(Column::new().push(legend).push(
        ChartWidget::new(chart).width(Length::Fill).height(Length::Fill)))
    .width(Length::Fill)
    .height(Length::FillPortion(2)) // doppia altezza — grafico più importante
    .padding(iced::Padding { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 })
    .style(|_: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(iced::Color { r: 0.007, g: 0.055, b: 0.042, a: 0.86 })),
            border: iced::Border { color: iced::Color { r: 0.0, g: 0.7, b: 0.45, a: 0.25 }, width: 1.5, radius: 10.0.into() },
            text_color: None, shadow: crate::palette::glow(iced::Color::from_rgb8(0,180,160),0.12),
        })
    .into()
}


#[derive(Default)]
pub struct OverlapState {
    hover_x: Option<f32>,
    bounds:  iced::Rectangle,
    /// Vedi `ChartState::hover_idx`: stessa ragione, cache da invalidare solo
    /// al cambio del punto evidenziato.
    hover_idx: Option<usize>,
}

impl Chart<Message> for OverlapChart {
    type State = OverlapState;

    fn update(
        &self, state: &mut Self::State,
        event: iced::widget::canvas::Event,
        bounds: iced::Rectangle,
        cursor: iced::mouse::Cursor,
    ) -> (iced::event::Status, Option<Message>) {
        if let iced::widget::canvas::Event::Mouse(iced::mouse::Event::CursorLeft) = event {
            state.hover_x = None; self.cache.clear(); return (iced::event::Status::Captured, None);
        }
        if let iced::mouse::Cursor::Available(p) = cursor {
            if cursor.position_in(bounds).is_some() {
                // Invalida la cache SOLO quando cambia il **punto** sotto il
                // cursore, non a ogni pixel mosso. Il mouse puo' generare
                // centinaia di eventi al secondo e il re-render di plotters
                // e' l'operazione piu' costosa del grafico: senza questo
                // confronto, passare sopra una curva la fa rifare di continuo.
                state.hover_x = Some(p.x);
                state.bounds = bounds;
                let idx = hover_idx(state.hover_x, self.tec.len(),
                                    bounds.width, bounds.x);
                if idx != state.hover_idx {
                    state.hover_idx = idx;
                    self.cache.clear();

                }
            } else if state.hover_x.is_some() {
                state.hover_x = None; self.cache.clear();
                state.bounds = bounds;
                if state.hover_idx.take().is_some() {

                }
            }
        }
        (iced::event::Status::Ignored, None)
    }

    fn mouse_interaction(&self, state: &Self::State, _: iced::Rectangle, _: iced::mouse::Cursor)
        -> iced::mouse::Interaction {
        if state.hover_x.is_some() { iced::mouse::Interaction::Crosshair }
        else { iced::mouse::Interaction::Idle }
    }

    /// Senza cache, come `MonitoringChart`: l'immagine congelata fra un
    /// ridisegno e l'altro e' cio' che rende visibile lo scatto.
    #[inline]
    fn draw<R: Renderer, F: Fn(&mut Frame)>(&self, r: &R, size: Size, f: F) -> Geometry {
        refresh_frame(&self.cache, &self.frame_slot);
        R::draw_cache(r, &self.cache, size, f)
    }

    fn build_chart<'a, DB: DrawingBackend>(&'a self, _state: &Self::State, mut b: ChartBuilder<'a, 'a, DB>) {
        if self.tec.is_empty() { return; }

        // **Stessa correzione di `MonitoringChart`: l'asse X scorre
        // sull'orologio, non sui campioni.**
        //
        // Era ancorato a `oldest..newest`: a ogni campione il bordo destro
        // saltava di mezzo secondo e tutte e tre le serie scivolavano di
        // colpo. E' la stessa causa che, sui grafici singoli, si vedeva come
        // scatto.
        let adesso = Utc::now() - chrono::Duration::milliseconds(PRESENTATION_DELAY_MS);
        let inizio = adesso - chrono::Duration::from_std(self.window)
            .unwrap_or_else(|_| chrono::Duration::seconds(180));

        // **Il punto piu' recente scorre, non salta** (come in
        // `MonitoringChart`). Il campione arriva ogni 500 ms ma il ridisegno
        // e' a ogni frame: senza intervento l'ultimo punto resta fermo per
        // mezzo secondo e poi si sposta di un gradino intero, e la linea si
        // vede strappare. Il valore resta quello misurato: cambia solo di
        // quanto e' tracciato sulla linea.
        let tec_pts = presentation_points(&self.tec, adesso);
        let dew_pts = presentation_points(&self.dew, adesso);
        let cpu_pts = presentation_points(&self.cpu, adesso);
        let (y_min, y_max) = self.y_finestra.get().unwrap_or((0.0, 50.0));
        let mut chart = match b
            .caption(format!("TEC / CPU / Rugiada   |   Margine {:+.1} C", self.tec.front().map(|p|p.1).unwrap_or(0.0) - self.dew.front().map(|p|p.1).unwrap_or(0.0)),
                ("sans-serif", 13, &RGBColor(0, 200, 140)))
            .x_label_area_size(18)
            .y_label_area_size(50)
            .margin_top(4).margin_bottom(2).margin_left(6).margin_right(4)
            .build_cartesian_2d(inizio..adesso, y_min..y_max)
        {
            Ok(c) => c,
            Err(_) => return,
        };

        let axis_labels = if chart.plotting_area().dim_in_pixel().1 < 100 { 3 } else { 5 };
        let _ = chart.configure_mesh()
            .bold_line_style(RGBAColor(0, 180, 120, 0.10))
            .light_line_style(RGBAColor(0, 100, 70, 0.04))
            .axis_style(ShapeStyle::from(LABEL_CLR).stroke_width(0))
            .y_labels(axis_labels).x_labels(4)
            .y_label_style(("sans-serif", 10).into_font().color(&LABEL_CLR)
                )
            .y_label_formatter(&|y| format!("{:.1}°C", y))
            .x_label_style(("sans-serif", 9).into_font().color(&LABEL_CLR))
            .x_label_formatter(&|x| x.time().format("%H:%M:%S").to_string())
            .draw();

        // ── Zona fra piastra e rugiada ──────────────────────────────
        //
        // Prima era **un rettangolo per ogni campione**: una fila di barre
        // verticali separate, con buchi in mezzo, che da lontano leggeva come
        // una scala a pettini invece di una zona continua. Era anche piu'
        // costoso: ~360 rettangoli a ogni ridisegno.
        //
        // Ora e' **un solo poligono** per tratto: si segue la rugiada e si
        // torna indietro lungo la piastra, quindi chiude una superficie
        // continua. Stesso significato, molto meno lavoro.
        //
        // Il colore segue la posizione della piastra rispetto alla rugiada:
        // verde sopra (sicuro), rosso sotto (rischio condensa).
        if self.tec.len() == self.dew.len() && self.tec.len() > 1 {
            // Terne (tempo, tec, dew) gia' allineate: il punto piu' recente
            // segue l'interpolazione come le curve, altrimenti la zona si
            // stacca dalle linee che dovrebbe riempiere.
            let punti: Vec<(DateTime<Utc>, f32, f32)> = tec_pts.iter().zip(dew_pts.iter())
                .map(|(&(ts, tec), &(_, dew))| (ts, tec, dew)).collect();
            // Un poligono per tratto di colore: quando la piastra attraversa
            // la rugiada il tratto si chiude e ne comincia un altro.
            let mut segmenti: Vec<(bool, Vec<(DateTime<Utc>, f32, f32)>)> = Vec::new();
            let mut tratto: Vec<(DateTime<Utc>, f32, f32)> = Vec::new();
            let mut segno_corrente: Option<bool> = None;

            for (i, &(ts, tec, dew)) in punti.iter().enumerate() {
                let sicuro = tec >= dew;
                if segno_corrente != Some(sicuro) {
                    if let Some(s) = segno_corrente {
                        // Il punto di confine entra in entrambi i tratti,
                        // altrimenti resta una fessura tra verde e rosso.
                        tratto.push(punti[i]);
                        segmenti.push((s, std::mem::take(&mut tratto)));
                    }
                    segno_corrente = Some(sicuro);
                    if i > 0 {
                        tratto.push(punti[i - 1]);
                    }
                }
                tratto.push((ts, tec, dew));
            }
            if let Some(s) = segno_corrente {
                if !tratto.is_empty() {
                    segmenti.push((s, tratto));
                }
            }

            for (sicuro, seg) in segmenti {
                if seg.len() < 2 {
                    continue;
                }
                // Avanti lungo la rugiada, indietro lungo la piastra.
                let poligono: Vec<(DateTime<Utc>, f32)> = seg.iter()
                    .map(|&(ts, _, dew)| (ts, dew))
                    .chain(seg.iter().rev().map(|&(ts, tec, _)| (ts, tec)))
                    .collect();
                let _ = chart.draw_series(std::iter::once(
                    plotters::element::Polygon::new(
                        poligono,
                        if sicuro {
                            RGBAColor(0, 220, 110, 0.10).filled()
                        } else {
                            RGBAColor(255, 60, 90, 0.20).filled()
                        },
                    )));
            }
        }

        // Linea TEC
        let _ = chart.draw_series(
            AreaSeries::new(tec_pts.iter().copied(), y_min,
                C_TEC_TEMP.mix(0.06))
                .border_style(ShapeStyle::from(C_TEC_TEMP).stroke_width(2)));

        // Linea Dew Point — **tratteggiata e spessa**.
        //
        // Con tratteggio e' inequivocabile: e' una soglia, non una misura che
        // insegue. Continua a piu' spessore della piastra perche' e' il
        // riferimento contro cui si giudica il rischio condensa.
        let _ = chart.draw_series(
            plotters::series::LineSeries::new(
                dew_pts.iter().copied(),
                ShapeStyle::from(C_DEW_POINT).stroke_width(3)));

        // Linea CPU — **arancio chiaro e sottile**.
        //
        // Prima era arancione pieno come la rugiada, quindi le due curve si
        // confondevano. Ora e' piu' chiara e piu' sottile, e non pretende di
        // essere una soglia: e' un contesto, non un pericolo.
        //
        // **Nessun filtro sui punti.** C'era un `i % 2 == 0` che buttava meta'
        // dei campioni: con la CPU letta ogni 2 s si passava da 90 punti a 45
        // in tre minuti, e la linea sembrava piatta perche' era campionata a
        // meta'. I punti sono gia' radi di loro (1 ogni 2 s contro 1 ogni
        // 0,5 s delle altre curve), quindi il filtro non alleggeriva niente:
        // toglieva solo informazione.
        if !self.cpu.is_empty() {
            let _ = chart.draw_series(
                plotters::series::LineSeries::new(cpu_pts,
                    ShapeStyle::from(C_CPU_LINE).stroke_width(1)));
        }

        // Dot live TEC
        if let Some(&(t, v)) = tec_pts.first() {
            let _ = chart.draw_series(std::iter::once(
                plotters::prelude::Circle::new((t, v), 3_i32, C_TEC_TEMP.filled())));
        }


    }
}

/// Decisione di visibilita' del banner cryogenic, pura e testabile.
///
/// Sotto i 20 °C con una misura reale: visibile. Sopra, senza misura, o con
/// misura non finita: nascosto. Estratta dal metodo per vincolarla con i
/// test invece di fidarsi della lettura del codice.
fn cryo_visibile_a(tec: f32, ha_dato: bool) -> bool {
    ha_dato && tec.is_finite() && tec < 20.0
}

#[cfg(test)]
mod test_cryo_visibilita {
    use super::cryo_visibile_a;

    #[test]
    fn sotto_soglia_con_misura_e_visibile() {
        assert!(cryo_visibile_a(19.9, true));
        assert!(cryo_visibile_a(5.0, true));
    }

    #[test]
    fn sopra_soglia_e_nascosto() {
        assert!(!cryo_visibile_a(20.0, true));
        assert!(!cryo_visibile_a(21.8, true));
        assert!(!cryo_visibile_a(41.5, true));
    }

    #[test]
    fn senza_misura_e_sempre_nascosto() {
        // Anche con un valore sotto soglia: nessun dato non e' freddo.
        assert!(!cryo_visibile_a(0.0, false));
        assert!(!cryo_visibile_a(15.0, false));
        assert!(!cryo_visibile_a(f32::NAN, true));
    }
}


#[cfg(test)]
mod scala_stabile_tests {
    use super::*;

    fn grafico() -> MonitoringChart {
        MonitoringChart::new("T", 0.0, 10.0, "°C", C_TEC_TEMP)
    }

    /// La scala non deve ** restringersi di colpo**: e' la causa dei
    /// gradini visibili, perche' l'asse si muoveva a ogni campione.
    #[test]
    fn la_scala_non_si_restringe_di_colpo() {
        let mut g = grafico();
        // Si popola con un range ampio, poi si stringe di colpo.
        for i in 0..20 {
            let v = if i < 10 { 0.0 } else { 100.0 };
            g.push(Utc::now(), v);
        }
        let prima = (g.min, g.max);
        for _ in 0..5 {
            g.push(Utc::now(), 50.0);
        }
        let dopo = (g.min, g.max);
        // Il restringimento per campione deve essere piccolo, non totale.
        let restringimento = (prima.0 - dopo.0).abs();
        assert!(
            restringimento < 10.0,
            "la scala si e' ristretta di {} in 5 campioni: torna a scattare",
            restringimento
        );
    }

    /// La scala non deve MAI tagliare un dato: se un valore esce, si
    /// espande subito. Sbagliare la scala e' peggio che muoversi.
    #[test]
    fn la_scala_espande_subito_e_non_taglia() {
        let mut g = grafico();
        for _ in 0..10 {
            g.push(Utc::now(), 0.0);
        }
        // Valore improvviso molto alto: deve stare dentro la scala subito.
        g.push(Utc::now(), 80.0);
        assert!(
            80.0 <= g.max,
            "valore fuori scala: {} > max {}", 80.0, g.max
        );
    }

    /// Una serie tutta uguale non deve dare una scala di altezza zero
    /// (divisione per zero, o una linea che sparisce).
    #[test]
    fn la_scala_non_si_chiude_su_se_stessa() {
        let mut g = grafico();
        for _ in 0..10 {
            g.push(Utc::now(), 5.0);
        }
        assert!(
            g.max - g.min > 0.1,
            "scala troppo stretta su serie piatta: {}..{}", g.min, g.max
        );
    }
}

/// Finestra verticale **stabile**: la scala Y si muove solo quando il dato
/// esce dai limiti, non a ogni campione.
///
/// **Perché.** La scala si ricalcolava dal minimo e massimo di tutti i
/// punti visibili, quindi ogni nuovo campione la spostava: tutta la linea
/// si moveva insieme, e l'occhio lo legge come uno scatto. Con la fluidita'
/// del ridisegno a 30 Hz sistemata, questo era l'ultima cosa che si vedeva.
///
/// **Come funziona.** I limiti si espandono per includere il valore nuovo
/// solo se esce da quelli attuali, e con un margine: cosi' un dato che
/// oscilla dentro la finestra non muove niente, e quando la finestra
/// cresce lo fa di un tratto visibile invece che di mezzo grado per volta.
///
/// L'espansione e' irreversibile dentro una sessione di disegno, e va
/// bene: restringere la scala mentre la linea e' tracciata la farebbe
/// saltare indietro, che e' peggio che allargarla.
pub fn finestra_verticale(
    attuale: Option<(f32, f32)>,
    nuovo_min: f32,
    nuovo_max: f32,
    margine: f32,
) -> (f32, f32) {
    match attuale {
        None => (nuovo_min - margine, nuovo_max + margine),
        Some((lo, hi)) => {
            // **Il margine si applica solo quando si esce davvero dai
            // limiti.** Applicarlo sempre sposta la finestra a ogni
            // campione: un valore interno come 12 °C in una finestra
            // 10..20 con margine 3 scenderebbe a 9 e allargherebbe tutto,
            // che e' esattamente lo scatto che si voleva eliminare.
            let mut nuovo_lo = lo;
            let mut nuovo_hi = hi;
            if nuovo_min < lo {
                nuovo_lo = nuovo_min - margine;
            }
            if nuovo_max > hi {
                nuovo_hi = nuovo_max + margine;
            }
            // Difesa contro un `NaN` in ingresso: un solo campione sporco
            // renderebbe la scala non finita e il grafico non si disegnerebbe
            // piu'. Meglio ignorarlo che lasciare la finestra rovinata.
            if nuovo_lo.is_finite() && nuovo_hi.is_finite() && nuovo_hi > nuovo_lo {
                (nuovo_lo, nuovo_hi)
            } else {
                (lo, hi)
            }
        }
    }
}

#[cfg(test)]
mod test_scala_stabile {
    use super::*;

    /// **Il caso reale: dati che oscillano dentro la finestra.**
    ///
    /// La scala si ricalcolava a ogni campione, quindi con una temperatura
    /// che oscilla di mezzo grado la finestra si spostava di mezzo grado
    /// e tutta la linea si moveva. E' lo scatto che l'utente vede.
    #[test]
    fn un_dato_dentro_i_limiti_non_muove_la_scala() {
        let (lo, hi) = (10.0, 20.0);
        for v in [12.0, 15.5, 18.2, 11.1, 19.9, 14.0] {
            let (a, b) = finestra_verticale(Some((lo, hi)), v, v, 3.0);
            assert_eq!(
                (a, b),
                (lo, hi),
                "il valore {v} era dentro la finestra e l'ha spostata a {a}..{b}"
            );
        }
    }

    /// Un valore **fuori** in alto allarga in su, e solo in su.
    #[test]
    fn un_valore_alto_allarga_solo_in_su() {
        let (lo, hi) = (10.0, 20.0);
        let (a, b) = finestra_verticale(Some((lo, hi)), 10.0, 25.0, 3.0);
        assert_eq!(a, lo, "il limite basso non doveva muoversi");
        assert_eq!(b, 28.0, "il limite alto doveva salire di 3, il margine");
    }

    /// Simmetrico per il basso: un valore freddo molto basso non deve
    /// spostare anche il tetto, altrimenti la finestra si gonfia tutta da
    /// una parte sola.
    #[test]
    fn un_valore_basso_allarga_solo_in_giu() {
        let (lo, hi) = (10.0, 20.0);
        let (a, b) = finestra_verticale(Some((lo, hi)), 2.0, 20.0, 3.0);
        assert_eq!(a, -1.0);
        assert_eq!(b, hi, "il limite alto non doveva muoversi");
    }

    /// La finestra non si restringe mai. Stringerla mentre la linea e'
    /// tracciata la farebbe saltare indietro, che si legge peggio che
    /// allargarla.
    #[test]
    fn la_finestra_non_si_restringe_mai() {
        let (lo, hi) = (10.0, 30.0);
        let (a, b) = finestra_verticale(Some((lo, hi)), 18.0, 19.0, 3.0);
        assert_eq!((a, b), (lo, hi), "la finestra si e' ristretta");
    }

    /// Una serie che si raffredda stabilmente deve comunque **entrare**,
    /// altrimenti il grafico taglia via la linea e non dice piu' niente.
    #[test]
    fn una_serie_in_raffreddamento_resta_visibile() {
        let (lo, hi) = (10.0, 20.0);
        let mut finestra = Some((lo, hi));
        for v in [9.0, 7.5, 6.0, 4.0, 2.0] {
            finestra = Some(finestra_verticale(finestra, v, v, 3.0));
            let (a, b) = finestra.unwrap();
            assert!(
                a <= v && v <= b,
                "il valore {v} e' uscito dalla finestra {a}..{b}: non si vede piu'"
            );
        }
    }

    /// Un `NaN` non deve rovinare la scala: un solo campione sporco
    /// renderebbe i limiti non finiti e il grafico non si disegnerebbe piu'.
    #[test]
    fn un_nan_non_rovina_la_scala() {
        let (lo, hi) = (10.0, 20.0);
        let (a, b) = finestra_verticale(Some((lo, hi)), f32::NAN, f32::NAN, 3.0);
        assert!((a, b).0.is_finite() && (a, b).1.is_finite());
        assert!((a, b).0 < (a, b).1, "finestra invertita: {a}..{b}");
    }

    /// Un singolo campione non puo' produrre una finestra di altezza zero:
    /// un singolo valore con margine zero darebbe lo=hi e il grafico non
    /// avrebbe nulla da disegnare dentro.
    #[test]
    fn un_solo_campione_da_una_finestra_utilizzabile() {
        let (a, b) = finestra_verticale(None, 15.0, 15.0, 3.0);
        assert!(b > a, "finestra piatta: {a}..{b}");
        assert!(a.is_finite() && b.is_finite());
    }
}
