# StargateCryo per Ubuntu · sorgenti

Questa cartella conserva lo snapshot delle sorgenti della variante Ubuntu di StargateCryo Emerald, separato dalla versione desktop Windows alla radice della repository. Include la dashboard Rust, la libreria del controller, le risorse grafiche e le regole di installazione presenti nella copia Ubuntu. Non include pacchetti `.deb`, configurazioni dell'utente, registri o build già compilate.

La dashboard può leggere controller Cryo e sensori Thermaltake TF2/TF3; i dispositivi, i permessi e le funzionalità disponibili dipendono dall'impianto. Le integrazioni specifiche di Windows non sono automaticamente disponibili su Linux. Il progetto locale documenta anche la gestione opzionale delle ventole: verificare dispositivi, porte e impostazioni prima dell'uso.

## Compilazione

In un ambiente Ubuntu con Rust e i pacchetti di sviluppo necessari:

```sh
sudo apt install build-essential pkg-config libudev-dev libgtk-3-dev libxkbcommon-dev libxdo-dev
cargo build --locked --release
```

Eseguire i comandi da questa cartella. Il lockfile proviene dallo snapshot Ubuntu; versione del compilatore, dipendenze di sistema e disponibilità dei pacchetti vanno verificate per la distribuzione utilizzata. Lo script `cryo_cooler_controller/install.sh` prepara un pacchetto Debian; installare il pacchetto aggiunge le regole USB e ne richiede la lettura preventiva.

## Stato delle verifiche e sicurezza

I manifest Cargo e il lockfile sono stati controllati sintatticamente nella copia preparata per la pubblicazione. La toolchain Rust non era disponibile nell'ambiente di preparazione: questa copia non è stata ricompilata né provata su hardware. Il test `probe_real` include verifiche esplicitamente escluse dai test normali, alcune delle quali inviano comandi al controller: non eseguirle senza comprendere gli effetti sul proprio impianto. I valori e le soglie del laboratorio non costituiscono certificazione per altri sistemi.

Licenza: [MIT](LICENSE). La provenienza e i limiti della versione Windows rimangono descritti nella documentazione principale.
