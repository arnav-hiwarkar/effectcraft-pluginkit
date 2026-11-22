use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{Value, json};

use super::{Messages, USAGE, assemble, option, plugin_dir, positional};

const DEFAULT_BRIDGE: &str = "9877";
const POLL: Duration = Duration::from_secs(1);
const ANSWER_WAIT: Duration = Duration::from_secs(60);

fn newest_change(dir: &Path) -> (usize, SystemTime) {
    let mut count = 0;
    let mut newest = SystemTime::UNIX_EPOCH;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(folder) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            let name = entry.file_name();
            if kind.is_dir() {
                if name != "target" && name != ".git" && name != "frames" {
                    stack.push(entry.path());
                }
            } else if name != "Cargo.lock"
                && let Ok(modified) = entry.metadata().and_then(|m| m.modified())
            {
                count += 1;
                newest = newest.max(modified);
            }
        }
    }
    (count, newest)
}

enum Failure {
    NoApp(String),
    Refused(String),
}

struct Control {
    address: String,
    stream: Option<BufReader<TcpStream>>,
    next: u64,
}

impl Control {
    fn execute(&mut self, command: &str, params: Value) -> Result<Value, Failure> {
        let answered = self.exchange(command, params);
        if matches!(answered, Err(Failure::NoApp(_))) {
            self.stream = None;
        }
        answered
    }

    fn exchange(&mut self, command: &str, params: Value) -> Result<Value, Failure> {
        let lost = |e: std::io::Error| Failure::NoApp(e.to_string());
        if self.stream.is_none() {
            let stream = TcpStream::connect(&self.address).map_err(lost)?;
            stream.set_read_timeout(Some(ANSWER_WAIT)).map_err(lost)?;
            self.stream = Some(BufReader::new(stream));
        }
        let stream = self.stream.as_mut().ok_or_else(|| Failure::NoApp("not connected".into()))?;
        self.next += 1;
        let request = json!({ "id": self.next, "method": "engine.execute", "params": { "command": command, "params": params } });
        writeln!(stream.get_mut(), "{request}").map_err(lost)?;
        let mut line = String::new();
        if stream.read_line(&mut line).map_err(lost)? == 0 {
            return Err(Failure::NoApp("the app closed the connection".into()));
        }
        let reply: Value = serde_json::from_str(&line).map_err(|e| Failure::Refused(format!("the app sent a line that is not JSON: {e}")))?;
        match reply["ok"].as_bool() {
            Some(true) => Ok(reply["result"].clone()),
            _ => Err(Failure::Refused(reply["error"].as_str().unwrap_or("no reason given").to_string())),
        }
    }
}

struct Built {
    bundle: PathBuf,
    id: String,
    seconds: f64,
}

struct Dev {
    control: Control,
    port: String,
    waiting: Option<Built>,
    told: bool,
    running: Option<String>,
    after: u64,
}

impl Dev {
    fn new(port: &str) -> Dev {
        Dev {
            control: Control { address: format!("127.0.0.1:{port}"), stream: None, next: 0 },
            port: port.into(),
            waiting: None,
            told: false,
            running: None,
            after: 0,
        }
    }

    fn reload(&mut self, out: &mut impl Write) {
        let Some(built) = &self.waiting else { return };
        match self.control.execute("plugin.load", json!({ "path": built.bundle })) {
            Ok(_) => {
                let _ = writeln!(out, "reloaded {}: built in {:.1} s", built.id, built.seconds);
                (self.running, self.after, self.told, self.waiting) = (Some(built.id.clone()), 0, false, None);
            }
            Err(Failure::Refused(why)) => {
                let _ = writeln!(out, "the app did not load {}: {why}", built.id);
                self.waiting = None;
            }
            Err(Failure::NoApp(why)) if !self.told => {
                let _ = writeln!(out, "waiting for the app on port {}: {why}. Start it with `cargo run -p effectcraft -- --control {}`", self.port, self.port);
                self.told = true;
            }
            Err(Failure::NoApp(_)) => {}
        }
    }

    fn print_log(&mut self, out: &mut impl Write) {
        let Some(id) = &self.running else { return };
        let Ok(lines) = self.control.execute("plugin.log", json!({ "id": id, "after": self.after })) else { return };
        for line in lines.as_array().into_iter().flatten() {
            self.after = self.after.max(line["seq"].as_u64().unwrap_or_default());
            let _ = writeln!(out, "[{}] {}", line["level"].as_str().unwrap_or("info"), line["message"].as_str().unwrap_or_default());
        }
    }
}

pub(super) fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let dir = plugin_dir(root, positional(args, &["--bridge"]).ok_or(USAGE)?);
    let port = option(args, "--bridge").unwrap_or(DEFAULT_BRIDGE);
    let out = root.join("plugins/dist");
    println!("watching {} and loading it into the app on port {port}; Ctrl-C stops", dir.display());
    let mut dev = Dev::new(port);
    let mut seen = None;
    let mut stdout = std::io::stdout();
    loop {
        let stamp = newest_change(&dir);
        if seen != Some(stamp) {
            seen = Some(stamp);
            let started = Instant::now();
            match assemble(&dir, &out, false, Messages::Short) {
                Ok((bundle, manifest)) => {
                    let id = manifest["id"].as_str().unwrap_or_default().to_string();
                    dev.waiting = Some(Built { bundle, id, seconds: started.elapsed().as_secs_f64() });
                }
                Err(error) => println!("not reloaded: {error}"),
            }
        }
        dev.reload(&mut stdout);
        dev.print_log(&mut stdout);
        std::thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn writing_a_lock_file_or_a_stored_frame_does_not_count_as_a_source_change() {
        let work = tempfile::tempdir().unwrap();
        super::super::tests::folder(work.path());
        let before = newest_change(work.path());
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(work.path().join("Cargo.lock"), "version = 4").unwrap();
        std::fs::create_dir_all(work.path().join("tests/frames")).unwrap();
        std::fs::write(work.path().join("tests/frames/a.png"), "png").unwrap();
        assert_eq!(newest_change(work.path()), before);
        std::fs::write(work.path().join("assets/b.txt"), "changed").unwrap();
        assert_ne!(newest_change(work.path()), before);
    }

    fn serve(replies: Vec<Value>) -> (String, std::thread::JoinHandle<Vec<Value>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port().to_string();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut seen = Vec::new();
            for reply in replies {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                writeln!(writer, "{}", json!({ "id": request["id"], "ok": reply.get("error").is_none(), "result": reply, "error": reply.get("error") }))
                    .unwrap();
                seen.push(request["params"].clone());
            }
            seen
        });
        (port, server)
    }

    #[test]
    fn a_built_plugin_is_loaded_into_the_app_and_its_new_log_lines_are_printed_once() {
        let panic = json!([{ "seq": 4, "level": "error", "message": "the plugin panicked at src/lib.rs:12:9: index out of bounds" }]);
        let (port, server) = serve(vec![json!({ "id": "com.example.tint" }), panic, json!([])]);
        let mut dev = Dev::new(&port);
        dev.waiting = Some(Built { bundle: PathBuf::from("/dist/com.example.tint"), id: "com.example.tint".into(), seconds: 1.25 });
        let mut out = Vec::new();
        dev.reload(&mut out);
        dev.print_log(&mut out);
        dev.print_log(&mut out);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "reloaded com.example.tint: built in 1.2 s\n[error] the plugin panicked at src/lib.rs:12:9: index out of bounds\n"
        );
        let seen = server.join().unwrap();
        assert_eq!(seen[0], json!({ "command": "plugin.load", "params": { "path": "/dist/com.example.tint" } }));
        assert_eq!(seen[2], json!({ "command": "plugin.log", "params": { "id": "com.example.tint", "after": 4 } }));
    }

    #[test]
    fn without_an_app_the_reload_waits_and_says_how_to_start_the_app_once() {
        let unused = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = unused.local_addr().unwrap().port().to_string();
        drop(unused);
        let mut dev = Dev::new(&port);
        dev.waiting = Some(Built { bundle: PathBuf::from("/dist/x"), id: "com.example.x".into(), seconds: 1.0 });
        let mut out = Vec::new();
        dev.reload(&mut out);
        dev.reload(&mut out);
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(text.ends_with(&format!("Start it with `cargo run -p effectcraft -- --control {port}`\n")), "{text}");
        assert!(dev.waiting.is_some());
    }

    #[test]
    fn a_plugin_the_app_refuses_is_reported_and_not_retried_until_the_next_change() {
        let (port, server) = serve(vec![json!({ "error": "plugin.json: unknown permission `clipbard`" })]);
        let mut dev = Dev::new(&port);
        dev.waiting = Some(Built { bundle: PathBuf::from("/dist/x"), id: "com.example.x".into(), seconds: 1.0 });
        let mut out = Vec::new();
        dev.reload(&mut out);
        dev.reload(&mut out);
        assert_eq!(String::from_utf8(out).unwrap(), "the app did not load com.example.x: plugin.json: unknown permission `clipbard`\n");
        server.join().unwrap();
    }
}
