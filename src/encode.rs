//! The ffmpeg process that turns rendered frames into an MP4.
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::mpsc,
};

pub struct Encode<'a> {
    pub fps: f64,
    pub audio: Option<&'a Path>,
    pub output: &'a Path,
}

/// Arguments after `ffmpeg`. The frames arrive as PNGs on stdin. H.264 in
/// yuv420p plays in every browser; audio, when given, is encoded as AAC and
/// the video keeps its length.
pub fn ffmpeg_args(e: &Encode<'_>) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "image2pipe",
        "-c:v",
        "png",
        "-framerate",
    ]
    .map(OsString::from)
    .into();
    args.push(e.fps.to_string().into());
    // Without these ffmpeg reads five seconds of frames before it starts,
    // and only then finds out that it cannot open the audio.
    args.extend(["-probesize", "32", "-analyzeduration", "0", "-i", "pipe:0"].map(OsString::from));
    if let Some(audio) = e.audio {
        args.push("-i".into());
        args.push(audio.into());
    }
    args.extend(["-map", "0:v"].map(OsString::from));
    if e.audio.is_some() {
        args.extend(
            [
                "-map",
                "1:a",
                "-c:a",
                "aac",
                "-b:a",
                "192k",
                "-af",
                "apad",
                "-shortest",
            ]
            .map(OsString::from),
        );
    }
    args.extend(
        [
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-crf",
            "18",
            "-movflags",
            "+faststart",
        ]
        .map(OsString::from),
    );
    args.push(e.output.into());
    args
}

/// The file the bridge writes for `frame`.
#[must_use]
pub fn frame_file(dir: &Path, frame: i32) -> PathBuf {
    dir.join(format!("frame_{frame:04}.png"))
}

/// Runs ffmpeg and hands it every frame file that arrives, removing each one
/// it took. Resolves once the channel is closed and ffmpeg has written the
/// MP4; dropping the future kills ffmpeg.
pub async fn encode(
    args: Vec<OsString>,
    mut frames: mpsc::UnboundedReceiver<PathBuf>,
) -> Result<(), String> {
    let mut child = Command::new("ffmpeg")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("cannot run ffmpeg: {e}"))?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let complaints = tokio::spawn(async move {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text).await;
        text
    });
    // Owns stdin, so ffmpeg sees the end of the frames when this is over.
    let feed = async move {
        while let Some(frame) = frames.recv().await {
            let png = tokio::fs::read(&frame)
                .await
                .map_err(|e| format!("cannot read {}: {e}", frame.display()))?;
            if stdin.write_all(&png).await.is_err() {
                // ffmpeg is going away; its exit status and stderr say why.
                return Ok(false);
            }
            let _ = tokio::fs::remove_file(&frame).await;
        }
        Ok::<_, String>(true)
    };
    let took_every_frame = tokio::select! {
        every = feed => every?,
        _ = child.wait() => false,
    };
    let status = child.wait().await;
    let complaints = complaints.await.unwrap_or_default();
    match status {
        Ok(status) if status.success() && took_every_frame => Ok(()),
        Ok(status) => Err(format!("ffmpeg failed ({status}): {}", complaints.trim())),
        Err(e) => Err(format!("ffmpeg failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: Vec<OsString>) -> Vec<String> {
        args.into_iter().map(|a| a.into_string().unwrap()).collect()
    }

    #[test]
    fn frames_come_from_stdin() {
        let args = ffmpeg_args(&Encode {
            fps: 24.0,
            audio: None,
            output: Path::new("/tmp/job/out.mp4"),
        });
        assert_eq!(
            strings(args),
            [
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "image2pipe",
                "-c:v",
                "png",
                "-framerate",
                "24",
                "-probesize",
                "32",
                "-analyzeduration",
                "0",
                "-i",
                "pipe:0",
                "-map",
                "0:v",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-crf",
                "18",
                "-movflags",
                "+faststart",
                "/tmp/job/out.mp4",
            ]
        );
    }

    #[test]
    fn with_audio_and_fractional_rate() {
        let args = strings(ffmpeg_args(&Encode {
            fps: 29.97,
            audio: Some(Path::new("/c/assets/voice.wav")),
            output: Path::new("/t/out.mp4"),
        }));
        let joined = args.join(" ");
        assert!(
            joined.contains("-framerate 29.97 -probesize 32 -analyzeduration 0 -i pipe:0 -i /c/a"),
            "{joined}"
        );
        assert!(
            joined.contains(
                "-i /c/assets/voice.wav -map 0:v -map 1:a -c:a aac -b:a 192k -af apad -shortest"
            ),
            "{joined}"
        );
        assert_eq!(args.last().map(String::as_str), Some("/t/out.mp4"));
    }

    #[test]
    fn frame_files_are_numbered_like_the_bridge_writes_them() {
        let dir = Path::new("/t/frames");
        assert_eq!(frame_file(dir, 1), dir.join("frame_0001.png"));
        assert_eq!(frame_file(dir, 12345), dir.join("frame_12345.png"));
        assert_eq!(frame_file(dir, -5), dir.join("frame_-005.png"));
    }
}
