//! TF2 on the observed LEDFanBox (264a:232b), port 1. Sensor queries only.
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct Reading {
    pub flow_lph: f32,
    pub sampled_at: Instant,
}

fn fresh(reading: Option<Reading>, now: Instant) -> Option<Reading> {
    reading.filter(|r| now.saturating_duration_since(r.sampled_at) <= Duration::from_secs(5))
}

pub struct Reader {
    latest: Arc<Mutex<Option<Reading>>>,
    stop: mpsc::Sender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Reader {
    pub fn new() -> Self {
        let latest = Arc::new(Mutex::new(None));
        let output = latest.clone();
        let (stop, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new().name("tf2-telemetry".into()).spawn(move || {
            #[cfg(windows)]
            {
                let mut device = None;
                loop {
                    if receiver.try_recv().is_ok() { break; }
                    let started = Instant::now();
                    if signalrgb_running() {
                        device = None;
                        if let Ok(mut slot) = output.lock() { *slot = bridge_reading(); }
                        if receiver.recv_timeout(Duration::from_secs(1)) != Err(mpsc::RecvTimeoutError::Timeout) { break; }
                        continue;
                    }
                    if device.is_none() { device = crate::tf3::native::open_device(0x232b, 0xff00, 1, 65, 193); }
                    let reading = device.as_ref().and_then(query);
                    let interval = if reading.is_some() { Duration::from_secs(1) } else { Duration::from_secs(3) };
                    // Keep a real sample across a brief USB timeout; latest() still
                    // rejects it after five seconds, including while reconnecting.
                    if let Some(reading) = reading {
                        if let Ok(mut slot) = output.lock() { *slot = Some(reading); }
                    }
                    if reading.is_none() { device = None; }
                    let delay = interval.saturating_sub(started.elapsed());
                    if receiver.recv_timeout(delay) != Err(mpsc::RecvTimeoutError::Timeout) { break; }
                }
            }
            #[cfg(not(windows))]
            { let _ = output; let _ = receiver.recv(); }
        }).ok();
        Self { latest, stop, worker }
    }
    pub fn latest(&self) -> Option<Reading> {
        fresh(self.latest.lock().ok().and_then(|r| *r), Instant::now())
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        let _ = self.stop.send(());
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
    let p = if report.first() == Some(&0) { report.get(1..)? } else { report };
    if p.len() < 8 || p[..4] != [0x33, opcode, 1, 0xff] { return None; }
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

#[cfg(windows)]
fn query(handle: &crate::tf3::native::Handle) -> Option<Reading> {
    fn exchange(handle: &crate::tf3::native::Handle, opcode: u8) -> Option<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_millis(300);
        let mut command = [0u8; 193];
        command[..4].copy_from_slice(&[0, 0x33, opcode, 1]);
        if crate::tf3::native::transfer(handle, &mut command, true, 300)? != command.len() { return None; }
        for _ in 0..16 {
            let remaining = deadline.checked_duration_since(Instant::now())?;
            let mut report = [0u8; 65];
            let count = crate::tf3::native::transfer(handle, &mut report, false, remaining.as_millis().max(1) as u32)?;
            if payload(&report[..count], opcode).is_some() { return Some(report[..count].to_vec()); }
        }
        None
    }
    let temperature = exchange(handle, 0x53)?;
    if !sensor_present(&temperature) { return None; }
    let speed = exchange(handle, 0x51)?;
    reading(&temperature, &speed)
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
