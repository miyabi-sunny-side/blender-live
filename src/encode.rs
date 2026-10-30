//! The ffmpeg command that turns rendered frames into an MP4.
use std::{ffi::OsString, path::Path};

pub struct Encode<'a> {
    pub frames_dir: &'a Path,
    pub frame_start: i32,
    pub fps: f64,
    pub audio: Option<&'a Path>,
    pub output: &'a Path,
}

/// Arguments after `ffmpeg`. H.264 in yuv420p plays in every browser;
/// audio, when given, is encoded as AAC and the video keeps its length.
pub fn ffmpeg_args(e: &Encode<'_>) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["-hide_banner", "-loglevel", "error", "-y", "-framerate"]
        .map(OsString::from)
        .into();
    args.push(e.fps.to_string().into());
    args.push("-start_number".into());
    args.push(e.frame_start.to_string().into());
    args.push("-i".into());
    args.push(e.frames_dir.join("frame_%04d.png").into());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: Vec<OsString>) -> Vec<String> {
        args.into_iter().map(|a| a.into_string().unwrap()).collect()
    }

    #[test]
    fn frames_only() {
        let args = ffmpeg_args(&Encode {
            frames_dir: Path::new("/camp/renders/clip"),
            frame_start: 1,
            fps: 24.0,
            audio: None,
            output: Path::new("/camp/renders/clip.mp4"),
        });
        assert_eq!(
            strings(args),
            [
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-framerate",
                "24",
                "-start_number",
                "1",
                "-i",
                "/camp/renders/clip/frame_%04d.png",
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
                "/camp/renders/clip.mp4",
            ]
        );
    }

    #[test]
    fn with_audio_and_fractional_rate() {
        let args = strings(ffmpeg_args(&Encode {
            frames_dir: Path::new("/c/r/x"),
            frame_start: 10,
            fps: 29.97,
            audio: Some(Path::new("/c/assets/voice.wav")),
            output: Path::new("/c/r/x.mp4"),
        }));
        let joined = args.join(" ");
        assert!(
            joined.contains("-framerate 29.97 -start_number 10 -i /c/r/x/frame_%04d.png"),
            "{joined}"
        );
        assert!(
            joined.contains(
                "-i /c/assets/voice.wav -map 0:v -map 1:a -c:a aac -b:a 192k -af apad -shortest"
            ),
            "{joined}"
        );
        assert_eq!(args.last().map(String::as_str), Some("/c/r/x.mp4"));
    }
}
