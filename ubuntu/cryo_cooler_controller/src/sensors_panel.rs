//! Pannello sensori di sistema con selezione sorgente HWiNFO64 / AIDA64.
//!
//! Bug fix rispetto alla versione precedente:
//! - Tensioni non raggruppate per categoria (nessuna categoria le conteneva)
//! - Ventole: fallback su nome lettura se sensor_type non è Fan
//! - Doppia reference && eliminata: tutto usa sensor_grid_ref(&[&SensorReading])
//! - Guard su lista vuota in sensor_grid_ref
//! - not_available_msg distingue tra sorgente offline e tipo sensore assente

use iced::{
    alignment,
    widget::{Column, Container, Row, Text},
    Element, Length,
};

use crate::{
    hwinfo::{SensorReading, SensorSource, SensorType, SourceStatus},
    palette,
    Message,
};

#[derive(Debug, Clone, PartialEq)]
pub enum SensorTab {
    Temperatura,
    Ventole,
    Tensioni,
    Carichi,
    Tutti,
}

impl std::fmt::Display for SensorTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SensorTab::Temperatura => write!(f, "Temperatura"),
            SensorTab::Ventole     => write!(f, "Ventole"),
            SensorTab::Tensioni    => write!(f, "Tensioni"),
            SensorTab::Carichi     => write!(f, "Carichi"),
            SensorTab::Tutti       => write!(f, "Tutti"),
        }
    }
}

/// Quale sorgente usare, dati gli stati e la situazione attuale.
///
/// Regola unica, usata sia all'avvio sia a ogni refresh: se la sorgente
/// corrente non dà nessun sensore e l'altra è disponibile, si passa
/// all'altra. Prima la scelta avveniva solo all'avvio e restava inchiodata:
/// con HWiNFO selezionato ma senza memoria condivisa e AIDA64 attivo, tutto
/// restava N/D — CPU esterna, COP, OC Score e modalità automatica — senza
/// che niente lo dicesse.
fn scegli_sorgente(
    hw_ok: bool,
    ai_ok: bool,
    corrente: SensorSource,
    n_sensori: usize,
) -> SensorSource {
    use SensorSource as S;
    if n_sensori > 0 {
        return corrente;
    }
    match corrente {
        S::HWiNFO if ai_ok => S::AIDA64,
        S::AIDA64 if hw_ok => S::HWiNFO,
        _ => corrente,
    }
}

pub struct SensorsPanel {
    pub sensors:       Vec<SensorReading>,
    pub active_tab:    SensorTab,
    pub active_source: SensorSource,
    pub status:        SourceStatus,
    refresh_ticks:     u32,
    page:              usize,
}

impl SensorsPanel {
    pub fn new() -> Self {
        let status = crate::hwinfo::check_sources();
        #[cfg(target_os = "linux")]
        let prima = SensorSource::HWMON;
        #[cfg(not(target_os = "linux"))]
        let prima = if status.hwinfo_available {
            SensorSource::HWiNFO
        } else {
            SensorSource::AIDA64
        };
        let sensors = crate::hwinfo::read_all_sensors(&prima);
        // Se la prima scelta e' vuota e l'altra ha dati, si passa subito:
        // partire ciechi costava un N/D permanente fino al primo refresh.
        let source = scegli_sorgente(
            status.hwinfo_available,
            status.aida64_available,
            prima,
            sensors.len(),
        );
        let sensors = crate::hwinfo::read_all_sensors(&source);
        SensorsPanel {
            sensors,
            active_tab:    SensorTab::Temperatura,
            active_source: source,
            status,
            refresh_ticks: 0,
            page: 0,
        }
    }

    pub fn tick(&mut self) {
        self.refresh_ticks = self.refresh_ticks.wrapping_add(1);
        let interval = if cfg!(target_os = "linux") { 4 } else { 20 };
        if self.refresh_ticks % interval == 0 {
            self.status  = crate::hwinfo::check_sources();
            self.sensors = crate::hwinfo::read_all_sensors(&self.active_source);
            // La sorgente attiva puo' essersi svuotata (programma chiuso,
            // memoria condivisa spenta): se l'altra ha dati si passa da
            // soli, invece di restare inchiodati su zero sensori.
            let nuova = scegli_sorgente(
                self.status.hwinfo_available,
                self.status.aida64_available,
                self.active_source.clone(),
                self.sensors.len(),
            );
            if nuova != self.active_source {
                self.active_source = nuova;
                self.sensors = crate::hwinfo::read_all_sensors(&self.active_source);
            }
        }
    }

    pub fn set_tab(&mut self, tab: SensorTab) {
        if self.active_tab != tab {
            self.page = 0;
            self.active_tab = tab;
        }
    }

    pub fn set_page(&mut self, page: usize) {
        self.page = page;
    }

    pub fn set_source(&mut self, source: SensorSource) {
        self.active_source = source;
        self.page = 0;
        self.sensors = crate::hwinfo::read_all_sensors(&self.active_source);
    }

    #[allow(dead_code)]
    pub fn hwinfo_ok(&self) -> bool { self.status.hwinfo_available }
    #[allow(dead_code)]
    pub fn aida64_ok(&self) -> bool { self.status.aida64_available }

    // ── View principale ───────────────────────────────────────────────────────

    pub fn view(&self) -> Element<'_, Message> {
        #[cfg(target_os = "linux")]
        let src_row = Row::new().padding([3u16, 8u16])
            .push(status_text(&self.active_source, &self.status, self.sensors.len()));
        #[cfg(not(target_os = "linux"))]
        let src_row = Row::new()
            .padding([3u16, 8u16])
            .spacing(5)
            .align_y(iced::Alignment::Center)
            .push(source_btn("HWiNFO64", self.active_source == SensorSource::HWiNFO,
                self.status.hwinfo_available, Message::SetSensorSource(SensorSource::HWiNFO)))
            .push(source_btn("AIDA64", self.active_source == SensorSource::AIDA64,
                self.status.aida64_available, Message::SetSensorSource(SensorSource::AIDA64)))
            .push(iced::widget::Space::with_width(8.0))
            .push(status_text(&self.active_source, &self.status, self.sensors.len()));

        let tab_row = Row::new()
            .width(Length::Fill)
            .spacing(4)
            .push(tab_btn("Temp.",    self.active_tab == SensorTab::Temperatura, Message::SensorTab(SensorTab::Temperatura)))
            .push(tab_btn("Ventole",  self.active_tab == SensorTab::Ventole,     Message::SensorTab(SensorTab::Ventole)))
            .push(tab_btn("Tensioni", self.active_tab == SensorTab::Tensioni,    Message::SensorTab(SensorTab::Tensioni)))
            .push(tab_btn("Carichi",  self.active_tab == SensorTab::Carichi,     Message::SensorTab(SensorTab::Carichi)))
            .push(tab_btn("Tutti",    self.active_tab == SensorTab::Tutti,       Message::SensorTab(SensorTab::Tutti)));

        // Header su DUE righe, non una.
        //
        // Prima era una `Row` con le tab dopo uno `Space::with_width(Fill)`.
        // Il messaggio di stato e' lungo ("Nessuna sorgente attiva — avviare
        // HWiNFO64 o AIDA64"), quindi su finestre strette le tab venivano
        // schiacciate oltre il bordo destro e risultavano tagliate a meta',
        // con "Tensioni" spezzato su due righe. Impilando, ogni riga sta
        // nel suo spazio e non c'e' piu' overflow orizzontale.
        let header = Column::new()
            .spacing(3)
            .push(src_row)
            .push(tab_row);

        let body = iced::widget::responsive(|size| self.view_tab(size));

        Column::new()
            .push(header)
            .push(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_tab(&self, size: iced::Size) -> Element<'_, Message> {
        let sections = self.sections();
        if sections.is_empty() {
            let detail = match self.active_tab {
                SensorTab::Temperatura => "Nessuna temperatura rilevata.",
                SensorTab::Ventole => "Nessuna ventola rilevata.",
                SensorTab::Tensioni => "Nessuna tensione o corrente rilevata.",
                SensorTab::Carichi => "Nessun carico, potenza o frequenza rilevato.",
                SensorTab::Tutti => "Nessun sensore disponibile.",
            };
            return empty_msg(detail, &self.active_source, &self.status);
        }

        let columns = ((size.width - 8.0) / 155.0).floor().clamp(1.0, 5.0) as usize;
        let pages = sensor_pages(sections, columns, (size.height - 8.0).max(0.0));
        let current = self.page.min(pages.len() - 1);
        let mut body = Column::new().width(Length::Fill);
        for line in &pages[current] {
            body = body.push(match line {
                SensorLine::Heading(label) => section_label(*label),
                SensorLine::Readings(items) => {
                    let mut row = Row::new().width(Length::Fill).height(ROW_HEIGHT).spacing(4);
                    for &sensor in items {
                        row = row.push(sensor_card(sensor));
                    }
                    for _ in items.len()..columns {
                        row = row.push(iced::widget::Space::with_width(Length::Fill));
                    }
                    row.into()
                }
            });
        }

        let mut panel = Column::new().padding(4).width(Length::Fill).height(Length::Fill)
            .push(Container::new(body).width(Length::Fill).height(Length::Fill).clip(true));
        if pages.len() > 1 {
            let btn = |label, target: Option<usize>| iced::widget::button(Text::new(label).size(12))
                .padding([3, 8])
                .style(crate::btn::secondary)
                .on_press_maybe(target.map(Message::SensorPage));
            panel = panel.push(
                Row::new().height(FOOTER_HEIGHT).spacing(8).align_y(iced::Alignment::Center)
                    .push(btn("Indietro", current.checked_sub(1)))
                    .push(Text::new(format!("{} / {}", current + 1, pages.len()))
                        .size(12).color(palette::TEXT_BRIGHT))
                    .push(btn("Avanti", (current + 1 < pages.len()).then_some(current + 1))),
            );
        }
        panel.into()
    }

    fn sections(&self) -> Vec<SensorSection<'_>> {
        let groups: &[(SensorType, &str)] = &[
            (SensorType::Temperature, "Temperature"),
            (SensorType::Fan, "Ventole"),
            (SensorType::Voltage, "Tensioni"),
            (SensorType::Current, "Correnti"),
            (SensorType::Power, "Potenze"),
            (SensorType::Load, "Carichi"),
            (SensorType::Clock, "Frequenze"),
            (SensorType::Other, "Altro"),
        ];
        let mut sections: Vec<SensorSection<'_>> = match self.active_tab {
            SensorTab::Temperatura => ["CPU", "GPU", "Scheda Madre", "Storage", "Altro"].into_iter()
                .map(|label| (label, self.sensors.iter().filter(|s| s.sensor_type == SensorType::Temperature
                    && !auxiliary_sensor(s) && s.category() == label).collect()))
                .collect(),
            SensorTab::Ventole => vec![("Ventole", self.sensors.iter()
                .filter(|s| s.sensor_type == SensorType::Fan
                    || ["fan", "rpm", "ventola"].iter().any(|name|
                        crate::hwinfo::ci_contains(&s.reading_name, name)))
                .collect())],
            _ => groups.iter().filter(|(kind, _)| match self.active_tab {
                SensorTab::Tensioni => matches!(kind, SensorType::Voltage | SensorType::Current),
                SensorTab::Carichi => matches!(kind, SensorType::Load | SensorType::Power | SensorType::Clock),
                _ => true,
            }).map(|(kind, label)| (*label, self.sensors.iter()
                .filter(|s| s.sensor_type == *kind && !auxiliary_sensor(s)).collect())).collect(),
        };
        if matches!(self.active_tab, SensorTab::Temperatura | SensorTab::Tutti) {
            let aux = self.sensors.iter().filter(|s| auxiliary_sensor(s)).collect();
            sections.push(("AUX", aux));
        }
        sections.retain(|(_, items)| !items.is_empty());
        sections
    }

}

impl Default for SensorsPanel {
    fn default() -> Self { Self::new() }
}

// ── Widget helpers ────────────────────────────────────────────────────────────

#[cfg(not(target_os = "linux"))]
fn source_btn(
    label: &'static str,
    active: bool,
    available: bool,
    msg: Message,
) -> Element<'static, Message> {
    // Colore del testo quando la sorgente non e' disponibile.
    //
    // Prima era un grigio scuro (0.30) su un bottone "secondary" anch'esso
    // scuro: il contrasto era cosi' basso che il pulsante sembrava vuoto,
    // e l'utente non capiva che HWiNFO64/AIDA64 fossero spenti. Ora e'
    // grigio chiaro: ben leggibile, ma chiaramente "non attivo".
    let text_color = if !available {
        iced::Color { r: 0.62, g: 0.66, b: 0.70, a: 1.0 }
    } else if active {
        iced::Color::WHITE
    } else {
        palette::TEXT_BRIGHT
    };
    let style = if active && available {
        crate::btn::primary
    } else {
        crate::btn::secondary
    };
    let dot = if available { " ●" } else { " ○" };
    iced::widget::button(
        Text::new(format!("{}{}", label, dot))
            .size(14)
            .color(text_color)
            .align_x(alignment::Horizontal::Center),
    )
    .padding([3u16, 10u16])
    .style(style)
    .on_press_maybe(if available { Some(msg) } else { None })
    .into()
}

fn status_text<'a>(source: &SensorSource, status: &SourceStatus, count: usize) -> Element<'a, Message> {
    let (text, color) = if count > 0 {
        (format!("{} attivo — {} sensori", source, count), palette::SUCCESS)
    } else if !status.hwinfo_available && !status.aida64_available {
        (format!("Nessuna sorgente attiva · {source}"), palette::DANGER)
    } else {
        (format!("{} connesso, nessun dato", source), palette::WARNING)
    };
    Text::new(text).size(13).color(color).into()
}

fn tab_btn(label: &'static str, active: bool, msg: Message) -> Element<'static, Message> {
    iced::widget::button(
        Text::new(label).size(13).width(Length::Fill)
            .align_x(alignment::Horizontal::Center),
    )
    .width(Length::Fill)
    .padding([4u16, 4u16])
    .style(if active { crate::btn::primary } else { crate::btn::secondary })
    .on_press(msg)
    .into()
}

fn section_label(text: &'static str) -> Element<'static, Message> {
    let accent = match text {
        "CPU" => iced::Color::from_rgb8(103, 229, 255),
        "GPU" => iced::Color::from_rgb8(205, 167, 255),
        "Scheda Madre" => iced::Color::from_rgb8(115, 240, 190),
        "AUX" => palette::WARNING,
        _ => palette::TEXT_BRIGHT,
    };
    Row::new().height(HEADING_HEIGHT).align_y(iced::Alignment::Center)
        .push(Container::new(Text::new(text.to_uppercase()).size(14).color(accent)
            .font(iced::Font { weight: iced::font::Weight::Semibold, ..crate::dashboard_font() }))
            .padding([1, 7])
            .style(move |_| iced::widget::container::Style {
                background: Some(iced::Color { a: 0.12, ..accent }.into()),
                border: iced::Border { radius: 5.0.into(), ..Default::default() },
                ..Default::default()
            }))
        .into()
}

const HEADING_HEIGHT: f32 = 22.0;
const ROW_HEIGHT: f32 = 31.0;
const FOOTER_HEIGHT: f32 = 29.0;

type SensorSection<'a> = (&'static str, Vec<&'a SensorReading>);

enum SensorLine<'a> {
    Heading(&'static str),
    Readings(Vec<&'a SensorReading>),
}

fn sensor_pages(
    sections: Vec<SensorSection<'_>>,
    columns: usize,
    available_height: f32,
) -> Vec<Vec<SensorLine<'_>>> {
    let height = sections.iter().map(|(_, items)|
        HEADING_HEIGHT + items.len().div_ceil(columns) as f32 * ROW_HEIGHT).sum::<f32>();
    let body_limit = if height > available_height {
        (available_height - FOOTER_HEIGHT).max(HEADING_HEIGHT + ROW_HEIGHT)
    } else {
        available_height
    };
    let mut pages = Vec::new();
    let mut page = Vec::new();
    let mut used = 0.0;
    for (label, items) in sections {
        let mut heading = true;
        for items in items.chunks(columns) {
            let needed = ROW_HEIGHT + if heading { HEADING_HEIGHT } else { 0.0 };
            if used + needed > body_limit && !page.is_empty() {
                pages.push(std::mem::take(&mut page));
                used = 0.0;
                heading = true;
            }
            if heading {
                page.push(SensorLine::Heading(label));
                used += HEADING_HEIGHT;
                heading = false;
            }
            page.push(SensorLine::Readings(items.to_vec()));
            used += ROW_HEIGHT;
        }
    }
    pages.push(page);
    pages
}

fn auxiliary_sensor(s: &SensorReading) -> bool {
    #[cfg(target_os = "linux")]
    return s.source == SensorSource::HWMON && s.sensor_type == SensorType::Temperature
        && s.reading_name.starts_with("AUXTIN");
    #[cfg(not(target_os = "linux"))]
    { let _ = s; false }
}

fn color_for_temp(v: f32) -> iced::Color {
    if      v > 90.0 { palette::DANGER }
    else if v > 75.0 { palette::WARNING }
    else if v > 60.0 { iced::Color { r: 1.0, g: 0.85, b: 0.1, a: 1.0 } }
    else             { palette::SUCCESS }
}

fn value_color(s: &SensorReading) -> iced::Color {
    match s.sensor_type {
        SensorType::Temperature => color_for_temp(s.value.abs()),
        SensorType::Fan         => palette::BLUE_PRIMARY,
        SensorType::Voltage     => iced::Color { r: 1.0, g: 0.85, b: 0.1, a: 1.0 },
        SensorType::Current     => iced::Color { r: 0.8, g: 0.5,  b: 1.0, a: 1.0 },
        SensorType::Power       => palette::DANGER,
        SensorType::Load        => palette::BLUE_PRIMARY,
        SensorType::Clock       => iced::Color { r: 0.3, g: 0.85, b: 1.0, a: 1.0 },
        _                       => palette::TEXT_BRIGHT,
    }
}

fn fmt_value(s: &SensorReading) -> String {
    match s.sensor_type {
        SensorType::Fan | SensorType::Clock => format!("{:.0}", s.value),
        SensorType::Voltage | SensorType::Current => format!("{:.3}", s.value),
        _ => format!("{:.1}", s.value),
    }
}

fn sensor_card(s: &SensorReading) -> Element<'_, Message> {
    use iced::{widget::tooltip, widget::text::Wrapping};
    #[cfg(target_os = "linux")]
    let (alarm, fault) = (s.hwmon_alarm, s.hwmon_fault);
    #[cfg(not(target_os = "linux"))]
    let (alarm, fault) = (None::<bool>, None::<bool>);
    let accent = if alarm == Some(true) || fault == Some(true) {
        palette::DANGER
    } else if auxiliary_sensor(s) {
        palette::WARNING
    } else {
        value_color(s)
    };
    let value = if s.value.is_finite() {
        format!("{} {}", fmt_value(s), s.unit)
    } else { "N/D".into() };
    let value = match (fault, alarm) {
        (Some(true), _) => format!("{value} !"),
        (_, Some(true)) => format!("{value} !"),
        _ => value,
    };
    let name = s.reading_name.replace('_', " ");
    let label = Container::new(Text::new(name).size(12).color(palette::TEXT_BRIGHT))
        .width(Length::Fill).height(ROW_HEIGHT - 9.0).clip(true)
        .align_y(iced::alignment::Vertical::Center);
    let reading = Row::new().align_y(iced::Alignment::Center).spacing(5)
        .push(label)
        .push(Text::new(value).size(14).color(accent).wrapping(Wrapping::None));
    let card = Container::new(reading).padding([3, 6]).center_y(ROW_HEIGHT - 3.0)
        .width(Length::Fill).clip(true)
        .style(|_| crate::appearance::panel(iced::widget::container::Style {
            background: Some(iced::Color::from_rgba8(7, 29, 20, 0.88).into()),
            border: iced::Border { color: iced::Color::from_rgba8(52, 120, 158, 0.5),
                width: 1.0, radius: 6.0.into() },
            ..Default::default()
        }));
    let mut detail = if auxiliary_sensor(s) {
        format!("{} · {}",
            s.reading_name, s.sensor_id)
    } else if s.min != 0.0 || s.max != 0.0 {
        format!("{} · {}\nMin {:.1} · Max {:.1}", s.reading_name, s.source, s.min, s.max)
    } else {
        format!("{} · {}", s.reading_name, s.source)
    };
    if fault == Some(true) {
        detail.push_str("\nGuasto");
    } else if alarm == Some(true) {
        detail.push_str("\nAllarme");
    }
    tooltip(card, Container::new(Text::new(detail).size(12)).padding(7)
        .max_width(320).style(iced::widget::container::rounded_box), tooltip::Position::Top)
        .into()
}

/// Messaggio vuoto con diagnostica — distingue tra sorgente offline e tipo sensore assente.
fn empty_msg<'a>(
    detail: &'static str,
    source: &SensorSource,
    status: &SourceStatus,
) -> Element<'a, Message> {
    let src_ok = match source {
        #[cfg(target_os = "linux")]
        SensorSource::HWMON => status.hwinfo_available,
        SensorSource::HWiNFO => status.hwinfo_available,
        SensorSource::AIDA64 => status.aida64_available,
    };

    let full_msg = if src_ok {
        format!("{}\n\n(La sorgente {} è attiva ma non riporta questo tipo di sensore.)", detail, source)
    } else {
        let how = match source {
            #[cfg(target_os = "linux")]
            SensorSource::HWMON =>
                "Verificare i sensori esposti dal kernel in /sys/class/hwmon.",
            SensorSource::HWiNFO =>
                "Avviare HWiNFO64 → Impostazioni → Generale → Supporto Shared Memory ✓",
            SensorSource::AIDA64 =>
                "Avviare AIDA64 → Preferenze → Hardware Monitoring → External Applications → Enable Shared Memory ✓",
        };
        format!("{}\n\n{} non rilevato.\n{}", detail, source, how)
    };

    Container::new(
        Text::new(full_msg)
            .size(14)
            .color(
                if src_ok { palette::BLUE_DIM } else { palette::WARNING }
            ),
    )
    .padding(16)
    .width(Length::Fill)
    .into()
}

// CardStyle: ora inline via closure

#[cfg(test)]
mod test_selezione_sorgente {
    use super::*;

    #[test]
    fn passa_a_quella_che_ha_dati() {
        // HWiNFO selezionato ma vuoto, AIDA64 con dati: si passa ad AIDA64.
        // E' il caso reale: HWiNFO senza memoria condivisa, AIDA64 attivo.
        assert_eq!(
            scegli_sorgente(true, true, SensorSource::HWiNFO, 0),
            SensorSource::AIDA64
        );
    }

    #[test]
    fn resta_dove_ci_sono_dati() {
        assert_eq!(
            scegli_sorgente(true, true, SensorSource::HWiNFO, 12),
            SensorSource::HWiNFO
        );
    }

    #[test]
    fn senza_dati_da_nessuna_parte_non_cambia() {
        // Niente da guadagnare a cambiare: resta dov'e'.
        assert_eq!(
            scegli_sorgente(false, false, SensorSource::AIDA64, 0),
            SensorSource::AIDA64
        );
    }
}

pub fn run_preview() {
    let _ = iced::application("Anteprima sensori CryoCooling", |panel: &mut SensorsPanel, message| {
        match message {
            Message::SensorTab(tab) => panel.set_tab(tab),
            Message::SensorPage(page) => panel.set_page(page),
            Message::SetSensorSource(source) => panel.set_source(source),
            Message::Tick => panel.tick(),
            _ => {},
        }
        iced::Task::none()
    }, SensorsPanel::view)
        .theme(|_| iced::Theme::Dark)
        .default_font(crate::dashboard_font())
        .window_size(iced::Size::new(734.0, 520.0))
        .subscription(|_| iced::time::every(std::time::Duration::from_millis(500)).map(|_| Message::Tick))
        .run();
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    fn reading(label: &str) -> SensorReading {
        SensorReading {
            source: SensorSource::default(),
            sensor_type: SensorType::Temperature,
            sensor_name: "CPU".into(),
            reading_name: label.into(),
            sensor_id: String::new(),
            #[cfg(target_os = "linux")]
            hwmon_alarm: None,
            #[cfg(target_os = "linux")]
            hwmon_fault: None,
            value: 42.0,
            unit: "°C".into(),
            min: 0.0,
            max: 0.0,
        }
    }

    #[test]
    fn keeps_portrait_temperatures_on_one_page_when_they_fit() {
        let r = reading("Core");
        let pages = sensor_pages(vec![("CPU", vec![&r; 8]), ("GPU", vec![&r; 4]),
            ("Scheda Madre", vec![&r; 6]), ("Ingressi AUX", vec![&r; 5])], 4, 500.0);
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn short_views_repeat_headings_and_preserve_every_reading() {
        let readings = (0..9).map(|n| reading(&n.to_string())).collect::<Vec<_>>();
        let pages = sensor_pages(vec![("CPU", readings.iter().collect())], 2, 130.0);
        assert!(pages.len() > 1);
        let visible_names = pages.iter().flat_map(|page| {
            assert!(matches!(page.first(), Some(SensorLine::Heading("CPU"))));
            page.iter().flat_map(|line| match line {
                SensorLine::Heading(_) => [].iter(),
                SensorLine::Readings(items) => items.iter(),
            })
        }).map(|s| s.reading_name.as_str()).collect::<Vec<_>>();
        assert_eq!(visible_names, readings.iter().map(|s| s.reading_name.as_str()).collect::<Vec<_>>());
    }
}
