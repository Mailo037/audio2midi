//! Audio von Links laden (YouTube, SoundCloud, Bandcamp …) über yt-dlp.
//! yt-dlp wird bei Bedarf automatisch heruntergeladen.

use anyhow::{anyhow, bail, Context, Result};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Debug, Default)]
pub struct LinkOptions {
    /// cookies.txt (Netscape-Format)
    pub cookies_file: Option<PathBuf>,
    /// z. B. "firefox", "chrome"
    pub cookies_browser: Option<String>,
}

pub struct Downloaded {
    pub bytes: Vec<u8>,
    pub extension: String,
    pub title: String,
}

const EXE: &str = if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" };

fn release_asset() -> &'static str {
    if cfg!(windows) {
        "yt-dlp.exe"
    } else if cfg!(target_os = "macos") {
        "yt-dlp_macos"
    } else if cfg!(target_arch = "aarch64") {
        "yt-dlp_linux_aarch64"
    } else {
        "yt-dlp_linux"
    }
}

/// Eigener Datenordner, z. B. `%LOCALAPPDATA%\audio2midi` oder `~/.local/share/audio2midi`.
pub fn data_dir() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("audio2midi")
}

fn in_path(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).map(|p| p.join(name)).find(|p| p.is_file())
}

/// yt-dlp suchen: neben dem Programm, im Datenordner oder im PATH.
pub fn find_ytdlp() -> Option<PathBuf> {
    let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(EXE)));
    beside
        .filter(|p| p.is_file())
        .or_else(|| Some(data_dir().join(EXE)).filter(|p| p.is_file()))
        .or_else(|| in_path(EXE))
}

pub fn has_ffmpeg() -> bool {
    in_path(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" }).is_some()
}

fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// Aktuelle yt-dlp-Version von GitHub laden (nutzt das System-`curl`, auch unter Windows 10+ vorhanden).
pub fn install_ytdlp() -> Result<PathBuf> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("Cannot create {}", dir.display()))?;
    let dst = dir.join(EXE);
    let url = format!("https://github.com/yt-dlp/yt-dlp/releases/latest/download/{}", release_asset());
    let status = no_window(Command::new("curl").args(["-fsSL", "--retry", "3", "-o"]).arg(&dst).arg(&url))
        .status()
        .context("curl not found – please install yt-dlp manually")?;
    if !status.success() || !dst.is_file() {
        bail!("yt-dlp could not be downloaded ({url})");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(dst)
}

fn temp_dir() -> Result<PathBuf> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("audio2midi-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Audiospur eines Links laden. `progress(anteil, text)` wird laufend aufgerufen.
pub fn download(url: &str, opts: &LinkOptions, mut progress: impl FnMut(f32, &str)) -> Result<Downloaded> {
    let ytdlp = match find_ytdlp() {
        Some(p) => p,
        None => {
            progress(0.0, "Downloading yt-dlp (one-time) …");
            install_ytdlp()?
        }
    };
    let dir = temp_dir()?;
    let mut cmd = Command::new(&ytdlp);
    cmd.args(["--no-playlist", "--newline", "--no-warnings", "--progress", "--no-simulate"])
        .args(["--progress-template", "download:PROG %(progress._percent_str)s"])
        .args(["--print", "after_move:FILE %(filepath)s"])
        .args(["--print", "TITLE %(title)s"])
        .arg("-o")
        .arg(dir.join("audio.%(ext)s"));
    if has_ffmpeg() {
        cmd.args(["-f", "bestaudio/best", "-x", "--audio-format", "wav"]);
    } else {
        // Formate bevorzugen, die ohne ffmpeg dekodiert werden können
        cmd.args([
            "-f",
            "bestaudio[acodec^=mp4a]/bestaudio[acodec=mp3]/bestaudio[acodec=vorbis]/bestaudio[acodec=flac]/bestaudio/best",
        ]);
    }
    if let Some(c) = &opts.cookies_file {
        cmd.arg("--cookies").arg(c);
    }
    if let Some(b) = opts.cookies_browser.as_deref().filter(|b| !b.is_empty()) {
        cmd.args(["--cookies-from-browser", b]);
    }
    cmd.arg(url).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = no_window(&mut cmd).spawn().with_context(|| format!("Cannot start {}", ytdlp.display()))?;

    let stderr = child.stderr.take().unwrap();
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        for line in BufReader::new(stderr).lines().map_while(|l| l.ok()) {
            s += &line;
            s.push('\n');
        }
        s
    });
    let (mut file, mut title) = (None, String::from("download"));
    for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(|l| l.ok()) {
        if let Some(p) = line.strip_prefix("PROG ") {
            if let Ok(v) = p.trim().trim_end_matches('%').parse::<f32>() {
                progress(v / 100.0, "Downloading audio …");
            }
        } else if let Some(f) = line.strip_prefix("FILE ") {
            file = Some(PathBuf::from(f.trim()));
        } else if let Some(t) = line.strip_prefix("TITLE ") {
            title = t.trim().to_string();
        }
    }
    let status = child.wait()?;
    let errors = err_thread.join().unwrap_or_default();
    if !status.success() {
        let msg = errors.lines().filter(|l| l.contains("ERROR")).collect::<Vec<_>>().join("\n");
        let mut msg = if msg.is_empty() { errors.trim().to_string() } else { msg };
        if msg.contains("Sign in to confirm") || msg.to_lowercase().contains("bot") {
            msg += "\n\nTip: YouTube requires you to sign in. In the link options, choose your browser for cookies \
                    or provide a cookies.txt.";
        }
        bail!("Download failed: {msg}");
    }
    let file = file
        .filter(|f| f.is_file())
        .or_else(|| std::fs::read_dir(&dir).ok()?.flatten().map(|e| e.path()).find(|p| p.is_file()))
        .ok_or_else(|| anyhow!("Download failed: no file received"))?;
    let bytes = std::fs::read(&file)?;
    let extension = ext_of(&file);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(Downloaded { bytes, extension, title })
}

fn ext_of(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase()
}

pub fn is_url(s: &str) -> bool {
    let s = s.trim();
    s.starts_with("http://") || s.starts_with("https://")
}
