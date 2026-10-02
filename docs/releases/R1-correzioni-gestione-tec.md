# Correzioni gestione TEC: 30 settembre 2026

## Difetti risolti
- La commutazione non impostava `Commutazione::Richiesta`: gli ACK non potevano confermare il comando corrente.
- Una rilettura discordante sostituiva l'intenzione dell'operatore. Ora conserva la richiesta originale.
- Ripetere un comando fallito era impedito dal controllo “gia attivo”. Ora vale solo per un regime confermato.
- La rampa partiva prima dell'ACK e dal livello precedente. Ora parte dopo l'ACK dal livello iniziale della sequenza.
- La rampa non aumenta con temperatura non valida o margine anticondensa sotto 1 °C.
- Le scritture di potenza usano il percorso di readback esistente: un valore ignorato dal firmware produce errore, non un livello applicato fittizio.
- Un errore seriale interrompe la sequenza di accensione e tenta disable; un disable fallito viene dichiarato esplicitamente.
- PID, offset e potenza vengono validati prima di iniziare l'accensione. NaN, infinito, PID negativi e potenza oltre 100% sono rifiutati.
- L'ACK di spegnimento e distinto dall'ACK di accensione: la chiusura non puo consumare un ACK di accensione precedente come conferma di spegnimento.
- La coda viene consumata anche dopo la chiusura del canale richieste; l'attore tenta disable prima di rilasciare la porta.
- La modifica del setpoint usa anche l'ultimo regime richiesto, evitando che un bit PID assente renda il comando inefficace.
- Il test commissioning usa un file temporaneo e non elimina il log reale ne modifica le variabili d'ambiente del processo.

## Verifica
`cargo test --workspace --offline --quiet`: 289 test passati. Due test di integrazione sono intenzionalmente ignorati: uno richiede hardware, uno modifica il database reale.
Aggiunte prove della sequenza seriale, di errore su ognuno dei sette comandi di accensione, di disable fallito, di parametri invalidi e di rilettura discordante.

## Fonti e limiti
Fonte originale: https://github.com/juvgrfunex/cryo-cooler-controller
Segnala esplicitamente che il limite massimo di potenza non funziona su alcuni firmware. Il readback e una verifica del registro esposto, non una certificazione della potenza elettrica massima.
Manuale EK Delta2: https://www.ekwb.com/shop/EK-IM/EK-IM-3831109859612.pdf
I documenti storici del progetto contengono affermazioni discordanti su modalita, GPIO, offset e temperatura PCB. Queste correzioni non certificano tali interpretazioni. Le soglie termiche esistenti non sono state ritarate senza misure.
La compilazione e i test simulati non sostituiscono il collaudo del controller collegato. Non e stata attivata la TEC durante questo lavoro. Restano da misurare: efficacia fisica del limite, corrispondenza dei bit di stato e transizioni Cryo/Unregulated/Standby sul firmware effettivo.
