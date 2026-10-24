use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex},
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

fn process_image(input: &Path, temporary: &Path, operations: &[Operation], cancelled: &AtomicBool) -> Result<(), String> {
    let mut command = Command::new("magick");
    command.arg(input).arg("-auto-orient");
    for operation in operations.iter().filter(|op| op.enabled) {
        match operation.kind.as_str() {
            "resize" => {
                let (width, height) = (operation.width.unwrap_or(2048).max(1), operation.height.unwrap_or(2048).max(1));
                match operation.fit.as_deref() {
                    Some("cover") => { command.args(["-resize", &format!("{width}x{height}^"), "-gravity", "center", "-extent", &format!("{width}x{height}")]); },
                    Some("exact") => { command.args(["-resize", &format!("{width}x{height}!")]); },
                    _ => { command.args(["-resize", &format!("{width}x{height}>")]); },
                }
            }
            "rotate" => { command.args(["-rotate", &operation.degrees.unwrap_or(90).to_string()]); },
            "stripMetadata" => { command.arg("-strip"); },
            "quality" => { command.args(["-quality", &operation.quality.unwrap_or(82).clamp(1, 100).to_string()]); },
            _ => {}
        }
    }
    command.arg(temporary).stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = command.spawn().map_err(|e| format!("Could not start ImageMagick: {e}"))?;
    wait_for_child(&mut child, cancelled)?;
    if format_operation(operations, "stripMetadata").is_some() {
        let verification = Command::new("magick").args(["identify", "-format", "%[EXIF:*]", temporary.to_string_lossy().as_ref()]).output().map_err(|e| e.to_string())?;
        if verification.status.success() && !verification.stdout.is_empty() { return Err("Metadata verification failed; the export was discarded".into()); }
    }
    Ok(())
}

fn process_video(input: &Path, temporary: &Path, extension: &str, operations: &[Operation], cancelled: &AtomicBool) -> Result<(), String> {
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
    let mut filters = Vec::new();
    for operation in operations.iter().filter(|op| op.enabled) {
        match operation.kind.as_str() {
            "resize" => {
                let (w, h) = (operation.width.unwrap_or(1280).max(2), operation.height.unwrap_or(720).max(2));
                filters.push(match operation.fit.as_deref() {
                    Some("cover") => format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}"),
                    Some("exact") => format!("scale={w}:{h}"),
                    _ => format!("scale={w}:{h}:force_original_aspect_ratio=decrease"),
                });
            }
            "rotate" => filters.push(match operation.degrees.unwrap_or(90) { 180 => "hflip,vflip".into(), 270 => "transpose=2".into(), _ => "transpose=1".into() }),
            _ => {}
        }
    }
    if !filters.is_empty() { command.args(["-vf", &filters.join(",")]); }
    let quality = format_operation(operations, "quality").and_then(|op| op.quality).unwrap_or(78).clamp(1, 100);
    let crf = (38.0 - f32::from(quality) * 0.22).round().clamp(16.0, 38.0) as u8;
    if extension == "webm" { command.args(["-c:v", "libvpx-vp9", "-crf", &crf.to_string(), "-b:v", "0", "-c:a", "libopus"]); }
    else { command.args(["-c:v", "libx264", "-preset", "medium", "-crf", &crf.to_string(), "-pix_fmt", "yuv420p", "-c:a", "aac", "-movflags", "+faststart"]); }
    if format_operation(operations, "removeAudio").is_some() { command.arg("-an"); }
    command.arg(temporary).stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = command.spawn().map_err(|e| format!("Could not start FFmpeg: {e}"))?;
    wait_for_child(&mut child, cancelled)
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
        let _ = app.emit("job-progress", JobProgress { job_id: request.job_id.clone(), index, total, path: file.path.clone(), status: "running".into(), output_path: None, output_size: None, error: None });
        let result = if file.kind == "image" { process_image(&input, &temporary, &request.operations, &cancelled) } else { process_video(&input, &temporary, &extension, &request.operations, &cancelled) };
        match result {
            Ok(()) => match fs::rename(&temporary, &final_path) {
                Ok(()) => { summary.completed += 1; let size = fs::metadata(&final_path).map(|m| m.len()).ok(); let _ = app.emit("job-progress", JobProgress { job_id: request.job_id.clone(), index, total, path: file.path.clone(), status: "completed".into(), output_path: Some(final_path.to_string_lossy().into_owned()), output_size: size, error: None }); },
                Err(error) => { summary.failed += 1; let _ = fs::remove_file(&temporary); let message = format!("Could not finish export: {error}"); let _ = app.emit("job-progress", JobProgress { job_id: request.job_id.clone(), index, total, path: file.path.clone(), status: "failed".into(), output_path: None, output_size: None, error: Some(message) }); }
            },
            Err(error) if error == "Job cancelled" => { let _ = fs::remove_file(&temporary); let _ = app.emit("job-progress", JobProgress { job_id: request.job_id.clone(), index, total, path: file.path.clone(), status: "cancelled".into(), output_path: None, output_size: None, error: None }); return Err(error); },
            Err(error) => { summary.failed += 1; let _ = fs::remove_file(&temporary); let _ = app.emit("job-progress", JobProgress { job_id: request.job_id.clone(), index, total, path: file.path.clone(), status: "failed".into(), output_path: None, output_size: None, error: Some(error) }); }
        }
    }
    Ok(summary)
}

#[tauri::command]
async fn process_job(app: AppHandle, request: ProcessRequest, jobs: State<'_, JobRegistry>) -> Result<ProcessSummary, String> {
    if request.files.is_empty() || request.files.len() > 1_000 { return Err("A batch must contain between 1 and 1,000 files".into()); }
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
        .invoke_handler(tauri::generate_handler![tool_status, inspect_paths, read_preview, pick_files, pick_output_directory, load_state, save_state, process_job, cancel_job, reveal_path])
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
        process_image(&input, &output, &[resize, quality, operation("stripMetadata")], &AtomicBool::new(false)).unwrap();
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
        process_video(&input, &output, "mp4", &[resize, operation("removeAudio")], &AtomicBool::new(false)).unwrap();
        let probe = Command::new("ffprobe").args(["-v", "error", "-show_entries", "stream=codec_type,width,height", "-of", "json", output.to_string_lossy().as_ref()]).output().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
        let streams = json["streams"].as_array().unwrap();
        assert!(streams.iter().all(|stream| stream["codec_type"] != "audio"));
        assert_eq!(streams[0]["width"], 320); assert_eq!(streams[0]["height"], 180);
        fs::remove_dir_all(dir).unwrap();
    }
}
