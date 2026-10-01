# Verifica richieste - R10

Build: StargateCryo-GEN1-TEC2-R10.exe
SHA256: E751EDB727A27D0016A97427F55F5A58C39CAA1D31E63D7FDCCB4E33056B4378
Validazione software: 310 test superati; compilazione release riuscita (5 warning).

| Richiesta | Intervento | Evidenza e limite |
|---|---|---|
| Controller Gen1 con TEC Gen2 | Offset e PID, feedback sui watt reali; percentuale firmware non trattata come limite fisico | Hardware firmware13.A0; ACK e offset riletti nei log |
| Abilita TEC | Corretto blocco InAttesa invertito | Avvio Cryo verificato; assorbimento reale nelle schermate R7/R8/R9 |
| Cryo cliccabile | Pulsante sempre disponibile, ripetizione comando possibile | Clic e offset+2 confermati sul controller |
| Unregulated dopo conferma | Conferma corretta e richiesta offset-30 con ACK/readback | Sequenza verificata; è la gestione compatibile via offset. Non certifica un cambio di modalità nativa TEMP_MODE |
| Gaming/Silenzioso/AI | Budget120/60/160W, margini3.5/6/3C, preset migrati e regolatore resettato | Test profili; selezione dei tre sotto carico non ancora provata sul dispositivo |
| CPU90 e pochi watt | Sensori die/core prioritari, recupero CPUcalda più rapido entro il budget | Test CPU90; letture osservate nella prova circa50-60C, problema90 non riprodotto né definitivamente escluso |
| Efficienza e dissipazione | Evitata riduzione troppo rapida; recupero dopo calo watt e correzione rollback inefficienza | Feedback sui watt, non misura di COP reale. Nessuna garanzia di minimo globale watt/volt/ampere |
| Diagnostica2problemi | Margine attuale al posto del minimo storico, misure elettriche simultanee | R8 osservata:1segnale OCP, senza falso guasto condensa |
| Dicituresinistra | Comandi, sensoriCPU, pompa, duty, profili e COP rivisti | R8 verificata visivamente |
| Grafica moderna senza regressioni | Modalità evidenziata, valori nella testagrafico, pannellodiagnostica scuro, effetti e sfondo mantenuti | R8 verificata; ultimo renderer R10 da osservare |
| Gocce blu sotto rugiada | Tre vettori con gradienti/riflessi, condizione campione fresco e piastra<rugiada | Test soglia/stale; verifica visiva simulata ancora da completare |
| Grafici fluidi | Interpolazione visiva500ms senza alterare dati o extrapolare | Test presentazione; animazioneR10 circa15fps in primo piano |
| GPUeccessiva | Stop animazione extra fuori focus/Home/tray, cache66ms, eliminato MSAA4x finestra | R9prima:12.7-15% in5campioni. Misura R10 ancora da completare |
| Build con tutti i fix | R10 include modifiche R7/R8/R9 e risparmioGPU | EXE separato; non sostituisce automaticamente la build attiva |

## Note operative

Sensori/polling e protezioni TEC rimangono separati dall'animazione; il budget watt è software e non un limite istantaneo garantito.
Il COP visualizzato è una stima con conduttanza ipotizzata di12W/C, non una misura del calore realmente rimosso.
Il verdetto osservativo non sostituisce il self-test del produttore. OCP da solo non conferma un guasto; un margine negativo attuale resta un problema reale da gestire.

## Fonti tecniche consultate

- EK Gen1: https://www.ekwb.com/custom-loop/quantumx-delta-tec-sub-ambient-cooling/?lang=de
- EK Delta2: https://www.ekwb.com/shop/ek-quantum-delta2-tec-d-rgb-full-nickel
- Modello termoelettrico Ferrotec: https://thermal.ferrotec.com/technology/thermoelectric-reference-guide/thermalRef11/

La combinazione modificata Gen1/Gen2 non è descritta da queste fonti come configurazione certificata. Non sono stati aumentati i limiti elettrici per inseguire il raffreddamento.
