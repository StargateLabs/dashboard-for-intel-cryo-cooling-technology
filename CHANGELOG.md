# Changelog

Tutte le build elencate sono **verificate e collaudate**. Ogni nota indica
esplicitamente cosa **non** è stato dimostrato.

Il versioning R1–R13 copre la serie di collaudo su hardware reale
(controller Intel Cryo Gen 1 `HW 4`, firmware `13.A0`, cella TEC V2).
La serie `rNN` più bassa corrisponde alle build di sviluppo giornaliere.

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
[`dati-gpu-r9-prima.csv`](docs/releases/dati-gpu-r9-prima.csv) (12.7–15.0 %) ·
[`dati-gpu-r10.csv`](docs/releases/dati-gpu-r10.csv) (1.2–1.8 %, non confrontabile)

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

Prima misura del carico GPU dell'applicazione: **12.7–15.0 %** su cinque campioni.
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
| Gaming | 120 W | +3.5 °C | +3 °C |
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
- Offset +2 → ~253–259 W · Offset +10 → ~6–10 W · Offset +20 → ~0.5–0.6 W.
  *Prova breve, condizioni diverse da regime stazionario: non è una curva di COP.*
- Ogni prova seriale diretta terminata con **disable confermato**.

**Gestione introdotta**

- Il tasto **ABILITA** usa un percorso diretto che non salta il comando per un regime
  memorizzato. Errori di regime e errori delle altre scritture sono distinti.
- In Cryo la regolazione cambia la domanda tramite offset e osserva piastra, rugiada,
  PCB e watt. **Non deduce** la temperatura della ceramica calda dalla PCB e **non usa
  un COP inventato** per comandare l'hardware.
- Ricerca graduale: passo di 0.5 °C, attesa 30 s; **annulla** un aumento che aggiunge
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