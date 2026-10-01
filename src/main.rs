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

// ============================================================================
// Embedded frontend — kept inside main.rs so the build is a single artifact.
// ============================================================================

const INDEX_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>C-Road</title>
<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/codemirror@5.65.16/lib/codemirror.css">
<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/codemirror@5.65.16/addon/hint/show-hint.css">
<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/codemirror@5.65.16/addon/fold/foldgutter.css">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=JetBrains+Mono:wght@400;500;600&display=swap" rel="stylesheet">
<style>
  :root {
    --bg-0: #0a0a0b;
    --bg-1: #0f0f11;
    --bg-2: #141416;
    --bg-3: #1a1a1d;
    --bg-hover: #202024;
    --border: #232328;
    --border-strong: #2e2e34;
    --fg-0: #f5f5f7;
    --fg-1: #b4b4bb;
    --fg-2: #7a7a83;
    --fg-3: #4a4a52;
    --accent: #5b8def;
    --accent-hi: #6f9cf5;
    --accent-dim: rgba(91, 141, 239, 0.12);
    --accent-ring: rgba(91, 141, 239, 0.35);
    --ok: #34d399;
    --warn: #fbbf24;
    --err: #f87171;
    --shadow-sm: 0 1px 2px rgba(0,0,0,.4);
    --shadow-md: 0 4px 12px rgba(0,0,0,.4);
    --shadow-lg: 0 24px 48px rgba(0,0,0,.55), 0 0 0 1px rgba(255,255,255,.03);
    --radius-sm: 6px;
    --radius: 8px;
    --radius-lg: 12px;
    --cm-bg: #0f0f11;
    --cm-fg: #e4e4e7;
    color-scheme: dark;
  }
  :root.light {
    --bg-0: #fafafa;
    --bg-1: #ffffff;
    --bg-2: #f5f5f7;
    --bg-3: #ebebee;
    --bg-hover: #e4e4e7;
    --border: #e4e4e7;
    --border-strong: #d4d4d8;
    --fg-0: #18181b;
    --fg-1: #52525b;
    --fg-2: #71717a;
    --fg-3: #a1a1aa;
    --accent: #2563eb;
    --accent-hi: #3b82f6;
    --accent-dim: rgba(37, 99, 235, 0.08);
    --accent-ring: rgba(37, 99, 235, 0.25);
    --ok: #059669;
    --warn: #d97706;
    --err: #dc2626;
    --shadow-sm: 0 1px 2px rgba(0,0,0,.06);
    --shadow-md: 0 4px 12px rgba(0,0,0,.08);
    --shadow-lg: 0 24px 48px rgba(0,0,0,.15), 0 0 0 1px rgba(0,0,0,.05);
    --cm-bg: #ffffff;
    --cm-fg: #18181b;
    color-scheme: light;
  }

  * { box-sizing: border-box; }
  html, body {
    margin: 0; height: 100%; background: var(--bg-0); color: var(--fg-0);
    font-family: 'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    font-size: 13px; overflow: hidden;
    font-feature-settings: 'cv02','cv03','cv04','cv11';
    -webkit-font-smoothing: antialiased;
    -moz-osx-font-smoothing: grayscale;
  }
  ::selection { background: var(--accent-ring); color: var(--fg-0); }

  /* Subtle noise texture for depth (no gradient) */
  body::before {
    content: ''; position: fixed; inset: 0; pointer-events: none;
    opacity: .015; z-index: 9999;
    background-image: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='120' height='120'><filter id='n'><feTurbulence type='fractalNoise' baseFrequency='.9' numOctaves='2'/></filter><rect width='100%25' height='100%25' filter='url(%23n)'/></svg>");
  }

  .app { display: flex; flex-direction: column; height: 100vh; position: relative; }

  /* ---------- Toolbar ---------- */
  .toolbar {
    display: flex; align-items: center; gap: 2px;
    padding: 10px 14px;
    background: var(--bg-1);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    -webkit-app-region: drag;
  }
  .toolbar .brand {
    display: flex; align-items: center; gap: 8px;
    font-weight: 600; font-size: 13px; letter-spacing: -0.01em;
    color: var(--fg-0); margin-right: 14px; user-select: none;
  }
  .toolbar .brand-mark {
    width: 20px; height: 20px; border-radius: 5px;
    background: var(--accent);
    display: inline-flex; align-items: center; justify-content: center;
    color: #fff; font-weight: 700; font-size: 11px;
    box-shadow: 0 0 0 1px var(--accent-ring);
  }
  .toolbar .divider {
    width: 1px; height: 20px; background: var(--border);
    margin: 0 8px; flex-shrink: 0;
  }
  .toolbar .spacer { flex: 1; }
  .toolbar .file-chip {
    display: inline-flex; align-items: center; gap: 6px;
    padding: 4px 10px; border-radius: var(--radius-sm);
    background: var(--bg-2); border: 1px solid var(--border);
    font-family: 'JetBrains Mono', monospace; font-size: 11.5px;
    color: var(--fg-1); user-select: none;
    max-width: 240px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .toolbar .file-chip::before {
    content: ''; width: 6px; height: 6px; border-radius: 50%;
    background: var(--fg-3); flex-shrink: 0;
  }
  .toolbar .file-chip.dirty::before { background: var(--warn); }

  /* ---------- Buttons ---------- */
  .btn {
    appearance: none; border: 1px solid transparent;
    background: transparent; color: var(--fg-1);
    font-family: inherit; font-size: 12.5px; font-weight: 500;
    padding: 6px 11px; border-radius: var(--radius-sm);
    cursor: pointer; display: inline-flex; align-items: center; gap: 6px;
    transition: background .12s ease, color .12s ease, border-color .12s ease, box-shadow .12s ease;
    line-height: 1.4; user-select: none;
    -webkit-app-region: no-drag;
  }
  .btn:hover { background: var(--bg-hover); color: var(--fg-0); }
  .btn:active { background: var(--bg-3); }
  .btn:focus-visible {
    outline: none; box-shadow: 0 0 0 2px var(--bg-1), 0 0 0 4px var(--accent-ring);
  }
  .btn.ghost { color: var(--fg-2); }
  .btn.ghost:hover { color: var(--fg-0); background: var(--bg-hover); }

  .btn.solid {
    background: var(--bg-3); border-color: var(--border);
    color: var(--fg-0);
    box-shadow: var(--shadow-sm);
  }
  .btn.solid:hover { background: var(--bg-hover); border-color: var(--border-strong); }

  .btn.primary {
    background: var(--accent); color: #fff; font-weight: 600;
    border-color: transparent;
    box-shadow: 0 1px 2px rgba(0,0,0,.3), inset 0 1px 0 rgba(255,255,255,.12);
  }
  .btn.primary:hover { background: var(--accent-hi); color: #fff; }

  .btn.danger { color: var(--err); }
  .btn.danger:hover { background: rgba(248,113,113,.1); color: var(--err); }

  .btn .kbd {
    font-family: 'JetBrains Mono', monospace; font-size: 10px;
    padding: 1px 5px; border-radius: 4px;
    background: rgba(255,255,255,.06); color: var(--fg-2);
    margin-left: 2px; font-weight: 500;
    border: 1px solid rgba(255,255,255,.04);
  }
  .btn.primary .kbd {
    background: rgba(0,0,0,.2); color: rgba(255,255,255,.85);
    border-color: rgba(0,0,0,.15);
  }
  :root.light .btn .kbd { background: rgba(0,0,0,.05); border-color: rgba(0,0,0,.04); }

  /* ---------- Main layout ---------- */
  .main { flex: 1; display: flex; min-height: 0; }

  /* ---------- Sidebar ---------- */
  .sidebar {
    width: 240px; min-width: 140px; background: var(--bg-1);
    border-right: 1px solid var(--border);
    display: flex; flex-direction: column; overflow: hidden;
  }
  .sidebar-header {
    padding: 10px 14px 8px;
    font-size: 10.5px; font-weight: 600; letter-spacing: 0.08em;
    color: var(--fg-3); text-transform: uppercase;
    user-select: none; flex-shrink: 0;
  }
  .file-tree { flex: 1; overflow: auto; padding: 4px 6px 12px; font-size: 12.5px; }
  .file-tree .empty {
    padding: 20px 14px; color: var(--fg-3); font-size: 12px;
    text-align: center; line-height: 1.6;
  }
  .file-tree .item {
    display: flex; align-items: center; gap: 8px;
    padding: 5px 8px; border-radius: var(--radius-sm);
    cursor: pointer; color: var(--fg-1); user-select: none;
    transition: background .1s ease, color .1s ease;
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  }
  .file-tree .item:hover { background: var(--bg-hover); color: var(--fg-0); }
  .file-tree .item.active {
    background: var(--accent-dim); color: var(--accent);
    font-weight: 500;
  }
  .file-tree .item.active::before { background: var(--accent); }
  .file-tree .item::before {
    content: ''; width: 3px; height: 14px; border-radius: 2px;
    background: transparent; flex-shrink: 0;
    margin-left: -4px;
  }
  .file-tree .item.folder { color: var(--fg-1); font-weight: 500; }
  .file-tree .item.folder:hover { color: var(--fg-0); }
  .file-tree .caret {
    width: 10px; height: 10px; flex-shrink: 0;
    display: inline-block; position: relative;
    transition: transform .15s ease;
  }
  .file-tree .caret::before {
    content: ''; position: absolute;
    top: 50%; left: 50%; transform: translate(-50%,-50%) rotate(45deg);
    width: 4px; height: 4px;
    border-right: 1.5px solid currentColor; border-bottom: 1.5px solid currentColor;
    transition: transform .15s ease;
  }
  .file-tree .caret.open::before { transform: translate(-50%,-50%) rotate(-135deg); }
  .file-tree .indent-1 { padding-left: 20px; }
  .file-tree .indent-2 { padding-left: 36px; }
  .file-tree .indent-3 { padding-left: 52px; }

  .file-tree .icon {
    width: 14px; height: 14px; flex-shrink: 0;
    display: inline-block; position: relative; opacity: .7;
  }
  .file-tree .icon.file::before {
    content: ''; position: absolute; inset: 1px 2px 1px 1px;
    border: 1.5px solid currentColor; border-radius: 2px;
    border-top-right-radius: 4px;
  }
  .file-tree .icon.file::after {
    content: ''; position: absolute;
    top: 1px; right: 1px; width: 4px; height: 4px;
    border-top: 1.5px solid currentColor;
    border-right: 1.5px solid currentColor;
    border-top-right-radius: 3px;
  }
  .file-tree .icon.folder {
    opacity: .8;
  }
  .file-tree .icon.folder::before {
    content: ''; position: absolute; inset: 2px 1px 1px;
    border: 1.5px solid currentColor; border-radius: 2px;
  }
  .file-tree .icon.folder::after {
    content: ''; position: absolute;
    top: 0; left: 2px; width: 6px; height: 3px;
    border: 1.5px solid currentColor; border-bottom: 0;
    border-radius: 2px 2px 0 0;
  }

  /* ---------- Editor area ---------- */
  .editor-area { flex: 1; display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  .editor-wrap { flex: 1; position: relative; min-height: 0; overflow: hidden; background: var(--cm-bg); }
  #editor { position: absolute; inset: 0; }
  .CodeMirror {
    height: 100% !important;
    font-family: 'JetBrains Mono', monospace !important;
    font-size: 13.5px !important; line-height: 1.65 !important;
    background: var(--cm-bg) !important;
    color: var(--cm-fg) !important;
  }
  .CodeMirror-lines { padding: 14px 0 !important; }
  .CodeMirror-gutters {
    background: var(--cm-bg) !important;
    border-right: 1px solid var(--border) !important;
    padding-right: 12px !important;
  }
  .CodeMirror-linenumber { color: var(--fg-3) !important; font-weight: 400; }
  .CodeMirror-cursor { border-left: 2px solid var(--accent) !important; }
  .CodeMirror-activeline-background { background: rgba(255,255,255,.018) !important; }
  :root.light .CodeMirror-activeline-background { background: rgba(0,0,0,.02) !important; }

  /* ---------- Splitters ---------- */
  .splitter-h {
    height: 1px; background: var(--border); cursor: row-resize;
    flex-shrink: 0; position: relative; z-index: 2;
  }
  .splitter-h::after {
    content: ''; position: absolute; left: 0; right: 0;
    top: -3px; bottom: -3px; z-index: 3;
  }
  .splitter-h:hover { background: var(--accent); }
  .splitter-v {
    width: 1px; background: var(--border); cursor: col-resize;
    flex-shrink: 0; position: relative; z-index: 2;
  }
  .splitter-v::after {
    content: ''; position: absolute; top: 0; bottom: 0;
    left: -3px; right: -3px; z-index: 3;
  }
  .splitter-v:hover { background: var(--accent); }

  /* ---------- Console ---------- */
  .console {
    height: 260px; min-height: 100px;
    background: var(--bg-1);
    display: flex; flex-direction: column; flex-shrink: 0;
  }
  .console-tabs {
    display: flex; align-items: center;
    padding: 0 14px; gap: 2px;
    background: var(--bg-1);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0; height: 36px;
  }
  .console-tab {
    padding: 8px 12px;
    font-size: 12px; font-weight: 500;
    color: var(--fg-2); cursor: pointer;
    border-radius: var(--radius-sm) var(--radius-sm) 0 0;
    display: inline-flex; align-items: center; gap: 6px;
    user-select: none; position: relative;
    transition: color .12s ease;
  }
  .console-tab:hover { color: var(--fg-0); }
  .console-tab.active { color: var(--fg-0); }
  .console-tab.active::after {
    content: ''; position: absolute; left: 12px; right: 12px; bottom: -1px;
    height: 1px; background: var(--accent);
  }
  .console-tab .badge {
    font-family: 'JetBrains Mono', monospace; font-size: 10px;
    padding: 1px 6px; border-radius: 100px; font-weight: 600;
    background: var(--bg-3); color: var(--fg-2);
  }
  .console-tab .badge.err { background: rgba(248,113,113,.15); color: var(--err); }
  .console-tab .badge.warn { background: rgba(251,191,36,.15); color: var(--warn); }
  .console-tab .badge.ok { background: rgba(52,211,153,.15); color: var(--ok); }

  .console-header-spacer { flex: 1; }
  .console-status {
    display: inline-flex; align-items: center; gap: 6px;
    font-size: 11.5px; color: var(--fg-2);
    padding: 4px 10px; border-radius: 100px;
    background: var(--bg-2); border: 1px solid var(--border);
  }
  .console-status::before {
    content: ''; width: 6px; height: 6px; border-radius: 50%;
    background: var(--fg-3); transition: background .15s ease;
  }
  .console-status.ready::before { background: var(--fg-3); }
  .console-status.busy::before { background: var(--warn); animation: pulse 1.4s ease-in-out infinite; }
  .console-status.ok::before { background: var(--ok); }
  .console-status.err::before { background: var(--err); }
  @keyframes pulse { 0%,100% { opacity: 1 } 50% { opacity: .35 } }

  .pane { flex: 1; overflow: auto; min-height: 0; }
  .pane.hidden { display: none; }

  .console-body {
    padding: 14px 16px;
    font-family: 'JetBrains Mono', monospace;
    font-size: 12.5px; line-height: 1.65;
    white-space: pre-wrap; word-break: break-word;
    color: var(--fg-0);
    background: var(--bg-0);
    min-height: 100%;
  }
  .console-body .line { display: block; }
  .console-body .err { color: var(--err); }
  .console-body .ok { color: var(--ok); }
  .console-body .warn { color: var(--warn); }
  .console-body .dim { color: var(--fg-2); }
  .console-body .prompt-line { color: var(--accent); }
  .console-body .exit-code { color: var(--fg-2); font-style: italic; }
  .console-body .sys { color: var(--fg-2); }
  .console-body .sys::before { content: ''; }

  .diagnostics-list { padding: 8px; }
  .diagnostics-list .diag {
    display: flex; gap: 12px; align-items: flex-start;
    padding: 10px 12px;
    background: var(--bg-1);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    margin-bottom: 6px;
    cursor: pointer;
    transition: border-color .12s ease, background .12s ease;
  }
  .diagnostics-list .diag:hover { border-color: var(--border-strong); background: var(--bg-2); }
  .diagnostics-list .diag.error { border-left: 3px solid var(--err); }
  .diagnostics-list .diag.warning { border-left: 3px solid var(--warn); }
  .diagnostics-list .diag.note { border-left: 3px solid var(--fg-3); }
  .diagnostics-list .diag .loc {
    font-family: 'JetBrains Mono', monospace; font-size: 11px; font-weight: 500;
    color: var(--fg-2); flex-shrink: 0; min-width: 68px;
    padding-top: 1px;
  }
  .diagnostics-list .diag.error .loc { color: var(--err); }
  .diagnostics-list .diag.warning .loc { color: var(--warn); }
  .diagnostics-list .diag .body { flex: 1; min-width: 0; }
  .diagnostics-list .diag .msg { color: var(--fg-0); font-size: 12.5px; line-height: 1.5; }
  .diagnostics-list .diag .sugg {
    margin-top: 6px; padding: 8px 10px;
    background: var(--accent-dim);
    border-radius: var(--radius-sm);
    color: var(--fg-1); font-size: 12px; line-height: 1.5;
    display: flex; gap: 8px; align-items: flex-start;
  }
  .diagnostics-list .diag .sugg::before {
    content: ''; flex-shrink: 0;
    width: 14px; height: 14px; margin-top: 1px;
    background: var(--accent);
    -webkit-mask: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='black' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'><circle cx='12' cy='12' r='10'/><path d='M12 16v-4'/><path d='M12 8h.01'/></svg>") center/contain no-repeat;
    mask: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='black' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'><circle cx='12' cy='12' r='10'/><path d='M12 16v-4'/><path d='M12 8h.01'/></svg>") center/contain no-repeat;
  }
  .diagnostics-list .empty {
    padding: 24px; text-align: center; color: var(--fg-3);
    font-size: 12px;
  }

  .console-input {
    display: flex; align-items: center; gap: 10px;
    padding: 10px 14px;
    background: var(--bg-1);
    border-top: 1px solid var(--border);
    flex-shrink: 0;
  }
  .console-input .prompt-mark {
    font-family: 'JetBrains Mono', monospace;
    color: var(--accent); font-size: 14px; font-weight: 600;
    user-select: none; flex-shrink: 0;
  }
  .console-input input {
    flex: 1;
    background: var(--bg-2);
    color: var(--fg-0);
    border: 1px solid var(--border);
    padding: 7px 11px;
    border-radius: var(--radius-sm);
    font-family: 'JetBrains Mono', monospace;
    font-size: 12.5px; outline: none;
    transition: border-color .12s ease, box-shadow .12s ease;
  }
  .console-input input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-dim);
  }
  .console-input input::placeholder { color: var(--fg-3); }

  /* ---------- Diagnostics inline markers ---------- */
  .cm-error-line { background: rgba(248,113,113,.10) !important; }
  .cm-warn-line { background: rgba(251,191,36,.08) !important; }

  /* ---------- Autocomplete hint popup ---------- */
  .CodeMirror-hints {
    background: var(--bg-1) !important;
    border: 1px solid var(--border-strong) !important;
    border-radius: var(--radius) !important;
    box-shadow: var(--shadow-lg) !important;
    font-family: 'JetBrains Mono', monospace !important;
    font-size: 12.5px !important;
    padding: 4px !important;
    z-index: 100 !important;
  }
  .CodeMirror-hint {
    padding: 5px 10px !important;
    border-radius: var(--radius-sm) !important;
    color: var(--fg-1) !important;
    white-space: pre;
  }
  .CodeMirror-hint-active {
    background: var(--accent) !important;
    color: #fff !important;
  }
  li.CodeMirror-hint.hint-func::before { content: ''; }
  .CodeMirror-hint .hint-kind {
    display: inline-block; font-size: 10px;
    padding: 1px 5px; border-radius: 4px;
    margin-left: 8px; opacity: .7; font-weight: 500;
  }

  /* ---------- About modal ---------- */
  .modal-bg {
    position: fixed; inset: 0;
    background: rgba(0,0,0,.7);
    backdrop-filter: blur(8px); -webkit-backdrop-filter: blur(8px);
    display: none; align-items: center; justify-content: center;
    z-index: 1000; padding: 20px;
    animation: fade-in .18s ease;
  }
  @keyframes fade-in { from { opacity: 0 } to { opacity: 1 } }
  .modal-bg.open { display: flex; }
  .modal {
    background: var(--bg-1);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-lg);
    max-width: 480px; width: 100%;
    padding: 28px;
    color: var(--fg-0);
    box-shadow: var(--shadow-lg);
    animation: modal-in .22s cubic-bezier(.2,.9,.3,1.1);
    position: relative;
  }
  @keyframes modal-in {
    from { opacity: 0; transform: scale(.94) translateY(8px); }
    to { opacity: 1; transform: scale(1) translateY(0); }
  }
  .modal .brand-row {
    display: flex; align-items: center; gap: 10px; margin-bottom: 6px;
  }
  .modal .brand-mark-lg {
    width: 36px; height: 36px; border-radius: 9px;
    background: var(--accent);
    display: inline-flex; align-items: center; justify-content: center;
    color: #fff; font-weight: 700; font-size: 16px;
    box-shadow: 0 0 0 1px var(--accent-ring), 0 4px 12px rgba(91,141,239,.25);
  }
  .modal h2 {
    margin: 0; font-size: 18px; font-weight: 600;
    letter-spacing: -0.02em; color: var(--fg-0);
  }
  .modal .sub {
    color: var(--fg-2); font-size: 12.5px; margin-top: 2px;
  }
  .modal .body-text {
    margin: 18px 0 0; line-height: 1.65; font-size: 13px;
    color: var(--fg-1);
  }
  .modal .body-text + .body-text { margin-top: 10px; }
  .modal code {
    font-family: 'JetBrains Mono', monospace; font-size: 11.5px;
    background: var(--bg-3); color: var(--fg-0);
    padding: 1px 6px; border-radius: 4px;
    border: 1px solid var(--border);
  }
  .modal strong { color: var(--fg-0); font-weight: 600; }
  .modal .links {
    display: flex; gap: 8px; flex-wrap: wrap;
    margin-top: 20px;
  }
  .modal .links a {
    display: inline-flex; align-items: center; gap: 6px;
    background: var(--bg-2); color: var(--fg-1);
    padding: 8px 12px; border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    font-size: 12.5px; text-decoration: none; font-weight: 500;
    transition: background .12s ease, color .12s ease, border-color .12s ease;
  }
  .modal .links a:hover {
    background: var(--bg-hover); color: var(--fg-0);
    border-color: var(--border-strong);
  }
  .modal .links a::after {
    content: ''; width: 10px; height: 10px;
    background: currentColor; opacity: .5;
    -webkit-mask: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='black' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'><path d='M7 17 17 7'/><path d='M7 7h10v10'/></svg>") center/contain no-repeat;
    mask: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='black' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'><path d='M7 17 17 7'/><path d='M7 7h10v10'/></svg>") center/contain no-repeat;
  }
  .modal .close-row {
    display: flex; justify-content: flex-end;
    margin-top: 24px;
  }

  /* Scrollbars */
  ::-webkit-scrollbar { width: 10px; height: 10px; }
  ::-webkit-scrollbar-track { background: transparent; }
  ::-webkit-scrollbar-thumb {
    background: var(--border-strong); border-radius: 100px;
    border: 2px solid var(--bg-1);
  }
  ::-webkit-scrollbar-thumb:hover { background: var(--fg-3); }
  ::-webkit-scrollbar-corner { background: transparent; }
</style>
</head>
<body>
<div class="app">

  <div class="toolbar">
    <div class="brand">
      <span class="brand-mark">CR</span>
      <span>C-Road</span>
    </div>
    <div class="divider"></div>

    <button class="btn ghost" id="btn-open-file">Open File</button>
    <button class="btn ghost" id="btn-open-folder">Open Folder</button>
    <button class="btn ghost" id="btn-save">Save</button>

    <div class="divider"></div>

    <button class="btn primary" id="btn-run">Run<span class="kbd">F5</span></button>
    <button class="btn danger" id="btn-stop">Stop</button>

    <div class="divider"></div>

    <button class="btn ghost" id="btn-clear">Clear</button>

    <div class="spacer"></div>

    <div class="file-chip" id="file-name">untitled.c</div>
    <div class="divider"></div>
    <button class="btn ghost" id="btn-theme">Theme</button>
    <button class="btn ghost" id="btn-about">About</button>
  </div>

  <div class="main">
    <div class="sidebar" id="sidebar">
      <div class="sidebar-header">Explorer</div>
      <div class="file-tree" id="file-tree">
        <div class="empty">No folder open.<br>Click <strong>Open Folder</strong> to browse files.</div>
      </div>
    </div>

    <div class="splitter-v" id="split-v"></div>

    <div class="editor-area">
      <div class="editor-wrap"><div id="editor"></div></div>
      <div class="splitter-h" id="split-h"></div>

      <div class="console" id="console">
        <div class="console-tabs">
          <div class="console-tab active" data-tab="output">
            Output
            <span class="badge" id="tab-output-badge" style="display:none"></span>
          </div>
          <div class="console-tab" data-tab="problems">
            Problems
            <span class="badge" id="tab-problems-badge" style="display:none"></span>
          </div>
          <div class="console-header-spacer"></div>
          <div class="console-status ready" id="status">Ready</div>
        </div>

        <div class="pane" id="pane-output">
          <div class="console-body" id="console-body"></div>
        </div>

        <div class="pane hidden" id="pane-problems">
          <div class="diagnostics-list" id="diagnostics"></div>
        </div>

        <div class="console-input">
          <span class="prompt-mark">&#8250;</span>
          <input id="stdin-input" placeholder="Type input for your program and press Enter" autocomplete="off" spellcheck="false">
          <button class="btn solid" id="stdin-send">Send</button>
        </div>
      </div>
    </div>
  </div>
</div>

<div class="modal-bg" id="about-modal">
  <div class="modal">
    <div class="brand-row">
      <span class="brand-mark-lg">CR</span>
      <div>
        <h2>C-Road Editor</h2>
        <div class="sub">A single-binary C editor for the browser</div>
      </div>
    </div>
    <p class="body-text">
      C-Road is a lightweight C development environment that runs entirely in
      your browser. Write C, hit <strong>Run</strong>, and it compiles with
      <code>gcc</code> and streams the output back live. It supports interactive
      stdin, real autocomplete, and inline diagnostics with fix suggestions.
    </p>
    <p class="body-text">
      Built by <strong>Nour Yahyaoui</strong> as a small, self-contained tool
      that doesn't require a full IDE or a heavyweight runtime. The whole
      server ships as one binary.
    </p>
    <div class="links">
      <a href="https://github.com/Nour-yahyaoui" target="_blank" rel="noopener">GitHub</a>
      <a href="https://github.com/Nour-yahyaoui/c-editor" target="_blank" rel="noopener">Repository</a>
      <a href="https://github.com/Nour-yahyaoui/c-editor/releases/latest" target="_blank" rel="noopener">Download</a>
    </div>
    <div class="close-row">
      <button class="btn solid" id="about-close">Close</button>
    </div>
  </div>
</div>

<script src="https://cdn.jsdelivr.net/npm/codemirror@5.65.16/lib/codemirror.js"></script>
<script src="https://cdn.jsdelivr.net/npm/codemirror@5.65.16/mode/clike/clike.js"></script>
<script src="https://cdn.jsdelivr.net/npm/codemirror@5.65.16/addon/edit/closebrackets.js"></script>
<script src="https://cdn.jsdelivr.net/npm/codemirror@5.65.16/addon/edit/matchbrackets.js"></script>
<script src="https://cdn.jsdelivr.net/npm/codemirror@5.65.16/addon/hint/show-hint.js"></script>
<script src="https://cdn.jsdelivr.net/npm/codemirror@5.65.16/addon/comment/comment.js"></script>
<script src="https://cdn.jsdelivr.net/npm/codemirror@5.65.16/addon/search/searchcursor.js"></script>
<script src="https://cdn.jsdelivr.net/npm/codemirror@5.65.16/addon/fold/foldcode.js"></script>
<script src="https://cdn.jsdelivr.net/npm/codemirror@5.65.16/addon/fold/foldgutter.js"></script>
<script src="https://cdn.jsdelivr.net/npm/codemirror@5.65.16/addon/fold/brace-fold.js"></script>
<script src="https://cdn.jsdelivr.net/npm/codemirror@5.65.16/addon/fold/comment-fold.js"></script>
<script src="/c_data.js"></script>

<script>
/* ============================================================
   Theme
   ============================================================ */
const THEME_KEY = "croad.theme";
let currentTheme = localStorage.getItem(THEME_KEY) || "dark";

function applyTheme(name) {
  document.documentElement.classList.toggle("light", name === "light");
  editor.setOption("theme", name === "light" ? "default" : "material-darker");
  document.getElementById("btn-theme").textContent =
    name === "light" ? "Light" : "Dark";
}

/* ============================================================
   Editor
   ============================================================ */
const DEFAULT_CODE = `#include <stdio.h>

int main(void) {
    printf("Hello from C-Road!\\n");
    return 0;
}
`;

const editor = CodeMirror(document.getElementById("editor"), {
  value: localStorage.getItem("croad.code") || DEFAULT_CODE,
  mode: "text/x-csrc",
  theme: "material-darker",
  lineNumbers: true,
  indentUnit: 4,
  tabSize: 4,
  indentWithTabs: false,
  autoCloseBrackets: true,
  matchBrackets: true,
  styleActiveLine: true,
  foldGutter: true,
  gutters: ["CodeMirror-linenumbers", "CodeMirror-foldgutter"],
  extraKeys: {
    "F5": run,
    "Ctrl-Space": "autocomplete",
    "Ctrl-/": "toggleComment",
    "Ctrl-D": duplicateLine,
    "Shift-Ctrl-K": deleteLine,
    "Tab": cm => cm.replaceSelection("    ", "end"),
  },
});

let dirty = false;
editor.on("change", () => {
  localStorage.setItem("croad.code", editor.getValue());
  if (!dirty) {
    dirty = true;
    document.getElementById("file-name").classList.add("dirty");
  }
});

function markSaved() {
  dirty = false;
  document.getElementById("file-name").classList.remove("dirty");
}

function duplicateLine(cm) {
  const cur = cm.getCursor();
  const line = cm.getLine(cur.line);
  cm.replaceRange("\n" + line, { line: cur.line, ch: line.length });
  cm.setCursor({ line: cur.line + 1, ch: cur.ch });
}
function deleteLine(cm) {
  const cur = cm.getCursor();
  cm.replaceRange("", { line: cur.line, ch: 0 }, { line: cur.line + 1, ch: 0 });
  if (cur.line >= cm.lineCount()) cm.setCursor({ line: cm.lineCount() - 1, ch: 0 });
}

/* ============================================================
   Autocomplete
   ============================================================ */
const D = window.C_ROAD_DATA;

const KIND_LABEL = {
  "hint-func": "fn",
  "hint-kw": "kw",
  "hint-type": "ty",
  "hint-macro": "mac",
  "hint-snip": "snip",
};

function cHint(cm) {
  const cur = cm.getCursor();
  const line = cm.getLine(cur.line);
  const before = line.slice(0, cur.ch);

  const incMatch = before.match(/#\s*include\s*<([A-Za-z0-9_./]*)$/);
  if (incMatch) {
    const prefix = incMatch[1].toLowerCase();
    return {
      list: D.headers.filter(h => h.toLowerCase().startsWith(prefix))
        .map(h => ({
          text: h + ">",
          displayText: h + ">",
          className: "hint-hdr",
          _kind: "hdr",
        })),
      from: CodeMirror.Pos(cur.line, cur.ch - incMatch[1].length),
      to: CodeMirror.Pos(cur.line, cur.ch),
    };
  }

  const wordMatch = before.match(/[A-Za-z_][A-Za-z0-9_]*$/);
  if (!wordMatch) return null;
  const prefix = wordMatch[0];
  const from = CodeMirror.Pos(cur.line, cur.ch - prefix.length);
  const to = CodeMirror.Pos(cur.line, cur.ch);

  const list = [];
  const seen = new Set();
  function push(text, display, cls) {
    if (seen.has(text)) return;
    seen.add(text);
    list.push({ text, displayText: display || text, className: cls });
  }

  for (const f of Object.keys(D.functions)) {
    if (f.startsWith(prefix)) {
      push(f + "(", f + "   " + D.functions[f].sig, "hint-func");
    }
  }
  for (const k of D.keywords) {
    if (k.startsWith(prefix)) push(k + " ", k, "hint-kw");
  }
  for (const t of D.types) {
    if (t.startsWith(prefix)) push(t + " ", t, "hint-type");
  }
  for (const m of D.macros) {
    if (m.startsWith(prefix)) push(m + " ", m, "hint-macro");
  }
  for (const s of Object.keys(D.snippets)) {
    if (s.startsWith(prefix)) push(D.snippets[s], s, "hint-snip");
  }

  const order = { "hint-func": 0, "hint-kw": 1, "hint-type": 2, "hint-macro": 3, "hint-snip": 4 };
  list.sort((a, b) => (order[a.className] ?? 9) - (order[b.className] ?? 9));
  if (!list.length) return null;
  return { list, from, to };
}
CodeMirror.registerHelper("hint", "c", cHint);
editor.setOption("hintOptions", { hint: cHint, completeSingle: false });

editor.on("inputRead", (cm, change) => {
  if (change.origin !== "+input") return;
  const c = change.text[0];
  if (c && /[A-Za-z_#>]/.test(c)) {
    clearTimeout(window._hintTimer);
    window._hintTimer = setTimeout(() => cm.showHint({ hint: cHint, completeSingle: false }), 100);
  }
});

/* ============================================================
   Console
   ============================================================ */
const consoleBody = document.getElementById("console-body");
const statusEl = document.getElementById("status");
const stdinInput = document.getElementById("stdin-input");
const diagnosticsEl = document.getElementById("diagnostics");
const tabOutputBadge = document.getElementById("tab-output-badge");
const tabProblemsBadge = document.getElementById("tab-problems-badge");

function setStatus(text, kind = "ready") {
  statusEl.textContent = text;
  statusEl.className = "console-status " + kind;
}

function appendConsole(text, cls = "") {
  const span = document.createElement("span");
  if (cls) span.className = cls;
  span.textContent = text;
  consoleBody.appendChild(span);
  consoleBody.scrollTop = consoleBody.scrollHeight;
}

function appendSystem(text) {
  appendConsole(text + "\n", "sys");
}

function clearConsole() {
  consoleBody.textContent = "";
}

/* ============================================================
   Tabs
   ============================================================ */
const tabs = document.querySelectorAll(".console-tab");
const panes = {
  output: document.getElementById("pane-output"),
  problems: document.getElementById("pane-problems"),
};
tabs.forEach(tab => {
  tab.onclick = () => {
    tabs.forEach(t => t.classList.remove("active"));
    tab.classList.add("active");
    const name = tab.dataset.tab;
    Object.entries(panes).forEach(([k, el]) => {
      el.classList.toggle("hidden", k !== name);
    });
  };
});
function switchTab(name) {
  tabs.forEach(t => t.classList.toggle("active", t.dataset.tab === name));
  Object.entries(panes).forEach(([k, el]) => {
    el.classList.toggle("hidden", k !== name);
  });
}

/* ============================================================
   Diagnostics
   ============================================================ */
let diagnosticLines = [];

function clearDiagnosticMarkers() {
  diagnosticLines.forEach(line => editor.removeLineClass(line, "background", "cm-error-line"));
  diagnosticLines.forEach(line => editor.removeLineClass(line, "background", "cm-warn-line"));
  diagnosticLines = [];
}

function renderDiagnostics(diags) {
  diagnosticsEl.textContent = "";
  clearDiagnosticMarkers();

  let errs = 0, warns = 0;

  if (!diags.length) {
    diagnosticsEl.innerHTML = '<div class="empty">No problems detected.</div>';
    tabProblemsBadge.style.display = "none";
    return;
  }

  for (const d of diags) {
    if (d.severity === "error") errs++;
    else if (d.severity === "warning") warns++;

    const div = document.createElement("div");
    div.className = "diag " + d.severity;

    const loc = document.createElement("span");
    loc.className = "loc";
    loc.textContent = `${d.line}:${d.col}`;

    const body = document.createElement("div");
    body.className = "body";

    const msg = document.createElement("div");
    msg.className = "msg";
    msg.textContent = d.message;
    body.appendChild(msg);

    if (d.suggestion) {
      const sg = document.createElement("div");
      sg.className = "sugg";
      sg.appendChild(document.createTextNode(d.suggestion));
      body.appendChild(sg);
    }

    div.appendChild(loc);
    div.appendChild(body);
    div.onclick = () => {
      editor.setCursor({ line: d.line - 1, ch: d.col - 1 });
      editor.focus();
    };
    diagnosticsEl.appendChild(div);

    const lineIdx = d.line - 1;
    if (d.severity === "error") {
      editor.addLineClass(lineIdx, "background", "cm-error-line");
      diagnosticLines.push(lineIdx);
    } else if (d.severity === "warning") {
      editor.addLineClass(lineIdx, "background", "cm-warn-line");
      diagnosticLines.push(lineIdx);
    }
  }

  tabProblemsBadge.style.display = "";
  tabProblemsBadge.textContent =
    errs > 0 ? `${errs}` : `${warns}`;
  tabProblemsBadge.className = "badge " + (errs > 0 ? "err" : warns > 0 ? "warn" : "");
}

async function fetchDiagnostics(code) {
  try {
    const res = await fetch("/diagnostics", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ code }),
    });
    const data = await res.json();
    renderDiagnostics(data.diagnostics || []);
  } catch (e) {}
}

let liveDiagTimer = null;
editor.on("change", () => {
  clearTimeout(liveDiagTimer);
  liveDiagTimer = setTimeout(() => fetchDiagnostics(editor.getValue()), 800);
});

/* ============================================================
   Run + WebSocket
   ============================================================ */
let ws = null;

function run() {
  clearConsole();
  setStatus("Compiling…", "busy");
  switchTab("output");
  tabOutputBadge.style.display = "none";

  const code = editor.getValue();
  fetchDiagnostics(code);

  if (ws) { try { ws.close(); } catch(e){} }
  const proto = location.protocol === "https:" ? "wss" : "ws";
  ws = new WebSocket(`${proto}://${location.host}/ws`);

  ws.onopen = () => ws.send(JSON.stringify({ code }));

  ws.onmessage = (e) => {
    const msg = JSON.parse(e.data);
    if (msg.compile_error) {
      appendConsole(msg.compile_error, "err");
      setStatus("Compile failed", "err");
      tabOutputBadge.style.display = "";
      tabOutputBadge.textContent = "err";
      tabOutputBadge.className = "badge err";
      ws.close();
    } else if (msg.event === "compiled") {
      appendSystem("Program started\n");
      setStatus("Running…", "busy");
    } else if (msg.out !== undefined) {
      appendConsole(msg.out);
    } else if (msg.err !== undefined) {
      appendConsole(msg.err, "err");
    } else if (msg.exit !== undefined) {
      appendConsole(`\nProcess exited with code ${msg.exit}\n`, "exit-code");
      setStatus(msg.exit === 0 ? "Finished" : "Failed", msg.exit === 0 ? "ok" : "err");
      tabOutputBadge.style.display = "";
      tabOutputBadge.textContent = String(msg.exit);
      tabOutputBadge.className = "badge " + (msg.exit === 0 ? "ok" : "err");
      ws.close();
    } else if (msg.error) {
      appendConsole(msg.error + "\n", "err");
      setStatus("Error", "err");
      ws.close();
    }
  };

  ws.onerror = () => {
    appendConsole("WebSocket error\n", "err");
    setStatus("Connection error", "err");
  };
  ws.onclose = () => { ws = null; };
}

function stop() {
  if (ws) { try { ws.close(); } catch(e){} ws = null; }
  setStatus("Stopped", "err");
}

function sendStdin() {
  const text = stdinInput.value;
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ stdin: text + "\n" }));
    appendConsole("› " + text + "\n", "prompt-line");
    stdinInput.value = "";
  } else {
    appendConsole("No running program to send input to.\n", "dim");
  }
}

/* ============================================================
   Splitters
   ============================================================ */
function installVSplitter(splitterId, panelId, minWidth, storageKey) {
  const splitter = document.getElementById(splitterId);
  const panel = document.getElementById(panelId);
  let dragging = false;
  splitter.addEventListener("mousedown", () => {
    dragging = true;
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
  });
  window.addEventListener("mousemove", (e) => {
    if (!dragging) return;
    const rect = panel.parentElement.getBoundingClientRect();
    const w = Math.max(minWidth, Math.min(e.clientX - rect.left, rect.width - 260));
    panel.style.width = w + "px";
    editor.refresh();
  });
  window.addEventListener("mouseup", () => {
    if (dragging) {
      dragging = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      localStorage.setItem(storageKey, panel.style.width);
      editor.refresh();
    }
  });
  const saved = localStorage.getItem(storageKey);
  if (saved) panel.style.width = saved;
}

function installHSplitter(splitterId, panelId, minHeight, storageKey) {
  const splitter = document.getElementById(splitterId);
  const panel = document.getElementById(panelId);
  let dragging = false;
  splitter.addEventListener("mousedown", () => {
    dragging = true;
    document.body.style.cursor = "row-resize";
    document.body.style.userSelect = "none";
  });
  window.addEventListener("mousemove", (e) => {
    if (!dragging) return;
    const rect = panel.parentElement.getBoundingClientRect();
    const h = Math.max(minHeight, Math.min(rect.bottom - e.clientY, rect.height - 160));
    panel.style.height = h + "px";
    editor.refresh();
  });
  window.addEventListener("mouseup", () => {
    if (dragging) {
      dragging = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      localStorage.setItem(storageKey, panel.style.height);
      editor.refresh();
    }
  });
  const saved = localStorage.getItem(storageKey);
  if (saved) panel.style.height = saved;
}

/* ============================================================
   File / Folder
   ============================================================ */
let currentFileHandle = null;

async function openFile() {
  if (window.showOpenFilePicker) {
    try {
      const [handle] = await window.showOpenFilePicker({
        types: [{
          description: "C source",
          accept: { "text/plain": [".c", ".h", ".cpp", ".cc", ".hpp", ".txt"] }
        }],
      });
      const file = await handle.getFile();
      const text = await file.text();
      editor.setValue(text);
      localStorage.setItem("croad.code", text);
      currentFileHandle = handle;
      document.getElementById("file-name").textContent = file.name;
      markSaved();
    } catch (e) {}
  } else {
    const inp = document.createElement("input");
    inp.type = "file";
    inp.accept = ".c,.h,.cpp,.cc,.hpp,.txt";
    inp.onchange = async () => {
      const file = inp.files[0];
      if (!file) return;
      const text = await file.text();
      editor.setValue(text);
      localStorage.setItem("croad.code", text);
      document.getElementById("file-name").textContent = file.name;
      markSaved();
    };
    inp.click();
  }
}

async function saveFile() {
  const code = editor.getValue();
  if (currentFileHandle) {
    try {
      const w = await currentFileHandle.createWritable();
      await w.write(code);
      await w.close();
      setStatus("Saved", "ok");
      markSaved();
      return;
    } catch (e) {}
  }
  if (window.showSaveFilePicker) {
    try {
      const handle = await window.showSaveFilePicker({
        suggestedName: "main.c",
        types: [{ description: "C source", accept: { "text/plain": [".c", ".h"] } }],
      });
      const w = await handle.createWritable();
      await w.write(code);
      await w.close();
      currentFileHandle = handle;
      const file = await handle.getFile();
      document.getElementById("file-name").textContent = file.name;
      setStatus("Saved", "ok");
      markSaved();
    } catch (e) {}
  } else {
    const blob = new Blob([code], { type: "text/plain" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = "main.c";
    a.click();
    markSaved();
  }
}

async function openFolder() {
  if (!window.showDirectoryPicker) {
    alert("Your browser does not support the File System Access API. Use Chrome or Edge.");
    return;
  }
  try {
    const dir = await window.showDirectoryPicker();
    const tree = document.getElementById("file-tree");
    tree.textContent = "";
    await renderDirectory(dir, tree, 0);
  } catch (e) {}
}

async function renderDirectory(dirHandle, container, depth) {
  const entries = [];
  for await (const [name, handle] of dirHandle.entries()) {
    if (name.startsWith(".")) continue;
    entries.push({ name, handle });
  }
  entries.sort((a, b) => {
    const aDir = a.handle.kind === "directory";
    const bDir = b.handle.kind === "directory";
    if (aDir !== bDir) return aDir ? -1 : 1;
    return a.name.localeCompare(b.name);
  });

  for (const { name, handle } of entries) {
    if (handle.kind === "directory") {
      const wrap = document.createElement("div");
      const row = document.createElement("div");
      row.className = "item folder indent-" + Math.min(depth, 3);
      row.innerHTML = '<span class="caret"></span><span class="icon folder"></span><span>' +
        name.replace(/[<>&]/g, c => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;' }[c])) + '</span>';
      wrap.appendChild(row);
      container.appendChild(wrap);

      const subContainer = document.createElement("div");
      subContainer.style.display = "none";
      wrap.appendChild(subContainer);

      let loaded = false;
      row.onclick = async (e) => {
        e.stopPropagation();
        const caret = row.querySelector(".caret");
        const isOpen = caret.classList.toggle("open");
        if (isOpen && !loaded) {
          await renderDirectory(handle, subContainer, depth + 1);
          loaded = true;
        }
        subContainer.style.display = isOpen ? "" : "none";
      };
    } else {
      const ext = name.split(".").pop().toLowerCase();
      if (!["c", "h", "cpp", "cc", "hpp", "txt"].includes(ext)) continue;

      const row = document.createElement("div");
      row.className = "item indent-" + Math.min(depth, 3);
      row.innerHTML = '<span class="icon file"></span><span>' +
        name.replace(/[<>&]/g, c => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;' }[c])) + '</span>';
      row.onclick = async () => {
        const file = await handle.getFile();
        const text = await file.text();
        editor.setValue(text);
        localStorage.setItem("croad.code", text);
        currentFileHandle = handle;
        document.getElementById("file-name").textContent = name;
        markSaved();
        document.querySelectorAll(".file-tree .item.active").forEach(el => el.classList.remove("active"));
        row.classList.add("active");
      };
      container.appendChild(row);
    }
  }
}

/* ============================================================
   Wiring
   ============================================================ */
document.getElementById("btn-run").onclick = run;
document.getElementById("btn-stop").onclick = stop;
document.getElementById("btn-clear").onclick = clearConsole;
document.getElementById("btn-open-file").onclick = openFile;
document.getElementById("btn-open-folder").onclick = openFolder;
document.getElementById("btn-save").onclick = saveFile;
document.getElementById("btn-theme").onclick = () => {
  currentTheme = currentTheme === "dark" ? "light" : "dark";
  localStorage.setItem(THEME_KEY, currentTheme);
  applyTheme(currentTheme);
};
document.getElementById("stdin-send").onclick = sendStdin;
stdinInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter") { e.preventDefault(); sendStdin(); }
});

const aboutModal = document.getElementById("about-modal");
document.getElementById("btn-about").onclick = () => aboutModal.classList.add("open");
document.getElementById("about-close").onclick = () => aboutModal.classList.remove("open");
aboutModal.addEventListener("click", e => {
  if (e.target === aboutModal) aboutModal.classList.remove("open");
});
document.addEventListener("keydown", (e) => {
  if (e.key === "F5") { e.preventDefault(); run(); }
  if (e.ctrlKey && e.key === "s") { e.preventDefault(); saveFile(); }
  if (e.ctrlKey && e.key === "o") { e.preventDefault(); openFile(); }
  if (e.key === "Escape") aboutModal.classList.remove("open");
});

applyTheme(currentTheme);
installVSplitter("split-v", "sidebar", 140, "croad.sidebarW");
installHSplitter("split-h", "console", 100, "croad.consoleH");
window.addEventListener("resize", () => editor.refresh());
</script>
</body>
</html>
"#;

// ============================================================================
// HTTP handlers
// ============================================================================

const C_DATA: &str = r#"window.C_ROAD_DATA = {
  keywords: ["auto","break","case","char","const","continue","default","do",
    "double","else","enum","extern","float","for","goto","if","inline","int",
    "long","register","restrict","return","short","signed","sizeof","static",
    "struct","switch","typedef","union","unsigned","void","volatile","while",
    "_Bool","_Complex","_Imaginary","_Alignas","_Alignof","_Atomic","_Generic",
    "_Noreturn","_Static_assert","_Thread_local"],
  types: ["size_t","ssize_t","ptrdiff_t","int8_t","int16_t","int32_t","int64_t",
    "uint8_t","uint16_t","uint32_t","uint64_t","bool","FILE"],
  macros: ["NULL","EOF","EXIT_SUCCESS","EXIT_FAILURE","SEEK_SET","SEEK_CUR",
    "SEEK_END","stdin","stdout","stderr","true","false","M_PI","M_E"],
  headers: ["stdio.h","stdlib.h","string.h","math.h","ctype.h","time.h",
    "stdbool.h","stdint.h","stddef.h","stdarg.h","assert.h","errno.h",
    "limits.h","float.h","signal.h","setjmp.h","locale.h","wchar.h",
    "inttypes.h","complex.h","stdatomic.h","threads.h","unistd.h","fcntl.h",
    "sys/types.h","sys/stat.h","pthread.h","dirent.h"],
  functions: {
    printf:{sig:"int printf(const char *fmt, ...)",h:"stdio.h"},
    fprintf:{sig:"int fprintf(FILE *f, const char *fmt, ...)",h:"stdio.h"},
    sprintf:{sig:"int sprintf(char *s, const char *fmt, ...)",h:"stdio.h"},
    snprintf:{sig:"int snprintf(char *s, size_t n, const char *fmt, ...)",h:"stdio.h"},
    scanf:{sig:"int scanf(const char *fmt, ...)",h:"stdio.h"},
    fscanf:{sig:"int fscanf(FILE *f, const char *fmt, ...)",h:"stdio.h"},
    sscanf:{sig:"int sscanf(const char *s, const char *fmt, ...)",h:"stdio.h"},
    fopen:{sig:"FILE *fopen(const char *path, const char *mode)",h:"stdio.h"},
    fclose:{sig:"int fclose(FILE *f)",h:"stdio.h"},
    fread:{sig:"size_t fread(void *p, size_t sz, size_t n, FILE *f)",h:"stdio.h"},
    fwrite:{sig:"size_t fwrite(const void *p, size_t sz, size_t n, FILE *f)",h:"stdio.h"},
    fgets:{sig:"char *fgets(char *s, int n, FILE *f)",h:"stdio.h"},
    fputs:{sig:"int fputs(const char *s, FILE *f)",h:"stdio.h"},
    fgetc:{sig:"int fgetc(FILE *f)",h:"stdio.h"},
    fputc:{sig:"int fputc(int c, FILE *f)",h:"stdio.h"},
    fseek:{sig:"int fseek(FILE *f, long off, int whence)",h:"stdio.h"},
    ftell:{sig:"long ftell(FILE *f)",h:"stdio.h"},
    rewind:{sig:"void rewind(FILE *f)",h:"stdio.h"},
    feof:{sig:"int feof(FILE *f)",h:"stdio.h"},
    ferror:{sig:"int ferror(FILE *f)",h:"stdio.h"},
    perror:{sig:"void perror(const char *s)",h:"stdio.h"},
    puts:{sig:"int puts(const char *s)",h:"stdio.h"},
    putchar:{sig:"int putchar(int c)",h:"stdio.h"},
    getchar:{sig:"int getchar(void)",h:"stdio.h"},
    remove:{sig:"int remove(const char *path)",h:"stdio.h"},
    rename:{sig:"int rename(const char *o, const char *n)",h:"stdio.h"},
    fflush:{sig:"int fflush(FILE *f)",h:"stdio.h"},
    setvbuf:{sig:"int setvbuf(FILE *f, char *buf, int mode, size_t sz)",h:"stdio.h"},
    malloc:{sig:"void *malloc(size_t n)",h:"stdlib.h"},
    calloc:{sig:"void *calloc(size_t n, size_t sz)",h:"stdlib.h"},
    realloc:{sig:"void *realloc(void *p, size_t n)",h:"stdlib.h"},
    free:{sig:"void free(void *p)",h:"stdlib.h"},
    exit:{sig:"void exit(int status)",h:"stdlib.h"},
    abort:{sig:"void abort(void)",h:"stdlib.h"},
    atexit:{sig:"int atexit(void (*fn)(void))",h:"stdlib.h"},
    atoi:{sig:"int atoi(const char *s)",h:"stdlib.h"},
    atol:{sig:"long atol(const char *s)",h:"stdlib.h"},
    atof:{sig:"double atof(const char *s)",h:"stdlib.h"},
    strtol:{sig:"long strtol(const char *s, char **e, int base)",h:"stdlib.h"},
    strtod:{sig:"double strtod(const char *s, char **e)",h:"stdlib.h"},
    rand:{sig:"int rand(void)",h:"stdlib.h"},
    srand:{sig:"void srand(unsigned int seed)",h:"stdlib.h"},
    qsort:{sig:"void qsort(void *b, size_t n, size_t s, int(*c)(const void*,const void*))",h:"stdlib.h"},
    bsearch:{sig:"void *bsearch(const void *k, const void *b, size_t n, size_t s, int(*c)(const void*,const void*))",h:"stdlib.h"},
    abs:{sig:"int abs(int x)",h:"stdlib.h"},
    labs:{sig:"long labs(long x)",h:"stdlib.h"},
    getenv:{sig:"char *getenv(const char *name)",h:"stdlib.h"},
    system:{sig:"int system(const char *cmd)",h:"stdlib.h"},
    strlen:{sig:"size_t strlen(const char *s)",h:"string.h"},
    strcpy:{sig:"char *strcpy(char *d, const char *s)",h:"string.h"},
    strncpy:{sig:"char *strncpy(char *d, const char *s, size_t n)",h:"string.h"},
    strcat:{sig:"char *strcat(char *d, const char *s)",h:"string.h"},
    strncat:{sig:"char *strncat(char *d, const char *s, size_t n)",h:"string.h"},
    strcmp:{sig:"int strcmp(const char *a, const char *b)",h:"string.h"},
    strncmp:{sig:"int strncmp(const char *a, const char *b, size_t n)",h:"string.h"},
    strchr:{sig:"char *strchr(const char *s, int c)",h:"string.h"},
    strrchr:{sig:"char *strrchr(const char *s, int c)",h:"string.h"},
    strstr:{sig:"char *strstr(const char *h, const char *n)",h:"string.h"},
    strtok:{sig:"char *strtok(char *s, const char *d)",h:"string.h"},
    strdup:{sig:"char *strdup(const char *s)",h:"string.h"},
    memcpy:{sig:"void *memcpy(void *d, const void *s, size_t n)",h:"string.h"},
    memmove:{sig:"void *memmove(void *d, const void *s, size_t n)",h:"string.h"},
    memset:{sig:"void *memset(void *s, int c, size_t n)",h:"string.h"},
    memcmp:{sig:"int memcmp(const void *a, const void *b, size_t n)",h:"string.h"},
    memchr:{sig:"void *memchr(const void *s, int c, size_t n)",h:"string.h"},
    sqrt:{sig:"double sqrt(double x)",h:"math.h"},
    pow:{sig:"double pow(double x, double y)",h:"math.h"},
    exp:{sig:"double exp(double x)",h:"math.h"},
    log:{sig:"double log(double x)",h:"math.h"},
    log10:{sig:"double log10(double x)",h:"math.h"},
    sin:{sig:"double sin(double x)",h:"math.h"},
    cos:{sig:"double cos(double x)",h:"math.h"},
    tan:{sig:"double tan(double x)",h:"math.h"},
    asin:{sig:"double asin(double x)",h:"math.h"},
    acos:{sig:"double acos(double x)",h:"math.h"},
    atan:{sig:"double atan(double x)",h:"math.h"},
    atan2:{sig:"double atan2(double y, double x)",h:"math.h"},
    floor:{sig:"double floor(double x)",h:"math.h"},
    ceil:{sig:"double ceil(double x)",h:"math.h"},
    round:{sig:"double round(double x)",h:"math.h"},
    fabs:{sig:"double fabs(double x)",h:"math.h"},
    fmod:{sig:"double fmod(double x, double y)",h:"math.h"},
    hypot:{sig:"double hypot(double x, double y)",h:"math.h"},
    isalpha:{sig:"int isalpha(int c)",h:"ctype.h"},
    isdigit:{sig:"int isdigit(int c)",h:"ctype.h"},
    isalnum:{sig:"int isalnum(int c)",h:"ctype.h"},
    isspace:{sig:"int isspace(int c)",h:"ctype.h"},
    isupper:{sig:"int isupper(int c)",h:"ctype.h"},
    islower:{sig:"int islower(int c)",h:"ctype.h"},
    toupper:{sig:"int toupper(int c)",h:"ctype.h"},
    tolower:{sig:"int tolower(int c)",h:"ctype.h"}
  },
  snippets: {
    main:"int main(void) {\n    \n    return 0;\n}",
    for:"for (int i = 0; i < n; i++) {\n    \n}",
    while:"while (condition) {\n    \n}",
    if:"if (condition) {\n    \n}",
    ifelse:"if (condition) {\n    \n} else {\n    \n}",
    struct:"struct Name {\n    \n};",
    typedef:"typedef struct {\n    \n} Name;"
  }
};
"#;
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
        .arg(&src).arg("-o").arg(&exe).arg("-lm")
        .output().await;

    let comp = match comp {
        Ok(c) => c,
        Err(e) => return HttpResponse::Ok().json(RunRes {
            ok: false, stdout: String::new(),
            stderr: format!("failed to run gcc: {e}"), exit: -1,
        }),
    };

    if !comp.status.success() {
        return HttpResponse::Ok().json(RunRes {
            ok: false, stdout: String::new(),
            stderr: String::from_utf8_lossy(&comp.stderr).into_owned(),
            exit: comp.status.code().unwrap_or(-1),
        });
    }

    let mut child = match Command::new(&exe)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return HttpResponse::Ok().json(RunRes {
            ok: false, stdout: String::new(),
            stderr: format!("failed to run program: {e}"), exit: -1,
        }),
    };

    if let Some(input) = &body.stdin {
        if let Some(mut si) = child.stdin.take() {
            let _ = si.write_all(input.as_bytes()).await;
            drop(si);
        }
    } else { drop(child.stdin.take()); }

    match child.wait_with_output().await {
        Ok(o) => HttpResponse::Ok().json(RunRes {
            ok: true,
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            exit: o.status.code().unwrap_or(-1),
        }),
        Err(e) => HttpResponse::Ok().json(RunRes {
            ok: false, stdout: String::new(),
            stderr: format!("wait failed: {e}"), exit: -1,
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
        .arg(&src).output().await;

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
        let rest = match line.strip_prefix("main.c:") { Some(r) => r, None => continue };
        let parts: Vec<&str> = rest.splitn(4, ':').collect();
        if parts.len() < 4 { continue; }
        let line_no: usize = match parts[0].trim().parse() { Ok(n) => n, Err(_) => continue };
        let col: usize = match parts[1].trim().parse() { Ok(n) => n, Err(_) => continue };
        let sev = parts[2].trim();
        if !(sev.contains("error") || sev.contains("warning") || sev.contains("note")) { continue; }
        let message = parts[3].trim().to_string();
        let src_line = source_lines.get(line_no.saturating_sub(1)).copied().unwrap_or("");
        let suggestion = suggest_fix(&message, src_line, source_lines);
        out.push(Diagnostic {
            line: line_no, col,
            severity: sev.replace(":", "").trim().to_string(),
            message, suggestion,
        });
    }
    out
}

fn suggest_fix(message: &str, _src_line: &str, all_lines: &[&str]) -> Option<String> {
    let quoted = extract_first_quoted(message);
    if message.contains("undeclared") {
        if let Some(name) = &quoted {
            if let Some(best) = best_match(name, all_lines) {
                return Some(format!("'{name}' is not declared. Did you mean '{best}'? Or did you forget the right #include?"));
            }
            return Some(format!("'{name}' is not declared. Check spelling or add the right #include."));
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
            return Some(format!("Function '{name}' has no prototype. Add '#include <{header}>' at the top."));
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
            if best != name { return Some(format!("Did you mean '{best}'?")); }
        }
    }
    None
}

fn extract_first_quoted(s: &str) -> Option<String> {
    let mut buf = String::new();
    let mut in_quote = false;
    for c in s.chars() {
        if c == '\'' {
            if in_quote { return Some(buf); }
            in_quote = true; buf.clear();
        } else if in_quote { buf.push(c); }
    }
    None
}

fn best_match(target: &str, lines: &[&str]) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for line in lines {
        for tok in tokenize_identifiers(line) {
            if tok == target { continue; }
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
        if c.is_alphanumeric() || c == '_' { cur.push(c); }
        else if !cur.is_empty() { out.push(std::mem::take(&mut cur)); }
    }
    if !cur.is_empty() { out.push(cur); }
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
                        Err(e) => { let _ = session.text(format!("{{\"error\":\"{e}\"}}")).await; continue; }
                    };
                    let dir = tempfile::tempdir().unwrap();
                    let src = dir.path().join("main.c");
                    let exe = dir.path().join("program");
                    std::fs::write(&src, &req.code).ok();

                    let comp = Command::new("gcc")
                        .args(["-Wall", "-Wextra", "-std=c11", "-g", "-O0", "-fdiagnostics-color=never"])
                        .arg(&src).arg("-o").arg(&exe).arg("-lm")
                        .output().await;

                    match comp {
                        Ok(c) if c.status.success() => {
                            let _ = session.text("{\"event\":\"compiled\"}").await;
                            let mut child = match Command::new(&exe)
                                .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
                                .spawn()
                            {
                                Ok(c) => c,
                                Err(e) => { let _ = session.text(format!("{{\"error\":\"{e}\"}}")).await; continue; }
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
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("c-road {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("C-Road Editor — single-binary C editor for the browser");
        println!();
        println!("USAGE:  c-road [OPTIONS]");
        println!();
        println!("OPTIONS:");
        println!("  -h, --help       Show this help");
        println!("  -V, --version    Show version");
        println!();
        println!("ENVIRONMENT:");
        println!("  CROAD_HOST   Bind address (default 127.0.0.1)");
        println!("  CROAD_PORT   Bind port    (default 8080)");
        println!();
        println!("Requires gcc on PATH to compile submitted C code.");
        return Ok(());
    }

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