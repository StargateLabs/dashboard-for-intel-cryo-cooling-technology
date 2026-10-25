# StargateCryo — aggiornamento desktop del 23 ottobre 2026

Questa pubblicazione distribuisce l’eseguibile effettivamente in uso sul desktop e aggiorna i sorgenti dal progetto di lavoro conservato nel backup. Il file mantiene il nome `StargateCryo-GEN1-TEC2-R16.exe`; il suo hash coincide con quello annotato alla conclusione dell’integrazione TF2.

## Modifiche documentate

- **Telemetria del circuito:** lettori Thermaltake TF3 e TF2, grafici dedicati e gestione esplicita dei campioni mancanti o scaduti.
- **Un solo lettore TF2:** con SignalRGB attivo, la dashboard usa i campioni pubblicati dal plugin nel log. Senza SignalRGB utilizza il lettore USB nativo. Dopo cinque secondi senza un campione valido mostra N/D.
- **Clock CPU corrente:** lettura Core Temp v2, con massimo fra i core al momento del campione. Dati assenti o non validi restano N/D; non viene mostrato il picco storico come clock corrente.
- **Interfaccia:** badge della frequenza, titoli dei grafici più chiari, gestione dell’avvio automatico nello stato operativo e conferma visiva corretta per il salvataggio riuscito.
- **Rendering:** cache del grafico ridisegnata quando cambiano dati o interazione, mantenendo il renderer GPU della versione verificata.
- **Diagnostica:** spiegazioni OCP e COP corrette; eliminata la conversione non verificata tra percentuale del comando e watt massimi. Log dei panic con backtrace; supervisore esistente conservato.

## Provenienza e versioni

La release pubblica è `v2.9.0`. La versione interna del pacchetto desktop resta `2.5.0`, come nei sorgenti di lavoro; non è stata cambiata soltanto per allineare il numero del tag. La serie R16 identifica la linea di build, non l’assenza di aggiornamenti successivi.

SHA256 dell’eseguibile desktop:
`DFB9CAE8468DA8DC7114B85D5E534001712179E73EDB034CEF60599636DDB8F8`

Le note del progetto riportano 315 test unitari per questa integrazione. I risultati rieseguiti durante la pubblicazione sono riportati separatamente in `PUBLICATION-VERIFICATION.md`. Non è stata dimostrata una build riproducibile byte per byte fra sorgenti e binario desktop.

## Limiti da conoscere

Il collaudo riguarda l’impianto modificato del laboratorio: controller Intel Cryo Gen 1 e cella TEC Gen 2. Le soglie non sono una certificazione per altri impianti. La dashboard non applica un filtro sul modello della CPU: questo non dimostra compatibilità meccanica, elettrica o termica con qualsiasi CPU.

La ripresa della pressione TF3 è stata osservata dopo cambio USB e riapertura della dashboard; la ripartenza dopo rimozione completa dell’alimentazione non è dimostrata. Il valore di portata letto dal log richiede il plugin SignalRGB compatibile. L’assenza definitiva dei crash intermittenti originari non è dimostrata. Nessuna calibrazione o installazione firmware fa parte di questa release.

## Schermata aggiornata

[Versione desktop attuale](../images/dashboard-current.png), fornita dal proprietario il 25 ottobre 2026. Immagine originale conservata senza ritagli; cornice SVG arrotondata per la presentazione.
