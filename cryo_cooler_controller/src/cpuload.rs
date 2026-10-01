//! Carico CPU letto dalle API di Windows, senza dipendenze.
//!
//! **Perche'.** La modalita' automatica aveva una sola sorgente: i sensori di
//! HWiNFO o AIDA64. Se non sono installi non legge niente, resta in Idle e
//! sembra rotta. Invece il carico CPU e' esposto dal sistema stesso e si legge
//! con una chiamata, `GetSystemTimes`, senza installare nulla e senza
//! permessi.
//!
//! **La temperatura non e' uguale.** Windows non espone la temperatura della
//! CPU in modo affidabile (`MSAcpi_ThermalZoneTemperature` non c'e' sui desktop
//! e restituisce comunque la zona termica, non il pacchetto). Perci' la
//! temperatura resta **opzionale**: se c'e' un sensore si usa, altrimenti il
//! regime si decide sul carico da solo. Non si inventa mai un valore.

/// Cumulativi di CPU letti dal sistema, in "unità di 100 ns".
#[derive(Debug, Clone, Copy, Default)]
pub struct TempiCpu {
    pub idle:   u64,
    /// Attenzione: il tempo kernel **include** quello idle.
    pub kernel: u64,
    pub user:   u64,
}

/// Percentuale di CPU occupata fra due letture.
///
/// Formula: `occupato = 1 - idle / (kernel + user)`, e non
/// `1 - idle / kernel`, perche' il tempo kernel contiene gia' quello idle.
/// Sbaggiarlo dava sempre valori più alti del reale.
///
/// `None` quando non c'e' stato nessun intervalo: al primo campione non si
/// può calcolare nulla, e restituire 0 farebbe pensare che la CPU sia ferma.
pub fn percentuale(t0: &TempiCpu, t1: &TempiCpu) -> Option<f32> {
    let d_idle   = t1.idle   as f64 - t0.idle   as f64;
    let d_kernel = t1.kernel as f64 - t0.kernel as f64;
    let d_user   = t1.user   as f64 - t0.user   as f64;
    let totale = d_kernel + d_user;
    if totale <= 0.0 {
        return None;
    }
    let occupato = 1.0 - d_idle / totale;
    Some((occupato * 100.0).clamp(0.0, 100.0) as f32)
}

/// Legge i cumulativi attuali. `None` se la chiamata fallisce.
#[cfg(target_os = "windows")]
pub fn leggi() -> Option<TempiCpu> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::GetSystemTimes;

    unsafe {
        let mut idle = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let mut kernel = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let mut user = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        if GetSystemTimes(&mut idle, &mut kernel, &mut user) == 0 {
            return None;
        }
        let g = |f: &FILETIME| (f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64;
        Some(TempiCpu { idle: g(&idle), kernel: g(&kernel), user: g(&user) })
    }
}

#[cfg(not(target_os = "windows"))]
pub fn leggi() -> Option<TempiCpu> {
    None
}

/// Campionatore: tiene la lettura precedente e ricava la percentuale.
///
/// Un'istanza per l'applicazione. Non usa un thread: si chiama a ogni tick,
/// e il delta e' gia' pronto perche' il tick passa 2 volte al secondo.
pub struct CaricoCpu {
    precedente: Option<TempiCpu>,
}

impl CaricoCpu {
    pub fn new() -> Self {
        Self { precedente: None }
    }

    /// Percentuale occupata dall'ultimo campione, o `None` al primo.
    ///
    /// Se `leggi()` fallisce non si tiene il campione precedente: meglio
    /// saltare un dato che accostare due letture non consecutive, che
    /// produrrebbe una media falsa.
    pub fn campiona(&mut self) -> Option<f32> {
        let ora = leggi()?;
        let esito = self.precedente.as_ref().and_then(|p| percentuale(p, &ora));
        self.precedente = Some(ora);
        esito
    }
}

impl Default for CaricoCpu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Il caso che conta: il tempo kernel CONTIENE quello idle. Se si
    /// sbaglia il denominatore si ottiene sempre un carico più alto del reale.
    #[test]
    fn kernel_contiene_idle() {
        // 1000 unità totali, 250 idle. Occupato = 75%.
        // Con la formula sbagliata (1 - idle/kernel) verrebbe 1 - 250/500 = 50%.
        let t0 = TempiCpu { idle: 0, kernel: 0, user: 0 };
        let t1 = TempiCpu { idle: 250, kernel: 500, user: 500 };
        let p = percentuale(&t0, &t1).expect("calcolabile");
        assert!((p - 75.0).abs() < 0.5, "atteso 75, ottenuto {p}");
    }

    /// CPU ferma: idle coincide con il totale, occupato zero.
    #[test]
    fn cpu_ferma() {
        let t0 = TempiCpu { idle: 0, kernel: 0, user: 0 };
        let t1 = TempiCpu { idle: 800, kernel: 800, user: 0 };
        let p = percentuale(&t0, &t1).expect("calcolabile");
        assert!(p < 0.5, "atteso ~0, ottenuto {p}");
    }

    /// CPU al massimo: nessun tempo idle.
    #[test]
    fn cpu_al_massimo() {
        let t0 = TempiCpu { idle: 0, kernel: 0, user: 0 };
        let t1 = TempiCpu { idle: 0, kernel: 400, user: 400 };
        let p = percentuale(&t0, &t1).expect("calcolabile");
        assert!(p > 99.0, "atteso ~100, ottenuto {p}");
    }

    /// Il denominatore non puo' essere negativo ne' nullo: non si inventa una
    /// percentuale quando non c'e' stato tempo trascorso.
    ///
    /// Un intervallo "indietro" (orologio andato all'indietro, lettura spuria)
    /// torna `None` e non un numero: la percentuale sarebbe falsa, e una
    /// percentuale falsa che entra nel regime automatico fa scegliere la
    /// potenza sbagliata.
    #[test]
    fn nessun_intervallo_non_da_numeri() {
        let t = TempiCpu { idle: 100, kernel: 100, user: 100 };
        assert_eq!(percentuale(&t, &t), None, "nessun tempo trascorso");
        // Intervallo negativo: non calcolabile, non clampato a 0.
        let t2 = TempiCpu { idle: 50, kernel: 50, user: 50 };
        assert_eq!(percentuale(&t, &t2), None, "intervallo negativo");
    }
}
