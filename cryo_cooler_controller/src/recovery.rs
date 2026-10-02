//! Separate supervisor: no controller access and no forced termination.
use std::{path::PathBuf, io::Write, sync::atomic::{AtomicBool, Ordering}};

fn state_path() -> Option<PathBuf> { std::env::var_os("CRYO_RECOVERY_STATE").map(PathBuf::from) }
fn quit_path() -> Option<PathBuf> { state_path().map(|p|p.with_extension("quit")) }

pub fn record_enabled(enabled: bool) {
    if let Some(p)=state_path() {
        let result=if enabled {std::fs::write(p,b"enabled")} else {
            match std::fs::remove_file(p) {Err(e) if e.kind()!=std::io::ErrorKind::NotFound=>Err(e),_=>Ok(())}
        };
        if let Err(e)=result {log(&format!("Cannot record cooling intent: {e}"));}
    }
}
pub fn intentional_quit() {
    if let Some(p)=quit_path() { if let Err(e)=std::fs::write(p,b"quit") {log(&format!("Cannot record intentional exit: {e}"));} }
}
pub fn resume_once() -> bool {
    static CONSUMED:AtomicBool=AtomicBool::new(false);
    std::env::var_os("CRYO_RECOVER").is_some()
        && cooling_requested()
        && !CONSUMED.swap(true,Ordering::SeqCst)
}
pub fn cooling_requested()->bool {
    state_path().and_then(|p|std::fs::read(p).ok()).is_some_and(|bytes|bytes==b"enabled")
}
pub fn log(message:&str) {
    let base=std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("stargate-cryo");
    let _=std::fs::create_dir_all(&base);
    let path=base.join("recovery.log");
    if std::fs::metadata(&path).is_ok_and(|m|m.len()>1_000_000) {let _=std::fs::rename(&path,base.join("recovery.previous.log"));}
    if let Ok(mut f)=std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _=writeln!(f,"{} pid={} {message}",chrono::Utc::now().to_rfc3339(),std::process::id());
    }
}
fn retry(code:Option<i32>, intentional:bool)->bool { !intentional && code!=Some(1) && code!=Some(2) }

/// Returns true in the supervisor; child and non-Windows proceed to the UI.
pub fn supervise()->bool {
    #[cfg(target_os="windows")]
    {
        if std::env::var_os("CRYO_SUPERVISED_CHILD").is_some() {
            if std::env::args().any(|a|a=="--recovery-self-test") {
                if resume_once() {
                    log("SELFTEST: enabled intent recovered after process failure; controller not accessed");
                    intentional_quit();
                    std::process::exit(0);
                }
                record_enabled(true);
                log("SELFTEST: injecting exit 70 without controller access");
                std::process::exit(70);
            }
            return false;
        }
        use std::os::windows::process::CommandExt;
        let Ok(exe)=std::env::current_exe() else {log("Cannot locate executable; supervisor unavailable");return false;};
        let state=std::env::temp_dir().join(format!("stargate-cryo-recovery-{}.state",std::process::id()));
        let quit=state.with_extension("quit");
        // A reused process ID must not inherit intent from an older session.
        let _=std::fs::remove_file(&state);
        let _=std::fs::remove_file(&quit);
        let mut attempt=0u32;
        loop {
            let mut command=std::process::Command::new(&exe);
            command.args(std::env::args_os().skip(1)).env_remove("CRYO_RECOVER").env("CRYO_SUPERVISED_CHILD","1")
                .env("CRYO_RECOVERY_STATE",&state).creation_flags(0x08000000);
            if attempt>0 {command.env("CRYO_RECOVER","1");}
            let started=std::time::Instant::now();
            let result=command.spawn().and_then(|mut child|child.wait());
            let code=match result {Ok(status)=>status.code(),Err(e)=>{log(&format!("Launch/wait failed: {e}"));None}};
            log(&format!("Dashboard exited: code={code:?}, intentional={}, attempt={attempt}",quit.exists()));
            if !retry(code,quit.exists()) {break;}
            if started.elapsed()>std::time::Duration::from_secs(120) {attempt=0;}
            attempt=attempt.saturating_add(1);
            let pause=(2u64.saturating_pow(attempt.min(5))).min(30);
            log(&format!("Restart in {pause}s; resume Cryo={}",state.exists()));
            std::thread::sleep(std::time::Duration::from_secs(pause));
        }
        let _=std::fs::remove_file(state);
        let _=std::fs::remove_file(quit);
        return true;
    }
    #[cfg(not(target_os="windows"))]
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn crashes_and_unexpected_clean_exits_restart() {for code in [None,Some(0),Some(70),Some(-1073741819)] {assert!(retry(code,false));}}
    #[test] fn user_quit_and_duplicate_or_invalid_start_do_not_restart() {assert!(!retry(Some(0),true));assert!(!retry(None,true));assert!(!retry(Some(1),false));assert!(!retry(Some(2),false));}
}
