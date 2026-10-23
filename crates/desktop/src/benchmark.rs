//! Explicit, local benchmark transport. Ordinary app launches never open this channel.
use anyhow::{Context as _, bail};
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, Write},
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::mpsc;

pub struct EventLog {
    file: Mutex<File>,
    origin: Instant,
}

impl EventLog {
    pub fn emit(&self, event: &str, data: Value) {
        let value = json!({"event":event,"elapsed_s":self.origin.elapsed().as_secs_f64(),
            "at":chrono::Utc::now().to_rfc3339(),"data":data});
        // A failed evidence channel must fail the benchmark, not silently lose events.
        let mut file = self.file.lock().expect("benchmark event writer poisoned");
        writeln!(file, "{value}").expect("could not write benchmark evidence");
        file.flush().expect("could not flush benchmark evidence");
    }
}

pub struct Session {
    pub log: Arc<EventLog>,
    pub commands: mpsc::UnboundedReceiver<Value>,
}

impl Session {
    pub fn start(directory: &Path) -> anyhow::Result<Self> {
        let config: Value =
            serde_json::from_slice(&std::fs::read(directory.join("benchmark.json"))?)?;
        if config["protocol"] != 1 {
            bail!("unsupported benchmark protocol");
        }
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("app-events.jsonl"))
            .context("could not create benchmark evidence")?;
        let log = Arc::new(EventLog {
            file: Mutex::new(file),
            origin: Instant::now(),
        });
        log.emit(
            "boot_waiting",
            json!({"pid":std::process::id(),"protocol":1}),
        );
        // Let the collector attach before application initialization begins.
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        let command: Value = serde_json::from_str(&line)?;
        if command["command"] != "start" {
            bail!("benchmark requires a start command on stdin");
        }
        log.emit("initializing", json!({}));
        let (sender, commands) = mpsc::unbounded_channel();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let command = line
                    .ok()
                    .and_then(|line| serde_json::from_str::<Value>(&line).ok())
                    .unwrap_or_else(|| json!({"command":"invalid"}));
                if sender.send(command).is_err() {
                    return;
                }
            }
            let _ = sender.send(json!({"command":"quit"}));
        });
        Ok(Self { log, commands })
    }
}
