//! Avvio automatico con Windows — usa winreg per modificare il registro di sistema.
//!
//! Chiave: HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run
//! Valore: "StargateCryoCooling" -> percorso dell'eseguibile

#[cfg(target_os = "windows")]
mod platform {
    use std::io;
    use std::path::PathBuf;
    use winreg::enums::*;
    use winreg::RegKey;

    const APP_NAME: &str = "StargateCryoCooling";
    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    pub fn is_autostart_enabled() -> bool {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey(RUN_KEY) {
            key.get_value::<String, _>(APP_NAME).is_ok()
        } else {
            false
        }
    }

    /// Percorso attualmente registrato per l'avvio automatico.
    pub fn autostart_target() -> Option<String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = hkcu.open_subkey(RUN_KEY).ok()?;
        key.get_value::<String, _>(APP_NAME).ok()
    }

    /// Se l'avvio automatico e' attivo ma punta a un eseguibile diverso da
    /// quello in esecuzione, lo riallinea.
    ///
    /// **Perche' serve.** Il registro sopravvive a una reinstallazione o a uno
    /// spostamento della cartella. In quel caso Windows avviava una copia
    /// vecchia dell'app, che si prendeva la porta COM del controller: la
    /// copia nuova non poteva piu' connettersi e l'utente vedeva sempre
    /// "seleziona la porta", senza apparentemente alcuna causa. Il sintomo
    /// sembrava un bug di rilevamento, ma era un percorso obsoleto.
    ///
    /// Restituisce `true` se ha corretto qualcosa, cosi' l'app puo' dirlo.
    pub fn repair_autostart() -> bool {
        let Ok(current) = std::env::current_exe() else { return false };
        let current = current.to_string_lossy().to_string();

        let Some(registered) = autostart_target() else {
            // Non e' attivo: niente da riparare, e non lo riattiviamo da soli
            // (sarebbe una scelta dell'utente, non nostra).
            return false;
        };

        // Normalizziamo: il registro puo' avere virgolette e slashes diversi,
        // che non sono un disallineamento reale.
        let norm = |s: &str| s.trim_matches('"').replace('/', "\\").to_lowercase();
        if norm(&registered) == norm(&current) {
            return false;
        }

        // Il percorso registrato non esiste piu': e' un binario cancellato.
        // In quel caso non è un problema di percorso: non tocchiamo la scelta
        // dell'utente, ma eliminiamo la voce spazzatura.
        if !std::path::Path::new(registered.trim_matches('"')).exists() {
            let _ = disable_autostart();
            return true;
        }

        if enable_autostart().is_ok() {
            eprintln!(
                "[autostart] Corretto: puntava a '{registered}', ora punta a '{current}'."
            );
            true
        } else {
            false
        }
    }

    pub fn enable_autostart() -> io::Result<()> {
        let exe_path = std::env::current_exe()
            .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(RUN_KEY)?;
        key.set_value(APP_NAME, &exe_path.to_string_lossy().to_string())?;
        Ok(())
    }

    pub fn disable_autostart() -> io::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        // BUG: prima `if let Ok(key) = ... { let _ = key.delete_value(...) }` —
        // ogni errore veniva ignorato e la funzione ritornava Ok(()).
        // Risultato: il toggle mostrava "OFF" ma l'app ripartiva comunque.
        let key = hkcu.open_subkey_with_flags(RUN_KEY, KEY_WRITE)?;
        match key.delete_value(APP_NAME) {
            Ok(()) => Ok(()),
            // La chiave non esisteva già: l'obiettivo ("disattivato") è raggiunto.
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub fn toggle_autostart(enable: bool) -> io::Result<()> {
        if enable {
            enable_autostart()
        } else {
            disable_autostart()
        }
    }
}

#[cfg(target_os = "windows")]
pub use platform::*;

#[cfg(not(target_os = "windows"))]
pub use crate::autostart_stub::*;

#[cfg(not(target_os = "windows"))]
mod autostart_stub {
    use std::io;

    pub fn is_autostart_enabled() -> bool {
        false
    }

    pub fn enable_autostart() -> io::Result<()> {
        Ok(())
    }

    pub fn disable_autostart() -> io::Result<()> {
        Ok(())
    }

    pub fn toggle_autostart(_enable: bool) -> io::Result<()> {
        Ok(())
    }

    pub fn autostart_target() -> Option<String> { None }

    pub fn repair_autostart() -> bool { false }
}