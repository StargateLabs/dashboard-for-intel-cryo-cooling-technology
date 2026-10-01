//! Guardia di istanza singola (Windows).
//!
//! Perché serve: due istanze dell'app aperte contemporaneamente non
//! condividono la porta COM. La seconda riceve "Accesso negato" dal probe,
//! non trova nessun TEC e mostra la schermata di selezione — un fallimento
//! che sembra un bug di rilevamento ma è un conflitto di processi.
//!
//! Il mutex è *senza nome* di sessione e vive finché il processo è vivo:
//! se il processo muore, Windows lo rilascia automaticamente, quindi non
//! restano lock "fantasma" dopo un crash.

/// Handle del mutex posseduto. Va tenuto vivo per tutta la durata del
/// programma: se viene droppato, il mutex viene rilasciato.
#[cfg(target_os = "windows")]
pub struct SingleInstance {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

// SAFETY: l'handle non viene toccato da altri thread se non dall'init,
// e il mutex è thread-safe per costruzione (API Win32).
#[cfg(target_os = "windows")]
unsafe impl Send for SingleInstance {}
#[cfg(target_os = "windows")]
unsafe impl Sync for SingleInstance {}

#[cfg(target_os = "windows")]
impl SingleInstance {
    /// Tenta di diventare l'istanza unica.
    /// `Ok` = siamo noi l'unica istanza, `Err` = ce n'è già una.
    pub fn acquire() -> Result<Self, ()> {
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
        use windows_sys::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};

        // Escape di collaudo: con CRYO_ALLOW_MULTI=1 la guardia non blocca.
        // Serve a verificare il layout lanciando una seconda istanza senza
        // dover chiudere quella dell'utente, che non va toccata.
        if std::env::var_os("CRYO_ALLOW_MULTI").is_some() {
            return Ok(SingleInstance { handle: 0 });
        }

        const NOME: &[u16] = &[
            b'L' as u16, b'o' as u16, b'c' as u16, b'a' as u16, b'l' as u16,
            b'S' as u16, b't' as u16, b'a' as u16, b't' as u16, b'e' as u16,
            b'g' as u16, b'a' as u16, b't' as u16, b'e' as u16, b'T' as u16,
            b'E' as u16, b'C' as u16, b'S' as u16, b'i' as u16, b'n' as u16,
            b'g' as u16, b'l' as u16, b'e' as u16, b'M' as u16, b'u' as u16,
            b't' as u16, b'e' as u16, b'x' as u16,
            0, // CreateMutexW requires a NUL-terminated UTF-16 string.
        ];

        unsafe {
            let h = CreateMutexW(std::ptr::null(), 1, NOME.as_ptr());
            // In windows-sys 0.48 `HANDLE` è `isize`: 0 (e non un puntatore
            // nullo) è il valore d'errore.
            if h == 0 {
                // Non riusciamo a creare il mutex: meglio procedere che
                // impedire l'avvio dell'app.
                return Ok(SingleInstance { handle: 0 });
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(h);
                return Err(());
            }
            // Attendiamo l'ownership per escludere la race tra due processi
            // lanciati nello stesso istante.
            WaitForSingleObject(h, 0);
            Ok(SingleInstance { handle: h })
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for SingleInstance {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        unsafe {
            if self.handle != 0 {
                CloseHandle(self.handle);
            }
        }
    }
}

/// Su altri sistemi la guardia è una no-op.
#[cfg(not(target_os = "windows"))]
pub struct SingleInstance;

#[cfg(not(target_os = "windows"))]
impl SingleInstance {
    pub fn acquire() -> Result<Self, ()> {
        Ok(SingleInstance)
    }
}
