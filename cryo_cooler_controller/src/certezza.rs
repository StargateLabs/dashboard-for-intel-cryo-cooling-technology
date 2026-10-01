//! Se il software sa cosa sta facendo il controller, o lo sta deducendo.
//!
//! Esiste perche' questa distinzione oggi non e' rappresentata da nessuna parte,
//! e la sua assenza e' un rischio. Il regime viene dedotto da `PID_RUNNING`, un
//! bit letto dai binari del produttore e **mai osservato cambiare** su questo
//! controller. Un errore di deduzione non si vede sui numeri — i numeri
//! descrivono il controller, non l'ipotesi sul suo regime — quindi viene
//! presentato con la stessa sicurezza di un fatto misurato.
//!
//! I tre stati non sono tre gradi di un aumento continuo. Sono tre diritti
//! diversi: `Conosciuto` permette di automatizzare, `Dedotto` permette di
//! mostrare ma non di decidere, `Ignoto` non permette di scrivere.

use std::time::Duration;

use cryo_cooler_controller_lib::TecStatus;

use crate::commutazione::Regime;

/// Quanto il software sa del regime che il controller sta tenendo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Certezza {
    /// Stato fresco **e** regime confermato da una rilettura: abbiamo visto il
    /// controller assumere il regime che gli abbiamo chiesto. Si puo'
    /// automatizzare e scrivere.
    Conosciuto,
    /// Stato fresco, ma il regime e' dedotto dai bit senza conferma. Si mostra
    /// **marcato come dedotto** e non si automatizza.
    Dedotto,
    /// Lo stato non e' fresco, o non c'e'. Non sappiamo cosa stia succedendo
    /// adesso, quindi non si scrive niente in seriale.
    Ignoto,
}

/// Oltre questo, lo stato non descrive piu' il presente.
///
/// Due secondi sono tre round mancati. E' la stessa soglia che gia' fa scattare
/// il watchdog di `poll_in_flight`, quindi non introduce un numero magico nuovo:
/// rileva lo stesso fatto da due punti di vista.
pub const SOGLIA_FRESCHEZZA: Duration = Duration::from_millis(2000);

/// Il regime corrente e quanto ne siamo certi.
///
/// `eta` e' quanto tempo e' passato dall'ultimo campione valido, e `confermato`
/// dice se l'ultima rilettura dopo una scrittura coincideva con il regime
/// richiesto. Entrambi esistono gia' in `RunningState`: non si introduce nulla,
/// si smette di ignorarli.
///
/// **Nota sul modello di partenza.** Una prima versione di questo modulo usava
/// `LAST_CMD_OK` per dichiarare `Ignoto`. Era sbagliato: `tec_status` contiene
/// solo letture riuscite, quindi quel bit non distingue mai "non ho letto" da
/// "l'ho letto ed e' fermo". `Ignoto` qui e' una questione di **freschezza**,
/// che e' l'unica cosa che puo' essere vera e non verificata.
pub fn regime_e_certezza(
    status: TecStatus,
    eta: Duration,
    confermato: bool,
) -> (Regime, Certezza) {
    if eta >= SOGLIA_FRESCHEZZA {
        return (Regime::Spento, Certezza::Ignoto);
    }
    let regime = Regime::da_stato(status).unwrap_or(Regime::Spento);
    let certezza = if confermato {
        Certezza::Conosciuto
    } else {
        Certezza::Dedotto
    };
    (regime, certezza)
}

/// Se il software puo' scrivere in seriale.
///
/// **Gate di I10: niente scritture quando lo stato e' `Ignoto`.** Cio' quando non
/// si sa cosa stia facendo il controller *adesso*.
///
/// La distinzione da `Dedotto` e' deliberata, ed e' costata una correzione.
/// Una versione precedente di questa funzione ammetteva solo `Conosciuto`, e il
/// test lo fissava. Ma `Conosciuto` richiede una conferma di rilettura, e la
/// conferma nasce solo da una scrittura: al primo avvio non e' mai successo
/// niente, quindi il sistema sarebbe partito in `Dedotto` **per sempre**, e la
/// guardia non avrebbe mai potuto scrivere potenza. La regola piu' severa non
/// era piu' sicura: rendeva il software muto.
///
/// `Dedotto` e' pero' "fresco e inferito", non "non lo so": c'e' una lettura
/// valida, e l'inferenza e' dichiarata nella UI (I9). Scrivere su dati freschi e
/// dichiarati e' diverso che scrivere alla cieca.
pub fn puo_scrivere(certezza: Certezza) -> bool {
    certezza != Certezza::Ignoto
}

#[cfg(test)]
mod test {
    use super::*;
    use cryo_cooler_controller_lib::TecStatus;

    const FRESCO: Duration = Duration::from_millis(0);
    const VECCHIO: Duration = Duration::from_millis(60_000);

    fn stato_cryo() -> TecStatus {
        TecStatus::PID_RUNNING | TecStatus::TEMP_MODE
    }

    /// **Uno stato vecchio non e' un regime.** E' la difesa contro la riga
    /// verde falsa: il numero era valido *prima*, e se ne continua a mostrare
    /// la validita' di adesso.
    #[test]
    fn uno_stato_vecchio_e_ignoto() {
        let (regime, certezza) = regime_e_certezza(stato_cryo(), VECCHIO, true);
        assert_eq!(certezza, Certezza::Ignoto, "vecchio non e' mai conosciuto");
        assert_eq!(regime, Regime::Spento, "non si dichiara un regime vecchio");
        assert!(!puo_scrivere(certezza));
    }

    /// **Stato fresco ma non confermato = dedotto.** E' lo stato normale
    /// all'avvio: il controller ha parlato, ma nessuno ha ancora scritto niente
    /// e verificato che la mappatura sia quella giusta.
    #[test]
    fn stato_fresco_non_confermato_e_dedotto() {
        let (regime, certezza) = regime_e_certezza(stato_cryo(), FRESCO, false);
        assert_eq!(regime, Regime::Cryo);
        assert_eq!(certezza, Certezza::Dedotto);
        // **Un dedotto autorizza ancora a scrivere.** E' la correzione: vedi la
        // nota su `puo_scrivere`. Il divieto e' per `Ignoto`, non per `Dedotto`.
        assert!(
            puo_scrivere(certezza),
            "stato fresco e dichiarato: si puo' scrivere"
        );
    }

    /// **Confermato e fresco = conosciuto.** E' l'unico stato che autorizza a
    /// scrivere, e ci si arriva solo dopo aver visto il controller assumere il
    /// regime richiesto. La certezza si guadagna, non si presume.
    #[test]
    fn stato_fresco_confermato_e_conosciuto() {
        let (_, certezza) = regime_e_certezza(stato_cryo(), FRESCO, true);
        assert_eq!(certezza, Certezza::Conosciuto);
        assert!(puo_scrivere(certezza), "confermato e fresco: si puo' scrivere");
    }

    /// **Standby eroga potenza.** E' il punto su cui il software precedente
    /// sbagliava: derivava "acceso" da `LOW_POWER_MODE` negato, cosi' in
    /// Standby la guardia perdeva l'autorizzazione a scrivere proprio nello
    /// stato in cui il modulo funziona.
    #[test]
    fn lo_standby_eroga_potenza() {
        assert!(Regime::Standby.tec_acceso());
    }

    #[test]
    fn solo_lo_spento_non_eroga_potenza() {
        for r in [Regime::Standby, Regime::Cryo, Regime::Unregulated] {
            assert!(r.tec_acceso(), "{r} deve erogare potenza");
        }
        assert!(!Regime::Spento.tec_acceso());
    }

    /// **`LOW_POWER_MODE` da solo non decide piu' niente.** Il difetto era
    /// esattamente questo: un bit che descrive *standby* usato come se
    /// descrivesse *alimentazione*.
    #[test]
    fn low_power_da_solo_non_decide_l_alimentazione() {
        let s = TecStatus::LOW_POWER_MODE_ACTIVE;
        let (regime, _) = regime_e_certezza(s, FRESCO, true);
        assert_eq!(regime, Regime::Spento, "il PID non gira: non eroga");
    }

    /// Il modulo fermo ma letto di fresco e' *dedotto spento*, non ignoto: la
    /// differenza conta, perche' su dati freschi l'operatore puo' ancora agire.
    #[test]
    fn spento_ma_letto_e_dedotto() {
        let (regime, certezza) = regime_e_certezza(TecStatus::POWER_OK, FRESCO, false);
        assert_eq!(regime, Regime::Spento);
        assert_eq!(certezza, Certezza::Dedotto);
    }
}
