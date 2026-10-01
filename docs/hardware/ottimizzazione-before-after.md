# Ottimizzazione del consumo sulla cella TEC — analisi before/after

Data: 2026-09-28 · Hardware: controller Intel Cryo **Gen 1** (kit QuantumX Delta) + cella **EK-Quantum Delta² TEC V2** (LGA1700)

> **Correzione 2026-09-28 (build r99).** La versione precedente di questo
> documento concludeva che il controller Gen 1 avesse un tetto rigido di 200 W
> e che l'OCP fosse una protezione reale. **Entrambe le conclusioni sono
> smentite dalle misure.** Il documento era stato scritto su un'ipotesi, e
> un'ipotesi presentata come fatto porta a decisioni sbagliate: chi lo leggeva
> avrebbe tenuto la cella al 60% per una protezione inesistente.
>
> Cosa e' cambiato, in breve:
> - **Non esiste un tetto a 200 W.** Il controller eroga 220/230/237 W in modo
>   stabile, con il raffreddamento che funziona.
> - **L'OCP non e' una protezione.** Si accende a 73 W e a 112 W con sistema
>   sano. Non deve mai ridurre la potenza.
> - **A carico parziale il rendimento e' migliore**: 112 W con COP 1,70
>   contro 225 W con COP 0,95.

## BEFORE — misure reali (regime 86%)

| Grandezza | Valore |
|---|---|
| Tensione TEC | 10,38 V |
| Corrente TEC | **21,70 A** |
| Potenza TEC | 230 W (misurata 225 W) |
| Livello potenza | 86% |
| **OCP** | **ATTIVO** (rumore, vedi sotto) |
| Piastra | 8,2 °C |
| Punto di rugiada | 15,01 °C |
| CPU esterna | 26,0 °C |
| CTRL (PCB controller) | 29,8 °C |
| COP stimato | 0,93 |

Calcoli:

- `V × I = 10,38 × 21,70 = 225 W` (concorda con la misura)
- Resistenza equivalente: **0,478 Ω**
- La corrente e' il **90% della Imax** della cella (24 A)
- COP 0,95 → il radiatore riceve **225 W elettrici + 214 W di calore CPU = 439 W**

## L'ipotesi che era sbagliata: "il tetto e' 200 W"

La versione precedente ragionava cosi': *"il kit Gen 1 e' dichiarato 200 W,
quindi chiedere 230 W al 100% e' il 115% del tetto, e serve tagliare"*.

**La premessa era sbagliata.** Le misure su questo hardware mostrano che
l'etichetta "200 W" del kit non e' un limite erogabile dal firmware: il
controller eroga stabilmente 220 W, 230 W e 237 W, e in tutti e tre i casi il
raffreddamento funziona normalmente. Tagliare al 60% non proteggeva niente:
si spendevano meno watt per un freddo che il modulo gia' non concedeva.

Di conseguenza `TETTO_WATT_CONTROLLER` **non blocca nulla**: e' un **avviso**.
La decisione sulla potenza resta dell'utente.

## L'OCP non e' una protezione

L'OCP si e' acceso a **73 W** e a **112 W** con il sistema sano e il
raffreddamento funzionante. Una protezione che scatta a un terzo della potenza
nominale non e' una protezione: e' un segnale che non significa nulla, e su
cui non si deve basare una decisione automatica.

Cosa e' stato fatto:

| Build | Comportamento | Esito |
|---|---|---|
| r96 | riduzione stabile della potenza, con risalita a 1%/tick | **TEC a 73 W**: la cella non raffreddava piu' |
| r97 | intervento **una volta sola**, niente persistenza | potenza stabile, il messaggio si chiude da solo quando il bit cade |

In r97 l'intervento automatico esiste ancora, ma **una sola volta** e solo se
la potenza e' sopra il 60%: riduce a meta', non torna indietro da solo, e non
si riattiva. Un avviso che resta per sempre non e' un avviso.

La correlazione con i 14 bit di stato non decodificati e' in corso: il file
`%LOCALAPPDATA%\stargate-cryo\bit-stato.csv` raccoglie quei bit a ogni
campione, e se cambiano insieme all'OCP quello e' il suo codice errore. Finche'
non sappiamo cosa significhino, **non si inventa una decodifica**: un'etichetta
sbagliata in diagnostica fa diagnosticare il problema sbagliato.

## AFTER — cosa cambia davvero

| | Prima (r96) | Ora (r97+) |
|---|---|---|
| Potenza | 73 W (bloccata) | **112 W** a COP 1,70 |
| Comportamento OCP | riduzione stabile persistente | **una volta sola**, poi stabile |
| Tetto 200 W | treated come blocco | **avviso**, mai un blocco |

Il punto che conta e' il **rendimento a carico parziale**:

| Regime | Potenza | COP | Note |
|---|---|---|---|
| 86% | 225 W | 0,95 | radiatore sotto carico |
| ~48% | 112 W | **1,70** | **rendimento quasi doppio** |

Meta' watt elettrici per COP quasi doppio. Il radiatore riceve molto meno
calore, quindi il limite reale non e' elettrico: e' il **lato caldo**.

## Il limite reale: il lato caldo

Il COP e' limitato dal lato caldo, e li' il software non arriva.

Il valore "CTRL" che la dashboard mostra (29,8 °C) e' il **PCB del controller**,
non il lato caldo della cella: il controller e' montato lontano dal water block,
quindi segna fresco mentre il water block incassa ~400 W. Il controller Gen 1
legge una sola sonda TEC, quindi **non c'e' un secondo canale per misurare il
lato caldo**: COP e rendimento lato caldo non sono misurabili con precisione
qui, e vanno stimati.

Per questo la guardia termica non scende mai, e non deve: fermare il TEC
perche' il PCB e' fresco mentre il water block scalderebbe sarebbe il modo
giusto di distruggere la piastra.

## Cosa serve per il prossimo passo

1. **La tabella di funzionamento reale** (registratore in `src/curva.rs`,
   attivo con `CRYO_COMMISSIONING`): per ogni potenza, watt per grado di
   freddo. Da li' si sceglie il cap di massimo rendimento, che con questi
   dati e' probabilmente **parziale**, non 100%.
2. **Il lato caldo**, che vale piu' di ogni modifica di pilotaggio:
   flusso d'aria sul dissipatore, radiatore pulito, isolamento.
3. **La correlazione dei bit alternativi** per capire l'OCP.

## Costanti nel codice

```rust
const TETTO_WATT_CONTROLLER: f32 = 200.0;   // Gen 1 — solo AVVISO, non blocco
const WATT_AL_100_PERCENTO: f32 = 230.0;    // misurato su questo hardware
```

`TETTO_WATT_CONTROLLER` vale quanto l'etichetta del kit, non quanto il
controller eroga davvero. Resta un avviso utile ("stai chiedendo piu' di
quanto il kit dichiara"), ma **non deve mai limitare la potenza**:
l'hardware misurato regge 220-237 W.

Se si monta il controller Gen 2, entrambe le costanti vanno rimisurate: il Gen 2
regge piu' corrente, e i valori copiati dal Gen 1 non valgono.

## La modalità non regolata: chi la decide

La modalità **non la imposta il software, e non è un comando seriale**: è lo
stato di un pin GPIO del controller. L'hardware la legge e la scrive
(`CCHWApiExt.sys` fa `kHWAPIReadGPIO` / `kHWAPIWriteGPIO`), e il software
Intel la *legge* invece di impostarla — la riga `Cooler is in standby mode` è
un messaggio di stato, non un'azione. I dettagli sono in
`analisi-cella-peltier.md` § 9.1.

Quindi il controller la sceglie da solo, in base al **carico della CPU**:

- **CPU sotto 25 W** per più di 10 minuti → l'Unregulated viene sospeso e si
  torna a Cryo
- **CPU sopra 25 W** per più di 10 minuti → l'Unregulated continua

Con il PC sotto carico si resta in modalità non regolata; con il PC inattivo
si torna al regolato da soli. La protezione è dentro l'hardware e non
dipende da nessun programma.

Il pulsante della dashboard mostra lo **stato reale**, con il colore del LED,
e dice dove si attiva. **Non è un interruttore, e non finge di esserlo**: quel
comando non esiste su quel canale. Sarebbe stato un interruttore fittizio,
che preme e non fa niente.

## Perché la dashboard esiste

Il software Intel non parte su questo processore: il suo elenco di CPU
supportate contiene 23 modelli, tutti di 10ª generazione, e il 14900KS non è
fra quelli (`analisi-cella-peltier.md` § 9.2).

La dashboard fa il resto del lavoro — raffreddamento regolato, PID, potenza,
temperature, margine di condensa, diagnostica, stato del controller — senza
aggiungere un servizio di terzi che si mette in mezzo all'hardware. L'unica
funzione che non fa è cambiare modalità, e per il punto precedente quel
cambio non è un comando che si possa mandare.
