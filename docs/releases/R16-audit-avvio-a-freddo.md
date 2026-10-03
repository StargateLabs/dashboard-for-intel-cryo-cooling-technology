# Audit R16 — 3 ottobre 2026

## Problema di prima accensione
Dopo il distacco dell'alimentatore, la R15 accettava le scritture ma non inizializzava il controller. Nel backup funzionante, Tec::new leggeva HEART_BEAT e chiamava reset() se BOARD_INIT mancava. Questa inizializzazione era stata rimossa per evitare reset durante connessioni e scansioni: la rimozione completa ha lasciato scoperto l'avvio a freddo.

Il log Desktop tec-controller.log documenta tre tentativi falliti (02:47:43, 02:47:49, 02:47:56 nel timestamp del file): offset2 verificato, PID=false e POWER_OK=false. Dopo l'avvio mediante la vecchia versione, il tentativo02:48:46 ha PID=true e POWER_OK=true. La correlazione sostiene il problema di inizializzazione; il collaudo fisico della R16 deve ancora confermare il risultato dopo un vero distacco alimentazione.

## Correzione
Nel percorso esplicito Abilita TEC, dopo validazione completa dei parametri e PCB sotto37 °C:
1. Legge HEART_BEAT. Se BOARD_INIT è già impostato non manda reset.
2. Se BOARD_INIT manca e PID_RUNNING è impostato, rifiuta il reset e restituisce un errore.
3. Invia DISABLE, poi RESET_BOARD0x1E una sola volta. Nessun retry automatico di questo opcode dopo perdita dell'ACK.
4. Attende BOARD_INIT con massimo8 heartbeat distanziati100ms. I timeout seriali restano limitati; non esiste un ciclo infinito.
5. Ricontrolla PCB e ripristina offset, P/I/D, enable e potenza richiesta. Se la preparazione fallisce, tenta DISABLE e mostra l'esito.
La scansione COM e Tec::new restano in sola lettura: non inizializzano e non resettano.

Il comando RESET_BOARD può modificare impostazioni del controller. Viene usato perché è presente nel vecchio percorso funzionante; PID, offset e potenza richiesti vengono riscritti prima di completare la transazione. Non si dichiara preservata ogni eventuale calibrazione interna non documentata.

## Altro difetto corretto
La guardia38 °C considerava principalmente l'intento e PID_RUNNING. Ora tiene conto anche dei watt misurati: se è stato richiesto Spento ma il carico continua ad assorbire e non c'è una commutazione pendente, torna a richiedere DISABLE. Riduzione Cryo37 °C, stop38 °C, riabilitazione sotto37 °C invariati. Nessun limite a34 °C.

## Audit dei percorsi
- Protocollo: CRC/opcode controllati, recupero seriale limitato, niente opcode sconosciuti nelle sonde. Reset0x1E escluso dalla allowlist di lettura.
- Comandi: parametri offset/PID/potenza validati prima di abilitarli; rollback DISABLE sui fallimenti; commutazioni distinte da conferme di offset.
- Coda: accorpamento aggiornamenti e priorità DISABLE conservati; test1000 aggiornamenti passante.
- Telemetria: valori non finiti/fuori intervallo rifiutati; freschezza basata sull'ultimo campione valido; riconnessione dopo errori ripetuti.
- Profili: Gaming120 W/+3.5 °C, Silenzioso60 W/+6 °C, AI160 W/+3 °C sono obiettivi software. Budget watt non è un tetto elettrico istantaneo.
- Unregulated: offset Gen1 compatibile; non ottimizzato automaticamente come Cryo, ma stop PCB38 °C presente. Non certificato il bit modalità nativo su hardware misto.
- Diagnostica: OCP isolato può essere rumoroso; COP resta una stima, non misura termodinamica del rendimento.
- UI: grafici con buffer1s, cache icone e card TEC mantenuti. Nessun nuovo effetto grafico aggiunto durante la correzione di avvio.
- Configurazione: backup del JSON non parsabile presente; gli errori di salvataggio e alcuni avvisi di codice restano migliorabili. Non si dichiara una revisione senza debito tecnico.
- Repository: sorgenti modificati allineati nella cartella stargate-cryo-dashboard; README e SECURITY corretti per soglie37/38 e inizializzazione. Nessun commit/push eseguito.

## Crash GPU osservato
recovery.log contiene un panic wgpu Queue::submit / Parent device is lost il3 ottobre. Il supervisore ha rilevato exit101, riavviato dopo2s e recuperato l'intento Cryo. Il crash del driver/rendering non viene eliminato da questa release; resta una dipendenza dell'interfaccia dal rendererGPU. Un blocco completo della UI, un guasto al supervisore o il sistema operativo sospeso non sono coperti da una garanzia di continuità termica.

## Verifiche
325 test:297 applicazione e28 libreria;0 fallimenti. Nuovi test: reset solo su scheda non inizializzata, nessun reset a caldo, rifiuto stato PID_RUNNING incoerente, timeout limitato, interruzione della sequenza a ogni errore.
5 integrazioni escluse perché toccano controller/database reale. Clippy workspace/all-targets completato; presenti avvisi (stile, codice non usato e unwrap nei test), non certificazione di assenza di difetti.
Il test di prima accensione reale e la risposta fisica al DISABLE38 °C devono essere distinti dai test simulati. Non togliere alimentazione mentre il PC è sotto carico per effettuare la prova.

## Artefatto verificato
Release compilata in1m48s: StargateCryo-GEN1-TEC2-R16.exe.
SHA256:909013D20DB91768645A2B5CBA4285C96F9CE9F974F7AF8F55C51A0B42BAA275.
Self-test supervisore exit0: arresto simulato70, riavvio2s, recupero intentoCryo, uscita intenzionale. Nessun accesso COM.
Suite ripetuta nella cartella stargate-cryo-dashboard:325 passati,0 falliti. git diff --check senza errori.
