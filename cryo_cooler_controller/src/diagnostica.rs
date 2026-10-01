//! Procedura diagnostica **osservativa**.
//!
//! # Cosa NON e' questo modulo
//!
//! Il manuale EK (4.4) descrive un *built-in self test* **attivo**, eseguito dal
//! controller su comando dell'host: richiede PC inattivo da almeno un minuto,
//! dura meno di 30 s e produce `sac.diagnosis.YYYY-MM-DD.log`.
//!
//! Quel comando **non e' nel protocollo che conosciamo**: le scritture note sono
//! offset, PID, potenza, temperatura CPU, enable e disable. Nessuna imposta la
//! modalita' ne' avvia un test. Mandare un opcode a caso su una seriale
//! funzionante e' una scommessa, e in questo protocollo `0x1E` con dati nulli
//! fa il reset di fabbrica.
//!
//! Questo modulo guarda **solo dati gia' letti** e giudica lo stato di salute.
//! Non e' il self test del produttore, e il log lo dichiara: un utente che lo
//! invia al supporto EK deve sapere cosa ha inviato.
//!
//! # Cosa fa
//!
//! Raccoglie i tre campi del report ufficiale (temperatura iniziale, finale,
//! potenza massima) piu' i valori elettrici e lo stato dei 18 bit noti, e
//! produce un verdetto con l'elenco dei problemi e cosa fare per ciascuno.

use cryo_cooler_controller_lib::TecStatus;

/// Il verdetto. Tre livelli, non due: "Ok" e "Guasto" non bastano, perche'
/// l'OCP osservato e' rumore ma e' comunque un segnale da mostrare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdetto {
    Ok,
    Attenzione,
    Guasto,
}

/// Un dato grezzo da cui giudicare. `None` = non disponibile.
#[derive(Debug, Clone, Default)]
pub struct Esito {
    pub temp_iniziale_c: Option<f32>,
    pub temp_finale_c: Option<f32>,
    pub potenza_max_w: Option<f32>,
    pub watts: Option<f32>,
    /// Tensione, per il riepilogo elettrico.
    pub volts: Option<f32>,
    /// Corrente, per il riepilogo elettrico.
    pub amps: Option<f32>,
    /// COP **gia' calcolato** da `CopState::calculate`. Non ricalcolarlo qui:
    /// la formula vive in `analytics.rs` e duplicarla la farebbe divergere.
    pub cop: Option<f32>,
    pub margine_condensa: Option<f32>,
    pub modalita: Option<String>,
}

impl Esito {
    /// Stato vuoto, con la costruzione esplicita perche' `Default`
    /// derivato richiederebbe di ricordare quali campi sono `Option`.
    #[cfg(test)]
    pub fn vuoto() -> Self {
        Self::default()
    }
}

/// Dove finiscono le righe di diagnostica.
///
/// Mostrato a schermo nel pannello. L'operatore deve poter trovare il file
/// senza aprire il codice, e senza dover ricordare il percorso a memoria.
///
/// `%LOCALAPPDATA%` e' scritto per esteso e non espanso, perche' a schermo
/// serve il modello del percorso, non la cartella di questo PC: il file va
/// cercato anche su un'altra macchina.
pub const SORGENTE_LOG: &str = "%LOCALAPPDATA%\\stargate-cryo\\diagnostica.log";

/// Current faults use current samples; historical minima belong to session statistics.
pub fn margine_attuale(count: u64, margin: f32) -> Option<f32> {
    (count > 0 && margin.is_finite()).then_some(margin)
}

#[cfg(test)]
mod test_margine_attuale {
    use super::*;
    #[test] fn resolved_condensation_is_not_a_current_fault() {
        let status = TecStatus::POWER_OK | TecStatus::TEC_CONN_OK | TecStatus::BOARD_TEMP_OK
            | TecStatus::TEMP_SENSE_OK | TecStatus::HUM_SENSE_OK | TecStatus::LAST_CMD_OK
            | TecStatus::PID_RUNNING | TecStatus::OCP_ACTIVE;
        let mut measurements=Esito::default();
        measurements.margine_condensa=margine_attuale(20,2.8);
        let current=valuta(measurements,status);
        assert_eq!(current.verdetto,Verdetto::Attenzione);
        assert_eq!(current.problemi.len(),1);
        assert_eq!(margine_attuale(0,2.8),None);
        assert_eq!(margine_attuale(20,f32::NAN),None);
    }
}

/// Una misura mostrata, con il suo giudizio.
#[derive(Debug, Clone)]
pub struct Misura {
    pub etichetta: &'static str,
    pub valore: String,
    pub giudizio: Verdetto,
    /// `true` se e' un aggregato della sessione, non una misura di adesso.
    ///
    /// Serve a decidere **dove** la si mostra. "Temperatura minima" e
    /// "Potenza massima" sono gia' in "Statistiche sessione" nella sidebar,
    /// quindi ripeterle nel pannello diagnostico e' la stessa duplicazione che
    /// questo intervento toglie. Il **file** invece le tiene tutte: e' il
    /// report da confrontare con il self test del produttore, che riporta
    /// proprio quei tre campi, e un report incompleto non serve a niente.
    pub aggregata: bool,
}

impl EsitoDiagnostica {
    /// Le misure di adesso: quelle che la sidebar **non** mostra gia'.
    ///
    /// La regola "un dato una volta sola" e' applicata qui, in una funzione
    /// pura, cosi' e' testabile: il pannello non puo' reintrodurre la
    /// duplicazione perche' non vede i campi duplicati.
    pub fn misure_istantanee(&self) -> Vec<&Misura> {
        self.misure.iter().filter(|m| !m.aggregata).collect()
    }
}

/// Il risultato della valutazione.
pub struct EsitoDiagnostica {
    pub verdetto: Verdetto,
    pub misure: Vec<Misura>,
    /// I problemi, ognuno con cosa fare. La parte che l'utente cerca.
    pub problemi: Vec<String>,
    pub modalita: Option<String>,
    /// `true` se il regolatore e' fermo.
    ///
    /// Non e' un verdetto: e' una condizione. Serve perche' `Ok` da solo e'
    /// ambiguo con il modulo spento — nessun problema, ma nemmeno un
    /// controller che sta raffreddando — e l'operatore deve poter distinguere
    /// "tutto regolare" da "spento, e va bene cosi'".
    pub spento: bool,
}

impl EsitoDiagnostica {
    /// Una riga di log leggibile.
    ///
    /// La prima parte dichiara che questo **non** e' il self test del
    /// produttore. E' la cosa piu' importante del file: senza, il report
    /// puo' essere mandato al supporto come se fosse quello ufficiale.
    pub fn riga_log(&self, timestamp: &str) -> String {
        let v = match self.verdetto {
            Verdetto::Ok => "OK",
            Verdetto::Attenzione => "ATTENZIONE",
            Verdetto::Guasto => "GUASTO",
        };
        let mut s = format!(
            "{timestamp} | {v} | osservativo: NON e' il self test del produttore"
        );
        if let Some(m) = &self.modalita {
            s.push_str(&format!(" | modalita' {m}"));
        }
        for m in &self.misure {
            s.push_str(&format!(" | {}: {}", m.etichetta, m.valore));
        }
        for p in &self.problemi {
            s.push_str(&format!(" | {p}"));
        }
        s
    }
}

/// Cosa mostra la **sidebar**: verdetto e numero di problemi. Nient'altro.
///
/// Esiste per togliere la duplicazione. La sidebar mostra gia' tensione,
/// corrente, potenza, COP, margine di condensa e modalita' in altre sezioni:
/// ripeterle qui dentro faceva leggere gli stessi numeri due volte a due
/// schermate di distanza, e spingeva "Impostazioni" sotto la piega.
///
/// Quindi il riepilogo e' **solo un verdetto piu' un conteggio**: il dettaglio
/// e' nel pannello diagnostico, che si apre e li mostra tutti.
///
/// Non contiene i valori, e un test lo verifica: se domani un numero qui, la
/// sidebar duplica di nuovo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Riassunto {
    /// Etichetta pronta per la UI, senza congiuntivi inutili.
    pub etichetta: String,
    /// Quanti problemi ha trovato `valuta`.
    pub problemi: usize,
    /// Il modulo e' fermo: la riga lo dichiara, perche' `Ok` da solo e'
    /// ambiguo.
    pub spento: bool,
}

impl EsitoDiagnostica {
    /// Il riepilogo di una riga per la sidebar.
    pub fn riassunto(&self) -> Riassunto {
        // **Il modulo spento ha un'etichetta sua**, non "nessuna anomalia".
        //
        // Il difetto: con il regolatore fermo non ci sono problemi, quindi il
        // match qui sotto finiva inevitabilmente in "Nessuna anomalia" — che è
        // falso nel senso che importa. Il controller non sta raffreddando, e
        // l'operatore deve saperlo dalla riga che guarda ogni giorno, non
        // dedurlo dall'assenza di allarmi.
        //
        // Il caso va trattato *prima* del match, non dentro: nessuna delle
        // quattro coppie (verdetto, numero di problemi) distingue "acceso e
        // sano" da "spento", perché sono lo stesso verdetto con lo stesso
        // conteggio. Serviva un campo che dicesse *perché* non ci sono problemi.
        let etichetta = if self.problemi.is_empty() && self.spento {
            "Modulo spento".to_owned()
        } else {
            match (self.verdetto, self.problemi.len()) {
                (Verdetto::Ok, _) | (_, 0) => "Nessuna anomalia".to_owned(),
                (Verdetto::Attenzione, 1) => "1 segnale da controllare".to_owned(),
                (Verdetto::Attenzione, n) => format!("{n} segnali da controllare"),
                (Verdetto::Guasto, 1) => "GUASTO: 1 problema".to_owned(),
                (Verdetto::Guasto, n) => format!("GUASTO: {n} problemi"),
            }
        };
        Riassunto {
            etichetta,
            problemi: self.problemi.len(),
            spento: self.spento,
        }
    }
}

#[cfg(test)]
mod test_misure_istantanee {
    use super::*;

    fn stato_sano() -> TecStatus {
        TecStatus::POWER_OK
            | TecStatus::TEC_CONN_OK
            | TecStatus::BOARD_TEMP_OK
            | TecStatus::TEMP_SENSE_OK
            | TecStatus::HUM_SENSE_OK
            | TecStatus::LAST_CMD_OK
    }

    /// **Il pannello non puo' mostrare quello che la sidebar mostra gia'.**
    ///
    /// "Statistiche sessione" in sidebar da' gia' temperatura minima, potenza
    /// massima e margine minimo. Se il pannello diagnostico li ripete, quei
    /// tre numeri compaiono due volte a due schermate di distanza e
    /// l'operatore non sa quale sia quello aggiornato. E' la stessa duplicazione
    /// che aveva riempito la sidebar di V, A, W e COP, vista dall'altra parte.
    #[test]
    fn il_pannello_non_ripete_gli_aggregati_che_la_sidebar_ha_gia() {
        let mut m = Esito::vuoto();
        m.temp_iniziale_c = Some(30.0);
        m.temp_finale_c = Some(11.4);
        m.potenza_max_w = Some(237.0);
        m.volts = Some(19.4);
        m.amps = Some(5.8);
        m.watts = Some(112.5);
        m.cop = Some(1.7);
        let e = valuta(m, stato_sano());

        let sul_pannello: Vec<&str> =
            e.misure_istantanee().iter().map(|x| x.etichetta).collect();
        for gia_in_sidebar in ["Temp. iniziale", "Temp. minima", "Potenza massima"] {
            assert!(
                !sul_pannello.iter().any(|x| x.contains(gia_in_sidebar)),
                "'{gia_in_sidebar}' e' gia' in Statistiche sessione, nel pannello \
                 e' un duplicato: {sul_pannello:?}"
            );
        }
        // E i quattro che la sidebar non ha devono restare.
        for solo_pannello in ["Tensione", "Corrente", "Potenza", "COP"] {
            assert!(
                sul_pannello.iter().any(|x| x.contains(solo_pannello)),
                "'{solo_pannello}' non compare da nessun'altra parte, deve stare \
                 nel pannello: {sul_pannello:?}"
            );
        }
    }

    /// **Nascondere un dato dal pannello non puo' farlo sparire dal report.**
    ///
    /// Il file e' quello da allegare al supporto, e i tre campi che il self
    /// test del produttore riporta sono proprio temperatura iniziale,
    /// temperatura minima e potenza massima. Se il pannello non li mostra piu'
    /// e il file pure, il report non e' confrontabile con quello ufficiale e
    /// non serve a niente.
    #[test]
    fn il_report_tiene_tutti_i_campi_anche_quelli_aggrecati() {
        let mut m = Esito::vuoto();
        m.temp_iniziale_c = Some(30.0);
        m.temp_finale_c = Some(11.4);
        m.potenza_max_w = Some(237.0);
        m.volts = Some(19.4);
        let e = valuta(m, stato_sano());
        let riga = e.riga_log("2026-09-28T10:00:00Z");
        for campo in ["30.0", "11.4", "237.0", "19.4"] {
            assert!(
                riga.contains(campo),
                "il report perde '{campo}': {riga}"
            );
        }
    }

    /// **Nessun numero inventato quando il controller non ha mandato niente.**
    ///
    /// Il pannello mostra COP anche senza dati, perche' il COP richiede la
    /// temperatura CPU esterna e senza carico non e' calcolabile: dire
    /// "n/d" e' informazione, "0.00" sarebbe una misura falsa. Le altre
    /// misure invece non devono comparire affatto: `None` resta assente, e
    /// una griglia di righe a zero sembrerebbe un controller spento.
    #[test]
    fn senza_dati_il_pannello_non_inventa_numeri() {
        let e = valuta(Esito::vuoto(), stato_sano());
        let sul_pannello = e.misure_istantanee();
        assert_eq!(
            sul_pannello.len(),
            1,
            "senza dati il pannello deve mostrare solo COP: {:?}",
            sul_pannello.iter().map(|m| m.etichetta).collect::<Vec<_>>()
        );
        assert_eq!(sul_pannello[0].etichetta, "COP (stima)");
        assert_eq!(
            sul_pannello[0].valore, "n/d",
            "COP assente non si stampa come zero"
        );
        // E nessuna tensione o corrente inventata.
        for assente in ["Tensione", "Corrente", "Potenza"] {
            assert!(
                !sul_pannello.iter().any(|m| m.etichetta.contains(assente)),
                "'{assente}' e' comparso senza che il controller mandasse nulla"
            );
        }
    }
}

#[cfg(test)]
mod test_modulo_spento {
    use super::*;

    fn stato_sano() -> TecStatus {
        TecStatus::POWER_OK
            | TecStatus::TEC_CONN_OK
            | TecStatus::BOARD_TEMP_OK
            | TecStatus::TEMP_SENSE_OK
            | TecStatus::HUM_SENSE_OK
            | TecStatus::LAST_CMD_OK
    }

    /// **Un modulo spento non e' un guasto.**
    ///
    /// `PID_INVALID` e un bit di **regolamento**: descrive come il controller
    /// regola, e con il regolatore fermo non descrive niente. Senza questo
    /// filtro la diagnostica diceva "GUASTO" per un controller che l'operatore
    /// aveva appena spento lui — e la dashboard che dovrebbe aiutare a capire
    /// lo stato diventava lei stessa la fonte di un allarme inventato.
    #[test]
    fn il_pid_invalido_a_modulo_spento_non_e_un_guasto() {
        let spento = stato_sano() | TecStatus::PID_INVALID;
        // Il modulo e' spento: nessun PID in funzione.
        assert!(!spento.contains(TecStatus::PID_RUNNING));
        let e = valuta(Esito::vuoto(), spento);
        assert_eq!(
            e.verdetto,
            Verdetto::Ok,
            "spegnere il modulo non e' un guasto: {:?}", e.problemi
        );
        assert!(
            !e.problemi.iter().any(|p| p.contains("PID")),
            "i bit di regolamento non devono generare problemi a spento: {:?}",
            e.problemi
        );
    }

    /// **Ma i bit d'integrita' valgono anche da spenti.**
    ///
    /// E' la direzione opposta, ed e' quella che rende utile la diagnostica: un
    /// controller fermo con l'alimentazione assente o il TEC scollegato e' un
    /// guantoio, non uno spegnimento. Se il filtro fosse troppi, il difetto
    /// sarebbe peggiore: l'operatore crede che sia tutto a posto e non
    /// controlla.
    #[test]
    fn i_problemi_di_integrita_restano_a_modulo_spento() {
        let spento = stato_sano() & !TecStatus::POWER_OK;
        let e = valuta(Esito::vuoto(), spento);
        assert_eq!(
            e.verdetto,
            Verdetto::Guasto,
            "l'alimentazione manca: il filtro non deve coprire i guanti veri"
        );
        assert!(
            e.problemi.iter().any(|p| p.contains("Alimentazione")),
            "{:?}",
            e.problemi
        );
    }

    /// I due filtri non devono sovrapporsi: `LOW_POWER_MODE` e' regolamento e
    /// sparisce a spento, l'OCP e' sicurezza elettrica e resta.
    ///
    /// Il caso e' cambiato perche' il criterio di "spento" e' cambiato: ora
    /// l'OCP attivo **esclude** lo spento (un modulo in overcurrent sta
    /// lavorando), quindi non si puo' piu' costruire "spento con OCP". Il
    /// test verifica quindi la separazione con i due filtri al loro posto
    /// naturale: il regolamento sparisce a spento, la sicurezza no.
    #[test]
    fn il_filtro_separa_regolamento_da_sicurezza() {
        // Spento vero: nessun OCP, nessun PID. Qui `LOW_POWER_MODE` e'
        // regolamento e sparisce.
        let spento = stato_sano() | TecStatus::LOW_POWER_MODE_ACTIVE;
        let e = valuta(Esito::vuoto(), spento);
        assert!(
            !e.problemi.iter().any(|p| p.contains("basso consumo")),
            "il riposo a spento non e' un problema: {:?}",
            e.problemi
        );

        // OCP attivo: il modulo **lavora**, quindi i bit di regolamento non si
        // possono considerare "a riposo" e l'OCP resta comunque un problema
        // di sicurezza elettrica.
        let in_lavoro = stato_sano() | TecStatus::OCP_ACTIVE;
        let e2 = valuta(Esito::vuoto(), in_lavoro);
        assert!(
            e2.problemi.iter().any(|p| p.contains("OCP")),
            "l'OCP non e' un problema di regolamento: deve restare"
        );
    }
}

#[cfg(test)]
mod test_riassunto_sidebar {
    use super::*;

    fn stato_sano() -> TecStatus {
        TecStatus::POWER_OK
            | TecStatus::TEC_CONN_OK
            | TecStatus::BOARD_TEMP_OK
            | TecStatus::TEMP_SENSE_OK
            | TecStatus::HUM_SENSE_OK
            | TecStatus::LAST_CMD_OK
    }

    /// **La riga della sidebar non deve contenere nessuna misura.**
    ///
    /// Questo e' il test che chiude la duplicazione: la griglia principale
    /// mostra gia' tensione, corrente, potenza e COP, e la sezione modalita'
    /// mostra la modalita'. Se il riepilogo riporta un numero, l'operatore
    /// legge lo stesso valore due volte e non sa quale sia aggiornato.
    #[test]
    fn il_riassunto_non_riporta_nessuna_misura() {
        let mut m = Esito::vuoto();
        m.volts = Some(19.4);
        m.amps = Some(5.8);
        m.watts = Some(112.5);
        m.cop = Some(1.7);
        m.margine_condensa = Some(0.4);
        m.modalita = Some("Unregulated".to_owned());
        let e = valuta(m, stato_sano() | TecStatus::OCP_ACTIVE);
        let r = e.riassunto();
        for valore in ["19.4", "5.8", "112.5", "1.7", "0.4", "Unregulated"] {
            assert!(
                !r.etichetta.contains(valore),
                "il riepilogo sidebar riporta '{valore}': '{}'",
                r.etichetta
            );
        }
    }

    /// Una riga sola: se non ci sta in una riga della sidebar, non ci sta
    /// affatto, e finisce per spostare i controlli sotto la piega.
    #[test]
    fn il_riassunto_sta_in_una_riga() {
        let mut m = Esito::vuoto();
        m.margine_condensa = Some(-2.0);
        let e = valuta(m, stato_sano());
        let r = e.riassunto();
        assert!(
            r.etichetta.chars().count() <= 24,
            "etichetta troppo lunga ({} caratteri): '{}'",
            r.etichetta.chars().count(),
            r.etichetta
        );
    }

    /// Tutto regolare: nessun problema, e la riga lo dice. Una riga che
    /// tace quando non c'e' niente lascia l'operatore a chiedersi se il
    /// controllo sia girato.
    ///
    /// Nota il `PID_RUNNING` aggiunto: questo test parla del caso *acceso e
    /// sano*, e senza quel bit lo stato non descrive un modulo che sta
    /// raffreddando — sarebbe quello spento, che ha un'etichetta diversa (vedi
    /// `il_modulo_spento_ha_una_etichetta_sua`). Il test in forma precedente
    /// passava per un caso che non esisteva: non poteva accorgersi che stesse
    /// controllando lo stato sbagliato.
    #[test]
    fn senza_problemi_dice_che_e_a_posto() {
        let acceso = valuta(Esito::vuoto(), stato_sano() | TecStatus::PID_RUNNING);
        let r = acceso.riassunto();
        assert_eq!(r.problemi, 0);
        assert_eq!(r.etichetta, "Nessuna anomalia");
        assert!(!r.spento, "con il PID in marcia il modulo non e' spento");
    }

    /// **Il modulo spento non si dichiara "nessuna anomalia".**
    ///
    /// Il difetto: con il regolatore fermo non ci sono problemi, quindi la
    /// riga riportava "Nessuna anomalia" — che è falso nel senso che importa.
    /// Il controller non sta raffreddando, e l'operatore deve saperlo dalla riga
    /// che guarda ogni giorno, non dedurlo dal fatto che non compare nessun
    /// allarme.
    ///
    /// Il test fissa la distinzione fra i due stati che condividono lo stesso
    /// verdetto: acceso e sano, e spento. Se tornassero a dare la stessa
    /// etichetta, il pannello perderebbe l'unica informazione che li
    /// distingue — e sarebbe tornato a mentire con la stessa frase di prima.
    #[test]
    fn il_modulo_spento_ha_una_etichetta_sua() {
        let spento = valuta(Esito::vuoto(), stato_sano());
        let r = spento.riassunto();

        assert!(spento.spento, "senza PID in marcia il modulo e' spento");
        assert_eq!(
            r.etichetta, "Modulo spento",
            "'Nessuna anomalia' su un modulo fermo è un'affermazione falsa"
        );
        assert_eq!(
            r.problemi, 0,
            "spento non e' un problema: non deve gonfiare il contatore"
        );
    }

    /// Lo spento cambia *come* lo si annuncia, non *che cosa* e': il verdetto
    /// resta com'era. Questo test separa le due cose, perche' sono
    /// indipendenti — un spento con l'alimentazione mancante deve restare
    /// "GUASTO", non diventare "Modulo spento" solo perche' il regolatore e'
    /// fermo.
    #[test]
    fn lo_spento_non_alza_il_verdetto() {
        let e = valuta(Esito::vuoto(), stato_sano());
        assert_eq!(e.verdetto, Verdetto::Ok, "spento non e' un guasto");
        assert!(e.spento);

        // spento + alimentazione KO resta un guasto: la condizione non
        // cancella i problemi veri, cambia solo l'etichetta di fondo.
        let guasto = valuta(Esito::vuoto(), stato_sano() & !TecStatus::POWER_OK);
        assert!(guasto.spento, "il regolatore e' comunque fermo");
        assert_eq!(guasto.verdetto, Verdetto::Guasto);
        assert!(
            guasto.riassunto().etichetta.contains("GUASTO"),
            "i guanti veri hanno la precedenza sullo stato spento: {:?}",
            guasto.riassunto().etichetta
        );
    }

    /// I problemi si contano: il numero e' quello che fa capire se guardare
    /// il pannello o no, e non il testo del primo problema.
    #[test]
    fn i_problemi_sono_contati() {
        let e = valuta(Esito::vuoto(), stato_sano() & !TecStatus::POWER_OK);
        let r = e.riassunto();
        assert_eq!(r.problemi, e.problemi.len());
        assert!(r.problemi > 0, "il guasto non e' stato contato");
    }

    /// **L'OCP da solo resta un segnale, non un guasto.** Il riepilogo non
    /// puo' dire GUASTO quando la cella raffredda: e' il falso allarme che
    /// ha fatto spegnere un impianto sano, e qui si chiude.
    #[test]
    fn il_riassunto_dell_ocp_non_e_un_guasto() {
        let e = valuta(Esito::vuoto(), stato_sano() | TecStatus::OCP_ACTIVE);
        assert_eq!(e.verdetto, Verdetto::Attenzione);
        let r = e.riassunto();
        assert!(
            !r.etichetta.contains("GUASTO"),
            "l'OCP non e' un guasto: '{}'",
            r.etichetta
        );
    }

    /// Un guasto vero resta GUASTO: la regola vale per l'OCP, non per tutto.
    #[test]
    fn un_guasto_vero_rest_guasto() {
        let e = valuta(Esito::vuoto(), stato_sano() & !TecStatus::POWER_OK);
        assert!(e.riassunto().etichetta.contains("GUASTO"), "{:?}", e.riassunto());
    }
}

/// Valuta lo stato di salute.
///
/// L'ordine dei controlli non e' casuale: prima quelli che rendono fuorvianti
/// tutti gli altri numeri (comunicazione), poi quelli che riguardano la
/// sicurezza (corrente, margine di condensa, alimentazione), infine il
/// regolatore.
pub fn valuta(m: Esito, status: TecStatus) -> EsitoDiagnostica {
    use crate::tecdiag::{evaluate, Severity};

    let mut problemi: Vec<String> = Vec::new();
    let mut verdetto = Verdetto::Ok;

    // ── Il modulo spento non e' un'anomalia: e' uno stato ─────────────
    //
    // Se il regolatore e' fermo, la diagnostica non deve lamentarsi che non
    // raffredda: sta facendo quello che gli e' stato chiesto. Prima non
    // c'era questo caso, e il risultato era che "Nessuna anomalia" e
    // "GUASTO" erano le sole risposte possibili per un controller fermo —
    // quindi o si dichiarava una salute che non c'era, o si generava un
    // allarme per uno spegnimento che l'operatore aveva fatto lui.
    //
    // Il trattamento e' quello del software Intel, che ha una classe
    // `SafetyRules` con `IsValideOperationMode`: una regola non si applica a
    // un regime in cui non ha significato. Qui e' la stessa idea, portata
    // avanti: prima di giudicare, si chiede **in che stato siamo**.
    let regime = crate::commutazione::Regime::da_stato(status);
    let spento = regime == Some(crate::commutazione::Regime::Spento);

    // ── I 18 bit noti, con la gravita' gia' calibrata ────────────────
    // `tecdiag::evaluate` porta label e consiglio. Li riusiamo: duplicare
    // la tabella dei bit in due moduli significa che un giorno divergono.
    //
    // **L'OCP viene elencato una volta sola.** Prima questo ciclo lo
    // aggiungeva e poi il blocco qui sotto lo aggiungeva di nuovo, con
    // due testi diversi: in dashboard comparivano due righe "OCP attivo"
    // con due consigli che non coincidevano. Il segnale e' reale, quindi
    // toglierlo del tutto non sarebbe stato giusto — ripeterlo due volte
    // fa peggio, perche' fa sembrare che ci siano due problemi.
    for issue in evaluate(status) {
        // A modulo spento, i bit che descrivono il **regolamento** non
        // descrivono nulla: `TEMP_MODE` puo' restare acceso perche' e'
        // l'ultimo regime, `PID_READY` puo' segnalare un PID memorizzato ma non
        // in funzione. Giudicarli produrrebbe allarmi per uno spegnimento che
        // l'operatore ha ordinato.
        //
        // I bit che descrivono l'**integrita'** restano invece validi, e sono
        // anzi piu' importanti: un controller spento con l'alimentazione
        // assente, il sensore scollegato o la connessione TEC persa e' un
        // guantoio, non uno spegnimento.
        if spento && issue.solo_in_funzione {
            continue;
        }
        let peggiora = match issue.severity {
            Severity::Critical => Verdetto::Guasto,
            Severity::Warning => {
                if verdetto == Verdetto::Guasto {
                    Verdetto::Guasto
                } else {
                    Verdetto::Attenzione
                }
            }
        };
        if peggiora == Verdetto::Guasto || verdetto == Verdetto::Ok {
            verdetto = peggiora;
        }
        problemi.push(format!("{}: {}", issue.label, issue.advice));
    }

    // L'OCP e' rumore nel hardware reale (l'utente lo ha visto a 73 W e a
    // 112 W con raffreddamento funzionante, e a 220 W), ma resta un
    // segnale. Lo teniamo ad Attenzione con la spiegazione, perche' il
    // giorno in cui significasse davvero qualcosa ci saremmo accorti che
    // qualcosa e' cambiato.
    if status.contains(TecStatus::OCP_ACTIVE) {
        // `tecdiag::evaluate` classifica l'OCP come Critical, quindi il
        // verdetto e' gia' `Guasto` quando arriviamo qui. Per l'hardware
        // reale non va bene: l'utente ha misurato l'OCP attivo a 73 W e a
        // 112 W con il raffreddamento funzionante, e anche a 220 W. Un
        // Guasto che spinge a fermare un sistema sano e' un falso allarme,
        // che e' esattamente il difetto che questo intervento rimuove.
        //
        // Quindi: se il verdetto e' `Guasto` **solo** per l'OCP, si
        // riporta ad `Attenzione`. Se c'e' anche altro di grave, resta
        // `Guasto`. Non e' una scorciatoia: e' la rimozione documentata
        // di un falso allarme noto.
        if problemi.iter().all(|p| p.contains("OCP")) {
            verdetto = Verdetto::Attenzione;
        }
        if verdetto != Verdetto::Guasto {
            verdetto = Verdetto::Attenzione;
        }
        // **Nessuna seconda riga.** Il testo e' gia' in `problemi`,
        // portato da `tecdiag::evaluate`, e dice la cosa giusta: e' rumore
        // su questo impianto e non c'e' azione da fare.
        //
        // Qui prima c'era anche una riga con i 73 W, i 112 W e il nome del
        // file di log: era il diario della ricerca, scritto in un posto
        // dove non serve. Il confronto dei bit sta nel log, che e' dove si
        // fa quel lavoro, e in diagnostica la risposta a un segnale che non
        // significa niente e' una sola parola: nessuna azione.
    }

    // ── Sicurezza termica, dai numeri e non dai bit ──────────────────
    // Il margine di condensa e' la misura che conta. Sotto la rugiada la
    // piastra si condensa, e con ghiaccio il danno alla scheda e'
    // irreversibile: questo e' un Guasto, non un avviso.
    if let Some(margine) = m.margine_condensa {
        if !margine.is_finite() {
            verdetto = Verdetto::Guasto;
            problemi.push(
                "Margine di condensa non calcolabile: i sensori non danno un dato valido."
                    .to_owned(),
            );
        } else if margine_negativo(margine) {
            verdetto = Verdetto::Guasto;
            problemi.push(format!(
                "Margine di condensa {margine:+.1} °C: la piastra e' SOTTO il punto di \
                 rugiada, si condensa. Spegnere la TEC e asciugare la piastra."
            ));
        } else if margine < 1.0 {
            if verdetto == Verdetto::Ok {
                verdetto = Verdetto::Attenzione;
            }
            problemi.push(format!(
                "Margine di condensa {margine:+.1} °C: positivo ma sotto 1 °C. \
                 La protezione anticondensa sta lavorando ai limiti."
            ));
        }
    }

    // ── Le misure, con i campi del report ufficiale ─────────────────
    let mut misure: Vec<Misura> = Vec::new();
    let mut aggiungi = |etichetta: &'static str,
                        valore: Option<f32>,
                        aggregata: bool,
                        fmt: &dyn Fn(f32) -> String| {
        if let Some(v) = valore {
            let valido = v.is_finite();
            misure.push(Misura {
                etichetta,
                valore: if valido { fmt(v) } else { "n/d".to_owned() },
                giudizio: if valido { Verdetto::Ok } else { Verdetto::Attenzione },
                aggregata,
            });
        }
    };

    // I primi tre campi sono aggregati di sessione: vanno nel file, non nel
    // pannello. Vedi `Misura::aggregata`.
    aggiungi("Temp. iniziale", m.temp_iniziale_c, true, &|v| format!("{v:.1}°C"));
    // "Minima" e' il **minimo** di sessione, non l'ultimo campione: su una
    // cella che raffredda il punto piu' freddo e' il risultato, mentre
    // l'ultimo valore cambia di continuo e non dice nulla di come e' andata
    // la sessione. La dicitura lo dice, perche' un numero con un'etichetta
    // che non lo descrive fa leggere cose che non ci sono.
    aggiungi("Temp. minima", m.temp_finale_c, true, &|v| format!("{v:.1}°C"));
    aggiungi("Potenza massima", m.potenza_max_w, true, &|v| format!("{v:.1}W"));
    aggiungi("Tensione", m.volts, false, &|v| format!("{v:.1}V"));
    aggiungi("Corrente", m.amps, false, &|v| format!("{v:.2}A"));
    aggiungi("Potenza", m.watts, false, &|v| format!("{v:.1}W"));

    // Il COP arriva gia' calcolato: `CopState::calculate` in `analytics.rs`
    // fa `Q / P_elettrico` a partire da CPU, TEC e watt. Ricalcolarlo qui
    // duplicherebbe la formula in due punti, e un giorno i due numeri
    // divergerebbero senza che nessuno se ne accorga. `None` resta `n/d`:
    // il COP richiede la temperatura CPU, e senza carico non c'e' niente
    // da confrontare.
    misure.push(Misura {
        etichetta: "COP (stima)",
        valore: match m.cop {
            Some(c) if c.is_finite() && c > 0.0 => format!("{c:.2}"),
            _ => "n/d".to_owned(),
        },
        giudizio: match m.cop {
            Some(c) if c.is_finite() && c > 0.0 => Verdetto::Ok,
            _ => Verdetto::Attenzione,
        },
        aggregata: false,
    });

    EsitoDiagnostica {
        verdetto,
        misure,
        problemi,
        modalita: m.modalita,
        // Dichiaro esplicitamente che il modulo e' fermo, perche' il verdetto
        // "Ok" da solo e' ambiguo: con il modulo spento non ci sono problemi da
        // segnalare, ma nemmeno un controller che sta funzionando. Senza
        // questo campo l'operatore legge "nessuna anomalia" e conclude che il
        // raffreddamento sia regolare, mentre in realta non e' acceso.
        spento,
    }
}

/// Il margine e' negativo?
///
/// Separato perche' `last_cond_margin` parte da `f32::MAX` come sentinella
/// quando non e' ancora stato calcolato: `f32::MAX` e' positivo e
/// "finito", quindi passerebbe il test, ma non significa "ho 10^38 gradi
/// di margine". Il chiamante deve gia' escludere la sentinella; questo
/// controllo isola il segno, che e' la cosa che conta per la sicurezza.
fn margine_negativo(margine: f32) -> bool {
    margine < 0.0
}

#[cfg(test)]
mod test {
    use super::*;

    fn stato_sano() -> TecStatus {
        TecStatus::POWER_OK
            | TecStatus::TEC_CONN_OK
            | TecStatus::BOARD_TEMP_OK
            | TecStatus::TEMP_SENSE_OK
            | TecStatus::HUM_SENSE_OK
            | TecStatus::PID_READY
            | TecStatus::PID_RUNNING
            | TecStatus::LAST_CMD_OK
    }

    /// **Il punto che non si puo' negoziare:** il nostro test guarda dati
    /// gia' letti, il self test del produttore e' attivo e vive nel
    /// controller. Se il log non lo dice, un utente che lo manda al supporto
    /// EK fa diagnosticare un file che non e' quello ufficiale.
    #[test]
    fn il_log_dichiara_di_non_essere_il_self_test() {
        let e = valuta(Esito::vuoto(), stato_sano());
        let riga = e.riga_log("2026-09-28T10:00:00Z");
        assert!(
            riga.contains("NON e' il self test del produttore"),
            "il log non dichiara la propria natura: {riga}"
        );
    }

    /// Con i bit di stato sani, nessun guasto. E' il caso normale: il test
    /// non deve urlare allarme quando tutto va bene, altrimenti l'utente
    /// smette di leggerlo.
    #[test]
    fn uno_stato_sano_da_ok() {
        let e = valuta(Esito::vuoto(), stato_sano());
        assert_eq!(e.verdetto, Verdetto::Ok, "problemi: {:?}", e.problemi);
    }

    /// OCP attivo e' **Attenzione**, non Guasto: l'utente ha misurato l'OCP
    /// attivo a 112 W e a 73 W con raffreddamento funzionante, e anche a
    /// 220 W. Un Guasto farebbe spegnere un sistema sano. Ma nemmeno Ok: un
    /// segnale che non significa nulla va segnalato.
    ///
    /// Nota `tecdiag::evaluate` classifica l'OCP come `Critical`: e' la
    /// ragione per cui questo test esiste, ed e' il falso allarme che la
    /// diagnostica corregge.
    #[test]
    fn ocp_attivo_e_attenzione_non_guasto() {
        let e = valuta(Esito::vuoto(), stato_sano() | TecStatus::OCP_ACTIVE);
        assert_eq!(e.verdetto, Verdetto::Attenzione);
        assert!(
            e.problemi.iter().any(|p| p.contains("OCP")),
            "{:?}",
            e.problemi
        );
    }

    /// Senza alimentazione o senza TEC il Guasto e' certo: non si puo'
    /// raffreddare senza corrente, e il danno alla piastra e' irreversibile.
    #[test]
    fn alimentazione_ko_e_guasto() {
        let e = valuta(Esito::vuoto(), TecStatus::TEC_CONN_OK | TecStatus::BOARD_TEMP_OK);
        assert_eq!(e.verdetto, Verdetto::Guasto);
    }

    /// **Il margine di condensa e' la misura di sicurezza vera.** La
    /// dashboard mostrava +2.0 °C: positivo, quindi sicuro. Un margine
    /// negativo e' un Guasto, perche' la piastra e' sotto il punto di
    /// rugiada e si condensa.
    #[test]
    fn margine_di_condensa_negativo_e_guasto() {
        let mut m = Esito::vuoto();
        m.margine_condensa = Some(-0.5);
        let e = valuta(m, stato_sano());
        assert_eq!(e.verdetto, Verdetto::Guasto, "sotto la rugiada: condensa");
    }

    #[test]
    fn margine_di_condensa_positivo_e_ok() {
        let mut m = Esito::vuoto();
        m.margine_condensa = Some(2.0);
        let e = valuta(m, stato_sano());
        assert_eq!(e.verdetto, Verdetto::Ok, "{:?}", e.problemi);
    }

    /// Sotto 1 °C il margine e' positivo, quindi non c'e' condensa: ma la
    /// protezione sta ai limiti e l'utente deve saperlo.
    #[test]
    fn margine_sotto_un_grado_e_attenzione() {
        let mut m = Esito::vuoto();
        m.margine_condensa = Some(0.4);
        let e = valuta(m, stato_sano());
        assert_eq!(e.verdetto, Verdetto::Attenzione, "{:?}", e.problemi);
    }

    /// **Un Guasto non puo' essere declassato dall'OCP.** Il declassamento
    /// dell'OCP e' una correzione mirata: vale solo se l'OCP e' l'unico
    /// problema. Se manca anche l'alimentazione, resta `Guasto`: fermare un
    /// controller senza corrente e' l'unico caso in cui fermare e' giusto.
    #[test]
    fn un_guasto_vero_non_viene_declassato_dall_ocp() {
        let e = valuta(
            Esito::vuoto(),
            (stato_sano() | TecStatus::OCP_ACTIVE) & !TecStatus::POWER_OK,
        );
        assert_eq!(
            e.verdetto,
            Verdetto::Guasto,
            "manca l'alimentazione: declassare sarebbe pericoloso"
        );
    }

    /// I dati assenti non si inventano: senza tensione non c'e' COP, e un
    /// COP inventato fa prendere decisioni su numeri falsi.
    #[test]
    fn senza_cop_il_valore_e_non_calcolabile() {
        let mut m = Esito::vuoto();
        m.volts = Some(0.0);
        let e = valuta(m, stato_sano());
        let cop = e.misure.iter().find(|x| x.etichetta.contains("COP"));
        assert!(cop.is_some(), "il COP deve comparire comunque");
        assert!(
            cop.unwrap().valore.contains("n/d"),
            "COP inventato: {}",
            cop.unwrap().valore
        );
    }

    /// Le misure elettriche devono comparire tutte: e' la richiesta
    /// esplicita, e finora erano sparse in tre pannelli diversi.
    #[test]
    fn le_misure_elettriche_ci_sono_tutte() {
        let mut m = Esito::vuoto();
        m.volts = Some(19.4);
        m.amps = Some(5.8);
        m.watts = Some(112.5);
        let e = valuta(m, stato_sano());
        for atteso in ["Tensione", "Corrente", "Potenza", "COP"] {
            assert!(
                e.misure.iter().any(|x| x.etichetta.contains(atteso)),
                "manca la misura {atteso}: {:?}",
                e.misure.iter().map(|x| x.etichetta).collect::<Vec<_>>()
            );
        }
    }

    /// La potenza massima e' uno dei tre campi del report ufficiale: va
    /// riportata, perche' serve a confrontare col self test del produttore.
    #[test]
    fn la_potenza_massima_e_nel_log() {
        let mut m = Esito::vuoto();
        m.potenza_max_w = Some(239.4);
        let e = valuta(m, stato_sano());
        let riga = e.riga_log("2026-09-28T10:00:00Z");
        assert!(riga.contains("239"), "potenza massima assente: {riga}");
    }

    /// Ogni problema deve dire **cosa fare**, non solo cosa e' successo: e'
    /// la parte che l'utente cerca davvero quando legge una diagnostica.
    #[test]
    fn ogni_problema_ha_un_indicazione() {
        let e = valuta(Esito::vuoto(), stato_sano() | TecStatus::OCP_ACTIVE);
        assert!(!e.problemi.is_empty(), "nessun problema elencato");
        for p in &e.problemi {
            assert!(p.len() > 40, "indicazione troppo breve per essere utile: {p}");
        }
    }
}

/// Sceglie i valori da mostrare nella diagnostica.
///
/// **Funzione pura, senza accesso allo stato**: e' testabile senza
/// costruire la UI, che altrimenti significa che il test si rompe a ogni
/// refactor della finestra.
///
/// La separazione conta perche' i valori hanno significati diversi e vanno
/// tenuti separati: la temperatura iniziale e' il primo campione, la
/// temperatura finale e' l'ultima, e il margine di condensa e' una
/// differenza. Farli viaggiare negli stessi campi produce etichette che
/// promettono una cosa e mostrano un'altra.
pub fn scegli_misure(
    primo_tec: Option<f32>,
    ultimo_tec: Option<f32>,
    margine: Option<f32>,
    potenza_max: Option<f32>,
    volts: Option<f32>,
    amps: Option<f32>,
    cop: Option<f32>,
) -> Esito {
    Esito {
        temp_iniziale_c: primo_tec,
        temp_finale_c: ultimo_tec,
        potenza_max_w: potenza_max,
        watts: None, // impostato dal chiamante: e' la potenza istantanea
        volts,
        amps,
        cop,
        margine_condensa: margine,
        modalita: None,
    }
}

#[cfg(test)]
mod test_misure {
    use super::*;

    /// **"Temp. finale" non era una temperatura.**
    ///
    /// Il campo riceveva il **margine di condensa**: in dashboard compariva
    /// "Temp. finale 1.3°C" con margine +1,3 °C. Due numeri diversi con lo
    /// stesso valore, e l'utente leggeva una temperatura che non era una
    /// temperatura.
    #[test]
    fn il_margine_non_finisce_in_temp_finale() {
        let e = scegli_misure(Some(30.0), Some(12.0), Some(1.3), Some(240.0), None, None, None);
        assert_eq!(e.margine_condensa, Some(1.3), "il margine sta nel suo campo");
        assert_eq!(e.temp_finale_c, Some(12.0), "temp finale e' l'ultima temperatura");
        assert_ne!(
            e.temp_finale_c, e.margine_condensa,
            "temperatura e margine sono due cose diverse"
        );
    }

    /// "Temp. iniziale" era la **media** della sessione, non l'iniziale.
    /// Un'etichetta che promette l'inizio e mostra una media mente, e su
    /// una sessione lunga i due numeri non somigliano affatto.
    #[test]
    fn temp_iniziale_e_il_primo_campione() {
        let e = scegli_misure(Some(30.0), Some(12.0), Some(1.3), Some(240.0), None, None, None);
        assert_eq!(
            e.temp_iniziale_c, Some(30.0),
            "l'iniziale e' il primo valore, non la media"
        );
    }

    /// I dati assenti non si trasformano in zero: `None` resta `None`, e
    /// la UI mostra `n/d`. Uno zero sembra una misura.
    #[test]
    fn l_assenza_rest_a_assenza() {
        let e = scegli_misure(None, None, None, None, None, None, None);
        assert!(e.temp_iniziale_c.is_none());
        assert!(e.margine_condensa.is_none());
        assert!(e.cop.is_none());
    }

    /// La potenza massima e' la massima, non la media: e' uno dei tre campi
    /// del report del produttore e serve a confrontare l'andamento.
    #[test]
    fn la_potenza_massima_e_la_massima() {
        let e = scegli_misure(None, None, None, Some(239.4), None, None, None);
        assert_eq!(e.potenza_max_w, Some(239.4));
    }
}

#[cfg(test)]
mod test_layout_misure {
    use super::*;

    /// **Accorpare il layout non puo' significare nascondere un dato.**
    ///
    /// Sei misure in una riga andavano a capo nella sidebar, e una riga che
    /// si spezza fa leggere "44,0 W" come due fatti invece di uno. Il fix e'
    /// stato spostare le misure fuori dalla sidebar, non accorciarle a una
    /// sola: qui si verifica che la griglia del pannello le prenda tutte.
    #[test]
    fn tutte_le_misure_restano_visibili() {
        let mut m = Esito::vuoto();
        m.volts = Some(19.4);
        m.amps = Some(5.8);
        m.watts = Some(112.5);
        m.cop = Some(1.7);
        let e = valuta(
            m,
            TecStatus::POWER_OK
                | TecStatus::TEC_CONN_OK
                | TecStatus::BOARD_TEMP_OK
                | TecStatus::TEMP_SENSE_OK
                | TecStatus::HUM_SENSE_OK
                | TecStatus::LAST_CMD_OK,
        );
        // Tensione, corrente, potenza e COP: quattro, non sei, e le quattro
        // sono quelle che l'utente cerca in un colpo d'occhio.
        let etichette: Vec<&str> = e.misure.iter().map(|x| x.etichetta).collect();
        for atteso in ["Tensione", "Corrente", "Potenza", "COP"] {
            assert!(
                etichette.iter().any(|e| e.contains(atteso)),
                "manca {atteso}: {etichette:?}"
            );
        }
    }
}

#[cfg(test)]
mod test_ocp_una_volta_sola {
    use super::*;

    /// Stato senza anomalie: solo i bit che devono stare accesi.
    fn stato_sano() -> TecStatus {
        TecStatus::POWER_OK
            | TecStatus::TEC_CONN_OK
            | TecStatus::BOARD_TEMP_OK
            | TecStatus::TEMP_SENSE_OK
            | TecStatus::HUM_SENSE_OK
            | TecStatus::LAST_CMD_OK
    }

    /// **L'OCP compariva due volte in dashboard.**
    ///
    /// Lo aggiungeva `tecdiag::evaluate`, e poi la diagnostica lo
    /// aggiungeva di nuovo con un testo diverso. Due righe "OCP attivo"
    /// con due consigli che non coincidevano fanno sembrare che ci siano
    /// due problemi, quando ce n'e' uno solo e non e' nemmeno un guasto.
    #[test]
    fn locp_appare_una_volta_solo() {
        let e = valuta(
            Esito::vuoto(),
            stato_sano() | TecStatus::OCP_ACTIVE,
        );
        let righe: Vec<&String> = e.problemi.iter().filter(|p| p.contains("OCP")).collect();
        assert_eq!(
            righe.len(), 1,
            "l'OCP compare {} volte: {righe:?}",
            righe.len()
        );
    }

    /// Un altro bit che scatta insieme all'OCP non viene perso: se ne
    /// contiamo uno solo e c'e' anche altro da dire, quello deve restare.
    #[test]
    fn un_secondo_problema_resta() {
        let e = valuta(
            Esito::vuoto(),
            (stato_sano() | TecStatus::OCP_ACTIVE) & !TecStatus::POWER_OK,
        );
        assert!(
            e.problemi.iter().any(|p| p.contains("Alimentazione")),
            "l'alimentazione KO deve restare nell'elenco: {:?}",
            e.problemi
        );
        assert_eq!(
            e.verdetto, Verdetto::Guasto,
            "senza alimentazione resta Guasto"
        );
    }

    /// Il testo che resta deve dire che non c'e' azione da fare: e' la
    /// cosa che l'utente cerca in una diagnostica, e non un'altra frase
    /// da interpretare.
    #[test]
    fn il_testo_che_resta_dice_che_non_c_e_azione() {
        let e = valuta(Esito::vuoto(), stato_sano() | TecStatus::OCP_ACTIVE);
        let riga = e.problemi.iter().find(|p| p.contains("OCP")).unwrap();
        let basso = riga.to_lowercase();
        assert!(
            basso.contains("nessuna azione") || basso.contains("non e' una protezione"),
            "il testo deve dire che non c'e' azione: {riga}"
        );
    }
}

/// Come va letto il margine di condensa.
///
/// `NonFresco` **non** e' "nessun problema": e' "non lo so". La differenza
/// conta, perche' il difetto che questo tipo chiude e' una riga verde che
/// diceva `OK` con il controller scollegato da un minuto. Il numero era valido
/// *prima*, e il ramo di errore del campionamento non lo azzera mai, quindi
/// continuava a essere ristampato come se fosse di adesso.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EtichettaMargine {
    /// Margine fresco e sopra soglia.
    Ok,
    /// Margine fresco, vicino alla rugiada.
    Basso,
    /// Margine fresco, la piastra e' sotto la rugiada.
    Condensa,
    /// Il dato non e' fresco: non si dichiara nulla.
    NonFresco,
}

/// Il margine di condensa e la sua eta'.
///
/// La soglia dei due gradi e' quella che la UI usava gia', quindi non e' una
/// scelta nuova: e' il numero che l'operatore ha sempre visto accanto a
/// "basso". Quello che cambia e' il terzo caso.
pub const SOGLIA_MARGINE_BASSO: f32 = 2.0;

/// Decide cosa mostrare, sapendo se il dato e' fresco.
///
/// `fresco` arriva da `RunningState::margine_fresco`, che usa lo stesso
/// `SOGLIA_FRESCHEZZA` della `Certezza`: la freschezza ha **una** soglia in tutto
/// il software, non una per la diagnostica e una per la guardia.
pub fn etichetta_margine(margine: f32, fresco: bool) -> EtichettaMargine {
    if !fresco {
        return EtichettaMargine::NonFresco;
    }
    if margine < 0.0 {
        EtichettaMargine::Condensa
    } else if margine < SOGLIA_MARGINE_BASSO {
        EtichettaMargine::Basso
    } else {
        EtichettaMargine::Ok
    }
}

#[cfg(test)]
mod test_etichetta_margine {
    use super::*;

    /// **Un margine vecchio non si dichiara `Ok`.** E' la difesa contro la
    /// riga verde falsa. Con il controller scollegato, `last_cond_margin`
    /// tiene l'ultimo valore valido perche' il ramo `Err` del campionamento non
    /// lo azzera: la riga continuava a stampare `Margine +3.0°C OK` in verde
    /// mentre nessuno sapeva piu' nulla.
    #[test]
    fn un_margine_non_fresco_non_si_dichiara_ok() {
        assert_eq!(
            etichetta_margine(3.0, false),
            EtichettaMargine::NonFresco,
            "3.0 °C ma vecchio: non si dichiara 'OK'"
        );
        assert_ne!(
            etichetta_margine(3.0, false),
            etichetta_margine(3.0, true),
            "fresco e non fresco non possono avere la stessa etichetta"
        );
    }

    /// Non solo `Ok`: **nessun** valore vecchio si dichiara sicuro. Un margine
    /// vecchio negativo non e' "condensa", e' "non lo so".
    #[test]
    fn nessun_valore_vecchio_si_dichiara() {
        for m in [-5.0, -0.1, 0.5, 1.9, 2.0, 12.0, 99.0] {
            assert_eq!(
                etichetta_margine(m, false),
                EtichettaMargine::NonFresco,
                "{m} °C vecchio: nessuna dichiarazione di sicurezza"
            );
        }
    }

    /// Le tre etichette di un margine fresco, con i bordi della soglia.
    #[test]
    fn le_tre_etichette_di_un_margine_fresco() {
        assert_eq!(etichetta_margine(4.0, true), EtichettaMargine::Ok);
        assert_eq!(etichetta_margine(1.2, true), EtichettaMargine::Basso);
        assert_eq!(etichetta_margine(-0.4, true), EtichettaMargine::Condensa);
        // Bordi: sotto zero e' condensa, a zero no.
        assert_eq!(etichetta_margine(0.0, true), EtichettaMargine::Basso);
        // Soglia dei due gradi: sotto e' basso, a due gradi e' ok.
        assert_eq!(etichetta_margine(1.99, true), EtichettaMargine::Basso);
        assert_eq!(etichetta_margine(2.0, true), EtichettaMargine::Ok);
    }

    /// `NonFresco` non e' un quarto livello di pericolo: e' assenza di
    /// informazione. Nessuno dei tre colori di pericolo gli si addice, e
    /// sicuramente non il verde.
    #[test]
    fn non_fresco_e_distinto_da_pericolo() {
        assert_ne!(
            etichetta_margine(-5.0, true),
            etichetta_margine(5.0, false),
            "condensa e 'non lo so' non sono la stessa cosa"
        );
    }
}
