//! Share existing read-only telemetry; never communicate with the controllers.
use std::{fs, path::{Path, PathBuf}};

fn snapshot(tf3: Option<crate::tf3::Reading>, tf2: Option<crate::tf2::Reading>) -> serde_json::Value {
    serde_json::json!({
        "coolant_c": tf3.map(|r| r.coolant_c),
        "ambient_c": tf3.map(|r| r.ambient_c),
        "pressure_bar": tf3.map(|r| r.pressure_bar),
        "flow_lph": tf2.map(|r| r.flow_lph),
        "tf3_age_ms": tf3.map(|r| r.sampled_at.elapsed().as_millis() as u64),
        "tf2_age_ms": tf2.map(|r| r.sampled_at.elapsed().as_millis() as u64),
    })
}

pub fn publish(tf3: Option<crate::tf3::Reading>, tf2: Option<crate::tf2::Reading>) -> std::io::Result<()> {
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") else { return Ok(()) };
    let directory = PathBuf::from(runtime);
    publish_to(&directory, tf3, tf2)
}

fn publish_to(directory: &Path, tf3: Option<crate::tf3::Reading>, tf2: Option<crate::tf2::Reading>) -> std::io::Result<()> {
    let temporary = directory.join("stargate-cryo-formula.tmp");
    fs::write(&temporary, snapshot(tf3, tf2).to_string())?;
    fs::rename(temporary, directory.join("stargate-cryo-formula.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    #[test]
    fn snapshot_only_uses_actual_readings() {
        let t = crate::tf3::Reading { coolant_c: 19.5, ambient_c: 23.0, pressure_bar: 0.25, sampled_at: Instant::now() };
        let result = snapshot(Some(t), None);
        assert_eq!(result["coolant_c"], 19.5);
        assert_eq!(result["pressure_bar"], 0.25);
        assert!(result["flow_lph"].is_null());
        assert!(snapshot(None, None)["coolant_c"].is_null());
    }

    #[test]
    fn snapshot_is_written_atomically() {
        let dir = std::env::temp_dir().join(format!("formula-bridge-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = crate::tf2::Reading { flow_lph: 210.0, sampled_at: Instant::now() };
        publish_to(&dir, None, Some(f)).unwrap();
        let data = std::fs::read_to_string(dir.join("stargate-cryo-formula.json")).unwrap();
        let result: serde_json::Value = serde_json::from_str(&data).unwrap();
        assert_eq!(result["flow_lph"], 210.0);
        assert!(result["coolant_c"].is_null());
        assert!(!dir.join("stargate-cryo-formula.tmp").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
