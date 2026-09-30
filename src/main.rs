use actix_web::{web, App, HttpServer, HttpResponse, Responder};
use actix_ws::Message;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

#[derive(Deserialize)]
struct RunReq {
    code: String,
    stdin: Option<String>,
}

#[derive(Serialize)]
struct RunRes {
    ok: bool,
    stdout: String,
    stderr: String,
    exit: i32,
}

#[derive(Deserialize)]
struct DiagReq {
    code: String,
}

#[derive(Serialize)]
struct Diagnostic {
    line: usize,
    col: usize,
    severity: String,
    message: String,
    suggestion: Option<String>,
}

#[derive(Serialize)]
struct DiagRes {
    diagnostics: Vec<Diagnostic>,
}

const INDEX_HTML: &str = include_str!("index.html");
const C_DATA: &str = include_str!("c_data.js");

async fn index() -> impl Responder {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(INDEX_HTML)
}

async fn c_data() -> impl Responder {
    HttpResponse::Ok()
        .content_type("application/javascript; charset=utf-8")
        .body(C_DATA)
}

async fn run_code(body: web::Json<RunReq>) -> impl Responder {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };
    let src = dir.path().join("main.c");
    let exe = dir.path().join("program");

    if let Err(e) = std::fs::write(&src, &body.code) {
        return HttpResponse::InternalServerError().body(e.to_string());
    }

    let comp = Command::new("gcc")
        .args(["-Wall", "-Wextra", "-std=c11", "-g", "-O0", "-fdiagnostics-color=never"])
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .arg("-lm")
        .output()
        .await;

    let comp = match comp {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::Ok().json(RunRes {
                ok: false,
                stdout: String::new(),
                stderr: format!("failed to run gcc: {e}"),
                exit: -1,
            })
        }
    };

    if !comp.status.success() {
        return HttpResponse::Ok().json(RunRes {
            ok: false,
            stdout: String::new(),
            stderr: String::from_utf8_lossy(&comp.stderr).into_owned(),
            exit: comp.status.code().unwrap_or(-1),
        });
    }

    let mut child = match Command::new(&exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::Ok().json(RunRes {
                ok: false,
                stdout: String::new(),
                stderr: format!("failed to run program: {e}"),
                exit: -1,
            })
        }
    };

    if let Some(input) = &body.stdin {
        if let Some(mut si) = child.stdin.take() {
            let _ = si.write_all(input.as_bytes()).await;
            drop(si);
        }
    } else {
        drop(child.stdin.take());
    }

    match child.wait_with_output().await {
        Ok(o) => HttpResponse::Ok().json(RunRes {
            ok: true,
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            exit: o.status.code().unwrap_or(-1),
        }),
        Err(e) => HttpResponse::Ok().json(RunRes {
            ok: false,
            stdout: String::new(),
            stderr: format!("wait failed: {e}"),
            exit: -1,
        }),
    }
}

async fn diagnostics(body: web::Json<DiagReq>) -> impl Responder {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };
    let src = dir.path().join("main.c");
    if let Err(e) = std::fs::write(&src, &body.code) {
        return HttpResponse::InternalServerError().body(e.to_string());
    }

    let comp = Command::new("gcc")
        .args(["-Wall", "-Wextra", "-std=c11", "-fsyntax-only", "-fdiagnostics-color=never"])
        .arg(&src)
        .output()
        .await;

    let mut diags = Vec::new();
    if let Ok(c) = comp {
        let stderr = String::from_utf8_lossy(&c.stderr);
        let lines: Vec<&str> = body.code.lines().collect();
        diags = parse_gcc_diagnostics(&stderr, &lines);
    }

    HttpResponse::Ok().json(DiagRes { diagnostics: diags })
}

fn parse_gcc_diagnostics(stderr: &str, source_lines: &[&str]) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for line in stderr.lines() {
        let rest = match line.strip_prefix("main.c:") {
            Some(r) => r,
            None => continue,
        };
        let parts: Vec<&str> = rest.splitn(4, ':').collect();
        if parts.len() < 4 {
            continue;
        }
        let line_no: usize = match parts[0].trim().parse() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let col: usize = match parts[1].trim().parse() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let sev = parts[2].trim();
        if !(sev.contains("error") || sev.contains("warning") || sev.contains("note")) {
            continue;
        }
        let message = parts[3].trim().to_string();
        let src_line = source_lines.get(line_no.saturating_sub(1)).copied().unwrap_or("");
        let suggestion = suggest_fix(&message, src_line, source_lines);

        out.push(Diagnostic {
            line: line_no,
            col,
            severity: sev.replace(":", "").trim().to_string(),
            message,
            suggestion,
        });
    }
    out
}

fn suggest_fix(message: &str, _src_line: &str, all_lines: &[&str]) -> Option<String> {
    let quoted = extract_first_quoted(message);

    if message.contains("undeclared") {
        if let Some(name) = &quoted {
            if let Some(best) = best_match(name, all_lines) {
                return Some(format!(
                    "'{name}' is not declared. Did you mean '{best}'? Or did you forget the right #include?"
                ));
            }
            return Some(format!(
                "'{name}' is not declared. Check spelling or add the right #include."
            ));
        }
    }

    if message.contains("expected ';'") {
        return Some("Missing semicolon at the end of the statement above.".into());
    }
    if message.contains("expected ')'") || message.contains("expected '}'") {
        return Some("Unbalanced parentheses or braces nearby. Check the matching opener.".into());
    }
    if message.contains("implicit declaration of function") {
        if let Some(name) = &quoted {
            let header = guess_header(name);
            return Some(format!(
                "Function '{name}' has no prototype. Add '#include <{header}>' at the top."
            ));
        }
    }
    if message.contains("format") && message.contains("%") {
        return Some("Format specifier doesn't match the argument type. Check %d / %f / %s / %c vs. your values.".into());
    }
    if message.contains("unused variable") {
        if let Some(name) = &quoted {
            return Some(format!("Variable '{name}' is set but never used. Remove it or use it."));
        }
    }
    if message.contains("control reaches end of non-void") {
        return Some("Function must return a value. Add 'return 0;' or the appropriate return.".into());
    }

    if let Some(name) = quoted {
        if let Some(best) = best_match(&name, all_lines) {
            if best != name {
                return Some(format!("Did you mean '{best}'?"));
            }
        }
    }

    None
}

fn extract_first_quoted(s: &str) -> Option<String> {
    let mut buf = String::new();
    let mut in_quote = false;
    for c in s.chars() {
        if c == '\'' {
            if in_quote {
                return Some(buf);
            }
            in_quote = true;
            buf.clear();
        } else if in_quote {
            buf.push(c);
        }
    }
    None
}

fn best_match(target: &str, lines: &[&str]) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for line in lines {
        for tok in tokenize_identifiers(line) {
            if tok == target {
                continue;
            }
            let d = levenshtein(target, &tok);
            let threshold = (target.len() / 3).max(1);
            if d <= threshold {
                if best.as_ref().map_or(true, |(bd, _)| d < *bd) {
                    best = Some((d, tok));
                }
            }
        }
    }
    best.map(|(_, s)| s)
}

fn tokenize_identifiers(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() || c == '_' {
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

fn guess_header(func: &str) -> &'static str {
    match func {
        "printf" | "fprintf" | "sprintf" | "snprintf" | "scanf" | "fscanf"
        | "sscanf" | "fopen" | "fclose" | "fread" | "fwrite" | "fgets"
        | "fputs" | "fgetc" | "fputc" | "fseek" | "ftell" | "rewind"
        | "feof" | "ferror" | "perror" | "puts" | "putchar" | "getchar"
        | "remove" | "rename" | "fflush" | "setvbuf" => "stdio.h",
        "malloc" | "calloc" | "realloc" | "free" | "exit" | "abort"
        | "atexit" | "atoi" | "atol" | "atof" | "strtol" | "strtod"
        | "rand" | "srand" | "qsort" | "bsearch" | "abs" | "labs"
        | "getenv" | "system" => "stdlib.h",
        "strlen" | "strcpy" | "strncpy" | "strcat" | "strncat"
        | "strcmp" | "strncmp" | "strchr" | "strrchr" | "strstr"
        | "strtok" | "strdup" | "memcpy" | "memmove" | "memset"
        | "memcmp" | "memchr" => "string.h",
        "sqrt" | "pow" | "exp" | "log" | "log10" | "sin" | "cos"
        | "tan" | "asin" | "acos" | "atan" | "atan2" | "floor"
        | "ceil" | "round" | "fabs" | "fmod" | "hypot" => "math.h",
        "isalpha" | "isdigit" | "isalnum" | "isspace" | "isupper"
        | "islower" | "toupper" | "tolower" => "ctype.h",
        _ => "the correct header",
    }
}

async fn ws_run(req: actix_web::HttpRequest, stream: web::Payload) -> actix_web::Result<HttpResponse> {
    let (res, mut session, mut msg_stream) = actix_ws::handle(&req, stream)?;

    actix_web::rt::spawn(async move {
        while let Some(Ok(msg)) = msg_stream.next().await {
            match msg {
                Message::Text(text) => {
                    let req: RunReq = match serde_json::from_str(&text) {
                        Ok(r) => r,
                        Err(e) => {
                            let _ = session.text(format!("{{\"error\":\"{e}\"}}")).await;
                            continue;
                        }
                    };
                    let dir = tempfile::tempdir().unwrap();
                    let src = dir.path().join("main.c");
                    let exe = dir.path().join("program");
                    std::fs::write(&src, &req.code).ok();

                    let comp = Command::new("gcc")
                        .args(["-Wall", "-Wextra", "-std=c11", "-g", "-O0",
                               "-fdiagnostics-color=never"])
                        .arg(&src).arg("-o").arg(&exe).arg("-lm")
                        .output().await;

                    match comp {
                        Ok(c) if c.status.success() => {
                            let _ = session.text("{\"event\":\"compiled\"}").await;

                            let mut child = match Command::new(&exe)
                                .stdin(Stdio::piped())
                                .stdout(Stdio::piped())
                                .stderr(Stdio::piped())
                                .spawn()
                            {
                                Ok(c) => c,
                                Err(e) => {
                                    let _ = session.text(format!("{{\"error\":\"{e}\"}}")).await;
                                    continue;
                                }
                            };

                            let mut stdin_handle = child.stdin.take();
                            let stdout = child.stdout.take().unwrap();
                            let stderr = child.stderr.take().unwrap();
                            let (tx, mut rx) = mpsc::unbounded_channel::<String>();

                            let tx1 = tx.clone();
                            tokio::spawn(async move {
                                let mut lines = BufReader::new(stdout).lines();
                                while let Ok(Some(line)) = lines.next_line().await {
                                    let _ = tx1.send(format!("{{\"out\":{}}}\n",
                                        serde_json::to_string(&(line + "\n")).unwrap()));
                                }
                            });
                            let tx2 = tx.clone();
                            tokio::spawn(async move {
                                let mut lines = BufReader::new(stderr).lines();
                                while let Ok(Some(line)) = lines.next_line().await {
                                    let _ = tx2.send(format!("{{\"err\":{}}}\n",
                                        serde_json::to_string(&(line + "\n")).unwrap()));
                                }
                            });
                            drop(tx);

                            if let Some(input) = &req.stdin {
                                if let Some(si) = stdin_handle.as_mut() {
                                    let _ = si.write_all(input.as_bytes()).await;
                                }
                            }

                            loop {
                                tokio::select! {
                                    Some(line) = rx.recv() => {
                                        if session.text(line).await.is_err() { break; }
                                    }
                                    incoming = msg_stream.next() => {
                                        match incoming {
                                            Some(Ok(Message::Text(t))) => {
                                                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
                                                    if let Some(s) = v.get("stdin").and_then(|x| x.as_str()) {
                                                        if let Some(si) = stdin_handle.as_mut() {
                                                            let _ = si.write_all(s.as_bytes()).await;
                                                            let _ = si.flush().await;
                                                        }
                                                    }
                                                }
                                            }
                                            Some(Ok(Message::Close(_))) | None => break,
                                            _ => {}
                                        }
                                    }
                                    else => break,
                                }
                            }

                            let status = child.wait().await;
                            let code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
                            let _ = session.text(format!("{{\"exit\":{code}}}")).await;
                        }
                        Ok(c) => {
                            let err = String::from_utf8_lossy(&c.stderr);
                            let msg = serde_json::json!({ "compile_error": err });
                            let _ = session.text(msg.to_string()).await;
                        }
                        Err(e) => {
                            let msg = serde_json::json!({ "error": format!("{e}") });
                            let _ = session.text(msg.to_string()).await;
                        }
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    Ok(res)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let host = std::env::var("CROAD_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("CROAD_PORT").unwrap_or_else(|_| "8080".into());
    let addr = format!("{host}:{port}");
    println!("C-Road running at http://{addr}");
    HttpServer::new(|| {
        App::new()
            .route("/", web::get().to(index))
            .route("/c_data.js", web::get().to(c_data))
            .route("/run", web::post().to(run_code))
            .route("/diagnostics", web::post().to(diagnostics))
            .route("/ws", web::get().to(ws_run))
    })
    .bind(&addr)?
    .run()
    .await
}