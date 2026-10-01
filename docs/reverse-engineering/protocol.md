# Protocollo seriale Intel Cryo Cooling

Riferimento implementativo: `cryo_cooler_controller_lib/src/lib.rs`.
Ogni valore in questo documento è letto dal codice o dai binari del produttore, mai dedotto.

---

## 1. Livello fisico

| Parametro | Valore | Fonte |
|---|---|---|
| Ponte USB-seriale | **Silicon Labs CP210x** | `silabser.sys` presente nel pacchetto Intel, con `CP210xVCPInstaller_x64.exe` |
| Baud rate | **115200** | `lib.rs:432` e `lib.rs:449`, `set_baud_rate(serial::Baud115200)` |
| Formato | 8 bit, nessuna parità, 1 bit di stop (8N1) | default `serialport` |
| Timeout di risposta | 150 ms | `lib.rs:66`: ampio per un riscontro a 115200 baud |

La porta COM non è una UART nativa: è un ponte CP210x. Il programma parla quindi il protocollo
CP210x, che è quello implementato.

---

## 2. Frame

Pacchetto fisso di **8 byte**:

```
byte   0      1        2    3    4    5        6         7
      [0xAA] [opcode] [d0] [d1] [d2] [d3] [crc_lo] [crc_hi]
```

- `0xAA` è il byte di start fisso, identico in richiesta e risposta
- il CRC è **CRC16-XMODEM** calcolato sui primi 6 byte (`lib.rs:842`: `checksum(&buffer[0..6])`)
- il CRC viaggia in **little endian** (`lib.rs:854-855`: byte basso poi byte alto)
- la risposta è validata in tre passi: byte di start (`lib.rs:298`), CRC (`lib.rs:316`), opcode

```rust
// lib.rs:841-843
let buffer = [0xAA, op_code, data[0], data[1], data[2], data[3]];
let crc = CRC_16_XMODEM.checksum(&buffer);
```

---

## 3. Opcode

**21 opcode**, divisi dal nome del modulo in letture (`get`) e scritture (`set`).

### Letture (`commands::get`)

| Opcode | Costante | Grandezza |
|---|---|---|
| `0x00` | `HEART_BEAT` | stato completo, 32 bit |
| `0x01` | `TEC_TEMPERATURE` | temperatura lato freddo (piastra) |
| `0x02` | `HUMIDITY` | umidità relativa |
| `0x03` | `DEW_POINT` | punto di rugiada |
| `0x04` | `SET_POINT_OFFSET` | offset di setpoint in lettura |
| `0x05` | `P_COEFFICIENT` | coefficiente P |
| `0x06` | `I_COEFFICIENT` | coefficiente I |
| `0x07` | `D_COEFFICIENT` | coefficiente D |
| `0x08` | `TEC_POWERLEVEL` | percentuale di potenza letta |
| `0x09` | `HW_VERSION` | revisione hardware |
| `0x0A` | `FW_VERSION` | versione firmware |
| `0x1B` | `NTC_COEFFICIENT` | coefficiente del termistore |
| `0x1F` | `BOARD_TEMP` | temperatura del controller |
| `0x22` | `VOLTAGE_AND_CURRENT` | tensione e corrente in un colpo |
| `0x23` | `TEC_VOLTAGE` | tensione TEC |
| `0x24` | `TEC_CURRENT` | corrente TEC |

### Scritture (`commands::set`)

| Opcode | Costante | Ruolo | Conseguenza se mal interpretato |
|---|---|---|---|
| `0x14` | `POINT_OFFSET` | scrive l'offset di setpoint | **azzera il setpoint** |
| `0x15` | `P_COEFFICIENT` | coefficiente P | **azzera il guadagno P** |
| `0x16` | `I_COEFFICIENT` | coefficiente I | **azzera il guadagno I** |
| `0x17` | `D_COEFFICIENT` | coefficiente D | **azzera il guadagno D** |
| **`0x18`** | `DISABLE_NOT_ENABLE` | `[0,0,0,0]` **abilita** il TEC, `[1,0,0,0]` **disabilita** | polarità opposta al nome del metodo |
| `0x19` | `CPU_TEMP` | temperatura CPU al PID | il regolatore lavora su un dato falso |
| `0x1A` | `NTC_COEFFICIENT` | NTC | nessuna |
| `0x1C` | `TEMP_SENSOR` | modalità sensore | nessuna |
| `0x1D` | `TEC_POWER_LEVEL` | tetto di potenza | **porta il tetto a 0 %** |
| **`0x1E`** | `RESET_BOARD` | **reset di fabbrica** | **perde PID, setpoint e tetto** |

I due opcode da conoscere prima degli altri sono `0x18` e `0x1E`, perché entrambi hanno dati
nulli e effetti opposti a quanto suggerisce il nome.

---

## 4. Allowlist di sola lettura

Gli opcode che la sonda `probe_opcodes` è autorizzata a inviare sono definiti in `lib.rs:33`:

```rust
const SOLO_LETTURE: &[std::ops::RangeInclusive<u8>] = &[
    commands::HEART_BEAT..=commands::get::FW_VERSION,        // 0x00..=0x0A
    commands::get::NTC_COEFFICIENT..=commands::get::NTC_COEFFICIENT, // 0x1B
    commands::get::BOARD_TEMP..=commands::get::BOARD_TEMP,         // 0x1F
    commands::get::VOLTAGE_AND_CURRENT..=commands::get::TEC_CURRENT, // 0x22..=0x24
];
```

Sono **17 opcode** di lettura. `0x14..=0x17` compaiono in `set` e non in `get`: i due gruppi non
sono disgiunti per numero, quindi l'allowlist è scritta a partire dai valori leggibili nella tabella
dei comandi, non da una supposizione sul layout.

La garanzia è strutturale, non convenzionale: `probe_plan` (`lib.rs:52`) filtra ogni opcode
prima di costruire il frame, quindi **qualsiasi argomento passato dal chiamante** viene scartato se
non è in lista.

```rust
// lib.rs:52-54
fn probe_plan(da: u8, a: u8) -> Vec<u8> {
    (da..=a).filter(|op| is_solo_lettura(*op)).collect()
}
```

Il test associato verifica che `0x18` con dati nulli, che pure abilita il TEC, non venga mai
prodotto dal piano di sonda.

---

## 5. Bit di stato

Il campo di risposta è a **32 bit**. Il codice ne decodifica **18**, da bit 0 a bit 17:

| Bit | Costante | Significato |
|---|---|---|
| 0 | `BOARD_INIT` | inizializzazione completata |
| 1 | `POWER_OK` | alimentazione rilevata |
| 2 | `TEMP_SENSE_OK` | sensore di temperatura in range |
| 3 | `HUM_SENSE_OK` | sensore di umidità in range |
| 4 | `LAST_CMD_OK` | ultimo comando accettato |
| 5 | `LAST_CMD_BAD_CRC` | ultimo comando rifiutato per CRC |
| 6 | `LAST_CMD_INCOMPLETE` | ultimo comando incompleto |
| 7 | `FAILSAFE_ACTIVE` | failsafe attivo |
| 8 | `PID_READY` | PID pronto |
| 9 | `PID_INVALID` | PID non valido |
| 10 | `PID_OUT_OF_RANGE` | PID fuori range |
| 11 | `PID_DEFAULT` | PID di default in uso |
| **12** | **`PID_RUNNING`** | **il regolatore è in marcia** |
| 13 | `OCP_ACTIVE` | overcurrent attivo |
| 14 | `BOARD_TEMP_OK` | temperatura scheda in range |
| 15 | `TEC_CONN_OK` | connessione TEC ok |
| 16 | `LOW_POWER_MODE_ACTIVE` | low power mode attivo (standby) |
| 17 | `TEMP_MODE` | modalità temperatura registrata |

Restano **14 bit non decodificati** (`bit 18..31`).

---

## 6. Discrepanza aperta: il bit di `PID_RUNNING`

`docs/reverse-engineering/cryo-gen1.md` §4 scrive che `PID_RUNNING` è il **bit 2**. Il codice in
`lib.rs:820` assegna invece:

```rust
const TEMP_SENSE_OK = 1 << 2;   // bit 2
const PID_RUNNING    = 1 << 12;  // bit 12
```

Le due fonti non concordano. La lettura del regime corrente dipende da questo bit, quindi la
discrepanza non è trascurabile.

Non è stata arbitrata in codice perché non è stata misurata: serve una tabella di osservazioni
sulla board reale. Fino ad allora la dashboard **espone entrambi i bit** e non dichiara quale sia
`PID_RUNNING`. Un regime dedotto è mostrato come dedotto, non come fatto.

---

## 7. I 14 bit sconosciuti

Il controller risponde al heartbeat con 32 bit, l'app ne legge 18. I restanti 14 sono mascherati
prima della decodifica e **non vengono interpretati**: sono registrati grezzi a ogni campione in
`%LOCALAPPDATA%\stargate-cryo\bit-stato.csv`, per una correlazione futura con l'OCP.

Il criterio è dichiarato nel progetto e vale per tutti i 14 bit:

> Un codice errato in diagnostica fa diagnosticare il problema sbagliato, che è peggio di non
> avere nessuna diagnostica.

---

## 8. Sequenza di commutazione

`InitCryoMode`, `InitUnregulatedMode` e `InitStandbyMode` del software vendor non sono
un interruttore: sono una sequenza di 3 passi.

```
1. SetSetPointOffset(offset)   opcode 0x14, float32 LE
2. SetLowPowerMode(bool)       opcode 0x18
3. GetBoardStatus()            opcode 0x00, per CONFERMARE
```

Il passo 3 è quello che rende l'interfaccia onesta: se il controller non conferma, l'interfaccia
deve dire "non commutato", non "fatto".

Lo **spento** usa un solo comando, `0x18 [1,0,0,0]`, senza `0x14`: su questo bus ogni byte in più
può essere il reset di fabbrica, e un controller spento non applica comunque un setpoint.

---

## 9. Costanti hardware documentate

| Costante | Valore | Significato |
|---|---|---|
| `CRC_16_XMODEM` | tabella `crc` | checksum del frame |
| fattore ADC tensione | `21.1` | conversione raw → volt |
| fattore ADC corrente | `4.6545` | conversione raw → ampere |
| `SOFT_START_POWER` | `30` | potenza iniziale in soft start |
| `CPU_THERMAL_CONDUCTANCE` | `12.0` | **ipotizzata**, serve solo al COP stimato |

`CPU_THERMAL_CONDUCTANCE` non è una misura. Il COP mostrato è
`max(0, 12 × (CPU - piastra) / watt)` limitato a 5, ed è etichettato come stima nell'interfaccia.

---

## 10. Collegamento alla sorgente

| Argomento | File |
|---|---|
| frame, CRC, byte di start | `cryo_cooler_controller_lib/src/lib.rs` |
| allowlist `SOLO_LETTURE`, `probe_plan` | `cryo_cooler_controller_lib/src/lib.rs` |
| costanti opcode | `cryo_cooler_controller_lib/src/lib.rs`, modulo `commands` |
| bit di stato | `cryo_cooler_controller_lib/src/lib.rs`, `bitflags! TecStatus` |
| sequenza di commutazione e conferma | `cryo_cooler_controller/src/commutazione.rs` |
| attore di scrittura unico | `cryo_cooler_controller/src/attore_tec.rs` |
| origine dei dati e limiti | [`cryo-gen1.md`](cryo-gen1.md) |
| errori e limiti dichiarati | [`../SECURITY.md`](../../SECURITY.md) |