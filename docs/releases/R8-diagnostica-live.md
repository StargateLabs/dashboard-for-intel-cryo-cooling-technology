R8 - diagnostica live e interfaccia (30 settembre 2026)

La R8 include le correzioni termiche e ai comandi della R7.
Nella R7 sul dispositivo sono stati osservati ACK e lettura offset corretta per Cryo (+2 C) e Unregulated (-30 C).
Questo conferma i comandi seriali, non certifica le modalità native del firmware o la dissipazione massima sotto carico.

Diagnostica:
- Il margine attuale sostituisce il minimo storico nel verdetto live. Il minimo rimane nelle statistiche della sessione.
- Volt, ampere e watt provengono dall'ultimo campione, anziché mescolare medie e valori attuali.
- OCP rimane un segnale da controllare; da solo non conferma un guasto hardware.
- Gli avvisi rientrati non vengono mostrati come ancora attivi.
- COP indicato come stima. Formula: max(0,12*(CPU-piastra)/watt), limitata a5; non è una misura di calore rimosso né di rendimento reale.

Interfaccia:
- Modalità selezionata evidenziata, entrambe le modalità sempre cliccabili.
- Valori TEC, CPU e rugiada nella testa del primo grafico.
- Gocce vettoriali blu solo per un campione fresco con piastra sotto rugiada.
- Pannello diagnostica coordinato al tema scuro.
- Diciture di comando, pompa, sensori, duty e profili riviste.
- Sfondo, immagini ed effetti esistenti conservati.

Anteprima indipendente: avviare con --preview-grafici --condensa per simulare il caso sotto rugiada; aggiungere --wide per layout largo. Nessuna connessione seriale.

Controlli software: 310 test superati. Test e log build nella stessa cartella.
La verifica sotto carico della CPU e la dissipazione fisica restano da completare. Nessuno stress test automatico o sostituzione della dashboard attiva.
Verifica visiva: anteprima eseguita e terminata senza errori, ma Computer Use non ha esposto la finestra per acquisire lo screenshot. Layout completo R8 e gocce non ancora confermati visivamente. La R7 resta attiva.
