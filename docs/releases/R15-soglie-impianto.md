# Verifica R15 — controller Gen 1 modificato / TEC Gen 2

## Limiti dell'impianto richiesti dall'utente
- Riduzione della domanda Cryo da 37 °C PCB; nessuna limitazione termica a 34 °C.
- A PCB >=38 °C: richiesta DISABLE reale, prioritaria nella coda seriale. Non si usa il registro percentuale come sostituto dello spegnimento.
- Abilitazione consentita solo con temperatura PCB valida e inferiore a 37 °C. Dopo uno stop termico occorre riabilitare manualmente.
- Limiti specifici di questo impianto modificato, non certificazioni del costruttore.

## Correzioni dell'audit
- Sensori NaN, infiniti e valori fuori intervallo rifiutati prima del controllo; età telemetria basata sull'ultimo campione valido.
- Coda seriale accorpa richieste superate e dà precedenza allo spegnimento. Test di 1000 aggiornamenti verificano che non cresca senza controllo.
- Conferme Cryo/Unregulated abbinate all'offset richiesto; una risposta di una modalità precedente non conferma quella nuova. Regolatore sospeso durante la commutazione.
- Offset ammesso -30..50 °C; PID 0..1000; percentuale 0..100. Parametri non validi rifiutati prima della transazione di abilitazione.
- Ripristino seriale R14 mantenuto: rilascio della vecchia porta dopo errori ripetuti, scansione unica e nuovo tentativo con intento Cryo precedente.
- Supervisore separato mantenuto. Poll bloccato oltre 15 secondi termina il processo controllato per consentire il recupero; uscita volontaria distinta dal crash.
- Nessun reset di fabbrica aggiunto.

## Profili e grafica conservati
- Gaming: budget software 120 W, margine target rugiada +3.5 °C.
- Silenzioso: 60 W, +6 °C. AI/Rendering: 160 W, +3 °C.
- Domanda corretta in base a watt e temperature misurati; CPU calda può accelerare il recupero, senza scavalcare controller/condensa.
- Grafici con buffer visivo 1 secondo e circa 30 fps; sensori, allarmi e controllo non ritardati dal buffer.
- Cache delle icone, card TEC con grafico interno e indicatore gocce conservati. Gocce solo sotto rugiada con dati freschi.
- OCP isolato resta segnalazione da verificare, non prova sufficiente di guasto; COP resta stima.

## Verifica e limiti
- Suite workspace: 297 test applicazione +23 libreria passati. Cinque prove che toccano hardware/database reale escluse esplicitamente.
- Test delle soglie: 37.9 °C non spegne, 38/40 °C attivano lo stop; 34/36.9 °C consentono l'abilitazione, 37 °C la bloccano.
- Controllo visivo della R14 attiva: PCB 29.3 °C, TEC 64.76 W, CPU 49 °C al momento del campione. Nessuna interruzione del raffreddamento effettuata.
- Il build e i test software non dimostrano la risposta fisica al DISABLE, l'assenza di overshoot fra campioni, il recupero da ogni blocco dell'interfaccia o la stabilità dopo un vero distacco dell'alimentazione. Queste prove sull'impianto restano da confermare.
- Unregulated usa l'offset compatibile Gen 1; il bit modalità nativo su hardware misto non è certificato. La riduzione automatica Cryo non opera in Unregulated, ma resta lo stop PCB a 38 °C.
- Il budget watt è un obiettivo con feedback, non un tetto elettrico istantaneo: il registro percentuale Gen 1 non ha mostrato un limite watt affidabile.

Log test: test-R15.log. Log compilazione: build-R15.log. Test supervisore: recovery-self-test-R15/stargate-cryo/recovery.log.

## Esito build finale
Compilazione release riuscita in 1m45s. SHA256 R15: 1D6497AF5999255BE17F8ED750BC1678184DDED4906BFCC6FF0563989570B481.
Self-test supervisore concluso con exit 0: primo figlio exit70 simulato; riavvio dopo2s; recupero intento Cryo; secondo figlio uscita intenzionale. Nessun accesso al controller durante questa prova.
Cinque avvisi di compilazione riguardano funzioni/elementi non usati; non sono errori di build.
