//! Session database — SQLite via rusqlite.
//!
//! Cross-platform using XDG directories:
//! Windows: %APPDATA%\StargateLabsCryo\sessions.db
//! Linux: ~/.local/share/stargate-cryo/sessions.db
//!
//! Permette di confrontare sessioni passate, calcolare trend OC nel tempo,
//! e fare replay dei grafici.

use rusqlite::{Connection, params};
use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};

// ── Schema SQL ────────────────────────────────────────────────────────────────

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at  TEXT    NOT NULL,
    ended_at    TEXT,
    label       TEXT    NOT NULL DEFAULT '',
    oc_score    INTEGER NOT NULL DEFAULT 0,
    min_tec     REAL,
    avg_cop     REAL,
    min_margin  REAL,
    ocp_events  INTEGER NOT NULL DEFAULT 0,
    samples     INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS samples (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id     INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    ts             TEXT    NOT NULL,
    tec_temp       REAL    NOT NULL,
    dew_point      REAL    NOT NULL,
    cpu_temp       REAL,
    power_w        REAL    NOT NULL,
    humidity       REAL    NOT NULL,
    cop            REAL,
    margin         REAL    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_samples_session ON samples(session_id);
CREATE INDEX IF NOT EXISTS idx_samples_ts      ON samples(ts);
"#;

// ── Tipi pubblici ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct SessionSummary {
    pub id:          i64,
    pub started_at:  String,
    pub ended_at:    Option<String>,
    pub label:       String,
    pub oc_score:    u32,
    pub min_tec:     Option<f32>,
    pub avg_cop:     Option<f32>,
    pub min_margin:  Option<f32>,
    pub ocp_events:  u32,
    pub samples:     u64,
}

#[derive(Debug, Clone)]
pub struct SampleRow {
    pub ts:       DateTime<Utc>,
    pub tec_temp: f32,
    pub dew_point: f32,
    pub cpu_temp:  Option<f32>,
    pub power_w:   f32,
    pub humidity:  f32,
    pub cop:       Option<f32>,
    pub margin:    f32,
}

// ── SessionDb ─────────────────────────────────────────────────────────────────

pub struct SessionDb {
    conn:            Option<Connection>,
    pub session_id:  Option<i64>,
    sample_buf:      Vec<SampleRow>,   // buffer — flush ogni 60 sample
    flush_interval:  usize,
    /// Flush trascorsi dall'ultima potatura dei campioni. Serve a non
    /// interrogare il DB a ogni flush (il flush avviene piu' volte al secondo).
    flushes_since_prune: u32,
}


#[allow(dead_code)]
impl SessionDb {
    pub fn open() -> Self {
        let path = db_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let conn = Connection::open(&path).ok();
        if let Some(ref c) = conn {
            let _ = c.execute_batch(SCHEMA);
            // WAL mode per performance su write frequenti
            let _ = c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;");
        }

        let db = SessionDb {
            conn, session_id: None, sample_buf: Vec::new(),
            flush_interval: 60, flushes_since_prune: 0,
        };
        // NOTA: qui NON si pota. `prune_old_sessions` finisce in `VACUUM`,
        // che riscrive l'intero file del database e vuole ~2x lo spazio
        // libero. `open()` gira dentro `RunningState::new`, che gira dentro
        // `Message::Open` sul thread dell'event loop iced: farlo a ogni avvio
        // congelava la finestra per secondi — un tempo che cresce con il
        // database — per una condizione che quasi mai si verifica. La potatura
        // e' ora a richiesta: vedi `pota_se_necessario`.
        db
    }

    /// Rimuove sessioni oltre le `keep` più recenti, inclusi i relativi campioni.
    /// Previene crescita illimitata del database su dischi.
    pub fn prune_old_sessions(&self, keep: i64) {
        let Some(c) = self.conn.as_ref() else { return; };
        let _ = c.execute(
            "DELETE FROM samples WHERE session_id NOT IN \
             (SELECT id FROM sessions ORDER BY id DESC LIMIT ?1)",
            params![keep],
        );
        let _ = c.execute(
            "DELETE FROM sessions WHERE id NOT IN \
             (SELECT id FROM sessions ORDER BY id DESC LIMIT ?1)",
            params![keep],
        );
        // Restituisce lo spazio al filesystem (no WAL, best-effort).
        // Ordine importante: VACUUM *prima* del checkpoint.
        //
        // Il checkpoint tronca il WAL, ma il VACUUM successivo riscrive il
        // database da capo e ri-gonfia il WAL. Facendo il contrario, il
        // checkpoint non serve a nulla: misurato su questo database, il WAL
        // restava a 45 MB dopo "checkpoint + VACUUM" nello stesso ordine.
        let _ = c.execute_batch("VACUUM;");
        let _ = c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }

    /// Quante sessioni sono salvate nel database.
    ///
    /// Restituisce 0 se il database non e' aperto: in quel caso non c'e'
    /// niente da contare, e `pota_se_necessario` non deve fare lavoro.
    pub fn conta_sessioni(&self) -> i64 {
        let Some(c) = self.conn.as_ref() else { return 0; };
        c.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap_or(0)
    }

    /// Sessioni conservate oltre le quali la potatura serve davvero.
    const MAX_SESSIONI_CONSERVATE: i64 = 50;

    /// Potatura delle sessioni vecchie **solo quando il numero le supera**.
    ///
    /// `prune_old_sessions` finisce sempre in `VACUUM`, che riscrive tutto il
    /// file del database: chiamarla a ogni avvio congelava la finestra per
    /// secondi (crescenti con il database) sul thread dell'event loop. Qui la
    /// si interroga prima con un `COUNT(*)` — una riga, senza scritture — e
    /// solo se il tetto e' davvero superato si paga il VACUUM.
    pub fn pota_se_necessario(&mut self) {
        if self.conta_sessioni() > Self::MAX_SESSIONI_CONSERVATE {
            self.prune_old_sessions(Self::MAX_SESSIONI_CONSERVATE);
        }
    }

    /// Crea una nuova sessione nel DB e ritorna il session_id.
    pub fn start_session(&mut self, label: &str) -> Option<i64> {
        let c = self.conn.as_ref()?;
        let ts = Utc::now().to_rfc3339();
        c.execute(
            "INSERT INTO sessions (started_at, label) VALUES (?1, ?2)",
            params![ts, label],
        ).ok()?;
        let id = c.last_insert_rowid();
        self.session_id = Some(id);
        Some(id)
    }

    /// Chiude la sessione corrente aggiornando le statistiche finali.
    pub fn close_session(&mut self,
        oc_score:   u32,
        min_tec:    f32,
        avg_cop:    f32,
        min_margin: f32,
        ocp_events: u32,
        samples:    u64,
    ) {
        self.flush();
        let (Some(c), Some(sid)) = (self.conn.as_ref(), self.session_id) else { return; };
        let ts = Utc::now().to_rfc3339();
        let _ = c.execute(
            "UPDATE sessions SET ended_at=?1, oc_score=?2, min_tec=?3,
             avg_cop=?4, min_margin=?5, ocp_events=?6, samples=?7 WHERE id=?8",
            params![ts, oc_score, min_tec, avg_cop, min_margin, ocp_events, samples, sid],
        );
        self.session_id = None;
    }

    /// Aggiunge un sample al buffer (flush automatico ogni N sample).
    pub fn push_sample(&mut self, s: SampleRow) {
        self.sample_buf.push(s);
        if self.sample_buf.len() >= self.flush_interval {
            self.flush();
        }
    }

    /// Tetto massimo di campioni per singola sessione.
    ///
    /// 72.000 campioni = 10 ore a 2 Hz: abbondanti per analizzare una
    /// sessione, e oltre non serve a nulla.
    const MAX_SAMPLES_PER_SESSION: i64 = 72_000;

    /// Tetto massimo di campioni su TUTTO il database.
    ///
    /// **Perche' serve, oltre al tetto per sessione.** `prune_old_sessions`
    /// conserva 50 sessioni *con tutti i loro campioni*: con 50 sessioni da
    /// 72.000 campioni a ~149 byte l'uno, il database arriverebbe a ~540 MB.
    /// Il tetto globale e' la rete di sicurezza reale: quando si supera,
    /// vengono rimossi i campioni piu' vecchi di tutte le sessioni.
    const MAX_SAMPLES_TOTAL: i64 = 300_000;

    /// Ogni quanti flush controllare (ed eventualmente potare) i campioni.
    /// 60 sample = 30 s a 2 Hz, quindi ~5 minuti.
    const PRUNE_EVERY_FLUSHES: u32 = 10;

    /// Scrive tutti i sample nel buffer sul DB.
    pub fn flush(&mut self) {
        if self.sample_buf.is_empty() { return; }
        let (Some(c), Some(sid)) = (self.conn.as_ref(), self.session_id) else {
            self.sample_buf.clear(); return;
        };
        let Ok(mut stmt) = c.prepare_cached(
            "INSERT INTO samples (session_id,ts,tec_temp,dew_point,cpu_temp,power_w,humidity,cop,margin)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)"
        ) else { return; };

        for s in self.sample_buf.drain(..) {
            let _ = stmt.execute(params![
                sid,
                s.ts.to_rfc3339(),
                s.tec_temp, s.dew_point, s.cpu_temp,
                s.power_w,  s.humidity,  s.cop,
                s.margin,
            ]);
        }
        drop(stmt);

        // Potatura dei campioni eccedenti per questa sessione.
        //
        // Non gira a ogni flush: il flush avviene piu' volte al secondo e una
        // DELETE con subquery a ogni giro costerebbe piu' del salvataggio.
        // Si esegue ogni ~5 minuti, quando il tetto e' stato superato.
        self.flushes_since_prune += 1;
        if self.flushes_since_prune >= Self::PRUNE_EVERY_FLUSHES {
            self.flushes_since_prune = 0;
            self.prune_session_samples();
        }
    }

    /// Tiene la sessione corrente entro il tetto di campioni.
    ///
    /// elimina i piu' vecchi in eccesso, e poi compatta il WAL: senza il
    /// checkpoint il file WAL cresce anche dopo le DELETE e la dimensione
    /// su disco non torna indietro da sola.
    fn prune_session_samples(&mut self) {
        let Some(c) = self.conn.as_ref() else { return; };
        let Some(sid) = self.session_id else { return; };

        let n: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM samples WHERE session_id = ?1",
                params![sid],
                |r| r.get(0),
            )
            .unwrap_or(0);

        if n > Self::MAX_SAMPLES_PER_SESSION {
            let eccedenti = n - Self::MAX_SAMPLES_PER_SESSION;
            if let Ok(n_del) = c.execute(
                "DELETE FROM samples WHERE id IN (
                     SELECT id FROM samples WHERE session_id = ?1
                     ORDER BY id ASC LIMIT ?2)",
                params![sid, eccedenti],
            ) {
                let _ = n_del;
            }
        }

        // Tetto GLOBALE: rete di sicurezza quando le 50 sessioni conservate
        // hanno tutte molti campioni. Si rimuovono i piu' vecchi in assoluto.
        let totale: i64 = c
            .query_row("SELECT COUNT(*) FROM samples", [], |r| r.get(0))
            .unwrap_or(0);
        if totale > Self::MAX_SAMPLES_TOTAL {
            let eccedenti = totale - Self::MAX_SAMPLES_TOTAL;
            let _ = c.execute(
                "DELETE FROM samples WHERE id IN (
                     SELECT id FROM samples ORDER BY id ASC LIMIT ?1)",
                params![eccedenti],
            );
        }

        // WAL compattato solo quando si e' davvero scritto qualcosa: il
        // checkpoint e' costoso, farlo a vuoto spreca I/O.
        if n > 0 {
            let _ = c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }
    }

    /// Ritorna le ultime N sessioni, ordinate dal più recente.
    pub fn recent_sessions(&self, limit: usize) -> Vec<SessionSummary> {
        let Some(c) = self.conn.as_ref() else { return Vec::new(); };
        let Ok(mut stmt) = c.prepare(
            "SELECT id,started_at,ended_at,label,oc_score,min_tec,avg_cop,
             min_margin,ocp_events,samples FROM sessions
             ORDER BY started_at DESC LIMIT ?1"
        ) else { return Vec::new(); };

        stmt.query_map(params![limit as i64], |row| {
            Ok(SessionSummary {
                id:          row.get(0)?,
                started_at:  row.get(1)?,
                ended_at:    row.get(2)?,
                label:       row.get(3)?,
                oc_score:    row.get::<_,i64>(4)? as u32,
                min_tec:     row.get(5)?,
                avg_cop:     row.get(6)?,
                min_margin:  row.get(7)?,
                ocp_events:  row.get::<_,i64>(8)? as u32,
                samples:     row.get::<_,i64>(9)? as u64,
            })
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// Carica i sample di una sessione (usato per replay).
    pub fn load_samples(&self, session_id: i64, limit: usize) -> Vec<SampleRow> {
        let Some(c) = self.conn.as_ref() else { return Vec::new(); };
        let Ok(mut stmt) = c.prepare(
            "SELECT ts,tec_temp,dew_point,cpu_temp,power_w,humidity,cop,margin
             FROM samples WHERE session_id=?1 ORDER BY ts ASC LIMIT ?2"
        ) else { return Vec::new(); };

        stmt.query_map(params![session_id, limit as i64], |row| {
            let ts_str: String = row.get(0)?;
            let ts = ts_str.parse::<DateTime<Utc>>()
                .unwrap_or(DateTime::<Utc>::MIN_UTC);
            Ok(SampleRow {
                ts, tec_temp: row.get(1)?, dew_point: row.get(2)?,
                cpu_temp: row.get(3)?, power_w: row.get(4)?,
                humidity: row.get(5)?, cop: row.get(6)?, margin: row.get(7)?,
            })
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// Best OC score di sempre
    pub fn all_time_best(&self) -> Option<SessionSummary> {
        self.recent_sessions(100).into_iter().max_by_key(|s| s.oc_score)
    }

    pub fn is_open(&self) -> bool { self.conn.is_some() }
}

fn db_path() -> std::path::PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("stargate-cryo")
        .join("sessions.db")
}
