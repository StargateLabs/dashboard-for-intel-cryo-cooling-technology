//! I bit di stato che il protocollo scarta.
//!
//! Il controller risponde al heartbeat con un campo a 32 bit. Fino a ora
//! l'app ne leggeva 18, e i restanti 14 venivano mascherati via prima di
//! `from_bits`: il risultato era che `from_bits` non poteva mai fallire,
//! e la validazione era codice morto.
//!
//! Il manuale EK documenta codici errore (CB1, CB2, CF1..CF7, OT1..OT3,
//! DT1, TD1) che non corrispondono a nessuno dei 18 bit noti: e' plausibile
//! che stiano proprio in questi 14.
//!
//! **Qui non li interpretiamo.** Non sappiamo cosa significhino, e inventare
//! una decodifica sarebbe peggio che non averla: un'etichetta sbagliata in
//! diagnostica fa diagnosticare il problema sbagliato. Si registrano, e la
//! correlazione li chiarisce.

use crate::TecStatus;

/// I 18 bit che hanno un nome noto. Tutti gli altri del campo a 32 bit
/// sono "alternativi": non decodificati, conservati e registrati.
pub const CAMPO_STATO_CONOSCIUTO: u32 = 0b0000_0000_0000_0011_1111_1111_1111_1111;

/// Il campo di stato separato nelle due parti che ci servono.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusCompleto {
    /// I 18 bit con significato noto, decodificati.
    pub noti: TecStatus,
    /// I bit 18..=31, **senza interpretazione**. Vanno da 0 a 0x3FFF.
    pub bit_alternativi: u16,
}

/// Separa i bit noti dai bit alternativi.
///
/// Non fallisce mai: il campo e' a 32 bit e non sappiamo cosa contenga
/// negli spazi non mappati. Un campo di stato con un bit inaspettato non
/// e' un errore di comunicazione, e trattarlo come tale farebbe
/// scollegare un controller perfettamente sano.
pub fn scompone(codice: u32) -> StatusCompleto {
    StatusCompleto {
        noti: TecStatus::from_bits_truncate(codice & CAMPO_STATO_CONOSCIUTO),
        bit_alternativi: ((codice >> 18) & 0x3FFF) as u16,
    }
}

impl StatusCompleto {
    /// Una riga CSV: timestamp, poi i bit alternativi in esadecimale.
    ///
    /// Solo i bit alternativi: i noti sono gia' visibili nella dashboard e
    /// duplicarli qui raddopperebbe il file senza aggiungere informazione.
    pub fn riga_csv(&self, timestamp: &str) -> String {
        format!("{timestamp},{:04X}", self.bit_alternativi)
    }
}

/// Intestazione del CSV dei bit alternativi.
pub fn intestazione_csv() -> &'static str {
    "timestamp,bit_alternativi"
}

#[cfg(test)]
mod test {
    use super::*;

    /// I 18 bit noti non devono finire nei "bit alternativi": sono gia'
    /// decodificati, e contarli due volte nasconderebbe un errore di mask.
    #[test]
    fn i_bit_noti_non_sono_nei_bit_alternativi() {
        let tutti_noti = CAMPO_STATO_CONOSCIUTO;
        let s = scompone(tutti_noti);
        assert_eq!(s.noti, TecStatus::all(), "i 18 bit noti devono tornare tutti");
        assert_eq!(s.bit_alternativi, 0, "nessun bit noto deve finire negli alternativi");
    }

    /// Il caso che ha motivato il task: il campo a 32 bit contiene anche
    /// bit oltre i 18 noti, e il mask non deve scambiarli per un errore.
    #[test]
    fn i_bit_alternativi_vengono_conservati() {
        let alto = 1u32 << 18;
        let s = scompone(alto);
        assert_eq!(s.bit_alternativi, 0b01, "il bit 18 deve essere conservato");
    }

    /// `from_bits` poteva fallire solo su un bit noto sbagliato: con il
    /// mask a 18 bit non poteva mai fallire, quindi la validazione era
    /// codice morto. `scompone` non deve fallire su nessun valore.
    #[test]
    fn nessun_valore_causa_errore() {
        for codice in [0u32, u32::MAX, 0xDEAD_BEEF, 1 << 17, 1 << 31] {
            let _ = scompone(codice);
        }
    }

    /// I 18 noti e i 14 alternativi devono ricostruire il campo originale
    /// bit per bit: e' la prova che il mask non perde nulla.
    #[test]
    fn la_scomposizione_e_completa() {
        for codice in [0u32, 0x3_FFFF, 0x4_0000, u32::MAX, 0x8001_2345] {
            let s = scompone(codice);
            let ricostruito = s.noti.bits() | ((s.bit_alternativi as u32) << 18);
            assert_eq!(ricostruito, codice, "bit persi nella scomposizione di {codice:#010x}");
        }
    }

    /// Il CSV deve poter essere correlato: una colonna per i bit alternativi
    /// in esadecimale. `0x8_0000` e' il bit 19: shiftato di 18 diventa 2.
    #[test]
    fn il_csv_ha_una_colonna_per_i_bit_alternativi() {
        let s = scompone(0x8_0000);
        let riga = s.riga_csv("2026-09-28T10:00:00Z");
        assert!(riga.starts_with("2026-09-28T10:00:00Z,"), "manca il timestamp: {riga}");
        assert!(riga.contains("0002"), "i bit alternativi non sono 0002: {riga}");
    }
}
