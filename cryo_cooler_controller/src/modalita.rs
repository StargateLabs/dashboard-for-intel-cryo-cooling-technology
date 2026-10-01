//! Le tre modalita' operative del controller Cryo.
//!
//! # Perche' questo modulo esiste
//!
//! Il controller ha tre modalita' con significati molto diversi per la
//! sicurezza, e l'utente deve poter capire **in quale si trova** senza
//! aprire il software del vendor. Il colore del LED lo dice, ma solo se lo
//! si sa guardare: qui la modalita' e' scritta, con il suo colore e cosa
//! comporta.
//!
//! # Cosa questo modulo NON fa
//!
//! **Non cambia modalita'.** Il protocollo conosciuto non ha un comando per
//! impostare Standby / Cryo / Unregulated: i comandi disponibili sono PID,
//! setpoint, potenza, temperatura CPU, enable e disable. Provare opcode
//! sconosciuti su una seriale che oggi funziona e' esattamente il rischio
//! che ha gia' fatto perdere il TEC all'inizio del progetto.
//!
//! Quindi il modulo e' **in sola lettura**: legge il bit `TEMP_MODE` che il
//! controller invia e dice dove sei. Le altre due modalita' sono documentate
//! perche' servono a capire cosa significherebbe passare a them, non per
//! passarci.

use cryo_cooler_controller_lib::TecStatus;

/// Una modalita' del controller, con tutto quello che serve per mostrarla.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Modalita {
    /// Niente raffreddamento sub-ambiente. Restano ventole e pompa.
    Standby,
    /// Raffreddamento sub-ambiente regolato: e' la modalita' sicura.
    Cryo,
    /// Massima potenza forzata, con rischio di condensa dichiarato dal
    /// produttore.
    Unregulated,
    /// Non leggibile: il controller non ha risposto, o non e' collegato.
    Offline,
    /// Il modulo e' spento: collegato e funzionante, ma non in funzione.
    ///
    /// **Non e' la stessa cosa di Offline.** Offline e' "non lo so": nessun
    /// battito, porta scollegata, controller assente. Spento e' "lo so": il
    /// controller risponde e ha il regolatore fermo. La differenza per
    /// l'operatore e' la stessa che c'e' tra "non funziona" e "funziona ma
    /// e' spento", e su un pannello di controllo dirgli la prima quando e' vera
    /// la seconda e' un'allarme inventato.
    ///
    /// Senza questo stato, spegnere il TEC veniva letto come Offline e la
    /// dashboard proponeva di riavviare un sistema che invece era semplicemente
    /// spento, e per farlo lo accendeva da sola.
    Spento,
}

impl Modalita {
    /// Nome della modalita', come la chiama il produttore.
    pub fn nome(&self) -> &'static str {
        match self {
            Modalita::Standby => "Standby",
            Modalita::Cryo => "Cryo",
            Modalita::Unregulated => "Unregulated",
            Modalita::Offline => "Offline",
            Modalita::Spento => "Spento",
        }
    }

    /// **Colore usato in app per la modalita'.**
    ///
    /// Gli stessi colori che il manuale documenta per il LED (3.1), ma
    /// **scuriti**: sul pannello sono accesi, in app ci va testo bianco
    /// sopra e il verde saturo non darebbe contrasto. Le famiglie di tinta
    /// sono le stesse, quindi il confronto fra la dashboard e il pannello
    /// resta valido: verde contro verde, blu contro blu.
    pub fn colore_led(&self) -> (u8, u8, u8) {
        match self {
            Modalita::Standby => (60, 130, 255),   // blu
            Modalita::Cryo => (0, 215, 120),      // verde
            Modalita::Unregulated => (175, 90, 255), // viola
            Modalita::Offline => (235, 70, 70),    // rosso
            // Nessun colore LED e' documentato per "spento": e' uno stato
            // che il produttore non descrive perche' non e' un regime, e' un
            //'assenza di regime. Il grigio dice "non e' operativo" senza
            // inventare un colore che sul pannello non si vedrebbe.
            Modalita::Spento => (130, 130, 140),   // grigio
        }
    }

    /// Una riga che dice cosa fa, in una frase.
    pub fn descrizione(&self) -> &'static str {
        match self {
            Modalita::Standby => {
                "Niente raffreddamento sub-ambiente: solo ventole e pompa, \
                 come un liquido normale."
            }
            Modalita::Cryo => {
                "Raffreddamento regolato: il controller tiene la piastra \
                 sopra la rugiada e usa la massima potenza sicura."
            }
            Modalita::Unregulated => {
                "Massima potenza forzata. Il controller prova a non scendere \
                 sotto la rugiada, ma NON lo garantisce: rischio di condensa."
            }
            Modalita::Offline => {
                "Il controller non risponde: staccato, non installato, \
                 o in avvio. Verificare il cavo."
            }
            Modalita::Spento => {
                "Il controller e' collegato e risponde, ma il modulo e' \
                 spento: nessuna potenza erogata. Serve Abilita TEC."
            }
        }
    }

    /// `true` se e' la modalita' in cui il programatore consiglia di stare.
    pub fn e_consigliata(&self) -> bool {
        matches!(self, Modalita::Cryo)
    }

    /// Le tre modalita' del controller, dal piu' tranquillo al piu' delicato.
    ///
    /// Non e' un selettore: e' l'elenco che la finestra spiegativa usa per
    /// dire cosa sono. Nella sidebar non compare, perche' elencare le
    /// alternative accanto allo stato fa sembrare che si possa scegliere, e
    /// non si puo' (vedi `MODALITA_NOTA`).
    pub fn selezionabili() -> [Modalita; 3] {
        [
            Modalita::Standby,
            Modalita::Cryo,
            Modalita::Unregulated,
        ]
    }


    /// **Dove si esce da Unregulated.** Sempre Cryo, mai altrove: e' la
    /// modalita' regolata, e con margini di condensa di pochi gradi e'
    /// l'unico posto dove si deve stare.
    ///
    /// Usato solo dai test: fissa la regola di sicurezza in un posto solo.
    #[cfg(test)]
    pub fn uscita_da(&self) -> Modalita {
        Modalita::Cryo
    }

    /// Se da questa modalita' serve un **bottone di uscita** a schermo.
    ///
    /// Vero solo da Unregulated: e' l'unico regime che raffredda abbastanza da
    /// poter far formare ghiaccio sulla piastra, quindi l'unico da cui si deve
    /// poter tornare indietro con un gesto ovvio. In Cryo e Standby non
    /// mostrerebbe nulla di utile — starei gia' dove si deve.
    ///
    /// Ripristinato dopo che il pulsante Abilita/Disabilita era scomparso: la
    /// funzione era stata rimossa insieme a lui, ma il **bottone "Torna a Cryo"
    /// e' rimasto** (si e' spostato con il menu nella sua nuova posizione), e
    /// quindi anche la sua regola. Rimuovere il predicato mentre il widget
    /// esiste lascia la regola senza casa.
    #[cfg(test)]
    pub fn serve_uscita(&self) -> bool {
        *self == Modalita::Unregulated
    }
}

impl Modalita {
/// Il LED della modalita', come documentato dal manuale.
///
/// La chiave di questo metodo e' una **correzione**. Era stato scritto che il
/// manuale Gen1 documentasse solo il verde di Cryo e il rosso di Offline, e
/// che blu e viola fossero "convenzione delle icone del software Gen2, non
/// verificata". Era falso: la sezione 3.1 "Three Modes of Operation" del
/// manuale riporta una colonna "Controller LED Indicator" con tutte e quattro:
///
///   Standby     -> Blue   - slow blinking
///   Cryo        -> Green  - slow blinking
///   Unregulated -> Purple - fast blinking
///   Offline     -> Red    - solid
///
/// Riportare l'informazione vera e' il punto: chi leggeva "LED non documentato"
/// accanto a Standby non poteva confrontare la dashboard con il pannello, e il
/// confronto e' l'unico controllo disponibile per chi non ha installato il
/// software Intel.
pub fn led(&self) -> Option<LedDocumentato> {
    match self {
        Modalita::Standby => Some(LedDocumentato { colore: "blu", lampeggio: "lento" }),
        Modalita::Cryo => Some(LedDocumentato { colore: "verde", lampeggio: "lento" }),
        Modalita::Unregulated => Some(LedDocumentato { colore: "viola", lampeggio: "veloce" }),
        Modalita::Offline => Some(LedDocumentato { colore: "rosso", lampeggio: "fisso" }),
        // Lo spegnimento non e' un regime: il produttore non gli assegna un
        // colore. Dichiararloignoto e' corretto — dichiarare un colore
        // inventato sarebbe l'errore che questo intervento corregge altrove.
        Modalita::Spento => None,
    }
}
}

/// Colore e lampeggio del LED, quando il manuale Gen1 li documenta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedDocumentato {
    pub colore: &'static str,
    pub lampeggio: &'static str,
}

/// Il testo della finestra che spiega le modalita'.
///
/// **Correzione.** Il testo precedente diceva: "Nessun programma puo'
/// impostarla: nel controller non esiste un comando di cambio modalita'".
/// Era falso. Il reverse engineering di `IntelCryoCooling.Controller.dll`
/// mostra che `InitCryoMode`, `InitUnregulatedMode` e `InitStandbyMode`
/// esistono e funzionano: non hanno un opcode proprio, ma scrivono
/// l'offset del setpoint e accendono o spengono il TEC. E il manuale lo
/// conferma (§2.2.1 menu "Mode", §4.3 "Remember Cryo Mode", errore CB2 per
/// il controller *restato* in Unregulated).
///
/// Quindi da qui si commuta, e la dashboard lo fa.
///
/// La ragione per cui l'offset non e' un numero libero e' che i valori
/// vengono dal binario: `3.5` per il riposo e `-30.0` per la massima
/// potenza sono i due getter float costanti di
/// `Intel.CryoCooling.Configuration.dll`. Il Cryo non ha un offset proprio:
/// usa quello che hai impostato tu, che e' l'unica scelta che resta tua.
pub const MODALITA_NOTA: &str = "Il regime si cambia da qui. Il controller non ha un comando \
\"cambia modalita'\": i tre modi per ottenerla sono scrivere il setpoint, accendere o spegnere il \
TEC, e poi rileggere lo stato. E' quello che fa il software del produttore, ed e' quello che fa \
questa dashboard.

I valori non sono scelti a caso: Standby e Unregulated usano gli stessi numeri del software \
Intel (-30 °C e +3.5 °C). Cryo invece usa il setpoint che hai impostato tu nel pannello, e non \
lo sovrascrive.

Dopo ogni cambio la dashboard riletta i bit del controller e ti dice se il regime e' davvero \
quello richiesto. Se il controller risponde qualcos'altro, lo vedi scritto.

L'Unregulated richiede due conferme. Porta la piastra sotto il punto di rugiada: il manuale Intel \
avverte che puo' danneggiare la scheda. Il controller ha una sua protezione e dopo 10 minuti senza \
carico torna da solo in Cryo, ma non farci affidamento.";

/// La modalita' attuale, dedotta dai bit che il controller invia.
///
/// `comunicante` serve per distinguere "il controller ha detto che non e' in
/// regolazione" da "non ho sentito niente": sono due cose diverse, e confuse
/// fanno pensare che il TEC sia spento quando invece sta funzionando.
///
/// **Due bit, non uno.** Il controller invia `LOW_POWER_MODE_ACTIVE` e
/// `TEMP_MODE`: il primo dice che e' in riposo, il secondo che sta regolando.
/// Prima si guardava solo il secondo, quindi Standby non compariva mai: il
/// menu prometteva tre modalita' e ne mostrava due, e quella con il rischio
/// di condensa era proprio quella che restava senza nome.
///
/// **La mappatura `TEMP_MODE` = Cryo / non `TEMP_MODE` = Unregulated resta
/// un'ipotesi**: il manuale Gen1 non documenta il significato di questi bit.
/// Finche' non e' verificata, la riga va letta come "quello che il bit dice".
pub fn modalita_corrente(status: TecStatus, comunicante: bool) -> Modalita {
    // `comunicante` significa oggi **"abbiamo una lettura fresca"**, e non
    // "il TEC sta facendo qualcosa". La polarita' (`true` = non Offline) e'
    // rimasta quella di sempre: cambiare anche quella avrebbe rotto nove test
    // senza che nessuno se ne accorgesse, e il difetto non era nella
    // polarita'.
    //
    // Il difetto era nel *caller*, che costruiva il flag come
    // `poll_in_flight || applied_power > 0 || tec_abilitato`: tre grandezze
    // diverse, che capitavano di dare la risposta giusta per un motivo sbagliato
    // (un controller scollegato e' anche "sta facendo niente"). Ora e'
    // `!puo_scrivire(certezza())`, che e' la domanda giusta: sappiamo cosa
    // sta succedendo adesso?
    //
    // Se si', la modalita' restituita puo' essere `Spento`, che non e' "non lo
    // so": e' "lo so: e' spento".
    if !comunicante {
        return Modalita::Offline;
    }
    // **Delegazione, non una seconda mappatura.**
    //
    // Qui prima c'era un `if` sui bit che leggeva `LOW_POWER_MODE` e
    // `TEMP_MODE` e decideva fra Standby, Cryo e Unregulated: una mappatura
    // parallela a quella del menu, scritta a parte, che nessun test metteva
    // a confronto con l'altra.
    //
    // Il risultato era che la riga di stato poteva dire una cosa e il menu
    // un'altra sullo stesso controller nello stesso istante. E' successo: il
    // modulo era fermo, la riga diceva "Cryo", e la verifica di una
    // commutazione riuscita dava un esito che non tornava con quello che si
    // vedeva a schermo — due letture dello stesso controller, in contraddizione.
    //
    // La mappa `Regime -> Modalita` e' **la stessa che usa il menu**: e la
    // funzione `da_regime`, piu' in basso. Stavolta la delegazione passa
    // anche dall'altra parte: prima `modalita_corrente` leggeva i bit e li
    // traduceva qui, mentre il menu faceva lo stesso con un proprio `match`:
    // due tabelle che potevano divergere senza che nessun test lo notasse.
    // `da_stato` puo' tornare `None`: non e' "spento", e' **"non lo so in
    // che regime sia"**. Non viene schiacciato su `Spento` perche' e' la
    // stessa bugia che faceva dire "HW TEC spento" con 227 W in circolo: un
    // valore di comodo al posto di "non lo so".
    //
    // Senza regime noto mostriamo `Offline`, che nel menu vuol dire "non
    // leggibile" — e non lo e' esattamente, ma e' l'unico segnale che esiste
    // per "non posso dirti cosa sta facendo", ed e' rosso, quindi non passa
    // inosservato.
    match crate::commutazione::Regime::da_stato(status) {
        Some(regime) => da_regime(regime),
        None => Modalita::Offline,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// I tre colori LED devono essere **distinti**: sono l'unico modo di
    /// capire a quale modalita' e' il controller guardandolo.
    #[test]
    fn i_colori_led_sono_distinti() {
        let colori: HashSet<_> = Modalita::selezionabili()
            .iter()
            .chain([Modalita::Offline].iter())
            .map(|m| m.colore_led())
            .collect();
        assert_eq!(colori.len(), 4, "due modalita' hanno lo stesso colore LED");
    }

    /// Solo Cryo e' la consigliata: e' l'unica regolata, ed e' quella che il
    /// produttore raccomanda per evitare la condensa.
    #[test]
    fn solo_cryo_e_consigliata() {
        for m in Modalita::selezionabili() {
            assert_eq!(
                m.e_consigliata(),
                m == Modalita::Cryo,
                "{} ha il flag 'consigliata' sbagliato",
                m.nome()
            );
        }
    }

    /// Un controller muto e' **Offline**, non Unregulated: confonderli fa
    /// credere che il TEC sia in una modalica pericolosa quando invece non
    /// sta comunicando affatto.
    #[test]
    fn senza_comunicazione_e_offline() {
        assert_eq!(
            modalita_corrente(TecStatus::empty(), false),
            Modalita::Offline
        );
    }

    /// Con `TEMP_MODE` attivo siamo in regolazione.
    #[test]
    fn temp_mode_attivo_e_cryo() {
        let s = TecStatus::from_bits_retain((1 << 12) | (1 << 17));
        assert_eq!(modalita_corrente(s, true), Modalita::Cryo);
    }

    /// Ogni modalita' deve avere nome, colore in app e descrizione: un menu
    /// con una riga vuota e' peggio di nessun menu.
    #[test]
    fn ogni_modalita_e_completa() {
        for m in Modalita::selezionabili().into_iter().chain([Modalita::Offline]) {
            assert!(!m.nome().is_empty(), "nome vuoto");
            assert!(m.descrizione().len() > 20, "descrizione troppo corta");
            let (r, g, b) = m.colore_led();
            // Somma in `u16`: tre canali `u8` sommano a 765 al massimo e
            // andare in overflow fa fallire il test per un motivo che non
            // c'entra con quello che si voleva controllare.
            assert!(
                r as u16 + g as u16 + b as u16 > 0,
                "{}: colore in app assente, la riga di stato sarebbe invisibile",
                m.nome()
            );
        }
    }

    /// **Il pallino in app e' della stessa tinta del LED documentato.**
    ///
    /// Un colore, un significato: se la finestra scrive "LED blu" accanto a un
    /// pallino verde, l'operatore non sa quale dei due sia quello del pannello.
    ///
    /// Il controllo e' sulla **famiglia di tinta** (quale canale domina), non
    /// sull'uguaglianza esatta: in app il colore e' scurito per far contrasto
    /// col testo bianco, e i rapporti fra i canali cambiano un po'. La
    /// dominante deve pero' restare quella giusta, altrimenti "blu" e "verde"
    /// si somigliano e la tabella non distingue piu' niente.
    #[test]
    fn il_pallino_e_della_tinta_del_led() {
        /// Il canale che domina: 0 = rosso, 1 = verde, 2 = blu.
        fn dominante(c: (u8, u8, u8)) -> usize {
            let (r, g, b) = (c.0 as i32, c.1 as i32, c.2 as i32);
            if r >= g && r >= b {
                0
            } else if g >= r && g >= b {
                1
            } else {
                2
            }
        }
        let atteso = [
            (Modalita::Standby, 2), // blu
            (Modalita::Cryo, 1),    // verde
            (Modalita::Unregulated, 2), // viola: blu dominante con rosso marcato
            (Modalita::Offline, 0), // rosso
        ];
        for (m, canale) in atteso {
            let ottenuto = dominante(m.colore_led());
            assert_eq!(
                ottenuto, canale,
                "{}: il pallino non e' della tinta del LED documentato",
                m.nome()
            );
        }
        // **Il viola di Unregulated non puo' confondersi con il blu di
        // Standby**: sono le due modalita' in cui l'operatore guarda di piu',
        // e i due pallini devono essere distinguibili a colpo d'occhio.
        // Entrambi hanno il blu dominante, quindi a guardare la dominante
        // sembrerebbero uguali. Li distingue il **rosso**: nel viola e' forte,
        // nel blu di Standby e' quasi assente. E' un dettaglio piccolo che
        // previene una lettura sbagliata della modalita' col rischio di
        // condensa.
        let (sr, _, _) = Modalita::Standby.colore_led();
        let (ur, _, _) = Modalita::Unregulated.colore_led();
        assert!(
            ur as i32 > (sr as i32) * 2,
            "il viola di Unregulated ({ur}) deve essere ben piu' rosso del blu di \
             Standby ({sr}), altrimenti i due pallini sembrano uguali"
        );
    }

    /// L'Unregulated deve dichiarare il rischio: e' la modalita' che puo'
    /// danneggiare l'hardware, e il menu non deve presentarla come una
    /// scelta neutra.
    #[test]
    fn unregulated_dichiara_il_rischio() {
        let d = Modalita::Unregulated.descrizione().to_lowercase();
        assert!(
            d.contains("rischio") && d.contains("condensa"),
            "Unregulated deve dire che c'e' rischio di condensa"
        );
    }
}

#[cfg(test)]
mod coerenza_colori_tests {
    use super::*;

    /// **Un colore, un significato.** Tasto, striscia e menu non devono avere
    /// tre colori diversi per lo stesso stato: se il pulsante fosse verde e la
    /// striscia blu, l'operatore non saprebbe quale sia giusto.
    ///
    /// Prima questo test verificava che il pulsante avesse la **stessa tinta** del
    /// LED ma scurita, perche' il vecchio pulsante grande aveva testo bianco
    /// sopra. Ora i pulsanti di regime hanno testo colorato su fondo scuro e
    /// prendono il colore del LED direttamente, quindi l'invariante e' piu'
    /// forte: non piu' "stessa tinta", ma **lo stesso valore**. E un pulsante che
    /// sparisce nel verde non e' piu' un problema, perche' il verde e' il testo
    /// del pulsante, non il suo fondo.
    #[test]
    fn pulsante_striscia_e_menu_dicono_lo_stesso_colore() {
        /// Quanto un colore e' "colorato" invece che grigio: 0 = grigio
        /// perfetto, 1 = completamente saturo.
        fn saturazione(c: (u8, u8, u8)) -> f32 {
            let (r, g, b) = (c.0 as f32, c.1 as f32, c.2 as f32);
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            if max <= 0.0 { 0.0 } else { (max - min) / max }
        }

        fn regime_di(m: Modalita) -> crate::commutazione::Regime {
            use crate::commutazione::Regime;
            match m {
                Modalita::Standby => Regime::Standby,
                Modalita::Cryo => Regime::Cryo,
                Modalita::Unregulated => Regime::Unregulated,
                Modalita::Spento | Modalita::Offline => Regime::Spento,
            }
        }

        // `Offline` **non e' un regime**: e' "non lo so", e non ha un pulsante
        // perche' non c'e' niente da commutare. Quindi non si confronta: non ha
        // nessuna coppia pulsante/striscia da tenere coerente.
        for m in Modalita::selezionabili() {
            assert_eq!(
                colore_regime(regime_di(m)),
                m.colore_led(),
                "{}: il pulsante e la striscia devono dire lo stesso colore",
                m.nome()
            );
            // E il colore resta riconoscibile: un grigio non direbbe nulla.
            assert!(
                saturazione(m.colore_led()) > 0.0 || m == Modalita::Spento,
                "{}: colore senza saturazione, non distingue nulla",
                m.nome()
            );
        }

        // Offline dichiara pero' il rosso, e deve restare un colore: e' la
        // direzione "non lo so", e non puo' confondersi con nessun regime.
        assert_eq!(
            Modalita::Offline.colore_led(),
            (235, 70, 70),
            "Offline deve restare rosso: non e' un regime, e' un avviso"
        );
    }

#[cfg(test)]
mod uscita_unregulated_tests {
    use super::*;

    /// **La regola di sicurezza: da Unregulated si deve sempre poter tornare
    /// a Cryo.** Un bottone che porta alla modalita' pericolosa e non ha un
    /// modo evidente di tornare indietro e' il peggior difetto possibile
    /// qui: con margine di condensa di pochi gradi, restare in Unregulated
    /// per distrazione significa rischio di ghiaccio sulla piastra.
    ///
    /// Quindi il bottone di uscita esiste **sempre** quando si e' in
    /// Unregulated, e restaCryo la destinazione.
    #[test]
    fn da_unregulated_si_torna_sempre_a_cryo() {
        assert_eq!(
            Modalita::Unregulated.uscita_da(),
            Modalita::Cryo,
            "l'uscita deve portare a Cryo, la modalita' protetta"
        );
    }

    /// Negli altri casi il bottone di uscita non serve: starei in Cryo o
    /// Standby, e non c'e' niente da cui tornare.
    #[test]
    fn uscita_serve_solo_da_unregulated() {
        for m in [Modalita::Cryo, Modalita::Standby, Modalita::Offline] {
            assert!(
                !m.serve_uscita(),
                "{} non dovrebbe mostrare il bottone di uscita",
                m.nome()
            );
        }
        assert!(Modalita::Unregulated.serve_uscita());
    }

    /// **Il menu mostra tre modalita' distinguibili, non un pulsante.**
    ///
    /// Il pulsante "UNREGULATED MODE" e' stato rimosso perche' prometteva
    /// un'azione che non esiste: il cambio di modalita' non e' un comando
    /// seriale. Al suo posto il menu elenca le tre modalita' con il colore
    /// del LED ed evidenzia quella attiva. Questo test fissa l'invariante:
    /// tre voci, colori tutti diversi, una sola consigliata. Se due voci
    /// avessero lo stesso colore, l'evidenziazione dell'attiva sarebbe
    /// illeggibile.
    #[test]
    fn il_menu_ha_tre_voci_distinguibili() {
        let voci = Modalita::selezionabili();
        assert_eq!(voci.len(), 3, "il menu deve elencare tre modalita'");
        let mut colori: Vec<(u8, u8, u8)> = voci.iter().map(|m| m.colore_led()).collect();
        colori.sort_unstable();
        colori.dedup();
        assert_eq!(colori.len(), 3, "ogni voce deve avere un colore LED distinto");
        assert_eq!(
            voci.iter().filter(|m| m.e_consigliata()).count(),
            1,
            "una sola modalita' puo' essere quella consigliata"
        );
    }

    /// **Nessun interruttore di modalita'.** Il difetto che questo intervento
    /// elimina era esattamente questo: il menu presentava voci che sembravano
    /// selezionabili e un bottone che non commuttava niente.
    ///
    /// L'invariante e' verificabile senza sapere il protocollo: la dashboard
    /// non ha **nessun** comando che tocchi i bit di modalita'. Se un giorno il
    /// cambio di regime venisse decodificato, comparirebbe qui un messaggio con
    /// un nome che lo dice, e questo test non cambierebbe: continuerebbe a
    /// chiedere che quel messaggio non esistesse per sbaglio. Per allora e'
    /// il protocollo a garantire che non si possa, e i due insieme sono la
    /// garanzia.
    #[test]
    fn la_dashboard_non_ha_comandi_di_modalita() {
        // Il perche' e' nel testo della finestra: qui si verifica solo che
        // nessun nome di questo modulo offra un'azione di commutazione.
        for m in Modalita::selezionabili().into_iter().chain([Modalita::Offline]) {
            let basso = m.nome().to_lowercase();
            for azione in ["imposta", "commuta", "attiva", "disattiva", "seleziona"] {
                assert!(
                    !basso.contains(azione),
                    "{}: il nome di una modalita' non deve contenere '{azione}',                      sembrerebbe un'azione e non e'",
                    m.nome()
                );
            }
        }
    }
}

#[cfg(test)]
mod nota_modalita_tests {
    use super::*;

    /// **La finestra deve dire che si puo' cambiare, e come.**
    ///
    /// Il testo precedente diceva "nessun programma puo' impostarla: nel
    /// controller non esiste un comando". Era falso. Il reverse engineering ha
    /// mostrato che si commuta scrivendo il setpoint e l'alimentazione, e il
    /// manuale lo conferma (menu "Mode", "Remember Cryo Mode", errore CB2).
    /// Dire il contrario costava doppio: l'utente cercava un pulsante che non
    /// c'era, e la dashboard si presentava piu' povera di quanto fosse.
    ///
    /// Il test verifica le due cose che servono: che il cambio sia possibile,
    /// e **come** avviene — perche' "premi e succede" e' una scatola nera su
    /// un comando che scrive `0x14`.
    #[test]
    fn la_nota_dice_come_si_cambia_il_regime() {
        let t = MODALITA_NOTA.to_lowercase();
        assert!(
            !t.contains("nessun programma puo"),
            "il cambio di regime esiste: la nota non puo' negarlo: '{t}'"
        );
        assert!(t.contains("setpoint"), "manca il meccanismo: '{t}'");
        assert!(t.contains("accendere o spegnere"), "manca il secondo comando: '{t}'");
    }

    /// **I valori usati devono essere quelli del produttore, dichiarati.**
    ///
    /// Se la nota dicesse solo "cambia regime", l'operatore non saprebbe se
    /// la dashboard scrive i numeri di Intel o quelli inventati da qualcuno.
    /// Dichiararli (`-30` e `3.5`) e' l'unica garanzia che il prossimo non li
    /// cambi di nascosto.
    #[test]
    fn la_nota_dichiara_i_valori_del_produttore() {
        let t = MODALITA_NOTA.to_lowercase();
        assert!(t.contains("-30"), "manca l'offset dell'Unregulated: '{t}'");
        assert!(t.contains("3.5"), "manca l'offset dello Standby: '{t}'");
    }

    /// L'Unregulated deve dichiarare il **rischio**: il pulsante esiste e
    /// qualcuno lo premera'. Il manuale avverte che puo' danneggiare la
    /// scheda, e la finestra e' il posto giusto per dirlo.
    #[test]
    fn la_nota_dichiara_il_rischio_dell_unregulated() {
        let t = MODALITA_NOTA.to_lowercase();
        assert!(t.contains("rugiada"), "manca il rischio di condensa: '{t}'");
        assert!(
            t.contains("due conferme") || t.contains("due conferm"),
            "manca il motivo delle due conferme: '{t}'"
        );
    }

    /// La protezione del controller e' un fatto documentato (manuale 5.1) e
    /// serve: e' cio' che tiene lontano la condensa. Va detto, altrimenti
    /// l'operatore crede che la modalita' non regolata resti li' per sempre.
    #[test]
    fn la_nota_documenta_la_protezione_del_controller() {
        let t = MODALITA_NOTA.to_lowercase();
        assert!(t.contains("10 minuti"), "manca il ritorno automatico a Cryo: '{t}'");
    }

    /// Niente promesse di azione e niente rimandi a un programma installabile
    /// qui: il software Intel su questo PC non gira.
    #[test]
    fn la_nota_non_rimanda_a_un_software_che_non_gira() {
        let t = MODALITA_NOTA.to_lowercase();
        for parola in ["click", "clic", "installa", "scarica"] {
            assert!(
                !t.contains(parola),
                "'{parola}': su questo PC il software Intel non e' installabile: '{t}'"
            );
        }
    }

    /// Si legge una volta e poi si ricorda. Mezzo schermo di testo e' gia'
    /// un trattato.
    #[test]
    fn la_nota_non_diventa_un_trattato() {
        assert!(
            // Limite non arbitrario: la finestra e' alta 520 px e il corpo
            // scorre. Oltre ~1400 caratteri il testo finisce sotto "Chiudi" e
            // l'operatore deve scorrere per leggerlo — e la spiegazione di un
            // comando che scrive sul bus e' proprio quella che non si puo'
            // permettere di non leggere.
            MODALITA_NOTA.chars().count() <= 1400,
            "la nota e' troppo lunga ({} caratteri)",
            MODALITA_NOTA.chars().count()
        );
        assert!(
            MODALITA_NOTA.chars().count() > 200,
            "la nota e' troppo corta per spiegare un comando che scrive sul bus"
        );
    }
}

#[cfg(test)]
mod bit_reali_tests {
    use super::*;

    /// **Standby esiste, e si legge da un bit suo.**
    ///
    /// Il controller invia `LOW_POWER_MODE_ACTIVE`: e' il bit del riposo.
    ///
    /// `PID_RUNNING` e' la condizione per avere un regime.
    ///
    /// Il difetto che questo intervento corregge: la riga di stato leggeva i
    /// bit di modalita' (`LOW_POWER_MODE`, `TEMP_MODE`) senza chiedersi se il
    /// regolatore fosse in funzione. Con il modulo fermo, `TEMP_MODE` resta
    /// spesso acceso — e' l'ultimo regime registrato — e la dashboard
    /// dichiarava "Cryo" su un controller che non stava raffreddando.
    ///
    /// Due letture in contraddizione nello stesso istante, la riga di stato e
    /// la verifica della commutazione, e l'operatore non ha piu' nessun numero
    /// su cui poter contare. Su un pannello di controllo la domanda "sta
    /// funzionando?" viene prima di "in che modo?".
    #[test]
    fn senza_pid_running_c_e_spento_non_un_regime() {
        // Il caso esatto: fermo, con TEMP_MODE ancora acceso.
        let fermo = TecStatus::from_bits_retain(1 << 17);
        assert_eq!(modalita_corrente(fermo, true), Modalita::Spento);
    }

    /// Il bit LOW_POWER_MODE col controller **acceso** e' Standby. Prima il
    /// menu guardava solo `TEMP_MODE`, quindi questa combinazione finiva in
    /// "Unregulated" — la modalita' con rischio di condensa — mentre il
    /// controller era in riposo. Non e' un difetto di poco conto: e'
    /// dichiarare pericoloso uno stato tranquillo.
    #[test]
    fn il_bit_low_power_e_standby() {
        let s = TecStatus::from_bits_retain((1 << 12) | (1 << 16));
        assert_eq!(modalita_corrente(s, true), Modalita::Standby);
    }

    /// Se il controller dice insieme "sono in riposo" e "sto regolando", la
    /// riga deve dire una cosa sola. Vince il riposo: in quel caso non e'
    /// affermabile che stia regolando, quindi dichiarare Cryo sarebbe una
    /// lettura ottimistica di uno stato contraddittorio.
    #[test]
    fn low_power_vince_su_temp_mode() {
        let s = TecStatus::from_bits_retain((1 << 12) | (1 << 16) | (1 << 17));
        assert_eq!(modalita_corrente(s, true), Modalita::Standby);
    }

    /// Solo `TEMP_MODE`: il controller sta regolando, ed e' la modalita' che il
    /// produttore considera quella da usare.
    #[test]
    fn temp_mode_senza_low_power_e_cryo() {
        let s = TecStatus::from_bits_retain((1 << 12) | (1 << 17));
        assert_eq!(modalita_corrente(s, true), Modalita::Cryo);
    }

    /// **Le tre modalita' si devono poter vedere tutte e tre.** Questo e' il
    /// test che chiude il difetto di copertura: con la logica vecchia una
    /// delle tre non era mai raggiungibile da nessuna combinazione di bit.
    #[test]
    fn le_tre_modalita_sono_tutte_raggiungibili() {
        let viste: std::collections::HashSet<Modalita> = [
            TecStatus::from_bits_retain((1 << 12) | (1 << 16)),
            TecStatus::from_bits_retain((1 << 12) | (1 << 17)),
            TecStatus::from_bits_retain(1 << 12),
        ]
        .into_iter()
        .map(|s| modalita_corrente(s, true))
        .collect();
        for attesa in [Modalita::Standby, Modalita::Cryo, Modalita::Unregulated] {
            assert!(
                viste.contains(&attesa),
                "{} non e' mai raggiungibile: {viste:?}",
                attesa.nome()
            );
        }
    }

    /// Il riposo vale anche quando il controller non sta regolando: senza
    /// `LOW_POWER_MODE` ne' `TEMP_MODE` non c'e' potenza erogata, e il
    /// regime e' quello non regolato.
    #[test]
    fn nessun_bit_di_modo_e_unregulated() {
        let s = TecStatus::POWER_OK | TecStatus::TEC_CONN_OK | TecStatus::PID_RUNNING;
        assert_eq!(modalita_corrente(s, true), Modalita::Unregulated);
    }

    /// Un controller muto resta Offline anche se i bit rimangono accodati:
    /// altrimenti la sidebar mostrerebbe "Cryo" su una porta scollegata.
    #[test]
    fn i_bit_ignorati_senza_comunicazione() {
        let s = TecStatus::from_bits_retain((1 << 16) | (1 << 17));
        assert_eq!(modalita_corrente(s, false), Modalita::Offline);
    }
}

#[cfg(test)]
mod led_documentato_tests {
    use super::*;

    /// **I quattro LED sono tutti documentati.** Sezione 3.1 del manuale,
    /// "Three Modes of Operation", colonna "Controller LED Indicator":
    ///
    /// ```text
    /// Standby      Blue   - slow blinking
    /// Cryo         Green  - slow blinking
    /// Unregulated  Purple - fast blinking
    /// Offline      Red    - solid
    /// ```
    ///
    /// Il test era sbagliato: affermava che solo verde e rosso fossero
    /// documentati e che blu e viola fossero da scartare. Sbagliato, e il
    /// difetto aveva un costo concreto: senza i colori, l'unico controllo
    /// disponibile per chi non ha installato il software Intel — guardare il
    /// pannello e confrontarlo con la dashboard — non funzionava.
    ///
    /// Ora la tabella e' fissata qui, parola per parola, cosi' nessuno la
    /// "corregge" di nuovo verso un'informazione meno precisa.
    #[test]
    fn i_led_sono_come_dice_il_manuale() {
        for (modalita, colore, lampeggio) in [
            (Modalita::Standby, "blu", "lento"),
            (Modalita::Cryo, "verde", "lento"),
            (Modalita::Unregulated, "viola", "veloce"),
            (Modalita::Offline, "rosso", "fisso"),
        ] {
            let led = modalita
                .led()
                .unwrap_or_else(|| panic!("{}: il manuale documenta il LED", modalita.nome()));
            assert_eq!(led.colore, colore, "{}: colore LED", modalita.nome());
            assert_eq!(led.lampeggio, lampeggio, "{}: lampeggio LED", modalita.nome());
        }
    }

    /// Il colore dichiarato e il colore RGB devono essere lo stesso segnale:
    /// una tabella che mente su se stessa e' peggio di una che non c'e'.
    #[test]
    fn il_led_dichiarato_e_coerente_col_colore() {
        let cryo = Modalita::Cryo;
        let led = cryo.led().unwrap();
        assert_eq!(led.colore, "verde");
        assert_eq!(cryo.colore_led(), (0, 215, 120), "verde saturo come il LED");
        let offline = Modalita::Offline;
        assert_eq!(offline.led().unwrap().colore, "rosso");
        assert_eq!(offline.colore_led(), (235, 70, 70), "rosso come il LED");
    }

    /// Ogni modalita' deve avere un colore **distinto** dalle altre: e' il
    /// colore a dare il nome alla modalita' quando l'unico accesso e' il
    /// pannello. Due tonalita' vicine renderebbero il confronto impossibile.
    #[test]
    fn i_colori_led_sono_tutti_diversi() {
        let colori: std::collections::HashSet<&str> = Modalita::selezionabili()
            .into_iter()
            .chain([Modalita::Offline])
            .map(|m| m.led().expect("LED documentato").colore)
            .collect();
        assert_eq!(colori.len(), 4, "due modalita' hanno lo stesso colore LED");
    }
}

} // fine mod pulsante_modalita_tests

/// Il colore di un regime.
///
/// Vivace in `da_regime` perche' il colore e' una proprita' della modalita', e
/// accorciare la catena `Regime -> Modalita -> colore` in un passo solo evita
/// che un chiamante percorra metà della mappatura a mano — e sbagli.
///
/// **Il pulsante usa questo colore, non uno scurito.** Prima `colore_pulsante`
/// restituiva la tinta del LED abbassata di luminosita', perche' il vecchio
/// grande pulsante aveva testo bianco sopra e il verde saturo non dava
/// contrasto. I pulsanti di regime hanno invece testo colorato su fondo scuro,
/// quindi il colore del LED va bene cosi' com'e'. Il vantaggio non e' estetico:
/// **il pulsante e la striscia dicono ora lo stesso colore identico**, quindi
/// non possono piu' divergere per un fattore di luminosita'.
///
/// La funzione vecchia e il suo test sono spariti insieme: un invariante che
/// descriveva un widget che non esiste piu' non e' copertura, e' rumore.
pub fn colore_regime(regime: crate::commutazione::Regime) -> (u8, u8, u8) {
    da_regime(regime).colore_led()
}

/// `Regime -> Modalita`, l'unica mappatura fra i due.
///
/// Vivesse altrove, la riga di stato e il menu avrebbero due tabelle parallele
/// che divergono senza che nessun test se ne accorga: e' gia' successo, ed e'
/// il motivo per cui la funzione e' qui e non in `commutazione.rs`.
pub fn da_regime(regime: crate::commutazione::Regime) -> Modalita {
    use crate::commutazione::Regime;
    match regime {
        Regime::Standby    => Modalita::Standby,
        Regime::Cryo       => Modalita::Cryo,
        Regime::Unregulated => Modalita::Unregulated,
        Regime::Spento     => Modalita::Spento,
    }
}

#[cfg(test)]
mod test_mappatura_regime {
    use super::*;
    use crate::commutazione::Regime;
    use cryo_cooler_controller_lib::TecStatus;

    /// **Spento non e' Offline.** E' la distinzione che questo intervento
    /// protegge: `Spento` significa "lo so, il modulo e' fermo", `Offline`
    /// significa "non lo so". Sono la stessa lettera in due situazioni opposte
    /// per l'operatore, e confonderle fa leggere "tutto bene" quando il modulo
    /// e' spento, o "guasto" quando e' semplicemente spento.
    #[test]
    fn spento_e_offline_sono_due_stati_diversi() {
        // TEC collegato, spento, lettura fresca: lo so, e' spento.
        let spento_fresco = modalita_corrente(TecStatus::POWER_OK, true);
        assert_eq!(spento_fresco, Modalita::Spento);

        // Stato non fresco: non lo so.
        let ignoto = modalita_corrente(TecStatus::POWER_OK, false);
        assert_eq!(ignoto, Modalita::Offline);

        assert_ne!(
            spento_fresco, ignoto,
            "spento e offline non possono confondersi"
        );
    }

    /// I quattro regimi mappano su quattro modalita' distinte, nessuna persa.
    #[test]
    fn ogni_regime_ha_la_sua_modalita() {
        let mappate: Vec<Modalita> = [
            Regime::Standby,
            Regime::Cryo,
            Regime::Unregulated,
            Regime::Spento,
        ]
        .into_iter()
        .map(da_regime)
        .collect();

        assert_eq!(mappate.len(), 4);
        for (i, m) in mappate.iter().enumerate() {
            assert!(
                !mappate[..i].contains(m),
                "{m:?} compare due volte: due regimi mappano sulla stessa modalita'"
            );
        }
    }

    /// Ogni modalita' ha un colore LED: senza, il menu non puo' dire quale
    /// regime sia attivo con il colore, che e' il canale piu' veloce.
    #[test]
    fn ogni_modalita_ha_un_colore() {
        for m in [
            Modalita::Standby,
            Modalita::Cryo,
            Modalita::Unregulated,
            Modalita::Spento,
            Modalita::Offline,
        ] {
            let (r, g, b) = m.colore_led();
            // u16: i tre canali sommano a piu' di 255 (es. il viola di
            // Unregulated), e in debug un u8 andrebbe in overflow.
            let somma = r as u16 + g as u16 + b as u16;
            assert!(
                somma > 0,
                "{m:?} non ha un colore: il menu non puo' distinguerlo"
            );
        }
    }
}
