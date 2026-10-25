# Publication verification — 25 October 2026

- Desktop executable SHA256 matches the recorded final integration note: `DFB9CAE8468DA8DC7114B85D5E534001712179E73EDB034CEF60599636DDB8F8`.
- Updated Rust source copied from the working `stargate-cryo-fixed` snapshot; private settings, databases, logs and original Intel binaries excluded.
- Fresh offline MSVC release tests: **322 application tests + 28 library tests passed (350 total)**; 0 failures. Five hardware/database-dependent integration tests remained ignored. Compilation completed successfully.
- The controller was not stopped, reconfigured or probed during publication. Hardware-dependent tests remain ignored.
- Official logos are preserved. The current dashboard screenshot was supplied by the owner on 25 October 2026; previous images remain available as historical assets.
- Source-to-binary byte reproducibility has not been established.
