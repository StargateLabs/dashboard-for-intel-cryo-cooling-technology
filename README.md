# Dashboard for Intel® Cryo Cooling Technology

> Software alternativo di controllo per cooler TEC basati su **Intel Cryo Cooling Technology**.
> Funziona con **qualsiasi CPU**, non solo con le 23 elencate nel software ufficiale.

[![License: MIT](https://img.shields.io/badge/License-MIT-00FF41.svg)](LICENSE)
[![Language: Rust](https://img.shields.io/badge/Rust-1.75%2B-orange.svg)](https://www.rust-lang.org)
[![GUI: iced](https://img.shields.io/badge/GUI-iced%200.13-5c8dff.svg)](https://iced.rs)
[![Reverse Engineering](https://img.shields.io/badge/reverse%20engineering-documented-ff00a0.svg)](docs/reverse-engineering/cryo-gen1.md)
[![Test: 314](https://img.shields.io/badge/tests-314%20passing-00FF41.svg)](#compilazione)
[![Binari Intel](https://img.shields.io/badge/binari%20Intel-non%20ridistribuiti-00d26a.svg)](SECURITY.md)

![Dashboard CryoCooling, layout per monitor verticali](docs/images/dashboard-verticale.jpg)

*Screenshot reale: dashboard `r143` su controller **Intel Cryo Gen 1** (`HW 4`, `FW 13.A0`) con cella
**EK-Quantum Delta² TEC V2** montata. 78.217 campioni, TEC a 18.6 °C, 71.85 W misurati, duty 53 %,
margine di condensa +3.7 °C.*

---

## Indice

1. [Il problema](#il-problema)
2. [La scoperta che cambia tutto](#la-scoperta-che-cambia-tutto)
3. [Cosa fa la dashboard](#cosa-fa-la-dashboard)
4. [Risultati misurati](#risultati-misurati)
5. [Cosa NON fa e cosa NON è dimostrato](#cosa-non-fa-e-cosa-nonè-dimostrato)
6. [Il protocollo in breve](#il-protocollo-in-breve)
7. [Architettura](#architettura)
8. [Sicurezza hardware](#sicurezza-hardware)
9. [Compilazione](#compilazione)
10. [Documentazione](#documentazione)
11. [Crediti](#crediti)

---

## Il problema

Il software ufficiale Intel è chiuso e **non parte** su quasi tutte le CPU moderne.

L'installer contiene uno script WMI che confronta il nome del processore con una lista di **23 modelli,
tutti di 10ª generazione**:

```vbscript
CPUList = Array("10900K", "10850K", ..., "9900KS", "9700KF")
Set SearchObj = GetObject("WinMgmts:").instancesof("Win32_Processor")
' ~40 Replace() eliminano Intel(R), Core(TM), i9, -, @, GHz, MHz, Edition...
' poi il confronto è per TESTO ESATTO
```

Su una CPU di 14ª generazione il confronto fallisce e l'applicazione si rifiuta di avviarsi:
*"This cooling solution is not supported on this processor."*

Il blocco **non è un bug**: è la condizione prevista dal produttore. Ed è anche inutile come blocco,
perché quel software **non imposta la modalità di raffreddamento**, come si vede sotto.

Questa dashboard fa il lavoro che serve davvero: regolazione del freddo, PID, potenza, temperature,
margine di condensa, diagnostica e stato del controller. Su **qualsiasi CPU**.

---

## La scoperta che cambia tutto

> **Non esiste un opcode "cambia modalità".**

`IntelCryoCooling.Controller.dll` espone due scritture: `SetLowPowerMode` e `SetTemperatureSensorMode`.
**Non c'è `SetMode`.** Nessuno dei 16 moduli estratti contiene le stringhe `Unregulated`, `Standby` o
`Cryo mode`: la modalità non passa dalla seriale.

La modalità è lo **stato di un pin GPIO**, letto e scritto dal driver `CCHWApiExt.sys`
(`kHWAPIReadGPIO` / `kHWAPIWriteGPIO`) e da `TATInterface.dll` (`MMIOReadGPIO` / `MMIOWriteGPIO`).
**Chi sceglie la modalità è l'hardware**, in base al carico della CPU:

| Carico CPU | Comportamento del controller |
|---|---|
| **< 25 W** per > 10 minuti | l'Unregulated viene sospeso → torna a **Cryo** |
| **> 25 W** per > 10 minuti | l'Unregulated **continua** |

Questo spiega perché il software Intel *legga* la modalità invece di impostarla: la riga
`Cooler is in standby mode` è un messaggio di stato, non un'azione. E spiega perché non si può
aggiungere un interruttore: quel comando non esiste su quel canale.

### La conseguenza progettuale

Il regime si ottiene **scrivendo il setpoint offset e poi abilitando il controller**. Il regime è una
*conseguenza* di quei due comandi:

```
SetSetPointOffset(offset)   opcode 0x14, float32 LE
SetLowPowerMode(bool)       opcode 0x18  →  [0,0,0,0] accende · [1,0,0,0] spegne
GetBoardStatus()            opcode 0x00  →  per CONFERMARE il regime
```

I valori dei profili sono stati trovati in `Intel.CryoCooling.Configuration.dll`, dentro getter che
restituiscono un `float` costante (IL verificato: `ldarg.0; ldc.r4 -30.0; ret`):

| Offset | Regime | Comandi |
|---|---|---|
| setpoint utente | **Cryo** | `0x14`, `0x18 [0,0,0,0]` |
| **`-30.0`** | **Unregulated** | `0x14`, `0x18 [0,0,0,0]` |
| **`3.5`** | **Standby** | `0x14`, `0x18 [0,0,0,0]` |
| nessun offset | **Spento** | `0x18 [1,0,0,0]` *(solo: nessun `0x14`)* |

> **Perché "Spento" non scrive `0x14`**: su questo bus ogni byte in più può essere il reset di
> fabbrica (`0x1E`), e un controller spento non applica comunque un setpoint. Le impostazioni restano
> memorizzate perché lo spegnimento non è un reset.

Dettaglio completo dell'analisi: [`docs/reverse-engineering/cryo-gen1.md`](docs/reverse-engineering/cryo-gen1.md).

---

## Cosa fa la dashboard

| Area | Funzioni |
|---|---|
| **Controllo** | Offset setpoint, PID (P/I/D), budget in watt, duty letto, enable/disable con ACK e readback |
| **Regimi** | Cryo · Unregulated (offset `-30`) · Standby · Spento, con **dialogo unico** e conferma di regime |
| **Sicurezza** | guardia anticondensa sul margine `TEC − rugiada`, guardia termica controller, watchdog con disable reale, OCP come **segnale da controllare** (non azione automatica) |
| **Misure** | piastra, punto di rugiada, umidità, tensione, corrente, watt, duty, temperatura scheda, 14 bit di stato grezzi |
| **Sensori** | HWiNFO64 shared memory **e** AIDA64, entrambi selezionabili a runtime; priorità ai sensori die/core |
| **Grafici** | 8 grafici su asse temporale condiviso, interpolazione 1 s, cache geometria, ~30 fps, stop automatico in Home e nel tray |
| **Profili** | 3 preset (Silenzioso 60 W / Gaming 120 W / AI 160 W) + profili personali, isteresi ±0.75 °C |
| **Test** | **314 test passanti** (294 applicazione + 20 libreria), verificati con `cargo test --workspace`; 5 test di integrazione intenzionalmente ignorati perché richiedono hardware collegato o scrivono sul database reale |
| **Diagnostica** | 2 problemi reali, storico min/avg/max di sessione, COP **etichettato come stima**, log `tec-controller.log` |
| **Dati** | export CSV su Desktop, auto-save CSV ogni 5 min, report PDF, storico sessioni SQLite |
| **Continuità** | supervisore che riavvia dopo crash, riattiva **solo** Cryo se era stato richiesto, backoff fino a 30 s, `recovery.log` |
| **Integrazioni** | AI Advisor (modello via `ANTHROPIC_API_KEY` da env), RTSS overlay, Discord, PID Auto-Tuning wizard |

### Layout per monitor verticali

![](./docs/images/dashboard-verticale.jpg)

Progettato per **1080×1920 e simili**: sidebar fissa a sinistra (≈26 %) con strumentazione, controlli,
PID, profili e integrazioni; colonna destra fluida con 8 grafici impilati su **asse temporale condiviso**
e pannello sensori a 27 canali in basso. Card scure traslucide sopra un'immagine full-bleed,
separazione 8 px, badge `CRYOGENIC HAZARD` sempre visibile in testata.

---

## Risultati misurati

Su questo hardware (controller Gen 1, cella TEC V2, misure in regime):

| Regime | Tensione | Corrente | Potenza | Piastra | Rugiada | COP stimato |
|---|---|---|---|---|---|---|
| 86 % | 10.38 V | 21.70 A | **225 W** | 8.2 °C | 15.01 °C | **0.95** |
| ~48 % | non rilevato | non rilevato | **112 W** | non rilevato | non rilevato | **1.70** |

**Metà watt per COP quasi doppio.** Il radiatore riceve molto meno calore. Questo è il risultato
centrale del lavoro: *a parità di freddo, il punto di funzionamento più basso consuma meno e scalda meno*.

### Due ipotesi iniziali che le misure hanno smentito

| Ipotesi iniziale | Verdetto | Evidenza |
|---|---|---|
| *"Il kit ha un tetto rigido di 200 W"* | **Falsa** | il controller eroga **220 / 230 / 237 W** in modo stabile, con raffreddamento regolare |
| *"L'OCP è una protezione"* | **Falsa** | si accende a **73 W** e a **112 W** con sistema sano e funzionante |

Conseguenze applicate nel codice: `TETTO_WATT_CONTROLLER = 200.0` è diventato un **avviso**, mai un
blocco; l'intervento automatico su OCP è stato **limitato a una sola volta** (build r97), perché un
segnale che scatta a un terzo della potenza nominale non è una protezione e un avviso permanente non
è un avviso.

### Il limite reale è il lato caldo, non il software

Il COP è limitato dal lato caldo. Con lato freddo a 5 °C:

| Lato caldo | COP massimo (Carnot) |
|---|---|
| 40 °C | 0.79 |
| 33 °C | **1.04** (+32 %) |

Portare il lato caldo da 40 °C a 33 °C vale **+32 % di prestazione utile a parità di watt**: nessun
algoritmo arriva lontano. È lavoro meccanico (flusso d'aria, radiatore pulito, isolamento).

Il valore `CTRL` mostrato nella dashboard è il **PCB del controller**, non il lato caldo della cella:
il controller è montato lontano dal water block e segna fresco mentre il water block incassa ~400 W.
Per questo la guardia termica **non scende mai**: fermare il TEC perché il PCB è fresco mentre il
water block scalderebbe sarebbe il modo giusto di distruggere la piastra.

---

## Cosa NON fa e cosa NON è dimostrato

Questa sezione esiste perché nel progetto un numero di test verdi non viene usato come prova di
correttezza. È la parte più importante del documento.

- **Non certifica le modalità native del firmware.** `Unregulated` e `Standby` sono raggiunti
  **tramite offset**, per compatibilità col comportamento osservato. Non è stato dimostrato che
  cambi il bit `TEMP_MODE`. Nessun comando è stato inventato per forzarlo.
- **Non dichiara un tetto elettrico.** Il budget watt è un **obiettivo di retroazione software**.
  Non è un limite istantaneo né il rating del controller: sono stati misurati picchi iniziali
  ~260 W prima che la retroazione riducesse la domanda.
- **Non dichiara un COP misurato.** Il COP mostrato è `max(0, 12 × (CPU − piastra) / watt)`, limitato a 5.
  La conduttanza `12 W/°C` è **ipotizzata**. Non è una misura di calore rimosso.
- **Nessuna percentuale di risparmio a pari carico** è dimostrata. Senza la tabella di funzionamento
  reale sarebbe un'ipotesi, e un'ipotesi presentata come fatto porta a decisioni sbagliate.
- **Non certifica la dissoluzione termica sotto carico prolungato.** La prova completa a carico
  elevato e prolungato non è stata eseguita.
- **Non interpreta i 14 bit di stato sconosciuti.** Sono registrati grezzi in `bit-stato.csv`. Un codice
  errato in diagnostica fa diagnosticare il problema sbagliato, che è peggio di non avere
  diagnostica.
- **Non ridistribuisce binari Intel.** Vedi [`SECURITY.md`](SECURITY.md).

### Quattro difetti trovati con la suite verde

Una verifica sistematica del 2026-09-29 trovò **quattro difetti mentre i 210 test erano verdi**.
Nessuno dei quattro era coperto. Da allora il numero di test non viene più usato come prova:

| # | Difetto | Conseguenza |
|---|---|---|
| **D1** | `Tec::new()` mandava `0x1E` (reset di fabbrica) se `BOARD_INIT` non era impostato | a ogni connessione perdevi **PID, setpoint e power cap**. Un limite impostato spariva senza avviso |
| **D2** | il pulsante abilita scriveva l'offset in proprio | poteva sovrascrivere il `-30` dell'Unregulated mentre il menu mostrava il regime vecchio |
| **D3** | `tec_abilitato = !LOW_POWER_MODE_ACTIVE` | in Standby, dove il TEC **deve** essere acceso, la guardia perdeva l'autorizzazione a scrivere |
| **D4** | il margine di condensa non si azzerava su errore | la riga stampava `Margine +3.0 °C OK` in verde **con il controller scollegato da un minuto** |

La specifica con le **12 invarianti** che ne deriva è in
[`docs/architecture/unico-percorso-di-comando.md`](docs/architecture/unico-percorso-di-comando.md).

---

## Il protocollo in breve

Ponte USB-seriale **Silicon Labs CP210x**. `115200 baud, 8N1`, pacchetti da 8 byte:

```
[0xAA] [opcode] [data ×4] [CRC16-XMODEM ×2]
```

**21 opcode**, verificati uno a uno contro i metodi reali di `VcpProtocol`
(l'opcode è l' immediato `ldc.i4.s` prima di `stfld Oper`):

| Opcode | Metodo Intel | Ruolo | Pericolo |
|---|---|---|---|
| `0x00` | `HeartBeat` | lettura stato (32 bit) | nessuno |
| `0x01`–`0x0A` | vari getter | temperature, umidità, rugiada, PID, HW/FW | nessuno |
| `0x14` | `SetSetPointOffset` | scrive l'offset del setpoint | **azzera il setpoint** |
| `0x15` `0x16` `0x17` | `SetP` `SetI` `SetD` | guadagni PID | **azzera i guadagni** |
| **`0x18`** | `SetLowPowerMode` | `[0,0,0,0]` **abilita** · `[1,0,0,0]` **disabilita** | polarità opposta al nome |
| `0x19` | `SetCpuTemperature` | temperatura CPU al PID | nessuno |
| `0x1A` `0x1B` | `SetNTC` `GetNTC` | NTC | nessuno |
| `0x1C` | `SetTemperatureSensorMode` | modalità sensore | nessuno |
| `0x1D` | `SetTECPower` | tetto di potenza | **porta il tetto a 0 %** |
| **`0x1E`** | `ResetBoard` | **reset di fabbrica** | **perde PID, setpoint, tetto** |

Per questo gli opcode inviati passano da una **allowlist di sola lettura** (`SOLO_LETTURE` in
`lib.rs`): `0x18` con dati nulli *abilita* il TEC e `0x1E` con dati nulli *è il reset di fabbrica*.
Nessuno dei due può essere trattato come lettura, e spostare gli argomenti basta ad aggirare una
fiducia basata sul nome del metodo.

---

## Architettura

Workspace Cargo con due crate:

```
cryo_cooler_controller_lib/   protocollo seriale, framing, CRC, allowlist, decodifica 32 bit
cryo_cooler_controller/       GUI iced 0.13, regolazione, diagnostica, grafici, persistenza
```

```
                        ┌──────────────────────────────────────┐
   AIDA64 / HWiNFO64 ───►│  sensori: CPU, GPU, umidità, Dew pt  │
                        └───────────────┬──────────────────────┘
                                        │
   ┌────────────┐   comandi    ┌─────────▼─────────┐   frame 8 byte   ┌──────────────┐
   │  UI iced    ├────────────►│  attore_tec       ├─────────────────►│ CP210x COM5  │
   │ (iced 0.13) │◄────────────┤  unico percorso   │◄─────────────────┤ Intel Gen 1  │
   └────────────┘   misure     │  di scrittura     │    32 bit status  └──────────────┘
        │                       └─────────┬─────────┘
        │                                 │
        │            ┌────────────────────▼───────────────────┐
        └───────────►│  watchdog · anticondensa · termica     │
                     │  recovery · sessioni SQLite · PDF/CSV  │
                     └────────────────────────────────────────┘
```

**Un solo percorso di scrittura**: nessun elemento della UI scrive direttamente un offset, una potenza
o un PID. L'unica sorgente di comandi è il regime selezionato, e ogni sequenza finisce con una
`GetBoardStatus()` di conferma: se il controller non conferma, l'interfaccia dice **"non commutato"**,
non "fatto".

Ogni modulo è un file Focused: 36 file `.rs`, **24.536 righe**, 26 dipendenze dirette nell'app.

---

## Sicurezza hardware

> Il TEC raffredda sotto il punto di rugiada. **L'acqua sulla piastra è il danno che uccide per primo.**

**Prima di collegare l'hardware**

- HW spento e **piastra asciutta**.
- Non usare mai il software ufficiale Intel insieme a questa dashboard: occupa la stessa porta COM.
- Chiudere l'altra dashboard **anche dal tray** prima di aprire questa.

**Durante l'uso**

- Il software **non alza mai** le soglie di protezione per "raffreddare di più". Il firmware taglia a
  80 °C e 90 °C; la guardia software sta volutamente sotto (58/66/76 °C).
- **Non toccare il piano anticondensa**: con margine di 1–2 °C è l'unica cosa che impedisce la
  condensa sulla piastra.
- **Non inseguire la potenza massima**: oltre il punto di rendimento si spendono watt e si scalda il
  lato caldo per guadagno nullo o negativo.
- Non usare la temperatura CPU come riferimento per ventola e pompa: il TEC raffredda la CPU, quindi
  il resto del circuito di liquido riceve il calore tolto e il segnale è falsato. Il manuale del
  produttore lo dice esplicitamente. Le ventole le pilota la scheda madre, non questa dashboard.

**Regole di protocollo da non violare**

- `0x1E` è il reset di fabbrica: **non deve mai essere emesso per sbaglio**, né come fallback.
- Connettersi **non è un'azione di stato**: aprire la porta non modifica nulla (difetto D1).
- `0x18` ha polarità opposta al nome: `[0,0,0,0]` accende.

Dettaglio: [`SECURITY.md`](SECURITY.md).

---

## Compilazione

Prerequisiti: **Rust 1.75+** e il driver **CP210x** (Silabs USB-UART, incluso in Windows 10/11).

```bash
git clone https://github.com/StargateLabs/dashboard-for-intel-cryo-cooling-technology.git
cd dashboard-for-intel-cryo-cooling-technology
cargo build --release
# eseguibile: target/release/cryo_cooler_controller.exe
```

**Uso**

1. Collega il cooler via USB
2. Seleziona la porta COM dal menu a tendina (se incerto: scollega/ricollega e guarda quale sparisce)
3. Imposta offset e budget watt, poi **ABILITA TEC**
4. Grafici e misure in tempo reale a 2 Hz, animazione ~30 fps

**Opzioni utili**

```bash
# anteprima grafici con dati simulati, senza seriale né configurazione
cryo_cooler_controller.exe --preview-grafici

# anteprima del caso sotto rugiada (gocce di condensa)
cryo_cooler_controller.exe --preview-grafici --condensa

# collaudo del supervisore di riavvio, senza aprire porte COM
cryo_cooler_controller.exe --recovery-self-test
```

**Test**

```bash
cargo test --workspace
```

Alcuni test di integrazione sono **intenzionalmente ignorati**: richiedono hardware collegato o
toccherebbero il database reale. Ignorarli è la scelta corretta: un `cargo test` non deve mai
accendere il TEC da solo.

---

## Documentazione

Tutto il lavoro è documentato, incluse le ipotesi **smentite**.

| Documento | Contenuto |
|---|---|
| [`reverse-engineering/cryo-gen1.md`](docs/reverse-engineering/cryo-gen1.md) | analisi dei binari Intel: architettura a 3 strati, tabella opcode con RVA, logica dei regimi, lista CPU dell'installer |
| [`reverse-engineering/protocol.md`](docs/reverse-engineering/protocol.md) | protocollo operativo: frame, CRC, 21 opcode, allowlist di sola lettura, 18 bit di stato e una discrepanza ancora aperta |
| [`hardware/cella-peltier.md`](docs/hardware/cella-peltier.md) | la legge che governa una cella Peltier, cosa manca nelle misure, ordine di intervento |
| [`hardware/ottimizzazione-before-after.md`](docs/hardware/ottimizzazione-before-after.md) | due ipotesi smentite dalle misure: il tetto 200 W e l'OCP |
| [`hardware/error-codes.md`](docs/hardware/error-codes.md) | CB2, CF1-CF7, OT1-OT3, TD1, DT1-DT2, CB1 e le soglie della dashboard |
| [`hardware/layout-verticale.md`](docs/hardware/layout-verticale.md) | disposizione a due colonne per monitor verticali, palette dei canali, comportamento degli indicatori |
| [`architecture/unico-percorso-di-comando.md`](docs/architecture/unico-percorso-di-comando.md) | 12 invarianti, i 4 difetti D1-D4, e cosa il progetto ha deciso di non fare |
| [`architecture/plans/`](docs/architecture/plans/) | i piani di lavoro che hanno prodotto le correzioni |
| [`releases/`](docs/releases/) | una nota per ogni build verificata, da R1 a R13, con evidenze e limiti |
| [`README-originale-upstream.md`](docs/README-originale-upstream.md) | il README del progetto originale, per confronto |

---

## Crediti

- **Progetto originale**: [juvgrfunex/cryo-cooler-controller](https://github.com/juvgrfunex/cryo-cooler-controller), MIT. Questo repository è un fork avanzato.
- **Hardware**: controller Intel Cryo Cooling Technology Gen 1 · cella EK-Quantum Delta² TEC V2 (LGA1700).
- **Documentazione consultata**: manuale EK Delta² (`EK-IM-3831109859612.pdf`) · referenza termoelettrica Ferrotec.
- **Nessun binario proprietario Intel è incluso o ridistribuito** in questo repository.

<div align="center">

**Stargate Labs**

*Reverse engineering documentato. Misure dichiarate solo quando misurate.*

</div>