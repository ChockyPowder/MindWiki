//! Command line for mindwiki-server. Run `mindwiki-server help` for usage.

use mindwiki_server::users::Users;
use mindwiki_server::{Options, pretty_fingerprint, start};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "mindwiki-server: share a mindwiki over HTTPS

USAGE:
  mindwiki-server serve [OPTIONS]            start the server
  mindwiki-server user add NAME [--read-only]  create an account (asks for a password)
  mindwiki-server user passwd NAME           change a password
  mindwiki-server user readonly NAME on|off  allow or forbid editing
  mindwiki-server user remove NAME           delete an account
  mindwiki-server user list                  show accounts

OPTIONS (all commands):
  --data DIR     where accounts, the certificate and pages are kept
                 (default: ./mindwiki-data)
OPTIONS (serve):
  --addr ADDR    address to listen on (default: 0.0.0.0:8443)
  --host NAME    a name or IP address clients will use to reach this server;
                 repeatable. Only used when the self-signed certificate is
                 first created.
  --cert FILE --key FILE
                 use your own certificate (e.g. from Let's Encrypt) instead
                 of a self-signed one
";

struct Args {
    rest: Vec<String>,
}

impl Args {
    /// Removes `--name VALUE` and returns every VALUE given.
    fn values(&mut self, name: &str) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        while let Some(i) = self.rest.iter().position(|a| a == name) {
            if i + 1 >= self.rest.len() {
                return Err(format!("{name} needs a value"));
            }
            out.push(self.rest.remove(i + 1));
            self.rest.remove(i);
        }
        Ok(out)
    }

    fn value(&mut self, name: &str) -> Result<Option<String>, String> {
        Ok(self.values(name)?.pop())
    }

    fn flag(&mut self, name: &str) -> bool {
        match self.rest.iter().position(|a| a == name) {
            Some(i) => {
                self.rest.remove(i);
                true
            }
            None => false,
        }
    }
}

/// Asks for a new password twice. When input is piped in (for scripts), the
/// password is read as one line from standard input instead.
fn ask_password() -> Result<String, String> {
    use std::io::{BufRead, IsTerminal};
    if !std::io::stdin().is_terminal() {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line).map_err(|e| e.to_string())?;
        return Ok(line.trim_end_matches(['\r', '\n']).to_string());
    }
    let a = rpassword::prompt_password("New password: ").map_err(|e| e.to_string())?;
    let b = rpassword::prompt_password("Repeat password: ").map_err(|e| e.to_string())?;
    if a != b {
        return Err("the passwords don't match".into());
    }
    Ok(a)
}

fn run() -> Result<(), String> {
    let mut args = Args { rest: std::env::args().skip(1).collect() };
    let data = PathBuf::from(args.value("--data")?.unwrap_or_else(|| "mindwiki-data".into()));
    let cmd: Vec<String> = args.rest.iter().take_while(|a| !a.starts_with("--")).cloned().collect();
    let cmd: Vec<&str> = cmd.iter().map(String::as_str).collect();

    match cmd.as_slice() {
        ["serve"] => {
            let addr: SocketAddr = args
                .value("--addr")?
                .unwrap_or_else(|| "0.0.0.0:8443".into())
                .parse()
                .map_err(|_| "--addr must look like 0.0.0.0:8443".to_string())?;
            let opts = Options {
                data: data.clone(),
                addr,
                cert: args.value("--cert")?.map(PathBuf::from),
                key: args.value("--key")?.map(PathBuf::from),
                hosts: args.values("--host")?,
            };
            if args.rest.len() > 1 {
                return Err(format!("unknown option {}", args.rest[1]));
            }
            let users = Users::new(&data).list().map_err(|e| e.to_string())?;
            let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
            rt.block_on(async move {
                let (running, task) = start(opts).await.map_err(|e| e.to_string())?;
                println!("mindwiki-server listening on https://{}", running.addr);
                println!("data folder: {}", data.display());
                if running.self_signed {
                    println!("\nUsing a self-signed certificate. Give this fingerprint to your users;");
                    println!("they enter it the first time they connect:\n");
                    println!("  {}\n", pretty_fingerprint(&running.fingerprint));
                }
                if users.is_empty() {
                    println!("No accounts yet. Create one with:  mindwiki-server user add NAME");
                }
                let handle = running.handle.clone();
                tokio::spawn(async move {
                    let _ = tokio::signal::ctrl_c().await;
                    println!("\nshutting down");
                    handle.graceful_shutdown(Some(std::time::Duration::from_secs(3)));
                });
                task.await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())
            })
        }
        ["user", "list"] => {
            let users = Users::new(&data).list().map_err(|e| e.to_string())?;
            if users.is_empty() {
                println!("no accounts");
            }
            for u in users {
                println!("{}{}", u.name, if u.read_only { "  (read-only)" } else { "" });
            }
            Ok(())
        }
        ["user", "add", name] => {
            let read_only = args.flag("--read-only");
            std::fs::create_dir_all(&data).map_err(|e| e.to_string())?;
            let password = ask_password()?;
            Users::new(&data).add(name, &password, read_only).map_err(|e| e.to_string())?;
            println!("created {name}");
            Ok(())
        }
        ["user", "passwd", name] => {
            let password = ask_password()?;
            Users::new(&data).set_password(name, &password).map_err(|e| e.to_string())?;
            println!("password changed for {name}");
            Ok(())
        }
        ["user", "readonly", name, state @ ("on" | "off")] => {
            Users::new(&data).set_read_only(name, *state == "on").map_err(|e| e.to_string())?;
            println!("{name} is now {}", if *state == "on" { "read-only" } else { "allowed to edit" });
            Ok(())
        }
        ["user", "remove", name] => {
            Users::new(&data).remove(name).map_err(|e| e.to_string())?;
            println!("removed {name}");
            Ok(())
        }
        [] | ["help"] => {
            print!("{USAGE}");
            Ok(())
        }
        _ => Err(format!("unknown command\n\n{USAGE}")),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
