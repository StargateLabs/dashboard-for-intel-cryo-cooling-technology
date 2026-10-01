//! Valutazione dei flag di stato del controller TEC.
//!
//! Il controller comunica 18 bit di stato (`TecStatus`). Fino ad ora
//! l'app ne controllava 5: se il TEC si scollasse, il sensore della board
//! guastasse, il protocollo si corrompesse o il PID fosse invalido, la
//! dashboard avrebbe continuato a mostrare numeri con un aspetto normale,
//! dando l'impressione che tutto andasse bene.
//!
//! Qui ogni bit viene tradotto in un problema comprensibile, con la
//! gravità corretta: non tutte le anomalie hanno la stessa importanza.

use cryo_cooler_controller_lib::TecStatus;

/// Gravità di un'anomalia rilevata sul controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Attenzione: degrado ma il sistema è comunque sicuro.
    Warning,
    /// Pericolo: va segnalato subito all'utente.
    Critical,
}

/// Un'anomalia rilevata, pronta per la UI.
pub struct Issue {
    pub severity: Severity,
    /// Etichetta breve mostrata nella UI.
    pub label: &'static str,
    /// Cosa fare. È la parte che l'utente cerca davvero.
    pub advice: &'static str,
    /// `true` se questa anomalia ha senso **solo mentre il modulo lavora**.
    ///
    /// Serve a non generare allarmi per uno spegnimento volontario. Ci sono
    /// due famiglie di bit, e confonderle produce falsi positivi:
    ///
    /// - **Integrità**: comunicazione, alimentazione, sensori, connessione
    ///   TEC. Valgono sempre, e quando il modulo è spento sono *più*
    ///   importanti: un controller fermo con l'alimentazione assente è un
    ///   guantoio, non uno spegnimento.
    /// - **Regolamento**: `TEMP_MODE`, `LOW_POWER_MODE`, `PID_READY`,
    ///   `PID_INVALID`, `PID_OUT_OF_RANGE`. Descrivono *come* il controller
    ///   regola, e con il regolatore fermo non descrivono niente:
    ///   `TEMP_MODE` resta spesso acceso perché è l'ultimo regime registrato.
    ///
    /// Il default è `false` (integrità): si marca `true` solo le poche voci
    /// che riguardano il regolamento, così un nuovo bit non viene silenziosamente
    /// escluso dalla diagnostica perché qualcuno ha dimenticato il flag.
    pub solo_in_funzione: bool,
}

/// Legge i bit di stato e restituisce le anomalie, dalla più grave in giù.
///
/// L'ordine è quello in cui vanno mostrati: prima ciò che può sporcare le
/// misure (errori di comunicazione), poi ciò che riguarda la sicurezza
/// (corrente, fail-safe, sensori), infine lo stato del regolatore.
pub fn evaluate(status: TecStatus) -> Vec<Issue> {
    let mut out = Vec::new();

    // ── Integrità della comunicazione ──────────────────────────────────
    // Se la link seriale è corrotta, i valori mostrati non sono fidabili:
    // è più importante di qualsiasi altro indicatore, perché rende
    // fuorvianti TUTTI gli altri numeri della dashboard.
    if status.contains(TecStatus::LAST_CMD_BAD_CRC) {
        out.push(Issue {
            severity: Severity::Critical,
            label: "CRC seriale errato",
            advice: "I valori mostrati potrebbero non corrispondere alla realta'. \
                     Controlla il cavo USB e riduci la lunghezza del cavo o usa una porta diversa.",
            solo_in_funzione: false,
        });
    }
    if status.contains(TecStatus::LAST_CMD_INCOMPLETE) {
        out.push(Issue {
            severity: Severity::Critical,
            label: "Risposta seriale incompleta",
            advice: "Tipicamente rumore elettrico o porta USB instabile. Ricollega il cavo.",
            solo_in_funzione: false,
        });
    }

    // ── Sicurezza elettrica ────────────────────────────────────────────
    if status.contains(TecStatus::OCP_ACTIVE) {
        out.push(Issue {
            severity: Severity::Warning,
            label: "OCP attivo",
            // **Non consigliare di riavviare, e non parlare di radiatori
            // e pompa.** Il consiglio precedente diceva "verifica che il
            // radiatore/pompa siano collegati al controller e non alla
            // scheda madre", ed è falso su questo impianto: ventola e
            // pompa le comanda l'utente a velocità fissa, e il connettore
            // a 5 pin della board fa solo passare il segnale PWM dalla
            // scheda madre (vedi `docs/analisi-cella-peltier.md` § 9.3).
            //
            // Un consiglio che descrive un'altra macchina fa perdere tempo:
            // l'utente va a controllare collegamenti che sono già a posto.
            advice: "Su questo impianto il bit è rumore: si accende anche a 73 W \
                     con il raffreddamento che funziona. Nessuna azione. Se compare \
                     l'errore CB1 a schermo, allora il guasto è reale.",
            solo_in_funzione: false,
        });
    }
    if status.contains(TecStatus::FAILSAFE_ACTIVE) {
        out.push(Issue {
            severity: Severity::Critical,
            label: "Fail-safe attivo",
            advice: "Leggi il manuale: il fail-safe si attende se il circuito di raffreddamento \
                     non e' collegato correttamente o se una temperatura e' fuori limite.",
            solo_in_funzione: false,
        });
    }
    if !status.contains(TecStatus::POWER_OK) {
        out.push(Issue {
            severity: Severity::Critical,
            label: "Alimentazione KO",
            advice: "Verifica l'alimentazione PSU e il cavo di potenza del TEC.",
            solo_in_funzione: false,
        });
    }

    // ── Sensori ────────────────────────────────────────────────────────
    // Il blocco freddo del TEC trasferisce calore: se il refrigerante non
    // gira, la piastra si scalda senza che nessuno se ne accorga, e il
    // danno e' irreversibile.
    if !status.contains(TecStatus::TEC_CONN_OK) {
        out.push(Issue {
            severity: Severity::Critical,
            label: "TEC non collegato",
            advice: "Controlla il connettore del TEC e il cavo verso la piastra. \
                     Non lasciare il TEC attivo senza refrigerante.",
            solo_in_funzione: false,
        });
    }
    if !status.contains(TecStatus::BOARD_TEMP_OK) {
        out.push(Issue {
            severity: Severity::Critical,
            label: "Sensore board KO",
            advice: "La protezione termica del controller si basa su questo sensore: \
                     finche' e' KO, ferma il TEC e verifica il cablaggio.",
            solo_in_funzione: false,
        });
    }
    if !status.contains(TecStatus::TEMP_SENSE_OK) {
        out.push(Issue {
            severity: Severity::Warning,
            label: "Sensore temp. KO",
            advice: "Il punto di rugiada e il margine di condensa non sono affidabili.",
            solo_in_funzione: false,
        });
    }
    if !status.contains(TecStatus::HUM_SENSE_OK) {
        out.push(Issue {
            severity: Severity::Warning,
            label: "Sensore umidita' KO",
            advice: "La protezione anticondensa usa l'umidita': senza, il margine di \
                     sicurezza non puo' essere calcolato correttamente.",
            solo_in_funzione: false,
        });
    }

    // ── Regolatore ─────────────────────────────────────────────────────
    if status.contains(TecStatus::PID_INVALID) {
        out.push(Issue {
            severity: Severity::Critical,
            label: "PID non valido",
            advice: "Reimposta i coefficienti dalla sidebar e osserva l'etichetta FW: \
                     deve mostrare il valore che hai impostato.",
            solo_in_funzione: true,
        });
    }
    if status.contains(TecStatus::PID_OUT_OF_RANGE) {
        out.push(Issue {
            severity: Severity::Critical,
            label: "PID fuori intervallo",
            advice: "Nel dubbio ripristina i valori predefiniti: la TEC era tarata di fabbrica.",
            solo_in_funzione: true,
        });
    }
    if status.contains(TecStatus::PID_DEFAULT) {
        out.push(Issue {
            severity: Severity::Warning,
            label: "PID di fabbrica",
            advice: "Se hai modificato P/I/D, non sono stati applicati: ricontrolla.",
            solo_in_funzione: false,
        });
    }
    if status.contains(TecStatus::LOW_POWER_MODE_ACTIVE) {
        out.push(Issue {
            severity: Severity::Warning,
            label: "Modalita' basso consumo",
            advice: "Le prestazioni sono ridotte finche' la modalita' resta attiva.",
            solo_in_funzione: true,
        });
    }

    out.sort_by(|a, b| b.severity.cmp(&a.severity));
    out
}


