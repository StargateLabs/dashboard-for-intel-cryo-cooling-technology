# Sicurezza

## Nessun binario Intel in questo repository

Questo repository contiene **solo codice sorgente, documentazione e immagini**.

**Non** sono inclusi, e non devono esserlo:

- i binari del pacchetto Intel Cryo Cooling Technology (`CryoCoolingService.exe`,
  `IntelCryoCooling.Controller.dll`, `TATInterface.dll`, `Intel.CryoCooling.*.dll`, …);
- l'installer Intel modificato con la lista CPU alterata;
- le licenze di terze parti allegate agli installer originali.

L'analisi di reverse engineering documentata in
[`docs/reverse-engineering/`](docs/reverse-engineering/) descrive il **comportamento
osservato** di quei file. Chi desidera riprodurre le misure deve estrarre i binari
dal proprio pacchetto Intel legittimamente acquisito.

I nomi dei file e i relativi hash MD5 sono citati a scopo identificativo.

---

## Rischio hardware: condensa

Il TEC raffredda **sotto il punto di rugiada**. L'acqua sulla piastra è il danno che
danneggia per primo l'hardware.

- HW spento e **piastra asciutta** prima di ogni collegamento.
- Non usare mai il software ufficiale Intel insieme a questa dashboard: occupa la
  stessa porta COM. Chiudere l'altra dashboard **anche dal tray**.
- Non usare la temperatura CPU come riferimento per ventola e pompa: il TEC raffredda
  la CPU, quindi il resto del circuito di liquido riceve il calore tolto e il segnale
  è falsato. Il testo è nel manuale del produttore.
- Non alzare le soglie di protezione per "raffreddare di più": il firmware taglia a
  80 °C e 90 °C, la guardia software sta volutamente sotto.

---

## Opcode pericolosi

| Opcode | Effetto | Regola |
|---|---|---|
| **`0x1E`** | **reset di fabbrica**: perde PID, setpoint, tetto di potenza | non deve mai essere emesso, né come fallback di connessione |
| `0x18` `[0,0,0,0]` | **abilita** il TEC (polarità opposta al nome del metodo) | da trattare come scrittura, mai come lettura |
| `0x15` `0x16` `0x17` | azzerano i guadagni PID | validare prima dell'invio |
| `0x1D` | porta il tetto di potenza a **0 %** | validare prima dell'invio |
| `0x14` | azzera il setpoint | non emettere a caso per "provare" |

Gli opcode inviati sono filtrati da una **allowlist di sola lettura**
(`SOLO_LETTURE` in `cryo_cooler_controller_lib/src/lib.rs`). La sequenza di
accensione è a 7 comandi e ogni errore seriale interrompe la sequenza e tenta
un disable reale.

---

## Limiti dichiarati

- Il **budget in watt è un obiettivo di retroazione software**, non un limite
  elettrico istantaneo né il rating del controller. Sono stati misurati picchi
  iniziali ~260 W prima che la retroazione riducesse la domanda.
- L'etichetta "200 W" del kit Gen 1 **non è un tetto erogabile**: il controller
  misurato regge 220–237 W in modo stabile. La costante corrispondente nel codice
  è un **avviso**, mai un blocco.
- L'**OCP non è una protezione**: si accende a 73 W e a 112 W con sistema sano.
  Non deve mai ridurre la potenza in automatico in modo persistente.
- Il **COP mostrato è una stima** con conduttanza ipotizzata `12 W/°C`. Non è una
  misura di calore rimosso.
- Le **modalità native del firmware non sono certificate**: `Unregulated` e
  `Standby` sono raggiunti tramite offset, per compatibilità col comportamento
  osservato.
- I **14 bit di stato sconosciuti non vengono interpretati**: sono registrati grezzi.
  Un codice errato in diagnostica fa diagnosticare il problema sbagliato.

---

## Credenziali

Nessuna credenziale è inclusa nel repository. L'AI Advisor legge `ANTHROPIC_API_KEY`
dall'ambiente:

```bash
export ANTHROPIC_API_KEY=...        # Linux
setx ANTHROPIC_API_KEY ...          # Windows
```

Il client non viene mai eseguito senza la variabile presente.

---

## Avvisi di sicurezza delle dipendenze

GitHub segnala sei avvisi su `Cargo.lock`. Uno è stato corretto, cinque non hanno una correzione
compatibile con le versioni attuali delle dipendenze.

| Gravità | Pacchetto | Nel lock | Corretta in | Stato |
|---|---|---|---|---|
| alta | `rustls-webpki` | 0.101.7 | 0.103.13 | accettato |
| media | `glib` | 0.16.9 | 0.20.0 | accettato, non compilato su Windows |
| bassa | `rustls-webpki` ×2 | 0.101.7 | 0.103.12 | accettato |
| bassa | `lru` | 0.12.5 | 0.16.3 | accettato |
| bassa | `rand` | 0.8.5 | 0.8.6 | **corretto a 0.8.8** |

### Perché i cinque rimanenti sono accettati

**`rustls-webpki`, tre avvisi.** L'unica connessione in uscita del programma è una `POST` a
`https://api.anthropic.com/v1/messages`, in `src/ai_advisor.rs`. L'avviso alto è un denial of
service che richiede a un server di inviare un certificato malformato: servirebbe una posizione
man-in-the-middle con un certificato firmato da un'autorità riconosciuta. I due avvisi bassi
riguardano i vincoli di nome dei certificati, applicabili ai certificati con autorità
intermedia, non al foglia emesso per un dominio pubblico. La correzione richiede
`rustls-webpki` 0.103, che `reqwest` 0.11 non ammette: servirebbe passare a `reqwest` 0.12.

**`glib`.** Dipendenza del backend Linux di `iced`. Non entra nel collegamento su Windows, che è
la piattaforma di questo progetto. La correzione richiede una major (`0.16` → `0.20`).

**`lru`.** L'avviso riguarda `IterMut`, che questo progetto non usa. La correzione richiede una
major (`0.12` → `0.16`).

### Non è una scusa per non aggiornare

Le major vanno fatte quando si tocca quel codice. Finché il blocco non cambia, il rischio sopra
descritto è accettato per iscritto.

---

## Segnalazione

Le correzioni di sicurezza si riportano come issue privata, senza allegare binari
proprietari.