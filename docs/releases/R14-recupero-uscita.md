# Verifica R14 - avvio a freddo e recupero USB

Versione di riferimento dell'utente: C:/Users/Stargate/Desktop/cryo_cooler_controller.exe, SHA256 04495C1701434E96D927F2EE42A34A6F996E6F6B1F2E8C7CDE3D54B7D240F71E, data file 29 settembre 2026. Durante il lavoro questa versione resta in esecuzione; nessuna porta COM e stata aperta dalle prove.

Evidenze:
- Il log della R13 sul Desktop riporta errori Windows 22 e centinaia di letture fallite, seguite da ripetuti comandi di spegnimento sulla stessa connessione.
- Il messaggio "Heartbeat: nessun battito: round fallito" era costruito anche quando falliva una lettura monitor: mascherava la vera causa.
- Non e stato misurato il consumo CPU della R13 durante l'errore: l'utente ha chiuso tale versione durante il lavoro e riaperto quella vecchia. Il quasi 100% resta una segnalazione dell'utente, non una misura di questo collaudo.

Correzioni:
- Dopo tre round falliti, un unico recupero riporta alla schermata di connessione, tenta lo spegnimento con attesa limitata e rilascia la vecchia connessione seriale; quindi ripete il rilevamento USB.
- Dopo riconnessione recupera Cryo se era stato richiesto raffreddamento attivo. Una richiesta di spegnimento resta spenta. Non ripristina Unregulated automaticamente.
- Una sola scansione seriale simultanea; pausa di 2 secondi, massimo 20 tentativi per permettere l'avvio del controller dopo interruzione completa di alimentazione.
- Timeout di lavoro 300 ms, pulizia iniziale dei byte ricevuti, errore con opcode e causa originale invece del messaggio generico.
- Animazione dei grafici sospesa durante gli errori di telemetria e fino al primo campione valido. Il controllo e i tentativi di recupero restano attivi.
- Nessun reset di fabbrica 0x1E automatico. Restano protezioni e supervisore della R13.

Validazione: 315 test superati (295 applicazione e 20 libreria), incluso test contro scansioni sovrapposte. Il nuovo recupero su controller fisico dopo vera perdita di alimentazione deve essere verificato sulla R14: la prova non e stata eseguita interrompendo il raffreddamento della vecchia app.

Build release completata con successo. SHA256 R14: D3969A6D081D37144C8F3D4E42DF5C97390EEAD8D233071D53A96290B2025388.
Collaudo del supervisore R14 superato: exit 70 simulato, riavvio dopo 2 secondi, intento attivo recuperato, uscita volontaria termina il supervisore. Nessun accesso al controller durante il test. Log: recovery-self-test-R14/stargate-cryo/recovery.log.
