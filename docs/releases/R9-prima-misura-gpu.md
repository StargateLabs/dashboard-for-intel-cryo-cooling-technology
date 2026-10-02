# R9: prima misura del sovraccarico GPU

Build: `StargateCryo-GEN1-TEC2-R9.exe`
SHA256: `6D4EDCADBB90DC61A45A9F225AED7DD10D838799636E5EE4CCC86B052187827F`
Data del file: 30 settembre 2026, 08:05.

## Misura

Cinque campioni consecutivi, un secondo di distanza, conservati in
[`dati-gpu-r9-prima.csv`](dati-gpu-r9-prima.csv):

| Ora | MaxEnginePct | TotalEnginePct |
|---|---|---|
| 08:13:17 | 14,030 | 14,030 |
| 08:13:18 | 12,709 | 12,709 |
| 08:13:19 | 13,504 | 13,504 |
| 08:13:20 | 13,897 | 13,897 |
| 08:13:21 | 15,037 | 15,037 |

Minimo **12,709%**, massimo **15,037%**. `MaxEnginePct` e `TotalEnginePct` coincidono in tutti e
cinque i campioni.

Questa è la misura **prima** dell'intervento di risparmio GPU. Serve da riferimento per R10 e
R11.

## Limiti dichiarati

- La misura è taken con la dashboard **in primo piano**. Non dice nulla del consumo a finestra
  nascosta o nel tray.
- Cinque campioni in cinque secondi coprono un intervallo molto breve: non descrivono il
  comportamento nella mezz'ora di un carico reale.
- Nessuna nota di collaudo in markdown esiste per questa build. Restano disponibili
  `build-R9.log` e `test-R9.log` nella cartella di collaudo.
- Il numero di test superati da questa build non è registrato.

## Discrepanza con le note successive

La nota di R11 attribuisce a R10 un consumo fra 1,2% e 1,8% basandosi su `GPU-R10.csv`. La nota
di R10, scritta al momento della build, dice invece che la misura su R10 era ancora da
completare. Le due affermazioni non possono essere entrambe vere nello stesso momento.

La tabella in questa cartella conserva i file come sono, senza scegliere quale delle due note
sia corretta: la verifica va rifatta sulla build attiva per chiudere il punto.