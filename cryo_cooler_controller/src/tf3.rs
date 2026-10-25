//! Pacific TF3 telemetry and vendor connection handshake; no configuration commands.
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct Reading {
    pub coolant_c: f32,
    pub ambient_c: f32,
    pub pressure_bar: f32,
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
        let worker = std::thread::Builder::new().name("tf3-telemetry".into()).spawn(move || {
            #[cfg(windows)]
            {
                let mut device = None;
                loop {
                    if receiver.try_recv().is_ok() { break; }
                    if device.is_none() { device = native::open(); }
                    let reading = device.as_ref().and_then(|d| native::query(d));
                    let delay = if reading.is_some() { Duration::from_secs(1) } else { Duration::from_secs(3) };
                    if let Ok(mut slot) = output.lock() { *slot = reading; }
                    if reading.is_none() { device = None; }
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

#[derive(Default)]
struct Response { bytes: Vec<u8> }

// EasyPODx64.dll 0x180001810: connection challenge response, byte arithmetic.
fn connection_response(a: u8, b: u8) -> [u8; 2] {
    match ((a >> 6) & 2) + ((b >> 1) & 4) + (a & 1) {
        0 => [a | 0x5a, b & 0x69],
        1 => [a & 0xa5, b | 0x96],
        2 => [a ^ 0x41, b & 0x2b],
        3 => [a ^ 0x14, b | 0xb2],
        4 => [a | 0x3c, b ^ 0x7e],
        5 => [a & 0xc3, b ^ 0xe7],
        6 => [0xff - a, b.wrapping_sub(0x73)],
        _ => [a.wrapping_sub(0x28), 0xff - b],
    }
}
impl Response {
    fn push(&mut self, report: &[u8]) -> Option<Reading> {
        let count = *report.first()? as usize;
        if count == 0 || count > 63 || count + 1 > report.len() {
            self.bytes.clear(); return None;
        }
        self.bytes.extend_from_slice(&report[1..=count]);
        // Discard unrelated traffic, retaining a possible split header.
        while self.bytes.len() >= 2 && self.bytes[..2] != [0x33, 0x54] {
            self.bytes.remove(0);
        }
        if self.bytes.len() < 9 { return None; }
        let p = &self.bytes[..9];
        let coolant_c = i16::from_be_bytes([p[3], p[4]]) as f32 / 10.0;
        let ambient_c = i16::from_be_bytes([p[5], p[6]]) as f32 / 10.0;
        let pressure_bar = u16::from_be_bytes([p[7], p[8]]) as f32 / 100.0;
        self.bytes.clear();
        // Reject out-of-spec values; zero pressure is a valid observed reading.
        if !(0.0..=70.0).contains(&coolant_c) || !(-20.0..=80.0).contains(&ambient_c)
            || !(0.0..=1.0).contains(&pressure_bar) { return None; }
        Some(Reading { coolant_c, ambient_c, pressure_bar, sampled_at: Instant::now() })
    }
}

#[cfg(windows)]
pub(crate) mod native {
    use super::*;
    use std::{mem::{size_of, zeroed}, ptr::{null, null_mut}};
    use windows_sys::Win32::{
        Devices::{DeviceAndDriverInstallation::*, HumanInterfaceDevice::*},
        Foundation::*, Storage::FileSystem::*, System::{IO::*, Threading::*},
    };
    pub(crate) struct Handle(HANDLE);
    impl Drop for Handle { fn drop(&mut self) { unsafe { CloseHandle(self.0); } } }

    pub(super) fn open() -> Option<Handle> { open_matching(0x233b, None) }
    pub(crate) fn open_device(pid: u16, usage_page: u16, usage: u16, input: u16, output: u16) -> Option<Handle> {
        open_matching(pid, Some((usage_page, usage, input, output)))
    }
    fn open_matching(pid: u16, expected_caps: Option<(u16, u16, u16, u16)>) -> Option<Handle> { unsafe {
        let mut guid = zeroed();
        HidD_GetHidGuid(&mut guid);
        let list = SetupDiGetClassDevsW(&guid, null(), 0, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
        if list == INVALID_HANDLE_VALUE { return None; }
        struct List(isize);
        impl Drop for List { fn drop(&mut self) { unsafe { SetupDiDestroyDeviceInfoList(self.0); } } }
        let _list = List(list);
        for index in 0..256 {
            let mut interface: SP_DEVICE_INTERFACE_DATA = zeroed();
            interface.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
            if SetupDiEnumDeviceInterfaces(list, null(), &guid, index, &mut interface) == 0 { break; }
            let mut needed = 0;
            SetupDiGetDeviceInterfaceDetailW(list, &interface, null_mut(), 0, &mut needed, null_mut());
            if needed < 8 || needed > 65536 { continue; }
            // u64 allocation keeps the native detail structure correctly aligned.
            let mut storage = vec![0u64; (needed as usize + 7) / 8];
            let detail = storage.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(list, &interface, detail, needed, null_mut(), null_mut()) == 0 { continue; }
            let path = std::slice::from_raw_parts((*detail).DevicePath.as_ptr(), (needed as usize - 4) / 2);
            let end = path.iter().position(|&v| v == 0).unwrap_or(path.len());
            let name = String::from_utf16_lossy(&path[..end]).to_ascii_lowercase();
            if !name.contains(&format!("vid_264a&pid_{pid:04x}")) { continue; }
            let handle = CreateFileW((*detail).DevicePath.as_ptr(), GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE, null(), OPEN_EXISTING, FILE_FLAG_OVERLAPPED, 0);
            if handle != INVALID_HANDLE_VALUE {
                let handle = Handle(handle);
                if let Some(expected) = expected_caps {
                    let mut preparsed = 0;
                    if HidD_GetPreparsedData(handle.0, &mut preparsed) == 0 { continue; }
                    let mut caps: HIDP_CAPS = zeroed();
                    let status = HidP_GetCaps(preparsed, &mut caps);
                    HidD_FreePreparsedData(preparsed);
                    if status == 0x110000 && (caps.UsagePage, caps.Usage, caps.InputReportByteLength, caps.OutputReportByteLength) == expected {
                        return Some(handle);
                    }
                } else if initialize(&handle) { return Some(handle); }
            }
        }
        None
    } }

    // Reproduce EasyPOD ConnectPOD's feature handshake before normal sensor reads.
    // This does not write serial numbers, pressure calibration, or device settings.
    fn initialize(handle: &Handle) -> bool { unsafe {
        use chrono::{Datelike, Timelike};
        let mut preparsed = 0;
        if HidD_GetPreparsedData(handle.0, &mut preparsed) == 0 { return false; }
        let mut caps: HIDP_CAPS = zeroed();
        let status = HidP_GetCaps(preparsed, &mut caps);
        HidD_FreePreparsedData(preparsed);
        let length = caps.FeatureReportByteLength as usize;
        if status != 0x110000 || caps.UsagePage != 12 || caps.Usage != 1
            || !(4..=4096).contains(&length) { return false; }
        let now = chrono::Utc::now();
        let b = now.timestamp_subsec_millis() as u8;
        let a = [now.year() as u8, now.month() as u8,
            now.weekday().num_days_from_sunday() as u8, now.day() as u8,
            now.hour() as u8, now.minute() as u8, now.second() as u8, b]
            .into_iter().fold(0u8, u8::wrapping_add);
        let mut report = vec![0u8; length];
        report[..4].copy_from_slice(&[0x40, 0x55, a, b]);
        if HidD_SetFeature(handle.0, report.as_ptr().cast(), length as u32) == 0 { return false; }
        report.fill(0);
        report[0] = 0x40;
        if HidD_GetFeature(handle.0, report.as_mut_ptr().cast(), length as u32) == 0 { return false; }
        report[0] == 0x40 && report[1..3] == connection_response(a, b)
    } }

    // Each pending operation owns its event/buffer until cancellation is drained.
    pub(crate) fn transfer(handle: &Handle, buffer: &mut [u8], write: bool, timeout: u32) -> Option<usize> { unsafe {
        let event = CreateEventW(null(), 1, 0, null());
        if event == 0 { return None; }
        let event = Handle(event);
        let mut overlapped: OVERLAPPED = zeroed();
        overlapped.hEvent = event.0;
        let mut transferred = 0;
        let ok = if write {
            WriteFile(handle.0, buffer.as_ptr(), buffer.len() as u32, &mut transferred, &mut overlapped)
        } else {
            ReadFile(handle.0, buffer.as_mut_ptr().cast(), buffer.len() as u32, &mut transferred, &mut overlapped)
        };
        if ok != 0 { return Some(transferred as usize); }
        if GetLastError() != ERROR_IO_PENDING { return None; }
        if WaitForSingleObject(event.0, timeout) != WAIT_OBJECT_0 {
            CancelIoEx(handle.0, &overlapped);
            // Windows requires completion before releasing OVERLAPPED and its buffer.
            GetOverlappedResult(handle.0, &overlapped, &mut transferred, 1);
            return None;
        }
        if GetOverlappedResult(handle.0, &overlapped, &mut transferred, 0) == 0 { None }
        else { Some(transferred as usize) }
    } }
    pub(super) fn query(handle: &Handle) -> Option<Reading> {
        let deadline = Instant::now() + Duration::from_millis(300);
        let mut command = [0u8; 64];
        command[..3].copy_from_slice(&[2, 0x33, 0x54]);
        if transfer(handle, &mut command, true, 300)? != 64 { return None; }
        let mut response = Response::default();
        for _ in 0..32 {
            let remaining = deadline.checked_duration_since(Instant::now())?;
            let mut report = [0u8; 64];
            let count = transfer(handle, &mut report, false, remaining.as_millis().max(1) as u32)?;
            if let Some(reading) = response.push(&report[..count]) { return Some(reading); }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vendor_connection_challenge_all_branches() {
        for (a, b, expected) in [
            (0, 0, [0x5a, 0]), (1, 0, [1, 0x96]),
            (128, 0, [0xc1, 0]), (129, 0, [0x95, 0xb2]),
            (0, 8, [0x3c, 0x76]), (1, 8, [1, 0xef]),
            (128, 8, [0x7f, 0x95]), (129, 8, [0x59, 0xf7]),
        ] { assert_eq!(connection_response(a, b), expected); }
    }
    #[test]
    fn live_fragmented_telemetry_and_zero_pressure() {
        let mut r = Response::default();
        assert!(r.push(&[1, 0x33]).is_none());
        assert!(r.push(&[1, 0x54]).is_none());
        let r = r.push(&[7, 0xfc, 1, 0x2b, 0, 0xf7, 0, 0]).unwrap();
        assert_eq!((r.coolant_c, r.ambient_c, r.pressure_bar), (29.9, 24.7, 0.0));
    }
    #[test]
    fn malformed_and_out_of_spec_are_not_live_values() {
        let mut r = Response::default();
        assert!(r.push(&[64, 0x33]).is_none());
        assert!(r.push(&[9, 0x33, 0x54, 0, 0xff, 0xff, 0, 250, 0, 0]).is_none());
        assert!(r.push(&[9, 0x33, 0x54, 0, 1, 0x2b, 0, 250, 0, 101]).is_none());
    }
    #[test]
    fn expired_sample_is_not_reported() {
        let sample = Reading { coolant_c: 30.0, ambient_c: 25.0, pressure_bar: 0.0, sampled_at: Instant::now() };
        assert!(fresh(Some(sample), sample.sampled_at + Duration::from_secs(5)).is_some());
        assert!(fresh(Some(sample), sample.sampled_at + Duration::from_secs(6)).is_none());
        assert!(fresh(None, Instant::now()).is_none());
    }
}
