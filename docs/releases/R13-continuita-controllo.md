# R13: continuita del controllo TEC

- La X nasconde la dashboard nel tray: non termina il controllo.
- Il menu tray "Esci e arresta il controllo" e l'uscita volontaria definitiva.
- Supervisore separato nello stesso eseguibile. Non apre COM e non interroga il controller. Attende la fine del processo dashboard; un arresto inatteso, anche con codice zero, causa riavvio dopo 2 secondi. Arresti ripetuti comportano attesa progressiva fino a 30 secondi.
- Dopo crash, riattiva Cryo SOLO se in questa sessione era stato richiesto raffreddamento attivo. Non ripristina automaticamente Unregulated. I profili caricati e le protezioni termiche/condensa restano in uso. Una richiesta di spegnimento annulla il recupero del raffreddamento.
- Mutex esistente: una seconda dashboard viene respinta senza ciclo di riavvii. Il supervisore non termina un processo bloccato e non sostituisce le protezioni termiche hardware.
- Panic ed errori UI registrati in LOCALAPPDATA/stargate-cryo/recovery.log, con rotazione a 1 MB. I panic dei thread sono registrati, ma non implicano necessariamente l'arresto dell'intero processo.
- Icone condivise con OnceLock: stessa immagine e identificativo GPU tra fotogrammi, invece di ricreare immagini ad ogni frame. Windows aveva registrato RADAR_PRE_LEAK_64 per R11; questo e un indizio di memoria, non la prova della causa della chiusura.

Collaudo senza hardware: --recovery-self-test inietta exit 70 nel primo processo figlio, verifica recupero dell'intento attivo nel secondo, poi esce volontariamente. Non apre porte COM e non abilita la TEC.

Limiti: non e ancora dimostrato il motivo della chiusura precedente; nessuna garanzia contro guasti del controller, mancanza di alimentazione o blocco del sistema operativo. La continuita reale dopo riavvio deve essere verificata sulla nuova build attiva, senza arrestare la dashboard attuale sotto carico.

Suite completa: 314 test superati (294 applicazione, 20 libreria). Il test delle icone verifica 1000 richieste successive per ciascuna delle 8 icone con identificativo invariato e distingue icone diverse.

Build release riuscita. SHA256 R13: 3F9460CD82CCBA867BBE120B2D79F94704774B552FD7F5BE76EABB35FE0B84EA.
Collaudo reale del supervisore superato: primo figlio exit 70, riavvio dopo 2 secondi, secondo figlio recupera intento abilitato, uscita volontaria codice 0 ferma il supervisore. Log conservato in recovery-self-test/stargate-cryo/recovery.log. LOCALAPPDATA reindirizzato solo durante il collaudo nella cartella del progetto, poi ripristinato. Nessuna porta COM aperta nel collaudo. Il recupero fisico TEC dopo crash non e stato provocato sulla macchina sotto raffreddamento.
