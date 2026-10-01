# Grafica R4 - 30 settembre 2026

Eseguibile: StargateCryo-GEN1-TEC2-GRAFICA-R4.exe.
Include le correzioni e i profili della R3.

## Cambiamenti
- Animazione richiesta ogni 33 ms (circa 30 fps) quando la dashboard collegata e visibile. Il timer di animazione si ferma nella schermata iniziale e quando nascosta dal tray. Il polling del controller e le protezioni mantengono la loro cadenza.
- Cache della geometria per evitare di ricostruire un grafico piu volte nello stesso intervallo di animazione. Invalidazione all'arrivo dei dati e al cambio del punto evidenziato.
- Rimossa la traslazione artificiale del timestamp dell'ultimo campione. La finestra visiva ritarda di 500 ms e il bordo della curva interpola linearmente tra due campioni realmente disponibili, senza sovraelongazione o extrapolazione. Questa presentazione non modifica i campioni delle letture numeriche, dei controlli TEC, delle registrazioni e delle esportazioni.
- Scala del grafico termico calcolata all'arrivo dei campioni invece di scandire tutte e tre le serie durante ogni disegno.
- Etichette degli assi con contrasto maggiore. Legenda colorata compatta sopra il grafico termico; margine dalla rugiada nella didascalia. I canali esistenti sono conservati.
- Le zone termiche usano poligoni riempiti; il punto terminale segue la curva visualizzata. Separazione discreta tra le card nella disposizione verticale.
- Il tooltip dei grafici singoli individua il campione usando l'asse temporale reale, anziche distribuire uniformemente il numero di campioni su tutta la larghezza.
- Campioni non finiti esclusi dai grafici per non danneggiare scala o rendering.

File modificati per questa revisione: charts.rs, main.rs, nuovo chart_preview.rs. Nessuna modifica a running.rs, alla libreria seriale, ai profili o all'ottimizzatore TEC rispetto alla R3.

## Verifiche e limiti
302 test software passati (282 dashboard, 20 libreria). Quattro nuove verifiche: interpolazione senza sovraelongazione e senza modificare i campioni, assenza di extrapolazione, associazione temporale del tooltip, campioni non finiti e scala termica.
L'anteprima isolata con dati simulati e stata avviata e si chiude da sola dopo 45 secondi. Il sistema di automazione desktop non ne ha esposto una finestra catturabile: la verifica visiva completa e i frame effettivi sul desktop dell'utente restano da confermare. Non viene dichiarato un risparmio CPU/GPU misurato. Aumentare la frequenza del disegno puo aumentare il carico grafico rispetto ai precedenti 10 Hz; la cache evita lavoro duplicato, ma non prova un consumo inferiore.

Per aprire la preview senza seriale, configurazione, database o tray:
StargateCryo-GEN1-TEC2-GRAFICA-R4.exe --preview-grafici
Per la disposizione larga aggiungere --wide. La finestra porta il titolo DATI SIMULATI e non rappresenta una misura del controller.

Per usare normalmente R4, chiudere la precedente dashboard anche dal tray per liberare COM5. Il processo R3 attualmente raffreddante non e stato interrotto durante questo intervento.
