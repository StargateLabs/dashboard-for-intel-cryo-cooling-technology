# StargateCryo — 23 October 2026 desktop update

This publication distributes the executable currently used on the desktop and updates source from the working project retained in the backup. Its filename remains `StargateCryo-GEN1-TEC2-R16.exe`; its hash matches the final TF2 integration note.

## Documented changes

- **Loop telemetry:** Thermaltake TF3 and TF2 readers, dedicated charts and explicit missing/stale sample handling.
- **Single TF2 reader:** when SignalRGB is running, the dashboard consumes samples published by its plugin in the log. Otherwise it uses the native USB reader. Data older than five seconds becomes unavailable.
- **Current CPU clock:** Core Temp v2 readings use the highest current core frequency, not a historical peak. Missing or invalid data remains unavailable.
- **Interface:** frequency badge, clearer chart titles, startup toggle handling in the running state and correct success styling for saved settings.
- **Rendering:** chart cache redraws when data or interaction changes, retaining the verified GPU renderer.
- **Diagnostics:** corrected OCP and COP explanations; removed an unverified conversion between power-command percentage and maximum watts. Panic backtraces and the existing supervisor are retained.

## Provenance and versioning

The public release is `v2.9.0`. The working desktop package version remains `2.5.0`; it was not changed solely to match the release tag. R16 denotes a build lineage, not the absence of subsequent updates.

Desktop executable SHA256:
`DFB9CAE8468DA8DC7114B85D5E534001712179E73EDB034CEF60599636DDB8F8`

Project notes record 315 unit tests for this integration. Checks rerun during publication are recorded separately in `PUBLICATION-VERIFICATION.md`. Byte-for-byte reproducibility between the source snapshot and desktop binary has not been established.

## Known limits

Validation concerns the modified lab setup: Intel Cryo Gen 1 controller with a Gen 2 TEC assembly. Thresholds are not certified for other systems. The dashboard does not filter CPU models; this does not establish mechanical, electrical or thermal compatibility with every CPU.

TF3 pressure recovery was observed after changing USB port and reopening the dashboard; recovery after complete power removal remains unproven. Log-based TF2 readings require the compatible SignalRGB plugin. Permanent elimination of the original intermittent crashes is not established. This release does not install firmware or change calibration.
