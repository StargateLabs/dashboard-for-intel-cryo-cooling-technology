R7 - correzioni termiche e comandi (30 settembre 2026)

Questa build è candidata alla verifica sul controller Gen 1 modificato per TEC Gen 2.
Non sostituisce automaticamente la dashboard attiva.

Correzioni:
- InAttesa::Nessuna non blocca più i comandi di cambio modalità.
- Il pulsante Cryo resta cliccabile anche per ripetere una richiesta.
- Priorità ai sensori CPU die/core rispetto alla lettura AIDA64 CPU generica; a pari priorità usa il più caldo.
- Riduzione per budget watt: un grado di offset ogni 30 secondi invece di due ogni 2 secondi.
- Recupero del raffreddamento quando i watt scendono sotto metà budget, dopo assestamento.
- Con CPU >=85 C: richiesta di raffreddamento più rapida entro il budget esistente. 85 C è un'indicazione di domanda, non una soglia certificata del processore.
- La condensa e la temperatura controller continuano ad avere priorità.
- Un aumento della temperatura piastra non viene interpretato da solo come aumento inefficiente da annullare.

Limiti:
- Nessuna conferma del comportamento sotto carico sul dispositivo reale per questa build.
- Modalità native e lettura offset devono essere confermate sul firmware Gen 1 presente.
- Il registro percentuale non costituisce un limite watt verificato su questo controller.
- Nessun aumento dei limiti elettrici della modifica hardware.
- Gocce visibili solo se la piastra misurata scende sotto la rugiada; nello screenshot piastra34.2, margine+19.3, quindi devono rimanere nascoste.

Validazione: 309 test superati. Log: test-R7-termica.log.
