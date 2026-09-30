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
<title>C-Road</title>
<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/codemirror@5.65.16/lib/codemirror.css">
<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/codemirror@5.65.16/addon/hint/show-hint.css">
<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/codemirror@5.65.16/addon/fold/foldgutter.css">
<style>
  :root {
    --bg:#1e1e1e;--bg2:#181818;--bg3:#252526;--fg:#d4d4d4;--fg2:#9a9a9a;
    --border:#2d2d2d;--accent:#4a9eff;--accent2:#3a7d44;
    --err:#ff6b6b;--ok:#6bcf7f;--warn:#ffcc66;--cm-bg:#1e1e1e;--cm-fg:#d4d4d4;
  }
  :root.light {
    --bg:#fff;--bg2:#f3f3f3;--bg3:#eaeaea;--fg:#1f2328;--fg2:#5a5a5a;
    --border:#d8d8d8;--accent:#0969da;--accent2:#1a7f37;
    --err:#cf222e;--ok:#1a7f37;--warn:#9a6700;--cm-bg:#fff;--cm-fg:#1f2328;
  }
  *{box-sizing:border-box}
  html,body{margin:0;height:100%;background:var(--bg);color:var(--fg);
    font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;
    overflow:hidden}
  .app{display:flex;flex-direction:column;height:100vh}
  .toolbar{display:flex;gap:6px;padding:8px 12px;background:var(--bg2);
    border-bottom:1px solid var(--border);align-items:center;font-size:13px;
    flex-shrink:0}
  .toolbar button{background:var(--bg3);color:var(--fg);border:1px solid var(--border);
    padding:6px 12px;border-radius:6px;cursor:pointer;font-weight:500;font-size:13px}
  .toolbar button:hover{background:var(--accent);color:#fff;border-color:var(--accent)}
  .toolbar .run{background:var(--accent2);border-color:var(--accent2);color:#fff}
  .toolbar .run:hover{background:#4a9d54}
  .toolbar .stop{background:#8b2e2e;border-color:#a33;color:#fff}
  .toolbar .spacer{flex:1}
  .toolbar .title{color:var(--fg2);font-size:12px}
  .main{flex:1;display:flex;min-height:0;position:relative}
  .sidebar{width:220px;min-width:120px;background:var(--bg2);
    border-right:1px solid var(--border);display:flex;flex-direction:column;overflow:hidden}
  .sidebar h3{margin:0;padding:6px 12px;font-size:11px;letter-spacing:.1em;
    color:var(--fg2);background:var(--bg3);border-bottom:1px solid var(--border);
    font-weight:600;flex-shrink:0}
  .file-tree{flex:1;overflow:auto;padding:6px;font-size:13px}
  .file-tree .item{padding:3px 6px;border-radius:4px;cursor:pointer;
    white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  .file-tree .item:hover{background:var(--bg3)}
  .file-tree .item.active{background:var(--accent);color:#fff}
  .file-tree .folder{color:var(--warn);font-weight:500}
  .file-tree .indent-1{padding-left:18px}
  .file-tree .indent-2{padding-left:32px}
  .file-tree .indent-3{padding-left:46px}
  .editor-area{flex:1;display:flex;flex-direction:column;min-width:0;min-height:0}
  .editor-wrap{flex:1;position:relative;min-height:0;overflow:hidden}
  #editor{position:absolute;inset:0}
  .CodeMirror{height:100%;font-size:14px;
    font-family:"JetBrains Mono","Fira Code",Consolas,monospace}
  .splitter-h{height:5px;background:var(--bg2);cursor:row-resize;flex-shrink:0;
    border-top:1px solid var(--border);border-bottom:1px solid var(--border)}
  .splitter-h:hover{background:var(--accent)}
  .console{height:240px;min-height:80px;background:var(--bg2);
    display:flex;flex-direction:column;flex-shrink:0}
  .console-header{display:flex;align-items:center;gap:8px;padding:4px 12px;
    background:var(--bg3);border-bottom:1px solid var(--border);
    font-size:11px;letter-spacing:.1em;color:var(--fg2);font-weight:600}
  .console-header .status{margin-left:auto;text-transform:none;
    letter-spacing:0;font-weight:400;font-size:11px}
  .console-body{flex:1;overflow:auto;padding:8px 12px;
    font-family:"JetBrains Mono","Fira Code",Consolas,monospace;font-size:13px;
    white-space:pre-wrap;word-break:break-word;background:var(--cm-bg);color:var(--cm-fg)}
  .console-body .err{color:var(--err)}
  .console-body .ok{color:var(--ok)}
  .console-body .warn{color:var(--warn)}
  .console-body .dim{color:var(--fg2)}
  .console-input{display:flex;align-items:center;gap:6px;padding:6px 12px;
    background:var(--bg3);border-top:1px solid var(--border)}
  .console-input .prompt{color:var(--accent);font-family:monospace;font-weight:600}
  .console-input input{flex:1;background:var(--bg2);color:var(--fg);
    border:1px solid var(--border);padding:5px 8px;border-radius:4px;
    font-family:monospace;font-size:13px;outline:none}
  .console-input input:focus{border-color:var(--accent)}
  .console-input button{background:var(--accent);color:#fff;border:0;
    padding:5px 12px;border-radius:4px;cursor:pointer;font-size:12px}
  .splitter-v{width:5px;background:var(--bg2);cursor:col-resize;flex-shrink:0;
    border-left:1px solid var(--border);border-right:1px solid var(--border)}
  .splitter-v:hover{background:var(--accent)}
  .diagnostics{max-height:140px;overflow:auto;background:var(--bg3);
    border-top:1px solid var(--border);font-size:12px;
    font-family:"JetBrains Mono",monospace}
  .diagnostics:empty{display:none}
  .diagnostics .diag{padding:6px 12px;border-bottom:1px solid var(--border);
    cursor:pointer;display:flex;gap:8px}
  .diagnostics .diag:hover{background:var(--bg2)}
  .diagnostics .diag .loc{color:var(--accent);flex-shrink:0;font-weight:600}
  .diagnostics .diag.error .loc{color:var(--err)}
  .diagnostics .diag.warning .loc{color:var(--warn)}
  .diagnostics .diag .msg{flex:1}
  .diagnostics .diag .sugg{color:var(--ok);margin-top:3px;font-style:italic}
  .CodeMirror-gutters{background:var(--bg2)!important;
    border-right:1px solid var(--border)!important}
  .cm-s-material-darker.CodeMirror{background:#1e1e1e;color:#d4d4d4}
  .cm-s-default.CodeMirror{background:#fff;color:#1f2328}
  .cm-error-line{background:rgba(255,107,107,.15)!important}
  .cm-warn-line{background:rgba(255,204,102,.12)!important}
  /* About modal */
  .modal-bg{position:fixed;inset:0;background:rgba(0,0,0,.6);
    display:none;align-items:center;justify-content:center;z-index:1000}
  .modal-bg.open{display:flex}
  .modal{background:var(--bg2);border:1px solid var(--border);border-radius:10px;
    max-width:520px;width:90%;padding:24px 28px;color:var(--fg);
    box-shadow:0 20px 60px rgba(0,0,0,.5)}
  .modal h2{margin:0 0 4px;font-size:20px;color:var(--accent)}
  .modal .sub{color:var(--fg2);font-size:13px;margin-bottom:16px}
  .modal p{line-height:1.6;font-size:14px;margin:0 0 12px}
  .modal a{color:var(--accent);text-decoration:none}
  .modal a:hover{text-decoration:underline}
  .modal .close{margin-top:8px;background:var(--accent);color:#fff;border:0;
    padding:8px 18px;border-radius:6px;cursor:pointer;font-size:13px}
  .modal .links{display:flex;gap:10px;flex-wrap:wrap;margin-top:12px}
  .modal .links a{background:var(--bg3);padding:6px 12px;border-radius:6px;
    border:1px solid var(--border);font-size:13px}
  .modal .links a:hover{background:var(--accent);color:#fff;border-color:var(--accent);
    text-decoration:none}
</style>
</head>
<body>
<div class="app">

  <div class="toolbar">
    <button id="btn-open-file">Open File</button>
    <button id="btn-open-folder">Open Folder</button>
    <button id="btn-save">Save</button>
    <span style="width:1px;height:20px;background:var(--border)"></span>
    <button class="run" id="btn-run">Run (F5)</button>
    <button class="stop" id="btn-stop">Stop</button>
    <button id="btn-clear">Clear</button>
    <span class="spacer"></span>
    <button id="btn-about">About</button>
    <button id="btn-theme">Theme: Dark</button>
    <span class="title" id="file-name">untitled.c</span>
  </div>

  <div class="main">
    <div class="sidebar" id="sidebar">
      <h3>EXPLORER</h3>
      <div class="file-tree" id="file-tree">
        <div style="padding:8px;color:var(--fg2);font-size:12px">
          Click "Open Folder" to browse files.
        </div>
      </div>
    </div>

    <div class="splitter-v" id="split-v"></div>

    <div class="editor-area">
      <div class="editor-wrap"><div id="editor"></div></div>
      <div class="splitter-h" id="split-h"></div>
      <div class="console" id="console">
        <div class="console-header">
          CONSOLE
          <span class="status" id="status">Ready</span>
        </div>
        <div class="diagnostics" id="diagnostics"></div>
        <div class="console-body" id="console-body"></div>
        <div class="console-input">
          <span class="prompt">$</span>
          <input id="stdin-input" placeholder="type a line of input and press Enter..." autocomplete="off">
          <button id="stdin-send">Send</button>
        </div>
      </div>
    </div>
  </div>
</div>

<!-- About modal -->
<div class="modal-bg" id="about-modal">
  <div class="modal">
    <h2>C-Road Editor</h2>
    <div class="sub">A single-binary C editor for the browser</div>
    <p>
      C-Road is a lightweight C development environment that runs entirely in
      your browser. Write C, hit Run, and it compiles with <code>gcc</code> and
      streams the output back live. It supports interactive stdin, real
      autocomplete, and inline diagnostics with fix suggestions.
    </p>
    <p>
      Built by <strong>Nour Yahyaoui</strong> as a small, self-contained tool
      that doesn't require a full IDE or a heavyweight runtime. The whole
      server ships as one binary.
    </p>
    <div class="links">
      <a href="https://github.com/Nour-yahyaoui" target="_blank" rel="noopener">GitHub Profile</a>
      <a href="https://github.com/Nour-yahyaoui" target="_blank" rel="noopener">Project Repo</a>
    </div>
    <button class="close" id="about-close">Close</button>
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
const THEME_KEY="croad.theme";
let currentTheme=localStorage.getItem(THEME_KEY)||"dark";

function applyTheme(name){
  document.documentElement.classList.toggle("light",name==="light");
  editor.setOption("theme",name==="light"?"default":"material-darker");
  document.getElementById("btn-theme").textContent=
    name==="light"?"Theme: Light":"Theme: Dark";
}

const DEFAULT_CODE=`#include <stdio.h>

int main(void) {
    printf("Hello from C-Road!\\n");
    return 0;
}
`;

const editor=CodeMirror(document.getElementById("editor"),{
  value:localStorage.getItem("croad.code")||DEFAULT_CODE,
  mode:"text/x-csrc",theme:"material-darker",lineNumbers:true,
  indentUnit:4,tabSize:4,indentWithTabs:false,autoCloseBrackets:true,
  matchBrackets:true,foldGutter:true,
  gutters:["CodeMirror-linenumbers","CodeMirror-foldgutter"],
  extraKeys:{
    "F5":run,"Ctrl-Space":"autocomplete","Ctrl-/":"toggleComment",
    "Ctrl-D":duplicateLine,"Shift-Ctrl-K":deleteLine,
    "Tab":cm=>cm.replaceSelection("    ","end"),
  },
});
editor.on("change",()=>localStorage.setItem("croad.code",editor.getValue()));

function duplicateLine(cm){
  const cur=cm.getCursor(),line=cm.getLine(cur.line);
  cm.replaceRange("\n"+line,{line:cur.line,ch:line.length});
  cm.setCursor({line:cur.line+1,ch:cur.ch});
}
function deleteLine(cm){
  const cur=cm.getCursor();
  cm.replaceRange("",{line:cur.line,ch:0},{line:cur.line+1,ch:0});
  if(cur.line>=cm.lineCount())cm.setCursor({line:cm.lineCount()-1,ch:0});
}

const D=window.C_ROAD_DATA;

function cHint(cm){
  const cur=cm.getCursor(),line=cm.getLine(cur.line),before=line.slice(0,cur.ch);
  const incMatch=before.match(/#\s*include\s*<([A-Za-z0-9_./]*)$/);
  if(incMatch){
    const prefix=incMatch[1].toLowerCase();
    return{list:D.headers.filter(h=>h.toLowerCase().startsWith(prefix))
      .map(h=>({text:h+">",displayText:h+">"})),
      from:CodeMirror.Pos(cur.line,cur.ch-incMatch[1].length),
      to:CodeMirror.Pos(cur.line,cur.ch)};
  }
  const wordMatch=before.match(/[A-Za-z_][A-Za-z0-9_]*$/);
  if(!wordMatch)return null;
  const prefix=wordMatch[0];
  const from=CodeMirror.Pos(cur.line,cur.ch-prefix.length);
  const to=CodeMirror.Pos(cur.line,cur.ch);
  const list=[],seen=new Set();
  function push(text,display,cls){
    if(seen.has(text))return;seen.add(text);
    list.push({text,displayText:display||text,className:cls});
  }
  for(const f of Object.keys(D.functions))if(f.startsWith(prefix)){
    const info=D.functions[f];push(f+"(",f+"()  -  "+info.sig,"hint-func");
  }
  for(const k of D.keywords)if(k.startsWith(prefix))push(k+" ",k+"  -  keyword","hint-kw");
  for(const t of D.types)if(t.startsWith(prefix))push(t+" ",t+"  -  type","hint-type");
  for(const m of D.macros)if(m.startsWith(prefix))push(m+" ",m+"  -  macro","hint-macro");
  for(const s of Object.keys(D.snippets))if(s.startsWith(prefix))push(D.snippets[s],s+"  -  snippet","hint-snip");
  const order={"hint-func":0,"hint-kw":1,"hint-type":2,"hint-macro":3,"hint-snip":4};
  list.sort((a,b)=>(order[a.className]??9)-(order[b.className]??9));
  if(!list.length)return null;
  return{list,from,to};
}
CodeMirror.registerHelper("hint","c",cHint);
editor.setOption("hintOptions",{hint:cHint,completeSingle:false});
editor.on("inputRead",(cm,change)=>{
  if(change.origin!=="+input")return;
  const c=change.text[0];
  if(c&&/[A-Za-z_#>]/.test(c)){
    clearTimeout(window._hintTimer);
    window._hintTimer=setTimeout(()=>cm.showHint({hint:cHint,completeSingle:false}),100);
  }
});

const consoleBody=document.getElementById("console-body");
const statusEl=document.getElementById("status");
const stdinInput=document.getElementById("stdin-input");
const diagnosticsEl=document.getElementById("diagnostics");

function setStatus(text,cls=""){statusEl.textContent=text;statusEl.className="status "+cls}
function appendConsole(text,cls=""){
  const span=document.createElement("span");
  if(cls)span.className=cls;
  span.textContent=text;
  consoleBody.appendChild(span);
  consoleBody.scrollTop=consoleBody.scrollHeight;
}
function clearConsole(){consoleBody.textContent="";diagnosticsEl.textContent="";clearDiagnosticMarkers()}

let diagnosticLines=[];
function clearDiagnosticMarkers(){
  diagnosticLines.forEach(l=>editor.removeLineClass(l,"background","cm-error-line"));
  diagnosticLines.forEach(l=>editor.removeLineClass(l,"background","cm-warn-line"));
  diagnosticLines=[];
}
function renderDiagnostics(diags){
  diagnosticsEl.textContent="";
  clearDiagnosticMarkers();
  for(const d of diags){
    const div=document.createElement("div");
    div.className="diag "+d.severity;
    const loc=document.createElement("span");
    loc.className="loc";loc.textContent=`line ${d.line}:${d.col}`;
    const msgDiv=document.createElement("div");
    msgDiv.className="msg";msgDiv.textContent=d.message;
    if(d.suggestion){
      const sg=document.createElement("div");
      sg.className="sugg";sg.textContent="Suggestion: "+d.suggestion;
      msgDiv.appendChild(sg);
    }
    div.appendChild(loc);div.appendChild(msgDiv);
    div.onclick=()=>{editor.setCursor({line:d.line-1,ch:d.col-1});editor.focus()};
    diagnosticsEl.appendChild(div);
    const lineIdx=d.line-1;
    if(d.severity==="error"){editor.addLineClass(lineIdx,"background","cm-error-line");diagnosticLines.push(lineIdx)}
    else if(d.severity==="warning"){editor.addLineClass(lineIdx,"background","cm-warn-line");diagnosticLines.push(lineIdx)}
  }
}
async function fetchDiagnostics(code){
  try{
    const res=await fetch("/diagnostics",{method:"POST",
      headers:{"Content-Type":"application/json"},body:JSON.stringify({code})});
    const data=await res.json();
    renderDiagnostics(data.diagnostics||[]);
  }catch(e){}
}
let liveDiagTimer=null;
editor.on("change",()=>{
  clearTimeout(liveDiagTimer);
  liveDiagTimer=setTimeout(()=>fetchDiagnostics(editor.getValue()),800);
});

let ws=null;
function run(){
  clearConsole();setStatus("Compiling...");
  const code=editor.getValue();
  fetchDiagnostics(code);
  if(ws){try{ws.close()}catch(e){}}
  const proto=location.protocol==="https:"?"wss":"ws";
  ws=new WebSocket(`${proto}://${location.host}/ws`);
  ws.onopen=()=>ws.send(JSON.stringify({code}));
  ws.onmessage=(e)=>{
    const msg=JSON.parse(e.data);
    if(msg.compile_error){appendConsole(msg.compile_error,"err");setStatus("Compile failed","err");ws.close()}
    else if(msg.event==="compiled"){setStatus("Running...")}
    else if(msg.out!==undefined){appendConsole(msg.out)}
    else if(msg.err!==undefined){appendConsole(msg.err,"err")}
    else if(msg.exit!==undefined){
      appendConsole(`\n[exit code ${msg.exit}]\n`,msg.exit===0?"ok":"err");
      setStatus("Done",msg.exit===0?"ok":"err");ws.close();
    }
    else if(msg.error){appendConsole(msg.error+"\n","err");setStatus("Error","err");ws.close()}
  };
  ws.onerror=()=>{appendConsole("WebSocket error\n","err");setStatus("Error","err")};
  ws.onclose=()=>{ws=null};
}
function stop(){if(ws){try{ws.close()}catch(e){}ws=null}setStatus("Stopped","err")}
function sendStdin(){
  const text=stdinInput.value;
  if(ws&&ws.readyState===WebSocket.OPEN){
    ws.send(JSON.stringify({stdin:text+"\n"}));
    appendConsole(text+"\n","dim");
    stdinInput.value="";
  } else appendConsole("(program not running)\n","dim");
}

function installVSplitter(splitterId,panelId,minWidth,storageKey){
  const splitter=document.getElementById(splitterId);
  const panel=document.getElementById(panelId);
  let dragging=false;
  splitter.addEventListener("mousedown",()=>{dragging=true;document.body.style.cursor="col-resize"});
  window.addEventListener("mousemove",(e)=>{
    if(!dragging)return;
    const rect=panel.parentElement.getBoundingClientRect();
    const w=Math.max(minWidth,Math.min(e.clientX-rect.left,rect.width-200));
    panel.style.width=w+"px";editor.refresh();
  });
  window.addEventListener("mouseup",()=>{
    if(dragging){dragging=false;document.body.style.cursor="";
      localStorage.setItem(storageKey,panel.style.width);editor.refresh()}
  });
  const saved=localStorage.getItem(storageKey);if(saved)panel.style.width=saved;
}
function installHSplitter(splitterId,panelId,minHeight,storageKey){
  const splitter=document.getElementById(splitterId);
  const panel=document.getElementById(panelId);
  let dragging=false;
  splitter.addEventListener("mousedown",()=>{dragging=true;document.body.style.cursor="row-resize"});
  window.addEventListener("mousemove",(e)=>{
    if(!dragging)return;
    const rect=panel.parentElement.getBoundingClientRect();
    const h=Math.max(minHeight,Math.min(rect.bottom-e.clientY,rect.height-150));
    panel.style.height=h+"px";editor.refresh();
  });
  window.addEventListener("mouseup",()=>{
    if(dragging){dragging=false;document.body.style.cursor="";
      localStorage.setItem(storageKey,panel.style.height);editor.refresh()}
  });
  const saved=localStorage.getItem(storageKey);if(saved)panel.style.height=saved;
}

let currentFileHandle=null;
async function openFile(){
  if(window.showOpenFilePicker){
    try{
      const [handle]=await window.showOpenFilePicker({
        types:[{description:"C source",
          accept:{"text/plain":[".c",".h",".cpp",".cc",".hpp",".txt"]}}]});
      const file=await handle.getFile();
      const text=await file.text();
      editor.setValue(text);
      localStorage.setItem("croad.code",text);
      currentFileHandle=handle;
      document.getElementById("file-name").textContent=file.name;
    }catch(e){}
  } else {
    const inp=document.createElement("input");
    inp.type="file";inp.accept=".c,.h,.cpp,.cc,.hpp,.txt";
    inp.onchange=async()=>{
      const file=inp.files[0];if(!file)return;
      const text=await file.text();
      editor.setValue(text);localStorage.setItem("croad.code",text);
      document.getElementById("file-name").textContent=file.name;
    };
    inp.click();
  }
}
async function saveFile(){
  const code=editor.getValue();
  if(currentFileHandle){
    try{
      const w=await currentFileHandle.createWritable();
      await w.write(code);await w.close();setStatus("Saved","ok");return;
    }catch(e){}
  }
  if(window.showSaveFilePicker){
    try{
      const handle=await window.showSaveFilePicker({
        suggestedName:"main.c",
        types:[{description:"C source",accept:{"text/plain":[".c",".h"]}}]});
      const w=await handle.createWritable();
      await w.write(code);await w.close();
      currentFileHandle=handle;
      const file=await handle.getFile();
      document.getElementById("file-name").textContent=file.name;
      setStatus("Saved","ok");
    }catch(e){}
  } else {
    const blob=new Blob([code],{type:"text/plain"});
    const a=document.createElement("a");
    a.href=URL.createObjectURL(blob);a.download="main.c";a.click();
  }
}
async function openFolder(){
  if(!window.showDirectoryPicker){
    alert("This browser does not support the File System Access API. Use Chrome or Edge.");
    return;
  }
  try{
    const dir=await window.showDirectoryPicker();
    const tree=document.getElementById("file-tree");
    tree.textContent="";
    await renderDirectory(dir,tree,0);
  }catch(e){}
}
async function renderDirectory(dirHandle,container,depth){
  const entries=[];
  for await(const [name,handle] of dirHandle.entries()){
    if(name.startsWith("."))continue;
    entries.push({name,handle});
  }
  entries.sort((a,b)=>{
    const aDir=a.handle.kind==="directory",bDir=b.handle.kind==="directory";
    if(aDir!==bDir)return aDir?-1:1;
    return a.name.localeCompare(b.name);
  });
  for(const {name,handle} of entries){
    const div=document.createElement("div");
    div.className="item indent-"+Math.min(depth,3);
    if(handle.kind==="directory"){
      div.textContent="[DIR] "+name;div.classList.add("folder");
      container.appendChild(div);
      let expanded=false,subContainer=null;
      div.onclick=async()=>{
        if(!expanded){
          subContainer=document.createElement("div");
          container.appendChild(subContainer);
          await renderDirectory(handle,subContainer,depth+1);
          expanded=true;
        } else subContainer.style.display=subContainer.style.display==="none"?"":"none";
      };
    } else {
      const ext=name.split(".").pop().toLowerCase();
      if(!["c","h","cpp","cc","hpp","txt"].includes(ext))continue;
      div.textContent=name;
      div.onclick=async()=>{
        const file=await handle.getFile();
        const text=await file.text();
        editor.setValue(text);localStorage.setItem("croad.code",text);
        currentFileHandle=handle;
        document.getElementById("file-name").textContent=name;
        document.querySelectorAll(".file-tree .item.active").forEach(el=>el.classList.remove("active"));
        div.classList.add("active");
      };
      container.appendChild(div);
    }
  }
}

document.getElementById("btn-run").onclick=run;
document.getElementById("btn-stop").onclick=stop;
document.getElementById("btn-clear").onclick=clearConsole;
document.getElementById("btn-open-file").onclick=openFile;
document.getElementById("btn-open-folder").onclick=openFolder;
document.getElementById("btn-save").onclick=saveFile;
document.getElementById("btn-theme").onclick=()=>{
  currentTheme=currentTheme==="dark"?"light":"dark";
  localStorage.setItem(THEME_KEY,currentTheme);applyTheme(currentTheme);
};
document.getElementById("stdin-send").onclick=sendStdin;
stdinInput.addEventListener("keydown",e=>{
  if(e.key==="Enter"){e.preventDefault();sendStdin()}
});

const aboutModal=document.getElementById("about-modal");
document.getElementById("btn-about").onclick=()=>aboutModal.classList.add("open");
document.getElementById("about-close").onclick=()=>aboutModal.classList.remove("open");
aboutModal.addEventListener("click",e=>{if(e.target===aboutModal)aboutModal.classList.remove("open")});
document.addEventListener("keydown",e=>{
  if(e.key==="F5"){e.preventDefault();run()}
  if(e.ctrlKey&&e.key==="s"){e.preventDefault();saveFile()}
  if(e.ctrlKey&&e.key==="o"){e.preventDefault();openFile()}
  if(e.key==="Escape")aboutModal.classList.remove("open");
});

applyTheme(currentTheme);
installVSplitter("split-v","sidebar",120,"croad.sidebarW");
installHSplitter("split-h","console",80,"croad.consoleH");
window.addEventListener("resize",()=>editor.refresh());
</script>
</body>
</html>
"#;

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

// ============================================================================
// HTTP handlers
// ============================================================================

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