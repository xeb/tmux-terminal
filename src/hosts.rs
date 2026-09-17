//! Request-scoped host routing. Only registry entries can select a transport.
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    io,
    process::{Output, Stdio},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::io::AsyncWriteExt;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Host {
    pub id: String,
    #[serde(skip_serializing)]
    pub ssh: Option<String>,
}

static HOSTS: OnceLock<Vec<Host>> = OnceLock::new();
pub fn registry() -> &'static [Host] {
    HOSTS.get_or_init(|| {
        let hosts: Vec<Host> = match std::env::var("TMUX_HOSTS") {
            Ok(value) => {
                serde_json::from_str(&value).expect("TMUX_HOSTS must be a JSON array of {id, ssh}")
            }
            Err(_) => vec![
                Host {
                    id: "not-invented-here".into(),
                    ssh: None,
                },
                Host {
                    id: "vade".into(),
                    ssh: Some("vade".into()),
                },
            ],
        };
        assert!(
            !hosts.is_empty() && hosts[0].ssh.is_none(),
            "first host must be local"
        );
        let mut ids = std::collections::HashSet::new();
        for h in &hosts {
            assert!(
                !h.id.is_empty()
                    && h.id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c)),
                "invalid host id"
            );
            assert!(ids.insert(h.id.clone()), "duplicate host id");
            if let Some(alias) = &h.ssh {
                assert!(
                    !alias.is_empty() && !alias.starts_with('-') && !alias.contains(['\n', '\0']),
                    "invalid SSH alias"
                );
            }
        }
        hosts
    })
}

#[derive(Clone)]
struct Context {
    host: Host,
    failure: Arc<Mutex<Option<String>>>,
}
tokio::task_local! { static CONTEXT: Context; }
pub fn current() -> Host {
    CONTEXT
        .try_with(|c| c.host.clone())
        .unwrap_or_else(|_| registry()[0].clone())
}
pub fn remote() -> bool {
    current().ssh.is_some()
}
pub fn key(target: &str) -> String {
    format!("{}::{target}", current().id)
}

pub fn spawn(future: impl std::future::Future<Output = ()> + Send + 'static) {
    let context = Context {
        host: current(),
        failure: Arc::default(),
    };
    tokio::spawn(CONTEXT.scope(context, future));
}

pub async fn list() -> Json<serde_json::Value> {
    Json(serde_json::json!({"default_host": registry()[0].id, "hosts": registry()}))
}

pub async fn route(request: Request, next: Next) -> Response {
    // Query is used for previews/uploads; the header handles JSON API actions.
    let query_host = request
        .uri()
        .query()
        .and_then(|q| q.split('&').find_map(|pair| pair.strip_prefix("host=")));
    let header_host = request
        .headers()
        .get("x-tmux-host")
        .and_then(|v| v.to_str().ok());
    if header_host.zip(query_host).is_some_and(|(a, b)| a != b) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"Conflicting hosts"})),
        )
            .into_response();
    }
    let id = header_host.or(query_host).unwrap_or(&registry()[0].id);
    let Some(host) = registry().iter().find(|h| h.id == id).cloned() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"success":false,"error":"Unknown host"})),
        )
            .into_response();
    };
    let context = Context {
        host,
        failure: Arc::default(),
    };
    let failure = context.failure.clone();
    let response = CONTEXT.scope(context, next.run(request)).await;
    let error = failure.lock().unwrap().clone();
    match error {
        Some(error) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"success":false,"error":error,"offline":true})),
        )
            .into_response(),
        None => response,
    }
}

fn failed(message: &str) {
    let _ = CONTEXT
        .try_with(|c| *c.failure.lock().unwrap() = Some(format!("{}: {message}", c.host.id)));
}

pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn remote_command(program: &str, args: &[OsString]) -> io::Result<String> {
    // Reconstruct argv on the destination instead of embedding control bytes
    // in the SSH shell command. Direct SSH on vade otherwise turns tmux format
    // tabs into underscores. This also preserves multiline literal input and
    // embedded filesystem helper source without a second shell interpretation.
    let mut argv = vec![program];
    for arg in args {
        argv.push(arg.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "SSH arguments must be UTF-8")
        })?);
    }
    let payload = BASE64.encode(serde_json::to_vec(&argv).map_err(io::Error::other)?);
    let launcher =
        "import os,sys,json,base64; a=json.loads(base64.b64decode(sys.argv[1])); os.execvp(a[0],a)";
    Ok(format!(
        "python3 -c {} {}",
        quote(launcher),
        quote(&payload)
    ))
}

fn connection_slots() -> Arc<tokio::sync::Semaphore> {
    static SLOTS: OnceLock<Mutex<std::collections::HashMap<String, Arc<tokio::sync::Semaphore>>>> =
        OnceLock::new();
    SLOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .entry(current().id)
        .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(6)))
        .clone()
}

pub struct Command {
    program: String,
    args: Vec<OsString>,
    input: Option<Vec<u8>>,
    deadline: Duration,
}
pub fn command(program: &str) -> Command {
    Command {
        program: program.into(),
        args: vec![],
        input: None,
        deadline: Duration::from_secs(8),
    }
}
pub fn tmux() -> Command {
    command("tmux")
}
impl Command {
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|s| s.as_ref().to_owned()));
        self
    }
    pub fn input(&mut self, bytes: Vec<u8>) -> &mut Self {
        self.input = Some(bytes);
        self
    }
    pub fn timeout(&mut self, seconds: u64) -> &mut Self {
        self.deadline = Duration::from_secs(seconds);
        self
    }
    pub async fn output(&mut self) -> io::Result<Output> {
        if CONTEXT
            .try_with(|c| c.failure.lock().unwrap().is_some())
            .unwrap_or(false)
        {
            return Err(io::Error::other(
                "A previous host operation failed; action stopped",
            ));
        }
        let _slot = tokio::time::timeout(self.deadline, connection_slots().acquire_owned())
            .await
            .map_err(|_| {
                failed("Host is busy; try again");
                io::Error::new(io::ErrorKind::TimedOut, "Host is busy")
            })?
            .map_err(io::Error::other)?;
        let host = current();
        let mut cmd = if let Some(alias) = host.ssh {
            static SOCKET_DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
            let dir = SOCKET_DIR
                .get_or_init(|| {
                    tempfile::Builder::new()
                        .prefix("tmux-terminal-ssh-")
                        .tempdir()
                        .expect("private SSH socket directory")
                })
                .path();
            let mut ssh = tokio::process::Command::new("ssh");
            ssh.args([
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "StrictHostKeyChecking=yes",
                "-o",
                "ConnectTimeout=5",
                "-o",
                "ServerAliveInterval=5",
                "-o",
                "ServerAliveCountMax=1",
                "-o",
                "ControlMaster=auto",
                "-o",
                "ControlPersist=60",
                "-o",
            ])
            .arg(format!("ControlPath={}/%C", dir.display()))
            .arg(alias);
            ssh.arg(remote_command(&self.program, &self.args)?);
            ssh
        } else {
            let mut local = tokio::process::Command::new(&self.program);
            local.args(&self.args);
            local
        };
        cmd.kill_on_drop(true)
            .stdin(if self.input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let result = async {
            let mut child = cmd.spawn()?;
            let input = self.input.take();
            let stdin = child.stdin.take();
            let write = async move {
                if let (Some(bytes), Some(mut stdin)) = (input, stdin) {
                    stdin.write_all(&bytes).await?;
                    stdin.shutdown().await?;
                }
                Ok::<_, io::Error>(())
            };
            let (written, output) = tokio::join!(write, child.wait_with_output());
            let output = output?;
            if output.status.success() {
                written?;
            }
            Ok::<_, io::Error>(output)
        };
        match tokio::time::timeout(self.deadline, result).await {
            Ok(Ok(out)) if remote() && out.status.code() == Some(255) => {
                failed("SSH connection failed");
                Err(io::Error::other(
                    String::from_utf8_lossy(&out.stderr).trim(),
                ))
            }
            Ok(Ok(out)) => Ok(out),
            Ok(Err(e)) => {
                failed(&e.to_string());
                Err(e)
            }
            Err(_) => {
                failed("operation timed out");
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Host operation timed out",
                ))
            }
        }
    }
    pub async fn status(&mut self) -> io::Result<std::process::ExitStatus> {
        Ok(self.output().await?.status)
    }
}

pub async fn filesystem(
    op: &str,
    data: serde_json::Value,
    input: Option<Vec<u8>>,
) -> Result<Vec<u8>, String> {
    let mut cmd = command("python3");
    cmd.args([
        "-c",
        include_str!("../scripts/host-files.py"),
        op,
        &data.to_string(),
    ]);
    if let Some(bytes) = input {
        cmd.input(bytes).timeout(120);
    }
    let out = cmd.output().await.map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(out.stdout)
}

pub async fn fs_json(op: &str, data: serde_json::Value) -> Result<serde_json::Value, String> {
    serde_json::from_slice(&filesystem(op, data, None).await?).map_err(|e| e.to_string())
}

pub async fn agents() -> impl IntoResponse {
    // Output names only. Do not expose shell startup output or environment values.
    let out = command("bash").args(["-ic", "for a in claude codex agy eunice hermes; do command -v \"$a\" >/dev/null 2>&1 && printf '__TMUX_AGENT__%s\\n' \"$a\"; done; true"]).output().await;
    match out {
        Ok(out) if out.status.success() => {
            let names: Vec<String> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|s| s.strip_prefix("__TMUX_AGENT__").map(str::to_string))
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"agents":names})))
        }
        _ => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error":"Could not read available agents"})),
        ),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shell_arguments_remain_literal() {
        assert_eq!(
            super::quote("a'b\n$(touch /tmp/no);"),
            "'a'\\''b\n$(touch /tmp/no);'"
        );
    }
}
