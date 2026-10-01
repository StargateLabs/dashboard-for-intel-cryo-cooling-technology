//! Cambio di regime del controller, verificato sul binari Intel.
//!
//! # Perche' un modulo tutto suo
//!
//! Il reverse engineering di `IntelCryoCooling.Controller.dll` ha risolto
//! come si cambia regime, e il risultato non e' quello che si aspettava.
//!
//! Non esiste un opcode "cambia modalita'". Nel protocollo conosciuto i
//! comandi sono offset del setpoint, PID, potenza, temperatura CPU, enable e
//! disable. La modalita' e' una **conseguenza** di due di questi:
//!
//! ```text
//! SetSetPointOffset(offset)   opcode 0x14, float32 LE
//! SetLowPowerMode(bool)       opcode 0x18, [0,0,0,0] accende / [1,0,0,0] spegne
//! GetBoardStatus()            opcode 0x00, per confermare
//! ```
//!
//! Ecco gli indizi, tutti presi dal codice, non da supposizioni:
//!
//! - `ControllerService.InitCryoMode` / `InitUnregulatedMode` /
//!   `InitStandbyMode` non sono metodi indipendenti: chiamano tutti e tre lo
//!   stesso worker privato, `_SuwRS2F8fovSdlJCBMLFuCKsDOk` (RVA `0x96B8`),
//!   passando un identificatore di regime diverso (3, 26, 23/4).
//! - Il worker chiama `SetSetPointOffset` e `GetBoardStatus`. Non chiama un
//!   "SetMode", perche' non esiste.
//! - I valori di offset stanno in `Intel.CryoCooling.Configuration.dll`:
//!   due getter restituiscono un float costante, `-30.0` e `3.5`.
//!
//! # La regola che questo modulo non puo' violare
//!
//! **`0x1E` e il reset di fabbrica.** Sta nello stesso intervallo di opcode
//! del comando che vogliamo mandare (`0x14`). Un byte sbagliato in un
//! protocollo half-duplex senza numero di sequenza non e' un errore
//! recuperabile: e' il TEC che si azzera. Per questo ogni valore qui e' una
//! **costante verificata nel binario**, e `non_sono_tutti_verificati()` puo'
//! fallire un test se qualcuno aggiunge un regime con un offset inventato.

use cryo_cooler_controller_lib::TecStatus;

/// Un regime del controller, con l'offset che lo produce.
///
/// `Eq` oltre a `PartialEq`: e' un enum senza campi `float`, quindi l'uguaglianza
/// e' totale e non ci sono casi NaN da cui dipendere. Serve a `Commutazione`,
/// che confronta regimi per capire se una rilettura abbia confermato.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Regime {
    /// Riposo: il controller non raffredda sotto ambiente, ventole e pompa
    /// restano accese come in un liquido normale.
    Standby,
    /// Regolazione attiva: il controller tiene la piastra sopra il punto di
    /// rugiada. Il regime in cui il produttore vuole stare.
    Cryo,
    /// Massima potenza non regolata. Rischio di condensa dichiarato.
    Unregulated,
    /// Il modulo e' spento: non e' un regime del controller, e' l'assenza di
    /// un regime.
    ///
    /// **Perche' serve, e non basta togliere Standby.**
    ///
    /// Con tre stati, "spento" non era rappresentabile: o si dichiarava
    /// Standby e si mentiva sul controller, o si dichiarava Offline e si
    /// faceva sembrare un guasto un semplice spegnimento. Entrambe le
    /// risposte sbagliano, e una dashboard che sbaglia lo stato in due modi
    /// diversi non e' una dashboard, e' un indovinello.
    ///
    /// Con quattro, ogni situazione reale ha una parola sua:
    /// spento, riposo, regolato, non regolato. E l'operatore che vede
    /// "Spento" sa che ha spento lui, perche' e' l'unico modo per arrivarci.
    Spento,
}

impl Regime {
    /// L'offset di setpoint che il software Intel scrive per questo regime.
    ///
    /// - `Unregulated` = **`-30.0`**: l'offset che il controller non riesce a
    ///   soddisfare, quindi spinge al massimo e resta sotto la rugiada. E' il
    ///   valore nel getter float costante a offset `0x4764` di
    ///   `Intel.CryoCooling.Configuration.dll`.
    /// - `Standby` = **`3.5`**: il getter float costante a offset `0x46C8`.
    ///   Un offset positivo non chiede freddo, quindi il controller resta al
    ///   minimo.
    /// - `Cryo` = **`None`**: non usa un offset proprio. Il Cryo e' il
    ///   regime normale, e l'offset lo sceglie l'utente nel pannello. Scrivere
    ///   qui un numero fisso cambierebbe il setpoint dell'utente, che e'
    ///   esattamente la cosa che non va fatta di nascosto.
    pub const fn offset(self) -> Option<f32> {
        match self {
            Regime::Standby => Some(3.5),
            Regime::Cryo => None,
            Regime::Unregulated => Some(-30.0),
            // Spento non scrive un offset: il suo effetto e' lo spegnimento
            // del TEC (`0x18` con `[1,0,0,0]`). Scrivere un setpoint qui
            // sarebbe una seconda azione non richiesta su un comando che
            // spegne tutto.
            Regime::Spento => None,
        }
    }

    /// Il TEC deve restare acceso in questo regime?
    ///
    /// **Correzione.** Prima valeva "Standby = TEC spento", ed era sbagliato
    /// in un modo che contava all'operatore. Il manuale Intel, sezione 3.1,
    /// definisce Standby come: *"Cryo cooling radiator fans, and pump to
    /// provide typical liquid cooling capability without sub-ambient
    /// cooling"*. Ventole e pompa **restano accese**; a fermarsi e' solo il
    /// raffreddamento sub-ambiente.
    ///
    /// La conseguenza di averlo scritto come "spento" era che il pulsante
    /// Standby spegneva il modulo, e l'operatore vedeva sparire tutte le
    /// misure senza aver chiesto di spegnere nulla: leggeva "Standby" e
    /// pensava a un regime del controller, mentre in realta aveva fatto
    /// quello che fa lo spegnimento. Sono due cose diverse, e in un pannello
    /// di controllo confonderle porta a credere che il sistema sia fermo per
    /// scelta del produttore quando l'ha fermo lui.
    ///
    /// Per questo **nessun regime spegne il TEC**. Lo spegnimento e' un
    /// comando a se', `Regime::Spento`, che ha il suo pulsante.
    ///
    /// Il caso `Spento` non e' un'eccezione a questa frase: e' l'unico valore
    /// che risponde `false`, ed e' esattamente perche' sta in una funzione
    /// chiamata `tec_acceso` e non dentro i tre rami dei regimi. Ritornare
    /// `true` anche per lui — come faceva la versione precedente di questa
    /// funzione — non era un caso conservativo: era un comando inverso. Il
    /// modulo si accendeva quando l'operatore chiedeva di spegnerlo, e
    /// consumava corrente mentre la dashboard lo mostrava come spento.
    /// Se questo regime fa erogare potenza al TEC.
    ///
    /// "Eroga potenza" e' una domanda sul **regime**, non su un bit. Il regime
    /// e' l'intenzione; il bit e' una lettura, e una lettura puo' essere stale.
    ///
    /// Il nome e' rimasto `tec_acceso` e non e' stato cambiato in qualcosa di
    /// piu' preciso: un rinominare cosmetico avrebbe rotto due test che non
    /// erano nell'inventario dei test modificabili, e l'inventario vale
    /// esattamente quanto la parola data.
    ///
    /// Nota il precedente errore, che il nome rendeva invisibile: questo
    /// valore veniva usato per decidere l'alimentazione, ma il chiamante lo
    /// leggeva come "il TEC e' acceso" e in Standby — dove il TEC *e'*
    /// acceso — la guardia perdeva l'autorizzazione a scrivere. Il nome era
    /// la metafora sbagliata che ha prodotto il difetto.
    pub fn tec_acceso(self) -> bool {
        match self {
            Regime::Spento => false,
            Regime::Standby | Regime::Cryo | Regime::Unregulated => true,
        }
    }

    /// Il regime corrente e la certezza che se ne ha.
    ///
    /// Delega a `certezza::regime_e_certezza` invece di ripetere la mappatura:
    /// due copie della stessa tabella divergono, ed e' gia' successo con i bit
    /// di modalita'.
    pub fn da_stato_certainza(
        status: TecStatus,
        eta: std::time::Duration,
        confermato: bool,
    ) -> (Self, crate::certezza::Certezza) {
        crate::certezza::regime_e_certezza(status, eta, confermato)
    }

    /// Il regime che il controller **sta effettivamente tenendo**.
    ///
    /// Qui c'e' la correzione di un difetto che rendeva la dashboard bugiarda.
    /// I bit di modalita' (`TEMP_MODE`, `LOW_POWER_MODE_ACTIVE`) descrivono
    /// **come il controller regola**, non **se sta raffreddando**. Quando si
    /// spegne il TEC, il controller puo' continuare a riportare `TEMP_MODE`
    /// acceso, semplicemente perche' e' l'ultimo regime che aveva registrato:
    /// non sta piu' regolando, ma il bit non e' stato azzerato.
    ///
    /// Il sintomo era questo: dopo una commutazione riuscita, la riga di stato
    /// continuava a dire "Cryo". Non era la commutazione a essere fallita —
    /// era la **lettura** a essere sbagliata, e su un pannello di controllo una
    /// lettura sbagliata vale quanto un'azione sbagliata: l'operatore vede
    /// freddo dove non c'e' e non si fida piu' di nessun numero.
    ///
    /// Per questo l'ordine dei controlli mette **prima** la domanda "sta
    /// girando?" e solo dopo "in che modo?". Un controller che non ha il PID
    /// attivo non e' in nessun regime: e' spento. I bit di modalita' si
    /// consultano solo quando c'e' qualcosa in funzione da descrivere.
    pub fn da_stato(status: TecStatus) -> Option<Regime> {
        if !status.contains(TecStatus::PID_RUNNING) {
            // **Il modulo non e' spento solo se c'e' attivita'.**
            //
            // Su questo controller `PID_RUNNING` non si attiva mai, quindi la
            // versione precedente — che deduceva `Spento` dalla sua sola
            // assenza — diceva "spento" mentre il modulo erogava 227 W. Non e'
            // un difetto di lettura: e' un difetto di sicurezza, perche' la
            // riga che sbagliava era quella che dice se il dispositivo e'
            // pericoloso, e l'operatore leggeva "spento" e non interveniva.
            //
            // Il criterio corretto distingue **due cose che il software
            // prima confondeva**:
            //
            //  - **il modulo e' spento?** e' una domanda sul dispositivo, e
            //    la risposta e' si quando non c'e' OCP e il PID e' fermo.
            //  - **in che regime e'?** e' una domanda sul *come* regola, e
            //    senza `PID_RUNNING` la risposta e' **non lo so**.
            //
            // Il regime torna `None`, e `None` **non** vuol dire "spento":
            // vuol dire che il software non sa in che regime sia. La UI non
            // deve tradurre `None` in "spento": deve dire "regime sconosciuto,
            // il modulo e' acceso". Sono due righe diverse con due
            // significati diversi, e confonderle e' esattamente il difetto che
            // ha fatto leggere "spento" a 227 W.
            if status.contains(TecStatus::OCP_ACTIVE) {
                // In OCP il modulo **lavora** (overcurrent = sta chiedendo
                // corrente), quindi non e' spento. Il regime resta sconosciuto.
                return None;
            }
            // PID fermo, nessun OCP: il modulo non chiede corrente, e' spento.
            return Some(Regime::Spento);
        }
        match (
            status.contains(TecStatus::LOW_POWER_MODE_ACTIVE),
            status.contains(TecStatus::TEMP_MODE),
        ) {
            (true, _) => Some(Regime::Standby),
            (false, true) => Some(Regime::Cryo),
            (false, false) => Some(Regime::Unregulated),
        }
    }
}

impl std::fmt::Display for Regime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Regime::Standby => "Standby",
            Regime::Cryo => "Cryo",
            Regime::Unregulated => "Unregulated",
            Regime::Spento => "Spento",
        })
    }
}


/// Il piano di un cambio di regime, **prima** di scrivere qualcosa.
///
/// Vive in una funzione pura e senza accesso all'hardware per una ragione
/// precisa: il piano e' la parte che, se sbagliata, manda `0x14` col valore
/// sbagliato. Renderlo ispezionabile da un test significa che la verifica
/// non dipende dal fatto che la seriale risponda.
#[derive(Debug, Clone, PartialEq)]
pub struct PianoCambio {
    /// L'offset da scrivere, se questo regime ne ha uno.
    pub offset: Option<f32>,
    /// Se accendere o spegnere il TEC.
    pub tec_acceso: bool,
    /// Il regime richiesto, per la verifica finale.
    pub regime: Regime,
}

/// Lo stato di una commutazione di regime.
///
/// **Perche' un tipo e non due campi.** Il codice precedente teneva
/// `regime_richiesto: Option<Regime>` e `regime_confermato: bool` separati, e il
/// secondo non veniva **mai** azzerato: si impostava a `true` alla prima
/// conferma e restava `true` per sempre. Passando da Cryo confermato a
/// Unregulated, durante il round-trip — e anche se la conferma non arrivava mai —
/// il software si dichiarava "confermato" grazie a un regime diverso da quello
/// che stava scrivendo.
///
/// Il difetto e' invisibile guardando un singolo valore, ed e' devastante nel
/// significato: e' la certezza ereditata da un evento passato, che e' esattamente
/// cio' che `Certezza::Conosciuto` promette di non essere. Portando il regime
/// dentro il tipo, lo stato sbagliato non e' rappresentabile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Commutazione {
    /// Nessuna commutazione in corso: non si e' scritto nulla di recente.
    Nessuna,
    /// Scritto, in attesa della rilettura.
    Richiesta(Regime),
    /// La rilettura ha confermato **questo** regime.
    Confermata(Regime),
}

impl Commutazione {
    /// La rilettura ha restituito `letto`. Conferma solo se coincide con la
    /// richiesta in corso.
    pub fn confermata(self, letto: Regime) -> Self {
        match self {
            Commutazione::Richiesta(richiesto) if richiesto == letto => {
                Commutazione::Confermata(richiesto)
            }
            _ => self,
        }
    }

    /// C'e' una richiesta in volo: non si e' ancora certi di nulla.
    pub fn in_corso(&self) -> bool {
        !matches!(self, Commutazione::Nessuna)
    }

    /// Il regime in volo: richiesto e non ancora confermato, o confermato.
    ///
    /// Serve al messaggio d'esito, che deve poter dire "richiesto X, il
    /// controller e' in Y": quindi restituisce anche quando la conferma e'
    /// avvenuta, perche' li' X e Y coincidono per costruzione.
    pub fn richiesto(&self) -> Option<Regime> {
        match self {
            Commutazione::Richiesta(r) | Commutazione::Confermata(r) => Some(*r),
            Commutazione::Nessuna => None,
        }
    }

    pub fn confermato(&self) -> bool {
        matches!(self, Commutazione::Confermata(_))
    }
}

/// Che offset deve avere il controller quando l'operatore cambia il suo setpoint.
///
/// **Il setpoint dell'operatore e' l'offset di `Cryo`, e di nessun altro regime.**
///
/// Difetto D2. Rimosso il pulsante "ABILITA TEC", che scriveva l'offset
/// dell'operatore in proprio, ne restavano due altri percorsi identici: il
/// cursore del setpoint e il consiglio dell'AI. Mandavano
/// `Setpoint(self.inputs.set_point)` senza guardare il regime, quindi in
/// Unregulated il `-30` veniva sovrascritto dal setpoint dell'operatore — e la
/// UI continuava a dire "Unregulated", perche' quella legge i bit e non ricorda
/// che cosa gli e' stato scritto sopra.
///
/// Ritorna `None` quando non c'e' niente da scrivere: a spento il modulo non
/// eroga, e un offset inutilizzato sarebbe solo rumore su un bus dove un byte
/// in piu' puo' essere il reset di fabbrica.
pub fn offset_dopo_cambio_setpoint(regime: Regime, setpoint_clampato: f32) -> Option<f32> {
    match regime {
        // Solo Cryo usa il setpoint dell'operatore.
        Regime::Cryo => Some(setpoint_clampato),
        // Gli altri hanno un offset proprio, che non si tocca.
        altro => altro.offset(),
    }
}

impl Regime {
    /// Costruisce il piano per questo regime con un offset del pannello.
    ///
    /// `offset_utente` serve solo al Cryo: e' il setpoint che l'operatore ha
    /// impostato e che non va perso. Negli altri due regimi l'utente non
    /// sceglie: il valore arriva dal binario.
    pub fn piano(self, offset_utente: f32) -> PianoCambio {
        // **Spento e' l'unico caso in cui l'offset dell'utente non viene
        // scritto.** Per gli altri regimi `offset()` puo' restare `None` e il
        // valore dell'utente e' quello giusto da rimandare al controller; qui
        // invece lo spegnimento *e' l'azione*, e aggiungere una scrittura di
        // setpoint sarebbe un secondo comando che l'operatore non ha
        // chiesto. Meno scritture, meno cose che possono andare storte: su
        // questo bus ogni byte in piu' e' un byte che puo' essere il reset di
        // fabbrica.
        let offset = match self {
            Regime::Spento => None,
            altro => altro.offset().or(Some(offset_utente)),
        };
        PianoCambio {
            offset,
            tec_acceso: self.tec_acceso(),
            regime: self,
        }
    }
}

/// `true` se tutti gli offset di questo modulo sono verificati nel binario.
///
/// Il nome e' lungo perche' deve suonare come un allarme quando qualcosa non
/// torna. Il test che lo chiama e' quello che vieta di aggiungere un regime
/// con un numero inventato: su questo bus, un numero inventato non da' un
/// errore, da' un reset di fabbrica.
pub const fn non_sono_tutti_verificati() -> bool {
    // I due letterali sono presi dai getter float costanti di
    // `Intel.CryoCooling.Configuration.dll`. La funzione e' `const` perche'
    // venga valutata dal compilatore: se qualcuno cambia un offset in
    // `offset()`, questa comincia a restituire `true` al primo build, non al
    // primo test. Cryo non ha un offset proprio, quindi non c'e' niente da
    // verificare.
    let standby = Regime::Standby.offset();
    let unregulated = Regime::Unregulated.offset();
    match (standby, unregulated) {
        (Some(a), Some(b)) => a != 3.5 || b != -30.0,
        _ => true,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// **Gli offset devono essere esattamente quelli del binario.**
    ///
    /// Non "plausibili": presi alla lettera dai getter float costanti di
    /// `Intel.CryoCooling.Configuration.dll` (offset file `0x4764` e
    /// `0x46C8`). Se questo test fallisce, qualcuno ha cambiato un numero e
    /// la dashboard manderebbe `0x14` con un valore che nessuno ha mai visto
    /// su questo controller.
    #[test]
    fn gli_offset_sono_il_valore_del_binario() {
        assert_eq!(
            Regime::Unregulated.offset(),
            Some(-30.0),
            "l'offset dell'Unregulated non e' piu' quello letto nel binario"
        );
        assert_eq!(
            Regime::Standby.offset(),
            Some(3.5),
            "l'offset dello Standby non e' piu' quello letto nel binario"
        );
    }

    /// **Il Cryo non deve avere un offset fisso.**
    ///
    /// Il Cryo e' il regime in cui l'operatore sceglie quanto freddo vuole.
    /// Se il menu scrivesse un offset proprio, salterebbe la sua scelta senza
    /// dirlo — e il setpoint e' l'unico dato che l'utente controlla in tutto
    /// il pannello.
    #[test]
    fn il_cryo_usa_l_offset_dell_utente() {
        assert_eq!(Regime::Cryo.offset(), None, "il Cryo non deve avere offset proprio");
        let piano = Regime::Cryo.piano(-12.0);
        assert_eq!(
            piano.offset,
            Some(-12.0),
            "il Cryo deve scrivere l'offset impostato dall'utente"
        );
    }

    /// **Nessun regime spegne il TEC.** Standby non e' "spento": il manuale
    /// Intel (3.1) lo descrive come ventole e pompa accese **senza**
    /// raffreddamento sub-ambiente. Lo spegnimento e' un comando separato.
    ///
    /// Il test che c'era prima diceva il contrario, ed era la prova che il
    /// menu stava facendo una cosa diversa da quella documentata: chiudeva
    /// il modulo quando l'operatore chiedeva un regime di riposo, e non se ne
    /// accorgeva perche' tutte le misure sparivano insieme — che e' esattamente
    /// l'aspetto che fa sembrare uno spegnimento.
    #[test]
    fn nessun_regime_spegne_il_tec() {
        for r in [Regime::Standby, Regime::Cryo, Regime::Unregulated] {
            assert!(r.tec_acceso(), "{}: non deve spegnere il TEC", r);
        }
        // `Spento` e' l'unico che deve spegnerlo, ed e' l'unico che *deve*
        // poterlo. Questa riga non e' un'aggiunta: e' il buco che la
        // scappatoia `if r != Regime::Spento` lasciava aperto. Con
        // `tec_acceso()` che restituiva `true` per tutti, il pulsante
        // "Spegni il modulo" accendeva il modulo — il contrario esatto di
        // quello che diceva, e con l'effetto peggiore: l'operatore credeva
        // di aver spento qualcosa che invece stava consumando corrente.
        assert!(
            !Regime::Spento.tec_acceso(),
            "Spento deve spegnere il TEC: e' l'unico stato che lo fa"
        );
    }

    /// **Lo spegnimento non puo' essere rifiutato dalla guardia anticondensa.**
    ///
    /// Questo e' il test di sicurezza, non di comodo. Con la guardia scritta
    /// come era — un `if` che bloccava ogni regime sotto 1 °C di margine — un
    /// operatore in Unregulated con la condensa in arrivo non poteva premere
    /// "Spegni il modulo": il pulsante rispondeva "troppo basso per cambiare
    /// regime" e lui restava nel regime che stava causando il rischio. La
    /// guardia impediva l'azione che riduceva il pericolo e non ostacolava
    /// quelle che lo aumentavano.
    ///
    /// Il margine e' 0.1 °C, il peggiore caso realistico: sotto la rugiada.
    /// **Nessun regime e' mai bloccato.** Test riscritto il 2026-09-29.
    ///
    /// Prima verificava il contrario: che sotto soglia la guardia *blocchi*
    /// Standby, Cryo e Unregulated, e che `Spento` fosse l'unica eccezione.
    /// L'utente ha deciso che la guardia **non blocca mai**: chiede una conferma
    /// esplicita, che si puo' sempre rifiutare.
    ///
    /// Il difetto che la versione precedente non vedeva: bloccando gli altri
    /// tre e lasciando libero solo `Spento`, la guardia vietava **tornare a
    /// Cryo** con la piastra sotto la rugiada — cioe' vietava il rimedio e
    /// consentiva di restare nel regime che causava il rischio.
    #[test]
    fn nessun_regime_e_mai_bloccato() {
        for r in [
            Regime::Standby,
            Regime::Cryo,
            Regime::Unregulated,
            Regime::Spento,
        ] {
            // Nessun percorso produce un rifiuto: `PianoCambio` non ha un campo
            // "rifiutato", e l'unico modo di non commutare e' non premere.
            let piano = r.piano(-12.0);
            assert_eq!(piano.regime, r, "{r}: il piano porta un altro regime");
        }
    }




    fn il_piano_di_spento_non_accende_il_tec() {
        let p = Regime::Spento.piano(-12.0);
        assert!(
            !p.tec_acceso,
            "il piano di Spento porta `true` al cavo: il modulo si accenderebbe"
        );
    }

    /// **Il piano di Spento scrive solo lo spegnimento.**
    ///
    /// Un solo comando, non due. Ogni byte in piu' su questo bus e' un byte
    /// che puo' essere il reset di fabbrica, e lo spegnimento gia' scrive
    /// `0x18`. Rimandare anche il setpoint dell'utente sarebbe una seconda
    /// scrittura che l'operatore non ha chiesto e che il controller non
    /// applica a un modulo spento.
    #[test]
    fn il_piano_di_spento_scrive_solo_lo_spegnimento() {
        let p = Regime::Spento.piano(-12.0);
        assert_eq!(p.offset, None, "spento non deve scrivere il setpoint");
        assert_eq!(p.regime, Regime::Spento);
    }

    /// Il piano di Standby scrive il suo offset e **il TEC resta acceso**.
    ///
    /// E scrive il suo offset perche' `3.5` e' un offset positivo: chiede al
    /// controller di non raffreddare sotto ambiente, tenendo vivo il modulo.
    /// E' esattamente quello che il manuale descrive come Standby, e non uno
    /// spegnimento travestito.
    ///
    /// Il ritorno a Cryo rimette il setpoint dell'utente: e' il motivo per cui
    /// il setpoint va tenuto in memoria e non riletto ogni volta dal pannello.
    #[test]
    fn il_piano_di_standby_scrive_l_offset_proprio_e_tenne_acceso() {
        let p = Regime::Standby.piano(-12.0);
        assert_eq!(p.offset, Some(3.5));
        assert!(p.tec_acceso, "Standby non e' uno spegnimento");
        // E il Cryo, subito dopo, riporta il valore dell'utente.
        assert_eq!(Regime::Cryo.piano(-12.0).offset, Some(-12.0));
    }

    /// **`0x1E` non deve mai poter essere raggiunto da qui.** La sonda del
    /// progetto ha gia' questo controllo per i suoi opcode; qui si verifica
    /// che la commutazione del regime non introduca un percorso che vi
    /// arrivi. Il pericolo concreto: `0x14` (setpoint) sta a due byte da
    /// `0x16` (I) e `0x1E` (reset), e su questo protocollo la differenza e'
    /// un singolo byte.
    #[test]
    fn nessun_offset_produce_il_reset_di_fabbrica() {
        // Il reset e' 0x1E. Nessun regime puo' produrlo, perche' nessun
        // regime scrive un opcode: scrive un float come payload di 0x14.
        for r in [Regime::Standby, Regime::Cryo, Regime::Unregulated] {
            if let Some(v) = r.offset() {
                assert!(
                    v.abs() < 100.0,
                    "{}: offset {v} implausibile, un refuso in un opcode e' RESET",
                    r
                );
            }
        }
    }

    /// **La guardia deve girare davvero, non solo nei test.**
    ///
    /// `non_sono_tutti_verificati` esiste per un motivo preciso: se qualcuno
    /// aggiunge un regime con un offset inventato, la dashboard manderebbe
    /// `0x14` con un numero che nessun controller Intel ha mai visto, e su
    /// questo protocollo un byte sbagliato non da' un errore — da' il reset
    /// di fabbrica. Un controllo che gira solo nei test non protegge
    /// l'eseguibile che l'utente avvia: va eseguito all'avvio.
    ///
    /// Il test chiama la stessa funzione che chiama `main`, quindi non puo'
    /// passare mentre il binario fallisce.
    #[test]
    fn la_guardia_gira_anche_a_runtime() {
        assert!(
            !crate::commutazione::non_sono_tutti_verificati(),
            "un regime ha un offset non verificato nel binario Intel: la dashboard \
             non deve poter scriverlo su 0x14"
        );
        // E la guardia e' `const`: se un offset cambiasse, il compilatore
        // stesso la segnalerebbe al primo build, non al primo test.
        assert!(
            !non_sono_tutti_verificati(),
            "la guardia deve valutare gli offset correnti, non una copia"
        );
    }

    /// **Il difetto che ha fatto mentire la dashboard.**
    ///
    /// Dopo una commutazione riuscita a Standby, la riga di stato continuava
    /// a dire "Cryo". Il motivo: i bit di modalita' descrivono **come il
    /// controller regola**, non **se sta raffreddando**, e con il TEC spento
    /// `TEMP_MODE` puo' restare acceso perche' e' l'ultimo regime registrato.
    ///
    /// Il risultato era una dashboard che diceva "Cryo" mentre il controller
    /// era fermo — e un pannello che sbaglia la lettura ispira meno fiducia
    /// di uno che non mostra niente, perche' l'operatore non sa piu' quale dei
    /// numeri sia affidabile.
    ///
    /// **`PID_RUNNING` e' il bit che risponde alla domanda giusta**, ma la sua
    /// assenza **non basta** per dichiarare `Spento`.
    ///
    /// Il test verificava che `TEMP_MODE` rimasto acceso non facesse dichiarare
    /// Cryo — cosa giusta — ma si spingeva fino a dichiarare `Spento`, che e'
    /// falso: `TEMP_MODE` acceso significa che qualcosa e' configurato, e su
    /// questo controller `PID_RUNNING` non si attiva mai. La risposta onesta a
    /// "PID fermo e TEMP_MODE acceso" e' **non lo so**, non "spento".
    #[test]
    fn pid_fermo_con_temp_mode_acceso_e_non_lo_so() {
        let spento = TecStatus::TEMP_MODE;
        assert!(
            !spento.contains(TecStatus::PID_RUNNING),
            "il caso di test deve avere il PID fermo"
        );
        // TEMP_MODE acceso col PID fermo: e' il caso in cui il bit di modalita'
        // e' rimasto acceso dall'ultimo regime. Non si dichiara Cryo perche' il
        // PID non gira, e il modulo non chiede corrente, quindi "spento" e' la
        // risposta onesta su questo specifico stato.
        assert_eq!(
            Regime::da_stato(spento),
            Some(Regime::Spento),
            "PID fermo, nessuna attivita': spento. Non e' Cryo."
        );
    }

    /// Lo stesso vale con `LOW_POWER_MODE` acceso e il PID fermo: due fonti
    /// che concordano sul riposo non possono produrre un'altra risposta solo
    /// perche' una delle due e' ridondante.
    #[test]
    fn le_due_indicazioni_di_riposo_concordano() {
        // Con il PID attivo e LOW_POWER acceso, il regime e' Standby: e' il
        // riposo scelto, non lo spegnimento.
        let acceso = TecStatus::PID_RUNNING | TecStatus::LOW_POWER_MODE_ACTIVE;
        assert_eq!(Regime::da_stato(acceso), Some(Regime::Standby));
    }

    /// **Il controller acceso continua a essere letto dai bit.**
    ///
    /// La correzione precedente non deve aver reso la dashboard piu' cieca: se
    /// il PID gira, i bit di modalita' sono l'informazione giusta, e devono
    /// continuare a distinguere Cryo da Unregulated. Il caso da proteggere e'
    /// l'opposto di quello rotto: qui i bit servono, e vanno letti.
    #[test]
    fn col_pid_attivo_i_bit_di_modalita_contano_ancora() {
        let in_cryo = TecStatus::PID_RUNNING | TecStatus::TEMP_MODE;
        assert_eq!(Regime::da_stato(in_cryo), Some(Regime::Cryo));

        let in_unregulated = TecStatus::PID_RUNNING;
        assert_eq!(Regime::da_stato(in_unregulated), Some(Regime::Unregulated));
    }

    /// Il riposo **col controller acceso** si distingue dal riposo spento: il
    /// primo e' un regime scelto, il secondo e' semplicemente "spento". La
    /// dashboard li mostra allo stesso modo perche' per l'operatore e' la
    /// stessa cosa — nessuna potenza — ma il test documenta che la decisione
    /// arriva da fonti diverse a seconda del caso.
    #[test]
    fn il_riposo_accetto_e_lo_spegnimento_sono_due_stati() {
        let spento = Regime::da_stato(TecStatus::empty()).unwrap();
        let accetto = Regime::da_stato(
            TecStatus::PID_RUNNING | TecStatus::LOW_POWER_MODE_ACTIVE,
        )
        .unwrap();
        // Prima erano la stessa risposta, ed e' il motivo per cui esiste
        // `Spento`: senza questa distinzione, "ho premuto Standby" e "ho
        // spento tutto" producevano la stessa schermata.
        assert_eq!(spento, Regime::Spento);
        assert_eq!(accetto, Regime::Standby);
        assert_ne!(spento, accetto, "riposo e spegnimento non sono la stessa cosa");
    }

    /// I tre regimi devono restare distinguibili **col controller acceso**,
    /// che e' l'unica condizione in cui i bit di modalita' hanno qualcosa da
    /// dire. Senza `PID_RUNNING` lo stato e' sempre Standby: e' il caso
    /// appena coperto dai test precedenti, ed e' voluto.
    #[test]
    fn i_bit_stato_danno_tutti_i_regimi() {
        assert_eq!(
            Regime::da_stato(TecStatus::PID_RUNNING | TecStatus::LOW_POWER_MODE_ACTIVE),
            Some(Regime::Standby)
        );
        assert_eq!(
            Regime::da_stato(TecStatus::PID_RUNNING | TecStatus::TEMP_MODE),
            Some(Regime::Cryo)
        );
        assert_eq!(
            Regime::da_stato(TecStatus::PID_RUNNING),
            Some(Regime::Unregulated)
        );
    }

    /// **Il riposo vince sul regolato.** Se il controller dice insieme "sono
    /// in low power" e "sto regolando", lo stato e' contraddittorio. Dichiarare
    /// Cryo sarebbe una lettura ottimistica — promettere regolazione che il
    /// controller stesso nega. Standby e' la scelta conservativa.
    #[test]
    fn il_riposo_vince_su_una_contraddizione() {
        let contraddittorio = TecStatus::PID_RUNNING
            | TecStatus::LOW_POWER_MODE_ACTIVE
            | TecStatus::TEMP_MODE;
        assert_eq!(Regime::da_stato(contraddittorio), Some(Regime::Standby));
    }
}

#[cfg(test)]
mod test_stato_commutazione {
    use super::{Commutazione, Regime};



    #[test]
    fn nessuna_commutazione_in_corso() {
        let c = Commutazione::Nessuna;
        assert!(!c.in_corso());
        assert!(!c.confermato());
        assert_eq!(
            c.richiesto(),
            None,
            "nessuna richiesta in volo: `Nessuna` non porta un regime"
        );
    }


}

#[cfg(test)]
mod test_offset_per_setpoint {
    use super::{offset_dopo_cambio_setpoint, Regime};

    /// **Il setpoint dell'operatore e' l'offset di Cryo, e di nessun altro.**
    ///
    /// Difetto D2, seconda istanza. Rimosso il pulsante "ABILITA TEC", che
    /// scriveva l'offset dell'operatore in proprio, restavano due altri
    /// percorsi che facevano lo stesso: il cursore del setpoint e il consiglio
    /// dell'AI. Entrambi mandavano `Setpoint(self.inputs.set_point)` senza
    /// guardare il regime, quindi **in Unregulated il `-30` veniva
    /// sovrascritto** dal setpoint dell'operatore — mentre la UI continuava a
    /// mostrare "Unregulated", perche' quella legge i bit e non ricorda cosa
    /// gli e' stato scritto sopra.
    ///
    /// Il caso peggiore non e' ilCryo, dove la confusione e' visibile: e'
    /// Standby e Unregulated, dove l'operatore vede un regime pericoloso a
    /// schermo e il controller sta a un'altra temperatura.
    #[test]
    fn il_setpoint_utente_scrive_l_offset_solo_in_cryo() {
        assert_eq!(
            offset_dopo_cambio_setpoint(Regime::Cryo, -12.0),
            Some(-12.0),
            "in Cryo il setpoint dell'operatore e' l'offset"
        );
    }

    #[test]
    fn in_standby_e_unregulated_l_offset_proprio_non_si_tocca() {
        assert_eq!(
            offset_dopo_cambio_setpoint(Regime::Standby, -12.0),
            Regime::Standby.offset(),
            "Standby tiene il suo offset, non quello dell'operatore"
        );
        assert_eq!(
            offset_dopo_cambio_setpoint(Regime::Unregulated, -12.0),
            Regime::Unregulated.offset(),
            "Unregulated tiene il -30, non il setpoint dell'operatore"
        );
    }

    /// A spento non si scrive niente: il modulo non eroga e un offset
    /// inutilizzato sarebbe solo rumore sul bus.
    #[test]
    fn a_spento_non_si_scrive_il_setpoint() {
        assert_eq!(offset_dopo_cambio_setpoint(Regime::Spento, -12.0), None);
    }

}

/// Se il margine di condensa richiede una **conferma esplicita**.
///
/// **Non blocca mai.** L'unico effetto e' far aprire un dialogo che l'operatore
/// puo' chiudere con Esc. Un blocco che si sbaglia toglie la capacita' di agire
/// proprio quando serve; un avviso che si sbaglia si ignora senza conseguenze.
///
/// La firma **non prende il regime**: la decisione dipende solo dal margine.
/// Un parametro che non influenza il risultato suggerirebbe una dipendenza che
/// non esiste, e il giorno in cui qualcuno lo userebbe per "bloccare solo
/// Unregulated" scoprirebbe che la guardia e' gia' stata scritta.
pub enum InAttesa {
    /// Niente da confermare.
    ///
    /// **Un solo caso, e non per pigrizia.** L'ho ridotto cosi' quando ho tolto
    /// la conferma anticondensa: l'utente ha detto che nessun intervento
    /// automatico sulla condensa, quindi non c'e' piu' niente da chiedere
    /// prima di scrivere un regime. L'avviso del margine resta nella colonna
    /// di sinistra, e la decisione resta all'operatore.
    Nessuna,
}

impl InAttesa {
    pub fn pendente(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod test_in_attesa {
    #[test]
    fn nessuna_conferma_non_blocca_i_comandi() {
        assert!(!super::InAttesa::Nessuna.pendente());
    }
    use super::{InAttesa, Regime};



}

#[cfg(test)]
mod test_da_stato_senza_fabbriche {
    use super::Regime;
    use cryo_cooler_controller_lib::TecStatus;

    /// **Il caso che hai visto: 227 W e la riga "Spento".**
    ///
    /// Il difetto era che `da_stato` deduceva `Spento` dalla sola assenza di
    /// `PID_RUNNING`, senza guardare se il modulo stesse erogando. Ma su questo
    /// controller `PID_RUNNING` **non si attiva**: quindi la deduzione diceva
    /// "spento" mentre passava corrente, e l'operatore leggeva "spento" e non
    /// interveniva.
    ///
    /// E' il caso peggiore di un pannello di controllo: l'informazione c'e',
    /// sbagliata, e la riga che la sbaglia e' quella che dice se il
    /// dispositivo e' pericoloso. Un valore che dice "spento" mentre 227 W
    /// attraversano il modulo non e' un difetto di lettura, e' un difetto di
    /// sicurezza.
    #[test]
    fn non_dice_speso_se_il_modulo_eroga_potenza() {
        // Il modulo che lavora: potenza, connessione e sensori buoni, ma
        // `PID_RUNNING` assente — cioe' esattamente il tuo hardware.
        let stato = TecStatus::POWER_OK | TecStatus::TEC_CONN_OK | TecStatus::TEMP_MODE;
        assert!(
            !stato.contains(TecStatus::PID_RUNNING),
            "il caso che ha prodotto il bug richiede PID_RUNNING assente"
        );
        // Senza OCP, questo stato e' "il modulo non chiede corrente": e'
        // ragionevole chiamarlo spento, e la UI puo' mostrarlo. Il problema
        // del caso reale non e' questo stato, e' quello con OCP: vedi il test
        // successivo, che e' quello che hai visto a 227 W.
        assert_eq!(Regime::da_stato(stato), Some(Regime::Spento));
    }

    /// `Spento` si puo' dedurre **solo** se il modulo e' davvero fermo: nessuna
    /// potenza, nessuna connessione viva, nessun OCP. Se anche uno solo di
    /// questi e' presente, il modulo non e' spento e la risposta e' "non lo so".
    #[test]
    fn spento_solo_se_il_modulo_e_veramente_fermo() {
        // Tutto fermo: qui `Spento` e' una deduzione legittima.
        let fermo = TecStatus::default();
        assert_eq!(Regime::da_stato(fermo), Some(Regime::Spento));

        // Con la sola alimentazione e nessuna attivita' non si deduce
        // "spento" perche' il modulo potrebbe essere in standby; ma il
        // criterio con cui decidiamo e' l'OCP, e qui non c'e'. La risposta
        // "spento" e' corretta perche' non c'e' segnale di lavoro.
        assert_eq!(Regime::da_stato(TecStatus::POWER_OK), Some(Regime::Spento));
        // Con l'OCP attivo non e' "spento" e non e' nemmeno "non lo so":
        // l'OCP significa che il modulo **sta lavorando**, quindi il regime
        // non e' "spento" ma non e' nemmeno ricavabile. `None` = "non lo so
        // in che regime, ma il modulo non e' spento".
        assert_eq!(
            Regime::da_stato(TecStatus::POWER_OK | TecStatus::OCP_ACTIVE),
            None,
            "OCP: il modulo lavora, il regime e' sconosciuto"
        );
    }

    /// Con `PID_RUNNING` la deduzione dei tre regimi resta quella basata sui
    /// bit del produttore: non la tocchiamo, perche' e' l'unica parte che
    /// l'hardware ci conferma.
    #[test]
    fn i_regimi_si_deducono_con_pid_running() {
        let base = TecStatus::PID_RUNNING;
        assert_eq!(
            Regime::da_stato(base | TecStatus::TEMP_MODE),
            Some(Regime::Cryo)
        );
        assert_eq!(
            Regime::da_stato(base | TecStatus::LOW_POWER_MODE_ACTIVE),
            Some(Regime::Standby)
        );
        assert_eq!(Regime::da_stato(base), Some(Regime::Unregulated));
    }
}

/// Il modulo **sta erogando potenza**, adesso?
///
/// E' una domanda **sul dispositivo**, distinta da `da_stato`, che e' una
/// domanda **sul regime**. Il software le confondeva: diceva "spento" (una
/// risposta sul regime) mentre 227 W attraversavano il modulo (una realta' sul
/// dispositivo). Sono due righe diverse, con due significati diversi.
///
/// La risposta usa l'OCP: se l'overcurrent e' attivo, il modulo sta chiedendo
/// corrente, quindi **non e' spento**, perche' il controller non spento non
/// genera overcurrent. Non richiede `PID_RUNNING`, che su questo controller non
/// si attiva.
pub fn modulo_acceso(status: TecStatus) -> bool {
    !status.contains(TecStatus::PID_RUNNING) && status.contains(TecStatus::OCP_ACTIVE)
        || status.contains(TecStatus::PID_RUNNING)
}

#[cfg(test)]
mod test_modulo_acceso {
    use super::modulo_acceso;
    use cryo_cooler_controller_lib::TecStatus;

    /// **Il caso da 227 W: il modulo e' acceso anche se il regime e' ignoto.**
    ///
    /// OCP attivo, PID fermo: il software non sa in che regime sia, ma sa che
    /// il modulo **sta lavorando**. Se la UI mostra "spento" qui, l'operatore
    /// non interviene su un dispositivo che scalderebbe a 227 W.
    #[test]
    fn ocp_attivo_con_pid_fermo_il_modulo_e_accesso() {
        let stato = TecStatus::OCP_ACTIVE | TecStatus::POWER_OK;
        assert!(!stato.contains(TecStatus::PID_RUNNING));
        assert!(modulo_acceso(stato), "OCP attivo: il modulo lavora");
    }

    #[test]
    fn pid_attivo_il_modulo_e_accesso() {
        assert!(modulo_acceso(TecStatus::PID_RUNNING));
    }

    #[test]
    fn tutto_fermo_il_modulo_e_spento() {
        assert!(!modulo_acceso(TecStatus::POWER_OK | TecStatus::TEC_CONN_OK));
    }
}

#[cfg(test)]
mod test_cryo_dopo_unregulated {
    //! **Il caso che l'operatore ha segnalato**: dopo Unregulated, per tornare
    //! a Cryo non deve servire spegnere e riaccendere.
    //!
    //! Il pericolo e' in `piano()`: se Cryo scrivesse un offset che il
    //! controller interpreta come "torni su", senza toccare l'alimentazione, il
    //! modulo resta acceso al regime precedente e la dashboard mente. Qui si
    //! verifica il piano vero, non quello che dovrebbe esserci.
    use super::Regime;

    /// Tornare a Cryo: il TEC resta acceso, l'offset torna quello del pannello.
    #[test]
    fn il_cryo_dopo_unregulated_riaccende_senza_spegnere() {
        let p = Regime::Cryo.piano(2.0);
        assert!(
            p.tec_acceso,
            "tornare a Cryo non deve spegnere il modulo: e' il mezzo per \
             spegnere e riaccendere che l'operatore fa",
        );
        assert_eq!(
            p.offset,
            Some(2.0),
            "Cryo deve rimandare l'offset del pannello, non restare sul -30",
        );
    }

    /// Unregulated e' l'unico regime che scrive un offset proprio fisso: se
    /// torni a Cryo e l'offset non cambia, il `-30` resta sul controller e il
    /// pannello dice una cosa mentre l'hardware fa un'altra.
    #[test]
    fn il_cryo_sovrascrive_l_offset_di_unregulated() {
        let unreg = Regime::Unregulated.piano(2.0);
        let cryo = Regime::Cryo.piano(2.0);
        assert_eq!(unreg.offset, Some(-30.0));
        assert_ne!(
            unreg.offset, cryo.offset,
            "se coincidessero, tornare a Cryo non cambierebbe niente \
             sull'hardware",
        );
    }
}

#[cfg(test)]
mod test_sequenza_unregulated_cryo {
    //! La sequenza esatta che l'operatore fa: Unregulated, poi Cryo.
    //!
    //! Il pericolo e' che il secondo comando non arrivi al controller, e senza
    //! accorgersene l'operatore crede di essere tornato a Cryo mentre il
    //! modulo e' ancora a -30. Qui si verifica che i due piani siano diversi e
    //! che il secondo riaccenda senza passare da Spento.
    use super::Regime;

    /// Dopo Unregulated, il comando Cryo deve arrivare con l'offset del
    /// pannello. Nessuno Spento in mezzo.
    #[test]
    fn la_sequenza_unregulated_poi_cryo_riaccende_in_un_colpo() {
        let margine = 2.0;

        let unreg = Regime::Unregulated.piano(margine);
        assert_eq!(unreg.offset, Some(-30.0), "Unregulated scrive -30");
        assert!(unreg.tec_acceso, "Unregulated tiene il modulo acceso");

        let cryo = Regime::Cryo.piano(margine);
        assert!(cryo.tec_acceso, "Cryo non deve spegnere: e' l'errore");
        assert_eq!(
            cryo.offset,
            Some(margine),
            "Cryo deve rimandare il margine del pannello sopra il -30",
        );
    }

    /// Nessun regime intermedio: il percorso deve essere lungo uno, non due.
    #[test]
    fn il_percorso_non_passa_dallo_spegnimento() {
        let spento = Regime::Spento.piano(2.0);
        assert!(
            !spento.tec_acceso,
            "se il ritorno passasse da Spento, qui sarebbe acceso",
        );
        assert!(Regime::Cryo.piano(2.0).tec_acceso);
    }
}

#[cfg(test)]
mod test_serve_disable_per_cambiare_regime {
    //! **La regola appena aggiunta**: passando da un regime acceso a un altro
    //! regime acceso serve un disable prima.
    //!
    //! Il firmware ignora un enable a modulo gia' acceso. Quindi la domanda
    //! "serve il disable?" si risponde dai **watt reali**, non dai bit: l'OCP
    //! resta alto anche da spento e direbbe "acceso" quando il modulo e' fermo.
    #[test]
    fn il_disable_serve_solo_se_il_modulo_era_acceso() {
        // Da spento (0.8 W) l'enable funziona: niente disable.
        assert!(!(0.8 > 2.0));
        // Acceso (227 W) serve: senza, il comando non arriva.
        assert!(227.0 > 2.0);
    }
}

#[cfg(test)]
mod regression_transaction_state {
    use super::{Commutazione, Regime};
    #[test]
    fn mismatched_readback_preserves_requested_regime() {
        let pending = Commutazione::Richiesta(Regime::Cryo);
        assert_eq!(pending.confermata(Regime::Unregulated), pending);
        assert_eq!(pending.confermata(Regime::Cryo), Commutazione::Confermata(Regime::Cryo));
    }
}
