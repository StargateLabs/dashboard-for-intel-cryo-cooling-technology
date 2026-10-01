# Aggiornamento finale R11

R11 include tutte le correzioni TEC, modalità, profili, diagnostica e diciture delle versioni precedenti.

Richiesta GPU e fluidità:
- R9: GPU12.7-15.0% nei cinque campioni conservati in GPU-R9-prima.csv.
- R10: GPU1.2-1.8% nei cinque campioni conservati in GPU-R10.csv; la frequenza grafica era ridotta e il risultato non è comparabile a30fps.
- R11: animazione circa30fps anche senza focus; resta spenta in Home e quando nascosta nel tray.
- Il multisampling4x resta disattivato; testo e filtroimmagini mantenuti.
- Ritardo visivo curve1000ms richiesto dall'utente, interpolazione tra campioni reali. Valori live, controllo, allarmi e log non sono ritardati.
- CampionamentoCPUpergrafico ogni1s; invioCPUcontroller mantiene l'intervallo precedente.
- Etichetteassi ridotte nei grafici bassi, avvisi risolti rimossi e chiusura avviso riferita al canale corretto.

Validazione:311test superati.
La misurazioneGPUdiR11 e la conferma visiva della fluidità devono essere completate sulla build attiva.

Riferimenti di verifica precedenti: VERIFICA-RICHIESTE-R10.md. Le righe su15fps e stopfuorifocus sono superate dallaR11.
Il raffreddamento a90C non è stato riprodotto nelle schermate della prova osservata; non viene dichiarato definitivamente risolto.
Le gocce blu sono testate come condizione e vettori nel codice; la verifica visiva in anteprima rimane richiesta.
Collaudo R11 attiva: ACK offset+2 verificato; piastra18.6C, margine+3.5C, TEC98.8W nella schermata osservata. GPU10campioni: media12.406%, minimo11.121%, massimo13.504%. Log GPU-R11.csv. La fluidità percepita resta da confermare con lutente.
