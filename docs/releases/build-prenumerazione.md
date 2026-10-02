# Le quattro build precedenti alla numerazione

Queste quattro build non hanno un numero della serie R. La numerazione parte da
`PROFILI-R3`, che è il primo eseguibile con numero.

Nessuna delle quattro ha una nota di collaudo. Per tutte e quattro l'unico dato disponibile è
la data del file e la dimensione.

| # | File | SHA256 | Data | Byte |
|---|---|---|---|---|
| 1 | `StargateCryo-TEC-20260930.exe` | `54FC25ED1D591B5601D2FDF7FD55B6E64EE098F311B71E9BAA994FD1A0637DB18` | 30/09 06:03 | 25.330.176 |
| 2 | `StargateCryo-GEN1-TEC2.exe` | `49F85F38D9F8F435E6634DC49F882F58C8CA0E63F0A3D2D9DC9475642B011BC1` | 30/09 06:21 | 25.335.296 |
| 3 | `StargateCryo-GEN1-TEC2-FINALE.exe` | `49711138F626B1E0661A118E44DB01BDD366B7EEFAA0A476CDDF464BD4F5FCAD` | 30/09 06:24 | 25.335.808 |
| 4 | `StargateCryo-GEN1-TEC2-FINALE-R2.exe` | `18634B4EDA9394AD69B9C0117A61DB30FBD5B9F9008504172B5FA0463F820962` | 30/09 06:33 | 25.334.272 |

## Il suffisso `-R2` non è la R2 della serie

`FINALE-R2.exe` è il **secondo tentativo** della build `FINALE`. Non ha a che fare con la R2
della serie, che non ha eseguibile proprio: per R2 esiste solo la nota di lavoro
[`R2-gen1-tec2.md`](R2-gen1-tec2.md). Lo stesso vale per `FINALE`, il cui nome dichiara una
build data per buona senza che esista un collaudo a supporto.

## Conseguenza per chi le usa

Non c'è modo di sapere cosa contengano. Prima di usarne una su un impianto in funzione, il
log di compilazione va letto e la build va provata: non è possibile stabilire dalle note se
una delle quattro abbia una regressione.

## Release su GitHub

| File | Release |
|---|---|
| `StargateCryo-TEC-20260930.exe` | `v1.0` |
| `StargateCryo-GEN1-TEC2.exe` | `v1.1` |
| `StargateCryo-GEN1-TEC2-FINALE.exe` | `v1.2` |
| `StargateCryo-GEN1-TEC2-FINALE-R2.exe` | `v1.3` |