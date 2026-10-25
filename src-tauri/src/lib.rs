use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{atomic::{AtomicBool, Ordering}, mpsc, Arc, Mutex},
    thread,
    time::Duration,
};
use tauri::{AppHandle, Emitter, State};

#[derive(Default)]
struct JobRegistry(Mutex<HashMap<String, Arc<AtomicBool>>>);

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MediaFile {
    path: String,
    name: String,
    kind: String,
    size: u64,
    width: Option<u32>,
    height: Option<u32>,
    duration: Option<f64>,
    format: Option<String>,
    has_audio: Option<bool>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Operation {
    id: String,
    kind: String,
    enabled: bool,
    width: Option<u32>,
    height: Option<u32>,
    fit: Option<String>,
    format: Option<String>,
    quality: Option<u8>,
    start: Option<f64>,
    end: Option<f64>,
    degrees: Option<u16>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProcessRequest {
    job_id: String,
    files: Vec<MediaFile>,
    operations: Vec<Operation>,
    output_dir: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobProgress {
    job_id: String,
    index: usize,
    total: usize,
    path: String,
    status: String,
    file_progress: Option<f32>,
    output_path: Option<String>,
    output_size: Option<u64>,
    error: Option<String>,
}

#[derive(Serialize)]
struct ToolStatus { ffmpeg: bool, ffprobe: bool, imagemagick: bool }

#[derive(Serialize)]
struct ProcessSummary { completed: usize, failed: usize }

#[derive(Serialize)]
struct PreviewData { bytes: Vec<u8>, mime: String }

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PresetPayload {
    #[serde(default)]
    id: String,
    name: String,
    description: String,
    operations: Vec<Operation>,
    #[serde(default)]
    builtin: bool,
}

fn validate_operations(operations: &[Operation]) -> Result<(), String> {
    let enabled: Vec<_> = operations.iter().filter(|operation| operation.enabled).collect();
    if enabled.is_empty() { return Err("Enable at least one processing step".into()); }
    if enabled.len() > 20 { return Err("A recipe can contain at most 20 enabled steps".into()); }
    if enabled.iter().filter(|operation| operation.kind == "format").count() > 1 { return Err("Use only one output-format step".into()); }
    for operation in enabled {
        match operation.kind.as_str() {
            "resize" if !matches!((operation.width, operation.height), (Some(1..=16_384), Some(1..=16_384))) => return Err("Resize dimensions must be between 1 and 16,384 pixels".into()),
            "resize" if !matches!(operation.fit.as_deref(), Some("contain" | "cover" | "exact")) => return Err("Resize fit must be contain, cover, or exact".into()),
            "quality" if !matches!(operation.quality, Some(1..=100)) => return Err("Quality must be between 1 and 100".into()),
            "trim" if operation.start.unwrap_or(0.0) < 0.0 || operation.end.is_none() || operation.end.unwrap_or(0.0) <= operation.start.unwrap_or(0.0) => return Err("Trim must end after its non-negative start time".into()),
            "rotate" if !matches!(operation.degrees, Some(90 | 180 | 270)) => return Err("Rotation must be 90, 180, or 270 degrees".into()),
            "format" if !matches!(operation.format.as_deref(), Some("original" | "jpg" | "png" | "webp" | "avif" | "mp4" | "webm")) => return Err("The requested output format is unsupported".into()),
            "resize" | "format" | "quality" | "stripMetadata" | "trim" | "removeAudio" | "rotate" => {}
            _ => return Err(format!("Unsupported operation: {}", operation.kind)),
        }
    }
    Ok(())
}

fn command_exists(name: &str) -> bool {
    Command::new(name).arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

fn binary_on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|directory| directory.join(name).is_file()))
}

#[tauri::command]
fn tool_status() -> ToolStatus {
    ToolStatus { ffmpeg: command_exists("ffmpeg"), ffprobe: command_exists("ffprobe"), imagemagick: command_exists("magick") }
}

fn extension_kind(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_string_lossy().to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" | "png" | "webp" | "avif" | "heic" | "heif" | "gif" | "tif" | "tiff" => Some("image"),
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v" => Some("video"),
        _ => None,
    }
}

fn inspect_one(raw: &str) -> Result<Option<MediaFile>, String> {
    let path = fs::canonicalize(raw).map_err(|e| format!("Cannot open {raw}: {e}"))?;
    if !path.is_file() { return Ok(None); }
    let Some(kind) = extension_kind(&path) else { return Ok(None); };
    let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let mut item = MediaFile { path: path.to_string_lossy().into_owned(), name, kind: kind.into(), size: metadata.len(), width: None, height: None, duration: None, format: None, has_audio: None };
    if kind == "image" {
        let output = Command::new("magick").args(["identify", "-format", "%w\n%h\n%m", path.to_string_lossy().as_ref()]).output().map_err(|e| format!("ImageMagick is unavailable: {e}"))?;
        if !output.status.success() { return Err(format!("Could not inspect {}", item.name)); }
        let values: Vec<_> = String::from_utf8_lossy(&output.stdout).lines().map(str::to_owned).collect();
        item.width = values.first().and_then(|v| v.parse().ok());
        item.height = values.get(1).and_then(|v| v.parse().ok());
        item.format = values.get(2).cloned();
    } else {
        let output = Command::new("ffprobe").args(["-v", "error", "-show_entries", "format=duration,format_name:stream=codec_type,width,height", "-of", "json", path.to_string_lossy().as_ref()]).output().map_err(|e| format!("ffprobe is unavailable: {e}"))?;
        if !output.status.success() { return Err(format!("Could not inspect {}", item.name)); }
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
        if let Some(streams) = json["streams"].as_array() {
            item.has_audio = Some(streams.iter().any(|s| s["codec_type"] == "audio"));
            if let Some(video) = streams.iter().find(|s| s["codec_type"] == "video") {
                item.width = video["width"].as_u64().map(|v| v as u32);
                item.height = video["height"].as_u64().map(|v| v as u32);
            }
        }
        item.duration = json["format"]["duration"].as_str().and_then(|v| v.parse().ok());
        item.format = json["format"]["format_name"].as_str().map(|v| v.split(',').next().unwrap_or(v).to_ascii_uppercase());
    }
    Ok(Some(item))
}

#[tauri::command]
fn inspect_paths(paths: Vec<String>) -> Result<Vec<MediaFile>, String> {
    let mut result = Vec::new();
    for raw in paths {
        let path = PathBuf::from(&raw);
        if path.is_dir() {
            let entries = fs::read_dir(&path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
            for entry in entries.flatten().take(1_000) {
                if let Some(item) = inspect_one(entry.path().to_string_lossy().as_ref())? { result.push(item); }
            }
        } else if let Some(item) = inspect_one(&raw)? { result.push(item); }
    }
    Ok(result)
}

#[tauri::command]
fn read_preview(path: String) -> Result<PreviewData, String> {
    let canonical = fs::canonicalize(path).map_err(|e| e.to_string())?;
    if extension_kind(&canonical) != Some("image") { return Err("Only supported images can be previewed".into()); }
    let metadata = fs::metadata(&canonical).map_err(|e| e.to_string())?;
    if metadata.len() > 40 * 1024 * 1024 { return Err("This image is too large for the inline preview".into()); }
    let extension = canonical.extension().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    let mime = match extension.as_str() { "jpg" | "jpeg" => "image/jpeg", "png" => "image/png", "webp" => "image/webp", "avif" => "image/avif", "gif" => "image/gif", "tif" | "tiff" => "image/tiff", _ => return Err("This image needs conversion before the webview can preview it".into()) };
    Ok(PreviewData { bytes: fs::read(canonical).map_err(|e| e.to_string())?, mime: mime.into() })
}

fn append_image_operations(command: &mut Command, operations: &[Operation]) {
    for operation in operations.iter().filter(|operation| operation.enabled) {
        match operation.kind.as_str() {
            "resize" => {
                let (width, height) = (operation.width.unwrap_or(2048), operation.height.unwrap_or(2048));
                match operation.fit.as_deref() {
                    Some("cover") => { command.args(["-resize", &format!("{width}x{height}^"), "-gravity", "center", "-extent", &format!("{width}x{height}")]); }
                    Some("exact") => { command.args(["-resize", &format!("{width}x{height}!")]); }
                    _ => { command.args(["-resize", &format!("{width}x{height}>")]); }
                }
            }
            "rotate" => { command.args(["-rotate", &operation.degrees.unwrap_or(90).to_string()]); }
            "stripMetadata" => { command.arg("-strip"); }
            "quality" => { command.args(["-quality", &operation.quality.unwrap_or(82).to_string()]); }
            _ => {}
        }
    }
}

fn video_filters(operations: &[Operation]) -> Vec<String> {
    let mut filters = Vec::new();
    for operation in operations.iter().filter(|operation| operation.enabled) {
        match operation.kind.as_str() {
            "resize" => {
                let (width, height) = (operation.width.unwrap_or(1280), operation.height.unwrap_or(720));
                filters.push(match operation.fit.as_deref() {
                    Some("cover") => format!("scale={width}:{height}:force_original_aspect_ratio=increase,crop={width}:{height}"),
                    Some("exact") => format!("scale={width}:{height}"),
                    _ => format!("scale={width}:{height}:force_original_aspect_ratio=decrease"),
                });
            }
            "rotate" => filters.push(match operation.degrees.unwrap_or(90) { 180 => "hflip,vflip".into(), 270 => "transpose=2".into(), _ => "transpose=1".into() }),
            _ => {}
        }
    }
    filters
}

#[tauri::command]
fn render_preview(path: String, operations: Vec<Operation>) -> Result<PreviewData, String> {
    if !operations.is_empty() { validate_operations(&operations)?; }
    let canonical = fs::canonicalize(path).map_err(|e| e.to_string())?;
    let kind = extension_kind(&canonical).ok_or("Only supported media can be previewed")?;
    let output = if kind == "image" {
        let mut command = Command::new("magick");
        command.arg(&canonical).arg("-auto-orient");
        append_image_operations(&mut command, &operations);
        command.args(["-thumbnail", "1400x900>", "-strip", "jpeg:-"]).output().map_err(|e| format!("Could not render preview: {e}"))?
    } else {
        let start = format_operation(&operations, "trim").and_then(|operation| operation.start).unwrap_or(0.0);
        let mut filters = video_filters(&operations);
        filters.push("scale=1400:900:force_original_aspect_ratio=decrease".into());
        Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-ss", &start.to_string(), "-i", canonical.to_string_lossy().as_ref(), "-vf", &filters.join(","), "-frames:v", "1", "-f", "image2pipe", "-vcodec", "mjpeg", "pipe:1"]).output().map_err(|e| format!("Could not render video preview: {e}"))?
    };
    if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()); }
    if output.stdout.len() > 12 * 1024 * 1024 { return Err("Rendered preview exceeded the 12 MiB limit".into()); }
    Ok(PreviewData { bytes: output.stdout, mime: "image/jpeg".into() })
}

fn picker(args: &[&str]) -> Result<Vec<String>, String> {
    let (program, actual): (&str, Vec<&str>) = if binary_on_path("kdialog") {
        ("kdialog", args.to_vec())
    } else {
        let zenity_args = if args.contains(&"--getexistingdirectory") { vec!["--file-selection", "--directory"] } else { vec!["--file-selection", "--multiple", "--separator=\n"] };
        ("zenity", zenity_args)
    };
    let output = Command::new(program).args(actual).output().map_err(|e| format!("Could not open the file picker: {e}"))?;
    if !output.status.success() { return Ok(Vec::new()); }
    Ok(String::from_utf8_lossy(&output.stdout).lines().map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned).collect())
}

#[tauri::command]
fn pick_files() -> Result<Vec<String>, String> { picker(&["--getopenfilename", ".", "Images and videos (*.jpg *.jpeg *.png *.webp *.avif *.heic *.heif *.gif *.tif *.tiff *.mp4 *.mov *.mkv *.webm *.avi *.m4v)", "--multiple", "--separate-output"]) }

#[tauri::command]
fn pick_output_directory() -> Result<Option<String>, String> { Ok(picker(&["--getexistingdirectory", "."])?.into_iter().next()) }

fn json_picker(save: bool) -> Result<Option<PathBuf>, String> {
    let output = if binary_on_path("kdialog") {
        let mode = if save { "--getsavefilename" } else { "--getopenfilename" };
        Command::new("kdialog").args([mode, ".", "Forge presets (*.json)"]).output()
    } else if binary_on_path("zenity") {
        let mut command = Command::new("zenity"); command.arg("--file-selection");
        if save { command.args(["--save", "--confirm-overwrite", "--filename=forge-recipe.forge.json"]); }
        else { command.arg("--file-filter=Forge presets | *.json"); }
        command.output()
    } else { return Err("Install kdialog or zenity to choose preset files".into()); };
    let output = output.map_err(|e| format!("Could not open the preset picker: {e}"))?;
    if !output.status.success() { return Ok(None); }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok((!value.is_empty()).then(|| PathBuf::from(value)))
}

fn validate_preset(preset: &PresetPayload) -> Result<(), String> {
    if preset.name.trim().is_empty() || preset.name.chars().count() > 80 { return Err("Preset names must contain 1–80 characters".into()); }
    if preset.description.chars().count() > 240 { return Err("Preset descriptions can contain at most 240 characters".into()); }
    validate_operations(&preset.operations)
}

#[tauri::command]
fn export_preset(mut preset: PresetPayload) -> Result<bool, String> {
    validate_preset(&preset)?;
    let Some(mut path) = json_picker(true)? else { return Ok(false); };
    if path.extension().and_then(|value| value.to_str()) != Some("json") { path.set_extension("forge.json"); }
    preset.builtin = false;
    let bytes = serde_json::to_vec_pretty(&preset).map_err(|e| e.to_string())?;
    fs::write(path, bytes).map_err(|e| format!("Could not export preset: {e}"))?;
    Ok(true)
}

#[tauri::command]
fn import_preset() -> Result<Option<PresetPayload>, String> {
    let Some(path) = json_picker(false)? else { return Ok(None); };
    let metadata = fs::metadata(&path).map_err(|e| format!("Could not read preset: {e}"))?;
    if metadata.len() > 1024 * 1024 { return Err("Preset files cannot exceed 1 MiB".into()); }
    let mut preset: PresetPayload = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| format!("Invalid Forge preset: {e}"))?;
    validate_preset(&preset)?;
    preset.builtin = false;
    Ok(Some(preset))
}

fn data_file() -> Result<PathBuf, String> {
    let directory = dirs::data_local_dir().ok_or("The application data directory is unavailable")?.join("forge");
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    Ok(directory.join("state.json"))
}

#[tauri::command]
fn load_state() -> Result<serde_json::Value, String> {
    let path = data_file()?;
    if !path.exists() { return Ok(serde_json::json!({ "presets": [], "history": [] })); }
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| format!("Stored Forge data is invalid: {e}"))
}

#[tauri::command]
fn save_state(persisted: serde_json::Value) -> Result<(), String> {
    let path = data_file()?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(&persisted).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::rename(temporary, path).map_err(|e| e.to_string())
}

fn format_operation<'a>(operations: &'a [Operation], kind: &str) -> Option<&'a Operation> { operations.iter().find(|op| op.enabled && op.kind == kind) }

fn target_extension(file: &MediaFile, operations: &[Operation]) -> String {
    let requested = format_operation(operations, "format").and_then(|op| op.format.as_deref()).filter(|v| *v != "original");
    match (file.kind.as_str(), requested) {
        ("image", Some("jpg" | "png" | "webp" | "avif")) => requested.unwrap().into(),
        ("video", Some("mp4" | "webm")) => requested.unwrap().into(),
        ("video", _) => "mp4".into(),
        _ => Path::new(&file.path).extension().unwrap_or_default().to_string_lossy().to_ascii_lowercase(),
    }
}

fn collision_safe_path(directory: &Path, stem: &str, extension: &str) -> PathBuf {
    let base = directory.join(format!("{stem}-forged.{extension}"));
    if !base.exists() { return base; }
    for index in 2..100_000 {
        let candidate = directory.join(format!("{stem}-forged-{index}.{extension}"));
        if !candidate.exists() { return candidate; }
    }
    directory.join(format!("{stem}-forged-overflow.{extension}"))
}

fn wait_for_child(child: &mut Child, cancelled: &AtomicBool) -> Result<(), String> {
    loop {
        if cancelled.load(Ordering::Relaxed) {
            let _ = child.kill(); let _ = child.wait();
            return Err("Job cancelled".into());
        }
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) if status.success() => return Ok(()),
            Some(status) => return Err(format!("Processor exited with {status}")),
            None => thread::sleep(Duration::from_millis(120)),
        }
    }
}

fn wait_for_ffmpeg(child: &mut Child, cancelled: &AtomicBool, duration: f64, mut progress: impl FnMut(f32)) -> Result<(), String> {
    let stdout = child.stdout.take().ok_or("FFmpeg progress output is unavailable")?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) { let _ = sender.send(line); }
    });
    let mut last = 0.0_f32;
    loop {
        if cancelled.load(Ordering::Relaxed) {
            let _ = child.kill(); let _ = child.wait();
            return Err("Job cancelled".into());
        }
        while let Ok(line) = receiver.try_recv() {
            if let Some(value) = line.strip_prefix("out_time_us=").and_then(|value| value.parse::<f64>().ok()) {
                let current = (value / 1_000_000.0 / duration.max(0.01)).clamp(0.0, 0.99) as f32;
                if current - last >= 0.01 { last = current; progress(current); }
            }
        }
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) if status.success() => { progress(1.0); return Ok(()); }
            Some(status) => return Err(format!("FFmpeg exited with {status}")),
            None => thread::sleep(Duration::from_millis(120)),
        }
    }
}

fn process_image(input: &Path, temporary: &Path, operations: &[Operation], cancelled: &AtomicBool) -> Result<(), String> {
    let mut command = Command::new("magick");
    command.arg(input).arg("-auto-orient");
    append_image_operations(&mut command, operations);
    command.arg(temporary).stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = command.spawn().map_err(|e| format!("Could not start ImageMagick: {e}"))?;
    wait_for_child(&mut child, cancelled)?;
    if format_operation(operations, "stripMetadata").is_some() {
        let verification = Command::new("magick").args(["identify", "-format", "%[EXIF:*]", temporary.to_string_lossy().as_ref()]).output().map_err(|e| e.to_string())?;
        if verification.status.success() && !verification.stdout.is_empty() { return Err("Metadata verification failed; the export was discarded".into()); }
    }
    Ok(())
}

fn process_video(input: &Path, temporary: &Path, extension: &str, source_duration: Option<f64>, operations: &[Operation], cancelled: &AtomicBool, progress: impl FnMut(f32)) -> Result<(), String> {
    let mut command = Command::new("ffmpeg");
    command.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]);
    if let Some(trim) = format_operation(operations, "trim") {
        command.args(["-ss", &trim.start.unwrap_or(0.0).max(0.0).to_string()]);
    }
    command.arg("-i").arg(input);
    if let Some(trim) = format_operation(operations, "trim") {
        if let (Some(start), Some(end)) = (trim.start, trim.end) {
            if end > start { command.args(["-t", &(end - start).to_string()]); }
        }
    }
    let mut filters = video_filters(operations);
    filters.push("scale=trunc(iw/2)*2:trunc(ih/2)*2".into());
    if !filters.is_empty() { command.args(["-vf", &filters.join(",")]); }
    let quality = format_operation(operations, "quality").and_then(|op| op.quality).unwrap_or(78).clamp(1, 100);
    let crf = (38.0 - f32::from(quality) * 0.22).round().clamp(16.0, 38.0) as u8;
    if extension == "webm" { command.args(["-c:v", "libvpx-vp9", "-crf", &crf.to_string(), "-b:v", "0", "-c:a", "libopus"]); }
    else { command.args(["-c:v", "libx264", "-preset", "medium", "-crf", &crf.to_string(), "-pix_fmt", "yuv420p", "-c:a", "aac", "-movflags", "+faststart"]); }
    if format_operation(operations, "removeAudio").is_some() { command.arg("-an"); }
    command.args(["-progress", "pipe:1", "-nostats"]).arg(temporary).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = command.spawn().map_err(|e| format!("Could not start FFmpeg: {e}"))?;
    let duration = format_operation(operations, "trim").and_then(|trim| match (trim.start, trim.end) { (Some(start), Some(end)) if end > start => Some(end - start), _ => None }).or(source_duration).unwrap_or(1.0);
    wait_for_ffmpeg(&mut child, cancelled, duration, progress)
}

fn available_bytes(path: &Path) -> Option<u64> {
    let output = Command::new("df").args(["--output=avail", "-B1", path.to_string_lossy().as_ref()]).output().ok()?;
    String::from_utf8_lossy(&output.stdout).lines().nth(1)?.trim().parse().ok()
}

fn run_job(app: AppHandle, request: ProcessRequest, cancelled: Arc<AtomicBool>) -> Result<ProcessSummary, String> {
    let output_dir = fs::canonicalize(&request.output_dir).map_err(|e| format!("Cannot use the output directory: {e}"))?;
    if !output_dir.is_dir() { return Err("The selected output location is not a directory".into()); }
    let required = request.files.iter().map(|file| fs::metadata(&file.path).map(|m| m.len()).unwrap_or(0)).sum::<u64>();
    if available_bytes(&output_dir).is_some_and(|available| available < required + required / 10) { return Err("There may not be enough free space for this batch".into()); }
    let total = request.files.len();
    let mut summary = ProcessSummary { completed: 0, failed: 0 };
    for (index, file) in request.files.iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) { return Err("Job cancelled".into()); }
        let input = fs::canonicalize(&file.path).map_err(|e| format!("Cannot open {}: {e}", file.name))?;
        if !input.is_file() || extension_kind(&input) != Some(file.kind.as_str()) { return Err(format!("{} is not a supported {}", file.name, file.kind)); }
        let extension = target_extension(file, &request.operations);
        let stem = input.file_stem().unwrap_or_default().to_string_lossy();
        let final_path = collision_safe_path(&output_dir, &stem, &extension);
        let temporary = output_dir.join(format!(".forge-{}-{index}.{extension}", request.job_id));
        let _ = app.emit("job-progress", JobProgress { job_id: request.job_id.clone(), index, total, path: file.path.clone(), status: "running".into(), file_progress: Some(0.0), output_path: None, output_size: None, error: None });
        let result = if file.kind == "image" { process_image(&input, &temporary, &request.operations, &cancelled) } else {
            let job_id = request.job_id.clone(); let source_path = file.path.clone(); let app_handle = app.clone();
            process_video(&input, &temporary, &extension, file.duration, &request.operations, &cancelled, move |file_progress| {
                let _ = app_handle.emit("job-progress", JobProgress { job_id: job_id.clone(), index, total, path: source_path.clone(), status: "running".into(), file_progress: Some(file_progress), output_path: None, output_size: None, error: None });
            })
        };
        match result {
            Ok(()) => match fs::rename(&temporary, &final_path) {
                Ok(()) => { summary.completed += 1; let size = fs::metadata(&final_path).map(|m| m.len()).ok(); let _ = app.emit("job-progress", JobProgress { job_id: request.job_id.clone(), index, total, path: file.path.clone(), status: "completed".into(), file_progress: Some(1.0), output_path: Some(final_path.to_string_lossy().into_owned()), output_size: size, error: None }); },
                Err(error) => { summary.failed += 1; let _ = fs::remove_file(&temporary); let message = format!("Could not finish export: {error}"); let _ = app.emit("job-progress", JobProgress { job_id: request.job_id.clone(), index, total, path: file.path.clone(), status: "failed".into(), file_progress: Some(1.0), output_path: None, output_size: None, error: Some(message) }); }
            },
            Err(error) if error == "Job cancelled" => { let _ = fs::remove_file(&temporary); let _ = app.emit("job-progress", JobProgress { job_id: request.job_id.clone(), index, total, path: file.path.clone(), status: "cancelled".into(), file_progress: None, output_path: None, output_size: None, error: None }); return Err(error); },
            Err(error) => { summary.failed += 1; let _ = fs::remove_file(&temporary); let _ = app.emit("job-progress", JobProgress { job_id: request.job_id.clone(), index, total, path: file.path.clone(), status: "failed".into(), file_progress: Some(1.0), output_path: None, output_size: None, error: Some(error) }); }
        }
    }
    Ok(summary)
}

#[tauri::command]
async fn process_job(app: AppHandle, request: ProcessRequest, jobs: State<'_, JobRegistry>) -> Result<ProcessSummary, String> {
    if request.files.is_empty() || request.files.len() > 1_000 { return Err("A batch must contain between 1 and 1,000 files".into()); }
    validate_operations(&request.operations)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    jobs.0.lock().map_err(|_| "Job registry is unavailable")?.insert(request.job_id.clone(), cancelled.clone());
    let id = request.job_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || run_job(app, request, cancelled)).await.map_err(|e| e.to_string())?;
    jobs.0.lock().map_err(|_| "Job registry is unavailable")?.remove(&id);
    result
}

#[tauri::command]
fn cancel_job(job_id: String, jobs: State<'_, JobRegistry>) -> Result<(), String> {
    if let Some(flag) = jobs.0.lock().map_err(|_| "Job registry is unavailable")?.get(&job_id) { flag.store(true, Ordering::Relaxed); }
    Ok(())
}

#[tauri::command]
fn reveal_path(path: String) -> Result<(), String> {
    let canonical = fs::canonicalize(path).map_err(|e| e.to_string())?;
    Command::new("xdg-open").arg(canonical).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(JobRegistry::default())
        .invoke_handler(tauri::generate_handler![tool_status, inspect_paths, read_preview, render_preview, pick_files, pick_output_directory, export_preset, import_preset, load_state, save_state, process_job, cancel_job, reveal_path])
        .run(tauri::generate_context!())
        .expect("error while running Forge");
}

#[cfg(test)]
mod tests {
    use super::*;
    fn operation(kind: &str) -> Operation {
        Operation { id: kind.into(), kind: kind.into(), enabled: true, width: None, height: None, fit: None, format: None, quality: None, start: None, end: None, degrees: None }
    }
    #[test]
    fn creates_collision_safe_names() {
        let dir = std::env::temp_dir().join(format!("forge-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let first = collision_safe_path(&dir, "photo", "webp");
        assert!(first.ends_with("photo-forged.webp"));
        fs::write(&first, b"x").unwrap();
        assert!(collision_safe_path(&dir, "photo", "webp").ends_with("photo-forged-2.webp"));
        fs::remove_file(first).unwrap(); fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn rejects_unknown_extensions() { assert_eq!(extension_kind(Path::new("note.txt")), None); }
    #[test]
    fn processes_a_real_image_recipe() {
        let dir = std::env::temp_dir().join(format!("forge-image-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let input = dir.join("input.jpg"); let output = dir.join("output.webp");
        assert!(Command::new("magick").args(["-size", "640x480", "gradient:#183126-#d39642", input.to_string_lossy().as_ref()]).status().unwrap().success());
        let mut resize = operation("resize"); resize.width = Some(320); resize.height = Some(320); resize.fit = Some("contain".into());
        let mut quality = operation("quality"); quality.quality = Some(75);
        let recipe = vec![resize, quality, operation("stripMetadata")];
        let preview = render_preview(input.to_string_lossy().into_owned(), recipe.clone()).unwrap();
        assert!(preview.bytes.starts_with(&[0xff, 0xd8, 0xff]));
        process_image(&input, &output, &recipe, &AtomicBool::new(false)).unwrap();
        let dimensions = Command::new("magick").args(["identify", "-format", "%wx%h", output.to_string_lossy().as_ref()]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&dimensions.stdout), "320x240");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn processes_a_real_video_recipe() {
        let dir = std::env::temp_dir().join(format!("forge-video-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let input = dir.join("input.mp4"); let output = dir.join("output.mp4");
        assert!(Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "color=c=#d39642:s=640x360:d=0.25", "-f", "lavfi", "-i", "anullsrc=r=44100:cl=stereo", "-shortest", "-c:v", "libx264", "-pix_fmt", "yuv420p", input.to_string_lossy().as_ref()]).status().unwrap().success());
        let mut resize = operation("resize"); resize.width = Some(320); resize.height = Some(180); resize.fit = Some("contain".into());
        let recipe = vec![resize, operation("removeAudio")];
        let preview = render_preview(input.to_string_lossy().into_owned(), recipe.clone()).unwrap();
        assert!(preview.bytes.starts_with(&[0xff, 0xd8, 0xff]));
        let mut progress = Vec::new();
        process_video(&input, &output, "mp4", Some(0.25), &recipe, &AtomicBool::new(false), |value| progress.push(value)).unwrap();
        assert_eq!(progress.last(), Some(&1.0));
        let probe = Command::new("ffprobe").args(["-v", "error", "-show_entries", "stream=codec_type,width,height", "-of", "json", output.to_string_lossy().as_ref()]).output().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
        let streams = json["streams"].as_array().unwrap();
        assert!(streams.iter().all(|stream| stream["codec_type"] != "audio"));
        assert_eq!(streams[0]["width"], 320); assert_eq!(streams[0]["height"], 180);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejects_unbounded_or_unknown_recipes() {
        let mut resize = operation("resize"); resize.width = Some(20_000); resize.height = Some(100); resize.fit = Some("contain".into());
        assert!(validate_operations(&[resize]).unwrap_err().contains("16,384"));
        assert!(validate_operations(&[operation("shell")]).unwrap_err().contains("Unsupported"));
    }
}
