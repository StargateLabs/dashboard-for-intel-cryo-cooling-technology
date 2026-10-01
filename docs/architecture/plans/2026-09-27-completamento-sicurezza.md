# Completamento sicurezza e rifinitura — Piano residuo

> **Per worker agentici:** SUB-SKILL RICHIESTA: `superpowers:executing-plans` per l'esecuzione inline, oppure `superpowers:subagent-driven-development` per le fasi indipendenti.

**Obiettivo:** Chiudere i quattro punti rimasti, senza regressioni sulle 33+7 verifiche già verdi.

**Architettura:** Le fasi A e B sono indipendenti e su file diversi, quindi possono andare in parallelo. La fase C è un refactor di stato e va fatta per ultima, quando le altre hanno smesso di muovere campi.

**Stack tecnologico:** Rust, iced 0.13.1, rusqlite 0.31, serde_json, plotters.

**Specifica:** questo file. Per il protocollo Delta² non esiste documentazione pubblica — verificato con ricerca online il 2026-09-27: nessuna fonte descrive la maschera di stato a 18 bit. Ogni assunzione sul firmware va quindi trattata come non verificata e mai usata per spegnere l'hardware.

## Vincoli globali

- Non terminare mai il processo dell'utente. Non fare commit Git. Italiano, commenti senza accenti.
- Baseline di riferimento: **33 test verdi** su `cryo_cooler_controller` + **7** su `cryo_cooler_controller_lib`, **0 warning**.
- Verifiche: `cd <TMP> && cmd //c b2.bat` e `cargo test --target-dir target\b2 -p cryo_cooler_controller`.
- Criterio di non-regressione: nessuna delle 40 verifiche esistenti può cambiare esito, e il build deve restare a **0 warning**.

---

## Fase A — L'auto mode non deve raffreddare di meno quando non sa

**File:** `cryo_cooler_controller/src/automanager.rs`, `cryo_cooler_controller/src/running.rs`

**Gravità:** CRITICA. È l'ultima protezione critica rimasta.

**Problema verificato.** `classifica(None, None) => Regime::Idle`. `Idle` è il regime **meno** aggressivo (`max_power = 60`, `set_point = -5 °C`). Quindi la perdita di entrambi i sensori non è neutra: **riduce il raffreddamento**. Sulla V1 la temperatura CPU è quasi sempre assente (niente HWiNFO), quindi l'unico segnale è il carico Windows: se `GetSystemTimes` fallisce una volta, il gestore dopo 15 campioni scivola a Idle e lo scrive in hardware. Il firmware definisce già `PID_INVALID` (`lib.rs:459`): la condizione è prevista, non sconosciuta.

**Interfacce:** produce `Regime::Invalido` e `AutoManager::push` che non emette nulla quando i dati mancano.

- [ ] **Step 1: scrivi il test rosso**

```rust
    /// Con entrambi i sensori assenti non si sceglie Idle.
    ///
    /// Idle e' il regime MENO aggressivo, quindi "non so niente" diventava
    /// "raffredda meno". E' la direzione sbagliata: su un controller il cui
    /// compito e' tenere la CPU sotto soglia, perdere un sensore deve
    /// significare "tengo l'ultimo regime", non "abbasso tutto".
    #[test]
    fn senza_ingressi_non_si_abbassa_a_idle() {
        assert_ne!(classifica(None, None), Regime::Idle);
    }

    /// E non si sceglie nemmeno Picco: non abbiamo dati per meritarlo.
    #[test]
    fn senza_ingressi_non_si_sale_a_picco() {
        assert_ne!(classifica(None, None), Regime::Picco);
    }
```

- [ ] **Step 2: aggiungi la variante**

```rust
    /// Nessun dato su cui decidere.
    ///
    /// Non e' un regime: e' l'assenza di una decisione. Il gestore non
    /// cambia nulla e `RunningState` mantiene l'ultimo regime valido. Il
    /// firmware ha gia' `PID_INVALID` per questo caso, quindi non e' una
    /// situazione inventata.
    Invalido,
```

`etichetta()` restituisce `"sensori assenti"`. In `RunningState`, `apply_regime` con `Invalido` deve **uscire senza scrivere nulla in hardware**.

- [ ] **Step 3: verifica**

`cmd //c b2.bat` → 0 warning. `cargo test` → **35 passed**.

---

## Fase B — Non perdere dati in silenzio

**File:** `cryo_cooler_controller/src/config.rs`, `session_db.rs`, `running.rs`

**Gravità:** MEDIA. Non può danneggiare l'hardware, ma fa perdere il lavoro di una sessione senza alcun indizio.

- [ ] **Step 1: un config illeggibile non deve cancellare i profili**

In `config.rs::load()`, oggi un errore di parsing viene ingoiato e `load()` restituisce i profili predefiniti. Il `save()` successivo scrive quelli **sopra** l'unica copia dei profili dell'utente. Copia il file altrove prima di sostituirlo:

```rust
            Err(e) => {
                // Il file originale non e' piu' scrivibile senza perdere
                // l'unica copia dei profili. Senza questo salvataggio, il
                // primo cambiamento successivo avrebbe scritto i valori
                // predefiniti sopra tutto.
                let backup = path.with_extension("corrotto.json");
                let _ = std::fs::copy(&path, &backup);
                eprintln!("config.json illeggibile ({e}), copia salvata in {backup:?}");
                Self::predefinita()
            }
```

- [ ] **Step 2: un database non aperto deve dirlo**

`SessionDb::is_open()` esiste e non viene mai chiamato. In `running.rs::new()`:

```rust
            session_db: {
                let mut db = SessionDb::open();
                // `is_open()` esisteva e non veniva letto: database
                // illeggibile o disco pieno facevano perdere una sessione
                // intera senza un solo messaggio.
                if !db.is_open() {
                    eprintln!("session_db: storico non disponibile, i campioni non verranno salvati");
                }
                db.start_session("Sessione");
                db
            },
```

- [ ] **Step 3: il VACUUM non va sul thread della UI**

`prune_old_sessions(50)` chiama `VACUUM`, che riscrive l'intero file e chiede circa il doppio dello spazio libero. Gira dentro `open()`, quindi dentro `new()`, quindi sul thread iced al momento della connessione: secondi di finestra bloccata a ogni avvio, crescenti con il database. Spostalo in un metodo esplicito chiamato una volta dopo la connessione, e solo se serve:

```rust
    /// Potatura delle sessioni vecchie, da chiamare **fuori** dal percorso
    /// di avvio. `VACUUM` riscrive tutto il file: dentro `open()` bloccava
    /// la finestra per secondi a ogni avvio, per una condizione che
    /// quasi mai si verifica.
    pub fn pota_se_necessario(&mut self) {
        if self.conta_sessioni() <= 50 { return; }
        self.prune_old_sessions(50);
    }
```

- [ ] **Step 4: verifica**

`cmd //c b2.bat` → 0 warning. `cargo test` → **35 passed**.

---

## Fase C — Colori semantici per categoria

**File:** `cryo_cooler_controller/src/main.rs` (modulo `palette`), `running.rs`

**Vincolo dichiarato:** le 8 icone esistenti sono **illustrazioni già a colori** (media RGB 71/119/147, canale R variato di 234), non maschere monocrome, e `iced 0.13` non espone `tint` su `Image`. Non si possono tingere senza rovinarle e non è possibile generare artwork nuovo di qualità da codice. Ciò che è alla portata ed è utile: **un colore semantico stabile per categoria**, così la dashboard si legge a colori senza decifrare ogni icona.

- [ ] **Step 1: i token**

```rust
    // Colori semantici: uno per categoria, mai riutilizzato. A 16 px
    // un'illustrazione dettagliata non e' leggibile e il colore e' l'unico
    // segnale che resta.
    pub const SEM_TEMPERATURA:  Color = Color { r: 0.20, g: 0.80, b: 0.95, a: 1.0 };
    pub const SEM_POTENZA:      Color = Color { r: 1.00, g: 0.70, b: 0.10, a: 1.0 };
    pub const SEM_CIRCOLAZIONE: Color = Color { r: 0.55, g: 0.45, b: 1.00, a: 1.0 };
    pub const SEM_PROTEZIONE:   Color = Color { r: 0.10, g: 0.90, b: 0.55, a: 1.0 };
    pub const SEM_RISCHIO:      Color = Color { r: 1.00, g: 0.30, b: 0.35, a: 1.0 };
```

- [ ] **Step 2: allinea `Gravita::colore()` ai token**

`Critica → SEM_RISCHIO`, `Avviso → SEM_POTENZA`, `Info → SEM_TEMPERATURA`, così avvisi e intestazioni parlano la stessa lingua.

- [ ] **Step 3: la pastiglia di categoria**

```rust
    /// Pastiglia di categoria: 8 px di colore pieno davanti al nome.
    ///
    /// Non decora: da' alla sezione un'identita' riconoscibile senza
    /// leggere, ed e' l'unico accenno di colore che regge a 16 px.
    fn intestazione_sezione(etichetta: &str, colore: iced::Color) -> Row<'_, Message> {
        Row::new()
            .spacing(7)
            .align_y(iced::Alignment::Center)
            .push(
                Container::new(iced::widget::Space::with_width(Length::Fixed(8.0)))
                    .width(Length::Fixed(8.0))
                    .height(Length::Fixed(8.0))
                    .style(move |_: &iced::Theme| iced::widget::container::Style {
                        background: Some(iced::Background::Color(colore)),
                        border: iced::Border {
                            color: iced::Color { r: colore.r, g: colore.g, b: colore.b, a: 0.35 },
                            width: 1.0,
                            radius: 4.0_f32.into(),
                        },
                        text_color: None,
                        shadow: palette::glow(colore, 0.35),
                    }),
            )
            .push(Text::new(etichetta).size(9).color(palette::BLUE_DIM))
    }
```

Applica a: temperature → `SEM_TEMPERATURA`; potenza e tensione → `SEM_POTENZA`; pompa e condensazione → `SEM_CIRCOLAZIONE`; allarmi e watchdog → `SEM_PROTEZIONE`; rischio condensa → `SEM_RISCHIO`.

- [ ] **Step 4: verifica**

`cmd //c b2.bat` → 0 warning. `cargo test` → **35 passed**.

---

## Fase D — Build e pubblicazione

- [ ] **Step 1: verifica finale completa**

```bash
cd <REPO>
touch cryo_cooler_controller/src/running.rs
call "C:\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
cargo build  --target-dir target\b2 -p cryo_cooler_controller
cargo test   --target-dir target\b2 -p cryo_cooler_controller
cargo test   --target-dir target\b2 -p cryo_cooler_controller_lib --lib
```

Atteso: 0 errori, **0 warning**, 35 + 7 test verdi.

- [ ] **Step 2: release in directory nuova**

```bash
cd <TMP>
sed 's/r10/r29/' rel10.bat > rel29.bat
cmd //c rel29.bat
```

- [ ] **Step 3: pubblicazione senza terminare il processo**

Il file in uso non è sovrascrivibile. Usare lo script di attesa: `deploy_attesa.ps1` con `r29` al posto di `r28`. Lo script non chiude nulla, attende che l'utente chiuda l'app e poi copia verificando l'hash. Se scade, esce con codice 2 **senza aver modificato nulla**, ed è il comportamento corretto.

- [ ] **Step 4: collaudo**

`CRYO_COMMISSIONING=1`. Il log deve mostrare la discesa per la guardia termica e il comportamento del limitatore pompa. Se non compare alcuna discesa, il criterio lento non sta vedendo campioni: verificare `campioni_lenti`.

- [ ] **Step 5: misura della GPU**

Scheda **AMD Radeon RX 7900 XTX**: `nvidia-smi` non serve. Gestione Attività → Dettagli → colonna GPU. Il confronto valido è con r24, che non conteneva nessuna delle fix.

---

## Autoverifica

**Copertura.** Le quattro lacune del piano precedente sono coperte: A (auto mode fail-closed), B (config/DB), C (colori), D (deploy). Le due lacune dichiarate nel piano precedente — `autostart.rs` e la pipe Discord senza timeout — restano **fuori** e sono ripetute qui perché non è giusto farle sparire fra un piano e l'altro: sono difetti reali, nessuno dei due può danneggiare l'hardware, e il secondo è inattivo finché il `client_id` è un segnaposto.

**Rischio dichiarato.** `BOARD_TEMP_OK` resta non verificato: nessuna fonte online descrive il protocollo. Per questo la Fase 6 precedente degrada a 30% invece di spegnere, e `resp.op_code == op` nella sonda è sempre `false` (non sempre `true` come riportava la revisione: `send_cmd_once` accetta solo risposte con opcode `richiesta + 127`, che non può mai coincidere). Entrambi da risolvere con l'hardware in mano, non con una ricerca.
