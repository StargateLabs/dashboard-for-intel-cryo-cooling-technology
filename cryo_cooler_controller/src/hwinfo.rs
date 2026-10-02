//! Lettura sensori cross-platform.
//!
//! Windows: HWiNFO64 / AIDA64 tramite shared memory
//! Linux: lm-sensors via /sys/class/hwmon e /sys/class/fan
//!
//! # HWiNFO64 (Windows)
//! Abilitare: Impostazioni → Generale → Supporto Shared Memory
//! Shared memory: `Global\HWiNFO_SENS_SM2`
//!
//! # AIDA64 (Windows)
//! Abilitare: File → Preferenze → Hardware Monitoring → External Applications
//!            → Enable Shared Memory
//! Shared memory: `Global\AIDA64_SensorValues`  (formato XML)

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SensorType {
    Temperature,
    Voltage,
    Fan,
    Current,
    Power,
    Clock,
    Load,
    #[allow(dead_code)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SensorSource {
    HWiNFO,
    AIDA64,
    #[cfg(target_os = "linux")]
    HWMON,
}

impl std::fmt::Display for SensorSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SensorSource::HWiNFO => write!(f, "HWiNFO64"),
            SensorSource::AIDA64 => write!(f, "AIDA64"),
            #[cfg(target_os = "linux")]
            SensorSource::HWMON => write!(f, "lm-sensors"),
        }
    }
}

impl Default for SensorSource {
    fn default() -> Self {
        #[cfg(target_os = "windows")]
        {
            SensorSource::HWiNFO
        }
        #[cfg(target_os = "linux")]
        {
            SensorSource::HWMON
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            SensorSource::HWiNFO
        }
    }
}

#[derive(Debug, Clone)]
pub struct SensorReading {
    pub source:       SensorSource,
    pub sensor_type:  SensorType,
    pub sensor_name:  String,
    pub reading_name: String,
    /// ID originale (solo AIDA64) — usato per scoring CPU temp
    pub sensor_id:    String,
    pub value:        f32,
    pub unit:         String,
    pub min:          f32,
    pub max:          f32,
}

/// Confronta due stringhe ASCII senza allocare (case-insensitive).
#[inline]
pub fn ci_contains(haystack: &str, needle: &str) -> bool {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || n.len() > h.len() { return false; }
    h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
}

#[inline]
fn ci_starts_with(haystack: &str, needle: &str) -> bool {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.len() > h.len() { return false; }
    h[..n.len()].eq_ignore_ascii_case(n)
}

impl SensorReading {
    pub fn category(&self) -> &'static str {
        let n = self.reading_name.as_str();
        let s = self.sensor_name.as_str();
        let id = self.sensor_id.as_str();

        // AIDA64: usa anche l'ID per classificare (es. TCPU, TGPU1DIO)
        if ci_starts_with(id, "T") {
            if ci_contains(id, "CPU") || ci_contains(n, "cpu") { return "CPU"; }
            if ci_contains(id, "GPU") || ci_contains(n, "gpu") { return "GPU"; }
            if ci_contains(id, "MB")  || ci_contains(id, "MOB") { return "Scheda Madre"; }
        }

        if ci_contains(n, "cpu") || ci_contains(n, "core") || ci_contains(n, "tdie")
            || ci_contains(n, "tctl") || ci_contains(s, "cpu") || ci_contains(s, "processor") {
            "CPU"
        } else if ci_contains(n, "gpu") || ci_contains(n, "graphic") || ci_contains(s, "gpu")
            || ci_contains(s, "rtx") || ci_contains(s, "gtx") || ci_contains(s, "radeon")
            || ci_contains(s, "geforce") {
            "GPU"
        } else if ci_contains(n, "vrm") || ci_contains(n, "chipset") || ci_contains(n, "pch")
            || ci_contains(n, "mosfet") || ci_contains(s, "motherboard") || ci_contains(s, "system") {
            "Scheda Madre"
        } else if ci_contains(n, "nvme") || ci_contains(n, "ssd") || ci_contains(n, "hdd")
            || ci_contains(s, "disk") || ci_contains(s, "storage") || ci_contains(s, "drive") {
            "Storage"
        } else if matches!(self.sensor_type, SensorType::Fan) {
            "Ventole"
        } else {
            "Altro"
        }
    }
}

#[derive(Debug, Clone)]
pub struct SourceStatus {
    pub hwinfo_available: bool,
    pub aida64_available: bool,
}

// ── Platform Windows ──────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    fn parse_str_nul(buf: &[u8]) -> String {
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).to_string()
    }

    // `VirtualQuery` non è esposto da windows-sys 0.48, ma il simbolo è in
    // kernel32. Lo dichiariamo noi per poter sapere QUANTO è grande la
    // shared memory realmente mappata (necessario per validare gli offset).
    #[link(name = "kernel32")]
    extern "system" {
        fn VirtualQuery(
            lp_address: *const core::ffi::c_void,
            lp_buffer: *mut windows_sys::Win32::System::Memory::MEMORY_BASIC_INFORMATION,
            dw_length: usize,
        ) -> usize;
    }

    use windows_sys::Win32::System::Memory::MEMORY_BASIC_INFORMATION;

    /// Byte utilizzabili a partire da `base` nella regione mappata.
    /// Restituisce 0 se la query fallisce.
    fn mapped_bytes_from(base: *const u8) -> usize {
        let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let ok = unsafe {
            VirtualQuery(
                base as *const core::ffi::c_void,
                &mut mbi,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if ok == 0 { return 0; }
        // Se `base` non è l'inizio della regione, contiamo da lì.
        let region_start = mbi.BaseAddress as *const u8 as usize;
        let region_size  = mbi.RegionSize;
        let here         = base as usize;
        if here < region_start { return 0; }
        region_start.checked_add(region_size)
            .and_then(|end| end.checked_sub(here))
            .unwrap_or(0)
    }

    /// Verifica che l'intervallo `[offset, offset + count*stride)` stia dentro
    /// `avail` byte. Tutti i calcoli sono su `usize`/`u64` per evitare overflow.
    fn span_fits(offset: u32, count: u32, stride: u32, avail: usize) -> bool {
        if count == 0 { return true; }
        let end = (offset as u64) + (count as u64) * (stride as u64);
        end <= (avail as u64)
    }

    // ── HWiNFO64 ─────────────────────────────────────────────────────────────

    const HWINFO_SM: &str = "Global\\HWiNFO_SENS_SM2";

    /// Byte minimi dell'header HWiNFO: sei `u32` agli offset 16, 20, 24, 28,
    /// 32 e 36, quindi l'ultimo legge fino al byte 40 escluso.
    const HW_HEADER_BYTES: usize = 40;

    fn hw_sensor_type(t: u32) -> Option<SensorType> {
        match t {
            1 => Some(SensorType::Temperature),
            2 => Some(SensorType::Voltage),
            3 => Some(SensorType::Fan),
            4 => Some(SensorType::Current),
            5 => Some(SensorType::Power),
            6 => Some(SensorType::Clock),
            7 => Some(SensorType::Load),
            _ => None,
        }
    }

    fn hw_unit(t: u32) -> &'static str {
        match t { 1=>"°C", 2=>"V", 3=>"RPM", 4=>"A", 5=>"W", 6=>"MHz", 7=>"%", _=>"" }
    }

    pub fn read_hwinfo() -> Vec<SensorReading> {
        unsafe {
            use windows_sys::Win32::System::Memory::*;
            use windows_sys::Win32::Foundation::CloseHandle;

            let h = OpenFileMappingW(FILE_MAP_READ, 0, wide(HWINFO_SM).as_ptr());
            if h == 0 { return Vec::new(); }
            // SIZE = 0 → mappa l'intera dimensione del file
            let view = MapViewOfFile(h, FILE_MAP_READ, 0, 0, 0);
            if view == 0 { CloseHandle(h); return Vec::new(); }

            let base = view as *const u8;
            // Dimensione reale della mappa: senza questa non si può validare
            // nulla e un header corrotto fa leggere fuori dai limiti (UB).
            let avail = mapped_bytes_from(base);

            // GUARDIA DI DIMENSIONE, PRIMA DI TOCCARE `base`.
            // `avail == 0` → VirtualQuery fallito, indirizzo non interrogabile.
            // Una mappa legittima può comunque essere più piccola dell'header.
            // In entrambi i casi leggere 16..40 sarebbe uscire dalla regione
            // mappata. `UnmapViewOfFile` e `CloseHandle` sono abbinati al
            // return: ogni uscita anticipata chiude tutto.
            if avail < HW_HEADER_BYTES {
                UnmapViewOfFile(view); CloseHandle(h);
                return Vec::new();
            }
            // La guardia è già passata: da qui in poi l'header è leggibile e la
            // validazione sotto non ha più bisogno di ripeterne il controllo.
            debug_assert!(avail >= HW_HEADER_BYTES, "guardia di dimensione già eseguita");

            let results = read_hwinfo_view(base, avail);

            UnmapViewOfFile(view); CloseHandle(h);
            results
        }
    }

    /// Lettura e parse di una mappa HWiNFO già aperta, con la dimensione reale
    /// già calcolata da `mapped_bytes_from`.
    ///
    /// Staccata da `read_hwinfo` solo per rendere la guardia provabile senza
    /// shared memory: qui `avail` è un parametro contratto, non ricalcolabile,
    /// e questa funzione non legge MAI oltre `base + avail`.
    unsafe fn read_hwinfo_view(base: *const u8, avail: usize) -> Vec<SensorReading> {
        // GUARDIA PRIMA DI QUALSIASI LETTURA (vedi `read_hwinfo`): le sei
        // `read_unaligned` qui sotto arrivano fino al byte 40, quindi un
        // `avail` inferiore a `HW_HEADER_BYTES` le manderebbe fuori mappa.
        if avail < HW_HEADER_BYTES {
            return Vec::new();
        }

        let sensor_offset:  u32 = std::ptr::read_unaligned(base.add(16) as *const u32);
        let sensor_size:    u32 = std::ptr::read_unaligned(base.add(20) as *const u32);
        let sensor_count:   u32 = std::ptr::read_unaligned(base.add(24) as *const u32);
        let reading_offset: u32 = std::ptr::read_unaligned(base.add(28) as *const u32);
        let reading_size:   u32 = std::ptr::read_unaligned(base.add(32) as *const u32);
        let reading_count:  u32 = std::ptr::read_unaligned(base.add(36) as *const u32);

        // Header minimo: 40 byte di header + almeno 1 record.
        //
        // NB: le copie leggono 128 byte a partire da `base+8`, quindi la
        // dimensione del record deve essere >= 136, non >= 128: era un
        // off-by-8 che permetteva di leggere 8 byte nel record successivo.
        const MIN_REC: u32 = 136;
        const MAX_RECORDS: u32 = 20_000;

        // Sul header non serve più `header_ok`: la guardia in testa l'ha già
        // garantito. Restano i controlli sui record e sugli offset, che
        // `span_fits` confronta con `avail`.
        let sensors_ok = sensor_size >= MIN_REC
            && sensor_count <= MAX_RECORDS
            && span_fits(sensor_offset, sensor_count, sensor_size, avail);
        let readings_ok = reading_size >= MIN_REC
            && reading_count <= MAX_RECORDS
            && span_fits(reading_offset, reading_count, reading_size, avail);

        if !readings_ok {
            return Vec::new();
        }

        let mut sensor_names: Vec<String> = Vec::new();
        if sensors_ok {
            for i in 0..(sensor_count as usize) {
                let s = base.add(sensor_offset as usize + i * sensor_size as usize);
                let mut nb = [0u8; 128];
                std::ptr::copy_nonoverlapping(s.add(8), nb.as_mut_ptr(), 128);
                sensor_names.push(parse_str_nul(&nb));
            }
        }

        let mut results = Vec::new();
        for i in 0..(reading_count as usize) {
            let e = base.add(reading_offset as usize + i * reading_size as usize);
            let rtype: u32 = std::ptr::read_unaligned(e as *const u32);
            let sidx:  u32 = std::ptr::read_unaligned(e.add(4) as *const u32);
            let stype = match hw_sensor_type(rtype) { Some(t) => t, None => continue };

            let mut nb = [0u8; 128];
            std::ptr::copy_nonoverlapping(e.add(8), nb.as_mut_ptr(), 128);
            let reading_name = parse_str_nul(&nb);
            if reading_name.is_empty() { continue; }

            let value: f64 = std::ptr::read_unaligned(e.add(40) as *const f64);
            let min:   f64 = std::ptr::read_unaligned(e.add(48) as *const f64);
            let max:   f64 = std::ptr::read_unaligned(e.add(56) as *const f64);
            if !value.is_finite() { continue; }

            results.push(SensorReading {
                source:       SensorSource::HWiNFO,
                sensor_type:  stype,
                sensor_name:  sensor_names.get(sidx as usize).cloned().unwrap_or_default(),
                reading_name,
                sensor_id:    String::new(), // HWiNFO non usa ID stringa
                value: value as f32,
                unit:  hw_unit(rtype).to_owned(),
                min:   min as f32,
                max:   max as f32,
            });
        }

        // Niente unmap/CloseHandle qui: la vista e l'handle sono di
        // `read_hwinfo`, che li chiude su ogni uscita dopo questa chiamata.
        results
    }

    // ── AIDA64 ───────────────────────────────────────────────────────────────
    // XML format: <aida64><temp><id>TCPU</id><label>CPU</label><value>72</value></temp>...</aida64>
    // Tag XML per tipo: temp, fan, volt, pwr, clk, sys (load/utilization)
    // Prefix ID per tipo: T=temp, F=fan, V=volt, P=pwr, C=clk, S=sys/load

    const AIDA64_SM:      &str   = "Global\\AIDA64_SensorValues";
    // FIX: SIZE=0 per mappare l'intera shared memory — evita troncamento XML
    const AIDA64_SM_SIZE: usize  = 0;
    const AIDA64_MAX_LEN: usize  = 0x100000; // 1MB limite di sicurezza scan

    pub fn read_aida64() -> Vec<SensorReading> {
        unsafe {
            use windows_sys::Win32::System::Memory::*;
            use windows_sys::Win32::Foundation::CloseHandle;

            let h = OpenFileMappingW(FILE_MAP_READ, 0, wide(AIDA64_SM).as_ptr());
            if h == 0 { return Vec::new(); }

            // FIX BUG 1: SIZE=0 mappa tutta la shared memory
            let view = MapViewOfFile(h, FILE_MAP_READ, 0, 0, AIDA64_SM_SIZE);
            if view == 0 { CloseHandle(h); return Vec::new(); }

            let base = view as *const u8;

            // Il limite di scansione non può essere un numero fisso: la shared
            // memory potrebbe essere più piccola di 1 MB e il byte-per-byte
            // finiva oltre la regione mappata (access violation). Usiamo la
            // dimensione reale, con 1 MB solo come tetto di sicurezza.
            let avail = mapped_bytes_from(base);
            if avail == 0 {
                UnmapViewOfFile(view); CloseHandle(h);
                return Vec::new();
            }
            let limit = avail.min(AIDA64_MAX_LEN);

            let mut len = 0usize;
            while len < limit {
                let b = *base.add(len);
                if b == 0 { break; }
                len += 1;
            }

            // Leggi bytes e converti
            let xml_bytes = std::slice::from_raw_parts(base, len);
            // FIX BUG 3: AIDA64 può usare Windows-1252 — rimpiazza byte non-UTF8
            let xml = String::from_utf8_lossy(xml_bytes).to_string();

            UnmapViewOfFile(view);
            CloseHandle(h);

            parse_aida64_xml(&xml)
        }
    }

    fn parse_aida64_xml(xml: &str) -> Vec<SensorReading> {
        let mut results = Vec::new();
        if xml.is_empty() { return results; }

        // AIDA64 usa questi tag di primo livello per i tipi di sensore
        // FIX: aggiunti anche i tag alternativi usati da versioni diverse di AIDA64
        let tag_list = [
            ("temp",  true),   // temperature — tag primario
            ("fan",   true),   // ventole RPM
            ("volt",  true),   // tensioni
            ("pwr",   true),   // potenza W
            ("clk",   true),   // frequenze MHz
            ("sys",   false),  // sistema: load, utilizzo, altri
            ("duty",  false),  // duty cycle ventole
        ];

        for (tag, _is_numeric) in &tag_list {
            let open  = format!("<{}>",  tag);
            let close = format!("</{}>", tag);
            let mut pos = 0;

            loop {
                let remaining = &xml[pos..];
                let start_rel = match remaining.find(&open) {
                    Some(i) => i,
                    None    => break,
                };
                let content_start = pos + start_rel + open.len();
                let after_open    = &xml[content_start..];
                let end_rel = match after_open.find(&close) {
                    Some(i) => i,
                    None    => break,
                };

                let block = &xml[content_start..content_start + end_rel];
                if let Some(r) = parse_aida64_block(block, tag) {
                    results.push(r);
                }
                pos = content_start + end_rel + close.len();
            }
        }
        results
    }

    fn xml_inner<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
        let open  = format!("<{}>",  tag);
        let close = format!("</{}>", tag);
        let s = xml.find(&open)? + open.len();
        let e = xml[s..].find(&close)?;
        Some(xml[s..s + e].trim())
    }

    fn parse_aida64_block(block: &str, tag: &str) -> Option<SensorReading> {
        let id    = xml_inner(block, "id")?;
        let label = xml_inner(block, "label")?;
        let val_s = xml_inner(block, "value")?;

        // FIX: gestisci valori vuoti o non numerici
        if val_s.is_empty() { return None; }
        let value: f32 = val_s.parse().ok()?;

        let (sensor_type, unit, sensor_name) = aida64_classify(id, tag, label);

        // Sanity check temperature
        if sensor_type == SensorType::Temperature && (value < -80.0 || value > 200.0) {
            return None;
        }

        Some(SensorReading {
            source:       SensorSource::AIDA64,
            sensor_type,
            sensor_name,
            reading_name: label.to_owned(),
            sensor_id:    id.to_owned(),
            value,
            unit,
            min: 0.0,
            max: 0.0,
        })
    }

    fn aida64_classify(id: &str, tag: &str, label: &str) -> (SensorType, String, String) {
        let id_up = id.to_uppercase();
        let lbl   = label.to_lowercase();

        // FIX: classificazione per PREFISSO ID con precedenza sul tag XML
        // I prefissi ID di AIDA64 sono definitivi e non ambigui
        let sensor_type = if id_up.starts_with('T') || tag == "temp" {
            SensorType::Temperature
        } else if id_up.starts_with('F') || tag == "fan" {
            SensorType::Fan
        } else if id_up.starts_with('V') || tag == "volt" {
            SensorType::Voltage
        } else if id_up.starts_with('P') || tag == "pwr" {
            SensorType::Power
        } else if id_up.starts_with('C') || tag == "clk" {
            SensorType::Clock
        } else if id_up.starts_with('D') || tag == "duty" {
            SensorType::Load
        } else {
            // 'S' prefix = system/utilization/load
            SensorType::Load
        };

        let unit = match sensor_type {
            SensorType::Temperature => "°C",
            SensorType::Fan         => "RPM",
            SensorType::Voltage     => "V",
            SensorType::Power       => "W",
            SensorType::Clock       => "MHz",
            SensorType::Load        => "%",
            _                       => "",
        }.to_owned();

        // FIX: usa ID prefix per classificare componente in modo più affidabile
        let component = if id_up.contains("CPU") || lbl.contains("cpu") {
            "CPU"
        } else if id_up.contains("GPU") || lbl.contains("gpu") || lbl.contains("graphic") {
            "GPU"
        } else if id_up.contains("MB") || id_up.contains("MOB") || id_up.contains("PCH")
            || lbl.contains("motherboard") || lbl.contains("system") {
            "Scheda Madre"
        } else if lbl.contains("disk") || lbl.contains("nvme") || lbl.contains("ssd") {
            "Storage"
        } else {
            "Sistema"
        };

        (sensor_type, unit, component.to_owned())
    }

    // ── Check disponibilità sorgenti ──────────────────────────────────────────

    pub fn check_sources() -> SourceStatus {
        unsafe {
            use windows_sys::Win32::System::Memory::{OpenFileMappingW, FILE_MAP_READ};
            use windows_sys::Win32::Foundation::CloseHandle;

            let hw = OpenFileMappingW(FILE_MAP_READ, 0, wide(HWINFO_SM).as_ptr());
            let hw_ok = hw != 0;
            if hw_ok { CloseHandle(hw); }

            let ai = OpenFileMappingW(FILE_MAP_READ, 0, wide(AIDA64_SM).as_ptr());
            let ai_ok = ai != 0;
            if ai_ok { CloseHandle(ai); }

            SourceStatus { hwinfo_available: hw_ok, aida64_available: ai_ok }
        }
    }

    pub fn read_all_sensors(pref: &super::SensorSource) -> Vec<SensorReading> {
        match pref {
            super::SensorSource::HWiNFO => read_hwinfo(),
            super::SensorSource::AIDA64 => read_aida64(),
        }
    }

    pub fn read_cpu_temperature(pref: &super::SensorSource) -> Option<f32> {
        let sensors = read_all_sensors(pref);
        best_cpu_temp(&sensors)
    }

    /// FIX PRINCIPALE: scoring CPU temp con supporto ID AIDA64 esplicito.
    /// AIDA64 usa ID come TCPU, TCPUDIES — molto più affidabili dei label.
    fn best_cpu_temp(sensors: &[SensorReading]) -> Option<f32> {
        let mut best: Option<(i32, f32)> = None;

        for s in sensors {
            if s.sensor_type != SensorType::Temperature { continue; }
            if s.value < -60.0 || s.value > 200.0      { continue; }

            let n  = s.reading_name.to_lowercase();
            let id = s.sensor_id.to_uppercase();

            // FIX: score basato su ID AIDA64 (se disponibile) con priorità massima
            let score: i32 = if id == "TCPU" {
                // AIDA64: TCPU = temperatura CPU principale
                75
            } else if id.starts_with("TCPU") && !id.contains("GPU") {
                // AIDA64: TCPUDIE, TCPUDIES, TCPUCORE ecc.
                105
            } else if n.contains("cpu package") {
                // HWiNFO: CPU Package
                100
            } else if n.contains("tdie") {
                95
            } else if n.contains("tctl") {
                90
            } else if n.contains("cpu die") {
                80
            } else if n.contains("core (average)") || n == "cpu" {
                // AIDA64: label "CPU" è la temp principale
                75
            } else if n.contains("cpu") && n.contains("temp") {
                50
            } else if n.contains("cpu") {
                40
            } else {
                continue
            };

            if best.as_ref().map_or(true, |b| score > b.0 || (score == b.0 && s.value > b.1)) {
                best = Some((score, s.value));
            }
        }
        best.map(|(_, v)| v)
    }

    // ── Test: guardia di dimensione della mappa HWiNFO ────────────────────────

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn cpu_die_wins_over_generic_cpu_and_hottest_core_breaks_ties() {
            let readings = [("TCPU",54.0),("TCPUDIE",80.0),("TCPUCORE1",90.0)]
                .into_iter().map(|(id,value)| SensorReading {
                    source: super::super::SensorSource::AIDA64,
                    sensor_type: SensorType::Temperature, sensor_name: "CPU".into(),
                    reading_name: "CPU".into(), sensor_id:id.into(), value,
                    unit:"C".into(), min:value, max:value,
                }).collect::<Vec<_>>();
            assert_eq!(best_cpu_temp(&readings),Some(90.0));
        }
        use windows_sys::Win32::System::Memory::{
            VirtualAlloc, VirtualFree, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE,
            PAGE_NOACCESS, PAGE_READWRITE,
        };

        /// Page size del sistema, ricavato da `VirtualQuery` su
        /// un'allocazione di 1 byte: `RegionSize` viene arrotondato alla
        /// pagina, quindi riporta il page size. Serve `GetSystemInfo`, che
        /// windows-sys 0.48 non espone senza una feature che non abbiamo.
        fn page_size() -> usize {
            let probe = unsafe {
                VirtualAlloc(std::ptr::null_mut(), 1, MEM_RESERVE | MEM_COMMIT, PAGE_NOACCESS)
            };
            assert!(!probe.is_null(), "VirtualAlloc di 1 byte fallito");

            let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
            let ok = unsafe {
                VirtualQuery(
                    probe,
                    &mut mbi,
                    std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            };
            unsafe { VirtualFree(probe, 0, MEM_RELEASE) };

            assert!(ok != 0, "VirtualQuery fallito");
            assert!(
                mbi.RegionSize >= 4096 && mbi.RegionSize.is_power_of_two(),
                "page size implausibile: {}",
                mbi.RegionSize
            );
            mbi.RegionSize
        }

        /// `len` byte leggibili con una pagina di guardia subito dopo.
        ///
        /// `VirtualAlloc` due pagine NOACCESS, poi committa in RW solo la
        /// prima; i `len` byte utili stanno in coda alla prima pagina, quindi
        /// `base[len]` cade nella seconda e la sua lettura termina il processo
        /// con STATUS_ACCESS_VIOLATION. Serve a verificare che il lettore non
        /// tocchi nulla oltre `avail`: un assert sul valore restituito NON
        /// basterebbe, perché la validazione dei record scarterebbe comunque
        /// un mapping piccolo, restituendo vuoto anche col codice rotto.
        struct GuardedView {
            region: *mut u8,
            base:   *const u8,
            len:    usize,
        }

        impl GuardedView {
            fn new(len: usize) -> Self {
                let page = page_size();
                assert!(len <= page, "il test vuole un mapping piccolo");

                let region = unsafe {
                    VirtualAlloc(
                        std::ptr::null_mut(),
                        page * 2,
                        MEM_RESERVE | MEM_COMMIT,
                        PAGE_NOACCESS,
                    )
                } as *mut u8;
                assert!(!region.is_null(), "VirtualAlloc di 2 pagine fallito");

                // `VirtualAlloc` senza hint allinea sempre al page size, ma
                // non lo diamo per scontato: se la prima pagina non coincide
                // con l'inizio della regione la guardia non coprirebbe i dati.
                let head = unsafe {
                    VirtualAlloc(region as *mut _, page, MEM_COMMIT, PAGE_READWRITE)
                };
                assert!(
                    head as *mut u8 == region,
                    "prima pagina non allineata all'inizio della regione"
                );

                GuardedView { region, base: unsafe { region.add(page - len) }, len }
            }
        }

        impl Drop for GuardedView {
            fn drop(&mut self) {
                // MEM_RELEASE vuole l'inizio dell'allocazione, non `base`.
                unsafe { VirtualFree(self.region as *mut _, 0, MEM_RELEASE) };
            }
        }

        /// Regressione: l'header (offset 16..40) veniva letto PRIMA del
        /// controllo sulla dimensione della mappa, quindi sei
        /// `read_unaligned` potevano uscire dalla regione mappata.
        ///
        /// Con il codice rotto questo test non fallisce con un assert ma
        /// CRASHA il processo (la terza lettura, offset 24, è già nella pagina
        /// di guardia). È il motivo della pagina NOACCESS invece di un buffer
        /// heap: la guardia va provata lì dove può fare del male.
        #[test]
        fn mappa_piu_piccola_di_40_byte_non_produce_sensori() {
            let v = GuardedView::new(HW_HEADER_BYTES - 16);
            let out = unsafe { read_hwinfo_view(v.base, v.len) };
            assert!(
                out.is_empty(),
                "{} sensori da una mappa di {} byte",
                out.len(),
                v.len
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Linux: lm-sensors via /sys/class/hwmon e /sys/class/fan
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_os = "linux")]
mod platform_linux {
    use super::*;
    use std::fs;

    const HWMON_PATH: &str = "/sys/class/hwmon";
    const FAN_PATH: &str = "/sys/class/fan";

    pub fn read_hwmon_sensors() -> Vec<SensorReading> {
        let mut results = Vec::new();

        // Leggi HWMonitor temperature
        if let Ok(entries) = fs::read_dir(HWMON_PATH) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if !path.is_dir() { continue; }

                let name = path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");

                // Skip "hwmon0" (pseudo-device)
                if name == "hwmon0" { continue; }

                // Leggi nome dispositivo
                let dev_name = fs::read_to_string(path.join("name"))
                    .unwrap_or_else(|_| String::new())
                    .trim()
                    .to_string();

                // Leggi temperature
                for i in 1..=20 {
                    let temp_input = path.join(format!("temp{i}_input"));
                    if let Ok(content) = fs::read_to_string(&temp_input) {
                        if let Ok(millideg) = content.trim().parse::<i32>() {
                            let temp_c = millideg as f32 / 1000.0;
                            if temp_c > -80.0 && temp_c < 200.0 {
                                let label_path = path.join(format!("temp{i}_label"));
                                let label = fs::read_to_string(&label_path)
                                    .unwrap_or_else(|_| String::new())
                                    .trim()
                                    .to_string();

                                let reading_name = if !label.is_empty() {
                                    label.clone()
                                } else {
                                    format!("temp{}", i)
                                };

                                let sensor_name = classify_hwmon_temp(&dev_name, &reading_name, i);

                                results.push(SensorReading {
                                    source: SensorSource::HWMON,
                                    sensor_type: SensorType::Temperature,
                                    sensor_name,
                                    reading_name,
                                    sensor_id: String::new(),
                                    value: temp_c,
                                    unit: "°C".to_owned(),
                                    min: 0.0,
                                    max: 100.0,
                                });
                            }
                        }
                    }
                }

                // Leggi tensioni
                for i in 1..=20 {
                    let in_path = path.join(format!("in{i}_input"));
                    if let Ok(content) = fs::read_to_string(&in_path) {
                        if let Ok(mv) = content.trim().parse::<i32>() {
                            let voltage = mv as f32 / 1000.0;
                            if voltage > 0.0 && voltage < 20.0 {
                                let label_path = path.join(format!("in{i}_label"));
                                let label = fs::read_to_string(&label_path)
                                    .unwrap_or_else(|_| String::new())
                                    .trim()
                                    .to_string();

                                let reading_name = if !label.is_empty() {
                                    label.clone()
                                } else {
                                    format!("in{}", i)
                                };

                                results.push(SensorReading {
                                    source: SensorSource::HWMON,
                                    sensor_type: SensorType::Voltage,
                                    sensor_name: dev_name.clone(),
                                    reading_name,
                                    sensor_id: String::new(),
                                    value: voltage,
                                    unit: "V".to_owned(),
                                    min: 0.0,
                                    max: 20.0,
                                });
                            }
                        }
                    }
                }

                // Leggi ventilatori (RPM)
                for i in 1..=10 {
                    let fan_input = path.join(format!("fan{i}_input"));
                    if let Ok(content) = fs::read_to_string(&fan_input) {
                        if let Ok(rpm) = content.trim().parse::<i32>() {
                            if rpm > 0 {
                                results.push(SensorReading {
                                    source: SensorSource::HWMON,
                                    sensor_type: SensorType::Fan,
                                    sensor_name: dev_name.clone(),
                                    reading_name: format!("Fan {}", i),
                                    sensor_id: String::new(),
                                    value: rpm as f32,
                                    unit: "RPM".to_owned(),
                                    min: 0.0,
                                    max: 5000.0,
                                });
                            }
                        }
                    }
                }

                // Leggi potenza
                for i in 1..=10 {
                    let power_path = path.join(format!("power{i}_input"));
                    if let Ok(content) = fs::read_to_string(&power_path) {
                        if let Ok(microwatts) = content.trim().parse::<i64>() {
                            let watts = microwatts as f32 / 1_000_000.0;
                            if watts > 0.0 && watts < 1000.0 {
                                results.push(SensorReading {
                                    source: SensorSource::HWMON,
                                    sensor_type: SensorType::Power,
                                    sensor_name: dev_name.clone(),
                                    reading_name: format!("Power {}", i),
                                    sensor_id: String::new(),
                                    value: watts,
                                    unit: "W".to_owned(),
                                    min: 0.0,
                                    max: 1000.0,
                                });
                            }
                        }
                    }
                }
            }
        }

        // Leggi anche da /sys/class/fan per retrocompatibilità
        if let Ok(entries) = fs::read_dir(FAN_PATH) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if !path.is_dir() { continue; }

                let input = path.join("input");
                if let Ok(content) = fs::read_to_string(&input) {
                    if let Ok(rpm) = content.trim().parse::<i32>() {
                        if rpm > 0 {
                            let name = path.file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("fan");

                            results.push(SensorReading {
                                source: SensorSource::HWMON,
                                sensor_type: SensorType::Fan,
                                sensor_name: "System".to_owned(),
                                reading_name: name.to_owned(),
                                sensor_id: String::new(),
                                value: rpm as f32,
                                unit: "RPM".to_owned(),
                                min: 0.0,
                                max: 5000.0,
                            });
                        }
                    }
                }
            }
        }

        results
    }

    fn classify_hwmon_temp(dev: &str, label: &str, _idx: usize) -> String {
        let l = label.to_lowercase();
        let d = dev.to_lowercase();

        if d.contains("k10temp") || d.contains("cpu") || l.contains("cpu") {
            "CPU".to_owned()
        } else if d.contains("gpu") || d.contains("radeon") || d.contains("nvidia")
            || l.contains("gpu") || l.contains("graphics") {
            "GPU".to_owned()
        } else if d.contains("nvme") || d.contains("ssd") || l.contains("nvme") {
            "Storage".to_owned()
        } else if d.contains("acpi") || d.contains("主板") || d.contains("mobo") {
            "Scheda Madre".to_owned()
        } else {
            "Sistema".to_owned()
        }
    }

    pub fn check_sources() -> SourceStatus {
        let hwmon_exists = std::path::Path::new(HWMON_PATH).exists();
        SourceStatus { hwinfo_available: hwmon_exists, aida64_available: false }
    }

    pub fn read_all_sensors(_source: &SensorSource) -> Vec<SensorReading> {
        read_hwmon_sensors()
    }

    pub fn read_cpu_temperature(_source: &SensorSource) -> Option<f32> {
        let sensors = read_hwmon_sensors();
        best_cpu_temp(&sensors)
    }

    fn best_cpu_temp(sensors: &[SensorReading]) -> Option<f32> {
        let mut best: Option<(i32, f32)> = None;

        for s in sensors {
            if s.sensor_type != SensorType::Temperature { continue; }
            if s.value < -60.0 || s.value > 200.0 { continue; }

            let n = s.reading_name.to_lowercase();
            let s_name = s.sensor_name.to_lowercase();
            let category = s.sensor_name.clone();

            let score: i32 = if category == "CPU" {
                100
            } else if n.contains("cpu") || n.contains("core") || n.contains("tdie") {
                80
            } else if s_name.contains("cpu") {
                70
            } else if n.contains("package") {
                60
            } else {
                continue;
            };

            if best.as_ref().map_or(true, |b| score > b.0 || (score == b.0 && s.value > b.1)) {
                best = Some((score, s.value));
            }
        }
        best.map(|(_, v)| v)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Export pubblico cross-platform
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_os = "windows")]
pub fn read_all_sensors(source: &SensorSource) -> Vec<SensorReading> {
    platform::read_all_sensors(source)
}

#[cfg(target_os = "windows")]
pub fn read_cpu_temperature(source: &SensorSource) -> Option<f32> {
    platform::read_cpu_temperature(source)
}

#[cfg(target_os = "windows")]
pub fn check_sources() -> SourceStatus {
    platform::check_sources()
}

// Linux: usa lm-sensors via /sys/class/hwmon
#[cfg(target_os = "linux")]
pub fn read_all_sensors(source: &SensorSource) -> Vec<SensorReading> {
    platform_linux::read_all_sensors(source)
}

#[cfg(target_os = "linux")]
pub fn read_cpu_temperature(source: &SensorSource) -> Option<f32> {
    platform_linux::read_cpu_temperature(source)
}

#[cfg(target_os = "linux")]
pub fn check_sources() -> SourceStatus {
    platform_linux::check_sources()
}

// Fallback per altre piattaforme
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn read_all_sensors(_source: &SensorSource) -> Vec<SensorReading> { Vec::new() }

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn read_cpu_temperature(_source: &SensorSource) -> Option<f32> { None }

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn check_sources() -> SourceStatus {
    SourceStatus { hwinfo_available: false, aida64_available: false }
}

