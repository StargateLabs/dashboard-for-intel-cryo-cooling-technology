# Changelog

Ogni voce dichiara esplicitamente cosa **non** è stato dimostrato.

Il versioning è diviso in due serie:

| Serie | Periodo | Scopo |
|---|---|---|
| `r81` → `r147` | 28–30 settembre 2026 | sviluppo giornaliero sul controller Intel Cryo Gen 1, senza celle TEC |
| `R1` → `R15` | 30 settembre – 2 ottobre 2026 | collaudo su hardware reale con cella TEC montata |

I numeri di versione pubblicati su GitHub (`v2.5`, `v2.6`, `v2.7`) corrispondono alle release
R13, R14 e R15.

---

## Linea evolutiva

| Data | Build | Intervento |
|---|---|---|
| 28/09 00:29 | `r81` | riferimento di partenza, conservato come backup funzionante |
| 28–30/09 | `r113` → `r147` | 31 build giornaliere, tutti della stessa base. Le variazioni principali sono nella serie R |
| 30/09 06:03 | `TEC-20260930` | primo avvio con cella TEC montata |
| 30/09 06:21 | `GEN1-TEC2` | prima build del binario attuale |
| 30/09 06:24 | `FINALE` | |
| 30/09 06:33 | `FINALE-R2` | secondo tentativo della build `FINALE`, **non** la R2 della serie |
| 30/09 06:45 | `PROFILI-R3` | tre profili di carico |
| 30/09 06:56 | `GRAFICA-R4` | grafici, animazione a 30 fps |
| 30/09 07:09 | `GOCCE-R5` | indicatore di condensa |
| 30/09 07:16 | `DESIGN-R6` | superfici e contrasto |
| 30/09 07:42 | `R7-TERMICA-CANDIDATA` | correzioni termiche e ordine dei comandi |
| 30/09 07:56 | `R8` | diagnostica live |
| 30/09 08:05 | `R9` | prima misura del carico GPU |
| 30/09 08:13–08:23 | `R10`, `R11` | riduzione del carico GPU |
| 30/09 08:32 | `R12` | pulsante TEC |
| 30/09 15:14 | `R13` | continuità del controllo, supervisore di riavvio |
| 02/10 16:46 | `R14` | recupero della connessione dopo perdita di heartbeat |
| 02/10 17:10 | `R15` | soglie termiche dell'impianto e coda seriale |

Le date sono quelle dei file in `build-verificata`. Le build senza nota di collaudo non hanno
descrizione del contenuto: il nome del file è l'unico dato disponibile.

### Versione di riferimento

Le note di R14 e R15 indicano come versione di riferimento
`04495C1701434E96D927F2EE42A34A6F996E6F6B1F2E8C7CDE3D54B7D240F71E`, del 29 settembre 2026.
Corrisponde al file `cryo_cooler_controller_r116.exe`.

---

## Release pubblicate

Tutte le 18 build della cartella di collaudo sono pubblicate su GitHub, con l'eseguibile come
asset e il checksum nelle note.

### Serie v2 — da R13 in poi

| Tag | Build | Esiguibile | Data | SHA256 |
|---|---|---|---|---|
| [`v2.7`](https://github.com/StargateLabs/dashboard-for-intel-cryo-cooling-technology/releases/tag/v2.7) | R15 | `StargateCryo-GEN1-TEC2-R15.exe` | 02/10 17:10 | `1d6497af…70b481` |
| [`v2.6`](https://github.com/StargateLabs/dashboard-for-intel-cryo-cooling-technology/releases/tag/v2.6) | R14 | `StargateCryo-GEN1-TEC2-R14.exe` | 02/10 16:46 | `d3969a6d…025388` |
| [`v2.5`](https://github.com/StargateLabs/dashboard-for-intel-cryo-cooling-technology/releases/tag/v2.5) | R13 | `StargateCryo-GEN1-TEC2-R13.exe` | 30/09 15:14 | `3f9460cd…fe0b84` |

### Serie v1 — dalla prima build con TEC a R12

| Tag | Build | Eseguibile | Data | Collaudo |
|---|---|---|---|---|
| [`v1.14`](https://github.com/StargateLabs/dashboard-for-intel-cryo-cooling-technology/releases/tag/v1.14) | R12 | `StargateCryo-GEN1-TEC2-R12.exe` | 30/09 08:38 | 311 test |
| [`v1.13`](https://github.com/StargateLabs/dashboard-for-intel-cryo-cooling-technology/releases/tag/v1.13) | R11, doppio nome | `StargateCryo-Condensa-Preview.exe` | 30/09 08:21 | 311 test |
| [`v1.12`](https://github.com/StargateLabs/dashboard-for-intel-cryo-cooling-technology/releases/tag/v1.12) | R11 | `StargateCryo-GEN1-TEC2-R11.exe` | 30/09 08:21 | 311 test |
| [`v1.11`](https://github.com/StargateLabs/dashboard-for-intel-cryo-cooling-technology/releases/tag/v1.11) | R10 | `StargateCryo-GEN1-TEC2-R10.exe` | 30/09 08:13 | 310 test |
| [`v1.10`](https://github.com/StargateLabs/dashboard-for-intel-cryo-cooling-technology/releases/tag/v1.10) | R9 | `StargateCryo-GEN1-TEC2-R9.exe` | 30/09 08:05 | log, senza nota |
| [`v1.9`](https://github.com/StargateLabs/dashboard-for-intel-cryo-cooling-technology/releases/tag/v1.9) | R8 | `StargateCryo-GEN1-TEC2-R8.exe` | 30/09 07:56 | log, senza nota |
| [`v1.8`](https://github.com/StargateLabs/dashboard-for-intel-cryo-cooling-technology/releases/tag/v1.8) | R7 | `StargateCryo-GEN1-TEC2-R7-TERMICA-CANDIDATA.exe` | 30/09 07:42 | log, senza nota |
| [`v1.7`](https://github.com/StargateLabs/dashboard-for-intel-cryo-cooling-technology/releases/tag/v1.7) | R6 | `StargateCryo-GEN1-TEC2-DESIGN-R6.exe` | 30/09 07:16 | log, senza nota |
| [`v1.6`](https://github.com/StargateLabs/dashboard-for-intel-cryo-cooling-technology/releases/tag/v1.6) | R5 | `StargateCryo-GEN1-TEC2-GOCCE-R5.exe` | 30/09 07:09 | log, senza nota |
| [`v1.5`](https://github.com/StargateLabs/dashboard-for-intel-cryo-cooling-technology/releases/tag/v1.5) | R4 | `StargateCryo-GEN1-TEC2-GRAFICA-R4.exe` | 30/09 07:01 | log, senza nota |
| [`v1.4`](https://github.com/StargateLabs/dashboard-for-intel-cryo-cooling-technology/releases/tag/v1.4) | R3 | `StargateCryo-GEN1-TEC2-PROFILI-R3.exe` | 30/09 06:45 | log, senza nota |
| [`v1.3`](https://github.com/StargateLabs/dashboard-for-intel-cryo-cooling-technology/releases/tag/v1.3) | — | `StargateCryo-GEN1-TEC2-FINALE-R2.exe` | 30/09 06:33 | nessuna nota |
| [`v1.2`](https://github.com/StargateLabs/dashboard-for-intel-cryo-cooling-technology/releases/tag/v1.2) | — | `StargateCryo-GEN1-TEC2-FINALE.exe` | 30/09 06:24 | nessuna nota |
| [`v1.1`](https://github.com/StargateLabs/dashboard-for-intel-cryo-cooling-technology/releases/tag/v1.1) | — | `StargateCryo-GEN1-TEC2.exe` | 30/09 06:21 | nessuna nota |
| [`v1.0`](https://github.com/StargateLabs/dashboard-for-intel-cryo-cooling-technology/releases/tag/v1.0) | — | `StargateCryo-TEC-20260930.exe` | 30/09 06:03 | nessuna nota |

**`v1.13` e `v1.12` sono lo stesso file.** `StargateCryo-Condensa-Preview.exe` e
`StargateCryo-GEN1-TEC2-R11.exe` hanno SHA256 identico: è lo stesso eseguibile con due nomi, non
due versioni. Chi scarica l'anteprima della condensa scarica R11.

**Le prime nove build non hanno una nota di collaudo.** Esistono i log di compilazione da R3 in
poi, e le note `VERIFICA-RICHIESTE` da R10 in poi. Per le altre sette la data è l'unico dato
disponibile e le note di rilascio lo dichiarano.

La numerazione della serie parte da `PROFILI-R3`, che è il primo eseguibile con un numero R.
**R1 e R2 non hanno un eseguibile**: per R1 e R2 esiste solo la nota di lavoro in
`docs/releases/`. Di conseguenza, il `R2` nel nome del file `FINALE-R2.exe` è un secondo
tentativo della build `FINALE` e **non** la R2 della serie.

La serie `r81` → `r147`, le quattro build con nome (`TEC-20260930`, `GEN1-TEC2`, `FINALE`,
`FINALE-R2`) e la serie R1–R2 restano in locale senza un proprio asset di rilascio.

---

## r81 – r147: sviluppo giornaliero senza cella TEC (28–30 settembre 2026)

Prima della serie R. Trentuno build giornaliere sul controller Intel Cryo Gen 1 `HW 4`,
firmware `13.A0`, senza cella TEC montata.

| Build | Data | Byte |
|---|---|---|
| `r81` | 28/09 00:29 | 24.812.544 |
| `r113` | 29/09 00:38 | 25.318.400 |
| `r115` | 29/09 03:07 | 25.334.784 |
| `r116` | 29/09 03:43 | 25.336.320 |
| `r117` – `r132` | 29/09 18:12 – 30/09 00:05 | 25.267.224 – 25.337.536 |
| `r133` – `r147` | 30/09 00:05 – 05:47 | 25.325.568 – 25.337.536 |

`r81` è conservata come backup funzionante ed è il riferimento da cui parte il lavoro sulla
cella TEC. `r116` è la versione usata come riferimento nelle note di R14 e R15.

**Non dimostrato:** per questa serie non esiste una nota di collaudo. Le date e le dimensioni
sono quelle dei file, ma non c'è un resoconto delle modifiche né una misura di stabilità
per singola build.

---

## Le prime build con cella TEC (30 settembre 2026)

La cella TEC V2 viene montata sul controller. Queste quattro build non hanno una nota di
collaudo: esistono come file e come data.

| Build | Data | Byte | Nome |
|---|---|---|---|
| 1 | 06:03 | 25.330.176 | `StargateCryo-TEC-20260930.exe` |
| 2 | 06:21 | 25.335.296 | `StargateCryo-GEN1-TEC2.exe` |
| 3 | 06:24 | 25.335.808 | `StargateCryo-GEN1-TEC2-FINALE.exe` |
| 4 | 06:33 | 25.334.272 | `StargateCryo-GEN1-TEC2-FINALE-R2.exe` |

**Non dimostrato:** nessuna di queste quattro build ha una nota che descriva le modifiche o i
test eseguiti. La numerazione parte dalla build 5, `PROFILI-R3`, che è la prima con
documentazione.

---

## R15: Soglie termiche dell'impianto e coda seriale

→ nota completa in [`docs/releases/R15-soglie-impianto.md`](docs/releases/R15-soglie-impianto.md)

Soglie per l'impianto modificato: abilitazione consentita sotto 37 °C di PCB, bloccata a 37 °C,
richiesta di DISABLE reale a 38 °C con priorità nella coda seriale. Sensori `NaN`, infiniti e fuori
intervallo rifiutati prima del controllo. Conferme di Cryo e Unregulated abbinate all'offset
richiesto. Offset -30..50 °C, PID 0..1000, percentuale 0..100.

320 test superati: 297 applicazione e 23 libreria. Cinque prove che toccano hardware o database
reale restano escluse.

Le soglie sono scelte per questo impianto e non sono certificazioni del costruttore.

---

## R14: Recupero della connessione dopo perdita di heartbeat

→ nota completa in [`docs/releases/R14-recupero-uscita.md`](docs/releases/R14-recupero-uscita.md)

Dopo tre round falliti un unico recupero riporta alla schermata di connessione, tenta lo
spegnimento con attesa limitata, rilascia la vecchia porta seriale e ripete il rilevamento USB.
Una sola scansione per volta, con pausa di 2 secondi e massimo 20 tentativi. L'errore riporta
opcode e causa originale. Nessun reset di fabbrica `0x1E` automatico.

315 test superati: 295 applicazione e 20 libreria.

Il recupero su controller fisico dopo una vera perdita di alimentazione non è stato provato.

---

## Basi di libreria per R14 e R15

Questo lavoro di rifatturizzazione **non ha un numero di release proprio**: è stato applicato
fra la costruzione di R14 e quella di R15.

| Ora del 02/10 | Evento |
|---|---|
| 16:44 | modifica di `main.rs` |
| 16:46 | **build di R14** |
| 16:54 | modifica di `recovery.rs` |
| 16:59 | modifica di `attore_tec.rs` |
| 17:08 | modifica di `running.rs` e `lib.rs` |
| 17:10 | **build di R15** |

**R14 non contiene questo lavoro**: quando è stato compilato, tre dei cinque file non erano
ancora stati modificati. **R15 lo contiene interamente**, perché è stato compilato dopo
l'ultima modifica.

**320 test passati** (297 applicazione, 23 libreria), 0 falliti, 5 ignorati. Build release
completata senza errori.

### Soglie sul controller

Due soglie esplicite nella libreria, al posto dei valori sparsi nel codice applicativo:

| Costante | Valore | Significato |
|---|---|---|
| `BOARD_REENABLE_TEMP` | 37,0 °C | sopra questa temperatura il TEC **non può essere acceso** |
| `CRITICAL_BOARD_TEMP` | 38,0 °C | a questa temperatura il TEC viene **spento** |

Entrambe accettano il sensore come unico criterio, ma il codice distingue due casi diversi:

- `board_allows_enable(board)` blocca l'accensione se la temperatura è sopra soglia **oppure**
  se il sensore non dà un numero finito. Una lettura `NaN`, infinita o fuori scala impedisce
  l'avvio, perché non è distinguibile da una temperatura pericolosa.
- `critical_board_shutdown(board)` spegne il modulo a 38 °C. È l'unico caso in cui il programma
  **spegge da solo** senza aspettare una conferma dell'operatore.

Il commento nel codice ricorda che 38 °C è un **limite di installazione configurato**, non una
classificazione del produttore valida per altri controller.

### Salute della telemetria

`telemetry_healthy()` richiede due condizioni insieme: nessun fallimento di monitoraggio
accumulato **e** un campione valido ricevuto meno di 3 secondi fa. È il segnale che il watchdog
usa per capire se i dati che sta proteggendo sono ancora freschi.

### Connessione non piu` parallela

Le richieste di rilevamento della porta non avviano piu' scansioni in contemporanea:
`AUTO_CONNECT_MAX_TRIES` limita i tentativi a 20, le richieste entrano in una coda e il
lavoro ha un timeout di 300 ms. Il test `repeated_detection_requests_do_not_start_parallel_scans`
copre il caso.

### Validazione degli ingressi

`validate_finite` e `validate_offset` rifiutano `NaN` e infinito prima che raggiungano il
controller. Un offset non finito arriverebbe al protocollo come un `float32` senza significato.

### Stato della richiesta esplicito

`cooling_requested()` e `matches_mode()` espongono come **intenzione registrata** ciò che
l'operatore ha chiesto, separato dallo stato che il controller ha confermato. La dashboard
continua a non mostrare un regime dedotto come se fosse un fatto.

### Riduzione di codice

Cinque file toccati, **275 righe aggiunte e 413 rimosse**: 138 righe in meno di netto. La riduzione
concentrata in `running.rs`, dove le soglie termiche passano da logica sparsa a chiamate a
funzioni testabili. Le nuove funzioni hanno test dedicati, quindi la logica che prima era
verificata solo a mano è ora coperta dalla suite.

### Non dimostrato

- Le soglie 37 °C e 38 °C sono **scelte per questa installazione**, non valori del produttore.
  Non sono confrontate con un riferimento termico esterno.
- `telemetry_healthy()` non è stata verificata con il controller realmente scollegato: il
  comportamento è provato da test che simulano l'assenza di campioni, non da un'interruzione
  fisica del collegamento.
- La coda di connessione non è stata provata sotto carico con porte che spariscono e
  ricompaiono.

---

## R13: Continuità del controllo TEC

- La `X` nasconde la dashboard nel tray: **non termina** il controllo.
- Menu tray con "Esci e arresta il controllo" come unica uscita definitiva.
- **Supervisore** nello stesso eseguibile: non apre COM, non interroga il
  controller. Attende la fine del processo dashboard; un arresto inatteso, anche
  con codice zero, causa riavvio dopo 2 s. Arresti ripetuti → attesa progressiva
  fino a 30 s.
- Dopo un crash riattiva **solo Cryo**, e solo se in quella sessione era stato
  richiesto raffreddamento attivo. **Non** ripristina Unregulated.
- Mutex esistente: una seconda dashboard viene respinta, senza ciclo di riavvii.
- Panic ed errori UI in `LOCALAPPDATA/stargate-cryo/recovery.log`, rotazione 1 MB.
- Icone condivise con `OnceLock`: identificativo GPU stabile tra i fotogrammi
  (Windows aveva registrato `RADAR_PRE_LEAK_64` per R11).

**Validazione**: 314 test (294 applicazione, 20 libreria). Collaudo reale del
supervisore: primo figlio `exit 70` → riavvio dopo 2 s → secondo figlio recupera
l'intento abilitato → uscita volontaria `0`. Nessuna porta COM aperta.
**Non dimostrato**: il motivo della chiusura precedente; la continuità reale del
recupero TEC dopo crash sulla macchina sotto raffreddamento.

→ [`docs/releases/R13-continuita-controllo.md`](docs/releases/R13-continuita-controllo.md)

---

## R12: Pulsante TEC

- Il pulsante TEC acquista icona dedicata, titolo, descrizione, potenza attuale e
  grafico incorporato. Fondo sfumato, bordi arrotondati, risposta al passaggio del
  mouse. Nessuna animazione continua.
- Messaggi di abilitazione/disabilitazione e comandi Cryo/Unregulated invariati.

**Validazione**: 311 test. **Non dimostrato**: il restyling non dimostra una
riduzione del consumo GPU.

→ [`docs/releases/R12-pulsante-tec.md`](docs/releases/R12-pulsante-tec.md)

---

## R11: Fluidità a 30 fps

- Animazione ~30 fps anche senza focus; spenta in Home e quando nascosta nel tray.
- Ritardo visivo delle curve portato a **1000 ms** con interpolazione tra campioni
  realmente disponibili. Valori live, controlli, allarmi e log **non** ritardati.
- Campionamento CPU per il grafico ogni 1 s; invio al controller invariato.
- Multisampling 4x disattivato; testo e filtro immagini mantenuti.

**Validazione**: 311 test.
**Misure GPU** (10 campioni, [`dati-gpu-r11.csv`](docs/releases/dati-gpu-r11.csv)):
media **12.406 %**, min 11.121 %, max 13.504 %. La fluidità percepita resta da
confermare con l'utente.

→ [`docs/releases/R11-fluidita-30fps.md`](docs/releases/R11-fluidita-30fps.md) ·
[`dati-gpu-r9-prima.csv`](docs/releases/dati-gpu-r9-prima.csv) (12,7–15,0 %) ·
[`dati-gpu-r10.csv`](docs/releases/dati-gpu-r10.csv) (1,2–1,8 %, non confrontabile)

---

## R10: Risposta al sovraccarico GPU

- Stop dell'animazione extra fuori focus, in Home e nel tray; cache 66 ms;
  eliminato il MSAA 4x della finestra.
- **BLOCCO D1 corretto**: `Tec::new()` non emette più `0x1E`. Prima, a ogni
  connessione perdevi PID, setpoint e **power cap**.
- Feedback sui watt reali; la percentuale firmware non è trattata come limite fisico.

**Validazione**: 310 test, build release riuscita.
**Non dimostrato**: nessuna garanzia di minimo globale watt/volt/ampere; la prova
del raffreddamento a 90 °C non è stata riprodotta.

→ [`docs/releases/R10-gpu-fluidita.md`](docs/releases/R10-gpu-fluidita.md)

---

## R9: Prima misura di sovraccarico GPU

Prima misura del carico GPU dell'applicazione: **12,7–15,0 %** su cinque campioni.
Ha motivato R10 e R11.

---

## R8: Diagnostica live e interfaccia

- Il **margine attuale** sostituisce il minimo storico nel verdetto live; il minimo
  resta nelle statistiche di sessione.
- Volt, ampere e watt provengono dall'**ultimo campione**, non da medie e valori
  correnti mescolati.
- **OCP** resta un segnale da controllare; da solo non conferma un guasto hardware.
  Gli avvisi rientrati non sono mostrati come ancora attivi.
- COP indicato come stima: `max(0, 12 × (CPU − piastra) / watt)`, limitato a 5.
- Modalità selezionata evidenziata, entrambe sempre cliccabili. Valori TEC, CPU e
  rugiada nella testata del primo grafico.

**Validazione**: 310 test. **ACK e offset riletti** sul controller per Cryo (+2 °C)
e Unregulated (−30 °C). **Non dimostrato**: le modalità native del firmware né la
dissipazione massima sotto carico. Layout completo e gocce non confermati
visivamente (Computer Use non ha esposto la finestra).

→ [`docs/releases/R8-diagnostica-live.md`](docs/releases/R8-diagnostica-live.md)

---

## R7: Correzioni termiche e comandi

- `InAttesa::Nessuna` non blocca più i comandi di cambio modalità.
- Il pulsante Cryo resta cliccabile anche per ripetere una richiesta.
- **Priorità ai sensori CPU die/core** rispetto alla lettura AIDA64 generica; a pari
  priorità usa il più caldo.
- Riduzione per budget watt: **1 °C di offset ogni 30 s** invece di 2 °C ogni 2 s.
- Recupero del raffreddamento quando i watt scendono sotto metà budget.
- Con CPU ≥ 85 °C: richiesta di raffreddamento più rapida entro il budget. *85 °C
  è un'indicazione di domanda, non una soglia certificata del processore.*
- Condensa e temperatura controller mantengono la priorità.

**Validazione**: 309 test. **Non dimostrato**: comportamento sotto carico reale per
questa build; il registro percentuale non è un limite watt verificato.

→ [`docs/releases/R7-termica-comandi.md`](docs/releases/R7-termica-comandi.md)

---

## R6: Design

- Superficie laterale scura e traslucida: contrasto dei controlli migliorato,
  foto sottostante conservata.
- Badge Cryo e OCP con gradienti discreti, bordi del colore semantico e bagliore
  conservato. OCP resta ambra; FAILSAFE mantiene la presentazione critica rossa.
- Evidenza visiva del profilo il cui nome corrisponde al campo corrente.
  *Non costituisce conferma della modalità da parte del firmware.*
- Statistiche di sessione: etichette più compatte, numeri ad alto contrasto.

**Validazione**: 304 test, build release completata. Nessun cambio a messaggi,
formule, profili, seriale o logica TEC.

→ [`docs/releases/R6-design.md`](docs/releases/R6-design.md)

---

## R5: Indicatore di condensa

Tre gocce vettoriali blu con sfumatura zaffiro, bordo chiaro e riflessi.
Visibili **solo** se l'ultima coppia TEC/rugiada è valida, contemporanea, recente
(entro 5 s) e `TEC < rugiada`. Alla soglia esatta le gocce non compaiono.
Testata ad altezza fissa anche quando le gocce sono nascoste.

**Validazione**: 304 test. Il tooltip chiarisce che è rischio condensa, non acqua
fisica. Verifica visiva sul desktop da confermare.

→ [`docs/releases/R5-gocce-condensa.md`](docs/releases/R5-gocce-condensa.md)

---

## R4: Grafica

- Animazione ogni 33 ms (~30 fps) quando la dashboard è collegata e visibile; il
  timer si ferma nella schermata iniziale e quando nascosta dal tray. Polling del
  controller e protezioni **mantengono la propria cadenza**.
- Cache della geometria per evitare di ricostruire un grafico più volte nello stesso
  intervallo di animazione.
- Rimossa la traslazione artificiale del timestamp dell'ultimo campione: la finestra
  visiva ritarda di 500 ms e il bordo della curva **interpola linearmente** tra due
  campioni realmente disponibili, senza sovraelongazione né estrapolazione. I
  campioni delle letture numeriche, dei controlli TEC, delle registrazioni e delle
  esportazioni **non** sono modificati.
- Scala del grafico termico calcolata all'arrivo dei campioni.
- Campioni non finiti esclusi dai grafici.

**Validazione**: 302 test, con 4 verifiche nuove su interpolazione, assenza di
estrapolazione, associazione temporale del tooltip e scala termica.
**Non dimostrato**: nessun risparmio CPU/GPU dichiarato; aumentare la frequenza di
disegno può aumentare il carico grafico.

→ [`docs/releases/R4-grafica.md`](docs/releases/R4-grafica.md)

---

## R3: Modalità e profili

- Il gestore automatico di carico operava **anche in Unregulated** e poteva
  riscrivere il setpoint. Ora agisce **solo in Cryo** e non sovrascrive i preset.
- Il caricamento profili **convertiva il segno dell'offset** e confondeva l'offset
  firmware con il margine misurato dalla rugiada. I due significati sono separati.
- Rimosso il re-push della percentuale di potenza ogni 5 s: non costituisce un
  limite affidabile.
- I preset usano la combinazione PID **100/1/0** già provata su questo hardware; la
  differenza di prestazioni si ottiene con offset e budget.

| Profilo | Budget software | Obiettivo piastra sopra rugiada | Offset iniziale |
|---|---|---|---|
| Silenzioso / Idle | 60 W | +6 °C | +6 °C |
| Gaming | 120 W | +3,5 °C | +3 °C |
| AI / Rendering | 160 W | +3 °C | +2 °C |

**Validazione**: 298 test. **Non dimostrato**: i tre profili non sono ancora stati
misurati a carico reale. Nessuna percentuale di risparmio dimostrata.

→ [`docs/releases/R3-profili.md`](docs/releases/R3-profili.md)

---

## R2: Gen 1 controller + TEC Gen 2

Prima verifica su hardware reale (COM5, `HW 4`, firmware `13.A0`).

**Prove eseguite**

- Prima dell'accensione: PID 0/0/0, FAILSAFE e LOW_POWER attivi, 0 W.
- Sequenza offset + PID 100/1/0 + enable: **ACK riuscito**, lettura PID 100/1/0,
  `PID_RUNNING` attivo, piastra da ~39 a 33 °C.
- **Il tetto di potenza non è un limite affidabile**: richiesto 30 %, letto
  98–100 %, circa 246–260 W. L'uguaglianza tra percentuale richiesta e duty letta
  **non** conferma un cap.
- Offset +2 → ~253–259 W · Offset +10 → ~6–10 W · Offset +20 → ~0,5–0,6 W.
  *Prova breve, condizioni diverse da regime stazionario: non è una curva di COP.*
- Ogni prova seriale diretta terminata con **disable confermato**.

**Gestione introdotta**

- Il tasto **ABILITA** usa un percorso diretto che non salta il comando per un regime
  memorizzato. Errori di regime e errori delle altre scritture sono distinti.
- In Cryo la regolazione cambia la domanda tramite offset e osserva piastra, rugiada,
  PCB e watt. **Non deduce** la temperatura della ceramica calda dalla PCB e **non usa
  un COP inventato** per comandare l'hardware.
- Ricerca graduale: passo di 0,5 °C, attesa 30 s; **annulla** un aumento che aggiunge
  watt senza migliorare la piastra; pausa 120 s dopo un aumento inutile.
- Budget software: 100 % = 200 W. *Non è una dichiarazione del rating elettrico del
  controller e NON è un limite istantaneo.*
- Se il monitoraggio fallisce ripetutamente, il watchdog richiede **disable reale**.

**Validazione**: 295 test. **Non dimostrato**: nessun optimum globale né percentuale
di risparmio a pari carico. La TEC non è stata attivata durante il lavoro di correzione.

→ [`docs/releases/R2-gen1-tec2.md`](docs/releases/R2-gen1-tec2.md)

---

## R1: Correzioni della gestione TEC

Dodici difetti corretti nella sequenza di commutazione e nella conferma dei comandi.
I più gravi:

- La commutazione non impostava la **richiesta**: gli ACK non potevano confermare il
  comando corrente.
- Una rilettura discordante **sostituiva l'intenzione dell'operatore**.
- La rampa partiva **prima dell'ACK** e dal livello precedente: ora parte dopo l'ACK
  dal livello iniziale della sequenza.
- L'ACK di spegnimento non era distinto da quello di accensione: la chiusura poteva
  consumare un ACK di accensione precedente come conferma di spegnimento.
- PID, offset e potenza validati **prima** dell'accensione: NaN, infinito, PID
  negativi e potenza oltre 100 % rifiutati.

**Validazione**: 289 test passati, con prove della sequenza seriale, di errore su
ognuno dei sette comandi di accensione, di disable fallito, di parametri invalidi e di
rilettura discordante. Due test di integrazione intenzionalmente ignorati: uno
richiede hardware, uno modifica il database reale.

→ [`docs/releases/R1-correzioni-gestione-tec.md`](docs/releases/R1-correzioni-gestione-tec.md)

---

## v2.4: Correzioni di tempistica e umidità

Correzioni a difetti che producevano dati falsi:

| # | Difetto | Correzione |
|---|---|---|
| 1 | contatori calibrati per 20–100 Hz con gate a 2 Hz | tutti i valori modulo corretti (`% 200 → % 4`, `% 6000 → % 120`) |
| 2 | `CondensationRisk` sempre `Critical` | `analyze(0.01)` → `analyze(0.5)`: il trend era **50× sovrastimato** |
| 3 | AI Advisor con `dew_point: 0.0` e `humidity: 0.0` hardcoded | letti dall'ultima `LogEntry` |
| 4 | config in `%APPDATA%\StargateCryo\`, dati in `%APPDATA%\StargateLabsCryo\` | directory unificata |
| 5 | PID Wizard: step di 25 minuti | `3000` tick (100 fps) → `60` (2 Hz) = **30 s** |
| 6 | `Vec` di log illimitato | cap a **10.000** con sliding window |
| 7 | `extract_params()` fragile | parsing token-based, ancorato alla sezione giusta |
| 8 | byte iniziale della risposta non validato | check `buffer[0] != 0xAA` |

Aggiunte: potenza TEC in watt `V × I`, Condensation Margin, Session Stats, export CSV,
profili PID, integrazione HWiNFO64, tema neon green.

---

## Documenti di analisi

- [`docs/reverse-engineering/cryo-gen1.md`](docs/reverse-engineering/cryo-gen1.md)
- [`docs/hardware/cella-peltier.md`](docs/hardware/cella-peltier.md)
- [`docs/hardware/ottimizzazione-before-after.md`](docs/hardware/ottimizzazione-before-after.md)