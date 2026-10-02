# MEMORIA: Reverse Engineering Intel Cryo Cooling Gen 1

> Documento di riferimento per il comando di cambio modalità.
> Fonte: binari originali Intel, estratti dall'installer
> `intel_r_cryo_cooling_technology_v1.1.0.319_release.exe`.
> Copia di lavoro: `<CRYO_RE>\`
> md5 `CryoCoolingService.exe` = `dfd3285b925800207245bfcec6d66da9`
> md5 `IntelCryoCooling.Controller.dll` = `e78c6706b27f900aa2028c2e4e411167`

---

## 1. La risposta in una riga

**Non esiste un opcode "cambia modalità".** La modalità non si comanda
direttamente: si ottiene scrivendo il **setpoint offset** e poi abilitando il
controller. Il regime (`Standby` / `Cryo` / `Unregulated`) è una **conseguenza**
di quei due comandi, non un comando a sé.

Questo è il punto che cambia tutto rispetto a quanto ipotizzato prima.

---

## 2. Architettura dei comandi

Tre strati, dal più alto al più basso

| Strato | Classe | Ruolo |
|---|---|---|
| API pubblica | `ControllerService` | WCF, espone `InitCryoMode` ecc. |
| Instradamento | `Controller` | mappa metodo → opcode |
| Protocollo | `VcpProtocol` | costruisce e invia il frame seriale |

### 2.1 `ControllerService` (in `CryoCoolingService.exe`)

Metodi pubblici, da `ICryoCoolingService`

```
GetOperationMode        GetBoardStatus         GetSetPointOffset
ResetBoard               GetBoardTemperature    GetTecVoltage / GetTecCurrent
SetSetPointOffset        SetStandbyMode         SetTemperatureSensorMode
InitCryoMode             InitUnregulatedMode    InitStandbyMode   (×2 overload)
```

I tre `Init*Mode` sono **wrapper sottili**: chiamano un unico worker privato
passandogli un **identificatore numerico di regime**.

| Metodo | ID regime | IL (byte) |
|---|---|---|
| `InitCryoMode` | **3** | `13 00 00 11 02 14 19 28 75 00 00 06` |
| `InitUnregulatedMode`| **26** | `13 00 00 11 02 14 1a 28 75 00 00 06` |
| `InitStandbyMode` (a)| **23** | `01 00 00 11 02 14 17 28 75` |
| `InitStandbyMode` (b)| **4** | `01 00 00 11 02 03 17 28 75` |

Worker comune: **`_SuwRS2F8fovSdlJCBMLFuCKsDOk`**, RVA `0x96B8`, 218 byte IL.
Nome offuscato con caratteri Unicode bidi (zero-width, RLO/LRO): un trucco per
ingannare gli editor che ordinano diversamente il testo.

> Nota: `InitCryoMode` (ID 3) e `InitStandbyMode` (ID 4/23) hanno ID che
> **non** coincidono con gli opcode del protocollo. Sono identificatori
> interni del livello applicativo, non byte di protocollo. Non vanno usati
> direttamente sul bus.

### 2.2 `Controller` (in `IntelCryoCooling.Controller.dll`)

Fa solo mappatura metodo → opcode e delega a `VcpProtocol`.

### 2.3 `VcpProtocol`: **qui sta la verità**

Ogni `Set*` costruisce un `SerialPortJob { int Oper; byte[] payload; ... }` e
lo invia. L'opcode è un `ldc.i4.s` immediatamente prima di `stfld Oper`.

Rilevazione automatica su tutti i metodi di `VcpProtocol` (byte `0x1f` =
`ldc.i4.s`, valore nell'intervallo opcode)

| Metodo | RVA | Opcode | Ruolo |
|---|---|---|---|
| `SetSetPointOffset` | `0x7FF0` | **`0x14`** | scrive l'offset del setpoint |
| `SetP` | `0x801C` | **`0x15`** | coefficiente P |
| `SetI` | `0x8048` | **`0x16`** | coefficiente I |
| `SetD` | `0x80B8` | **`0x17`** | coefficiente D |
| `SetLowPowerMode` | `0x8154` | **`0x18`** | abilita / disabilita il TEC |
| `SetCpuTemperature` | `0x8128` | **`0x19`** | temperatura CPU al PID |
| `SetNTC` | `0x81E0` | **`0x1A`** | NTC |
| `GetNTC` | `0x7DB0` | **`0x1B`** | lettura NTC |
| `SetTemperatureSensorMode` | `0x81B4` | **`0x1C`** | modalità sensore |
| `SetTECPower` | `0x820C` | **`0x1D`** | tetto di potenza |
| `ResetBoard` | `0x7F2C` | **`0x1E`** | **reset di fabbrica** |

**Confronto con il protocollo già implementato nella dashboard: identico, 21
opcode su 21.** Nessuna scoperta nuova sul lato del protocollo: la mia tabella
era corretta. La scoperta è su *come si usa*.

### 2.4 `SetLowPowerMode` in dettaglio

RVA `0x8154`, 81 byte IL

```
1b 00 00 11   ldarg.1                  ; valore richiesto (bool/int)
03            ldarg.0
28 e6 00 00 06  call  0x060000E6        ; -> costruisce SerialPortJob
0a            stloc.0
1f 18         ldc.i4.s 0x18            ; *** OPCODE 0x18 ***
06            ldloc.0
73 01 01 00 06 newobj SerialPortJob    ; [0x06000101]
0b            stloc.1
...           (payload dalla firma)
```

Nota il nome: **`SetLowPowerMode`, non `SetMode`.** Il "low power" è la
traduzione del produttore per **Standby**. Conferma: il regime riposo è
esattamente il bit `LOW_POWER_MODE_ACTIVE` che già leggo.

---

## 3. Come si cambia regime, quindi

Il percorso reale è

```
InitCryoMode / InitUnregulatedMode / InitStandbyMode
        │
        └─> worker(mode)
                │
                ├─> SetSetPointOffset(offset)   opcode 0x14
                └─> SetLowPowerMode(bool)        opcode 0x18
```

Il **setpoint offset** è il vero selettore: un offset "normale" (es. `-10 °C`)
con il TEC abilitato dà **Cryo**; un offset che il controller non può soddisfare
costringe alla potenza massima → **Unregulated**.

Per questo il produttore chiama il metodo `Init*Mode` e non `Set*Mode`: non
esiste un registro "modalità" nel controller, esiste il **setpoint**.

### Correzione: "disabilitare dà Standby" era falso

Una versione precedente di questo documento concludeva che *disabilitare il TEC
dà Standby*, e il software aveva conseguentemente un solo comando di
spegnimento: `SetLowPowerMode(false)`. La conseguenza pratica era che il
pulsante Standby spegneva il modulo, e l'operatore vedeva sparire tutte le
misure senza aver chiesto di spegnere nulla.

La correzione viene dal manuale Intel, sezione 3.1, che definisce Standby come

> *"Cryo cooling radiator fans, and pump to provide typical liquid cooling
> capability without sub-ambient cooling"*

**Ventole e pompa restano accese**; a fermarsi è solo il raffreddamento
sub-ambiente. Standby e "spento" sono quindi due stati distinti

| Stato | TEC | Offset |_offset scrivibile_ | Comandi |
|---|---|---|---|---|
| **Cryo** | acceso | setpoint utente | sì | `0x14`, `0x18 [0,0,0,0]` |
| **Unregulated** | acceso | `-30.0` | sì | `0x14`, `0x18 [0,0,0,0]` |
| **Standby** | **acceso** | `3.5` | sì | `0x14`, `0x18 [0,0,0,0]` |
| **Spento** | **spento** |: | **no** | `0x18 [1,0,0,0]` |

Note sulla tabella

- **Standby scrive comunque `0x14`.** Se l'offset non cambiasse, il passaggio a
 Standby non si distinguerebbe da Cryo per il controller. Il valore `3.5` resta
 però *non confermato su hardware Gen 1* (vedi § 6).
- **Spento non scrive `0x14`.** Un solo comando: ogni byte in più su questo bus
 è un byte che può essere il reset di fabbrica (`0x1E`), e il controller spento
 non applica comunque un setpoint. Le impostazioni restano quelle già
 memorizzate, perché lo spegnimento non è un reset.
- **I bit di stato da soli non distinguono i tre regimi.** `TEMP_MODE` e
 `LOW_POWER_MODE` descrivono *che cosa è stato impostato*, non *che cosa il
 modulo stia facendo*: dopo uno spegnimento `TEMP_MODE` resta spesso acceso
 perché è l'ultimo regime registrato. Per questo il regime corrente si legge
 da `PID_RUNNING` (il regolatore è in marcia) insieme a quei due bit, vedi § 4.

> **Non ancora verificato sul Gen 1.** L'interpretazione degli offset `3.5` e
> `-30.0` viene dai binari del produttore, non da misure su questo controller.
> Il software non deve essere dato per corretto su quel punto finché non è
> stato misurato.

---

## 4. Bit di stato e lettura del regime

`BoardRegisterMapping` espone i getter che il worker usa per riconoscere il
regime corrente

```
get_TempModeNTCEnabled     -> TEMP_MODE          (bit 17)
get_LowPowerEnabled        -> LOW_POWER_MODE     (bit 16)
get_OvercurrentTriggered   -> OCP                (bit 13)
get_PowerSupplyDetected    -> POWER_OK           (bit 1)
get_TecConnectionOk        -> TEC_CONN_OK        (bit 15)
get_BoardTempInRange       -> BOARD_TEMP_OK      (bit 14)
get_HumiditySensorInRange  -> HUM_SENSE_OK       (bit 3)
get_BoardInitCompleted     -> BOARD_INIT         (bit 0)
```

`get_OperationMode` (RVA `0x6600`) legge il campo e lo espone.

### Attenzione: i bit di stato non dicono da soli quale sia il regime

`TEMP_MODE` e `LOW_POWER_MODE` sono **stati registrati**, non stati correnti.
Sono entrambi latch: dopo uno spegnimento `TEMP_MODE` resta acceso, perché il
controller conserva l'ultimo regime impostato. Leggerli come "il regime
corrente" fa due errori allo stesso tempo, dice *Cryo* per un modulo spento, e
non distingue Standby da Cryo, perché non c'è un bit che lo faccia.

La lettura corretta usa **`PID_RUNNING`** (bit 2, `get_PidControllerRunning` /
stato del regolatore) come discriminante

```
PID_RUNNING assente            -> Spento      (il regolatore non gira)
PID_RUNNING + LOW_POWER_MODE   -> Standby
PID_RUNNING + TEMP_MODE        -> Cryo
PID_RUNNING, nessuno dei due   -> Unregulated (potenza massima)
```

Questa è la mappatura usata da `commutazione::Regime::da_stato`. È coerente con
le sequenze della § 6b, ma **deriva dal comportamento del protocollo, non da un
getter che lo dichiari**: va verificata su questo controller prima di considerarla
definitiva. Il caso da osservare per prima è proprio quello che ha rivelato il
difetto: dopo lo spegnimento, `TEMP_MODE` era ancora acceso e la dashboard
mostrava *Cryo* con tutte le misure a zero.

---

## 5. Regole di sicurezza nel protocollo (confermate)

Estratte dai metodi reali, non più dedotte

| Opcode | Pericolo | Confermato da |
|---|---|---|
| `0x18` `[0,0,0,0]` | **abilita** il TEC | `SetLowPowerMode` |
| `0x18` `[1,0,0,0]` | **disabilita** il TEC | `SetLowPowerMode` |
| `0x1E` | **reset di fabbrica**, perde PID, setpoint, tetto | `ResetBoard` |
| `0x15/16/17` | azzerano i guadagni PID | `SetP/SetI/SetD` |
| `0x1D` | porta il tetto a 0% | `SetTECPower` |
| `0x14` | azzera il setpoint | `SetSetPointOffset` |

La allowlist di sola lettura della dashboard (`SOLO_LETTURE` in `lib.rs`) è
**corretta** e va mantenuta.

---

## 6. I valori dei profili: trovati

In `Intel.CryoCooling.Configuration.dll`, classe config `_m4ZK0530JVpaF9wpRhN8nQiEUNi`
ci sono **getter che restituiscono un float costante** (pattern
`ldarg.0; ldc.r4 <f32>; ret`)

| Offset file | Metodo | Valore | RVA |
|---|---|---|---|
| `0x4764` | `_scsHwViN2QqxlTYKxT73MBYRkTC` | **`-30.0`** | `0x655C` |
| `0x46C8` | (metodo adiacente) | **`3.5`** | ~`0x6580` |

IL verificato del primo
```
01 00 00 11        ldarg.0
22 00 00 f0 c1     ldc.r4 -30.0
2a                ret
```

**Interpretazione.** `-30.0` e `3.5` sono i **setpoint offset** dei profili.
Con la logica del worker (l'offset determina il regime)

- **offset -30 °C** → il controller spinge al massimo → regime
 **non regolato / Unregulated** (potenza massima, sotto il rischio di rugiada)
- **offset 3.5 °C** → regime di **riposo / basso consumo** (nessun
 raffreddamento sub-ambiente spinto)

I default dei campi `initonly` float nella stessa classe (`.ctor`, RVA
`0x6xxx`, 106 byte, 10 `stfld`): `100.0`, `20.0`, `25.0`, `1.0`, `2.0`, questi
sono i **limiti** (tetto, soglie CB2, percentuali), non gli offset.

Il terzo profilo (Cryo bilanciato) non è un letterale: è derivato. La
sostituzione corretta per la dashboard, senza toccare `0x14` a caso, resta

```
Spento      -> LowPowerMode(false)              // SOLO 0x18, nessun 0x14
Cryo        -> SetSetPointOffset(quello che già fa enable()) + LowPowerMode(true)
Unregulated -> SetSetPointOffset(-30.0)         + LowPowerMode(true)
Standby     -> SetSetPointOffset(3.5)           + LowPowerMode(true)
```

> **Correzione rispetto alla versione precedente di questo documento.** Qui
> `Standby` era `LowPowerMode(true)` con l'offset `3.5`, e *manchiava del
> tutto* lo stato spento: i due venivano confusi, perché si credeva che
> `LowPowerMode(false)` fosse il modo di entrare in Standby. Non lo è, è il
> modo di spegnere il modulo. La conseguenza era che il pulsante Standby
> spegneva il TEC. La riga `Spento` qui sopra è quella che mancava.

> **Nota di prudenza.** `-30.0` e `3.5` sono **verificati nel binario**, ma non
> sono mai stati osservati in funzione su questo hardware. Il primo comando
> da provare in laboratorio è `Spento`, che è il meno invasivo in assoluto
> nessuna potenza, nessun freddo, e le impostazioni restano memorizzate. Poi
> `Standby`, che mette in funzione ventole e pompa senza freddo. Solo dopo
> con la piastra asciutta e l'occhio sulla temperatura, testare `Unregulated`.

## 6b. Come implementarlo (procedura, non interruttore)

`Init*Mode` non è un interruttore: è una sequenza. Per sicurezza il menu deve
fare esattamente quello che fa il software vendor, e in quest'ordine

1. `SetSetPointOffset(offset)`, opcode `0x14`, float32 LE
2. `SetLowPowerMode(bool)`, opcode `0x18`, `[0,0,0,0]` accende, `[1,0,0,0]`
 spegne
3. `GetBoardStatus()`, opcode `0x00`, per **confermare** che il regime è
 quello richiesto prima di dichiarare l'operazione riuscita

Il punto 3 è quello che rende il menu onesto: se il controller non conferma
l'interfaccia deve dire "non commutato", non "fatto".

---

## 7. Documentazione Intel (manuale `EK-IM-3831109859612.pdf`)

Sezione **3.1 "Three Modes of Operation"**, LED documentati

| Regime | Colore LED | Lampeggio |
|---|---|---|
| Standby | Blue | slow blinking |
| Cryo | Green | slow blinking |
| Unregulated | Purple | fast blinking |
| Offline | Red | solid |

Altre sezioni rilevanti
- **2.2.1**: il software espone una funzione **"Mode"**
- **4.3**: **"Remember Cryo Mode"**: il software ricorda la scelta Cryo
- **5.1**, *"Cryo Cooler switches itself from Unregulated mode to Cryo mode
 ... without any workload or User interaction for 10 minutes"*
- **5.2**: errore **CB2**: il controller è **restato** in Unregulated troppo
 a lungo (CPU non idle >25 W per >10 minuti)
- **5.2**: il software supporta **solo CPU Intel di 10ª, 11ª, 12ª generazione**

Conseguenza: la modalità **è** selezionabile dall'utente, ma il
comportamento "auto-ritorno a Cryo dopo 10 minuti" e l'errore CB2 sono
**controlli del controller**, indipendenti dal software.

---

## 8. Blocco CPU e installer

L'installer contiene uno **script WMI in chiaro** (offset `0x9D5A05` nel
file `intel_r_cryo_cooling_technology_v1.1.0.319_release.exe`)

```vbscript
CPUList = Array("10900K", "10850K", ..., "9900KF", "9700KF")
Set SearchObj = GetObject("WinMgmts:").instancesof("Win32_Processor")
For Each CPU In SearchObj
    ProcName = CStr(CPU.Name)
    ' ... ~40 Replace() ...
    If InStr(CPUList(i), ProcName) <> 0 Then
        Session.Property("RESULT_PROPERTY") = "#1"
    End If
Next
```

Le `Replace()` eliminano `Intel(R)`, `Core(TM)`, `i9`, `-`, ecc.
`Intel(R) Core(TM) i9-14900KS` → **`14900KS`**.

Installer patchato: `<INSTALLER_PATCHATO>`
con `14900KS` al posto di `10900KF`
`14900KF` al posto di `10700KF`, `14700KS` al posto di `11600KF`.
Sostituzioni a lunghezza identica: dimensione del file invariata.

> **Nota:** installare questo software su una CPU non supportata **può
> mandare in crash il PC** (è successo). L'hardware deve essere spento e la
> piastra asciutta. Il software serve solo come sorgente dei binari.

---

## 9. Indice dei binari

```
cryo-re/
├── CryoCoolingService.exe              177 608   WCF, orchestrazione, worker Init*Mode
├── IntelCryoCooling.Controller.dll      82 432   mappatura metodo → opcode
├── service/
│   ├── CryoCoolingService.exe                   copia
│   ├── IntelCryoCooling.Controller.dll          copia
│   ├── TATInterface.dll                1 248 768  GPIO / accesso hardware
│   ├── DataModel.dll                      61 952  modello dati
│   ├── Intel.CryoCooling.Configuration.dll  44 544  ** valori offset **
│   ├── Intel.CryoCooling.SafetyRules.dll  47 104  regole CB1/CB2/DT1
│   ├── safetyrules.dll                   124 416
│   ├── IntelCryoCooling.MLModel.dll      47 616  modello termico VT
│   ├── NLog.dll / Newtonsoft.Json.dll / ewma.dll
│   └── CryoCoolingService.exe.config      829
├── CryoCoolingNotifications.exe      1 075 664  UI tray
└── licenses/
```

**Prossimo file da aprire se si prosegue:**
`Intel.CryoCooling.Configuration.dll`, è dove stanno i valori di offset per
regime. È l'unico pezzo che manca per chiudere il menu.
