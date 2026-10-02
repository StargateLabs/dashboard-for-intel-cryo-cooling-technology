# Analisi e ottimizzazione della cella Peltier: TEC CryoCooler

Data: 2026-09-28 · Hardware: controller Delta², modulo TEC V2, soglia Standby 80 °C, shutdown 90 °C

## Scopo

Capire dove si perde prestazione e dove si produce calore inutile, usando **solo misure che
il programma legge già** (`tec_voltage`, `tec_current`, `tec_power_watts`
`tec_power_level`, `tec_temperature`, `pcb_temperature`), e proporre regolazioni che
migliorino il raffreddamento e riducano il calore dissipato.

---

## 1. La legge che governa una cella Peltier

Una cella Peltier traspira calore dal lato freddo a quello caldo, ma **pompa anche calore
dal lato caldo a quello freddo**, e consuma potenza elettrica per farlo.

Il punto centrale, spesso ignorato

> Il COP di una cella Peltier non è massimo alla massima potenza. Anzi, in genere
> **il COP crolla** quando si alza la corrente, e il prodotto "potenza elettrica" cresce
> più in fretta del "calore utile spostato".

Conseguenza pratica: **a parità di temperatura sul lato freddo, un punto di funzionamento
più basso consuma meno e scalda meno.** L'errore tipico è inseguire la potenza massima.

Il COP di Carnot dà il limite teorico

```
COP_max = T_freddo / (T_caldo - T_freddo)        (temperature in Kelvin)
```

Con lato freddo a 5 °C (278 K) e lato caldo a 40 °C (313 K): COP_max ≈ 0,79. Con il lato
caldo a 50 °C (323 K): COP_max ≈ 0,61. **Il solo riscaldamento del lato caldo taglia il
rendimento di circa un quarto** senza cambiare nulla sulla piastra.

## 2. Cosa misura oggi il programma, e cosa manca

Già disponibile a ogni campione (2 Hz)

| Grandezza | Uso nella regolazione attuale |
|---|---|
| `tec_temperature` | lato freddo (piastra) |
| `pcb_temperature` | lato caldo del modulo → **è il segnale che comanda la potenza** |
| `tec_voltage`, `tec_current` | solo per la potenza in watt e il COP stimato |
| `tec_power_level` | il cap impostato |

**Manca la misura più importante per l'efficienza: la tensione a vuoto della cella.** Il
rapporto tra tensione a vuoto e tensione di lavoro indica a che regime la cella sta
operando, ed è la misura che distingue "cella che lavora bene" da "cella che brucia watt".

**Manca anche la corrente effettiva a carico noto**, per capire se il pilotaggio è
lineare o se satura.

## 3. Difetti concreti della regolazione attuale

### 3.1 La potenza è scelta dal freddo, non dal rendimento

La guardia (`update_ctrl_guard`) regola sul `pcb_temperature` con soglie fisse a 58/66/76 °C
e gradini di ±1/±2%. Funziona per la **protezione**, ed è giusto così: la priorità è non
bruciare il modulo.

Ma non è un regolatore di **rendimento**. Conseguenze

- quando il lato caldo è già caldo e il freddo non scende, la guardia continua a salire di
 1% per tick fino al cap, consumando watt per un guadagno nullo;
- il punto di funzionamento non tiene conto di quanto calore sta effettivamente
 pompando nel lato caldo.

### 3.2 Il pavimento a 50% è pensato per non spegnere, ma costa

`CTRL_MIN_POWER = 50` serve a evitare che il controller smetta di lavorare. È una scelta
conservativa corretta. Però: **a 50% di potenza il lato caldo del modulo si scalda comunque**
e il modulo non ha un modo di raffreddarsi da solo. Il pavimento andrebbe abbassato quando
il lato freddo è già stabilizzato, non mantenuto costante.

### 3.3 Il COP stimato non serve a pilotare

`CopState::calculate` stima il calore rimosso come `ΔT × 12 W/°C`, con una costante
`CPU_THERMAL_CONDUCTANCE` fissa e non configurabile. Va bene per **mostrare** un numero
all'utente, ma **non è una misura**: se il valore reale è 20 W/°C, il COP mostrato è
errato di quasi il doppio, e la percentuale rispetto a Carnot pure.

## 4. Cosa si può fare, in ordine di rendimento per rischio

### Fase 1: Solo misura, nessun cambio di comportamento (rischio zero)

Due strumenti che non toccano la potenza e valgono più di ogni altro intervento

1. **Registrare la curva di funzionamento reale**: per ogni `tec_power_level` in 0..100
 annotare `V`, `A`, `W`, `T_freddo`, `T_caldo` in regime stabile. Da questa tabella si
 vede il punto di massimo rendimento, e non lo si deve dedurre dalla teoria.

2. **Mostrare il rapporto V/A** nella schermata diagnostica: un calo netto del rapporto
 segnala saturazione o surriscaldamento, che è la causa tipica del "il TEC non raffredda
 più".

### Fase 2: Regolazione sul rendimento (rischio basso, guadagno reale)

Sostituire la salita "a gradini ciechi" con una scelta del punto di funzionamento

- **se il lato freddo non migliora da 2-3 tick e il lato caldo è salito**, la potenza
 attuale sta pompando calore senza raffreddare: **scendere**, non salire;
- **se il lato freddo migliora e il lato caldo è stabile**, si può salire;
- **a parità di ΔT sul freddo, preferire sempre il punto di potenza più basso** che
 mantiene quel ΔT.

Questo è il cuore del guadagno: stessa temperatura, meno watt, meno calore.

### Fase 3: Lato caldo (il guadagno più grande, e non è software)

Il limite del COP lo fissa il lato caldo, e lì c'è da guadagnare

- **flusso d'aria sul dissipatore**: è il parametro che più influenza il COP, più della
 potenza scelta. Una ventola più efficace o un radiatore più grande riducono il lato caldo
 di parecchi gradi;
- **pulizia del dissipatore e del flusso**: il caso più comune di "il TEC non raffredda
 come prima" è il radiatore impolverato o l'aria che non circola;
- **isolamento dal lato freddo**: perdite parassite dal lato freddo verso l'ambiente
 annullano parte del lavoro della cella, e sono interventi meccanici.

Per dare un ordine: con lato caldo a 40 °C il COP massimo è ~0,79; portandolo a 33 °C
(~306 K) sale a ~1,04, cioè **+32% di prestazione utile a parità di watt**. Nessun
algoritmo arriva lontano.

## 5. Perché serve misurare, e cosa NON prometto

Non prometto percentuali di miglioramento, e il motivo è tecnico: **senza la tabella di
funzionamento reale** ogni stima è una ipotesi. Le misure disponibili bastano per la Fase 1
e la Fase 2, che sono le uniche che si possono fare con sicurezza da qui.

Le soglie di protezione (58/66/76 °C, pavimento 50%, guardia anticondensa) **non vanno
toccate**: sono ciò che tiene vivo l'hardware, e sono state verificate su questo modulo.

## 6. Ordine di lavoro consigliato

1. Registrare la curva reale (Fase 1), si fa girando il TEC su più potenze e annotando
 V/A/T con la diagnostica. Nessun rischio.
2. Da quella tabella, scegliere il punto di massimo rendimento e impostarlo come cap
 consigliato, invece del 100%.
3. Solo dopo, la regolazione "scendi se non migliora" (Fase 2).
4. Il lato caldo (Fase 3) è lavoro meccanico, non software.

## 7. Cosa non va fatto

- **Non alzare le soglie di protezione** per "raffreddare di più": il firmware taglia a
 80 °C e 90 °C per proteggere il modulo, e la guardia software sta volutamente sotto.
- **Non toccare il piano anticondensa**: con margine di ~1-2 °C è l'unica cosa che
 impedisce la condensa sulla piastra, e la condensa è il danno che uccide per primo.
- **Non inseguire la potenza massima**: oltre il punto di rendimento si spendono watt e si
 scalda il lato caldo per guadagno nullo o negativo.

## 8. Codici errore e comportamenti del controller (manuale EK, sezioni 5.1 e 5.2)

Raccolti dal manuale del produttore. Servono a distinguere "il sistema ha un
problema" da "il controller ha un problema": sono due cose diverse, e la
prima si risolve, la seconda no.

### Modalita': quando l'Unregulated torna da solo a Cryo

- **CB2**, *"Unregulated mode suspended after an extended period of inactivity
 due to risk of condensation damage. The transition from Unregulated to Cryo
 mode."* Causa: **CPU idle (potenza < 25 W) per piu' di 10 minuti**.
 **Comportamento atteso**, non un errore.
- **CB2** (seconda variante), *"The unregulated mode continues functioning
 after an extended period. Board remains in Unregulated mode."* Causa: **CPU non
 idle (potenza > 25 W) per piu' di 10 minuti**. Anche questo **e' atteso**.

Quindi la sospensione automatica dell'Unregulated dipende dal **carico della
CPU**, non da un timer fisso: con la CPU sotto 25 W il controller riporta a
Cryo, con la CPU sopra 25 W resta in Unregulated. E' una protezione, ed e'
perche' il pulsante di uscita verso Cryo e' sempre disponibile.

### Sensore e sicurezza termica

| Codice | Significato | Conseguenza |
|---|---|---|
| **CF1 / CF2 / CF4** | guasto del sensore di temperatura | **transizione a Standby** |
| **CF3** | resistenza termica scarsa verso l'ambiente (ventola o pompa inefficienti) | guasto d'installazione, non del controller |
| **CF6** | errore ventola | **transizione a Standby** |
| **CF7** | errore pompa | **transizione a Standby** |
| **OT1** | blocco oltre **80 °C** misurati | Standby, spegnere e controllare |
| **OT2** | blocco oltre **90 °C** stimati | spegnimento |
| **OT3** | surriscaldamento estremo | spegnimento in 5 secondi |
| **TD1** | termistore non funzionante | termistore guasto |
| **DT1 / DT2** | sanita' dei sensori fuori specifica | installazione errata della cella |
| **CB1** | guasto dell'alimentazione del controller | reboot, staccare l'alimentazione |
|, | "Your sub-ambient is malfunctioning" | **resistenza della TEC troppo alta** |

**Punto importante:** su CF1/CF2/CF4, CF6 e CF7 il controller entra in
**Standby da solo**. E' per questo che in questo progetto non si puo' usare un
singolo sensore del PCB come unica guardia: se il sensore guasta, il
controller si protegge da solo, e l'app deve accorgersene invece di tenere
il TEC acceso.

### Software Intel Cryo Cooling: quali CPU supporta

Solo **Intel di 10a, 11a e 12a generazione**. Su qualsiasi altra CPU
l'applicazione si rifiuta di partire con *"This cooling solution is not
supported on this processor. Please uninstall the application"*.

Su una CPU di 14a generazione questo non e' un bug: e' la condizione prevista
dal produttore. Per questo la nostra app **non tenta di replicare il software
ufficiale**, e non inventa comandi per le modalita': nel protocollo che
conosciamo non esiste il comando di cambio modalita'.

### I 14 bit di stato non decodificati

Il controller risponde al heartbeat con un campo a **32 bit**, ma l'app ne
leggeva 18. I restanti 14 venivano mascherati via prima della decodifica, e
nessuno dei 18 bit noti corrisponde ai codici qui sopra: e' plausibile che i
codici stiano in quei 14 bit.

Non li interpretiamo. Sono registrati in
`%LOCALAPPDATA%\stargate-cryo\bit-stato.csv` a ogni campione, e la
correlazione con l'OCP (che si accende anche a 73 W) potrebbe chiarire cosa
significano. **Finche' non sappiamo, non si inventa una decodifica**: un
codice errato in diagnostica fa diagnosticare il problema sbagliato, che e'
peggio di non avere nessuna diagnostica.

## 9. Cosa è stato verificato nel software Intel (reverse engineering, 2026-09-28)

Il pacchetto Gen 1 (`intel_r_cryo_cooling_technology_v1.1.0.319_release.exe`)
è stato estratto e letto. **Nessun file è stato eseguito.** I file rilevanti
sono dentro `Intel(R) Cryo Cooling Technology1.cab`.

### 9.1 La modalità NON è un comando seriale

`IntelCryoCooling.Controller.dll` (82 KB) espone due scritture sulla modalità

- `SetLowPowerMode`
- `SetTemperatureSensorMode`

**Non c'è `SetMode`.** Nessuno dei 16 moduli estratti contiene le stringhe
`Unregulated`, `Standby` o `Cryo mode`: la modalità non passa dalla seriale.

È lo stato di un **pin GPIO** del controller, letto e scritto dal driver
`CCHWApiExt.sys` (`kHWAPIReadGPIO` / `kHWAPIWriteGPIO`) e dalla libreria
`TATInterface.dll` (`MMIOReadGPIO` / `MMIOWriteGPIO`, 1,2 MB).

Conseguenza per questo progetto: **non esiste un opcode da aggiungere al
protocollo.** Non è un comando che non siamo riusciti a trovare: non esiste
su quel canale. Le tre modalità sono la stessa operazione di bassa potenza
interpretata dal controller in base al cablaggio del dissipatore.

Ecco perché il software Intel *legge* la modalità invece di impostarla: la
riga `Cooler is in standby mode` è un messaggio di stato, non un'azione.

**Chi sceglie la modalità, quindi, è l'hardware.** In base al carico della CPU

- **CPU sotto 25 W** per più di 10 minuti → l'Unregulated viene sospeso e si
 torna a Cryo
- **CPU sopra 25 W** per più di 10 minuti → l'Unregulated continua

La protezione è dentro il controller e non dipende da nessun programma.

### 9.2 Perché il software non parte sul 14900KS

Lo script VBScript di controllo del pacchetto Gen 1 contiene

```vb
CPUList = Array("10900K", "10850K", "10700K", "10600K", "10900KF",
                "10700KF", "10600KF", "11600K", "11700K", "11900K",
                "11600KF", "11700KF", "11900KF", "8700K", "8086K",
                "9600K", "9900K", "9700K", "9900KS", "9900KF", "9700KF")
```

23 modelli, **tutti di 10ª generazione**. Lo script elimina `@`, `GHz`, `MHz`
`CPU`, `Core(TM)`, `Edition`, `Extreme`, `Quad`, e confronta **per testo
esatto**: `Intel(R) Core(TM) i9-14900KS` diventa `i914900KS`, che non è nella
lista.

Non è un blocco aggirabile aggirando l'installazione. Il messaggio
`This processor is not supported.` sta nel pacchetto di setup e non nel
programma, e le proprietà che lo governano sono `FOUND_CPU` e `VALID_CPU`,
ma ricevono il risultato della lettura del processore, che per un 14900KS
dà "non è nella lista". Per farlo passare servirebbe cambiare come
`SoftwareDetector.dll` legge il processore, e non serve a nulla: per il
punto 9.1 quel software non imposta comunque la modalità.

### 9.3 Le ventole le pilota la scheda madre

Il connettore a **5 pin** della board EK va alla scheda madre e trasporta PWM
e tach: la scheda madre pilota le ventole, la board le fa passare. Confermato
anche dal menu del software ufficiale, che espone solo `Mode`, `Help`
`About` ed `Exit`: se le ventole fossero comandabili via software ci sarebbe
un `Fans` o `Pump` accanto a `Mode`.

Il manuale raccomanda comunque di **non** usare la temperatura della CPU come
riferimento per ventola e pompa: il TEC raffredda la CPU, quindi il resto del
circuito di liquido riceve il calore tolto e il segnale CPU è falsato. Il
testo è: *"Do not use your CPU temperatures as a baseline for your Fan or
Pump speeds!"*. L'utente le imposta a velocità fissa, che è la strada
indicata. Se un giorno servisse una curva, il posto giusto è il BIOS della
scheda che pilota le ventole, non la dashboard.

### 9.4 Il controller è un ponte Silicon Labs CP210x

`silabser.sys` è nella lista dei file del pacchetto, con
`CP210xVCPInstaller_x64.exe`: la porta COM è un ponte USB-seriale Silicon
Labs, non una UART nativa. Il programma deve quindi parlare il protocollo
CP210x, che è quello che già facciamo.

### 9.5 Cosa è stato escluso, e come

Per non ripetere la ricerca, con i metodi che **non** hanno funzionato

- `py7zr` non gestisce il filtro **BCJ2**: non può aprire gli archivi 7z di
 questi pacchetti. Le librerie Python disponibili non lo coprono.
- I due installer non sono archivi apribili con `7zr`: sono pacchetti
 bootstrapper. `7za` li rifiuta con "Cannot open the file as archive".
- I tre cab interni si aprono **solo** tagliandoli a byte precisi e passando
 l'offset giusto a `7za`: con `cabarchive` falliscono per checksum.
- L'estrazione l'ha fatta l'installer stesso con `/extract <cartella>`, che
 non installa niente. È l'unica strada trovata per arrivare ai file.
