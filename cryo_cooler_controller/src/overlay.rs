//! RTSS Overlay + Discord Rich Presence.
//!
//! Due integrazioni verso l'esterno, entrambe con la stessa regola: se non
//! si puo' verificare che la struttura sia quella giusta, non si scrive
//! niente. Meglio un banner "non disponibile" che la memoria di un altro
//! processo corrotta.
//!
//! RTSS (RivaTuner Statistics Server): la shared memory `RTSSSharedMemoryV2`
//! espone un array di voci OSD. Il layout usato qui e' quello dell'SDK
//! pubblico, e ogni scrittura e' preceduta dai controlli di firma, versione
//! e confine dei puntatori.
//!
//! Discord Rich Presence: IPC su named pipe `\\.\pipe\discord-ipc-N` con
//! frame `[op:u32, len:u32, payload]`.

// ── RTSS Overlay ──────────────────────────────────────────────────────────────

/// Aggiorna l'overlay RTSS via shared memory.
///
/// **Perche' i controlli non sono opzionali.** La struttura di
/// `RTSSSharedMemoryV2` comincia con:
///
/// ```text
/// +0x00  dwSignature   = 'RTSS' (0x53535452)
/// +0x04  dwVersion     = 0x0002xxxx
/// +0x08  osdArr.pOsmEntries  (u64)  <- puntatore all'array delle voci
/// +0x10  osdArr.dwArraySize   (u32)
/// ```
///
/// Ogni `RTSS_OSD_ENTRY` e' di 0x400 byte, con `szName` a +0x00 (64 WCHAR) e
/// `szValue` a +0x80 (256 WCHAR). Scrivere la stringa in un offset sbagliato
/// significa scrivere dentro `dwArraySize`, dentro un'altra voce, o dentro
/// un campo che RTSS usa per il suo stato interno: da li' RTSS puo' andare in
/// crash. La versione precedente usava `0x1000 + slot*0x100`, che non
/// corrisponde a nulla.
///
/// `pOsmEntries` e' un puntatore che RTSS ha allocato: non puo' essere
/// derivato, va letto. E non puo' essere fidato ciecamente, quindi lo si
/// accetta solo se cade dentro la regione mappata.
#[cfg(target_os = "windows")]
pub mod rtss {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    const RTSS_SM_NAME: &str = "RTSSSharedMemoryV2";
    /// Firma di `RTSSSharedMemoryV2`, letta con `read_unaligned::<u32>`.
    ///
    /// RTSS scrive il multichar literal dell'MSVC, `(DWORD)'RTSS'`, che
    /// mette `'R'` nel byte BASSO: in memoria i byte sono `52 54 53 53`.
    /// Il campo viene quindi letto in ordine nativo (little-endian su x86),
    /// e il valore da confrontare e' quello che mette "RTSS" in
    /// `to_le_bytes()` — `0x53535452`.
    ///
    /// Con il byte order inverso (`0x52545353`) i byte in memoria
    /// diventerebbero `53 53 54 52` = "SSTR", che RTSS non scrive mai: il
    /// confronto fallirebbe sempre e l'overlay non si attiverebbe mai.
    const RTSS_FIRMA: u32 = 0x5353_5452;
    /// Versione maggiore accettata: la 2. La 1 non ha `osdArr` a +0x08.
    const RTSS_MAJOR: u32 = 2;
    /// Slot della voce OSD che scriviamo.
    const OSD_SLOT: u32 = 0;
    /// Dimensione di una `RTSS_OSD_ENTRY` in byte (0x400).
    const ENTRY_SIZE: usize = 0x400;
    /// Offset di `szValue` dentro la voce: 64 WCHAR di `szName` = 128 byte.
    const VALUE_OFF: usize = 0x80;
    /// WCHAR disponibili in `szValue`.
    const VALUE_CHARS: usize = 256;
    /// Offset di `osdArr.pOsmEntries` e `osdArr.dwArraySize`.
    const ARRP_OFF: usize = 8;
    const ARRSZ_OFF: usize = 12;

    /// Perche' l'integrazione non sta funzionando. Esposto alla UI.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum RtssState {
        /// RTSS non trovato: non in esecuzione, o nome diverso.
        Assente,
        /// RTSS c'e' ma la struttura non corrisponde: NON si scrive.
        LayoutInvalido,
        /// Scritture attive.
        Attivo,
    }

    pub struct RtssOverlay {
        state: RtssState,
    }

    impl RtssOverlay {
        pub fn new() -> Self {
            Self { state: RtssState::Assente }
        }

        pub fn is_active(&self) -> bool {
            self.state == RtssState::Attivo
        }

        pub fn state(&self) -> RtssState {
            self.state
        }

        /// Apre la shared memory e valida il layout. Ritorna l'handle solo se
        /// tutto torna: firma, versione e puntatore dentro la vista.
        fn open_validated() -> Option<(windows_sys::Win32::Foundation::HANDLE, *mut u8, usize)> {
            use windows_sys::Win32::System::Memory::*;

            let name: Vec<u16> = OsStr::new(RTSS_SM_NAME)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            unsafe {
                // FILE_MAP_ALL_ACCESS: per scrivere `szValue`.
                let h = OpenFileMappingW(FILE_MAP_ALL_ACCESS, 0, name.as_ptr());
                if h == 0 {
                    return None;
                }
                // dwNumberOfBytesToMap = 0 mappa tutta la sezione.
                let view = MapViewOfFile(h, FILE_MAP_ALL_ACCESS, 0, 0, 0);
                if view == 0 {
                    windows_sys::Win32::Foundation::CloseHandle(h);
                    return None;
                }
                let base = view as *mut u8;

                // ── Controlli di layout ───────────────────────────────
                let ok = (|| {
                    let sig = std::ptr::read_unaligned(base as *const u32);
                    if sig != RTSS_FIRMA {
                        return false;
                    }
                    let ver = std::ptr::read_unaligned(base.add(4) as *const u32);
                    if (ver >> 16) != RTSS_MAJOR {
                        return false;
                    }
                    true
                })();
                if !ok {
                    UnmapViewOfFile(view);
                    windows_sys::Win32::Foundation::CloseHandle(h);
                    return None;
                }

                // Dimensione effettiva della vista, per il controllo di confine
                // sul puntatore.
                let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
                let size = if VirtualQuery(
                    base as *const _,
                    &mut mbi,
                    std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                ) != 0
                {
                    mbi.RegionSize
                } else {
                    0
                };

                // `pOsmEntries`: letto, non calcolato. Deve cadere nella vista.
                let arr_ptr = std::ptr::read_unaligned(base.add(ARRP_OFF) as *const u64);
                let arr_size = std::ptr::read_unaligned(base.add(ARRSZ_OFF) as *const u32);

                let lo = base as usize;
                let hi = lo + size;
                // Le parentesi servono: senza, `as usize < hi` viene
                // interpretato come un const generic e non compila.
                let inside = (arr_ptr as usize) >= lo && (arr_ptr as usize) < hi;
                let slot_ok = arr_size > OSD_SLOT;

                if !inside || !slot_ok {
                    UnmapViewOfFile(view);
                    windows_sys::Win32::Foundation::CloseHandle(h);
                    return None;
                }

                // Il byte finale della voce deve stare nella vista.
                let end = (arr_ptr as usize)
                    .checked_add((OSD_SLOT as usize + 1) * ENTRY_SIZE)
                    .unwrap_or(usize::MAX);
                if end > hi {
                    UnmapViewOfFile(view);
                    windows_sys::Win32::Foundation::CloseHandle(h);
                    return None;
                }

                Some((h, base, arr_ptr as usize))
            }
        }

        /// Tenta la connessione. `LayoutInvalido` vuol dire che RTSS c'e' ma
        /// la struttura non e' quella attesa: in quel caso non si scrive.
        pub fn try_connect(&mut self) -> bool {
            use windows_sys::Win32::System::Memory::{OpenFileMappingW, FILE_MAP_READ};

            // Prima solo lettura, per distinguere "assente" da "presente ma
            // strano" senza aprire in scrittura.
            let name: Vec<u16> = OsStr::new(RTSS_SM_NAME)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let esiste = unsafe {
                let h = OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr());
                if h == 0 {
                    false
                } else {
                    windows_sys::Win32::Foundation::CloseHandle(h);
                    true
                }
            };

            if !esiste {
                self.state = RtssState::Assente;
                return false;
            }
            match Self::open_validated() {
                Some((h, view, _)) => {
                    unsafe {
                        windows_sys::Win32::System::Memory::UnmapViewOfFile(view as isize);
                        windows_sys::Win32::Foundation::CloseHandle(h);
                    }
                    self.state = RtssState::Attivo;
                }
                None => self.state = RtssState::LayoutInvalido,
            }
            self.is_active()
        }

        /// Scrive `testo` in `szValue` della voce scelta.
        ///
        /// Tutte le stringhe RTSS sono UTF-16LE, quindi si converte: il
        /// formato con i codici colore (`<C0080FF>`) e' proprio cosi' che
        /// RTSS lo interpreta.
        fn write_slot(&self, testo: &str) {
            use windows_sys::Win32::System::Memory::UnmapViewOfFile;

            let Some((h, _view, arr_ptr)) = Self::open_validated() else {
                return;
            };
            let utf16: Vec<u16> = testo.encode_utf16().take(VALUE_CHARS - 1).collect();
            unsafe {
                let entry = (arr_ptr as *mut u8).add(OSD_SLOT as usize * ENTRY_SIZE);
                std::ptr::copy_nonoverlapping(
                    utf16.as_ptr(),
                    entry.add(VALUE_OFF) as *mut u16,
                    utf16.len(),
                );
                // Terminatore nullo: RTSS legge fino a qui.
                *((entry.add(VALUE_OFF) as *mut u16).add(utf16.len())) = 0;
                UnmapViewOfFile(_view as isize);
                windows_sys::Win32::Foundation::CloseHandle(h);
            }
        }

        /// Formato: "TEC -4.2°C | Condensa +3.1°C | COP 1.84"
        pub fn update(&self, tec_temp: f32, margin: f32, cop: f32) {
            if !self.is_active() {
                return;
            }
            let testo = if margin > 0.0 {
                format!(
                    "<C0080FF>TEC</C> {:.1}\u{b0}C  <C00FF80>+{:.1}\u{b0}</C>  COP {:.2}",
                    tec_temp, margin, cop
                )
            } else {
                format!(
                    "<CFF3030>TEC {:.1}\u{b0}C  CONDENSA {:.1}\u{b0}</C>  COP {:.2}",
                    tec_temp, margin, cop
                )
            };
            self.write_slot(&testo);
        }

        /// Svuota davvero l'OSD.
        ///
        /// La versione precedente chiamava `update(0.0, 99.0, 0.0)`, quindi il
        /// bottone "Disattiva" scriveva `TEC 0.0°C +99.0°C`: un dato falso
        /// lasciato sullo schermo. Qui si scrive una stringa vuota.
        pub fn clear(&self) {
            if !self.is_active() {
                return;
            }
            self.write_slot("");
        }
    }

    impl Default for RtssOverlay {
        fn default() -> Self {
            Self::new()
        }
    }

    #[cfg(test)]
    mod tests {
        /// `dwSignature` viene letto con `read_unaligned::<u32>`, quindi
        /// nell'ordine nativo: su x86 e' little-endian, e i byte in memoria
        /// sono esattamente quelli di `to_le_bytes()`. RTSS scrive "RTSS",
        /// quindi devono essere `52 54 53 53`.
        ///
        /// Il test fallisce se la costante torna al byte order inverso, che
        /// produceva "SSTR" e rendeva l'overlay impossibile da attivare.
        #[test]
        fn firma_rtss_si_little_endian() {
            assert_eq!(&super::RTSS_FIRMA.to_le_bytes(), b"RTSS");
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub mod rtss {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum RtssState {
        Assente,
        LayoutInvalido,
        Attivo,
    }

    pub struct RtssOverlay;
    impl RtssOverlay {
        pub fn new() -> Self {
            Self
        }
        pub fn try_connect(&mut self) -> bool {
            false
        }
        pub fn is_active(&self) -> bool {
            false
        }
        pub fn state(&self) -> RtssState {
            RtssState::Assente
        }
        pub fn update(&self, _: f32, _: f32, _: f32) {}
        pub fn clear(&self) {}
    }
    impl Default for RtssOverlay {
        fn default() -> Self {
            Self::new()
        }
    }
}

// ── Discord Rich Presence ─────────────────────────────────────────────────────

/// Discord Rich Presence via named pipe.
///
/// Il protocollo e' una sequenza di frame: due `u32` little-endian (opcode e
/// lunghezza del payload) seguiti dal JSON.
///
/// La versione precedente era uno stub completo:
///   - `try_connect` usava `fs::metadata` su `\\.\pipe\discord-ipc-N`, e
///     `metadata` non puo' aprire una named pipe, quindi `active` era sempre
///     `false` e la funzione non faceva mai niente;
///   - `send_handshake` aveva il corpo vuoto;
///   - `update` avviava un binario esterno `discord-rpc` e scartava l'errore.
///
/// Qui la pipe si apre davvero con `File`, e ogni comando viene inviato e
/// letto indietro. Se `client_id` non e' quello di un'applicazione Discord
/// reale, l'handshake viene rifiutato e resta `inattivo`: non si finge.
pub struct DiscordRpc {
    pipe:   Option<std::fs::File>,
    active: bool,
    client_id: String,
    last_score: u32,
}

/// `client_id` segnaposto: non corrisponde a nessuna applicazione, quindi
/// Discord rifiuta l'handshake. Serve a non mostrare "attivo" per errore.
pub const CLIENT_ID_PLACEHOLDER: &str = "1234567890";

impl DiscordRpc {
    /// `client_id` = Application ID dal Discord Developer Portal.
    /// Se resta il segnaposto, l'integrazione resta dichiarata non attiva.
    pub fn new(client_id: impl Into<String>) -> Self {
        let client_id = client_id.into();
        let credibile = client_id != CLIENT_ID_PLACEHOLDER && client_id.len() >= 17;
        Self {
            pipe: None,
            active: false,
            client_id: if credibile { client_id } else { CLIENT_ID_PLACEHOLDER.to_owned() },
            last_score: 0,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Il `client_id` e' quello di un'applicazione reale?
    pub fn client_id_valido(&self) -> bool {
        self.client_id != CLIENT_ID_PLACEHOLDER
    }

    /// Invia un frame e legge la risposta. Ritorna il corpo JSON, se arriva.
    fn roundtrip(&mut self, op: u32, payload: &str) -> Option<String> {
        use std::io::{Read, Write};

        let pipe = self.pipe.as_mut()?;
        let mut frame = Vec::with_capacity(8 + payload.len());
        frame.extend_from_slice(&op.to_le_bytes());
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(payload.as_bytes());
        if pipe.write_all(&frame).is_err() || pipe.flush().is_err() {
            return None;
        }

        // Header di risposta.
        let mut hdr = [0u8; 8];
        pipe.read_exact(&mut hdr).ok()?;
        let op_back = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
        let len = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize;
        // Difensivo: una lunghezza assurda non deve allocare.
        if len > 64 * 1024 {
            return None;
        }
        let mut body = vec![0u8; len];
        pipe.read_exact(&mut body).ok()?;
        if op_back != op {
            return None;
        }
        String::from_utf8(body).ok()
    }

    /// Prova le pipe 0-9 e manda l'handshake. Attiva solo se Discord
    /// risponde con un Dispatch e un `evt: null`.
    pub fn try_connect(&mut self) -> bool {
        self.active = false;
        self.pipe = None;

        if !self.client_id_valido() {
            return false;
        }

        for i in 0..10u8 {
            let path = format!(r"\\.\pipe\discord-ipc-{i}");
            let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(&path) else {
                continue;
            };
            self.pipe = Some(file);
            let payload = format!(r#"{{"v":1,"client_id":"{}"}}"#, self.client_id);
            match self.roundtrip(0, &payload) {
                Some(resp) if resp.contains(r#""evt":null"#) => {
                    self.active = true;
                    return true;
                }
                // Pipe presente ma handshake non riuscito: proviamo la
                // successiva, che potrebbe essere quella giusta.
                _ => self.pipe = None,
            }
        }
        false
    }

    /// Aggiorna l'attivita'. Nessun effetto se non c'e' un `client_id` valido
    /// o Discord non ha risposto all'handshake.
    pub fn update(&mut self, tec_temp: f32, oc_score: u32, mode: &str) {
        if !self.active {
            return;
        }
        if oc_score == self.last_score {
            return;
        }
        self.last_score = oc_score;

        let stato = if tec_temp < -5.0 {
            format!("Sub-zero cryo \u{00B7} {:.1}\u{00B0}C TEC", tec_temp)
        } else {
            format!("Monitoring \u{00B7} {:.1}\u{00B0}C TEC", tec_temp)
        };
        let dettagli = format!("OC Score {} \u{00B7} {}", oc_score, mode);
        let payload = format!(
            r#"{{"cmd":"SET_ACTIVITY","args":{{"pid":{},"activity":{{"state":"{}","details":"{}","large_image":"stargate_cryo","large_text":"StargateLabs CryoCooling Dashboard"}}}},"nonce":"cryo"}}"#,
            std::process::id(),
            stato.replace('"', "'"),
            dettagli.replace('"', "'"),
        );
        self.roundtrip(1, &payload);
    }

    pub fn clear(&mut self) {
        if self.active {
            // Activity vuota: e' il modo protocollo-ordinario di rimuoverla.
            let payload = format!(
                r#"{{"cmd":"SET_ACTIVITY","args":{{"pid":{},"activity":null}},"nonce":"cryo"}}"#,
                std::process::id(),
            );
            self.roundtrip(1, &payload);
        }
        self.active = false;
        self.pipe = None;
        self.last_score = 0;
    }
}

impl Default for DiscordRpc {
    fn default() -> Self {
        Self::new(CLIENT_ID_PLACEHOLDER)
    }
}
