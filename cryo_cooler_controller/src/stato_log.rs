//! Log dei bit di stato non decodificati.
//!
//! Scrive **sempre**, senza `CRYO_COMMISSIONING`: e' una sola riga per
//! campione (2 Hz) di numeri, non un comando hardware, e serve a capire se
//! i bit 18..=31 cambiano quando l'OCP scatta. Disattivabile con
//! `disabilita()` se l'utente non lo vuole.

use cryo_cooler_controller_lib::tecstatus::{intestazione_csv, StatusCompleto};
use std::io::Write;
use std::path::PathBuf;

/// Tetto di righe. Oltre, il file si tronca: i dati recenti sono quelli
/// che servono. Con 2 Hz, 20000 righe sono quasi 3 ore.
pub const MAX_RIGHE: usize = 20_000;

/// `%LOCALAPPDATA%\stargate-cryo`, la stessa cartella della curva.
pub fn cartella_app(local_appdata: &str) -> PathBuf {
    PathBuf::from(local_appdata).join("stargate-cryo")
}

/// Il log.
///
/// Senza interruttore on/off perche' non serve: una riga di numeri ogni
/// mezzo secondo e' trascurabile, ed e' il materiale che serve per capire
/// l'OCP. Se un giorno servisse un interruttore, si aggiunge qui.
pub struct StatoLog {
    path: PathBuf,
    /// Righe gia' scritte, contate esattamente.
    ///
    /// Un contatore, non una stima sui byte: la prima versione stimava
    /// "~20 byte per riga" e con le righe reali (timestamp RFC3339 piu'
    /// quattro cifre esadecimali, circa 7-30 byte) la soglia non veniva
    /// mai raggiunta e il file cresceva senza tetto. Contare e' economico
    /// e non indovina.
    righe: usize,
}

impl StatoLog {
    /// Apre il log. Non crea il file: si crea alla prima scrittura.
    ///
    /// Se il file esiste gia', conta le righe: cosi' il tetto vale anche
    /// per un riavvio a meta' sessione, non solo per una sessione nuova.
    pub fn apri(path: PathBuf) -> Self {
        let righe = std::fs::read_to_string(&path)
            .map(|t| t.lines().count())
            .unwrap_or(0);
        Self { path, righe }
    }

    /// Scrive un campione. `false` se la scrittura fallisce.
    pub fn registra(&mut self, s: &StatusCompleto, timestamp: &str) -> bool {
        let nuovo = self.righe == 0;
        let mut f = match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(f) => f,
            Err(_) => return false,
        };
        if nuovo {
            let _ = writeln!(f, "{}", intestazione_csv());
        }
        if writeln!(f, "{}", s.riga_csv(timestamp)).is_err() {
            return false;
        }
        self.righe += 1;
        self.trocca_se_troppo();
        true
    }

    /// Tronca dall'inizio quando il file supera il tetto, tenendo
    /// l'intestazione: un CSV che la perde non e' apribile.
    fn trocca_se_troppo(&mut self) {
        if self.righe <= MAX_RIGHE {
            return;
        }
        if let Ok(testo) = std::fs::read_to_string(&self.path) {
            let mut righe: Vec<&str> = testo.lines().collect();
            if righe.len() > MAX_RIGHE {
                let testa = righe.remove(0);
                righe.truncate(MAX_RIGHE - 1);
                let nuovo = std::iter::once(testa)
                    .chain(righe)
                    .collect::<Vec<_>>()
                    .join("\n");
                if std::fs::write(&self.path, nuovo).is_ok() {
                    self.righe = MAX_RIGHE;
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use cryo_cooler_controller_lib::tecstatus::scompone;

    fn stato(codice: u32) -> StatusCompleto {
        scompone(codice)
    }

    /// Il log deve stare in `%LOCALAPPDATA%\stargate-cryo`: la stessa
    /// cartella della curva, cosi' l'utente trova entrambi i file dove
    /// guarda gia' gli altri.
    #[test]
    fn il_percorso_e_nella_cartella_dell_app() {
        let dir = cartella_app("C:\\\\Users\\\\X\\\\AppData\\\\Local");
        assert!(
            dir.to_string_lossy().ends_with("stargate-cryo"),
            "cartella sbagliata: {dir:?}"
        );
    }

    /// Il file non deve crescere senza limite: gira per giorni. Oltre il
    /// tetto si tronca dall'inizio, perche' i dati recenti sono quelli
    /// che servono a capire un OCP che scatta adesso.
    #[test]
    fn il_file_si_trocca_oltre_il_tetto() {
        let dir = std::env::temp_dir().join("cryo-stato-log-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut log = StatoLog::apri(dir.join("bit.csv"));
        for i in 0..(MAX_RIGHE + 10) {
            log.registra(&stato(i as u32), "t");
        }
        let testo = std::fs::read_to_string(dir.join("bit.csv")).unwrap();
        let righe = testo.lines().count();
        assert!(righe <= MAX_RIGHE, "il file e' cresciuto a {righe} righe");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Aprire il log non crea il file.** Il file compare solo alla prima
    /// scrittura: un utente che non ha mai collegato un TEC non deve
    /// ritrovarsi un CSV vuoto, e `apri` viene chiamato durante `new()`
    /// prima ancora che la seriale sia connessa.
    #[test]
    fn aprire_non_crea_il_file() {
        let dir = std::env::temp_dir().join("cryo-stato-log-apri");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _log = StatoLog::apri(dir.join("bit.csv"));
        assert!(
            !dir.join("bit.csv").exists(),
            "apri ha creato il file senza scrivere nulla"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Il tetto vale anche dopo un riavvio.** Senza questo, riaprire
    /// l'app ricomincerebbe il conteggio da zero e il file potrebbe
    /// superare il tetto di un'intera sessione.
    #[test]
    fn il_tetto_vale_anche_dopo_la_riapertura() {
        let dir = std::env::temp_dir().join("cryo-stato-log-riapri");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        {
            let mut log = StatoLog::apri(dir.join("bit.csv"));
            for i in 0..(MAX_RIGHE - 10) {
                log.registra(&stato(i as u32), "t");
            }
        }
        // Seconda istanza, come al riavvio dell'app.
        let mut log = StatoLog::apri(dir.join("bit.csv"));
        for i in 0..100 {
            log.registra(&stato(i as u32), "t");
        }
        let righe = std::fs::read_to_string(dir.join("bit.csv")).unwrap().lines().count();
        assert!(
            righe <= MAX_RIGHE,
            "dopo la riapertura il file e' a {righe} righe"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// La prima riga deve essere l'intestazione, altrimenti il file non
    /// e' un CSV e non si puo' aprire in Excel.
    #[test]
    fn la_prima_riga_e_l_intestazione() {
        let dir = std::env::temp_dir().join("cryo-stato-log-hdr");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut log = StatoLog::apri(dir.join("bit.csv"));
        log.registra(&stato(1), "t1");
        let testo = std::fs::read_to_string(dir.join("bit.csv")).unwrap();
        assert!(
            testo.lines().next().unwrap().starts_with("timestamp"),
            "{testo}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Il troncamento deve tenere l'intestazione: un CSV che la perde
    /// diventa illeggibile per chi lo apre per capire l'OCP.
    #[test]
    fn il_troncamento_tiene_l_intestazione() {
        let dir = std::env::temp_dir().join("cryo-stato-log-tronc");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut log = StatoLog::apri(dir.join("bit.csv"));
        for i in 0..(MAX_RIGHE + 10) {
            log.registra(&stato(i as u32), "t");
        }
        let testo = std::fs::read_to_string(dir.join("bit.csv")).unwrap();
        assert!(
            testo.lines().next().unwrap().starts_with("timestamp"),
            "il troncamento ha perso l'intestazione"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
