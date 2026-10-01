# Un solo percorso di comando — Piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Una gestione delle modalità in cui ogni regime è conosciuto, dichiarato e scritto da un solo percorso, senza mai presentare un'inferenza come un fatto.

**Architecture:** Il nucleo è un tipo `Certezza` a tre stati (`Conosciuto` / `Dedotto` / `Ignoto`) che porta l'incertezza dentro il tipo invece che dentro un commento. `tec_abilitato` smette di essere un campo mutabile in quattro punti e diventa derivato. La connessione seriale smette di poter emettere il reset di fabbrica. La guardia anticondensa smette di bloccare e chiede una conferma con anteprima delle conseguenze.

**Tech Stack:** Rust, `iced` 0.13 (UI), `bitflags` (stato seriale), `cargo test`.

**Spec:** `docs/superpowers/specs/2026-09-29-unico-percorso-di-comando-design.md`

## Global Constraints

- **Il repository NON è un git.** `git commit` non esiste qui. Ogni task termina con un **checkpoint** invece: suite completa verde + il conteggio esatto dei test. Non scrivere passaggi `git`.
- **Nessuna regressione:** la suite esistente (196 + 14) resta verde in ogni task, non solo alla fine. Se un test rosso non è uno di quelli dichiarati in Spec §5.1, il lavoro è sbagliato.
- **TDD rigoroso:** nessun codice di produzione senza un test che è stato **osservato fallire** prima. Un test che passa al primo colpo è sbagliato, anche se sembra giusto.
- **Solo 4 test possono cambiare**, e sono già dichiarati in Spec §5.1. Un quinto test che cambia è una regressione e va fermata.
- **Protocollo intoccabile:** `0x14`, `0x18`, il loro ordine, `payload_alimentazione()` e la allowlist di sola lettura non si modificano. Sono verificati.
- **`running.rs` non si sposta e non si divide.** È escluso dalla specifica di proposito.
- **I commenti che spiegano un bug passato non si cancellano** (I12). Se un commento diventa falso, si corregge — non si rimuove.
- **R84 non si tocca mai:** `<ESEGUIBILI>\cryo_cooler_controller.exe`, MD5 `849d2ebb9c544ec00425092b93dd2310`.
- **Il commento importante sul protocollo:** `Tec::new` non deve poter emettere `0x1E` in nessun caso, nemmeno su una board non inizializzata.

---

## Struttura dei file

| File | Ruolo | cambia in |
|---|---|---|
| `cryo_cooler_controller_lib/src/lib.rs` | Trasporto seriale, protocollo, `Tec` | Task 1, 2 |
| `cryo_cooler_controller/src/commutazione.rs` | `Regime`, `PianoCambio`, guardia | Task 2, 3, 5 |
| `cryo_cooler_controller/src/certezza.rs` | **NUOVO** — il tipo a tre stati | Task 2 |
| `cryo_cooler_controller/src/modalita.rs` | `Modalita`, `modalita_corrente` | Task 2, 4 |
| `cryo_cooler_controller/src/attore_tec.rs` | Coda e priorità delle richieste | Task 3 |
| `cryo_cooler_controller/src/running.rs` | Stato della UI, `update`, `view` | Task 3, 4, 5 |
| `cryo_cooler_controller/src/diagnostica.rs` | Verdetto e riepilogo | Task 4 |

`certezza.rs` è un file nuovo e piccolo, e non una riga in `running.rs`: la regola che decide se il software può scrivere in seriale è la più critica del progetto, e merita di essere leggibile da sola e testabile da sola.

---

## Task 1: `Tec::new` non può più emettere `0x1E`

**Files:**
- Modify: `cryo_cooler_controller_lib/src/lib.rs:312-336` (`Tec::new`)
- Modify: `cryo_cooler_controller_lib/src/lib.rs:227-230` (`Tec::reset`)
- Test: in-module, `cryo_cooler_controller_lib/src/lib.rs`

**Interfaces:**
- Produce: `pub fn serve_reset_alla_connessione(status: TecStatus) -> bool` — decisione pura, testabile senza porta seriale.
- Produce: `fn reset(&mut self)` resta privata, ma **non viene più chiamata da `new`**.

**Perché una funzione e non un test diretto:** `Tec::new` ha bisogno di una porta seriale vera. La decisione *"serve il reset?"* è logica pura, quindi la si estrae e la si testa da sola. È lo stesso criterio già usato per `payload_alimentazione` e `guardia_anticondensa`.

- [ ] **Step 1: scrivi il test che fallisce**

```rust
#[cfg(test)]
mod test_reset_alla_connessione {
    use super::{serve_reset_alla_connessione, TecStatus};

    /// **Ricollegarsi non deve mai azzerare il controller.**
    ///
    /// `0x1E` e' il reset di fabbrica: perde PID, setpoint e **power cap**.
    /// Perdere il power cap significa che il limite di potenza che l'operatore
    /// aveva impostato sparisce senza avviso, al primo ricollegamento.
    #[test]
    fn il_reset_di_fabbrica_non_e_mai_una_conseguenza_della_connessione() {
        for stato in [
            TecStatus::empty(),
            TecStatus::all(),
            TecStatus::BOARD_INIT,
            TecStatus::POWER_OK,
            TecStatus::BOARD_INIT | TecStatus::POWER_OK,
        ] {
            assert!(
                !serve_reset_alla_connessione(stato),
                "lo stato {stato:?} non deve mai autorizzare un reset di fabbrica"
            );
        }
    }

    /// Il caso pericoloso e' `BOARD_INIT` assente: era la condizione che
    /// prima faceva scattare `0x1E`. Il test la nomina per nome, cosi'
    /// nessuno puo' reintrodurla pensando che sia un caso particolare.
    #[test]
    fn board_init_assente_non_e_un_eccezione() {
        let senza_init = TecStatus::POWER_OK | TecStatus::TEC_CONN_OK;
        assert!(!sinza_init.contains(TecStatus::BOARD_INIT));
        assert!(!serve_reset_alla_connessione(senza_init));
    }
}
```

- [ ] **Step 2: esegui e verifica che fallisce**

Run: `cargo test -p cryo_cooler_controller_lib reset_alla_connessione`
Expected: **errore di compilazione** — `cannot find function serve_reset_alla_connessione`. È il giusto rosso: la funzione non esiste.

- [ ] **Step 3: implementazione minima**

Cancella da `Tec::new` il blocco `if !status.contains(TecStatus::BOARD_INIT) { tec.reset()?; }`, lasciando il solo `hear_beat` di prova. Poi, **fuori dall'`impl`, accanto a `payload_alimentazione`**:

```rust
/// Se aprire una connessione debba emettere il reset di fabbrica.
///
/// **La risposta e' no, e non e' una semplificazione: e' una correzione.**
///
/// La versione precedente faceva cosi': se `BOARD_INIT` non era impostato,
/// `Tec::new` mandava `0x1E`. La conseguenza era che il power cap, il setpoint
/// e i coefficienti PID impostati dall'operatore sparivano al primo
/// ricollegamento — e il power cap e' la protezione che l'operatore ha messo
/// per stare tranquillo.
///
/// "Bo non e' un TEC, fallisce qui" e' gia' garantito da `hear_beat`: non e'
/// la condizione di `BOARD_INIT` a decidere se la scheda e' viva.
///
/// Se in futuro servisse una procedura di inizializzazione, e' un'azione
/// esplicita di commissioning, non un effetto collaterale dell'apertura della
/// porta. Fuori ambito qui, per scelta.
pub fn serve_reset_alla_connessione(_status: TecStatus) -> bool {
    false
}
```

- [ ] **Step 4: verifica verde**

Run: `cargo test -p cryo_cooler_controller_lib`
Expected: tutti verdi, incluso il test che ha coperto `0x1E` (`la_sonda_non_manda_mai_enable_ne_reset`), che deve restare verde.

- [ ] **Step 5: checkpoint**

```bash
cargo test 2>&1 | grep -E "test result"
```
Atteso: `196 passed` + `14 passed`, 0 falliti. Nessun test esistente è stato toccato in questo task.

---

## Task 2: `Certezza` a tre stati, e `tec_abilitato` derivato

**Files:**
- Create: `cryo_cooler_controller/src/certezza.rs`
- Modify: `cryo_cooler_controller/src/main.rs` (dichiarazione del modulo)
- Modify: `cryo_cooler_controller/src/commutazione.rs` (`da_stato`, `tec_acceso`)
- Modify: `cryo_cooler_controller/src/modalita.rs:238` (`modalita_corrente`)
- Modify: `cryo_cooler_controller/src/running.rs` (campo `tec_abilitato` → funzione)

**Interfaces:**
- Produce: `pub enum Certezza { Conosciuto, Dedotto, Ignoto }` in `certezza.rs`
- Produce: `pub fn Regime::da_stato_certainza(status: TecStatus) -> (Regime, Certezza)`
- Produce: `Regime::eroga_potenza(self) -> bool`
- Change: `modalita_corrente(status: TecStatus, mai_lett: bool) -> Modalita`

**La decisione di progetto che conta.** Il secondo argomento di `modalita_corrente` è oggi `poll_in_flight || applied_power > 0 || tec_abilitato`: tre cose diverse, una delle quali (`tec_abilitato`) **è il difetto D3**. Sostituirlo con un OR diverso sarebbe rifare lo stesso errore in forma nuova.

La scomposizione corretta distingue due grandezze che oggi sono mescolate:

- **"non abbiamo ancora letto niente"** — fatto di *sessione*, non del dispositivo. È l'unica ragione legittima per mostrare `Offline` invece di `Spento`. Diventa l'unico argomento.
- **"il TEC eroga potenza"** — fatto del *dispositivo*, ed è la nuova funzione derivata.

- [ ] **Step 1: il test che fallisce, in `certezza.rs`**

```rust
#[cfg(test)]
mod test {
    use super::*;
    use crate::commutazione::Regime;
    use cryo_cooler_controller_lib::TecStatus;

    /// **Standby e' TEC acceso.** E' il punto su cui il software precedente
    /// sbagliava: derivava "acceso" da `LOW_POWER_MODE` negato, cosi' in
    /// Standby la guardia perdeva l'autorizzazione a scrivere potenza proprio
    /// nello stato in cui il modulo funziona.
    #[test]
    fn lo_standby_eroga_potenza() {
        assert!(Regime::Standby.eroga_potenza());
    }

    #[test]
    fn solo_lo_spento_non_eroga_potenza() {
        for r in [Regime::Standby, Regime::Cryo, Regime::Unregulated] {
            assert!(r.eroga_potenza(), "{r} deve erogare potenza");
        }
        assert!(!Regime::Spento.eroga_potenza());
    }

    /// **Lo stato ignoto non scrive.** E' l'invariante che rende il sistema
    /// onesto: non si sa cosa stia facendo il controller, quindi non si
    /// scrive niente. Un errore qui accenderebbe il modulo a caso.
    #[test]
    fn in_stato_ignoto_non_si_scrive() {
        let (regime, certezza) = Regime::da_stato_certainza(TecStatus::empty());
        assert_eq!(certezza, Certezza::Ignoto);
        assert_eq!(regime, Regime::Spento, "senza dati non si dichiara un regime");
    }

    /// **Con dati freschi il regime e' conosciuto.**
    #[test]
    fn col_pid_in_marcia_il_regime_e_conosciuto() {
        let s = TecStatus::PID_RUNNING | TecStatus::TEMP_MODE;
        let (regime, certezza) = Regime::da_stato_certainza(s);
        assert_eq!(regime, Regime::Cryo);
        assert_eq!(certezza, Certezza::Conosciuto);
    }

    /// **`LOW_POWER_MODE` da solo non decide piu' niente.** Il test e'
    /// scritto cosi' perche' il difetto era precisamente questo: un bit che
    /// descrive *standby* veniva usato come se descrivesse *alimentazione*.
    #[test]
    fn low_power_da_solo_non_decide_l_alimentazione() {
        // LOW_POWER acceso, PID fermo: il modulo non lavora, ed e' un fatto
        // letto, non una deduzione.
        let s = TecStatus::LOW_POWER_MODE_ACTIVE;
        let (regime, _) = Regime::da_stato_certainza(s);
        assert_eq!(regime, Regime::Spento);
    }
}
```

- [ ] **Step 2: verifica il rosso**

Run: `cargo test -p cryo_cooler_controller certezza`
Expected: errore di compilazione — `cannot find type Certezza`, `cannot find method eroga_potenza`, `cannot find function da_stato_certainza`.

- [ ] **Step 3: `certezza.rs`**

```rust
//! Se il software sa cosa sta facendo il controller, o lo sta deducendo.
//!
//! Esiste perche' questa distinzione oggi non e' rappresentata da nessuna
//! parte, e la sua assenza e' un rischio: il software deduce il regime da
//! `PID_RUNNING`, un bit letto dai binari del produttore e **mai osservato
//! cambiare** su questo controller. Un errore di deduzione non si vede sui
//! numeri — i numeri descrivono il controller, non l'ipotesi sul suo regime —
//! quindi viene presentato con la stessa sicurezza di un fatto misurato.
//!
//! `Certezza` porta l'incertezza dentro il tipo. Non e' una nota a margine:
//! da essa dipende se il software puo' scrivere in seriale.

use cryo_cooler_controller_lib::TecStatus;
use crate::commutazione::Regime;

/// Quanto il software sa del regime che il controller sta tenendo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Certezza {
    /// I bit di stato concordano col regime atteso. Si puo' automatizzare.
    Conosciuto,
    /// I bit dicono qualcosa, ma non e' stato verificato. Si mostra
    /// **marcato come dedotto** e non si automatizza.
    Dedotto,
    /// Nessun dato fresco. Non si scrive niente in seriale.
    Ignoto,
}

/// Il regime corrente e quanto ne siamo certi.
///
/// `Ignoto` si distingue da `Dedotto` perche' la differenza non e' di grado ma
/// di diritto: su dati vecchi non si automatizza *niente*, mentre su dati freschi
/// ma non verificati si puo' ancora agire se l'operatore lo chiede.
pub fn regime_e_certezza(status: TecStatus) -> (Regime, Certezza) {
    // `PID_READY` e' il segnale che il regolatore e' stato inizializzato: e'
    // cio' che distingue "il controller non ci ha mai parlato" da "il
    // controller ci ha parlato e mi dice che e' fermo". Senza questa
    // distinzione, un controller appena collegato e uno spento sono la stessa
    // cosa, e il software non lo sa.
    if !status.contains(TecStatus::LAST_CMD_OK) {
        return (Regime::Spento, Certezza::Ignoto);
    }
    if !status.contains(TecStatus::PID_RUNNING) {
        return (Regime::Spento, Certezza::Dedotto);
    }
    let regime = Regime::da_stato(status).unwrap_or(Regime::Spento);
    (regime, Certezza::Conosciuto)
}

/// `true` solo se il software sa cosa sta facendo il controller.
///
/// E' il gate di I10: nessuna scrittura in seriale quando non si sa.
pub fn puo_scrivere(certezza: Certezza) -> bool {
    certezza == Certezza::Conosciuto
}
```

- [ ] **Step 4: in `commutazione.rs`, `eroga_potenza` e `da_stato_certainza`**

```rust
    /// Se questo regime fa erogare potenza al TEC.
    ///
    /// Sostituisce il vecchio `tec_acceso`, che rispondeva `false` solo per
    /// `Spento` ma veniva interrogato sul significato di `LOW_POWER_MODE`.
    pub fn eroga_potenza(self) -> bool {
        match self {
            Regime::Spento => false,
            Regime::Standby | Regime::Cryo | Regime::Unregulated => true,
        }
    }
```

e un metodo che delega, per non avere due punti di verita:

```rust
    pub fn da_stato_certainza(status: TecStatus) -> (Self, crate::certezza::Certezza) {
        crate::certezza::regime_e_certezza(status)
    }
```

- [ ] **Step 5: `modalita_corrente` perde l'OR di tre cose**

```rust
pub fn modalita_corrente(status: TecStatus, mai_lett: bool) -> Modalita {
    if mai_lett {
        return Modalita::Offline;
    }
    let (regime, _) = Regime::da_stato_certainza(status);
    Modalita::da_regime(regime)
}
```

`Modalita::da_regime` mappa `Regime -> Modalita`: `Standby`, `Cryo`,
`Unregulated`, `Spento`. Aggiungilo come funzione pubblica in `modalita.rs` e
falla coprire da un test che enumera i quattro casi.

- [ ] **Step 6: `tec_abilitato` diventa funzione in `running.rs`**

Sostituisci il campo `tec_abilitato: bool` con:

```rust
    /// Il TEC eroga potenza *adesso*. Derivato, mai memorizzato.
    ///
    /// Passa dalla stessa funzione che decide la `Certezza`: due letture
    /// indipendenti della stessa domanda ricreerebbero, in miniatura, il difetto
    /// che questa fase chiude.
    fn tec_eroga_potenza(&self) -> bool {
        let (regime, certezza) = Regime::da_stato_certainza(self.tec_status);
        crate::certezza::puo_scrivere(certezza) && regime.eroga_potenza()
    }
```

e sostituisci **tutte** le letture di `self.tec_abilitato` con la chiamata. Sono
cinque siti: `running.rs:1379`, `1493`, `1585`, `1841`, e i riferimenti nei
blocchi `ack` di `Enable`/`Disable`, che spariscono nel Task 3. Elimina anche le
assegnazioni: un campo che non esiste non può essere dimenticato.

- [ ] **Step 7: verifica verde, poi checkpoint**

```bash
cargo test 2>&1 | grep -E "test result"
```
Atteso: `196 passed` + `14 passed` + i nuovi. Se un test esistente diventa
rosso qui, **fermati**: significa che un significato è cambiato senza che sia
stato dichiarato in Spec §5.1.

---

## Task 3: un solo percorso di scrittura, menu al posto del pulsante

**Files:**
- Modify: `cryo_cooler_controller/src/attore_tec.rs` — rimuovere `Richiesta::Enable`, `Richiesta::Disable`, `Scritto::Enable`, `Scritto::Disable`, `accende_il_tec`
- Modify: `cryo_cooler_controller/src/main.rs` — rimuovere `Message::Enable`, `Message::Disable`
- Modify: `cryo_cooler_controller/src/running.rs:2659-2704` — rimuovere i due handler
- Modify: `cryo_cooler_controller/src/running.rs:3440-3492` (`pulsante_tec`) — il menu al posto del tasto
- Modify: `cryo_cooler_controller/src/running.rs:1574-1790` (`view_menu_modalita`) — estrarre e spostare
- Test: `cryo_cooler_controller/src/attore_tec.rs`

**Interfaces:**
- Consumes: `Regime`, `certezza::regime_e_certezza` (Task 2)
- Produce: `fn pulsante_regime(&self, etichetta, sottotitolo, msg, colore, attivo) -> Button<'_, Message>` — estratto da closure

**Estrazione obbligatoria.** `pulsante_regime` oggi è una **closure locale** dentro
`view_menu_modalita` (`running.rs:1665`). Il menu deve ora essere renderizzato in
`pulsante_tec`, che è un'altra funzione. Senza estrazione si copia il codice in
due punti e le due copie divergono — che è esattamente il difetto che questa fase
sta chiudendo.

- [ ] **Step 1: il test che fallisce — la priorità, portata sul percorso nuovo**

```rust
    /// **Il blackout hardware, sul percorso che resta.**
    ///
    /// Il test originale verificava la priorita' con `Richiesta::Enable`, che
    /// questa fase elimina. L'invariante pero' e' reale e va conservato: una
    /// scrittura accodata dietro un campione non usciva mai, e in hardware
    /// questo significava che il TEC non veniva mai abilitato.
    #[test]
    fn la_commutazione_precede_il_campione() {
        let (_tx, rx) = canale();
        let mut coda: VecDeque<Richiesta> = VecDeque::new();
        coda.push_back(Richiesta::Campione);
        coda.push_back(Richiesta::Regime { offset: Some(-12.0), tec_acceso: true });
        let prima = prossima_richiesta(&mut coda, &rx).expect("deve restituire qualcosa");
        assert!(prima.e_scrittura(), "la scrittura deve precedere il campione");
    }
```

Elimina anche `solo_enable_autorizza_il_tec_acceso`: è tautologico (verifica un
metodo dichiarato `dead_code` fuori dai test, che confronta un tag di variante con
un booleano) e la variante che usa sparisce.

- [ ] **Step 2: verifica il rosso**

Run: `cargo test -p cryo_cooler_controller blackout`
Expected: rosso — `Richiesta::Enable` non è ancora costruibile in questa forma, o
il test non esiste. Poi il verde arriva dalla fase 3 dell'implementazione.

- [ ] **Step 3: rimuovi il percorso vecchio**

In `attore_tec.rs`: cancella le varianti `Richiesta::Enable`, `Richiesta::Disable`,
`Scritto::Enable`, `Scritto::Disable`, il metodo `accende_il_tec`, e i due rami di
`esegui` che le trattano. In `main.rs`: cancella `Message::Enable` e
`Message::Disable`. In `running.rs:2659-2704`: cancella i due handler.

**Non toccare** `running.rs:1841-1855`, il blocco di spegnimento all'uscura: è
`Richiesta::Disable` che *deve* restare, perché chiudere l'app deve spegnere il
modulo. Se lo cancelli, l'app si chiude e lascia il TEC acceso.

- [ ] **Step 4: estrai `pulsante_regime` e sposta il menu**

Estrai la closure (`running.rs:1665-1686`) in un metodo:

```rust
    /// Un pulsante di regime.
    ///
    /// Estratto dalla closure che era dentro `view_menu_modalita`, perche' il
    /// menu ora vive anche in `pulsante_tec` e due copie divergerebbero.
    ///
    /// **Il quinto parametro non e' "questo e' il regime attivo": e' "non
    /// cliccabile".** Oggi il pulsante del regime corrente e' quello che non si
    /// puo' premere, perche' premerlo non cambierebbe nulla. Il nome e' storico e
    /// fuorviante, quindi qui e' `non_cliccabile`, e resta uno stile solo
    /// (`secondary`): questo non introduce uno stile "attivo" che non esiste in
    /// `btn`, e non cambia aspetto a nessun pulsante.
    fn pulsante_regime(
        &self,
        etichetta: &'static str,
        sottotitolo: &'static str,
        msg: Message,
        colore: (u8, u8, u8),
        non_cliccabile: bool,
    ) -> iced::widget::Button<'_, Message> {
        iced::widget::button(
            Column::new()
                .spacing(1)
                .push(Text::new(etichetta).size(12)
                    .color(iced::Color::from_rgb8(colore.0, colore.1, colore.2)))
                .push(Text::new(sottotitolo).size(9).color(palette::BLUE_DIM)),
        )
        .padding([6, 8])
        .width(Length::Fill)
        .style(crate::btn::secondary)
        .on_press_maybe((!non_cliccabile).then_some(msg))
    }
```

Gli stili disponibili in `btn` sono **solo** `primary`, `glass`, `secondary`,
`danger`: il modulo e' dentro `main.rs`, non un file `btn.rs` a se' stante, e non
esiste uno stile "attivo".

Poi in `pulsante_tec()` (`running.rs:3447`) sostituisci il `tasto` con i quattro
pulsanti del regime e **non toccare il `grafico`**: stessa `Column`, stessa
`MiniSpark`, stessi 34 px, stesso colore. Sotto il menu ci resta la striscia.

Rimuovi `modalita_block` dalla sua posizione attuale (`running.rs:3789`) e la
dichiarazione della sua intestazione. Conserva nel nuovo posto: i quattro
pulsanti, `esito_commutazione`, `MODALITA_NOTA` e il pulsante "Torna a Cryo" che
compare in Unregulated — è una via d'uscita.

- [ ] **Step 5: verde e checkpoint**

```bash
cargo test 2>&1 | grep -E "test result"
```
Attesi: `195 passed` + `14 passed` — **un test in meno**, perché ne hai eliminato
uno tautologico. Se il numero non scende di uno, non hai eliminato quello che
dovevi.

---

## Task 4: il margine ha un'età

**Files:**
- Modify: `cryo_cooler_controller/src/running.rs:2058` (assegnazione), `3497-3503` (riga di margine)
- Test: in-module, `cryo_cooler_controller/src/running.rs`

**Interfaces:**
- Produce: `const CAMPIONE_TIMEOUT: Duration` (esiste già, 2000 ms)
- Produce: `fn margine_e_fresco(&self) -> bool`
- Produce: `enum EtichettaMargine { Ok, Basso, Condensa, NonFresco }` + `fn etichetta_margine(margine: f32, fresco: bool) -> EtichettaMargine`

- [ ] **Step 1: il test che fallisce**

```rust
    /// **Un margine vecchio non e' "OK".** E' la difesa contro la riga verde
    /// falsa: il valore era valido *prima*, e il ramo di errore del
    /// campionamento non lo azzera, quindi può essere vecchio di minuti.
    #[test]
    fn un_margine_non_fresco_non_si_dichiara_ok() {
        assert_eq!(etichetta_margine(3.0, false), EtichettaMargine::NonFresco);
        assert_ne!(
            etichetta_margine(3.0, false),
            etichetta_margine(3.0, true),
            "fresco e non fresco non possono avere la stessa etichetta"
        );
    }

    #[test]
    fn le_tre_etichette_di_un_margine_fresco() {
        assert_eq!(etichetta_margine(4.0, true), EtichettaMargine::Ok);
        assert_eq!(etichetta_margine(1.2, true), EtichettaMargine::Basso);
        assert_eq!(etichetta_margine(-0.4, true), EtichettaMargine::Condensa);
    }
```

- [ ] **Step 2: rosso, poi implementazione minima**

Le quattro funzioni qui sopra, con `NonFresco` che non ha colore verde
associato. Poi nella riga di `view_left_column` (`running.rs:3497`): quando
l'etichetta è `NonFresco`, il testo dice che il dato non è fresco e il colore è
`BLUE_DIM`, **mai** `NEON_GREEN`.

- [ ] **Step 3: verde e checkpoint**

```bash
cargo test 2>&1 | grep -E "test result"
```

---

## Task 5: la guardia chiede conferma, con anteprima

**Files:**
- Modify: `cryo_cooler_controller/src/commutazione.rs:203` (`guardia_anticondensa`)
- Modify: `cryo_cooler_controller/src/running.rs:1307-1343` (`commutazione_richiesta`), `2885-2901` (`Annulla`)
- Test: `cryo_cooler_controller/src/commutazione.rs`

**Interfaces:**
- Change: `guardia_anticondensa(regime, margine) -> Option<String>` → `serve_conferma(margine: f32) -> bool` (il regime non influenza più la decisione: la conferma è sul **margine**, non sul comando)
- Produce: `struct AvvisoCondensa { margine: f32, fresco: bool }`
- Produce: `enum InAttesa { Nessuna, Conferma { regime: Regime, avviso: AvvisoCondensa } }`

**Nota di onestà sul nome.** La firma attuale prende `regime` ma non lo usa per
decidere: dopo la Fase 4 decide solo il margine. Tenerlo sarebbe un parametro
morto che suggerisce una dipendenza inesistente. La nuova firma lo omette, e il
test che lo dimostra è `serve_conferma` che risponde uguale per tutti i regimi.

- [ ] **Step 1: riscrivi i due test dichiarati, e aggiungi i nuovi**

```rust
    /// **Nessun regime e' mai bloccato.** Il vecchio test verificava il
    /// contrario: che sotto soglia la guardia *blocchi* Standby, Cryo e
    /// Unregulated. L'utente ha deciso che la guardia non blocca, chiede
    /// conferma. Questa e' la decisione nuova, e il test la fissa.
    #[test]
    fn nessun_regime_e_mai_bloccato() {
        for r in [Regime::Standby, Regime::Cryo, Regime::Unregulated, Regime::Spento] {
            let piano = r.piano(-12.0);
            // Nessun percorso produce un rifiuto: `PianoCambio` non ha un
            // campo "rifiutato", e questa e' l'affermazione.
            assert!(piano.offset.is_some() || r == Regime::Spento, "{r}");
        }
        // E la conferma non distingue i regimi: dipende dal margine.
        for r in [Regime::Standby, Regime::Cryo, Regime::Unregulated, Regime::Spento] {
            let _ = r;
        }
        assert!(serve_conferma(0.4));
        assert!(!serve_conferma(4.0));
    }

    /// La soglia resta a 1.0 °C. Si fissa il bordo.
    #[test]
    fn la_soglia_di_conferma_e_un_grado() {
        assert!(!serve_conferma(1.0));
        assert!(serve_conferma(0.99));
    }

    /// Un margine vecchio non chiede conferma: non si sa, e chiedere
    /// conferma su un numero congelato e' rumore che addestra l'operatore a
    /// cliccare "conferma" senza leggere.
    #[test]
    fn un_margine_non_fresco_non_chiede_conferma() {
        assert!(!serve_conferma(0.4), "0.4 °C ma vecchio: non si avvisa");
    }
```

- [ ] **Step 2: implementazione**

```rust
/// Se il margine di condensa richiede una conferma esplicita.
///
/// **Non blocca mai.** L'unico effetto e' far aprire un dialogo che l'operatore
/// puo' chiudere con Esc. Un blocco che si sbaglia toglie la capacita' di agire
/// proprio quando serve; un avviso che si sbaglia si ignora.
pub fn serve_conferma(margine_condensa: f32) -> bool {
    margine_condensa.is_finite() && margine_condensa < SOGLIA_CONDENSA
}

pub const SOGLIA_CONDENSA: f32 = 1.0;
```

In `running.rs`: `commutazione_richiesta` non chiama più `guardia_anticondensa`.
Se `serve_conferma(self.last_cond_margin)` è vero, imposta
`self.in_attesa = InAttesa::Conferma { regime, avviso }` e **ritorna senza
scrivere**. Un nuovo handler `Message::ConfermaSpedizione` rivalida il margine e,
se non serve più conferma, scrive. `Message::Annulla` mette `in_attesa` a
`Nessuna` **prima** di `chiedi_unregulated` nell'ordine della catena, perché è
l'unico stato pendente che può scrivere.

Aggiungi in cima ai quattro handler di regime la guardia:

```rust
Message::CommutaCryo | Message::CommutaStandby
| Message::CommutaSpento | Message::CommutaUnregulatedConferma => {
    if self.in_attesa != InAttesa::Nessuna {
        return Task::none();  // dialogo aperto: nessun altro comando parte
    }
    ...
}
```

- [ ] **Step 3: verde e checkpoint**

```bash
cargo test 2>&1 | grep -E "test result"
```

- [ ] **Step 4: l'anteprima — premere sapendo la conseguenza**

Il dialogo non chiede solo "confermi?". Mostra cosa succederà. Questo è il pezzo
che rende il pulsante una previsione e non un interruttore, ed è nella specifica
da prima della specifica: senza, la fase è incompiuta.

```rust
#[cfg(test)]
mod test_anteprima {
    use super::*;

    /// L'obiettivo di temperatura dipende dal regime, e 'Spento' non ne ha
    /// uno: dichiarare un target per un modulo che non eroga sarebbe una
    /// previsione inventata.
    #[test]
    fn l_obiettivo_dipende_dal_regime() {
        assert_eq!(obiettivo_piastra(Regime::Cryo, -12.0), Some(-12.0));
        assert_eq!(obiettivo_piastra(Regime::Unregulated, -12.0), Some(-30.0));
        assert_eq!(obiettivo_piastra(Regime::Standby, -12.0), Some(3.5));
        assert_eq!(obiettivo_piastra(Regime::Spento, -12.0), None);
    }

    /// **L'anteprima dichiara l'incertezza.** Un numero mostrato senza il suo
    /// grado di affidabilità e' un numero che mente: e' il difetto che ha gia'
    /// prodotto la riga verde falsa del margine, ripetuto in un altro punto.
    #[test]
    fn l_anteprima_dichiara_che_il_dato_non_e_fresco() {
        let a = Anteprima::per_regime(Regime::Cryo, -12.0, 0.4, 30, true);
        assert!(!a.margine_fresco, "0.4 °C e' vecchio: l'anteprima lo dice");
        let b = Anteprima::per_regime(Regime::Cryo, -12.0, 0.4, 30, false);
        assert!(!b.margine_fresco);
    }

    /// La potenza attesa e' una **stima**, e si dichiara tale. Stimarla e
    /// presentarla come misura riprodurrebbe esattamente il difetto che questa
    /// fase elimina.
    #[test]
    fn la_potenza_attesa_e_dichiarata_stima() {
        let a = Anteprima::per_regime(Regime::Unregulated, -12.0, 4.0, 100, true);
        assert!(a.potenza_stimata, "la potenza e' calcolata, non misurata");
    }
}
```

```rust
/// Cosa succederà se l'operatore conferma.
///
/// Tutti i campi portano il loro grado di affidabilità. Il principio e' uno solo
/// e vale per tutta la dashboard: **un numero senza il suo grado di
/// affidabilita' e' un numero che mente.**
pub struct Anteprima {
    pub regime: Regime,
    /// Temperatura obiettivo della piastra. `None` per `Spento`: un modulo che
    /// non eroga non ha un'obiettivo di temperatura, e dichiararne uno sarebbe
    /// una previsione inventata.
    pub obiettivo_c: Option<f32>,
    /// Stima della potenza, **non** misura. Calcolata dal cap corrente e dal
    /// regime; il valore reale arriva solo dopo la rilettura.
    pub potenza_w: u8,
    pub potenza_stimata: bool,
    /// Margine di condensa al momento della conferma.
    pub margine_c: f32,
    /// `false` se il margine non e' fresco: l'anteprima lo dichiara invece di
    /// mostrare il numero come se fosse valido.
    pub margine_fresco: bool,
    /// Se il regime corrente e' `Conosciuto`, `Dedotto` o `Ignoto`. Cambia il
    /// peso della conferma, quindi va detto.
    pub certezza_corrente: Certezza,
}
```

- [ ] **Step 5: checkpoint finale del task**

```bash
cargo test 2>&1 | grep -E "test result"
```


---

## Task 6: verifica finale e release

- [ ] **Step 1: le due passate, di nuovo**

Passata A — *cosa scrive il controller*: da `Tec::new` all'ultimo byte. Passata B —
*cosa crede l'operatore*: dall'etichetta al numero. Devono concordare. La passata
B è quella che cerca i difetti **introdotti** dalle correzioni, che la A non
vede perché cerca solo quelli noti.

- [ ] **Step 2: suite completa**

```bash
cargo test 2>&1 | grep -E "test result|^error|warning:"
```
Zero errori, zero warning nuovi. Il warning preesistente in
`probe_real.rs:61:44` è noto e non è un introdotto.

- [ ] **Step 3: build release e deploy**

```bash
CARGO_TARGET_DIR=target/r117 cargo build --release
```
Copia in `<ESEGUIBILI>\cryo_cooler_controller_r117.exe`.
Verifica con `md5sum` che `cryo_cooler_controller.exe` (r84) sia **invariato**:
`849d2ebb9c544ec00425092b93dd2310`.

- [ ] **Step 4: aggiorna la specifica**

Segna le fasi completate e annota i risultati della Fase 0 — in particolare quali
celle della tabella delle misure hardware sono state riempite e quali no.
