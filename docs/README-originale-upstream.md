# STARGATE labs — Cryo Cooler Controller v2.0

> Stargate Labs Edition — fork avanzato del progetto open source originale di [juvgrfunex](https://github.com/juvgrfunex/cryo-cooler-controller)

Software alternativo per il controllo di cooler TEC basati su **Intel Cryo Cooling Technology**. A differenza del software ufficiale Intel, non impone restrizioni artificiali sul modello di CPU e aggiunge funzionalità avanzate di monitoraggio, diagnostica e tuning.

---

## Cooler supportati

| Cooler | Note |
|---|---|
| CoolerMaster MasterLiquid ML360 SUB-ZERO | Testato |
| EK-QuantumX Delta TEC EVO | Compatibile |
| EK-Quantum Delta² TEC D-RGB | Compatibile |

---

## Novità v2.0 — Stargate Labs Edition

### Bug fix rispetto a v1.0.2
- **OCP false positive risolto**: il warning `OCP ACTIVE` ora richiede 3 letture consecutive positive prima di essere mostrato. Elimina i falsi positivi del firmware.
- **Timeout seriale esplicito**: 1000ms — previene blocchi indefiniti del thread se il device non risponde.
- **CRC opcode fix**: uso di `wrapping_add(127)` per evitare overflow u8 su opcode alti.

### Nuove feature
- **Potenza TEC in Watt** (`V × I`): calcolata e mostrata in tempo reale, loggata nel CSV.
- **Condensation Margin**: `TEC_temp − dew_point` con indicator colorato (verde/giallo/rosso).
- **Session Stats**: min/avg/max di TEC temp, potenza e margine di condensazione per tutta la sessione.
- **Export CSV** → Desktop: tutti i canali inclusi `cpu_temp_ext_C` da HWiNFO.
- **Profili PID**: salvataggio/caricamento da `%APPDATA%\StargateLabsCryo\config.json`, con 3 preset (Idle, Gaming, AI/Rendering).
- **HWiNFO64 integration**: lettura automatica CPU temp da shared memory (`Global\HWiNFO_SENS_SM2`) ogni 5 secondi, inviata al device via `setCpuTemp` (opcode 0x19) per PID corretto su CPU AMD e non-natively-supported Intel.
- **Tema neon green** Stargate Labs: palette completa scura con accenti `#00FF41`.

---

## Prerequisiti

### Driver CP210x (SiLabs USB-UART)
Incluso in Windows 10/11. Se assente: [download SiLabs](https://www.silabs.com/developers/usb-to-uart-bridge-vcp-drivers?tab=downloads)

### HWiNFO64 (opzionale — per CPU temp su AMD/sistemi non supportati)
1. Apri HWiNFO64 → **Settings → General → ✓ Shared Memory Support**
2. Il controller rileva automaticamente la shared memory e inietta la temp nel PID del device.

---

## Compilazione

```powershell
# Prerequisiti: Rust 1.75+ (https://rustup.rs)
cd stargate-cryo
cargo build --release
# Eseguibile: target\release\cryo_cooler_controller.exe
```

---

## Utilizzo

1. Collega il cooler via USB
2. Avvia `cryo_cooler_controller.exe`
3. Seleziona la porta COM dal dropdown (se incerto, scollega/ricollega e guarda quale sparisce)
4. Click **⚡ Connect**
5. Imposta Offset e Max Power, poi **▶ Enable TEC**
6. I grafici si aggiornano in tempo reale a 2 Hz

### Profili PID consigliati

| Scenario | P | I | D | Offset | Max Power |
|---|---|---|---|---|---|
| Idle / Silent | 80 | 1.0 | 0 | +2°C | 60% |
| Gaming | 120 | 1.5 | 0 | -3°C | 90% |
| AI / Rendering | 150 | 2.0 | 0.5 | -5°C | 100% |

---

## FAQ

**Q: "Access is denied" sulla porta COM**
A: Il software ufficiale Intel occupa la porta. Chiudilo o disabilita il servizio `IntelCryoCooling` da services.msc.

**Q: OCP ACTIVE warning**
A: Il warning appare solo dopo 3 letture consecutive (debounce). Se compare, ridurre Max Power e verificare il cablaggio del TEC.

**Q: CPU temp non rilevata**
A: Abilitare "Shared Memory Support" in HWiNFO64. Il campo `CPU Ext` nella sidebar mostra lo stato.

**Q: Supporto AMD / CPU non ufficialmente supportate**
A: Il software funziona su qualsiasi CPU. Con HWiNFO attivo, il PID riceve la temperatura reale. Senza HWiNFO, il device usa il suo sensore NTC integrato.

---

## Protocollo seriale (riferimento)

`115200 baud, 8N1 — pacchetti da 8 byte: [0xAA][opcode][data×4][CRC16-XMODEM×2]`

Tutte le implementazioni sono in `cryo_cooler_controller_lib/src/lib.rs`.

---

## Licenza

MIT — fork di [juvgrfunex/cryo-cooler-controller](https://github.com/juvgrfunex/cryo-cooler-controller)
Tema e feature aggiuntive: Stargate Labs

---

## Novità v2.4 — Fix & Improvements

### Bug fix critici
- **BUG #1 — Timing miscalibrazione** (running.rs): tutti i contatori `tick_count` / `fan_update_ticks` / `overlay_ticks` erano calibrati per 20–100 Hz ma il gate è a 2 Hz. Tutti i valori modulo corretti (`% 200 → % 4`, `% 6000 → % 120`, ecc.)
- **BUG #2 — CondensationRisk sempre Critical** (running.rs): `analyze(0.01)` → `analyze(0.5)` — il trend_per_sec era 50× overestimato
- **BUG #3 — AI Advisor dati falsi** (running.rs): `dew_point: 0.0` e `humidity: 0.0` hardcoded nel SessionSnapshot → ora letti dall'ultimo LogEntry
- **BUG #4 — Config directory split** (ai_advisor.rs): `%APPDATA%\StargateCryo\` → `%APPDATA%\StargateLabsCryo\` — unificato con config.json e sessions.db
- **BUG #5 — PID Wizard step 25 min** (pid_wizard.rs): `step_duration_ticks: 3000` (100fps) → `60` (2Hz) — ora lo step dura 30s
- **BUG #6 — Log Vec illimitato** (running.rs): cap a 10.000 entries con sliding window
- **BUG #7 — extract_params() fragile** (ai_advisor.rs): parsing token-based con `split_once('=')`, cerca solo nella sezione "PARAMETRI SUGGERITI:"
- **BUG #8 — Response start byte non validato** (lib.rs): aggiunto check `buffer[0] != 0xAA`

### Miglioramenti
- **Modello AI** aggiornato a `claude-sonnet-4-6`
- **export_csv** ora restituisce `Result` con feedback nell'UI (usa il campo pdf_status)
- **Dead code rimosso**: blocco `if false`, variabili `_target_speed`, `_mode_str`, `_gpu_load`, `parse_after`
- **Costanti hardware documentate**: fattori ADC `21.1` e `4.6545` in lib.rs con spiegazione circuito
- **COP formula**: costante `CPU_THERMAL_CONDUCTANCE = 12.0` nominata e documentata con valori per CPU comuni

### Nuove feature
- **FEAT #1 — Auto-save CSV** ogni 5 minuti in `%APPDATA%\StargateLabsCryo\autosave.csv` (protezione crash OC)
- **FEAT #3 — GPU temp in AI Advisor**: lettura GPU temperature da HWiNFO, inclusa nel SessionSnapshot e nel prompt AI
- **FEAT #4 — AutoProfiler configurabile**: `hysteresis`, `cpu_ai_threshold`, `gpu_game_threshold` ora sono campi pubblici modificabili
- **FEAT #5 — Alert PumpRpm + azione emergenza**: nuovo `AlertChannel::PumpRpm` — se la pompa scende sotto 200 RPM, riduce automaticamente TEC al 50% e invia toast
