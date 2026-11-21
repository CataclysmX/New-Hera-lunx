//! `hsm01ctl` – the open API of new hera-nux (docs/app/api.md) from the shell.
//!
//! ```sh
//! hsm01ctl alert "Compilation finie" "0 erreur" --icon dialog-ok --led 2 --wait
//! make && hsm01ctl alert "Build OK" || hsm01ctl alert "Build KO" --icon dialog-error --high
//! hsm01ctl send '{"type": "alert", "title": "Brut"}'
//! ```

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value, json};

const USAGE: &str = "\
usage:
  hsm01ctl alert <title> [text] [--icon NAME|PATH] [--image PATH] [--round] [--app NAME]
                 [--duration SECONDS] [--led 1-4] [--high] [--no-wake] [--wait]
  hsm01ctl send '<json request>' [--wait SECONDS]
  hsm01ctl events [--grab INPUT,INPUT...]

alert --wait: waits until the alert closes and prints why (timeout, dismissed, standby).
send: prints every line received within the wait (0.3 s by default).
events: prints every event (knobs, buttons… on any page) until Ctrl+C; --grab takes these
        inputs (knob1-4, button1-4, power) from the app meanwhile.";

fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("HSM01_API_SOCKET") {
        return p.into();
    }
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("hsm01")
        .join("api.sock")
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("alert") => alert(&args[1..]),
        Some("send") => send(&args[1..]),
        Some("events") => events(&args[1..]),
        _ => Err(USAGE.into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("hsm01ctl: {e}");
            ExitCode::FAILURE
        }
    }
}

fn connect() -> Result<(UnixStream, std::io::Lines<BufReader<UnixStream>>), String> {
    let path = socket_path();
    let s = UnixStream::connect(&path)
        .map_err(|e| format!("{}: {e} (is new hera-nux running?)", path.display()))?;
    let lines = BufReader::new(s.try_clone().map_err(|e| e.to_string())?).lines();
    let mut c = (s, lines);
    request(&mut c, &json!({"type": "hello", "name": "hsm01ctl"}))?;
    Ok(c)
}

/// Send `req` and return the next line received (its answer).
fn request(
    (s, lines): &mut (UnixStream, std::io::Lines<BufReader<UnixStream>>),
    req: &Value,
) -> Result<Value, String> {
    writeln!(s, "{req}").map_err(|e| e.to_string())?;
    let line = lines
        .next()
        .ok_or("connection closed")?
        .map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    if v["type"] == "error" {
        return Err(v["message"].as_str().unwrap_or("error").to_owned());
    }
    Ok(v)
}

fn alert(args: &[String]) -> Result<(), String> {
    let mut req = json!({"type": "alert", "req": 1});
    let (mut texts, mut wait) = (Vec::new(), false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{name}: value missing\n{USAGE}"))
        };
        match a.as_str() {
            "--icon" => req["icon"] = json!(value("--icon")?),
            "--image" => req["image"] = json!(value("--image")?),
            "--app" => req["app"] = json!(value("--app")?),
            "--duration" => {
                let d: f64 = value("--duration")?.parse().map_err(|_| "--duration: seconds")?;
                req["duration"] = json!(d);
            }
            "--led" => {
                let l: u8 = value("--led")?.parse().map_err(|_| "--led: 1 to 4")?;
                req["led"] = json!(l);
            }
            "--round" => req["round"] = json!(true),
            "--high" => req["priority"] = json!("high"),
            "--no-wake" => req["wake"] = json!(false),
            "--wait" => wait = true,
            s if s.starts_with("--") => return Err(format!("unknown option {s}\n{USAGE}")),
            s => texts.push(s.to_owned()),
        }
    }
    match texts.as_slice() {
        [title] => req["title"] = json!(title),
        [title, text] => {
            req["title"] = json!(title);
            req["text"] = json!(text);
        }
        _ => return Err(USAGE.into()),
    }
    let mut c = connect()?;
    let ok = request(&mut c, &req)?;
    let id = ok["id"].clone();
    if wait {
        for line in c.1.by_ref() {
            let v: Value = serde_json::from_str(&line.map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            if v["event"] == "alert_closed" && v["id"] == id {
                println!("{}", v["reason"].as_str().unwrap_or("?"));
                return Ok(());
            }
        }
        return Err("connection closed".into());
    }
    Ok(())
}

fn events(args: &[String]) -> Result<(), String> {
    let grab: Vec<&str> = match args {
        [] => Vec::new(),
        [flag, list] if flag == "--grab" => list.split(',').map(str::trim).collect(),
        _ => return Err(USAGE.into()),
    };
    let mut c = connect()?;
    if !grab.is_empty() {
        request(&mut c, &json!({"type": "grab", "inputs": grab, "req": 1}))?;
    }
    for line in c.1 {
        println!("{}", line.map_err(|e| e.to_string())?);
    }
    Err("connection closed".into())
}

fn send(args: &[String]) -> Result<(), String> {
    let (json_text, wait) = match args {
        [j] => (j, 0.3),
        [j, flag, s] if flag == "--wait" => (j, s.parse().map_err(|_| "--wait: seconds")?),
        _ => return Err(USAGE.into()),
    };
    let req: Value = serde_json::from_str(json_text).map_err(|e| format!("JSON: {e}"))?;
    let (mut s, lines) = connect()?;
    writeln!(s, "{req}").map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs_f64(wait)))
        .map_err(|e| e.to_string())?;
    for line in lines {
        match line {
            Ok(l) => println!("{l}"),
            Err(_) => break, // timeout
        }
    }
    Ok(())
}
