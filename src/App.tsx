import { useEffect, useMemo, useState } from "react";
import {
  ArrowDown, ArrowUp, Check, ChevronRight, Clock3, Download, FileImage, Film,
  FolderOpen, GripVertical, HardDrive, History, Image as ImageIcon, Layers3, LoaderCircle,
  LockKeyhole, Plus, RotateCw, Save, Settings2, ShieldCheck, Sparkles, Trash2, WandSparkles, X
} from "lucide-react";
import {
  cancelJob, defaultState, inspectPaths, loadState, onFileDrop, onProgress, pickFiles,
  pickOutputDirectory, readPreview, revealPath, saveState, startJob, toolStatus
} from "./bridge";
import { BUILTIN_PRESETS } from "./presets";
import { estimateOutput, formatBytes } from "./format";
import type { HistoryEntry, JobProgress, JobStatus, MediaFile, Operation, OperationKind, PersistedState, Preset, ToolStatus } from "./types";

const makeId = () => crypto.randomUUID();

const operationDefaults: Record<OperationKind, Omit<Operation, "id" | "kind">> = {
  resize: { enabled: true, width: 2048, height: 2048, fit: "contain" },
  format: { enabled: true, format: "webp" },
  quality: { enabled: true, quality: 82 },
  stripMetadata: { enabled: true },
  trim: { enabled: true, start: 0, end: 30 },
  removeAudio: { enabled: true },
  rotate: { enabled: true, degrees: 90 }
};

const operationNames: Record<OperationKind, string> = {
  resize: "Resize", format: "Convert format", quality: "Compression", stripMetadata: "Remove metadata",
  trim: "Trim video", removeAudio: "Remove audio", rotate: "Rotate"
};

function App() {
  const [files, setFiles] = useState<MediaFile[]>([]);
  const [selectedPath, setSelectedPath] = useState<string>();
  const [operations, setOperations] = useState<Operation[]>(BUILTIN_PRESETS[1].operations.map((item) => ({ ...item })));
  const [state, setState] = useState<PersistedState>(defaultState);
  const [outputDir, setOutputDir] = useState("");
  const [tools, setTools] = useState<ToolStatus>({ ffmpeg: false, ffprobe: false, imagemagick: false });
  const [jobId, setJobId] = useState<string>();
  const [jobStatus, setJobStatus] = useState<JobStatus>("ready");
  const [progress, setProgress] = useState(0);
  const [view, setView] = useState<"workspace" | "history">("workspace");
  const [showPresets, setShowPresets] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [notice, setNotice] = useState<string>();

  useEffect(() => {
    Promise.all([loadState(), toolStatus()]).then(([stored, available]) => {
      setState({ ...stored, presets: [...BUILTIN_PRESETS, ...stored.presets.filter((preset) => !preset.builtin)] });
      setOutputDir(stored.outputDir ?? "");
      setTools(available);
    });
    let unlistenProgress: () => void = () => {};
    let unlistenDrop: () => void = () => {};
    onProgress(handleProgress).then((fn) => { unlistenProgress = fn; });
    onFileDrop(addPaths).then((fn) => { unlistenDrop = fn; });
    return () => { unlistenProgress(); unlistenDrop(); };
  }, []);

  const selected = files.find((file) => file.path === selectedPath) ?? files[0];
  const totalBefore = files.reduce((sum, file) => sum + file.size, 0);
  const totalAfter = files.reduce((sum, file) => sum + (file.outputSize ?? estimateOutput(file, operations)), 0);
  const completeCount = files.filter((file) => file.status === "completed").length;
  const requiredReady = files.every((file) => file.kind === "image" ? tools.imagemagick : tools.ffmpeg && tools.ffprobe);

  function handleProgress(event: JobProgress) {
    setFiles((current) => current.map((file) => file.path === event.path ? { ...file, status: event.status, outputPath: event.outputPath, outputSize: event.outputSize, error: event.error } : file));
    setProgress(Math.round(((event.index + (event.status === "completed" || event.status === "failed" ? 1 : 0)) / event.total) * 100));
  }

  async function addPaths(paths: string[]) {
    if (!paths.length) return;
    try {
      const incoming = await inspectPaths(paths);
      setFiles((current) => {
        const known = new Set(current.map((file) => file.path));
        return [...current, ...incoming.filter((file) => !known.has(file.path))];
      });
      setSelectedPath((current) => current ?? incoming[0]?.path);
      if (!incoming.length) setNotice("No supported image or video files were found.");
    } catch (error) { setNotice(String(error)); }
  }

  async function chooseFiles() {
    try {
      const incoming = await pickFiles();
      setFiles((current) => [...current, ...incoming.filter((file) => !current.some((old) => old.path === file.path))]);
      setSelectedPath((current) => current ?? incoming[0]?.path);
    } catch (error) { setNotice(String(error)); }
  }

  async function chooseOutput() {
    const chosen = await pickOutputDirectory();
    if (chosen) {
      setOutputDir(chosen);
      const next = { ...state, outputDir: chosen };
      setState(next);
      await saveState(next);
    }
  }

  async function runJob() {
    if (!files.length || !outputDir || !requiredReady) return;
    const id = makeId();
    const startedAt = new Date().toISOString();
    setJobId(id); setJobStatus("running"); setProgress(0); setNotice(undefined);
    setFiles((current) => current.map((file) => ({ ...file, status: "ready", outputPath: undefined, error: undefined })));
    try {
      const summary = await startJob({ jobId: id, files, operations: operations.filter((item) => item.enabled), outputDir });
      setJobStatus(summary.failed ? "failed" : "completed"); setProgress(100);
      setFiles((current) => {
        const failed = current.filter((file) => file.status === "failed");
        const entry: HistoryEntry = {
          id, startedAt, finishedAt: new Date().toISOString(), status: failed.length ? "failed" : "completed", inputCount: current.length,
          outputCount: current.filter((file) => file.status === "completed").length,
          bytesBefore: current.reduce((sum, file) => sum + file.size, 0),
          bytesAfter: current.reduce((sum, file) => sum + (file.outputSize ?? 0), 0), outputDir,
          errors: current.flatMap((file) => file.error ? [`${file.name}: ${file.error}`] : [])
        };
        setState((old) => { const next = { ...old, history: [entry, ...old.history].slice(0, 100) }; void saveState(next); return next; });
        return current;
      });
    } catch (error) {
      const cancelled = String(error).toLowerCase().includes("cancel");
      setJobStatus(cancelled ? "cancelled" : "failed"); setNotice(String(error));
    } finally { setJobId(undefined); }
  }

  function addOperation(kind: OperationKind) {
    setOperations((current) => [...current, { id: makeId(), kind, ...operationDefaults[kind] }]);
  }

  function patchOperation(id: string, patch: Partial<Operation>) {
    setOperations((current) => current.map((item) => item.id === id ? { ...item, ...patch } : item));
  }

  function moveOperation(index: number, delta: number) {
    setOperations((current) => {
      const next = [...current]; const target = index + delta;
      if (target < 0 || target >= next.length) return current;
      [next[index], next[target]] = [next[target], next[index]];
      return next;
    });
  }

  async function savePreset() {
    const name = window.prompt("Name this preset");
    if (!name?.trim()) return;
    const preset: Preset = { id: makeId(), name: name.trim(), description: `${operations.length} operation${operations.length === 1 ? "" : "s"}`, operations: operations.map((item) => ({ ...item, id: makeId() })) };
    const next = { ...state, presets: [...state.presets, preset] };
    setState(next); await saveState(next); setNotice(`Saved “${preset.name}”.`);
  }

  const summary = useMemo(() => files.length ? `${files.length} file${files.length === 1 ? "" : "s"} · ${formatBytes(totalBefore)}` : "No files yet", [files, totalBefore]);

  return <div className="app-shell" onDragEnter={() => setDragging(true)} onDragLeave={() => setDragging(false)} onDrop={() => setDragging(false)}>
    <header className="topbar">
      <div className="brand"><span className="brand-mark"><Sparkles size={20} /></span><span>FORGE</span></div>
      <nav>
        <button className={view === "workspace" ? "active" : ""} onClick={() => setView("workspace")}><Layers3 size={17} /> Workspace</button>
        <button className={view === "history" ? "active" : ""} onClick={() => setView("history")}><History size={17} /> History</button>
      </nav>
      <div className="privacy"><LockKeyhole size={15} /><span>Private by design</span><i /></div>
    </header>

    {view === "history" ? <HistoryView entries={state.history} onReveal={revealPath} /> : <>
      <main className="workspace-grid">
        <aside className="source-panel panel">
          <div className="panel-heading"><div><span className="eyebrow">SOURCE</span><h2>Your files</h2></div><button className="icon-button" onClick={chooseFiles} title="Add files"><Plus size={19} /></button></div>
          {files.length === 0 ? <button className={`drop-zone ${dragging ? "dragging" : ""}`} onClick={chooseFiles}>
            <span className="drop-icon"><Download size={24} /></span><strong>Drop files here</strong><span>Images and videos</span><em>or choose files</em>
          </button> : <>
            <div className="file-summary"><span>{summary}</span><button onClick={() => { setFiles([]); setSelectedPath(undefined); }}>Clear</button></div>
            <div className="file-list">{files.map((file) => <button key={file.path} className={`file-row ${selected?.path === file.path ? "selected" : ""}`} onClick={() => setSelectedPath(file.path)}>
              <span className={`file-kind ${file.kind}`}>{file.kind === "video" ? <Film size={18} /> : <FileImage size={18} />}</span>
              <span className="file-copy"><strong>{file.name}</strong><small>{formatBytes(file.size)}{file.width ? ` · ${file.width}×${file.height}` : ""}</small></span>
              {file.status === "completed" ? <Check className="status-ok" size={17} /> : file.status === "failed" ? <span title={file.error}><X className="status-failed" size={17} /></span> : file.status === "running" ? <LoaderCircle className="spin" size={17} /> : <ChevronRight size={16} />}
            </button>)}</div>
            <button className="add-more" onClick={chooseFiles}><Plus size={16} /> Add more files</button>
          </>}
          <div className="local-note"><ShieldCheck size={18} /><div><strong>Nothing leaves this device</strong><span>Files are processed locally with installed tools.</span></div></div>
        </aside>

        <section className="preview-panel panel">
          <div className="preview-top"><div><span className="eyebrow">PREVIEW</span><h2>{selected?.name ?? "Select a file"}</h2></div>{selected && <span className="format-chip">{selected.format ?? selected.kind}</span>}</div>
          <div className="preview-canvas">
            {selected ? <Preview file={selected} estimated={estimateOutput(selected, operations)} /> : <div className="empty-preview"><ImageIcon size={44} /><strong>Your preview appears here</strong><span>Add an image or video to begin.</span></div>}
          </div>
          {selected && <div className="compare-strip">
            <div><small>ORIGINAL</small><strong>{formatBytes(selected.size)}</strong><span>{selected.width && `${selected.width} × ${selected.height}`}</span></div>
            <div className="savings"><span>EST.</span><strong>−{Math.max(0, Math.round((1 - estimateOutput(selected, operations) / selected.size) * 100))}%</strong></div>
            <div className="align-right"><small>AFTER</small><strong>{formatBytes(selected.outputSize ?? estimateOutput(selected, operations))}</strong><span>{selected.status === "completed" ? "Processed" : "Estimate"}</span></div>
          </div>}
        </section>

        <aside className="recipe-panel panel">
          <div className="panel-heading"><div><span className="eyebrow">RECIPE</span><h2>Processing steps</h2></div><button className="icon-button" onClick={() => setShowPresets(!showPresets)} title="Presets"><WandSparkles size={18} /></button></div>
          {showPresets && <div className="preset-menu">{state.presets.map((preset) => <button key={preset.id} onClick={() => { setOperations(preset.operations.map((item) => ({ ...item, id: makeId() }))); setShowPresets(false); }}><strong>{preset.name}</strong><span>{preset.description}</span></button>)}</div>}
          <div className="steps">{operations.map((operation, index) => <OperationCard key={operation.id} operation={operation} index={index} count={operations.length} patch={(value) => patchOperation(operation.id, value)} move={(delta) => moveOperation(index, delta)} remove={() => setOperations((current) => current.filter((item) => item.id !== operation.id))} />)}</div>
          <div className="operation-picker">
            <select defaultValue="" onChange={(event) => { if (event.target.value) addOperation(event.target.value as OperationKind); event.target.value = ""; }}>
              <option value="" disabled>+ Add operation</option>{Object.entries(operationNames).map(([kind, name]) => <option key={kind} value={kind}>{name}</option>)}
            </select>
            <button onClick={savePreset}><Save size={15} /> Save preset</button>
          </div>
        </aside>
      </main>

      <footer className="export-bar">
        <div className="output-choice"><FolderOpen size={21} /><div><small>EXPORT TO</small><button onClick={chooseOutput}>{outputDir || "Choose output folder"}</button></div></div>
        <div className="batch-summary"><div><small>TOTAL INPUT</small><strong>{formatBytes(totalBefore)}</strong></div><ChevronRight size={18} /><div><small>ESTIMATED OUTPUT</small><strong>{files.length ? formatBytes(totalAfter) : "—"}</strong></div></div>
        {jobStatus === "running" ? <div className="running-block"><div><span>Forging {completeCount + 1} of {files.length}</span><strong>{progress}%</strong><div className="progress-track"><i style={{ width: `${progress}%` }} /></div></div><button className="cancel-button" onClick={() => jobId && cancelJob(jobId)}><X size={17} /> Cancel</button></div>
          : <button className="forge-button" disabled={!files.length || !outputDir || !requiredReady} onClick={runJob}><Sparkles size={18} /> Forge {files.length || ""} file{files.length === 1 ? "" : "s"}</button>}
      </footer>
    </>}
    {notice && <button className="toast" onClick={() => setNotice(undefined)}>{notice}<X size={15} /></button>}
  </div>;
}

function Preview({ file, estimated }: { file: MediaFile; estimated: number }) {
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    let active = true;
    let url: string | null = null;
    if (file.kind === "image") readPreview(file.path).then((value) => {
      if (!active || !value) return;
      url = URL.createObjectURL(new Blob([new Uint8Array(value.bytes)], { type: value.mime }));
      setSrc(url);
    }).catch(() => setSrc(null));
    return () => { active = false; if (url) URL.revokeObjectURL(url); };
  }, [file.path, file.kind]);
  if (src && file.kind === "image") return <div className="preview-image-wrap"><img src={src} alt={file.name} /><div className="before-label">BEFORE</div><div className="after-label">AFTER · {formatBytes(estimated)}</div><div className="preview-split" /></div>;
  return <div className={`abstract-preview ${file.kind}`}><div className="glow one" /><div className="glow two" />{file.kind === "video" ? <Film size={54} /> : <ImageIcon size={54} />}<strong>{file.name}</strong><span>{file.kind === "video" && file.duration ? `${Math.floor(file.duration / 60)}:${String(Math.round(file.duration % 60)).padStart(2, "0")} · ` : ""}{file.width} × {file.height}</span><div className="split-line" /></div>;
}

function OperationCard({ operation, index, count, patch, move, remove }: { operation: Operation; index: number; count: number; patch: (value: Partial<Operation>) => void; move: (delta: number) => void; remove: () => void }) {
  return <div className={`operation-card ${operation.enabled ? "" : "disabled"}`}>
    <div className="operation-head"><GripVertical size={16} /><span className="step-number">{index + 1}</span><strong>{operationNames[operation.kind]}</strong><label className="switch"><input type="checkbox" checked={operation.enabled} onChange={(e) => patch({ enabled: e.target.checked })} /><i /></label></div>
    <div className="operation-body">
      {operation.kind === "resize" && <div className="two-fields"><label>Width<input type="number" min="1" value={operation.width} onChange={(e) => patch({ width: Number(e.target.value) })} /></label><span>×</span><label>Height<input type="number" min="1" value={operation.height} onChange={(e) => patch({ height: Number(e.target.value) })} /></label><select value={operation.fit} onChange={(e) => patch({ fit: e.target.value as Operation["fit"] })}><option value="contain">Fit</option><option value="cover">Fill</option><option value="exact">Exact</option></select></div>}
      {operation.kind === "format" && <label>Output format<select value={operation.format} onChange={(e) => patch({ format: e.target.value as Operation["format"] })}><option value="original">Original</option><option value="jpg">JPEG</option><option value="png">PNG</option><option value="webp">WebP</option><option value="avif">AVIF</option><option value="mp4">MP4</option><option value="webm">WebM</option></select></label>}
      {operation.kind === "quality" && <label className="range-label"><span>Quality <b>{operation.quality}%</b></span><input type="range" min="20" max="100" value={operation.quality} onChange={(e) => patch({ quality: Number(e.target.value) })} /></label>}
      {operation.kind === "trim" && <div className="two-fields"><label>Start (s)<input type="number" min="0" value={operation.start} onChange={(e) => patch({ start: Number(e.target.value) })} /></label><label>End (s)<input type="number" min="0" value={operation.end} onChange={(e) => patch({ end: Number(e.target.value) })} /></label></div>}
      {operation.kind === "rotate" && <label>Clockwise<select value={operation.degrees} onChange={(e) => patch({ degrees: Number(e.target.value) as Operation["degrees"] })}><option value="90">90°</option><option value="180">180°</option><option value="270">270°</option></select></label>}
      {operation.kind === "stripMetadata" && <p>Removes EXIF, GPS and profile metadata from exports.</p>}
      {operation.kind === "removeAudio" && <p>Exports the video without its audio stream.</p>}
    </div>
    <div className="operation-actions"><button disabled={index === 0} onClick={() => move(-1)}><ArrowUp size={14} /></button><button disabled={index === count - 1} onClick={() => move(1)}><ArrowDown size={14} /></button><button onClick={remove}><Trash2 size={14} /></button></div>
  </div>;
}

function HistoryView({ entries, onReveal }: { entries: HistoryEntry[]; onReveal: (path: string) => void }) {
  return <main className="history-view"><div className="history-title"><span className="eyebrow">LOCAL JOB LOG</span><h1>Processing history</h1><p>Up to 100 recent jobs are stored on this device.</p></div>
    {entries.length === 0 ? <div className="history-empty"><Clock3 size={36} /><strong>No completed jobs yet</strong><span>Your export summaries will appear here.</span></div> : <div className="history-list">{entries.map((entry) => <article key={entry.id}><span className={`history-status ${entry.status}`}><Check size={18} /></span><div><strong>{entry.inputCount} file{entry.inputCount === 1 ? "" : "s"} forged</strong><span>{new Date(entry.finishedAt).toLocaleString()} · {formatBytes(entry.bytesBefore)} → {formatBytes(entry.bytesAfter)}</span></div><span className="history-saved">Saved {formatBytes(Math.max(0, entry.bytesBefore - entry.bytesAfter))}</span><button onClick={() => onReveal(entry.outputDir)}><FolderOpen size={16} /> Open</button></article>)}</div>}
  </main>;
}

export default App;
