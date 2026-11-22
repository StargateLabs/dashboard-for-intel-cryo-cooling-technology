//! Manual fan speed control. The 232B shares the TF2 USB worker.
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant, SystemTime};

use iced::{Element, Length, Task, widget::{button, checkbox, slider, Column, Container, Row, Text}};
use crate::{linux_hid::{self, Handle}, palette, tf2, Message};

const BOX_INTERFACE: &[u8] = &[0x06, 0, 0xff, 0x09, 1, 0xa1, 1];
const PORTS: std::ops::RangeInclusive<u8> = 1..=5;

pub(crate) fn refresh_due(next: &mut Option<Instant>) -> bool {
    let now = Instant::now();
    if next.is_some_and(|deadline| now >= deadline) {
        *next = Some(now + Duration::from_secs(2));
        return true;
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoxId {
    #[default]
    First,
    Second,
}

impl BoxId {
    fn pid(self) -> u16 {
        match self { Self::First => 0x232b, Self::Second => 0x232d }
    }

    fn reserved(self, port: u8) -> bool {
        self == Self::First && port == 1
    }

    fn label(self) -> &'static str {
        match self { Self::First => "Box 232B", Self::Second => "Box 232D" }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FanReading {
    rpm: u16,
    duty: u8,
}

#[derive(Debug, Clone, Copy)]
pub enum Action {
    Open,
    Close,
    SelectBox(BoxId),
    SelectPort(u8),
    Confirm(bool),
    Speed(u8),
    Refresh,
    Apply,
    /// Accende/spegne la gestione ventole della dashboard. Spenta, i box sono
    /// liberi per SignalRGB: nessuna velocità, nessun LED, solo lettura TF2.
    ToggleControl(bool),
}

#[derive(Debug)]
pub(crate) enum Operation {
    Read(BoxId),
    Set { device: BoxId, port: u8, speed: u8 },
}

#[derive(Debug, Clone)]
pub enum Reply {
    Scanned([Option<FanReading>; 5]),
    Applied { port: u8, reading: FanReading },
}

pub(crate) fn open(id: BoxId) -> Option<Handle> {
    linux_hid::open(id.pid(), BOX_INTERFACE)
}

fn blue_frame(device: BoxId, port: u8) -> Option<[u8; 193]> {
    if !PORTS.contains(&port) || device.reserved(port) { return None; }
    let mut frame = [0; 193];
    frame[..5].copy_from_slice(&[0, 0x32, 0x52, port, 0x24]);
    for led in frame[5..185].chunks_exact_mut(3) {
        led.copy_from_slice(&[0, 0, 255]);
    }
    Some(frame)
}

#[derive(Clone, Copy)]
struct Pulse {
    start: Instant,
    end: Instant,
    origin: f32,
    speed: f32,
}

pub(crate) struct Lighting {
    device: BoxId,
    static_due: Option<Instant>,
    frame_due: Option<Instant>,
    check_owner: Instant,
    exclusive: bool,
    random: u64,
    next_pulse: [Instant; 5],
    pulses: [Option<Pulse>; 5],
}

impl Lighting {
    pub(crate) fn new(device: BoxId, blue: bool, energy: bool) -> Self {
        let now = Instant::now();
        let seed = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)
            .map_or(1, |epoch| epoch.as_nanos() as u64) ^ u64::from(device.pid());
        let mut lighting = Self {
            device,
            static_due: (blue && !energy).then_some(now),
            frame_due: energy.then_some(now),
            check_owner: now,
            exclusive: false,
            random: seed.max(1),
            next_pulse: [now; 5],
            pulses: [None; 5],
        };
        for port in 0..5 {
            lighting.next_pulse[port] = now + Duration::from_secs_f32(lighting.random_fraction() * 1.5);
        }
        lighting
    }

    fn random_fraction(&mut self) -> f32 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        (self.random >> 40) as f32 / 16_777_216.0
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.static_due.into_iter().chain(self.frame_due).min()
    }

    pub(crate) fn defer_reconnect(&mut self) {
        let retry = Instant::now() + Duration::from_secs(1);
        self.static_due = self.static_due.map(|_| retry);
        self.frame_due = self.frame_due.map(|_| retry);
        self.check_owner = retry;
    }

    fn paint_pulse(&mut self, port: u8, now: Instant, frame: &mut [u8; 193]) {
        let slot = usize::from(port - 1);
        let reservoir = self.device == BoxId::First && matches!(port, 2 | 4);
        let length = if reservoir { 8 } else { 60 };
        let trail = if reservoir { 4.0 } else { 6.0 };
        let pulse_finished = self.pulses[slot].is_none_or(|pulse| now >= pulse.end);
        if pulse_finished && now >= self.next_pulse[slot] {
            let reverse = self.random_fraction() < 0.5;
            let origin = if reservoir {
                if reverse { (length - 1) as f32 } else { 0.0 }
            } else {
                8.0 + self.random_fraction() * 33.0
            };
            let speed = if reservoir { 4.5 + self.random_fraction() * 2.5 } else { 10.0 + self.random_fraction() * 6.0 };
            let speed = if reverse { -speed } else { speed };
            let distance = if reverse { origin } else { length as f32 - origin };
            let end = now + Duration::from_secs_f32((distance + trail) / speed.abs());
            self.pulses[slot] = Some(Pulse { start: now, end, origin, speed });
            let gap = if reservoir { 1.1 } else { 2.4 };
            self.next_pulse[slot] = end + Duration::from_secs_f32(0.3 + self.random_fraction() * gap);
        }
        let Some(pulse) = self.pulses[slot].filter(|pulse| now < pulse.end) else { return; };
        let head = pulse.origin + pulse.speed * now.saturating_duration_since(pulse.start).as_secs_f32();
        for (index, led) in frame[5..185].chunks_exact_mut(3).enumerate() {
            let distance = (head - (index % length) as f32) * pulse.speed.signum();
            if (0.0..trail).contains(&distance) {
                let white = if distance < 1.5 { 255 } else { (255.0 * ((trail - distance) / (trail - 1.5)).powi(2)) as u8 };
                led[0] = white;
                led[1] = white;
            }
        }
    }

    pub(crate) fn refresh(&mut self, handle: &Handle) -> bool {
        let now = Instant::now();
        let static_due = refresh_due(&mut self.static_due);
        let frame_due = self.frame_due.is_some_and(|deadline| now >= deadline);
        if frame_due { self.frame_due = Some(now + Duration::from_millis(125)); }
        if !(static_due || frame_due) { return true; }

        if now >= self.check_owner {
            self.exclusive = !linux_hid::has_other_owner(handle);
            self.check_owner = now + Duration::from_secs(2);
        }
        if !self.exclusive { return true; }

        for port in PORTS {
            let animated = self.frame_due.is_some();
            if !(if animated { frame_due } else { static_due }) { continue; }
            let Some(mut frame) = blue_frame(self.device, port) else { continue; };
            if animated { self.paint_pulse(port, now, &mut frame); }
            if linux_hid::transfer(handle, &mut frame, true, 75) != Some(frame.len()) { return false; }
            let mut response = [0; 65];
            // Consume the lighting acknowledgement before the next tachometer request.
            let _ = linux_hid::transfer(handle, &mut response, false, 75);
        }
        true
    }
}

fn fan_reading(report: &[u8], port: u8) -> Option<FanReading> {
    let body = tf2::payload_for_port(report, 0x51, port)?;
    let duty = body[4];
    let rpm = u16::from_le_bytes([body[5], body[6]]);
    (duty <= 100).then_some(FanReading { rpm, duty })
}

fn read_port(handle: &Handle, port: u8) -> Result<FanReading, tf2::BoxReplyError> {
    fan_reading(&tf2::exchange_ledfan(handle, 0x51, port)?, port)
        .ok_or(tf2::BoxReplyError::NoReply)
}

fn read_ports(handle: &Handle, device: BoxId) -> Result<[Option<FanReading>; 5], tf2::BoxReplyError> {
    use tf2::BoxReplyError::{NoReply, Rejected};

    let mut readings = [None; 5];
    let mut all_refused = true;
    for port in PORTS.filter(|port| !device.reserved(*port)) {
        match read_port(handle, port) {
            Ok(reading) => {
                readings[usize::from(port - 1)] = Some(reading);
                all_refused = false;
            }
            Err(Rejected) => {}
            Err(NoReply) => all_refused = false,
        }
    }
    if readings.iter().any(Option::is_some) {
        Ok(readings)
    } else if all_refused {
        Err(Rejected)
    } else {
        Err(NoReply)
    }
}

/// Initialization observed on the vendor path (`FE 33`). A box that replies
/// `33 FE` to every query is in its refusal state; this clears it.
pub(crate) fn initialize(handle: &Handle) -> bool {
    let mut command = [0; 193];
    command[..3].copy_from_slice(&[0, 0xfe, 0x33]);
    if linux_hid::transfer(handle, &mut command, true, 300) != Some(command.len()) {
        return false;
    }
    let mut report = [0; 65];
    let Some(length) = linux_hid::transfer(handle, &mut report, false, 300) else {
        return false;
    };
    let bytes = &report[..length];
    bytes.strip_prefix(&[0]).unwrap_or(bytes).starts_with(&[0xfe, 0x33, 0xfc])
}

fn scan(handle: &Handle, device: BoxId) -> Result<[Option<FanReading>; 5], tf2::BoxReplyError> {
    match read_ports(handle, device) {
        // Both boxes can publish the FE33 refusal (seen on 232D and, after a
        // reboot, on 232B). FE33 is the observed initialization that clears it.
        Err(tf2::BoxReplyError::Rejected) if initialize(handle) => {
            read_ports(handle, device)
        }
        result => result,
    }
}

fn speed_frame(device: BoxId, port: u8, speed: u8) -> Option<[u8; 193]> {
    if !PORTS.contains(&port) || !(20..=100).contains(&speed) || device.reserved(port) {
        return None;
    }
    // Same unnumbered 192-byte HID payload as the captured TT Commander write.
    let mut frame = [0; 193];
    frame[..6].copy_from_slice(&[0, 0x32, 0x51, port, 1, speed]);
    Some(frame)
}

impl Operation {
    pub(crate) fn device(&self) -> BoxId {
        match *self { Self::Read(device) | Self::Set { device, .. } => device }
    }

    pub(crate) fn execute(self, current: Option<&Handle>) -> Result<Reply, String> {
        let device = self.device();
        let opened = current.is_none().then(|| open(device)).flatten();
        let handle = current.or(opened.as_ref())
            .ok_or_else(|| format!("{} non accessibile. Verifica USB e permessi.", device.label()))?;
        if linux_hid::has_other_owner(handle) {
            return Err(format!("{} in uso da un'altra applicazione.", device.label()));
        }
        match self {
            Self::Read(_) => scan(handle, device).map(Reply::Scanned).map_err(|error| {
                let reason = match error {
                    tf2::BoxReplyError::Rejected => "Lettura RPM rifiutata.",
                    tf2::BoxReplyError::NoReply => "Nessuna risposta RPM.",
                };
                format!("{} · {reason}", device.label())
            }),
            Self::Set { port, speed, .. } => {
                let mut frame = speed_frame(device, port, speed)
                    .ok_or_else(|| "Porta riservata o velocità fuori intervallo.".to_owned())?;
                if !read_port(handle, port).is_ok_and(|r| r.rpm > 0) {
                    return Err("Manca una lettura RPM valida sulla porta selezionata.".into());
                }
                if linux_hid::transfer(handle, &mut frame, true, 300) != Some(frame.len()) {
                    return Err("Invio della velocità fallito.".into());
                }
                for _ in 0..3 {
                    std::thread::sleep(Duration::from_millis(100));
                    if let Some(reading) = read_port(handle, port).ok().filter(|r| r.duty == speed) {
                        return Ok(Reply::Applied { port, reading });
                    }
                }
                Err("Comando inviato. Il box non ha confermato la percentuale.".into())
            }
        }
    }
}

pub struct Panel {
    pub visible: bool,
    device: BoxId,
    selected: Option<u8>,
    readings: [Option<FanReading>; 5],
    speed: u8,
    confirmed_fan: bool,
    busy: bool,
    status: String,
    control: Arc<AtomicBool>,
}

impl Default for Panel {
    fn default() -> Self {
        Self { visible: false, device: BoxId::First, selected: None, readings: [None; 5],
            speed: 75, confirmed_fan: false, busy: false, status: String::new(),
            control: Arc::new(AtomicBool::new(true)) }
    }
}

impl Panel {
    /// Condivide con il lettore TF2 l'interruttore della gestione ventole.
    pub fn with_control(mut self, control: Arc<AtomicBool>) -> Self {
        self.control = control;
        self
    }

    fn enabled(&self) -> bool { self.control.load(Ordering::Relaxed) }

    pub fn update(&mut self, action: Action, reader: &tf2::Reader) -> Task<Message> {
        if matches!(action, Action::Close) {
            self.visible = false;
            return Task::none();
        }
        if self.busy { return Task::none(); }
        if let Action::ToggleControl(enable) = action {
            self.control.store(enable, Ordering::Relaxed);
            self.readings = [None; 5];
            self.selected = None;
            self.confirmed_fan = false;
            self.status = if enable {
                "Gestione ventole attiva: la dashboard controlla velocità e LED.".into()
            } else {
                "Gestione ventole disattivata: velocità e LED sono liberi per SignalRGB. \
                 La portata TF2 resta in lettura.".into()
            };
            return Task::none();
        }
        if !self.enabled() {
            // Spenta: nessuna interrogazione dei box, solo lo stato del pannello.
            if matches!(action, Action::Open) { self.visible = true; }
            return Task::none();
        }
        let request = match action {
            Action::Open => {
                self.visible = true;
                Some(Operation::Read(self.device))
            }
            Action::SelectBox(device) => {
                self.device = device;
                self.selected = None;
                self.confirmed_fan = false;
                self.readings = [None; 5];
                Some(Operation::Read(device))
            }
            Action::Refresh => Some(Operation::Read(self.device)),
            Action::SelectPort(port) => {
                if let Some(reading) = self.reading(port).filter(|r| r.rpm > 0) {
                    self.selected = Some(port);
                    self.speed = reading.duty.clamp(20, 100);
                    self.confirmed_fan = false;
                }
                None
            }
            Action::Confirm(confirmed) => { self.confirmed_fan = confirmed; None }
            Action::Speed(speed) => { self.speed = speed; None }
            Action::Apply => self.selected.filter(|&port| self.can_apply(port))
                .map(|port| Operation::Set { device: self.device, port, speed: self.speed }),
            Action::ToggleControl(_) | Action::Close => None,
        };
        request.map_or_else(Task::none, |operation| {
            self.busy = true;
            self.status = "Comunicazione con il controller…".into();
            reader.operate_fans(operation)
        })
    }

    pub fn finish(&mut self, result: Result<Reply, String>) {
        self.busy = false;
        self.status = match result {
            Ok(Reply::Scanned(readings)) => {
                self.readings = readings;
                String::new()
            }
            Ok(Reply::Applied { port, reading }) => {
                self.readings[usize::from(port - 1)] = Some(reading);
                format!("Porta {port} · confermati {}% e {} RPM", reading.duty, reading.rpm)
            }
            Err(error) => {
                self.readings = [None; 5];
                error
            }
        };
    }

    fn reading(&self, port: u8) -> Option<FanReading> {
        if self.device.reserved(port) { return None; }
        self.readings.get(usize::from(port.checked_sub(1)?)).copied().flatten()
    }

    fn can_apply(&self, port: u8) -> bool {
        !self.busy && self.confirmed_fan && self.reading(port).is_some_and(|r| r.rpm > 0)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let action = |label, msg, enabled: bool, highlighted| button(Text::new(label).size(13)).padding([5, 9])
            .style(move |theme, status| {
                if !enabled {
                    let mut style = crate::btn::secondary(theme, status);
                    style.text_color.a *= 0.55;
                    style.border.color.a *= 0.3;
                    style
                } else if highlighted {
                    crate::btn::selected_profile(theme, status)
                } else {
                    crate::btn::secondary(theme, status)
                }
            })
            .on_press_maybe(enabled.then_some(Message::LedFanAction(msg)));
        let mut rows = Column::new().spacing(4);
        for port in PORTS {
            let reading = self.reading(port);
            let value = if self.device.reserved(port) {
                "TF2 · portata".to_owned()
            } else if let Some(reading) = reading {
                format!("{} RPM · {}%", reading.rpm, reading.duty)
            } else {
                "N/D".to_owned()
            };
            let selected = self.selected == Some(port);
            rows = rows.push(
                Row::new().spacing(10).height(28).align_y(iced::Alignment::Center)
                    .push(Text::new(format!("Porta {port}")).size(13).width(65))
                    .push(Text::new(value).size(14).width(Length::Fill))
                    .push_maybe(reading.filter(|r| r.rpm > 0).map(|_| {
                        action(if selected { "Selezionata" } else { "Seleziona" },
                            Action::SelectPort(port), !self.busy, true)
                    })),
            );
        }
        let selection = self.selected.filter(|&port| self.reading(port).is_some_and(|r| r.rpm > 0));
        let toggle = if self.enabled() {
            action("Disattiva gestione", Action::ToggleControl(false), !self.busy, false)
        } else {
            action("Riattiva gestione", Action::ToggleControl(true), !self.busy, true)
        };
        let mut content = Column::new().spacing(9)
            .push(Row::new().align_y(iced::Alignment::Center)
                .push(Text::new("Ventole Thermaltake").size(22).color(palette::TEXT_BRIGHT).width(Length::Fill))
                .push(toggle)
                .push(action("Chiudi", Action::Close, true, false)));
        if !self.enabled() {
            content = content.push(Text::new(
                "Gestione ventole disattivata. Velocità e LED sono liberi per SignalRGB; \
                 la portata TF2 resta in lettura.",
            ).size(13).color(palette::BLUE_DIM));
        } else {
            content = content
                .push(Row::new().spacing(7)
                    .push(action("Box 232B", Action::SelectBox(BoxId::First), !self.busy, self.device == BoxId::First))
                    .push(action("Box 232D", Action::SelectBox(BoxId::Second), !self.busy, self.device == BoxId::Second))
                    .push(action("Aggiorna RPM", Action::Refresh, !self.busy, false)))
                .push(rows)
                .push_maybe(selection.map(|port| Column::new().spacing(9)
                    .push(Text::new(format!("Porta {port} · {}%", self.speed)).size(15).color(palette::TEXT_BRIGHT))
                    .push(slider(20..=100, self.speed, |value| Message::LedFanAction(Action::Speed(value))))
                    .push(checkbox("Porta collegata a una ventola", self.confirmed_fan)
                        .on_toggle_maybe((!self.busy).then_some(|value| Message::LedFanAction(Action::Confirm(value)))))
                    .push(action("Applica velocità", Action::Apply, self.can_apply(port), true))))
                .push_maybe((!self.status.is_empty()).then(|| Text::new(&self.status).size(13).color(palette::TEXT_BRIGHT)));
        }
        Container::new(content).width(475).padding(17)
            .style(|_| iced::widget::container::Style {
                background: Some(iced::Color::from_rgb8(6, 26, 30).into()),
                border: iced::Border { color: palette::BLUE_PRIMARY, width: 1.0,
                    radius: 13.0.into() },
                ..Default::default()
            })
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::{fd::OwnedFd, unix::net::UnixDatagram}, thread};

    #[test]
    fn blue_reaches_led_only_ports_without_addressing_tf2() {
        assert!(blue_frame(BoxId::First, 1).is_none());
        for port in [2, 4] {
            let frame = blue_frame(BoxId::First, port).unwrap();
            assert_eq!(frame[..5], [0, 0x32, 0x52, port, 0x24]);
            assert!(frame[5..185].chunks_exact(3).all(|led| led == [0, 0, 255]));
            assert!(frame[185..].iter().all(|byte| *byte == 0));
        }
        assert!(PORTS.all(|port| blue_frame(BoxId::Second, port).is_some()));
    }

    #[test]
    fn white_pulse_can_illuminate_both_led_only_ports() {
        let mut lighting = Lighting::new(BoxId::First, true, true);
        let now = Instant::now();
        for port in [2, 4] {
            lighting.pulses[usize::from(port - 1)] = Some(Pulse {
                start: now, end: now + Duration::from_secs(1), origin: 4.0, speed: 5.0,
            });
            let mut frame = blue_frame(BoxId::First, port).unwrap();
            lighting.paint_pulse(port, now, &mut frame);
            assert_eq!(frame[17..20], [255, 255, 255]);
            assert_eq!(frame[5..8], [0, 0, 255]);
        }
    }

    #[test]
    fn speed_frame_targets_only_an_allowed_fan_port() {
        let frame = speed_frame(BoxId::First, 3, 75).unwrap();
        assert_eq!(&frame[..6], &[0, 0x32, 0x51, 3, 1, 75]);
        assert!(frame[6..].iter().all(|&byte| byte == 0));
        assert!(speed_frame(BoxId::First, 1, 75).is_none());
        assert!(speed_frame(BoxId::Second, 1, 75).is_some());
        assert!(speed_frame(BoxId::Second, 0, 75).is_none());
        assert!(speed_frame(BoxId::Second, 5, 19).is_none());
    }

    #[test]
    fn mismatched_or_malformed_replies_cannot_confirm_a_fan() {
        let reply = [0x33, 0x51, 1, 0xff, 100, 0xd5, 6, 0];
        assert_eq!(fan_reading(&reply, 1).unwrap().rpm, 1749);
        assert!(fan_reading(&reply, 2).is_none());
        assert!(fan_reading(&reply[..5], 1).is_none());
    }

    #[test]
    fn both_boxes_recover_from_rejected_reads() {
        for device in [BoxId::First, BoxId::Second] {
            let (client, controller) = UnixDatagram::pair().unwrap();
            let controller = Handle::from(OwnedFd::from(controller));
            let ports: Vec<u8> = PORTS.filter(|port| !device.reserved(*port)).collect();
            let emulation = thread::spawn(move || {
                let mut report = [0; 193];
                // Prima scansione: ogni porta risponde col rifiuto `33 FE`.
                for port in &ports {
                    assert_eq!(linux_hid::transfer(&controller, &mut report, false, 4000), Some(193));
                    assert_eq!(report[..4], [0, 0x33, 0x51, *port]);
                    assert_eq!(linux_hid::transfer(&controller, &mut [0x33, 0xfe], true, 300), Some(2));
                }
                // Inizializzazione osservata FE33, identica per entrambi i box.
                assert_eq!(linux_hid::transfer(&controller, &mut report, false, 4000), Some(193));
                assert_eq!(report[..3], [0, 0xfe, 0x33]);
                assert_eq!(linux_hid::transfer(&controller, &mut [0xfe, 0x33, 0xfc], true, 300), Some(3));
                // Dopo FE33 le porte rispondono con una lettura valida.
                for port in &ports {
                    assert_eq!(linux_hid::transfer(&controller, &mut report, false, 4000), Some(193));
                    assert_eq!(report[..4], [0, 0x33, 0x51, *port]);
                    let mut reply = [0x33, 0x51, *port, 0xff, 60, 0x5f, 4, 0];
                    assert_eq!(linux_hid::transfer(&controller, &mut reply, true, 300), Some(reply.len()));
                }
            });
            let handle = Handle::from(OwnedFd::from(client));
            let result = scan(&handle, device);
            assert!(result.as_ref().is_ok_and(|readings| PORTS
                .filter(|port| !device.reserved(*port))
                .all(|port| readings[usize::from(port - 1)]
                    .is_some_and(|r| r.duty == 60 && r.rpm == 1119))),
                "{device:?} deve recuperare dopo il rifiuto FE33: {result:?}");
            emulation.join().unwrap();
        }
    }
}
