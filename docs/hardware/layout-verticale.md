# Layout per monitor verticali

La dashboard nasce per schermi **verticali** (1080 × 1920 e simili), dove una griglia a colonne
sarebbe sprecata. Questo documento descrive la disposizione reale e le scelte dietro.

Screenshot di riferimento: [`../images/dashboard-verticale.jpg`](../images/dashboard-verticale.jpg)
(build `r143`, controller Gen 1 `HW 4`, firmware `13.A0`).

---

## 1. Struttura a due colonne

```
┌─────────────────────┬──────────────────────────────────────────────┐
│  SIDEBAR  ~26 %     │  COLONNA FLUIDA                              │
│                     │                                              │
│  testata + logo     │  pannello STRUMENTAZIONE (LIVE)               │
│  stato piastra      │  ┌────────────────────────────────────────┐  │
│  margine / condensa │  │ TERMICA CRITICA   TEC · CPU · Rugiada  │  │
│  CPU / pompa        │  └────────────────────────────────────────┘  │
│  statistiche        │  Temp. TEC          · 18,62 °C              │
│  controlli TEC      │  Punto di rugiada   · 14,95 °C              │
│  offset / budget    │  ┌────────────────────────────────────────┐  │
│  PID P/I/D          │  │ ELETTRICA        Tensione · Corrente    │  │
│  profili            │  └────────────────────────────────────────┘  │
│  auto-tuning        │  Tensione TEC       · 5,97 V                │
│  integrazioni       │  Corrente TEC       · 12,03 A               │
│  export             │  ┌────────────────────────────────────────┐  │
│                     │  │ POTENZA         Potenza TEC · Duty      │  │
│                     │  └────────────────────────────────────────┘  │
│                     │  Potenza TEC        · 71,85 W               │
│                     │  Livello potenza    · 53,00 %               │
│                     │  ┌────────────────────────────────────────┐  │
│                     │  │ AMBIENTE        Umidità · Temp. scheda │  │
│                     │  └────────────────────────────────────────┘  │
│                     │  Umidità            · 41,50 %               │
│                     │  Temp. scheda       · 29,39 °C              │
│                     │  HWiNFO64 / AIDA64 (27 sensori)             │
│                     │  griglia sensori CPU / scheda / memoria      │
└─────────────────────┴──────────────────────────────────────────────┘
```

La sidebar è **fissa**: il suo contenuto non scorre con la colonna destra, perché i controlli
critici (abilita TEC, offset, budget, PID) devono restare raggiungibili senza scorrere.

La colonna destra è **fluida** e contiene gli 8 grafici impilati, raggruppati in quattro blocchi
tematici.

---

## 2. Asse temporale condiviso

Tutti gli 8 grafici usano **lo stesso asse del tempo**. Nello screenshot le tacche leggibili sono
`22:45:00`, `22:46:00`, `22:47:00`.

È la scelta che rende leggibile un pannello così denso: il lettore può confrontare verticalmente
tensione, corrente e potenza senza dover cercare l'allineamento temporale in ogni grafico.

Il tooltip di ogni singolo grafico individua il campione usando **l'asse temporale reale**, non
distribuendo uniformemente il numero di campioni sulla larghezza.

---

## 3. Presentazione delle curve

| Proprietà | Valore | Motivo |
|---|---|---|
| Frequenza di animazione | ~30 fps (33 ms) | fluido senza saturare la GPU |
| Ritardo visivo | 1000 ms | la curva non tremola sull'ultimo campione |
| Interpolazione | lineare, **tra campioni reali** | nessuna estrapolazione |
| Cache geometria | sì | evita di ricostruire il grafico più volte per frame |
| Campioni non finiti | esclusi | non guastano scala né rendering |
| MSAA | disattivato | il costo non ripagava il beneficio |
| Timer in Home / tray | **spento** | il polling e le protezioni mantengono la loro cadenza |

Il ritardo visivo **non** tocca i dati: i valori live, i controlli TEC, gli allarmi, i log e le
esportazioni restano immediati. È solo la presentazione che è addolcita.

Nella versione precedente a 500 ms era presente una traslazione artificiale del timestamp
dell'ultimo campione. È stata rimossa: il bordo della curva ora interpola tra due campioni
realmente disponibili, senza sovraelongazione.

---

## 4. Palette dei canali

Un colore per grandezza, coerente in tutto il pannello:

| Canale | Colore |
|---|---|
| TEC, piastra | ciano |
| CPU | arancio |
| Punto di rugiada | blu |
| Corrente TEC | viola |
| Potenza TEC | rosso |
| Livello di potenza | giallo |
| Umidità | verde |
| Temperatura scheda | arancio |

Le zone termiche del grafico critico usano **poligoni riempiti**, e il punto terminale segue la
curva effettivamente visualizzata.

---

## 5. Superfici e gerarchia

- Card scure **traslucide** sopra un'immagine full-bleed: il contrasto dei controlli resta leggibile
  mantenendo la foto sottostante.
- Separazione discreta fra le card, 8 px.
- Superficie laterale scura e traslucida introdotta nella build R6.
- Etichette degli assi con contrasto aumentato, per la leggibilità sui grafici bassi.
- Legenda colorata compatta sopra il grafico termico, con il margine dalla rugiada in didascalia.
- La testata del primo grafico mantiene **altezza fissa** anche quando l'indicatore di condensa è
  nascosto, così il grafico non si sposta.

---

## 6. Indicatori di stato

### Badge in testata

Il badge `WARNING CRYOGENIC HAZARD` è sempre visibile in alto. Non è decorazione: dichiara il
rischio della classe di dispositivo, e su un pannello che può portare acqua sulla piastra deve
essere letto prima dei numeri.

### Pannello STRUMENTAZIONE

Sei valori, ognuno con la sua etichetta e la sua unità, e un badge `LIVE`:

| Campo | Valore nello screenshot | Sotto |
|---|---|---|
| PIASTRA | 18,6 °C | margine +3,7 |
| POTENZA | 72 W | budget 200 W |
| CTRL | 29,4 °C | guardia 66 |
| MARGINE | +3,7 °C | sicuro |
| OCP | OFF | nessun allarme |
| FW | 13.A0 | hw rev 4 |

Il pannello mette in evidenza **quattro grandezze per volta**, con la soglia sotto il valore. Il
potenziale di inganno è alto in un pannello di refrigerazione: ogni numero ha bisogno del suo
riferimento per non essere letto al contrario.

### Margine di condensa

Il margine è `piastra - punto di rugiada`. La coppia letta nello screenshot è `18,6 °C` e
`14,95 °C`, quindi margine `+3,7 °C`, marcato sicuro.

L'indicatore a gocce compare solo se l'ultima coppia di letture TEC e rugiada è valida,
contemporanea, recente entro 5 secondi, e `TEC < rugiada`. Alla soglia esatta o sopra la soglia le
gocce non sono visibili. Una lettura mancante, non finita o scaduta non genera l'indicatore.

Il tooltip chiarisce che si tratta di **rischio condensa**: non dimostra la presenza fisica di
acqua.

### OCP

Mostrato come segnale da controllare, mai come guasto. Da solo non conferma un guasto hardware, e
su questo hardware si accende a potenze sane. I segnali rientrati non vengono mostrati come ancora
attivi.

### COP

Etichettato **stima** nell'interfaccia, mai come misura. Formula:
`max(0, 12 × (CPU - piastra) / watt)` limitato a 5. La conduttanza `12 W/°C` è ipotizzata.

---

## 7. Statistiche di sessione

Riquadro con tre righe, min / medio / massimo, su tutta la sessione:

| Canale | min | medio | max |
|---|---|---|---|
| TEC | 13,8 °C | 18,8 °C | 39,1 °C |
| Potenza | 0 W | 96 W | 270 W |
| Margine | 0,8 °C | | |

Il **minimo del margine** è la cifra che conta: 0,8 °C significa che a un certo punto della
sessione la piastra è arrivata a 0,8 °C dalla rugiada. Il verdetto live usa invece il **margine
attuale**, perché un minimo storico che continua a colorare di verde una rilettura del momento
è una rassicurazione falsa.

Il **contatore di campioni** (78,217 nello screenshot) e la versione dell'applicazione (`r143`)
stanno nella riga di stato in alto a sinistra, subito sotto la testata.

---

## 8. Area sensori

Toggle fra **HWiNFO64** e **AIDA64**, entrambi selezionabili a runtime. Nello screenshot è attivo
AIDA64 con **27 sensori**.

I sensori CPU die/core hanno **priorità** sulla lettura AIDA64 generica della CPU, e a pari
priorità viene usato il più caldo. Con la CPU che scaldava il TEC, la media dei core dice meno
del core hotter.

Griglia a 4 colonne: CPU, CPU Package, CPU IA Cores, CPU GT Cores, poi Motherboard, PCH, VRM, poi
i moduli di memoria con nome e temperatura.

La striscia di schede (Temp. / Ventole / Tensioni / Carichi / Tutti) cambia il gruppo mostrato
senza chiudere il pannello.

---

## 9. Vincoli dichiarati

- La **fluidità percepita** resta da confermare con l'utente: le misure di carico GPU disponibili
  sono 12,406 % di media su dieci campioni sulla build R11, e la verifica visiva della build
  corrente non è stata eseguita.
- La disposizione a due colonne è pensata per il verticale. In orizzontale la sidebar resta
  utilizzabile, ma non è il caso ottimale.
- La verifica visiva sull'anteprima isolata (`--preview-grafici`) non ha esposto una finestra
  catturabile dall'automazione desktop: i frame reali restano da confermare.
- Il pannello sensori mostra la temperatura della **scheda**, che non è il lato caldo della cella.
  Da qui il COP stimato e non misurato.

---

## 10. Documenti collegati

| Argomento | File |
|---|---|
| Screenshot | [`../images/dashboard-verticale.jpg`](../images/dashboard-verticale.jpg) |
| Grafici e interpolazione | [`../releases/R4-grafica.md`](../releases/R4-grafica.md) |
| Indicatore di condensa | [`../releases/R5-gocce-condensa.md`](../releases/R5-gocce-condensa.md) |
| Ritocco delle superfici | [`../releases/R6-design.md`](../releases/R6-design.md) |
| Diagnostica live | [`../releases/R8-diagnostica-live.md`](../releases/R8-diagnostica-live.md) |
| Fluidità e carico GPU | [`../releases/R11-fluidita-30fps.md`](../releases/R11-fluidita-30fps.md) |
| Codici errore e soglie | [`error-codes.md`](error-codes.md) |