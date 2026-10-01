//! Verifica della potatura dei campioni sul database REALE.
//!
//! Non e' un test automatico: diagnostica. Eseguire con:
//!     cargo test -p cryo_cooler_controller_lib --test db_prune -- --nocapture
//!
//! Apre il database dell'app in sola lettura, applica gli stessi tetti del
//! codice di produzione e riporta la dimensione prima/dopo. Serve a capire
//! quanto spazio si recupera e se i tetti sono ragionevoli.

use std::path::PathBuf;

fn db_path() -> Option<PathBuf> {
    let base = dirs::config_dir()?;
    let p = base.join("stargate-cryo").join("sessions.db");
    if p.exists() { Some(p) } else { None }
}

#[test]
#[ignore = "toca il database reale dell'utente"]
fn potatura_campioni() {
    let Some(path) = db_path() else {
        println!("Database non trovato: nessuna sessione registrata.");
        return;
    };

    let wal = {
        let mut w = path.clone();
        let name = w.file_name().unwrap().to_string_lossy().to_string();
        w.set_file_name(format!("{name}-wal"));
        w
    };

    let db_mb = |p: &PathBuf| -> f64 {
        p.metadata().map(|m| m.len() as f64 / 1_000_000.0).unwrap_or(0.0)
    };

    println!("Database : {}", path.display());
    println!("  db  {:.1} MB", db_mb(&path));
    println!("  wal {:.1} MB", db_mb(&wal));

    let conn = rusqlite::Connection::open(&path).expect("apertura db");
    let tot: i64 = conn
        .query_row("SELECT COUNT(*) FROM samples", [], |r| r.get(0))
        .unwrap_or(0);
    println!("\nCampioni totali: {tot}");

    println!("\nCampioni per sessione:");
    let mut stmt = conn
        .prepare("SELECT session_id, COUNT(*) n FROM samples GROUP BY session_id ORDER BY n DESC")
        .unwrap();
    let rows: Vec<(i64, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    for (sid, n) in rows.iter().take(8) {
        println!("  sessione {sid:>4}: {n:>9}");
    }

    // Applica i tetti di produzione.
    const MAX_PER_SESSION: i64 = 72_000;
    const MAX_TOTAL: i64 = 300_000;

    let mut rimossi = 0i64;
    for (sid, n) in &rows {
        if *n > MAX_PER_SESSION {
            let d = conn.execute(
                "DELETE FROM samples WHERE id IN (SELECT id FROM samples WHERE session_id = ?1 ORDER BY id ASC LIMIT ?2)",
                rusqlite::params![sid, n - MAX_PER_SESSION],
            ).unwrap_or(0);
            rimossi += d as i64;
        }
    }
    println!("\nRimossi per tetto di sessione: {rimossi}");

    let tot2: i64 = conn
        .query_row("SELECT COUNT(*) FROM samples", [], |r| r.get(0))
        .unwrap_or(0);
    if tot2 > MAX_TOTAL {
        let d = conn.execute(
            "DELETE FROM samples WHERE id IN (SELECT id FROM samples ORDER BY id ASC LIMIT ?1)",
            rusqlite::params![tot2 - MAX_TOTAL],
        ).unwrap_or(0);
        println!("Rimossi per tetto globale    : {d}");
    }

    // Ordine corretto: VACUUM poi checkpoint (vedi session_db.rs).
    conn.execute_batch("VACUUM;").unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();

    let tot3: i64 = conn
        .query_row("SELECT COUNT(*) FROM samples", [], |r| r.get(0))
        .unwrap_or(0);
    println!("\nDopo: {tot3} campioni");
    println!("  db  {:.1} MB", db_mb(&path));
    println!("  wal {:.1} MB", db_mb(&wal));
}
