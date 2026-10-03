# Build Gen 1 / TEC Gen 2 — riscontri reali e correzioni

## Hardware interrogato
Controller su COM5: HW 4; firmware 13.A0 (bytes 19,160,3,0). TEC Gen 2 su controller Gen 1, come indicato dall'utente.

## Prove eseguite sul controller
- Prima dell'accensione: PID 0/0/0, FAILSAFE e LOW_POWER attivi, 0 W.
- Sequenza offset + PID 100/1/0 + enable: ACK riuscito; lettura PID 100/1/0; PID_RUNNING attivo; piastra da circa 39 a 33 C.
- Comando di potenza 30%: NON e un limite affidabile. Lettura reale 98–100%, circa 246–260 W. L'uguaglianza tra percentuale richiesta e duty letta non e una conferma di un cap.
- Offset +2: circa 253–259 W. Offset +10: circa 6–10 W. Offset +20: circa 0.5–0.6 W. Prova breve, condizioni diverse da regime stazionario; non e una curva di COP.
- Ogni prova seriale diretta e terminata con disable confermato.
- Dashboard avviata, clic registrato nel log, REGIME-ACK con PID=true. Durante la successiva osservazione: piastra 18.6 C, circa 46 W, PCB 28.1 C, rugiada circa 14.6 C. Questo e un riscontro sotto il carico presente, non un confronto a pari carico con i picchi precedenti.

## Gestione introdotta
Il tasto principale ABILITA usa un percorso diretto che non salta il comando per un regime memorizzato. Mostra subito il clic ricevuto. Gli errori di regime e gli errori delle altre scritture sono distinti.
In Cryo, la regolazione cambia la domanda tramite offset e osserva piastra, rugiada, PCB e watt. Non deduce la temperatura della ceramica calda dalla PCB e non usa un COP inventato per comandare l'hardware.
Ricerca graduale: passo di offset 0.5 C, attesa di 30 s per valutarne la risposta; annulla un aumento che aggiunge watt senza migliorare la piastra. Mantiene una pausa di 120 s dopo un aumento inutile.
Controllo rapido della domanda quando il consumo supera il budget, il margine scende sotto 1.5 C oppure la PCB supera la soglia termica preesistente. Obiettivo di lavoro della piastra: circa rugiada +2 C, con banda 2.5–3 C. Soglie e tempi sono scelte software iniziali, non una taratura certificata di questo assemblaggio.
Budget software: 100% equivale a 200 W. Non e una dichiarazione del rating elettrico del controller e NON e un limite istantaneo: sono stati misurati picchi iniziali intorno a 260 W prima che la retroazione riducesse la domanda.
Se il monitoraggio fallisce ripetutamente, il watchdog richiede disable reale, invece di affidarsi al comando 30% ignorato dal Gen 1. Le perdite del canale e gli errori di accensione tentano anch'essi disable.
La regolazione agisce in Cryo. Unregulated resta la scelta manuale, con rischio di condensa dichiarato.

## Correzioni di interfaccia e diagnosi
Mostra budget watt e duty letto separatamente. Mostra offset attivo e stato della regolazione; il campo offset dell'operatore resta l'intenzione iniziale. Il COP preesistente e etichettato come stimato.
Trace locale: tec-controller.log accanto all'eseguibile, con rotazione intorno a 2 MB. Gli orari del logger sono UTC.

## Validazione
295 test software passati; prove hardware aggiuntive eseguite esplicitamente. I test hardware rimangono ignorati nella suite ordinaria, per evitare accensioni automatiche durante cargo test. La build release deve terminare senza errori prima della consegna.
La prova completa del nuovo eseguibile finale a carico elevato e prolungato resta da eseguire; non e dimostrato un optimum globale ne una percentuale di risparmio a pari carico.

## Fonti
https://github.com/juvgrfunex/cryo-cooler-controller — segnala il mancato funzionamento del limite massimo di potenza.
https://thermal.ferrotec.com/technology/thermoelectric-reference-guide/thermalRef11/ — modello TEC e dipendenza delle prestazioni da corrente e condizioni termiche.

## Uso
Chiudere l'altra dashboard anche dal tray prima di aprire StargateCryo-GEN1-TEC2-FINALE.exe. Non mantenere due programmi sulla stessa COM5. La dashboard attualmente aperta e raffreddante non e stata chiusa durante il confezionamento della build finale.

Ultima revisione R2: aggiunto terminatore UTF-16 al nome del mutex Windows; corrette etichette budget/duty, offset attivo e stato della regolazione; watchdog con disable reale in caso di errori ripetuti. Eseguibile finale: StargateCryo-GEN1-TEC2-FINALE-R2.exe.
