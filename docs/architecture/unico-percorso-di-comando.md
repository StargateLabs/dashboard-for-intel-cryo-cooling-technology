# Un solo percorso di comando: specifica di progetto

Data: 2026-09-29
Stato: da rivedere con l'utente prima dell'implementazione
Ambito: dashboard Intel Cryo Gen 1, firmware `FW 13:A0`, hardware `HW 4`, controller `V1`

---

## 1. Perché questo lavoro esiste

Il menu modalità è stato riscritto perché confondeva due cose diverse: il regime
*Standby* e lo *spegnimento* del modulo. La correzione ha introdotto un quarto
stato, `Spento`, e ha distinto i due. Ma a quel punto il software conteneva **due
percorsi di controllo che scrivono sullo stesso controller**, e la semantica vecchia
era sopravvissuta in più punti.

Una verifica systematica del 2026-09-29 ha trovato quattro difetti. La suite di
210 test era **verde durante tutti e quattro**: nessuno dei quattro è coperto dai
test esistenti. Questo documento esiste perché quel numero non viene più usato
come prova di correttezza.

### I quattro difetti

| # | Difetto | File | Conseguenza |
|---|---|---|---|
| **D1** | `Tec::new()` chiama `reset()` (opcode `0x1E`, reset di fabbrica) quando `BOARD_INIT` non è impostato | `cryo_cooler_controller_lib/src/lib.rs:332` | Ad ogni connessione perde PID, setpoint e **power cap**. Un limite di potenza impostato dall'operatore sparisce senza avviso. |
| **D2** | Il pulsante `ABILITA TEC` / `DISABILITA TEC` scrive in proprio `set_setpoint_offset(self.inputs.set_point)` | `cryo_cooler_controller/src/running.rs:3462` | Può sovrascrivere l'offset del regime attivo (es. il `-30` di Unregulated) mentre il menu continua a mostrare il regime vecchio. |
| **D3** | `tec_abilitato = !LOW_POWER_MODE_ACTIVE` | `cryo_cooler_controller/src/running.rs:1493` | `LOW_POWER_MODE` è il flag *standby*, non il flag *alimentazione*. In Standby, dove il TEC deve essere acceso, la guardia perde l'autorizzazione a scrivere potenza. |
| **D4** | `last_cond_margin` mantiene l'ultimo valore valido per sempre: il ramo `Err` del campionamento non lo azzera | `cryo_cooler_controller/src/running.rs:2051` | Un avviso di sicurezza può basarsi su un dato vecchio di minuti, o anteriore a un disconnessione. |
In più, `guardia_anticondensa` blocca anche `Cryo` sotto la soglia di condensa
impedisce il passaggio che *riduce* il rischio, e consente quelli che lo aumentano.

**D4 è già visibile, non solo ipotetico.** La riga del margine in
`view_left_column` (`running.rs:3497`) stampa `Margine +3.0°C OK` in verde senza
alcun controllo di freschezza. Con il controller scollegato da un minuto, quella
riga continua a dire che va tutto bene: è una rassicurazione esplicita e falsa
mentre la guardia antistante silenziosamente continua a usare lo stesso numero
congelato. La correzione di D4 deve quindi toccare **due** consumatori, non uno.

---

## 2. Invarianti

Sono le proprietà che devono valere **sempre**. Ogni fase le copre con test che
sono stati osservati fallire prima dell'implementazione.

- **I1: Un solo percorso di scrittura.** Nessun elemento della UI scrive
 direttamente un offset, una potenza o un PID. L'unica sorgente di comandi è il
 regime selezionato.
- **I2: Connettersi non è un'azione di stato.** Aprire la porta seriale non
 modifica alcuna impostazione del controller. In particolare non emette mai
 `0x1E`.
- **I3: "Il TEC è acceso" ha un'unica definizione**, derivata dal regime, e non
 da un bit letto isolato.
- **I4: La guardia scrive potenza solo se il regime lo autorizza.** Standby
 TEC acceso, guardia autorizzata. Spento: nessuna scrittura.
- **I5: Nessun avviso di sicurezza poggia su un dato vecchio.** Se il dato non è
 fresco, l'avviso non compare e il pannello lo dichiara.
- **I6: La guardia anticondensa non blocca mai.** Può chiedere una conferma
 esplicita; non può impedire un'azione.
- **I7: Un solo dialogo.** La conferma di Unregulated e quella di condensa non
 si sommano: producono un'unica finestra con tutti i motivi.
- **I8: Il testo del dialogo non attribuisce causalità.** Spiega lo stato
 misurato e la direzione del movimento, senza affermare che il comando causi o
 prevenga la condensa.
- **I9: Un regime dedotto non è mai mostrato come un fatto.** Lo stato di
 conoscenza è parte dell'informazione mostrata, non una nota a margine.
- **I10: Il software non scrive in seriale quando lo stato è `Ignoto`.** L'unica
 eccezione è l'azione che riduce il rischio, e resta comunque esplicita.
- **I11: Il lavoro precedente non si rompe in silenzio.** Ogni test che
 l'implementazione modifica è nominato in anticipo in § 5, con la ragione. Un
 test che cambia senza essere stato dichiarato è una regressione.
- **I12: I commenti che registrano bug passati non si cancellano.** Questo
 codice ha Comments lunghi che *spiegano perché* una cosa è fatta in un certo
 modo, e sono la memoria dei difetti già corretti. Un refactor che li "ripulisce"
 distrugge la ragione per cui quei bug non tornano.

---

## 3. Cosa si decide di **non** fare

- **Non si ricostruisce l'inizializzazione della board.** Se un controller nuovo
 richiede una procedura di commissioning, questa diventa un'azione esplicita e
 separata, non un effetto della connessione. Fuori ambito: nessuna UI di
 commissioning in questa fase.
- **Non si tocca il protocollo.** `0x14`, `0x18`, ordine di scrittura, polarità e
 `payload_alimentazione()` restano come sono. Sono verificati e corretti.
- **Non si sposta `running.rs`.** La suddivisione del file è un progetto
 separato: mescolarla a correzioni di sicurezza rende impossibile attribuire una
 regressione.
- **Non si aggiunge un pulsante "auto"** finché `PID_RUNNING` non è misurato su
 hardware.
- **Non si allarga la allowlist di sola lettura.**

---

## 4. Fasi

L'ordine è **per conseguenza, non per comodità**: dalla perdita di dati a un
colpo (D1) fino a una correzione che non può far male (la guardia). I numeri di
difetto non seguono l'ordine delle fasi, e ogni intestazione di fase dice quale
difetto tratta, così la tracciabilità non si perde.

| Fase | Difetto | Conseguenza se lo facciamo per ultimo |
|---|---|---|
| 1 | D1, reset di fabbrica | power cap perso, nessuna avviso |
| 2 | D3, `tec_abilitato` | la guardia perde i permessi in Standby |
| 3 | D2, doppio percorso | sovrascrittura dell'offset di regime |
| 4 | D4, staleness del margine | rassicurazione verde falsa |
| 5 | guardia anticondensa | l'unico senza conseguenze fisiche |

### Fase 0: Verifica pre, due passate

Ogni difetto viene esaminato due volte con metodi diversi, perché la prima
verifica in assoluto ha mancato tutti e quattro i difetti

- **Passata A: "cosa scrive il controller"**: dalla costruzione di `Tec` fino
 all'ultimo byte, seguendo ogni chiamata di scrittura.
- **Passata B: "cosa crede l'operatore"**: dall'etichetta del pulsante fino al
 numero mostrato, seguendo la UI.

Se le due passate non concordano, il difetto non è ancora capito: si ferma tutto.

**Misure hardware richieste all'operatore** (non automatizzabili)

| Stato | `BOARD_INIT` | `PID_RUNNING` | `LOW_POWER_MODE` | `TEMP_MODE` | tensione TEC |
|---|---|---|---|---|---|
| Spento | | | | | |
| Standby | | | | | |
| Cryo | | | | | |

Se una riga non è ottenibile, la fase applica il **default sicuro**: per D1
questo significa *non emettere mai `0x1E`*, perché azzerare il power cap non è
mai la risposta giusta a un ricollegamento.

**Gate:** nessun codice di produzione scritto prima del completamento.

### Fase 1: D1: il reset esce dalla connessione

`Tec::new()` smette di chiamare `reset()`. La costruzione si limita ad aprire la
porta, configurarla e fare un `hear_beat` di prova.

*Test prima (rosso):* per qualunque combinazione di bit di `BOARD_INIT`
aprire una connessione non produce `0x1E` sul bus. Il test è sulla funzione pura
che decide, non sul mock della porta.

### Fase 2: D3: tre stati di conoscenza, e una sola verità sul TEC acceso

`tec_abilitato` **smette di essere un campo che viene aggiornato e diventa una
funzione derivata**. Oggi è assegnato in quattro punti distinti, l'ack di
`Enable`, l'ack di `Disable`, l'ack di `Regime`, e la chiusura, e ognuno può
lasciare il campo in uno stato diverso dagli altri. Se restasse un campo mutable
con la stessa semantica, il difetto sopravviverebbe alla correzione: basta che un
solo percorso dimentichi di aggiornarlo.

#### Il nucleo: la conoscenza non è un dettaglio, è uno stato

Oggi il software ha **una** verità sul regime e la tratta come un fatto. Ma è
un'inferenza: `PID_RUNNING` non è mai stato osservato cambiare su questo
controller. Un sistema che deduce e non lo dichiara è un sistema che mente con
fiducia, e un errore di regime non si vede sui numeri, perché i numeri
descrivono il controller, non l'ipotesi sul suo regime.

La correzione è portare l'incertezza **dentro il tipo**, non dentro un commento

```
Certezza = Conosciuto | Dedotto | Ignoto
```

- **Conosciuto**, i bit di stato concordano col regime atteso. Il software
 automatizza, scrive, ottimizza.
- **Dedotto**, i bit dicono qualcosa, ma non è stato verificato. Il regime
 viene **mostrato marcato come dedotto** e il software non automatizza.
- **Ignoto**: nessun dato fresco. Il software **non scrive niente** in seriale e
 dichiara che non sa.

La definizione unica, in un punto solo

```
"il TEC è acceso" = Certezza del regime corrente, e il regime eroga potenza
```

Concretamente: il TEC è acceso quando il regime corrente è `Standby`, `Cryo` o
`Unregulated`; non è acceso quando è `Spento`. `LOW_POWER_MODE` smette di
decidere qualsiasi cosa su questo.

Il terzo significato, l'argomento aggiuntivo di `modalita_corrente`, che oggi
riceve `poll_in_flight || applied_power > 0 || tec_abilitato`, viene eliminato
la funzione ne prende uno solo. È la terza definizione concorrente e deve sparire
con le altre due.

**Effetto collaterale verificato sulla chiusura:** `running.rs:1841` usa
`tec_abilitato || applied_power > 0` per decidere se mandare `disable` all'uscita.
Con la definizione nuova il comportamento è corretto in tutti e tre i casi, in
Standby e Cryo il `disable` parte, in Spento non parte perché è già spento.

*Test prima (rosso):* in Standby il TEC è acceso e la guardia è autorizzata a
scrivere; in Spento non lo è; `LOW_POWER_MODE` da solo non decide nulla; il
risultato non dipende da quale percorso è passato prima; in stato `Ignoto` non
viene emessa alcuna scrittura; in stato `Dedotto` l'etichetta del regime porta il
marchio di dedotto e non è identica a quella di `Conosciuto`.

### Fase 3: D2: un solo percorso di comando, e il menu al posto del pulsante

Il pulsante `ABILITA TEC` / `DISABILITA TEC` viene **rimosso dalla UI**, e al suo
posto entra il **menu dei regimi**. Non è una riorganizzazione cosmetica: il
controllo del regime diventa l'elemento primario della colonna, dove si arriva
con un clic e non cercandolo in fondo alla barra laterale.

`Message::Enable` e `Message::Disable` vengono eliminati insieme a
`Richiesta::Enable` e `Richiesta::Disable`, in modo che il percorso vecchio non
resti raggiungibile da nessuna parte.

Resta la disabilitazione **all'uscura** (`running.rs:1850`), che è un'altra cosa
e resta: chiudere l'app deve spegnere il modulo.

#### Il posto del pulsante: geometria verificata

`pulsante_tec()` (`running.rs:3447`) restituisce già una `Column` con due figli

```
Column
  ├── tasto      ← il pulsante ABILITA/DISABILITA, da sostituire
  └── grafico    ← striscia di potenza, da NON toccare
```

**La striscia di potenza resta intatta**, come richiesto: stesso `Canvas`
stesso `MiniSpark`, stessa altezza di 34 px, stesso colore guidato dalla modalità
corrente. Sotto il menu regime ci sarà ancora la striscia, e dirà ancora se il
TEC sta spingendo, calibrando o fermo.

**Non c'è rischio di larghezza**, e il motivo è verificato: il pulsante
(`running.rs:3495`) e il menu regime (`running.rs:3789`) sono costruiti nella
**stessa funzione**, `view_left_column()`, e finiscono nella **stessa colonna da
340 px**. Sono già fratelli. Lo spostamento non cambia la larghezza disponibile
e `pulsante_regime` usa già `Length::Fill`: due pulsanti per riga rendono
esattamente come rendono oggi.

#### Cosa porta con sé il menu

Dal menu attuale all'innalzamento, oltre ai quattro pulsanti

- **la riga di esito** (`esito_commutazione`), che è l'unica parte del menu di cui
 ci si può fidare: dice se la commutazione è avvenuta, o cosa ha fatto il
 controller al posto nostro. Un pannello di controllo senza risposta a un
 comando è un pannello a metà;
- **la nota `MODALITA_NOTA`**, che spiega perché da quel menu non si commuta in
 ogni momento;
- **il pulsante "Torna a Cryo"** che compare in Unregulated: è una via d'uscita
 e una via d'uscita non si tocca quando si sposta un pannello.

L'intestazione "MODALITA'" con il bottone di spiegazione **non** viene trasferita
nella sua posizione nuova il menu non ha bisogno diPresentarsi, e l'informazione
è già nel pulsante di spiegazione accanto alla nota.

*Test prima (rosso):* nessun `Message` della UI può produrre una scrittura di
offset diversa da quella del regime attivo; l'insieme dei messaggi raggiungibili
non contiene più `Enable`; la striscia di potenza è ancora renderizzata e riceve
ancora il colore della modalità corrente.

### Fase 4: D4: il margine ha un'età

`last_cond_margin` viene letto insieme a `last_sample_time`, che esiste già e
viene aggiornato **solo** sui campioni validi. La guardia usa l'età: oltre
`CAMPIONE_TIMEOUT` il margine è considerato non fresco.

I consumatori sono **due**, ed è il punto che rende D4 più di un dettaglio

1. la guardia anticondensa, che decide se chiedere una conferma;
2. la riga di margine in `view_left_column` (`running.rs:3497`), che oggi
 dichiara "OK" in verde su un valore congelato.

Entrambi devono distinguere *fresco e basso* da *vecchio*. Le tre etichette
diventano: margine fresco e sopra soglia → `OK`; fresco e sotto soglia →
`basso` o `CONDENSA`; non fresco → **nessuna dichiarazione di sicurezza**, e la
riga dice che il dato non è fresco. Un avviso che non parte è accettabile; una
riga verde che mente no.

*Test prima (rosso):* un margine vecchio non genera avviso, non viene
presentato come misura valida e non produce l'etichetta verde; uno fresco sotto
soglia produce l'avviso e l'etichetta di pericolo.

### Fase 5: La guardia anticondensa con conferma esplicita

`guardia_anticondensa` cambia natura: da "rifiuto" a "serve una conferma".

```
premi un regime
  ├─ margine assente o vecchio  ──► parte subito
  ├─ margine ≥ 1.0 °C          ──► parte subito
  └─ margine < 1.0 °C, fresco   ──► dialogo unico
                                    ├─ Esc / No ──► annulla, non scrive
                                    └─ Conferma ──► rivalida, poi parte
```

- **Modale sul serio**: mentre una conferma è pendente, i messaggi di regime
 vengono rifiutati. Oggi non lo sono, e un pulsante è cliccabile *attraverso* il
 dialogo.
- **Un solo dialogo**: Unregulated sotto soglia non produce due conferme in
 fila. I motivi si accumulano in un'unica finestra.
- **Rivalidazione**: alla conferma il margine viene riletto; se è nel frattempo
 salito sopra soglia, si procede senza chiedere.
- **Testo neutro** (I8): il dialogo dichiara il margine misurato e la direzione
 del movimento ("abbassa il raffreddamento" / "lo alza"), senza attribuire la
 causa della condensa al comando.

#### L'anteprima: premere sapendo la conseguenza

Il dialogo non chiede solo "confermi?". Mostra **cosa succederà**, calcolato dai
sENSORI e non scritto a mano, perché qui "innovativo" significa che il pulsante è
una previsione e non un interruttore

- **temperatura obiettivo** della piastra per quel regime (Cryo: il tuo
 setpoint; Unregulated: `-30 °C`; Standby: `3.5`; Spento: nessuna, il modulo non
 eroga);
- **potenza attesa**, per il regime e il cap correnti;
- **rischio di condensa** conseguente, in forma di margine previsto, con la
 distinzione "fresco" / "non fresco" già definita in Fase 4;
- **stato di conoscenza** del regime corrente (I9): se è `Dedotto` o `Ignoto`
 l'anteprima lo dichiara, perché cambia il peso della conferma.

La regola è che **l'anteprima dice sempre anche l'incertezza**. Un numero presentato
senza il suo grado di affidabilità è un numero che mente, ed è il difetto che ha
già prodotto la riga verde falsa di D4.

*Test prima (rosso):* l'anteprima contiene l'obiettivo di temperatura corretto per
ciascun regime; dichiara lo stato di conoscenza; quando il dato non è fresco
dichiara che non è fresco invece di mostrare il numero.

*Test prima (rosso):* nessun regime è mai bloccato · sotto soglia ogni regime
chiede conferma · sopra soglia nessuno · margine vecchio non avvisa · messaggi di
regime rifiutati a dialogo aperto · Esc non scrive nulla · Unregulated fa un
dialogo solo · la conferma rilegge il margine più recente.

### Fase 6: Verifica finale e release

Le stesse due passate della Fase 0, riapplicate sull'intero diff. Una seconda
passata serve a trovare i difetti *introdotti* dalle correzioni, che la prima
passata non può vedere perché cerca solo quelli noti.

Poi: suite completa, build release, `r117` in `StargateCryo`, **r84 intatta**.

---

## 5. Strategia di verifica

### 5.1 I test che l'implementazione modifica: inventario dichiarato

**Su 210 test, ne vengono toccati 4.** Gli altri 206 non si toccano e devono
restare verdi. L'inventario è qui *prima* di scrivere il codice, perché un test
che cambia senza essere stato dichiarato è una regressione nascosta (I11).

| Test | File | Cosa gli facciamo | Perché |
|---|---|---|---|
| `i_regimi_attivi_sono_bloccati_sotto_la_soglia` | `commutazione.rs:383` | **riscritto** | Verifica che la guardia **blocchi** Standby/Cryo/Unregulated. L'utente ha deciso che la guardia non blocca mai: chiede conferma. Il test codifica una decisione superata, e mantenerlo significa disfare la scelta. Le asserzioni diventano "nessun regime è bloccato". |
| `la_soglia_e_un_grado` | `commutazione.rs:413` | **riscritto** | Stessa ragione: asserisce `is_some()` sotto soglia, cioè un blocco. Diventa " sotto soglia chiede conferma, sopra soglia no", che è la soglia vera. |
| `solo_enable_autorizza_il_tec_acceso` | `attore_tec.rs:248` | **eliminato** | È tautologico: verifica `Scritto::Enable.accende_il_tec()`, e `accende_il_tec()` esiste **solo per i test** (`#[cfg_attr(not(test), allow(dead_code))]`) e non fa n'altro che confrontare il tag della variante con un booleano. Non può fallire per un motivo comportamentale, quindi non copre nulla. Inoltre D2 elimina la variante `Scritto::Enable`. |
| `la_scrittura_precede_il_campione` | `attore_tec.rs:261` | **portato, non eliminato** | L'invariante che copre è **reale**: un blackout hardware in cui la scrittura resta in coda dietro il campione. Ma usa `Richiesta::Enable`, che D2 elimina. Va riscritto su `Richiesta::Regime`, verificato che `e_scrittura()` è `!matches!(self, Campione)`, quindi la nuova richiesta ha già la priorità. |

La scoperta di `solo_enable_autorizza_il_tec_acceso` è il motivo per cui questo
inventario esiste: è un test che **inflava il conteggio** e ha dato una falsa
sensazione di copertura. Verificare l'onestà dei test fa parte della verifica
non è un extra.

### 5.1.1 Correzioni all'inventario, 2026-09-29

L'inventario era **incompleto**, e il difetto è di metodo, non di conteggio: elencavo i test che sapevo, senza elencare tutti quelli che toccano le **firme e le varianti che sto cambiando**.

**Correzione 1: firma di `modalita_corrente`.** Invertendo la polarità del secondo argomento ho messo in rosso **nove test** di `modalita.rs`, nessuno dei quali era nell'inventario. La suite li ha presi tutti: è esattamente il suo lavoro. La correzione è stata economica perché **la polarità non era il problema**: il difetto era il *caller*, che costruiva il flag da un `||` di tre grandezze diverse. Polarità ripristinata, significato chiarito, caller corretto. Zero test toccati.

**Correzione 2: varianti di `Richiesta`/`Scritto`.** Rimuovendo `Enable` e `Disable` ho toccato altri test **non presenti nell'inventario originale**

| Test | File | Cosa facciamo |
|---|---|---|
| `campione_e_ack_distinguibili` | `attore_tec.rs` | **aggiornato**: usa `Scritto::Pid` invece di `Scritto::Disable`. L'invariante che copre è reale, `Campione` e `Ack` devono restare distinguibili. |
| `tutte_le_scritture_prima_del_campione` | `attore_tec.rs` | **aggiornato**: accoda `Richiesta::spegnimento()`. Invariante reale: tre scritture escono prima del campione. |
| `il_pulsante_usa_la_stessa_tinta_del_led` | `modalita.rs` | **eliminato** | Verificava che il colore del pulsante fosse il colore del LED *scurito*, cosa necessaria quando il pulsante aveva **testo bianco** sopra. I pulsanti di regime hanno testo colorato su fondo scuro, quindi la condizione non esiste più. |
| `il_verde_del_pulsante_e_abbastanza_scuro_chiara` | `modalita.rs` | **eliminato** | Idem: verificava la luminosità del verde per fare contrasto col bianco. Non c'è più testo bianco. |
| `tasto_striscia_e_menu_dicono_la_stessa_cosa` | `modalita.rs` | **riscritto** | L'invariante sopravvive ed è **più forte**: non più "stessa tinta, luminosità diversa" ma **lo stesso valore identico**, perché il pulsante ora prende il colore del LED direttamente. |
| `serve_uscita` | `modalita.rs` | **ripristinato** | Era stato rimosso insieme al pulsante Abilita/Disabilita, ma il bottone **"Torna a Cryo" è rimasto** (è salito col menu nella nuova posizione). Rimuovere il predicato mentre il widget esiste lascia la regola di sicurezza senza casa. |

**Totale reale: 10 test, non 4.** L'inventario iniziale era sottovalutato di sei.

**La regola che ne segue:** prima di cambiare la firma di una funzione o di rimuovere una variante, si contano i test che la usano, e si mettono in inventario *quelli*, non quelli che si ricordano. Vale anche al contrario: **un elemento che sparisce dalla UI non fa sparire la regola che lo governava**, se un altro elemento la usa ancora.

### 5.2 TDD rigoroso

**TDD rigoroso.** Per ogni difetto: scrivere il test, vederlo fallire per la
causa giusta, scrivere il minimo che lo fa passare, rifare. Un test che passa al
primo colpo non è un test: è una descrizione di quello che già fa il codice.

**"Nessuna regressione" è verificabile, non retorico:**

1. La suite esistente (210 test) resta verde **in ogni fase**, non solo alla fine.
2. Ogni correzione ha un test che è stato osservato fallire.
3. Nessun numero esistente cambia significato senza che un test lo dimostri.

**"Due volte"** è Vincolante, non retorico. La prima verifica ha mancato i
quattro difetti; la seconda li ha trovati tutti. Una singola passata non ha
valore probatorio su questo codice.

**Limite dichiarato:** le Fasi 1, 2 e 3 non sono verificabili end-to-end senza il
controller. Si può provare che il codice fa quello che dice; non che il
controller risponda come atteso. Questo pezzo dipende dalle misure della Fase 0 e
resta dichiarato aperto finché non sono disponibili.

---

## 6. Dove siamo migliori, e come lo si dimostra

L'obiettivo è essere migliori del software Intel. Un'affermazione così non è
verificabile, quindi si restringe a dimensioni concrete, e ogni riga ha una prova.

| Dimensione | Intel | Dopo questo lavoro | Prova |
|---|---|---|---|
| Percorsi di scrittura sul controller | 3 `Init*Mode` più enable/disable | **1** | ispezione del codice |
| Il regime viene riletto dopo la scrittura | no | **sì** | test |
| Regime dedotto mostrato come fatto | sì | **no** | test sul rendering |
| Numero a schermo che può essere falso | non tracciato | **zero** | test per consumatore |
| Stato di spento esplicito | no | **sì** | test |
| Regole valide solo in certi stati | sì (`IsValideOperationMode`) | sì, **e lo dichiara** | test |
| L'incertezza è uno stato o un'assunzione | assunzione | **stato** | test |

**Dove siamo indietro, e va detto prima che lo scoprano altri.** Intel distribuisce
`SafetyRules` con soglie **CB2** e **DT1** tarate su questo hardware da chi aveva
accesso al firmware. Noi non sappiamo cosa siano: sono state trovate nei binari ma
non decodificate. Su quel punto specifico siamo indietro, non avanti. E sul
progetto intero siamo indietro su una cosa precisa: la **verifica hardware**.
Finché `PID_RUNNING` e `BOARD_INIT` non sono misurati, la nostra superiorità è
reale ma circoscritta a trasparenza e onestà del modello di stato.

---

## 7. Cosa può andare storto, e la risposta

- **Le misure hardware non arrivano** → si applicano i default sicuri: niente
 `0x1E`, e la semantica di `PID_RUNNING` resta quella derivata dal protocollo
 dichiarata non confermata.
- **`PID_RUNNING` non significa quello che credo** → `da_stato` è una funzione
 pura con test: si corregge in un punto e i test lo dichiarano. È il motivo per
 cui la logica di regime non vive dentro `running.rs`.
- **Una correzione ne introduce un'altra** → la seconda passata della Fase 6
 esiste per questo, ed è l'unico controllo che la intercetterebbe.
