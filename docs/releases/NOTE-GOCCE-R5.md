# Indicatore condensa R5

Include tutti i fix e la grafica della R4.
Tre gocce vettoriali blu nella testata del primo grafico, con sfumatura zaffiro, bordo chiaro e riflessi. Geometria conservata in cache; nessuna animazione o nuova dipendenza.
Visibili soltanto se l'ultima coppia di letture TEC/rugiada e valida, contemporanea, recente (entro 5 secondi) e TEC < rugiada. Alla soglia esatta o sopra la soglia le gocce non sono visibili. Una lettura mancante, non finita o scaduta non genera l'indicatore.
Il tooltip chiarisce che si tratta di rischio condensa: non dimostra la presenza fisica di acqua.
La testata mantiene altezza fissa anche quando le gocce sono nascoste, evitando spostamenti del grafico.
Gestione TEC, profili, controlli seriali e dati registrati invariati.
304 test software passati, compresi i controlli sulla soglia stretta e sulle letture scadute o non allineate. La verifica visiva sul desktop resta da confermare come per R4.
Eseguibile: StargateCryo-GEN1-TEC2-GOCCE-R5.exe. Chiudere la versione precedente anche dal tray prima di avviarlo.
