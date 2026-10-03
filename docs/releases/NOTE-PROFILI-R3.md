# Revisione R3: modalita e profili, 30 settembre 2026

Hardware: controller Gen 1 collegato alla TEC Gen 2 modificata.

## Difetti corretti
- Il gestore automatico di carico poteva riscrivere il setpoint in Unregulated. Adesso opera esclusivamente in Cryo e non sovrascrive i tre preset selezionati.
- Il caricamento profili convertiva il segno dell'offset e confondeva offset firmware con margine misurato dalla rugiada. I due significati sono separati.
- I profili precedenti avevano guadagni PID diversi senza taratura comparativa sul controller reale. I preset usano la combinazione 100/1/0 gia provata su questo hardware; la differenza di prestazioni viene richiesta mediante offset e budget.
- La percentuale di potenza veniva riscritta ogni cinque secondi pur non costituendo un limite affidabile. Rimosso il re-push.
- Allargata l'isteresi a +/-0.75 C rispetto all'obiettivo del profilo. Dopo una modifica ordinaria si attendono 30 secondi. Dopo riduzione per eccesso di watt si evita il rilancio per almeno 60 secondi; le protezioni possono intervenire prima.

## Preset iniziali, da validare a pari carico reale
| Profilo | Budget software | Obiettivo piastra sopra rugiada | Offset iniziale firmware |
| --- | --- | --- | --- |
| Silenzioso / Idle | 60 W | +6 C | +6 C |
| Gaming | 120 W | +3.5 C | +3 C |
| AI / Rendering | 160 W | +3 C | +2 C |

Budget = 200 W * percentuale / 100. Questi sono obiettivi di retroazione, NON limiti elettrici istantanei o rating del controller. Nelle prove precedenti sono stati osservati picchi iniziali di circa 260 W anche richiedendo 30%.
I tre nomi dei preset sono riservati: al caricamento vengono applicati i nuovi valori anche se la configurazione contiene la versione precedente. I profili personali con nomi diversi conservano l'offset, limitato a -30..50 C.

## Cryo e Unregulated
Cryo applica la regolazione software sul margine realmente misurato dalla rugiada e sui watt V*A. La riduzione della domanda ha precedenza sull'aumento del raffreddamento quando si supera il budget o il limite termico configurato.
Unregulated richiede offset -30 C e non usa l'ottimizzatore Cryo. Caricare un profilo non riscrive immediatamente PID o offset in questa modalita. Resta una richiesta aggressiva con rischio di condensa; non e il profilo efficiente consigliato.
Il passaggio alla modalita nativa Unregulated NON e stato dimostrato su questo Gen 1: un offset aggressivo non prova un cambiamento del bit TEMP_MODE. La dashboard conserva la distinzione tra richiesta e stato restituito dal controller. Non sono stati inventati comandi firmware per forzare il bit.

## Limiti fisici e validazione
Non esistono setter separati di tensione e corrente nel protocollo implementato. La richiesta di freddo e il solo controllo efficace verificato; V e A vengono letti, i watt sono calcolati da V*A. Una minore domanda puo ridurre l'assorbimento, ma non garantisce la riduzione indipendente di entrambe le grandezze.
La temperatura PCB non misura il lato caldo della TEC. Non si puo ricavare COP reale o calore estratto da PCB meno piastra. Il criterio di annullamento degli aumenti senza raffreddamento misurabile resta un'euristica, influenzata dalle variazioni del carico CPU.
298 test software passati (278 dashboard, 20 libreria), inclusi migrazione dei preset, conservazione offset personali, risposta diversa dei profili agli stessi sensori, isteresi e mancato rilancio immediato dopo eccesso di watt. I test hardware sono esclusi dalla suite ordinaria.
Questa revisione non e stata ancora misurata a carico Gaming/AI prolungato: nessuna percentuale di risparmio o massimo globale di efficienza e dimostrata.

Fonti consultate:
- https://github.com/juvgrfunex/cryo-cooler-controller (limite potenza non funzionante e OCP anomalo).
- https://thermal.ferrotec.com/technology/thermoelectric-reference-guide/thermalRef11/ (prestazioni TEC dipendenti da corrente e temperature).

Eseguibile: StargateCryo-GEN1-TEC2-PROFILI-R3.exe. Chiudere la dashboard precedente anche dal tray prima di aprire R3 per liberare COM5. Il programma raffreddante attualmente attivo non e stato interrotto durante questa revisione.
