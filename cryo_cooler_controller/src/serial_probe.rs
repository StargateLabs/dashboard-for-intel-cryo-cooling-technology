//! Auto-rilevamento porta seriale del controller TEC.
//!
//! Scansiona tutte le porte COM disponibili e invia un comando di probe (HEART_BEAT).
//! La porta corretta risponde con status valido.

pub struct SerialProbe;

/// Esito della scansione, con diagnostica per capire i fallimenti.
pub struct ScanReport {
    pub found: Option<String>,
    /// Porte esaminate, con l'esito di ciascuna.
    pub attempted: Vec<(String, bool)>,
}

impl SerialProbe {
    /// Enumera tutte le porte COM disponibili usando la crate `serialport`.
    #[cfg(target_os = "windows")]
    pub fn available_com_ports() -> Vec<String> {
        serialport::available_ports()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|port| {
                let name = port.port_name;
                if name.to_uppercase().starts_with("COM") {
                    Some(name)
                } else {
                    None
                }
            })
            .collect()
    }

    #[cfg(not(target_os = "windows"))]
    pub fn available_com_ports() -> Vec<String> {
        Vec::new()
    }

    /// Verifica se una porta è un controller TEC valido.
    /// Ritorna true se il device risponde con un heartbeat valido.
    ///
    /// Usa `Tec::probe` (non distruttivo): niente reset, e la porta viene
    /// chiusa subito dopo. La connessione reale la fa poi `Tec::new`.
    #[cfg(target_os = "windows")]
    pub fn probe_port(port_name: &str) -> bool {
        use cryo_cooler_controller_lib::Tec;

        Tec::probe(&port_name, Self::PROBE_TIMEOUT).is_ok()
    }

    #[cfg(not(target_os = "windows"))]
    pub fn probe_port(_port_name: &str) -> bool {
        false
    }

    /// Timeout del probe. Volutamente più corto del timeout di connessione
    /// (150ms): durante una scansione dobbiamo attraversare tutte le porte
    /// presenti, e quelle non-TEC (Bluetooth, debug, Arduino) devono fallire
    /// in fretta. 120ms è ampio per un round-trip a 115200 baud.
    const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(120);

    /// Scansiona le porte e restituisce la prima che risponde come TEC.
    ///
    /// Usa `Tec::probe`, che **non** manda il reset: una scansione non deve
    /// alterare lo stato del controller, altrimenti il retry automatico
    /// (uno ogni 2 secondi) farebbe reset ripetuti e la board ripartirebbe
    /// da capo ogni volta.
    pub fn find_tec_port_with_report() -> ScanReport {
        let ports = Self::available_com_ports();
        let mut attempted = Vec::with_capacity(ports.len());

        for port in &ports {
            let ok = Self::probe_port(port);
            attempted.push((port.clone(), ok));
            if ok {
                return ScanReport { found: Some(port.clone()), attempted };
            }
        }

        ScanReport { found: None, attempted }
    }

    /// Trova automaticamente la porta del controller TEC.
    /// Ritorna Some(nome_porta) se trovato, None altrimenti.
    #[cfg(target_os = "windows")]
    pub fn find_tec_port() -> Option<String> {
        Self::find_tec_port_with_report().found
    }

    #[cfg(not(target_os = "windows"))]
    pub fn find_tec_port() -> Option<String> {
        None
    }
}