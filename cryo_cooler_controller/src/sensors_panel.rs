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
    widget::{Column, Container, Row, Scrollable, Text, horizontal_rule},
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
}

impl SensorsPanel {
    pub fn new() -> Self {
        let status = crate::hwinfo::check_sources();
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
        }
    }

    pub fn tick(&mut self) {
        self.refresh_ticks = self.refresh_ticks.wrapping_add(1);
        if self.refresh_ticks % 20 == 0 {
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
        self.active_tab = tab;
    }

    pub fn set_source(&mut self, source: SensorSource) {
        self.active_source = source;
        self.sensors = crate::hwinfo::read_all_sensors(&self.active_source);
    }

    #[allow(dead_code)]
    pub fn hwinfo_ok(&self) -> bool { self.status.hwinfo_available }
    #[allow(dead_code)]
    pub fn aida64_ok(&self) -> bool { self.status.aida64_available }

    // ── View principale ───────────────────────────────────────────────────────

    pub fn view(&self) -> Element<'_, Message> {
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
            .spacing(0)
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

        let body = Scrollable::new(self.view_tab())
            .height(Length::Fill)
            .width(Length::Fill);

        Column::new()
            .push(header)
            .push(horizontal_rule(1))
            .push(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_tab(&self) -> Element<'_, Message> {
        match self.active_tab {
            SensorTab::Temperatura => self.view_temperatures(),
            SensorTab::Ventole     => self.view_fans(),
            SensorTab::Tensioni    => self.view_voltages(),
            SensorTab::Carichi     => self.view_loads(),
            SensorTab::Tutti       => self.view_all(),
        }
    }

    // ── Tab: Temperature ─────────────────────────────────────────────────────

    fn view_temperatures(&self) -> Element<'_, Message> {
        let temps: Vec<&SensorReading> = self.sensors.iter()
            .filter(|s| s.sensor_type == SensorType::Temperature)
            .collect();

        if temps.is_empty() {
            return empty_msg("Nessun sensore di temperatura rilevato.", &self.active_source, &self.status);
        }

        let categories = ["CPU", "GPU", "Scheda Madre", "Storage", "Altro"];
        let mut col = Column::new().spacing(3).padding(6).width(Length::Fill);

        for cat in &categories {
            let sub: Vec<&SensorReading> = temps.iter()
                .filter(|s| s.category() == *cat)
                .copied()
                .collect();
            if sub.is_empty() { continue; }
            col = col.push(section_label(*cat));
            col = col.push(sensor_grid_ref(&sub));
        }
        col.into()
    }

    // ── Tab: Ventole ──────────────────────────────────────────────────────────
    // FIX: cerca sia per tipo Fan sia per nome contenente "fan"/"rpm"

    fn view_fans(&self) -> Element<'_, Message> {
        // zero-alloc: niente to_lowercase() per frame
        let fans: Vec<&SensorReading> = self.sensors.iter()
            .filter(|s| {
                s.sensor_type == SensorType::Fan
                    || crate::hwinfo::ci_contains(&s.reading_name, "fan")
                    || crate::hwinfo::ci_contains(&s.reading_name, "rpm")
                    || crate::hwinfo::ci_contains(&s.reading_name, "ventola")
            })
            .collect();

        if fans.is_empty() {
            return empty_msg(
                "Nessuna ventola rilevata.\n\
                 Verificare che HWiNFO64 o AIDA64 stia riportando i sensori del SuperIO.",
                &self.active_source, &self.status,
            );
        }

        let mut col = Column::new().spacing(2).padding(6).width(Length::Fill);
        col = col.push(section_label(&format!("{} ventole rilevate", fans.len())));
        for fan in &fans {
            col = col.push(fan_row(fan));
        }
        col = col.push(section_label("Controllo velocità"));
        col = col.push(
            Text::new(
                "La lettura RPM è sempre disponibile. Il controllo diretto della velocità \
                 richiede LibreHardwareMonitor con privilegi Administrator o accesso \
                 ai registri EC/SuperIO della scheda madre."
            )
            .size(13)
            .color(palette::BLUE_DIM),
        );
        col.into()
    }

    // ── Tab: Tensioni ─────────────────────────────────────────────────────────
    // FIX: tensioni non hanno categoria significativa → mostrale tutte in una griglia piatta
    // aggiungendo anche Current (correnti) che arriva con le tensioni in molti chip

    fn view_voltages(&self) -> Element<'_, Message> {
        let volts: Vec<&SensorReading> = self.sensors.iter()
            .filter(|s| matches!(s.sensor_type, SensorType::Voltage | SensorType::Current))
            .collect();

        if volts.is_empty() {
            return empty_msg(
                "Nessuna tensione rilevata.\n\
                 Assicurarsi che HWiNFO64 stia leggendo il chip SuperIO/EC della scheda madre.",
                &self.active_source, &self.status,
            );
        }

        // Separa Tensioni da Correnti se entrambe presenti
        let v_only: Vec<&SensorReading> = volts.iter()
            .filter(|s| s.sensor_type == SensorType::Voltage)
            .copied().collect();
        let c_only: Vec<&SensorReading> = volts.iter()
            .filter(|s| s.sensor_type == SensorType::Current)
            .copied().collect();

        let mut col = Column::new().spacing(3).padding(6).width(Length::Fill);

        if !v_only.is_empty() {
            col = col.push(section_label(&format!("Tensioni ({} rilevate)", v_only.len())));
            col = col.push(sensor_grid_ref(&v_only));
        }
        if !c_only.is_empty() {
            col = col.push(section_label(&format!("Correnti ({} rilevate)", c_only.len())));
            col = col.push(sensor_grid_ref(&c_only));
        }
        col.into()
    }

    // ── Tab: Carichi ─────────────────────────────────────────────────────────
    // FIX: singola reference, no &&, raggruppamento per categoria con fallback "Altro"

    fn view_loads(&self) -> Element<'_, Message> {
        let loads: Vec<&SensorReading> = self.sensors.iter()
            .filter(|s| matches!(s.sensor_type,
                SensorType::Load | SensorType::Power | SensorType::Clock))
            .collect();

        if loads.is_empty() {
            return empty_msg(
                "Nessun dato di carico/potenza rilevato.",
                &self.active_source, &self.status,
            );
        }

        // Raggruppa: prima CPU/GPU/Scheda Madre, poi power, poi clock, poi resto
        let type_groups: &[(SensorType, &str)] = &[
            (SensorType::Load,  "Carichi (%)"),
            (SensorType::Power, "Potenze (W)"),
            (SensorType::Clock, "Frequenze (MHz)"),
        ];

        let mut col = Column::new().spacing(3).padding(6).width(Length::Fill);
        for (t, label) in type_groups {
            let sub: Vec<&SensorReading> = loads.iter()
                .filter(|s| &s.sensor_type == t)
                .copied().collect();
            if sub.is_empty() { continue; }
            col = col.push(section_label(*label));

            // Raggruppa per componente dentro ogni tipo
            let categories = ["CPU", "GPU", "Scheda Madre", "Altro"];
            for cat in &categories {
                let csub: Vec<&SensorReading> = sub.iter()
                    .filter(|s| s.category() == *cat)
                    .copied().collect();
                if csub.is_empty() { continue; }
                col = col.push(section_label(*cat));
                col = col.push(sensor_grid_ref(&csub));
            }
        }
        col.into()
    }

    // ── Tab: Tutti ────────────────────────────────────────────────────────────

    fn view_all(&self) -> Element<'_, Message> {
        if self.sensors.is_empty() {
            return empty_msg("Nessun sensore disponibile.", &self.active_source, &self.status);
        }

        let type_groups: &[(SensorType, &str)] = &[
            (SensorType::Temperature, "Temperature"),
            (SensorType::Fan,         "Ventole (RPM)"),
            (SensorType::Voltage,     "Tensioni (V)"),
            (SensorType::Current,     "Correnti (A)"),
            (SensorType::Power,       "Potenze (W)"),
            (SensorType::Load,        "Carichi (%)"),
            (SensorType::Clock,       "Frequenze (MHz)"),
        ];

        let mut col = Column::new().spacing(3).padding(6).width(Length::Fill);
        for (t, label) in type_groups {
            let sub: Vec<&SensorReading> = self.sensors.iter()
                .filter(|s| &s.sensor_type == t)
                .collect();
            if sub.is_empty() { continue; }
            col = col.push(section_label(&format!("{} ({})", label, sub.len())));
            col = col.push(sensor_grid_ref(&sub));
        }

        // Sensori con tipo Other o non classificati
        let other: Vec<&SensorReading> = self.sensors.iter()
            .filter(|s| matches!(s.sensor_type, SensorType::Other))
            .collect();
        if !other.is_empty() {
            col = col.push(section_label("Altro"));
            col = col.push(sensor_grid_ref(&other));
        }

        col.into()
    }
}

impl Default for SensorsPanel {
    fn default() -> Self { Self::new() }
}

// ── Widget helpers ────────────────────────────────────────────────────────────

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
        ("Nessuna sorgente attiva — avviare HWiNFO64 o AIDA64".to_owned(), palette::DANGER)
    } else {
        (format!("{} connesso, nessun dato", source), palette::WARNING)
    };
    Text::new(text).size(13).color(color).into()
}

fn tab_btn(label: &'static str, active: bool, msg: Message) -> Element<'static, Message> {
    iced::widget::button(
        Text::new(label).size(13)
            .align_x(alignment::Horizontal::Center),
    )
    .padding([3u16, 10u16])
    .style(if active { crate::btn::primary } else { crate::btn::secondary })
    .on_press(msg)
    .into()
}

fn section_label(text: impl Into<String>) -> Element<'static, Message> {
    let t = text.into();
    Row::new()
        .padding([3u16, 2u16])
        .spacing(6)
        .align_y(iced::Alignment::Center)
        .push(Text::new(t).size(12)
            .color(palette::BLUE_DIM))
        .push(horizontal_rule(1))
        .width(Length::Fill)
        .into()
}

/// Unica funzione griglia — nessuna doppia reference &&.
/// Divide i sensori in righe da 4 card. Guard su lista vuota.
fn sensor_grid_ref<'a>(items: &[&'a SensorReading]) -> Element<'a, Message> {
    if items.is_empty() {
        return Column::new().into();
    }

    let mut col = Column::new().spacing(3).width(Length::Fill);
    let mut row = Row::new().spacing(4).width(Length::Fill);
    let mut n   = 0usize;

    for s in items {
        row = row.push(sensor_card(s));
        n  += 1;
        if n % 4 == 0 {
            col = col.push(row);
            row = Row::new().spacing(4).width(Length::Fill);
        }
    }
    // Flush ultima riga parziale
    if n % 4 != 0 {
        col = col.push(row);
    }
    col.into()
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
    let color   = value_color(s);
    let min_max = if s.min == 0.0 && s.max == 0.0 {
        format!("{}", s.source)
    } else {
        format!("min {:.1}  max {:.1}", s.min, s.max)
    };

    Container::new(
        Column::new()
            .spacing(2)
            .push(
                Text::new(&s.reading_name)
                    .size(13)
                    .color(palette::BLUE_DIM),
            )
            .push(
                Row::new()
                    .align_y(iced::Alignment::End)
                    .spacing(2)
                    .push(Text::new(fmt_value(s)).size(20)
                        .color(color))
                    .push(Text::new(&s.unit).size(13)
                        .color(palette::BLUE_DIM)),
            )
            .push(
                Text::new(min_max).size(11)
                    .color(
                        iced::Color { r: 0.05, g: 0.28, b: 0.20, a: 1.0 }
                    ),
            ),
    )
    .padding([5u16, 8u16])
    .width(Length::Fill)
    .style(|_: &iced::Theme| iced::widget::container::Style {
        background: Some(iced::Background::Color(iced::Color { r: 0.010, g: 0.072, b: 0.055, a: 0.88 })),
        border: iced::Border { color: iced::Color { r: 0.12, g: 0.45, b: 1.0, a: 0.22 }, width: 1.0, radius: 10.0.into() },
        text_color: None, shadow: iced::Shadow::default(),
    })
    .into()
}

fn fan_row(s: &SensorReading) -> Element<'_, Message> {
    // RPM bar visiva proporzionale al max (se disponibile)
    let pct_str = if s.max > 0.0 {
        format!("  {:.0}% del max ({:.0} RPM)", s.value / s.max * 100.0, s.max)
    } else {
        String::new()
    };

    Row::new()
        .spacing(12)
        .align_y(iced::Alignment::Center)
        .padding([4u16, 8u16])
        .push(
            Text::new(&s.reading_name)
                .size(15)
                .width(Length::Fixed(200.0))
                .color(palette::TEXT_BRIGHT),
        )
        .push(
            Text::new(format!("{:.0} RPM", s.value))
                .size(17)
                .color(palette::BLUE_PRIMARY)
                .width(Length::Fixed(100.0)),
        )
        .push(
            Text::new(if pct_str.is_empty() {
                format!("sorgente: {}", s.source)
            } else {
                pct_str
            })
            .size(13)
            .color(palette::BLUE_DIM),
        )
        .width(Length::Fill)
        .into()
}

/// Messaggio vuoto con diagnostica — distingue tra sorgente offline e tipo sensore assente.
fn empty_msg<'a>(
    detail: &'static str,
    source: &SensorSource,
    status: &SourceStatus,
) -> Element<'a, Message> {
    let src_ok = match source {
        SensorSource::HWiNFO => status.hwinfo_available,
        SensorSource::AIDA64 => status.aida64_available,
    };

    let full_msg = if src_ok {
        format!("{}\n\n(La sorgente {} è attiva ma non riporta questo tipo di sensore.)", detail, source)
    } else {
        let how = match source {
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
