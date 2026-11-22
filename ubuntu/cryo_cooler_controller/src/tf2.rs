//! TF2 on the observed LEDFanBox (264a:232b), port 1. Sensor queries only.
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}, mpsc};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct Reading {
    pub flow_lph: f32,
    pub sampled_at: Instant,
}

fn fresh(reading: Option<Reading>, now: Instant) -> Option<Reading> {
    reading.filter(|r| now.saturating_duration_since(r.sampled_at) <= Duration::from_secs(5))
}

enum Command {
    Stop,
    #[cfg(target_os = "linux")]
    Fan(crate::ledfanbox::Operation, tokio::sync::oneshot::Sender<Result<crate::ledfanbox::Reply, String>>),
}

pub struct Reader {
    latest: Arc<Mutex<Option<Reading>>>,
    stop: mpsc::Sender<Command>,
    worker: Option<std::thread::JoinHandle<()>>,
    #[cfg(target_os = "linux")]
    secondary: (mpsc::Sender<Command>, Option<std::thread::JoinHandle<()>>),
    /// Interruttore condiviso con il pannello ventole: spento, nessun comando
    /// di velocità o LED lascia la dashboard.
    #[cfg(target_os = "linux")]
    fan_control: Arc<AtomicBool>,
}
impl Reader {
    pub fn new(thermaltake_blue: bool, thermaltake_energy: bool) -> Self {
        Self::with_fan_control(thermaltake_blue, thermaltake_energy, Arc::new(AtomicBool::new(true)))
    }

    pub fn with_fan_control(
        _thermaltake_blue: bool,
        _thermaltake_energy: bool,
        fan_control: Arc<AtomicBool>,
    ) -> Self {
        let latest = Arc::new(Mutex::new(None));
        let output = latest.clone();
        let (stop, receiver) = mpsc::channel();
        let worker_control = fan_control.clone();
        let worker = std::thread::Builder::new().name("tf2-telemetry".into()).spawn(move || {
            #[cfg(any(windows, target_os = "linux"))]
            {
                let mut device = None;
                let mut next_sample = Instant::now();
                let mut next_recovery = Instant::now();
                #[cfg(target_os = "linux")]
                let mut lighting = crate::ledfanbox::Lighting::new(
                    crate::ledfanbox::BoxId::First, _thermaltake_blue, _thermaltake_energy,
                );
                loop {
                    let started = Instant::now();
                    #[cfg(windows)]
                    if signalrgb_running() {
                        device = None;
                        if let Ok(mut slot) = output.lock() { *slot = bridge_reading(); }
                        if !matches!(receiver.recv_timeout(Duration::from_secs(1)), Err(mpsc::RecvTimeoutError::Timeout)) { break; }
                        continue;
                    }
                    if started >= next_sample {
                        if device.is_none() {
                            #[cfg(windows)]
                            { device = crate::tf3::native::open_device(0x232b, 0xff00, 1, 65, 193); }
                            #[cfg(target_os = "linux")]
                            { device = crate::ledfanbox::open(crate::ledfanbox::BoxId::First); }
                        }
                        let reading = device.as_ref()
                            .and_then(|handle| query_recovering(handle, &mut next_recovery));
                        let interval = if reading.is_some() { Duration::from_secs(1) } else { Duration::from_secs(3) };
                        // Preserve a real reading across a brief USB interruption,
                        // while latest() continues to enforce the five-second limit.
                        if let Some(reading) = reading {
                            if let Ok(mut slot) = output.lock() { *slot = Some(reading); }
                        } else {
                            device = None;
                        }
                        next_sample = started + interval;
                    }
                    #[cfg(target_os = "linux")]
                    if let Some(handle) = device.as_ref() {
                        if worker_control.load(Ordering::Relaxed) { lighting.refresh(handle); }
                    }

                    let delay = next_sample.saturating_duration_since(Instant::now());
                    #[cfg(target_os = "linux")]
                    let delay = match (device.as_ref(), lighting.deadline()) {
                        (Some(_), Some(deadline)) => delay.min(deadline.saturating_duration_since(Instant::now())),
                        _ => delay,
                    };
                    match receiver.recv_timeout(delay) {
                        Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {},
                        #[cfg(target_os = "linux")]
                        Ok(Command::Fan(operation, response)) => {
                            let _ = response.send(operation.execute(device.as_ref()));
                        }
                    }
                }
            }
            #[cfg(not(any(windows, target_os = "linux")))]
            { let _ = output; let _ = receiver.recv(); }
        }).ok();
        #[cfg(target_os = "linux")]
        let secondary = {
            let (sender, receiver) = mpsc::channel();
            let control = fan_control.clone();
            let worker = std::thread::Builder::new().name("ledfanbox-secondary".into())
                .spawn(move || secondary_worker(receiver, _thermaltake_blue, _thermaltake_energy, control)).ok();
            (sender, worker)
        };
        Self { latest, stop, worker, #[cfg(target_os = "linux")] secondary,
            #[cfg(target_os = "linux")] fan_control }
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn operate_fans(&self, operation: crate::ledfanbox::Operation) -> iced::Task<crate::Message> {
        if !self.fan_control.load(Ordering::Relaxed) {
            return iced::Task::perform(async {
                Err("Gestione ventole disattivata: velocità e LED sono liberi per SignalRGB.".to_owned())
            }, crate::Message::LedFanReply);
        }
        let sender = if operation.device() == crate::ledfanbox::BoxId::Second {
            self.secondary.0.clone()
        } else {
            self.stop.clone()
        };
        iced::Task::perform(async move {
            let (tx, rx) = tokio::sync::oneshot::channel();
            sender.send(Command::Fan(operation, tx)).map_err(|_| "Lettore USB non disponibile.".to_owned())?;
            rx.await.map_err(|_| "Richiesta ventola interrotta.".to_owned())?
        }, crate::Message::LedFanReply)
    }

    pub fn latest(&self) -> Option<Reading> {
        fresh(self.latest.lock().ok().and_then(|r| *r), Instant::now())
    }
}
#[cfg(target_os = "linux")]
fn secondary_worker(
    receiver: mpsc::Receiver<Command>,
    blue: bool,
    energy: bool,
    control: Arc<AtomicBool>,
) {
    use crate::ledfanbox::{self, BoxId};
    let mut lighting = ledfanbox::Lighting::new(BoxId::Second, blue, energy);
    let mut device = None;
    loop {
        if !control.load(Ordering::Relaxed) {
            // Gestione affidata a SignalRGB: rilascia il box e non inviare LED.
            device = None;
            if lighting.deadline().is_some() { lighting.defer_reconnect(); }
        }
        if lighting.deadline().is_some_and(|deadline| Instant::now() >= deadline) {
            if device.is_none() { device = ledfanbox::open(BoxId::Second); }
            if !device.as_ref().is_some_and(|handle| lighting.refresh(handle)) {
                device = None;
                lighting.defer_reconnect();
            }
        }
        let command = match lighting.deadline() {
            Some(deadline) => receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())),
            None => receiver.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
        };
        match command {
            Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Ok(Command::Fan(operation, response)) => { let _ = response.send(operation.execute(device.as_ref())); }
        }
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        let _ = self.stop.send(Command::Stop);
        #[cfg(target_os = "linux")]
        {
            let _ = self.secondary.0.send(Command::Stop);
            if let Some(worker) = self.secondary.1.take() { let _ = worker.join(); }
        }
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

// TT RGB Plus 2.1.9 Transform_TF2_RPM: integer RPM/30, calibrated float32,
// then integer truncation. Apply only after the known sensor port is present.
fn flow_lph(raw_rpm: u16) -> f32 {
    let n = raw_rpm / 30;
    let bin = if n <= 9 { 0 } else { (n - 9).min(11) as usize };
    let coefficients: [f32; 12] = [17.8, 17.7, 17.6, 17.5, 17.4, 17.3, 17.14, 17.3, 17.14, 16.66, 16.5, 15.9];
    (n as f32 * coefficients[bin]).trunc()
}

fn payload(report: &[u8], opcode: u8) -> Option<&[u8]> {
    payload_for_port(report, opcode, 1)
}

pub(crate) fn payload_for_port(report: &[u8], opcode: u8, port: u8) -> Option<&[u8]> {
    let p = if report.first() == Some(&0) { report.get(1..)? } else { report };
    if p.len() < 8 || p[..4] != [0x33, opcode, port, 0xff] { return None; }
    Some(p)
}

fn sensor_present(report: &[u8]) -> bool {
    let Some(p) = payload(report, 0x53) else { return false; };
    // Empty ports report 1000 C for both inputs. Do not turn their fan tach into flow.
    [u16::from_le_bytes([p[4], p[5]]), u16::from_le_bytes([p[6], p[7]])]
        .into_iter().all(|raw| (0.0..=100.0).contains(&(raw as f32 / 10.0)))
}

fn reading(temperature: &[u8], speed: &[u8]) -> Option<Reading> {
    if !sensor_present(temperature) { return None; }
    let p = payload(speed, 0x51)?;
    Some(Reading { flow_lph: flow_lph(u16::from_le_bytes([p[5], p[6]])), sampled_at: Instant::now() })
}

#[cfg(any(windows, target_os = "linux"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoxReplyError {
    NoReply,
    Rejected,
}

#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn exchange_ledfan(
    handle: &crate::tf3::native::Handle,
    opcode: u8,
    port: u8,
) -> Result<Vec<u8>, BoxReplyError> {
    use BoxReplyError::{NoReply, Rejected};
    let deadline = Instant::now() + Duration::from_millis(300);
    let mut command = [0u8; 193];
    command[..4].copy_from_slice(&[0, 0x33, opcode, port]);
    if crate::tf3::native::transfer(handle, &mut command, true, 300) != Some(command.len()) {
        return Err(NoReply);
    }
    let mut failure = NoReply;
    for _ in 0..16 {
        let remaining = deadline.checked_duration_since(Instant::now()).ok_or(failure)?;
        let mut report = [0u8; 65];
        let count = crate::tf3::native::transfer(handle, &mut report, false, remaining.as_millis().max(1) as u32)
            .ok_or(failure)?;
        let bytes = &report[..count];
        if payload_for_port(bytes, opcode, port).is_some() { return Ok(bytes.to_vec()); }
        let body = bytes.strip_prefix(&[0]).unwrap_or(bytes);
        if body.starts_with(&[0x33, 0xfe]) { failure = Rejected; }
    }
    Err(failure)
}

#[cfg(any(windows, target_os = "linux"))]
fn query(handle: &crate::tf3::native::Handle) -> Result<Option<Reading>, BoxReplyError> {
    let temperature = exchange_ledfan(handle, 0x53, 1)?;
    if !sensor_present(&temperature) { return Ok(None); }
    let speed = exchange_ledfan(handle, 0x51, 1)?;
    Ok(reading(&temperature, &speed))
}

/// Recover a box stuck in the `FE 33` refusal state. The vendor path bounded
/// this to once per 30 s; the caller owns that deadline.
#[cfg(any(windows, target_os = "linux"))]
fn query_recovering(
    handle: &crate::tf3::native::Handle,
    next_recovery: &mut Instant,
) -> Option<Reading> {
    match query(handle) {
        Ok(value) => value,
        Err(BoxReplyError::Rejected) if Instant::now() >= *next_recovery => {
            *next_recovery = Instant::now() + Duration::from_secs(30);
            #[cfg(target_os = "linux")]
            {
                // 232B carries TF2 on port 1. Without this initialization it
                // stays N/D for the whole session after a refusal (e.g. reboot).
                if crate::ledfanbox::initialize(handle) {
                    return query(handle).ok().flatten();
                }
            }
            None
        }
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const TEMPERATURE: [u8; 8] = [0x33, 0x53, 1, 0xff, 0xa3, 0, 0x7c, 1];
    const SPEED: [u8; 8] = [0x33, 0x51, 1, 0xff, 0x1b, 0x78, 1, 0];
    #[test]
    fn captured_tf2_frames_and_windows_report_id() {
        assert_eq!(reading(&TEMPERATURE, &SPEED).unwrap().flow_lph, 210.0);
        let mut temperature = vec![0]; temperature.extend(TEMPERATURE);
        let mut speed = vec![0]; speed.extend(SPEED);
        assert_eq!(reading(&temperature, &speed).unwrap().flow_lph, 210.0);
    }
    #[test]
    fn absent_sensor_wrong_port_and_truncated_frames_are_not_flow() {
        let missing = [0x33, 0x53, 1, 0xff, 0x10, 0x27, 0x10, 0x27];
        assert!(reading(&missing, &SPEED).is_none());
        for length in 0..8 {
            assert!(reading(&TEMPERATURE[..length], &SPEED).is_none());
            assert!(reading(&TEMPERATURE, &SPEED[..length]).is_none());
        }
        let mut wrong = SPEED; wrong[2] = 3;
        assert!(reading(&TEMPERATURE, &wrong).is_none());
    }
    #[test]
    fn vendor_conversion_and_zero_flow() {
        assert_eq!(flow_lph(376), 210.0);
        assert_eq!(flow_lph(300), 177.0);
        assert_eq!(flow_lph(0), 0.0);
    }
    #[test]
    fn expired_reading_is_not_current() {
        let sample = Reading { flow_lph: 210.0, sampled_at: Instant::now() };
        assert!(fresh(Some(sample), sample.sampled_at + Duration::from_secs(5)).is_some());
        assert!(fresh(Some(sample), sample.sampled_at + Duration::from_secs(6)).is_none());
    }
}

// SignalRGB owns all USB reads while running. Its plugin publishes real samples
// through its existing log; missing/stale bridge data must remain unavailable.
#[cfg(windows)]
fn signalrgb_running() -> bool { unsafe {
    use windows_sys::Win32::{System::Diagnostics::ToolHelp::*, Foundation::*};
    let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
    if snapshot == INVALID_HANDLE_VALUE { return true; }
    let mut entry: PROCESSENTRY32W = std::mem::zeroed();
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut ok = Process32FirstW(snapshot, &mut entry);
    let mut found = false;
    while ok != 0 {
        let end = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
        if String::from_utf16_lossy(&entry.szExeFile[..end]).eq_ignore_ascii_case("SignalRgb.exe") { found = true; break; }
        ok = Process32NextW(snapshot, &mut entry);
    }
    CloseHandle(snapshot);
    found
} }
fn bridge_line(line: &str, now_ms: i64) -> Option<f32> {
    let json = line.split_once("STARGATE_TF2 ")?.1;
    let sample: serde_json::Value = serde_json::from_str(json).ok()?;
    let timestamp = sample.get("unix_ms")?.as_i64()?;
    let value = sample.get("flow_lph")?.as_f64()? as f32;
    if !(0..=5000).contains(&now_ms.checked_sub(timestamp)?) || !value.is_finite() || !(0.0..=35000.0).contains(&value) { return None; }
    Some(value)
}
#[cfg(windows)]
fn bridge_reading() -> Option<Reading> {
    use std::io::{Read, Seek, SeekFrom};
    let directory = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("WhirlwindFX/SignalRgb/Logs");
    let file = std::fs::read_dir(directory).ok()?.filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("SignalRGB_") && e.path().extension().is_some_and(|x|x=="log"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?,e.path())))
        .max_by_key(|e|e.0)?.1;
    let mut file = std::fs::File::open(file).ok()?;
    let size = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(size.saturating_sub(262144))).ok()?;
    let mut bytes = Vec::new(); file.take(262144).read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let now = chrono::Utc::now().timestamp_millis();
    text.lines().rev().find_map(|line| bridge_line(line, now))
        .map(|flow_lph| Reading { flow_lph, sampled_at: Instant::now() })
}
#[cfg(test)]
mod bridge_tests {
    use super::*;
    #[test] fn bridge_requires_real_current_sample() {
        let line = r#"log STARGATE_TF2 {"flow_lph":210,"unix_ms":10000}"#;
        assert_eq!(bridge_line(line,12000),Some(210.0));
        assert_eq!(bridge_line(line,16000),None);
        assert_eq!(bridge_line(line,9000),None);
        assert_eq!(bridge_line("bad",12000),None);
    }
}
