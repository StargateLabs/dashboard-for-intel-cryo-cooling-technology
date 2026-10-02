# Sicurezza TEC e rifinitura dashboard: Piano di implementazione

> **Per worker agentici:** SUB-SKILL RICHIESTA: usa `superpowers:subagent-driven-development` (consigliato) oppure `superpowers:executing-plans` per eseguire questo piano task per task. I passi usano caselle di spunta (`- [ ]`) per il tracciamento.

**Obiettivo:** Chiudere i 4 bug CRITICI e i 6 HIGH trovati nella revisione del codice, che possono danneggiare l'hardware (TEC lasciato acceso, guardia termica invertita, protezione pompa inerte sulla V1, letture fuori dai limiti), e poi completare la rifinitura dell'interfaccia (notifiche, colori semantici, F11).

**Architettura:** Il problema di fondo emerso dalla revisione è che **nessuno possiede "la potenza con cui il modulo sta girando davvero"**. `inputs.max_power` (intenzione), `applied_power` (credenza), `max_power_before_emergency` (uno snapshot usato come se fosse un soffitto) e `fw_power_level` (misurato) sono quattro valori scritti indipendentemente, e ognuno dei 9 siti di scrittura può aggiornare un sottoinsieme diverso. Questo piano introduce un unico proprietario e, attorno ad esso, lascia che ogni protezione esprimi **quello che misura davvero** invece di dedurlo da campi che non le appartengono.

**Stack tecnologico:** Rust, iced 0.13.1 (wgpu), serialport 4.9, plotters 0.3, rusqlite 0.31, Windows (windows-sys).

**Specifica:** questo stesso file. Non esiste un documento di design separato: le revisioni parallele citate nelle note di ogni task sono la specifica.

## Vincoli globali

- Rispondere in italiano. Non fare commit Git. Non eseguire `taskkill`, `Stop-Process` o qualsiasi cosa che termini un processo.
- **Mai terminare il controller dell'utente.** Lo spegnimento va implementato nel codice dell'app, non dall'esterno.
- Hardware: controller **V1** su modulo TEC **V2**. Il V1 **non** ha controllo di pompa/ventole **né** sensore RPM pompa. Questo vale per ogni decisione sulla pompa.
- Soglie termiche: Standby oltre 80 °C, Shutdown oltre 90 °C, guardia software a 66 °C, banda morta 58–66 °C.
- Comandi di verifica, tutti già usati con successo
 - build: `cd <TMP> && cmd //c b2.bat`
 - test: `cd <REPO> && cargo test --target-dir target\b2 -p cryo_cooler_controller`
 - baseline al momento della stesura: **22 test verdi, 0 warning**
- Deploy: copiare l'eseguibile in `<ESEGUIBILE>` **solo** se nessun processo lo blocca; usare lo script di attesa descritto nel Task 13.
- Commenti in italiano, senza accenti (convenzione del file già esistente).
- Nessun valore di allarme può essere inventato. Se una misurazione non esiste, la regola non si valuta: è la regola introdotta in `alerts.rs` e protetta da test.

---

## Struttura dei file

| File | Ruolo | Stato |
|---|---|---|
| `cryo_cooler_controller/src/running.rs` | Stato, protezioni, auto mode, tutta la UI | Modificato in tutti i task |
| `cryo_cooler_controller/src/pumpwatch.rs` | Rilevamento pompa ferma dalla firma termica | Task 4 |
| `cryo_cooler_controller/src/alerts.rs` | Regole e cooldown | Task 9 (costanti in `running.rs`) |
| `cryo_cooler_controller/src/automanager.rs` | Regimi automatici | Task 4 (fail-closed) |
| `cryo_cooler_controller/src/hwinfo.rs` | Lettura sensori, `unsafe` | Task 5 |
| `cryo_cooler_controller/src/config.rs` | Persistenza | Task 10 |
| `cryo_cooler_controller/src/session_db.rs` | Storico sessioni | Task 10 |
| `cryo_cooler_controller/src/overlay.rs` | RTSS / Discord, `unsafe` | Task 8 |
| `cryo_cooler_controller/src/main.rs` | Ciclo di vita, finestra, `icons`, `btn`, `palette` | Task 7, 8, 11 |
| `cryo_cooler_controller_lib/src/lib.rs` | Protocollo seriale | Task 11 |

**Decisione di decomposizione:** `running.rs` è a 4231 righe e i tre quarti dei task lo toccano. NON si divide ora: è un refactoring rischioso che mescolerebbe con correzioni di sicurezza. Si valuta dopo, con il codice stabile e coperto da test.

---

## Fase 0: Baseline congelata

- [ ] **Step 1: Congela la baseline**

```bash
cd "<REPO>"
cp cryo_cooler_controller/src/running.rs "<TMP>\running_r27_backup.rs"
```

- [ ] **Step 2: Verifica che la baseline è verde**

Run: `cargo test --target-dir target\b2 -p cryo_cooler_controller`
Expected: `test result: ok. 22 passed; 0 failed`

Il file di backup serve perché i task 2 e 3 toccano lo stesso blocco di codice: se uno dei due non torna, si ripristina e si rifa con ordine inverso.

---

## Fase 1: Lo spegnimento non deve dipendere da un flag invecchiato

**File:** `cryo_cooler_controller/src/running.rs:674` (shutdown), `running.rs:1252-1259` (errore di battito), `running.rs:1202` (battito ok)

**Interfacce:** produce `fn tec_abilitato_software(&self) -> bool`

- [ ] **Step 1: Scrivi il test del fallback**

Il test non può coprire `shutdown()` (chiude la porta seriale), quindi si copre il **predicato** che la governa, estratto in un metodo puro.

```rust
    /// Il TEC e' stato acceso da un comando **di questo software**?
    ///
    /// Non usare `tec_status.contains(PID_RUNNING)` per decidere se
    /// spegnere: `tec_status` arriva dal polling e resta indietro quando il
    /// battito fallisce, ed e' proprio il caso in cui piu' serve spegnere.
    fn tec_abilitato_software(&self) -> bool {
        self.tec_abilitato || self.applied_power > 0
    }
```

```rust
    #[test]
    fn spegnimento_non_dipende_dal_battito() {
        // Il comando e' riuscito, ma il polling non ha mai confermato:
        // la combinazione che lasciava il modulo acceso.
        let s = Costrutto::di_test();
        s.tec_abilitato = true;
        s.tec_status = TecStatus::empty();
        assert!(s.tec_abilitato_software(), "spegnimento skippato");
    }
```

- [ ] **Step 2: Aggiungi i campi**

```rust
    /// L'utente ha chiesto l'accensione e il comando e' andato a buon fine.
    /// E' l'unica verita' su "il TEC e' acceso": non viene dal polling.
    tec_abilitato:    bool,
```

Inizializza `tec_abilitato: false` in `new()`.

- [ ] **Step 3: Imposta e azzera il campo nei due handler**

In `Message::Enable`, ramo `Ok(())`, subito dopo `self.applied_power = start;`

```rust
                self.tec_abilitato = true;
```

In `Message::Disable`, dopo la chiamata a `disable()`

```rust
        self.tec_abilitato = false;
```

- [ ] **Step 4: Usa il predicato in `shutdown()`**

```rust
        // Si spegne se il SOFTWARE l'aveva acceso, non se il polling dice che
        // gira. Con un link instabile `tec_status` resta sul valore
        // pre-accensione e il modulo restava alimentato a schermo spento,
        // senza guardia, senza watchdog e senza nessuno che lo controlli.
        if self.tec_abilitato_software() {
            if let Err(e) = self.tec.disable() {
                // Non c'e' piu' UI: l'errore va dove l'operatore lo vede
                // dopo, non perso.
                crate::commissioning::event("SHUTDOWN", &format!("disable fallito: {e}"));
            }
        }
```

- [ ] **Step 5: Compila e testa**

Run: `cmd //c b2.bat && cargo test --target-dir target\b2 -p cryo_cooler_controller`
Expected: 23 passed, 0 failed

---

## Fase 2: La pompa: sospendere la protezione, non dichiarare un guasto

**File:** `cryo_cooler_controller/src/running.rs:981-1013`

**Interfacce:** consuma `pump_stalled: bool` (già presente); produce `pompa_in_fallo() -> bool`

- [ ] **Step 1: Scrivi il test del caso V1**

Il caso che oggi è rotto: nessun sensore, e il software deve **non** limitare la potenza.

```rust
    /// Sulla V1 il sensore RPM non esiste. "Non leggibile" NON vuol dire
    /// "ferma": se si limitasse la potenza, il TEC resterebbe al 50% per
    /// tutta la sessione e la firma termica (che vuole potenza >= 60%)
    /// non potrebbe mai giudicare niente.
    #[test]
    fn senza_sensore_la_protezione_e_sospesa() {
        assert!(!pompa_in_fallo(pump_readable = false, rpm = 0.0, stalled = false));
    }

    /// Con il sensore che legge sotto soglia, il guasto e' reale.
    #[test]
    fn rpm_basso_e_guasto_reale() {
        assert!(pompa_in_fallo(pump_readable = true, rpm = 0.0, stalled = false));
    }

    /// La firma termica vale anche senza sensore: e' l'unico giudizio
    /// disponibile sulla V1, quindi DEVE poter azionare la protezione.
    #[test]
    fn firma_termica_aziona_la_protezione_senza_sensore() {
        assert!(pompa_in_fallo(pump_readable = false, rpm = 0.0, stalled = true));
    }
```

- [ ] **Step 2: Estrai la funzione pura**

```rust
/// C'è un guasto di circolazione **dimostrato**?
///
/// Tre casi, e vanno tenuti separati perché hanno conseguenze opposte:
/// - sensore che legge sotto soglia: guasto reale, si limita;
/// - nessun sensore e firma termica che ha parlato: guasto reale, si limita;
/// - nessun sensore e nessun altro indizio: **non si sa niente**, e non si
///   limita. Limitare "per sicurezza" qui è quello che rendeva la V1
///   bloccata al 50%: non è prudenza, è un guasto che si autosabotaggia.
fn pompa_in_fallo(pump_readable: bool, rpm: f32, stalled: bool) -> bool {
    (pump_readable && rpm < PUMP_MIN_RPM) || stalled
}
```

- [ ] **Step 3: Riscrivi il blocco d'emergenza**

Sostituisci l'intera struttura `if !pompa_alta { if !self.pump_emergency_active { … } }` con

```rust
                            // Guarda tutti i casi insieme, non solo l'RPM:
                            // `let _ = pompa_alta;` era il resto del
                            // ragionamento, scartato. Il rilevatore termico
                            // scriveva "FERMA" in rosso sulla dashboard e non
                            // cambiava nulla: un'indicazione di guasto senza
                            // nessuna protezione dietro è peggio di niente,
                            // perché l'operatore crede di essere coperto.
                            const PUMP_MIN_RPM: f32 = 200.0;
                            let in_fallo = pompa_in_fallo(
                                self.pump_readable,
                                self.last_pump_rpm,
                                self.pump_stalled,
                            );

                            if in_fallo && !self.pump_emergency_active {
                                self.max_power_before_emergency = self.inputs.max_power;
                                let lim = self.inputs.max_power.min(50);
                                self.inputs.max_power = lim;
                                if let Err(e) = self.tec.set_power_level(lim) {
                                    self.error_text =
                                        Some(format!("Limitatore pompa: {e}"));
                                } else {
                                    self.applied_power = lim;
                                }
                                self.pump_emergency_active = true;
                                crate::commissioning::event(
                                    "POMPA",
                                    &format!("circolazione non confermata: tetto {lim}%"),
                                );
                            } else if !in_fallo && self.pump_emergency_active {
                                // Ripristino: senza questo ramo il limite
                                // restava inchiodato per sempre, anche dopo
                                // che la pompa ripartiva.
                                self.pump_emergency_active = false;
                                self.inputs.max_power = self.max_power_before_emergency;
                                crate::commissioning::event(
                                    "POMPA",
                                    "circolazione ristabilita: tetto ripristinato",
                                );
                            }
```

- [ ] **Step 4: Verifica che lo spazio non sia più l'unica protezione**

Con la Fase 2 il TEC sulla V1 torna a poter salire sopra il 60%, quindi la firma termica di `pumpwatch` può finalmente giudicare. Questo è il motivo per cui la Fase 4 viene dopo e non prima.

Run: `cmd //c b2.bat && cargo test --target-dir target\b2 -p cryo_cooler_controller`
Expected: 26 passed

---

## Fase 3: Un solo proprietario della potenza

**File:** `cryo_cooler_controller/src/running.rs` (5 siti di scrittura: 675, 1176, 1259, 1305, 1360)

**Interfacce:** produce `fn applica_potenza(&mut self, next: u8) -> bool`

- [ ] **Step 1: Scrivi il test dell'invariante**

```rust
    /// L'invariante: dopo ogni tentativo, `applied_power` dice quello che
    /// l'hardware sta davvero facendo. Prima valeva solo in 4 dei 9 siti di
    /// scrittura, e il re-push periodico riscriveva in hardware il valore
    /// "creduto": se la guardia era ferma nella banda morta, l'emergenza
    /// pompa veniva annullata ogni 10 secondi, per sempre.
    #[test]
    fn applica_potenza_aggiorna_lo_stato_solo_se_scritta() {
        let mut s = Costrutto::di_test();
        s.applied_power = 100;
        assert!(s.applica_potenza(40));
        assert_eq!(s.applied_power, 40);

        let mut s = Costrutto::di_test();
        s.applicazione_fallisce = true;
        s.applied_power = 100;
        assert!(!s.applica_potenza(40));
        assert_eq!(s.applied_power, 100, "stato aggiornato senza successo hardware");
    }
```

- [ ] **Step 2: Implementa il proprietario unico**

```rust
    /// Scrive la potenza e, **solo se la scrittura è riuscita**, aggiorna
    /// `applied_power`.
    ///
    /// Questo è l'unico punto del programma che scrive la potenza del
    /// modulo. Esisteva in cinque, e ognuno ricordava una parte della
    /// verità: il risultato era che il re-push periodico poteva
    /// annullare una protezione già attiva, perché scriveva in hardware il
    /// valore *creduto* invece di quello *misurato*.
    ///
    /// Ritorna `true` se la scrittura è andata a buon fine.
    fn applica_potenza(&mut self, next: u8) -> bool {
        match self.tec.set_power_level(next) {
            Ok(()) => {
                self.applied_power = next;
                true
            }
            Err(e) => {
                self.error_text = Some(format!("Potenza TEC: {e}"));
                false
            }
        }
    }
```

- [ ] **Step 3: Sostituisci i cinque siti**

Ciascuno dei cinque diventa una chiamata a `applica_potenza`, mantenendo **esattamente** la semantica di successo/fallimento che avevano

- riga 675 (guardia): `if self.applica_potenza(next) { /* log OK */ }`
- riga 1176 (emergenza pompa): `self.applica_potenza(lim);`, il testo dell'errore resta quello di `applica_potenza`
- riga 1259 (watchdog): `self.applica_potenza(WATCHDOG_POWER); self.watchdog_tripped = true;`
- riga 1305 (OCP): `if self.applica_potenza(ridotto) { self.ocp_mitigated = true; /* log */ }`
- riga 1360 (soft start): `self.applied_power = start;`, **attenzione**: qui l'hardware è già stato acceso da `enable()`, che ha fatto la propria scrittura. Non chiamare `applica_potenza` o si scrive due volte; lascia l'assegnazione e aggiungi il commento

```rust
        // `enable()` ha gia' scritto la potenza in hardware e ha avuto
        // esito positivo: qui si registra solo lo stato, senza riscrivere.
```

- [ ] **Step 4: Il soffitto non è più uno snapshot**

`max_power_before_emergency` veniva impostato al cap **prima** dell'emergenza e poi usato con `.min()` per "limitare", ma il valore memorizzato era il più alto, quindi il `min` non limitava nulla. Sostituisci ogni `.min(self.max_power_before_emergency)` con

```rust
            // Il tetto effettivo e' il cap dell'utente ridotto, se c'e', dal
            // limitatore d'emergenza attualmente in forza. Si ricalcola ogni
            // volta invece di memorizzare uno snapshot: uno snapshot scaduto
            // e' un limite che non limita piu' niente.
            let tetto = match self.pump_emergency_active {
                true  => self.max_power_before_emergency,
                false => self.inputs.max_power,
            };
            let target = self.inputs.max_power.min(tetto);
```

- [ ] **Step 5: Il re-push usa il valore misurato**

```rust
                if self.tick_count % POWER_LEVEL_PUSH_INTERVAL == 3 {
                    if self.tec_status.contains(TecStatus::PID_RUNNING) && self.tec_abilitato {
                        // Re-push del valore **misurato**, non del cap
                        // richiesto: altrimenti si annullerebbe la protezione
                        // appena applicata.
                        let _ = self.applica_potenza(self.applied_power);
                    }
                }
```

Run: `cmd //c b2.bat && cargo test --target-dir target\b2 -p cryo_cooler_controller`
Expected: 28 passed

---

## Fase 4: La firma termica vede i cali lenti

**File:** `cryo_cooler_controller/src/pumpwatch.rs`

**Interfacce:** modifica `Config`, aggiunge `salita_lenta: f32` e `campioni_lenti: u32`

- [ ] **Step 1: Scrivi il test del falso negativo**

```rust
    /// Il caso che il test esistente definiva come corretto ma non lo e': un
    /// fermo pompa reale su un carico pesante sale di poco e in poco. Con la
    /// soglia di 1.5 °C in 6 s servono 16 °C/min, che un blocco
    /// scaldato male non raggiunge mai: il rilevatore era cieco proprio
    /// dove serve.
    #[test]
    fn detects_calo_lento() {
        let mut p = PompaWatch::with_config(Config {
            potenza_minima: 60, salita_minima: 1.5, campioni: 12,
            salita_lenta: 0.3, campioni_lenti: 60, ..Default::default()
        });
        p.reset();
        // 60 campioni, +0.2 °C ciascuno: in 30 s la piastra sale di 12 °C
        // mentre il TEC prova a raffreddare. Nessun singolo tratto di 12
        // campioni raggiunge 1.5 °C, ma il trend è inequivocabile.
        for i in 0..60 {
            p.poll(i as f32 * 0.2, true, 100.0, 40.0);
        }
        assert!(p.guasto(), "calo lento non rilevato");
    }
```

- [ ] **Step 2: Aggiungi i campi e il criterio lento**

```rust
    /// Variazione minima **al minuto**, su una finestra lunga.
    pub salita_lenta:   f32,
    /// Campioni della finestra lunga.
    pub campioni_lenti: u32,
```

```rust
            potenza_minima: 60,
            salita_minima: 1.5,
            campioni: 12,
            // 0.3 °C/min su un minuto: ben sopra il rumore di un sensore
            // (0.2 °C) e ben sotto qualsiasi salita dovuta al TEC, che
            // sta provando a raffreddare e non ha motivo di scaldare.
            salita_lenta: 0.3,
            campioni_lenti: 60,
```

In `poll()`, dopo il criterio breve, aggiungi quello lento sulla stessa finestra

```rust
        // Criterio lento: vale quando il guasto non e' rapido. Un loop
        // secco su un carico con molta massa termica non sale di 1.5 °C in
        // sei secondi, e il criterio breve, da solo, non lo vede mai.
        let lento = self.finestra.len() as f32 >= self.cfg.campioni_lenti as f32;
        if lento {
            let salita_al_minuto =
                (self.finestra[0] - self.finestra[self.finestra.len() - 1]) * 2.0;
            if salita_al_minuto >= self.cfg.salita_lenta {
                self.consecutivi += 1;
            }
        }
```

- [ ] **Step 3: Resetta su ogni cambio di regime**

```rust
    fn applica_regime(&mut self, r: Regime) {
        // Il regime sposta il setpoint anche di 13 °C e cambia la potenza:
        // la piastra sale per motivi **normali**, e senza azzerare la finestra
        // quella salita viene letta come loop fermo. Il manuale lo dice gia'
        // per il setpoint, qui mancava.
        self.pompa_watch.reset();
```

Applica lo stesso `self.pompa_watch.reset()` in `Message::UpdateMaxPower` e in `Message::Enable`.

- [ ] **Step 4: L'auto mode non deve scegliere "raffredda meno" quando non sa**

```rust
/// `classifica` con entrambi gli ingressi assenti.
///
/// Restituire `Idle` e' la direzione sbagliata: Idle e' il regime **meno**
/// aggressivo, quindi la perdita di un sensore riduceva il raffreddamento.
/// Il firmware definisce gia' `PID_INVALID`: la condizione e' prevista,
/// non e' un caso sconosciuto.
fn classifica(carico: Option<f32>, temp: Option<f32>) -> Regime {
    match (carico, temp) {
        (None, None) => Regime::Invalido,
        // ...
    }
}
```

Aggiungi `Regime::Invalido` con `etichetta() == "sensori assenti"` e, in `RunningState`, non applicare nulla quando il regime è `Invalido`: si mantiene l'ultimo regime valido.

Run: `cmd //c b2.bat && cargo test --target-dir target\b2 -p cryo_cooler_controller`
Expected: 30 passed

---

## Fase 5: La guardia di HWiNFO va prima della lettura

**File:** `cryo_cooler_controller/src/hwinfo.rs:233-249`

- [ ] **Step 1: Scrivi il test**

```rust
    /// La guardia di dimensione deve valere PRIMA di toccare la memoria
    /// condivisa. Prima si leggevano sei `u32` e solo dopo si controllava
    /// `avail >= 40`: con una mappatura piu' piccola la lettura era gia'
    /// avvenuta fuori dai limiti.
    #[test]
    fn mapping_troppo_piccolo_non_produce_nessun_valore() {
        assert!(read_sensors_from(0, base()).is_empty(),
                "ha letto da un mapping di 0 byte");
    }
```

- [ ] **Step 2: Sposta la guardia in cima**

```rust
    // PRIMA di qualsiasi lettura. `VirtualQuery` puo' fallire e restituire 0,
    // e un mapping puo' essere piu' piccolo dell'intestazione: in entrambi i
    // casi i sei `read_unaligned` qui sotto avrebbero letto oltre la fine.
    let size = match unsafe { VirtualQuery(base as *const _, std::ptr::null_mut(), &mut mq) } {
        0 => { let _ = unsafe { UnmapViewOfFile(base) }; CloseHandle(h); return Vec::new(); }
        _ => mq.RegionSize,
    };
    if size < 40 {
        let _ = unsafe { UnmapViewOfFile(base) };
        CloseHandle(h);
        return Vec::new();
    }
```

Rimuovi il controllo `header_ok` ora duplicato più in basso, o sostituiscilo con `true` commentando che la guardia è già passata.

Run: `cmd //c b2.bat && cargo test --target-dir target\b2 -p cryo_cooler_controller`
Expected: 31 passed

---

## Fase 6: Un sensore che non risponde non può accendere la guardia

**File:** `cryo_cooler_controller/src/running.rs:531-542`

- [ ] **Step 1: Scrivi il test**

```rust
    /// Il firmware espone gia' `BOARD_TEMP_OK` e `TEMP_SENSE_OK`. Se il
    /// sensore e' guasto la guardia restava inerte per sempre, **senza
    /// ridurre la potenza**: il caso peggiore, una protezione che
    /// sparisce in silenzio proprio quando servirebbe.
    #[test]
    fn sensore_guasto_abbassa_il_cap() {
        let s = Costrutto::di_test();
        s.tec_status = TecStatus::BOARD_INIT;   // nessun *_TEMP_OK
        s.applied_power = 100;
        s.applica_una_ guardia();
        assert!(s.applied_power < 100, "guardia inerte con sensore guasto");
    }
```

- [ ] **Step 2: Rendi la guardia fail-closed**

```rust
        // Sensore assente o fuori range: non si torna indietro senza motivo.
        // Un NTC scollegato o guasto non e' "nessun dato", e' "non lo so
        // e il modulo e' acceso": la risposta corretta e' togliere potenza,
        // non continuare come se nulla fosse.
        if !ctrl_temp.is_finite() || !(-40.0..=150.0).contains(&ctrl_temp) {
            self.power_push_failures = self.power_push_failures.wrapping_add(1);
            crate::commissioning::event("CTRL-LET", &format!(
                "lettura fuori range ({ctrl_temp:.1} °C), potenza ridotta"));
            self.applica_potenza(0);
            return;
        }

        // Stessa logica per il bit di stato: il firmware sa se il sensore
        // e' vivo, e noi non lo stavamo ascoltando.
        if !self.tec_status.contains(TecStatus::BOARD_TEMP_OK) {
            self.applica_potenza(0);
            return;
        }
```

Run: `cmd //c b2.bat && cargo test --target-dir target\b2 -p cryo_cooler_controller`
Expected: 33 passed

---

## Fase 7: F11 deve funzionare anche a regime

**File:** `cryo_cooler_controller/src/main.rs:581, 679-701, 1136-1142, 1245-1251`

**Problema:** il gestore di `ToggleFullscreen` esiste solo in `HomeState`. Passando allo stato `Running` il tasto non fa nulla e il flag viene distrutto, quindi da schermo intero **non si torna indietro** se non con Alt+F4.

- [ ] **Step 1: Sposta lo stato fuori da `HomeState`**

```rust
/// Stato condiviso fra le schermate.
///
/// F11 deve funzionare ovunque: se il flag vivesse solo nella schermata di
/// collegamento, entering the dashboard lo azzererebbe e dall'interno del
/// fullscreen non ci si tornerebbe piu' fuori.
#[derive(Default)]
pub struct Ui {
    pub fullscreen: bool,
}
```

- [ ] **Step 2: Passa `Ui` a entrambi gli stati e instrada il messaggio**

```rust
pub struct CryoCoolerController {
    pub ui: Ui,
    // ...
}
```

```rust
            // Aggiunto nello stato Running, accanto agli altri match:
            Message::ToggleFullscreen => {
                self.ui.fullscreen = !self.ui.fullscreen;
                let modalita = if self.ui.fullscreen {
                    iced::window::Mode::Fullscreen
                } else {
                    iced::window::Mode::Windowed
                };
                iced::window::get_latest().then(move |id| match id {
                    Some(id) => iced::window::change_mode(id, modalita),
                    None => Task::none(),
                })
            }
```

- [ ] **Step 3: Riallinea il flag quando il tray forza la finestra**

```rust
    // Il tray forza la finestra: il flag deve seguire, o il tasto successivo
    // ripeterebbe un'operazione gia' fatta e sembrerebbe non funzionare.
    self.ui.fullscreen = false;
```

Run: `cmd //c b2.bat && cargo test --target-dir target\b2 -p cryo_cooler_controller`
Expected: 33 passed

---

## Fase 8: La firma RTSS era invertita

**File:** `cryo_cooler_controller/src/overlay.rs:47`

- [ ] **Step 1: Correggi la costante e spiega l'errore**

```rust
/// Firma della memoria condivisa RTSS.
///
/// RTSS scrive il letterale multicarattere di MSVC `(DWORD)'RTSS'`, che
/// mette `'R'` nel byte **basso**: in little-endian la sequenza di byte e'
/// `52 54 53 53` e il valore letto vale `0x53535452`.
///
/// La costante era `0x52545353`, che in little-endian si legge come i byte
/// `53 53 54 52` — cioe' "SSTR". Nessuna interpretazione di "RTSS" produce
/// quella sequenza: era il letterale scritto nell'ordine in cui le lettere
/// si leggono a schermo, senza l'inversione di endianness. Il confronto non
/// e' mai stato vero, e la sovrapposizione non si e' mai attivata, con un
/// messaggio che accusava il layout invece che la costante.
const RTSS_FIRMA: u32 = 0x5353_5452;
```

- [ ] **Step 2: Usa il nome nuovo e correggi il commento**

Sostituisci ogni `RTSS_SIGNATURE` con `RTSS_FIRMA` e correggi la riga 24.

- [ ] **Step 3: Il controllo dei limiti deve guardare la vista, non la regione**

```rust
    // `MapViewOfFile(h, FILE_MAP_ALL_ACCESS, 0, 0, 0)` mappa tutta la
    // sezione, e `VirtualQuery` restituisce la dimensione della **regione**,
    // che e' la vista arrotondata per pagina: `hi` puo' superare la fine
    // della vista fino a 4095 byte. Un puntatore vicino alla fine passerebbe
    // il controllo e la copia dei 0x400 byte uscirebbe dalla mappatura.
    let size = ...;   // RegionSize
    let view_size = size.min(4096 * ((size + 4095) / 4096));  // vedi nota
```

Se `VirtualQuery` non dà la dimensione della vista, usa la somma degli
offset dichiarati nella struttura invece di `RegionSize`

```rust
    // Somma degli offset reali dei campi che si va a scrivere: e' l'unica
    // stima che non possa sforare la mappatura.
    let hi = (pOsmEntries as usize)
        .wrapping_add(std::mem::size_of::<OSM_ENTRY>() * 512);
```

Run: `cmd //c b2.bat && cargo test --target-dir target\b2 -p cryo_cooler_controller`
Expected: 33 passed

---

## Fase 9: Cooldown coerenti col tempo reale

**File:** `cryo_cooler_controller/src/running.rs:44, 53, 813`

- [ ] **Step 1: Raddoppia la frequenza di controllo**

```rust
    /// Controllo alert ogni 4 tick. A 2 Hz sono 2 s, e `ALERT_CHECK_HZ`
    /// deve valere 0.5 perche' i cooldown escano in secondi reali.
    ///
    /// Era 8: i controlli passavano ogni 4 s ma il moltiplicatore diceva 0.5,
    /// cosi' ogni cooldown durava il doppio — quello da 600 s durava 1200 s,
    /// e la regola della pompa che dichiara "max 1 ogni 5 minuti" durava 10.
    const ALERT_EVERY_TICKS: u32 = 4;
```

- [ ] **Step 2: Correggi i due intervalli documentati male**

```rust
    const CPU_TEMP_PUSH_INTERVAL: u32 = 10;   // 5 s a 2 Hz, non 10
    const POWER_LEVEL_PUSH_INTERVAL: u32 = 10; // 5 s a 2 Hz, non 10
```

Run: `cmd //c b2.bat && cargo test --target-dir target\b2 -p cryo_cooler_controller`
Expected: 33 passed

---

## Fase 10: Non perdere dati in silenzio

**File:** `cryo_cooler_controller/src/config.rs:120-151`, `session_db.rs:111-136`, `running.rs:326-330`

- [ ] **Step 1: Un config corrotto non deve cancellare i profili**

```rust
    pub fn load() -> Self {
        let path = Self::path();
        let testo = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => return Self::predefinita(),
        };
        match serde_json::from_str::<AppConfig>(&testo) {
            Ok(c) => c,
            Err(e) => {
                // Il file originale non e' sovrascrivibile: senza questo
                // salvataggio, il `save` successivo avrebbe scritto i
                // valori predefiniti sopra l'unica copia dei profili
                // dell'utente, rendendola irrecoverabile.
                let backup = path.with_extension("corrotto.json");
                let _ = std::fs::copy(&path, &backup);
                eprintln!("config.json illeggibile ({e}), copia in {backup:?}");
                Self::predefinita()
            }
        }
    }
```

- [ ] **Step 2: Il database fallito deve dirlo**

```rust
            // `is_open()` esisteva e non veniva mai chiamato: un database
            // illeggibile o un disco pieno facevano perdere una sessione
            // intera senza alcun indizio.
            session_db: {
                let mut db = SessionDb::open();
                if !db.is_open() {
                    eprintln!("session_db: storico non disponibile");
                }
                db.start_session("Sessione");
                db
            },
```

- [ ] **Step 3: Il VACUUM non va sul thread della UI**

```rust
    /// Potatura delle sessioni vecchie, **fuori** dal percorso di avvio.
    ///
    /// `VACUUM` riscrive l'intero file e chiede circa il doppio dello
    /// spazio libero: dentro `new()` bloccava la finestra per secondi a ogni
    /// avvio, e cresceva con il database. Non e' una condizione frequente:
    /// conviene pagarla solo quando si verifica davvero.
    pub fn pota_se_necessario(&mut self) {
        if self.conta_sessioni() <= 50 { return; }
        self.prune_old_sessions(50);
    }
```

Run: `cmd //c b2.bat && cargo test --target-dir target\b2 -p cryo_cooler_controller`
Expected: 33 passed

---

## Fase 11: La sonda degli opcode non deve poter resettare la scheda

**File:** `cryo_cooler_controller_lib/src/lib.rs:79-104`

**Problema:** la documentazione afferma che si inviano "solo letture con dati
nulli, che non possono scrivere un valore nuovo". È falso: **`0x1E` con
`[0;4]` è il reset di fabbrica** e **`0x18` con `[0;4]` è l'accensione**. La
funzione è `pub` e non ha né allowlist né interlock.

- [ ] **Step 1: Limitarla a una allowlist esplicita**

```rust
pub fn probe_opcodes(tec: &mut Tec, da: u8, a: u8) -> Vec<(u8, Result<Vec<u8>, String>)> {
    // **Solo opcode di lettura.** La sonda non e' un coltellino a molta
    // punta, ed e' il punto in cui un errore costa la scheda.
    //
    // Con dati nulli, 0x18 ACCENDE il TEC e 0x1E fa il RESET DI FABBRICA:
    // la versione precedente li spazzava via e la sua stessa
    // documentazione lo dichiarava impossibile.
    const SOLO_LETTURE: std::ops::RangeInclusive<u8> = 0x14..=0x17;

    let mut esiti = Vec::new();
    for op in da..=a {
        if !SOLO_LETTURE.contains(&op) { continue; }
        // ...
    }
    esiti
}
```

- [ ] **Step 2: Documentare che va eseguita a TEC spento**

Nel doc comment della funzione, in modo non negoziabile: «Chiamare solo con il
TEC disabilitato e senza carico. La funzione non verifica lo stato e non
proteggere il modulo.»

Run: `cmd //c b2.bat && cargo test --target-dir target\b2 -p cryo_cooler_controller`
Expected: 33 passed

---

## Fase 12: Colori semantici per le categorie

**File:** `cryo_cooler_controller/src/main.rs` (modulo `palette`), `running.rs`

**Vincolo onesto:** le 8 icone esistenti sono **illustrazioni già a colori**
(media RGB 71/119/147, canale R variato di 234), non maschere monocrome. Non
posso tingerle senza rovinarle, e non posso generare artwork nuovo di qualità
da codice. Quello che è invece alla mia portata ed è ciò che l'utente ha
chiesto in modo utile: **un colore semantico stabile per ogni categoria**
così che la dashboard si legga a colori senza dover decifrare ogni icona.

- [ ] **Step 1: Aggiungi i token al palette**

```rust
    // Colori semantici: uno per categoria, mai riutilizzato. Servono perche'
    // a 16 px un'illustrazione dettagliata non e' leggibile, e il colore e'
    // l'unico segnale che resta.
    pub const SEM_TEMPERATURA: Color = Color { r: 0.20, g: 0.80, b: 0.95, a: 1.0 }; // ciano
    pub const SEM_POTENZA:     Color = Color { r: 1.00, g: 0.70, b: 0.10, a: 1.0 }; // ambra
    pub const SEM_CIRCOLAZIONE:Color = Color { r: 0.55, g: 0.45, b: 1.00, a: 1.0 }; // viola
    pub const SEM_PROTEZIONE:  Color = Color { r: 0.10, g: 0.90, b: 0.55, a: 1.0 }; // verde
    pub const SEM_CARICO:      Color = Color { r: 0.30, g: 0.90, b: 0.80, a: 1.0 }; // verde- acqua
    pub const SEM_Rischio:     Color = Color { r: 1.00, g: 0.30, b: 0.35, a: 1.0 }; // rosso
```

- [ ] **Step 2: Associa ogni intestazione di sezione al suo colore**

```rust
    /// Pastiglia di categoria: 8 px di colore pieno con il nome della sezione.
    ///
    /// Non decora: da' alla sezione un'identita' che si riconosce senza
    /// leggere, ed e' l'unico accenno di colore che funziona a 16 px.
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

Usa `intestazione_sezione` con: `SEM_TEMPERATURA` per le temperature
`SEM_POTENZA` per potenza e tensione, `SEM_CIRCOLAZIONE` per pompa e
condensazione, `SEM_PROTEZIONE` per allarmi e watchdog, `SEM_CARICO` per
carico e profili.

- [ ] **Step 3: Le notifiche usano già i token**

`Gravita::colore()` in `running.rs` va fatto puntare a `SEM_RISHIO`
`SEM_POTENZA`, `SEM_CIRCOLAZIONE` invece dei grezzi, così avvisi e intestazioni
condividono lo stesso vocabolario di colori.

Run: `cmd //c b2.bat && cargo test --target-dir target\b2 -p cryo_cooler_controller`
Expected: 33 passed

---

## Fase 13: Collaudo e pubblicazione

- [ ] **Step 1: Verifica completa**

```bash
cd "<TMP>" && cmd //c b2.bat
cd "<REPO>"
call "C:\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
cargo test --target-dir target\b2 -p cryo_cooler_controller
```

Expected: 0 errori, 0 warning, tutti i test verdi.

- [ ] **Step 2: Build release in una directory nuova**

```bash
cd "<TMP>"
sed 's/r10/r28/' rel10.bat > rel28.bat
cmd //c rel28.bat
```

- [ ] **Step 3: Pubblica senza terminare il processo dell'utente**

```powershell
# deploy_attesa.ps1: aspetta che l'utente chiuda l'app, poi copia.
$src = '<REPO>\target\r28\release\cryo_cooler_controller.exe'
$dst = '<ESEGUIBILE>'
while (Get-Process -Name 'cryo_cooler_controller' -ErrorAction SilentlyContinue) {
    Start-Sleep -Seconds 2
}
Copy-Item -Path $src -Destination $dst -Force
(Get-FileHash $dst -Algorithm MD5).Hash
```

Expected: l'hash del file pubblicato coincide con quello di r28.

- [ ] **Step 4: Collaudo con log**

```bash
CRYO_COMMISSIONING=1
```

Verifica che il log registri, nell'ordine: la discesa per la guardia termica
il comportamento del limitatore pompa, e l'eventuale ripristino. Un
collaudo che **non** produce la discesa a 5 °C/min significa che la nuova
firma lenta non sta vedendo i campioni: controlla `campioni_lenti`.

- [ ] **Step 5: Misura la GPU**

Scheda **AMD Radeon RX 7900 XTX**: `nvidia-smi` non serve. Usare Gestione
Attività → Dettagli → colonna GPU, con l'app aperta da almeno un minuto.
Prima delle correzioni la misura era 50% **su r24**, che non conteneva nessuna
delle fix.

---

## Autoverifica del piano

**Copertura.** Ogni finding delle quattro revisioni ha un task: 1.1 → Fase 1;
1.2, 1.4, 1.5 → Fasi 2 e 3; 1.3 → Fase 3; 1.7, 1.8 → Fase 6; 1.9 → Fase 1;
2.1, 2.2 → Fase 2; 3.1, 3.2 → Fasi 2 e 9; 4.1 → Fase 6; 5.1 → Fuori portata
(miglioramento di rendering, non un difetto); 7.1 → Fase 5; 7.2 → Fase 5;
C1, C2, H2, H3, H4 → Fase 11 e Fase 6; 1.1 (shutdown) → Fase 1;
3.1, 3.2 (F11) → Fase 7; 4.1, 4.2 (RTSS) → Fase 8; 2.1 (env var) → Fase 10;
5.2, 5.3, 5.4, 5.5 → Fase 10; 6.1, 6.2, 6.3 → **non coperti**.

**Lacune dichiarate.** `autostart.rs` (percorso senza spazi non quotato
riparazione che disattiva l'avvio automatico) e il blocco senza timeout sulla
pipe Discord restano **fuori** da questo piano: sono difetti reali, ma nessuno
dei due può danneggiare l'hardware, e il secondo è inattivo finché il
`client_id` è un segnaposto. Vanno in un piano separato.

**Nessun segnaposto.** Ogni passo contiene il codice o il comando esatto. I
test citano `Costrutto::di_test()`: va creato come parte della Fase 1 come
costruttore di test in `running.rs`, con un `impl` separato dietro
`#[cfg(test)]`.

**Coerenza dei tipi.** `applica_potenza(&mut self, u8) -> bool`
`pompa_in_fallo(bool, f32, bool) -> bool`
`tec_abilitato_software(&self) -> bool`, `Gravita::{Critica, Avviso, Info}`
`Regime::Invalido`, `SEM_*` sono definiti una volta sola e usati con la stessa
firma ovunque.
