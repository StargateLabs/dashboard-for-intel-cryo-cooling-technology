# Codici errore e protezioni

Due livelli distinti, da non confondere:

- **il controller**: decide da solo, e in alcuni casi entra in Standby senza chiedere nulla
- **la dashboard**: sorveglia, dichiara, e in caso di errore ripetuto chiede un disable reale

Confondere i due fa diagnosticare il problema sbagliato.

---

## 1. Codici del controller

Fonte: manuale del produttore, sezioni 5.1 e 5.2.

### Regime e carico CPU

| Codice | Significato | Comportamento |
|---|---|---|
| **CB2** | *Unregulated mode suspended after an extended period of inactivity due to risk of condensation damage. The transition from Unregulated to Cryo mode.* | **Comportamento atteso**, non un errore. Causa: CPU idle (potenza sotto 25 W) per più di 10 minuti |
| **CB2** (seconda variante) | *The unregulated mode continues functioning after an extended period. Board remains in Unregulated mode.* | Anch'esso **atteso**. Causa: CPU non idle (potenza sopra 25 W) per più di 10 minuti |

La sospensione automatica dell'Unregulated dipende quindi dal **carico della CPU**, non da un timer
fisso. Sotto i 25 W il controller torna a Cryo, sopra i 25 W resta in Unregulated. È una protezione
interna all'hardware e non dipende da nessun programma.

### Sensore e sicurezza termica

| Codice | Significato | Conseguenza |
|---|---|---|
| **CF1 / CF2 / CF4** | guasto del sensore di temperatura | **transizione a Standby** |
| **CF3** | resistenza termica scarsa verso l'ambiente, ventilazione inefficace | guasto d'installazione, non del controller |
| **CF6** | errore ventola | **transizione a Standby** |
| **CF7** | errore pompa | **transizione a Standby** |
| **OT1** | blocco oltre **80 °C** misurati | Standby, spegnere e controllare |
| **OT2** | blocco oltre **90 °C** stimati | spegnimento |
| **OT3** | surriscaldamento estremo | spegnimento entro 5 secondi |
| **TD1** | termistore non funzionante | termistore guasto |
| **DT1 / DT2** | sanità dei sensori fuori specifica | installazione errata della cella |
| **CB1** | guasto dell'alimentazione del controller | reboot, staccare l'alimentazione |
| *Your sub-ambient is malfunctioning* | resistenza della TEC troppo alta | cella o cablaggio |

### Perché questa tabella cambia il progetto

Su **CF1/CF2/CF4, CF6 e CF7** il controller entra in Standby da solo. Non è un dettaglio: significa
che non si può usare un singolo sensore del PCB come unica guardia. Se quel sensore guasta, il
controller si protegge comunque, e l'applicazione deve accorgersene invece di tenere il TEC acceso.

Da qui la scelta di **più livelli indipendenti** nella dashboard: guardia anticondensa sul margine
di rugiada, guardia termica sul controller, watchdog che richiede un disable reale quando il
monitoraggio fallisce ripetutamente.

---

## 2. Soglie della dashboard

| Soglia | Valore | Ruolo |
|---|---|---|
| Guardia termica controller | 58 / 66 / 76 °C | tre livelli, sotto i limiti del firmware (80 °C e 90 °C) |
| Margine anticondensa | 1,0 °C | sotto questa soglia la potenza non sale |
| Margine di lavoro | 2,0 °C | obiettivo della piastra sopra la rugiada |
| Banda di lavoro | 2,5 – 3,0 °C | isteresi del regolatore, ±0,75 °C |
| Budget software | 100 % = 200 W | **obiettivo di retroazione**, non un tetto elettrico |

Questi numeri sono **scelte software**, non una taratura certificata di questo assemblaggio. Le
soglie esistenti non sono state ritarate senza misure.

Una soglia che non va mai alzata: il margine anticondensa. Con margine di 1 – 2 °C è l'unica cosa
che impedisce la condensa sulla piastra, e la condensa è il danno che uccide per primo.

---

## 3. Lo spegnimento non è un reset

Tre stati da tenere distinti, perché si confondono facilmente:

| Stato | TEC | Offset scrivibile | Comandi |
|---|---|---|---|
| **Cryo** | acceso | sì | `0x14`, `0x18 [0,0,0,0]` |
| **Unregulated** | acceso | sì, `-30.0` | `0x14`, `0x18 [0,0,0,0]` |
| **Standby** | **acceso** | sì, `3.5` | `0x14`, `0x18 [0,0,0,0]` |
| **Spento** | **spento** | **no** | `0x18 [1,0,0,0]` |

In Standby le ventole e la pompa **restano accese**: il manuale le definisce come il raffreddamento
che dà capacità di liquid cooling tipica senza il raffreddamento sub-ambiente. Standby e "spento"
sono quindi due stati distinti, non due nomi per la stessa cosa.

Lo spegnimento non scrive `0x14`: le impostazioni restano memorizzate perché spegnere non è
resettare.

---

## 4. Segnali da non interpretare come protezioni

### OCP

L'OCP si è acceso a **73 W** e a **112 W** con sistema sano e raffreddamento funzionante. Una
protezione che scatta a un terzo della potenza nominale non è una protezione.

Regola applicata nel software: **l'OCP non deve mai ridurre la potenza in modo persistente.** Se il
segnale viene ignorato dal firmware, l'intervento automatico è limitato a una sola occorrenza, senza
ripartenza automatica, perché un avviso che resta per sempre non è un avviso.

### Percentuale di potenza

Il controller legge percentuali che **non costituiscono un limite**: richiesto 30 %, letto 98 –
100 % con circa 246 – 260 W misurati. L'uguaglianza tra percentuale richiesta e duty letto non
conferma un cap. Le scritture di potenza passano quindi per un percorso di **readback**: un valore
ignorato dal firmware produce un errore, non un livello applicato fittizio.

### Tetto dichiarato del kit

L'etichetta "200 W" del kit Gen 1 **non è un tetto erogabile**. Il controller misurato regge
220 / 230 / 237 W in modo stabile, con raffreddamento regolare. La costante corrispondente nel
codice è un avviso e non deve mai bloccare.

---

## 5. Limiti dichiarati

- Nessun codice dei 14 bit sconosciuti viene interpretato.
- Il regime corrente è **dedotto**, non letto: i bit di stato sono latch e descrivono che cosa è
  stato impostato, non che cosa il modulo stia facendo.
- La percentuale di potenza non è un limite verificato su questo controller.
- Nessuna soglia termica è stata ritarata senza misure.
- Compilazione e test simulati non sostituiscono il collaudo del controller collegato.

---

## 6. Documenti collegati

| Argomento | File |
|---|---|
| Codici errore e limiti dichiarati | [`../SECURITY.md`](../../SECURITY.md) |
| Protocollo e opcode | [`../reverse-engineering/protocol.md`](../reverse-engineering/protocol.md) |
| Origine dei codici nel progetto | [`cella-peltier.md`](cella-peltier.md) §8 |
| Regimi e logica del controller | [`../reverse-engineering/cryo-gen1.md`](../reverse-engineering/cryo-gen1.md) |
| Prove su hardware reale | [`../releases/R2-gen1-tec2.md`](../releases/R2-gen1-tec2.md) |