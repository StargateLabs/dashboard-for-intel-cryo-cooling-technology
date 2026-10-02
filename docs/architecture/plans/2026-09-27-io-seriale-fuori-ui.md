# I/O seriale fuori dal thread UI: Piano di esecuzione

> **Per worker agentici:** SUB-SKILL: `superpowers:executing-plans`. Ogni fase si esegue, si compila e si testa **prima** di passare alla successiva. Una fase che rompe riporta il backup di quella fase sola.

**Obiettivo:** togliere dalla UI le 9 round-trip seriali che si verificano a ogni campionamento, senza cambiare nessuna decisione di sicurezza.

**Architettura:** la porta seriale passa dietro `Arc<Mutex<Tec>>`. Un thread dedicato la usa per `monitor()` e `hear_beat()` e spedisce il risultato su un canale; il tick della UI lo raccoglie con `try_recv`, che non blocca, e applica le regole esattamente come fanno oggi. I **setter** restano sincroni: prendono il lock, scrivono e lo rilasciano in pochi millisecondi, quindi `applied_power` continua a essere aggiornato solo dopo conferma hardware.

**Vincoli:** nessuna regressione sui 37 + 7 test; 0 warning; nessun cambiamento al comportamento delle protezioni; non terminare mai il processo dell'utente.

## Perché è fallito prima

Tre tentativi, tutti con lo stesso errore di fondo: **una trasformazione testuale che elimina blocchi in più posti**. Quando si cancella un blocco, tutti gli indici successivi si spostano; io li toccavo in ordine crescente invece che decrescente, e il risultato era un file con graffe non bilanciate. La prima volta avevo anche cancellato i corpi dei `match` **prima** di averli estratti in metodi, perdendo circa 400 righe di logica.

Da qui le due regole di questa esecuzione: **un passo per volta, con l'editor e non con uno script**, e **compilazione dopo ogni passo**.

---

## Fase 1: Idraulica: guardia e campi

Nessun comportamento cambia: solo gli strumenti per arrivarci.

- [ ] Aggiungere `PollResult` fuori da `impl`.
- [ ] Aggiungere i campi `tec: Arc<Mutex<Tec>>`, `poll_tx`, `poll_rx`, `poll_in_flight`.
- [ ] Aggiungere `fn tec(&self) -> MutexGuard<Tec>`, che recupera la guardia anche da un mutex avvelenato.
- [ ] Sostituire i `self.tec.` con `self.tec().` **eccetto** `monitor()` e `hear_beat()`.
- [ ] Compilare: 0 error, 0 warning. A questo punto il codice non gira ancora, perché i due match chiamerebbero `self.tec()`.

## Fase 2: Il thread di polling

- [ ] In `new()`: avvolgere `tec` nell'`Arc<Mutex<_>>`, creare i due canali, lanciare il thread `cryo-tec-poll`.
- [ ] Nel thread: `recv()` in loop, lock, `monitor()` e `hear_beat()`, `send(PollResult)`, uscita se la UI non ascolta più.
- [ ] Compilare: 0 error, 0 warning, 44 test verdi. Il thread gira ma nessuno lo usa: si può uccidere e riavviare senza effetti.

## Fase 3: Estrarre il ramo `Ok` del monitor

- [ ] **Con l'editor**: tagliare il corpo di `Ok(data) => { … }` e incollarlo in `fn applica_campione(&mut self, data: MonitoringData)`, ricavando `Ok(data) => {` dal nome del braccio.
- [ ] Il corpo diventa `match self.applica_risultato_poll(...)`? No: in questa fase il match chiama ancora i metodi.
- [ ] Compilare: 0 error. Se il file ha graffe non bilanciate, il ripristino è **solo** di questa fase.

## Fase 4: Estrarre `Err` del monitor e tutto il battito

- [ ] `applica_errore_monitor(&mut self, err: &str)`
- [ ] `applica_heartbeat(&mut self, status: TecStatus)`
- [ ] `applica_errore_heartbeat(&mut self, err: &str)`
- [ ] Compilare: 0 error, 44 test verdi.

## Fase 5: Scambiare i `match` con raccolta e richiesta

- [ ] Nel tick, al posto dei due `match`: `raccogli_poll()` → `applica_risultato_poll`, poi la richiesta se `should_update() && !poll_in_flight`.
- [ ] Compilare: 0 error, 44 test verdi.
- [ ] **Il punto da verificare a mano:** il watchdog deve ancora contare i fallimenti, l'emergenza pompa deve ancora reagire, la guardia termica deve ancora scrivere. Sono dentro i metodi estratti, quindi se i test passano e il codice è identico, lo sono.

## Fase 6: Verifica e build

- [ ] Build forzato (senza cache): 0 warning.
- [ ] `cargo test`: 37 + 7 verdi.
- [ ] Release in `target/r36`.
- [ ] Deploy su `StargateCryo`, e poi sul file dell'utente **solo se lo autorizza**, con scambio in ascolto che non interrompe il raffreddamento.

## Cosa NON cambia

Il refactor sposta **dove** i comandi vengono eseguiti, non **cosa** viene deciso. Le protezioni, le soglie, `applied_power` e la gestione degli errori restano identiche. L'unica differenza percepibile è che la UI non si blocca più durante le attese seriali, e che un campione arriva fino a 250 ms dopo.

## Rischio residuo dichiarato

La guardia sul setpoint non credibile e la protezione pompa restano quelle di r35. Il refactor non le tocca, ma una volta in più vale la pena ricordare che `BOARD_TEMP_OK` non è verificabile senza l'hardware: per questo la guardia **degrada** a 30% e non spegne.
