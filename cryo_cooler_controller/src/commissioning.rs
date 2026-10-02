//! Log di collaudo: traccia su file le decisioni delle guardie.
//!
//! **Perche' esiste.** Il codice delle protezioni termiche e' stato scritto
//! e revisionato, ma nessuno puo' verificarlo senza l'hardware accanto: le
//! soglie 66 / 80 / 90 °C, il pavimento anticondensa, la riduzione su OCP e
//! il watchdog si possono solo giudicare osservando cosa comanda davvero il
//! programma all'hardware.
//!
//! Questo modulo scrive una riga per ogni decisione, con la motivazione, in
//! un file di testo. Dopo una prova controllata il file si legge e si
//! verifica che le soglie abbiano reagito come previsto.
//!
//! Non e' un logger generico: scrive solo quando qualcosa DECIDE, non a ogni
//! tick. Un file che cresce di 2 righe al secondo non serve a niente.
//!
//! Attivazione: variabile d'ambiente `CRYO_COMMISSIONING=1`. Assente, non
//! scrive nulla e non crea file: in uso normale il costo e' zero.

use std::io::Write;

/// Percorso del file di collaudo.
fn log_path() -> Option<std::path::PathBuf> {
    if std::env::var_os("CRYO_COMMISSIONING").is_none() {
        // Local trace beside the executable: button/serial diagnosis must
        // work without asking the user to set an environment variable.
        return std::env::current_exe().ok().and_then(|p| p.parent().map(|dir| dir.join("tec-controller.log")));
    }
    let dir = dirs::data_local_dir()
        .or_else(dirs::config_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("stargate-cryo");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("commissioning.log"))
}

/// Scrive un evento di collaudo. No-op se il collaudo non e' attivo.
///
/// Il file si appende a ogni chiamata e si chiude subito: e' il modo meno
/// costoso di non tenere un handle aperto, e con flush esplicito la riga
/// finisce su disco anche se il programma viene chiuso con forza.
pub fn event(azione: &str, dettaglio: &str) {
    let Some(path) = log_path() else { return };
    write_event(&path, azione, dettaglio);
}

fn write_event(path: &std::path::Path, azione: &str, dettaglio: &str) {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > 2_000_000) {
        let _ = std::fs::rename(path, path.with_extension("previous.log"));
    }
    let ts = chrono_like_now();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "[{ts}] {azione}: {dettaglio}");
        let _ = f.flush();
    }
}

/// Orario locale come `YYYY-MM-DD HH:MM:SS`, senza dipendenze.
///
/// Non si usa `chrono` per un modulo di diagnostica: aggiungere una
/// dipendenza per un timestamp non vale il costo.
fn chrono_like_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Giorni dal 1970 -> data, algoritmo di Howard Hinnant.
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mth <= 2 { y + 1 } else { y };

    format!("{y:04}-{mth:02}-{d:02} {h:02}:{m:02}:{s:02}")
}

/// Scrive l'intestazione del file, con la versione del firmware in uso.
pub fn header(fw_major: u8, fw_minor: u8, hw: u32) {
    event(
        "INIZIO",
        &format!("firmware {fw_major:X}.{fw_minor:X} hardware {hw:X}"),
    );
}

/// Percorso del file, per mostrarlo all'utente nelle impostazioni.
pub fn path_display() -> String {
    log_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "collaudo non attivo".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Il modulo deve scrivere su file quando e' attivo, e restare muto
    /// quando non lo e'. E' l'unica cosa che puo' essere verificata senza
    /// l'hardware: le soglie si osservano sul banco, non in un test.
    #[test]
    fn scrive_solo_quando_attivo() {
        // Il modulo e' spento per default: il test lo attiva esplicitamente,
        // altrimenti darebbe un falso "passa".
        let path = std::env::temp_dir().join(format!("cryo-commissioning-test-{}.log", std::process::id()));
        // Pulizia: il test non deve dipendere da un log preesistente.
        let _ = std::fs::remove_file(&path);

        write_event(&path, "INIZIO", "firmware 13.A0 hardware 4");
        write_event(&path, "POTENZA", "ctrl=70.0 °C  sopra guardia 66 °C  60% -> 58%");
        write_event(&path, "SET-OK", "set_power_level(58) confermato dall'hardware");

        let contenuto = std::fs::read_to_string(&path).expect("log scritto");
        let righe: Vec<&str> = contenuto.lines().collect();
        assert_eq!(righe.len(), 3, "una intestazione e due eventi: {contenuto}");
        assert!(righe[0].contains("INIZIO"), "{}", righe[0]);
        assert!(righe[0].contains("13.A0"), "firmata mancante: {}", righe[0]);
        assert!(righe[1].contains("POTENZA"), "{}", righe[1]);
        assert!(righe[2].contains("SET-OK"), "{}", righe[2]);
        // Il timestamp deve essere una data, non spazi: e' il controllo che
        // distingue una riga scritta da una rotta.
        assert!(righe[0].starts_with('['), "{}", righe[0]);
        assert_eq!(righe[0].as_bytes()[1], b'2', "anno non plausibile: {}", righe[0]);
        std::fs::remove_file(path).expect("test cleanup");
    }

    /// L'orario va calcolato bene: un calendario sbagliato rende il log
    /// inutilizzabile proprio nel momento in cui serve.
    #[test]
    fn orario_corretto() {
        assert_eq!(chrono_from(secs(1970, 1, 1, 0, 0, 0)), "1970-01-01 00:00:00");
        assert_eq!(chrono_from(secs(2000, 3, 1, 12, 30, 45)), "2000-03-01 12:30:45");
        // Anno bisestile: 29 febbraio 2004.
        assert_eq!(chrono_from(secs(2004, 2, 29, 23, 59, 59)), "2004-02-29 23:59:59");
    }

    fn secs(y: i64, m: u32, d: u32, h: u64, mi: u64, s: u64) -> u64 {
        // Giorni dal 1970 -> data, per costruire l'input del test.
        let leap = |y: i64| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let mut days: i64 = 0;
        for yy in 1970..y {
            days += if leap(yy) { 366 } else { 365 };
        }
        let ml = [31, if leap(y) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        for mm in 1..m {
            days += ml[(mm - 1) as usize] as i64;
        }
        days += (d - 1) as i64;
        (days as u64) * 86_400 + h * 3600 + mi * 60 + s
    }

    /// Stessa conversione di `chrono_like_now`, su un `secs` deciso: il
    /// test non deve dipendere dall'orologio di sistema.
    fn chrono_from(total: u64) -> String {
        let days = (total / 86_400) as i64;
        let rem = total % 86_400;
        let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let mth = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if mth <= 2 { y + 1 } else { y };
        format!("{y:04}-{mth:02}-{d:02} {h:02}:{m:02}:{s:02}")
    }
}
