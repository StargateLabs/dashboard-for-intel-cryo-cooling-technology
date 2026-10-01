# Verifica R12

R12 mantiene la gestione TEC e il buffer visivo di un secondo della R11.
Il pulsante TEC ora contiene un'icona dedicata, titolo e descrizione, potenza attuale e grafico incorporato. Fondo sfumato, bordi arrotondati e risposta al passaggio del mouse; nessuna nuova animazione continua.
I messaggi di abilitazione e disabilitazione e i comandi Cryo/Unregulated restano quelli della R11.

Validazione: 311 test superati (291 applicazione, 20 libreria). Il consumo GPU misurato sulla R11 era in media 12,406%, coerente con il 10-15% segnalato dall'utente. R12 richiede una nuova misura sulla dashboard attiva; il restyling non dimostra una riduzione GPU.

Restano da confermare visivamente il nuovo pulsante, la fluidita percepita e le gocce nell'anteprima isolata. La prova che portava la CPU a 90 gradi non e stata riprodotta nelle osservazioni disponibili. Il comando Unregulated usa la compatibilita tramite offset verificato sul controller Gen 1; non certifica il modo nativo del controller Gen 2.

Build release completata con successo. Eseguibile: StargateCryo-GEN1-TEC2-R12.exe.
